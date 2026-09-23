import { createHash } from "node:crypto";

import { readGitHubFact, type GitFactReadback } from "./gitFact.ts";

export const R2_TEST_LEDGER = Object.freeze({
  repository: "taiyun668/gogoke",
  branch: "s1-r4-ledger-test/r2-02",
  pathRoot: "apps/desktop/test-fixtures/s1-r4/ledger/r2-02-results",
});

const SHA = /^[0-9a-f]{40}$/u;
const OPERATION = /^[a-z0-9][a-z0-9-]{0,63}$/u;
const MAX_BYTES = 32 * 1024;

export interface GitFactWritePort {
  readonly repository: string;
  readonly branch: string;
  assertCurrentAuthority(): Promise<void>;
  readHead(): Promise<{ readonly commit: string; readonly tree: string }>;
  readPath(commit: string, path: string): Promise<string | null>;
  createBlob(bytes: Uint8Array): Promise<string>;
  createTree(baseTree: string, path: string, blob: string): Promise<string>;
  createCommit(parent: string, tree: string, message: string): Promise<string>;
  updateRef(commit: string): Promise<void>;
}

export interface R2TestFactWrite {
  readonly operationId: string;
  readonly executionEvidenceSha: string;
  readonly bytes: Uint8Array;
}

export class GitFactWriteError extends Error {
  override readonly name = "GitFactWriteError";
  readonly code: string;
  readonly commit: string | null;
  readonly path: string | null;
  constructor(code: string, options?: ErrorOptions & { commit?: string; path?: string }) {
    super(code, options);
    this.code = code;
    this.commit = options?.commit ?? null;
    this.path = options?.path ?? null;
  }
}

const fail = (code: string): never => { throw new GitFactWriteError(code); };

/** This only creates a test draft on the Owner-authorized ledger branch. */
export async function writeR2TestFact(
  input: R2TestFactWrite,
  port: GitFactWritePort,
  fetcher: typeof fetch = fetch,
): Promise<{
  readonly state: "DRAFT_COMMITTED_NOT_ADOPTED";
  readonly commit: string;
  readonly path: string;
  readonly readback: GitFactReadback;
}> {
  if (!OPERATION.test(input.operationId) || !SHA.test(input.executionEvidenceSha) ||
      input.bytes.length === 0 || input.bytes.length > MAX_BYTES ||
      port.repository !== R2_TEST_LEDGER.repository || port.branch !== R2_TEST_LEDGER.branch) {
    return fail("INVALID_TEST_FACT_INPUT");
  }
  let body: unknown;
  try { body = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(input.bytes)); }
  catch { return fail("INVALID_TEST_FACT_INPUT"); }
  if (typeof body !== "object" || body === null || Array.isArray(body) ||
      (body as Record<string, unknown>).testOnly !== true ||
      (body as Record<string, unknown>).executionEvidenceSha !== input.executionEvidenceSha) {
    return fail("INVALID_TEST_FACT_INPUT");
  }
  const path = `${R2_TEST_LEDGER.pathRoot}/${input.operationId}.json`;
  const sourceHash = createHash("sha256").update(input.bytes).digest("hex");
  const expectedBlob = createHash("sha1").update(`blob ${input.bytes.length}\0`).update(input.bytes).digest("hex");
  await port.assertCurrentAuthority();
  const head = await port.readHead();
  if (!SHA.test(head.commit) || !SHA.test(head.tree)) return fail("INVALID_TEST_LEDGER_HEAD");
  if (await port.readPath(head.commit, path) !== null) return fail("TEST_FACT_PATH_ALREADY_EXISTS");
  const blob = await port.createBlob(input.bytes);
  if (blob !== expectedBlob) return fail("TEST_FACT_BLOB_MISMATCH");
  const tree = await port.createTree(head.tree, path, blob);
  if (!SHA.test(tree)) return fail("INVALID_TEST_FACT_TREE");
  const commit = await port.createCommit(head.commit, tree, `test-only R2-02 fact ${input.operationId}`);
  if (!SHA.test(commit)) return fail("INVALID_TEST_FACT_COMMIT");
  await port.assertCurrentAuthority();
  try {
    await port.updateRef(commit);
  } catch (error) {
    const observed = await port.readHead().catch(() => null);
    if (observed?.commit !== commit) {
      throw new GitFactWriteError("TEST_FACT_WRITE_OUTCOME_UNKNOWN", { cause: error });
    }
  }
  let readback: GitFactReadback;
  try {
    readback = await readGitHubFact({
      repository: R2_TEST_LEDGER.repository,
      commit,
      path,
      contentHash: `sha256:${sourceHash}`,
    }, R2_TEST_LEDGER.repository, fetcher);
  } catch (error) {
    throw new GitFactWriteError("TEST_FACT_COMMITTED_READBACK_UNVERIFIED",
      { cause: error, commit, path });
  }
  if (readback.gitBlob !== blob) return fail("TEST_FACT_READBACK_BLOB_MISMATCH");
  return Object.freeze({ state: "DRAFT_COMMITTED_NOT_ADOPTED" as const, commit, path, readback });
}
