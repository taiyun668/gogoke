import { createHash } from "node:crypto";
import { describe, expect, it } from "vite-plus/test";

import { GitFactWriteError, R2_TEST_LEDGER, writeR2TestFact } from "./gitFactWrite.ts";
import type { GitFactWritePort } from "./gitFactWrite.ts";

const source = {
  operationId: "r2-02-controlled-test",
  executionEvidenceSha: "1".repeat(40),
  bytes: Buffer.from(JSON.stringify({ testOnly: true, executionEvidenceSha: "1".repeat(40),
    reportSha256: "2".repeat(64) })),
};
const initial = "a".repeat(40);
const tree = "b".repeat(40);
const nextTree = "c".repeat(40);
const candidate = "d".repeat(40);
const blob = createHash("sha1").update(`blob ${source.bytes.length}\0`).update(source.bytes).digest("hex");
const path = `${R2_TEST_LEDGER.pathRoot}/${source.operationId}.json`;

function fixture(options: { updateThrows?: boolean; landed?: boolean; changedAuthority?: boolean;
  readbackFailed?: boolean } = {}) {
  let head = initial;
  const calls: string[] = [];
  let authorityCalls = 0;
  const port: GitFactWritePort = {
    repository: R2_TEST_LEDGER.repository,
    branch: R2_TEST_LEDGER.branch,
    async assertCurrentAuthority() {
      calls.push("authority");
      authorityCalls += 1;
      if (options.changedAuthority && authorityCalls === 2) throw new Error("revoked");
    },
    async readHead() { calls.push("head"); return { commit: head, tree }; },
    async readPath(_commit, value) { calls.push(`path:${value}`); return null; },
    async createBlob(bytes) { calls.push("blob"); expect(Buffer.from(bytes)).toEqual(source.bytes); return blob; },
    async createTree(base, value, valueBlob) {
      calls.push("tree"); expect([base, value, valueBlob]).toEqual([tree, path, blob]); return nextTree;
    },
    async createCommit(parent, valueTree) {
      calls.push("commit"); expect([parent, valueTree]).toEqual([initial, nextTree]); return candidate;
    },
    async updateRef(value) {
      calls.push("update"); expect(value).toBe(candidate);
      if (!options.updateThrows || options.landed) head = candidate;
      if (options.updateThrows) throw new Error("remote response lost");
    },
  };
  const fetcher: typeof fetch = async () => {
    calls.push("readback");
    if (options.readbackFailed) return new Response("unavailable", { status: 503 });
    return new Response(JSON.stringify({ type: "file", path, sha: blob,
      size: source.bytes.length, encoding: "base64", content: source.bytes.toString("base64") }),
      { status: 200 });
  };
  return { calls, port, fetcher };
}

describe("R2-02 test Git fact write discipline", () => {
  it("writes only a scoped draft and verifies exact committed bytes", async () => {
    const current = fixture();
    const result = await writeR2TestFact(source, current.port, current.fetcher);
    expect(result.state).toBe("DRAFT_COMMITTED_NOT_ADOPTED");
    expect(result.commit).toBe(candidate);
    expect(result.readback.gitBlob).toBe(blob);
    expect(current.calls).toEqual(["authority", "head", `path:${path}`, "blob", "tree",
      "commit", "authority", "update", "readback"]);
  });

  it("reconciles a lost update response only after reading the exact remote head", async () => {
    const current = fixture({ updateThrows: true, landed: true });
    expect((await writeR2TestFact(source, current.port, current.fetcher)).state)
      .toBe("DRAFT_COMMITTED_NOT_ADOPTED");
    expect(current.calls.slice(-3)).toEqual(["update", "head", "readback"]);
  });

  it("does not resend when the remote update outcome is unknown", async () => {
    const current = fixture({ updateThrows: true });
    await expect(writeR2TestFact(source, current.port, current.fetcher))
      .rejects.toMatchObject({ code: "TEST_FACT_WRITE_OUTCOME_UNKNOWN" });
    expect(current.calls.filter((call) => call === "update")).toHaveLength(1);
    expect(current.calls).not.toContain("readback");
  });

  it("reports committed bytes as unverified when readback fails after the write", async () => {
    const current = fixture({ readbackFailed: true });
    await expect(writeR2TestFact(source, current.port, current.fetcher))
      .rejects.toMatchObject({ code: "TEST_FACT_COMMITTED_READBACK_UNVERIFIED",
        commit: candidate, path });
    expect(current.calls.filter((call) => call === "update")).toHaveLength(1);
  });

  it("rechecks authority before the ref update and rejects a different branch", async () => {
    const revoked = fixture({ changedAuthority: true });
    await expect(writeR2TestFact(source, revoked.port, revoked.fetcher)).rejects.toThrow("revoked");
    expect(revoked.calls).not.toContain("update");
    const wrong = fixture();
    await expect(writeR2TestFact(source, { ...wrong.port, branch: "main" }, wrong.fetcher))
      .rejects.toBeInstanceOf(GitFactWriteError);
    expect(wrong.calls).toHaveLength(0);
  });
});
