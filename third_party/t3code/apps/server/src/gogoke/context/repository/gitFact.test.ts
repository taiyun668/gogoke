import { createHash } from "node:crypto";
import { describe, expect, it } from "vite-plus/test";

import { GitFactReadbackError, readGitHubFact } from "./gitFact.ts";

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
