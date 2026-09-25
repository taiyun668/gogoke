import { createHash } from "node:crypto";
import { describe, expect, it } from "vite-plus/test";

import { GitFactReadbackError, readAcceptedGitHubFact, readGitHubFact } from "./gitFact.ts";

const repository = "taiyun668/gogoke";
const commit = "6765d4e11ace61c47b9aeb123e0ef4770ab072c0";
const path = "apps/desktop/test-fixtures/s1-r4/ledger/r2-02-source-reference.json";
const bytes = Buffer.from('{"testOnly":true,"value":"known fixture"}\n');
const sha256 = createHash("sha256").update(bytes).digest("hex");
const blob = createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
const coordinate = { repository, commit, path, contentHash: `sha256:${sha256}` };

function fake(body: unknown, status = 200) {
  const calls: string[] = [];
  const fetcher: typeof fetch = async (url) => {
    calls.push(String(url));
    return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  };
  return { calls, fetcher };
}

describe("R2-02 Git fact readback", () => {
  it("binds an authorized repository, immutable commit, Git blob and exact bytes without adoption", async () => {
    const { calls, fetcher } = fake({
      type: "file", path, sha: blob, size: bytes.length, encoding: "base64",
      content: `${bytes.toString("base64").slice(0, 24)}\n${bytes.toString("base64").slice(24)}`,
    });
    const readback = await readGitHubFact(coordinate, repository, fetcher);
    expect(calls).toEqual([
      `https://api.github.com/repos/taiyun668/gogoke/contents/${path}?ref=${commit}`,
    ]);
    expect(readback.state).toBe("COMMITTED_BYTES_VERIFIED_NOT_ADOPTED");
    expect(readback.gitBlob).toBe(blob);
    expect(Buffer.from(readback.bytes)).toEqual(bytes);
  });

  it("rejects wrong repository, mutable ref and escaping path before network I/O", async () => {
    const { calls, fetcher } = fake({});
    for (const invalid of [
      { ...coordinate, repository: "elsewhere/repo" },
      { ...coordinate, commit: "main" },
      { ...coordinate, path: "../private.txt" },
    ]) {
      await expect(readGitHubFact(invalid, repository, fetcher)).rejects.toBeInstanceOf(GitFactReadbackError);
    }
    expect(calls).toHaveLength(0);
  });

  it("rejects a spoofed blob or changed content even when GitHub returned HTTP 200", async () => {
    const response = { type: "file", path, sha: blob, size: bytes.length,
      encoding: "base64", content: bytes.toString("base64") };
    await expect(readGitHubFact(coordinate,
      repository, fake({ ...response, sha: "0".repeat(40) }).fetcher))
      .rejects.toMatchObject({ code: "GIT_FACT_BLOB_MISMATCH" });
    await expect(readGitHubFact({ ...coordinate, contentHash: `sha256:${"0".repeat(64)}` },
      repository, fake(response).fetcher))
      .rejects.toMatchObject({ code: "GIT_FACT_HASH_MISMATCH" });
  });

  it("uses the supplied GitHub credential only for the pinned API request", async () => {
    const fixtureCredential = "test-only-value".repeat(3);
    const seen: Array<{ url: string; token: string | null; redirect: string | undefined }> = [];
    const fetcher: typeof fetch = async (url, init) => {
      seen.push({ url: String(url), token: new Headers(init?.headers).get("authorization"),
        redirect: init?.redirect });
      return new Response(JSON.stringify({ type: "file", path, sha: blob, size: bytes.length,
        encoding: "base64", content: bytes.toString("base64") }), { status: 200 });
    };
    await readGitHubFact(coordinate, repository, fetcher, fixtureCredential);
    expect(seen).toEqual([{ url: `https://api.github.com/repos/taiyun668/gogoke/contents/${path}?ref=${commit}`,
      token: `Bearer ${fixtureCredential}`, redirect: "error" }]);
    await expect(readGitHubFact(coordinate, repository, fetcher, "bad\nheader"))
      .rejects.toMatchObject({ code: "GIT_FACT_CREDENTIAL_INVALID" });
    expect(seen).toHaveLength(1);
  });
});

