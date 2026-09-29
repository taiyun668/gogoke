import { createHash } from "node:crypto";
import { describe, expect, it } from "vite-plus/test";

import { GitFactWriteError, R2_TEST_LEDGER, writeR2TestFact } from "./gitFactWrite.ts";
import { GitFactHttpError } from "./gitFactWriteHttp.ts";
import type { GitFactWritePort, R2TestFactJournal, R2TestFactJournalEntry } from "./gitFactWrite.ts";

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
  readbackFailed?: boolean; descendant?: boolean; unrelated?: boolean; headReadFails?: boolean;
  advancesAfterRead?: boolean } = {}) {
  let head = initial;
  const child = "e".repeat(40);
  const calls: string[] = [];
  let authorityCalls = 0;
  let updateCalls = 0;
  let saved: R2TestFactJournalEntry | null = null;
  const journal: R2TestFactJournal = {
    async begin(intent) {
      calls.push("journal:begin");
      if (saved === null) saved = { ...intent, baseHead: null, targetCommit: null };
      return saved;
    },
    async bindTarget(intent, baseHead, targetCommit) {
      calls.push("journal:bind");
      if (saved === null || saved.operationId !== intent.operationId ||
          (saved.targetCommit !== null && (saved.targetCommit !== targetCommit ||
            saved.baseHead !== baseHead))) throw new Error("journal conflict");
      saved = { ...intent, baseHead, targetCommit };
      return saved;
    },
    async rejectTarget(intent, baseHead, targetCommit) {
      calls.push("journal:reject");
      if (saved === null || saved.operationId !== intent.operationId ||
          saved.baseHead !== baseHead || saved.targetCommit !== targetCommit) {
        throw new Error("journal conflict");
      }
      saved = { ...intent, baseHead: null, targetCommit: null };
      return saved;
    },
  };
  const port: GitFactWritePort = {
    repository: R2_TEST_LEDGER.repository,
    branch: R2_TEST_LEDGER.branch,
    async assertCurrentAuthority() {
      calls.push("authority");
      authorityCalls += 1;
      if (options.changedAuthority && authorityCalls === 2) throw new Error("revoked");
    },
    async readHead() {
      calls.push("head");
      if (options.headReadFails && saved?.targetCommit) throw new Error("ref unavailable");
      const observed = head;
      if (options.advancesAfterRead && observed === initial) head = child;
      return { commit: observed, tree };
    },
    async readPath(_commit, value) { calls.push(`path:${value}`); return null; },
    async isAncestor(ancestor, descendant) {
      calls.push("ancestor");
      expect([candidate, "f".repeat(40)]).toContain(ancestor);
      return options.descendant === true && descendant === child;
    },
    async createBlob(bytes) { calls.push("blob"); expect(Buffer.from(bytes)).toEqual(source.bytes); return blob; },
    async createTree(base, value, valueBlob) {
      calls.push("tree"); expect([base, value, valueBlob]).toEqual([tree, path, blob]); return nextTree;
    },
    async createCommit(parent, valueTree) {
      calls.push("commit");
      expect(valueTree).toBe(nextTree);
      expect([initial, child]).toContain(parent);
      return parent === initial ? candidate : "f".repeat(40);
    },
    async updateRef(value) {
      calls.push("update"); expect([candidate, "f".repeat(40)]).toContain(value);
      updateCalls += 1;
      if (options.advancesAfterRead && updateCalls === 1) {
        throw new GitFactHttpError("GIT_FACT_REF_NON_FAST_FORWARD");
      }
      if (!options.updateThrows || options.landed) head = options.descendant ? child : value;
      if (options.unrelated) head = child;
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
  return { calls, port, fetcher, journal,
    setHead(value: string) { head = value; } };
}

describe("R2-02 test Git fact write discipline", () => {
  it("writes only a scoped draft and verifies exact committed bytes", async () => {
    const current = fixture();
    const result = await writeR2TestFact(source, current.port, current.fetcher, undefined, current.journal);
    expect(result.state).toBe("DRAFT_COMMITTED_NOT_ADOPTED");
    expect(result.commit).toBe(candidate);
    expect(result.readback.gitBlob).toBe(blob);
    expect(current.calls).toEqual(["authority", "journal:begin", "head", `path:${path}`,
      "blob", "tree", "commit", "journal:bind", "authority", "update", "head", "readback"]);
  });

  it("reconciles a lost update response only after reading the exact remote head", async () => {
    const current = fixture({ updateThrows: true, landed: true });
    expect((await writeR2TestFact(source, current.port, current.fetcher, undefined, current.journal)).state)
      .toBe("DRAFT_COMMITTED_NOT_ADOPTED");
    expect(current.calls.slice(-3)).toEqual(["update", "head", "readback"]);
  });

  it("retries a lost readback using the bound target without another Git write", async () => {
    const current = fixture({ readbackFailed: true });
    await expect(writeR2TestFact(source, current.port, current.fetcher, undefined, current.journal))
      .rejects.toMatchObject({ code: "TEST_FACT_COMMITTED_READBACK_UNVERIFIED",
        commit: candidate, path });
    current.calls.length = 0;
    const recovered = fixture();
    const result = await writeR2TestFact(source, current.port, recovered.fetcher,
      undefined, current.journal);
    expect(result.commit).toBe(candidate);
    expect(current.calls).toEqual(["authority", "journal:begin", "head"]);
    expect(recovered.calls).toEqual(["readback"]);
  });

  it("does not resend when the remote update outcome is unknown", async () => {
    const current = fixture({ updateThrows: true });
    await expect(writeR2TestFact(source, current.port, current.fetcher, undefined, current.journal))
      .rejects.toMatchObject({ code: "TEST_FACT_WRITE_OUTCOME_UNKNOWN",
        commit: candidate, path });
    expect(current.calls.filter((call) => call === "update")).toHaveLength(1);
    expect(current.calls).not.toContain("readback");
    current.calls.length = 0;
    await expect(writeR2TestFact(source, current.port, current.fetcher, undefined, current.journal))
      .rejects.toMatchObject({ code: "TEST_FACT_WRITE_OUTCOME_UNKNOWN",
        commit: candidate, path });
    expect(current.calls).toEqual(["authority", "journal:begin", "head", "ancestor"]);
  });

  it("releases only a confirmed non-fast-forward target, then rebuilds on the advanced branch", async () => {
    const current = fixture({ advancesAfterRead: true });
    await expect(writeR2TestFact(source, current.port, current.fetcher, undefined, current.journal))
      .rejects.toMatchObject({ code: "TEST_FACT_REF_NON_FAST_FORWARD", commit: candidate, path });
    expect(current.calls.slice(-4)).toEqual(["update", "head", "ancestor", "journal:reject"]);
    current.calls.length = 0;
    const result = await writeR2TestFact(source, current.port, current.fetcher, undefined, current.journal);
    expect(result.state).toBe("DRAFT_COMMITTED_NOT_ADOPTED");
    expect(current.calls).toContain("journal:bind");
    expect(current.calls.filter((call) => call === "update")).toHaveLength(1);
  });

  it("rejects changed bytes for the same operation before any Git read or write", async () => {
    const current = fixture({ updateThrows: true });
    await expect(writeR2TestFact(source, current.port, current.fetcher, undefined, current.journal))
      .rejects.toMatchObject({ code: "TEST_FACT_WRITE_OUTCOME_UNKNOWN" });
    current.calls.length = 0;
    const changed = { ...source, bytes: Buffer.from(JSON.stringify({ testOnly: true,
      executionEvidenceSha: source.executionEvidenceSha, reportSha256: "3".repeat(64) })) };
    await expect(writeR2TestFact(changed, current.port, current.fetcher, undefined, current.journal))
      .rejects.toMatchObject({ code: "TEST_FACT_OPERATION_CONFLICT" });
    expect(current.calls).toEqual(["authority", "journal:begin"]);
  });

  it("accepts a descendant ref only after exact target readback", async () => {
    const current = fixture({ updateThrows: true, landed: true, descendant: true });
    const result = await writeR2TestFact(source, current.port, current.fetcher,
      undefined, current.journal);
    expect(result.commit).toBe(candidate);
    expect(result.readback.coordinate.commit).toBe(candidate);
    expect(current.calls.slice(-3)).toEqual(["head", "ancestor", "readback"]);
    current.calls.length = 0;
    await writeR2TestFact(source, current.port, current.fetcher, undefined, current.journal);
    expect(current.calls).toEqual(["authority", "journal:begin", "head", "ancestor", "readback"]);
  });

  it("rejects unrelated head even when the target bytes can be read", async () => {
    const current = fixture({ updateThrows: true, unrelated: true });
    await expect(writeR2TestFact(source, current.port, current.fetcher,
      undefined, current.journal)).rejects.toMatchObject({
        code: "TEST_FACT_WRITE_OUTCOME_UNKNOWN", commit: candidate, path,
      });
    expect(current.calls).not.toContain("readback");
  });

  it("keeps a ref query failure unknown and never retries the update", async () => {
    const current = fixture({ updateThrows: true, headReadFails: true });
    await expect(writeR2TestFact(source, current.port, current.fetcher,
      undefined, current.journal)).rejects.toMatchObject({
        code: "TEST_FACT_WRITE_OUTCOME_UNKNOWN", commit: candidate, path,
      });
    current.calls.length = 0;
    await expect(writeR2TestFact(source, current.port, current.fetcher,
      undefined, current.journal)).rejects.toMatchObject({
        code: "TEST_FACT_WRITE_OUTCOME_UNKNOWN", commit: candidate, path,
      });
    expect(current.calls).toEqual(["authority", "journal:begin", "head"]);
  });

  it("reports committed bytes as unverified when readback fails after the write", async () => {
    const current = fixture({ readbackFailed: true });
    await expect(writeR2TestFact(source, current.port, current.fetcher, undefined, current.journal))
      .rejects.toMatchObject({ code: "TEST_FACT_COMMITTED_READBACK_UNVERIFIED",
        commit: candidate, path });
    expect(current.calls.filter((call) => call === "update")).toHaveLength(1);
  });

  it("rechecks authority before the ref update and rejects a different branch", async () => {
    const revoked = fixture({ changedAuthority: true });
    await expect(writeR2TestFact(source, revoked.port, revoked.fetcher, undefined, revoked.journal))
      .rejects.toThrow("revoked");
    expect(revoked.calls).not.toContain("update");
    const wrong = fixture();
    await expect(writeR2TestFact(source, { ...wrong.port, branch: "main" }, wrong.fetcher,
      undefined, wrong.journal))
      .rejects.toBeInstanceOf(GitFactWriteError);
    expect(wrong.calls).toHaveLength(0);
  });
});