const acceptancePolicy = { repository, targetBranch: "main", mergeActor: "taiyun668" };
const pullNumber = 42;
const mergedPull = {
  number: pullNumber, state: "closed", draft: false, merged: true,
  merged_at: "2026-09-24T08:12:34Z", merge_commit_sha: commit,
  merged_by: { login: acceptancePolicy.mergeActor, id: 12345, type: "User" },
  base: { ref: acceptancePolicy.targetBranch, repo: { full_name: repository, id: 67890 } },
};
const fileBody = { type: "file", path, sha: blob, size: bytes.length,
  encoding: "base64", content: bytes.toString("base64") };

function acceptanceFetcher(pull: unknown = mergedPull, file: unknown = fileBody,
                           changedFiles: unknown = [{ filename: path, status: "added" }]) {
  const calls: string[] = [];
  const fetcher: typeof fetch = async (url) => {
    const request = String(url);
    calls.push(request);
    return new Response(JSON.stringify(request.includes("/files?") ? changedFiles :
      request.includes("/pulls/") ? pull : file), { status: 200 });
  };
  return { calls, fetcher };
}

describe("R2-05 GitHub accepted fact readback", () => {
  it("verifies the exact merged PR and immutable fact bytes at its merge commit", async () => {
    const { calls, fetcher } = acceptanceFetcher();
    const accepted = await readAcceptedGitHubFact(coordinate, pullNumber, acceptancePolicy, fetcher);
    expect(calls).toEqual([
      `https://api.github.com/repos/taiyun668/gogoke/pulls/${pullNumber}`,
      `https://api.github.com/repos/taiyun668/gogoke/pulls/${pullNumber}/files?per_page=100&page=1`,
      `https://api.github.com/repos/taiyun668/gogoke/contents/${path}?ref=${commit}`,
    ]);
    expect(accepted).toMatchObject({ state: "PR_MERGE_ACCEPTED_FACT_VERIFIED", pullNumber,
      mergedAt: mergedPull.merged_at, mergeCommit: commit, mergedBy: acceptancePolicy.mergeActor });
    expect(accepted.fact.state).toBe("COMMITTED_BYTES_VERIFIED_NOT_ADOPTED");
    expect(Buffer.from(accepted.fact.bytes)).toEqual(bytes);
  });

  it("rejects untrusted coordinates and invalid policy before requesting GitHub", async () => {
    const { calls, fetcher } = acceptanceFetcher();
    for (const [candidate, number, policy] of [
      [{ ...coordinate, repository: "elsewhere/repo" }, pullNumber, acceptancePolicy],
      [{ ...coordinate, commit: "main" }, pullNumber, acceptancePolicy],
      [coordinate, 0, acceptancePolicy],
      [coordinate, pullNumber, { ...acceptancePolicy, mergeActor: "" }],
    ] as const) {
      await expect(readAcceptedGitHubFact(candidate, number, policy, fetcher))
        .rejects.toBeInstanceOf(GitFactReadbackError);
    }
    expect(calls).toHaveLength(0);
  });

  it("rejects wrong base repository, branch, actor, merge SHA, unmerged and draft PRs", async () => {
    for (const pull of [
      { ...mergedPull, base: { ...mergedPull.base, repo: { full_name: "elsewhere/repo" } } },
      { ...mergedPull, base: { ...mergedPull.base, ref: "other" } },
      { ...mergedPull, merged_by: { login: "another-user" } },
      { ...mergedPull, merge_commit_sha: "f".repeat(40) },
      { ...mergedPull, merged: false, merged_at: null },
      { ...mergedPull, state: "open" },
      { ...mergedPull, draft: true },
      { ...mergedPull, number: pullNumber + 1 },
    ]) {
      const { calls, fetcher } = acceptanceFetcher(pull);
      await expect(readAcceptedGitHubFact(coordinate, pullNumber, acceptancePolicy, fetcher))
        .rejects.toMatchObject({ code: "GIT_FACT_ACCEPTANCE_MISMATCH" });
      expect(calls).toHaveLength(1);
    }
  });

  it("requires complete GitHub merge metadata and valid merged time", async () => {
    for (const pull of [
      { ...mergedPull, base: { ref: "main" } },
      { ...mergedPull, base: { repo: { full_name: repository } } },
      { ...mergedPull, merged_by: null },
      { ...mergedPull, merged_at: null },
      { ...mergedPull, merged_at: "yesterday" },
      { ...mergedPull, merged_at: "2026-02-31T08:12:34Z" },
      { ...mergedPull, merge_commit_sha: null },
    ]) {
      const { calls, fetcher } = acceptanceFetcher(pull);
      await expect(readAcceptedGitHubFact(coordinate, pullNumber, acceptancePolicy, fetcher))
        .rejects.toMatchObject({ code: "GIT_FACT_ACCEPTANCE_MISMATCH" });
      expect(calls).toHaveLength(1);
    }
  });

  it("does not accept a valid merge record when fact bytes disagree with the coordinate", async () => {
    const { calls, fetcher } = acceptanceFetcher(mergedPull, { ...fileBody, content: Buffer.from("changed").toString("base64") });
    await expect(readAcceptedGitHubFact(coordinate, pullNumber, acceptancePolicy, fetcher))
      .rejects.toBeInstanceOf(GitFactReadbackError);
    expect(calls).toHaveLength(3);
  });

  it("rejects an unrelated PR, removed path and ambiguous duplicate path", async () => {
    for (const changedFiles of [
      [{ filename: "other.json", status: "added" }],
      [{ filename: path, status: "removed" }],
      [{ filename: path, status: "added" }, { filename: path, status: "modified" }],
    ]) {
      const { calls, fetcher } = acceptanceFetcher(mergedPull, fileBody, changedFiles);
      await expect(readAcceptedGitHubFact(coordinate, pullNumber, acceptancePolicy, fetcher))
        .rejects.toMatchObject({ code: "GIT_FACT_ACCEPTANCE_MISMATCH" });
      expect(calls).toHaveLength(2);
    }
  });

  it("checks later PR-files pages and rejects a full final page without proof of exhaustion", async () => {
    const filler = Array.from({ length: 100 }, (_, index) =>
      ({ filename: `other-${index}.json`, status: "added" }));
    const calls: string[] = [];
    const fetcher: typeof fetch = async (url) => {
      const request = String(url);
      calls.push(request);
      const body = request.includes("/files?") ? request.endsWith("page=1") ? filler :
        [{ filename: path, status: "modified" }] : request.includes("/pulls/") ? mergedPull : fileBody;
      return new Response(JSON.stringify(body), { status: 200 });
    };
    const accepted = await readAcceptedGitHubFact(coordinate, pullNumber, acceptancePolicy, fetcher);
    expect(accepted.state).toBe("PR_MERGE_ACCEPTED_FACT_VERIFIED");
    expect(calls).toHaveLength(4);
    expect(calls[2]).toContain("page=2");

    const fullPages = acceptanceFetcher(mergedPull, fileBody, filler);
    await expect(readAcceptedGitHubFact(coordinate, pullNumber, acceptancePolicy, fullPages.fetcher))
      .rejects.toMatchObject({ code: "GIT_FACT_PR_FILES_PAGE_LIMIT" });
    expect(fullPages.calls).toHaveLength(11);
  });

  it("rejects failed, malformed and oversized PR responses before fact readback", async () => {
    for (const [response, code] of [
      [new Response("missing", { status: 404 }), "GIT_FACT_ACCEPTANCE_READ_FAILED"],
      [new Response("not json", { status: 200 }), "GIT_FACT_ACCEPTANCE_RESPONSE_INVALID"],
      [new Response("x".repeat(96 * 1024 + 1), { status: 200 }), "GIT_FACT_RESPONSE_TOO_LARGE"],
    ] as const) {
      const calls: string[] = [];
      const fetcher: typeof fetch = async (url) => { calls.push(String(url)); return response; };
      await expect(readAcceptedGitHubFact(coordinate, pullNumber, acceptancePolicy, fetcher))
        .rejects.toMatchObject({ code });
      expect(calls).toHaveLength(1);
    }
  });
});
