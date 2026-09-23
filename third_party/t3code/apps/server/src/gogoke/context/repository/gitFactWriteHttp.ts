import { R2_TEST_LEDGER, type GitFactWritePort } from "./gitFactWrite.ts";

const API = "https://api.github.com/repos/taiyun668/gogoke";
const MAX_REPLY_BYTES = 96 * 1024;
const SHA = /^[0-9a-f]{40}$/u;

export class GitFactHttpError extends Error {
  override readonly name = "GitFactHttpError";
  readonly code: string;
  constructor(code: string) { super(code); this.code = code; }
}

function fail(code: string): never { throw new GitFactHttpError(code); }

export function createR2TestGitHubWritePort(input: {
  readonly currentAuthority: () => Promise<void>;
  readonly credential: () => Promise<string>;
  readonly fetcher?: typeof fetch;
}): GitFactWritePort {
  const fetcher = input.fetcher ?? fetch;
  async function request(method: string, path: string, body?: unknown): Promise<Record<string, unknown> | null> {
    const token = await input.credential();
    if (typeof token !== "string" || token.length === 0) return fail("GIT_FACT_CREDENTIAL_UNAVAILABLE");
    const response = await fetcher(`${API}${path}`, {
      method,
      headers: { Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28", Authorization: `Bearer ${token}`,
        ...(body === undefined ? {} : { "Content-Type": "application/json" }) },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      redirect: "error",
      signal: AbortSignal.timeout(10_000),
    });
    if (response.status === 404 && method === "GET" && path.startsWith("/contents/")) return null;
    if (!response.ok) return fail(`GIT_FACT_HTTP_${response.status}`);
    const length = Number(response.headers.get("content-length"));
    if (Number.isFinite(length) && length > MAX_REPLY_BYTES) return fail("GIT_FACT_HTTP_REPLY_TOO_LARGE");
    const bytes = new Uint8Array(await response.arrayBuffer());
    if (bytes.length > MAX_REPLY_BYTES) return fail("GIT_FACT_HTTP_REPLY_TOO_LARGE");
    let value: unknown;
    try { value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes)); }
    catch { return fail("GIT_FACT_HTTP_REPLY_INVALID"); }
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
      return fail("GIT_FACT_HTTP_REPLY_INVALID");
    }
    return value as Record<string, unknown>;
  }
  function sha(value: unknown): string {
    if (typeof value !== "string" || !SHA.test(value)) return fail("GIT_FACT_HTTP_SHA_INVALID");
    return value;
  }
  const ref = R2_TEST_LEDGER.branch.split("/").map(encodeURIComponent).join("/");
  const port: GitFactWritePort = {
    repository: R2_TEST_LEDGER.repository,
    branch: R2_TEST_LEDGER.branch,
    assertCurrentAuthority: input.currentAuthority,
    async readHead() {
      const reference = await request("GET", `/git/ref/heads/${ref}`);
      const object = reference?.object;
      if (typeof object !== "object" || object === null || Array.isArray(object)) {
        return fail("GIT_FACT_REF_INVALID");
      }
      const commit = sha((object as Record<string, unknown>).sha);
      const value = await request("GET", `/git/commits/${commit}`);
      const tree = value?.tree;
      if (typeof tree !== "object" || tree === null || Array.isArray(tree)) {
        return fail("GIT_FACT_COMMIT_INVALID");
      }
      return { commit, tree: sha((tree as Record<string, unknown>).sha) };
    },
    async readPath(commit, path) {
      const encoded = path.split("/").map(encodeURIComponent).join("/");
      const value = await request("GET", `/contents/${encoded}?ref=${commit}`);
      return value === null ? null : sha(value.sha);
    },
    async createBlob(bytes) {
      const value = await request("POST", "/git/blobs", {
        content: Buffer.from(bytes).toString("base64"), encoding: "base64",
      });
      return sha(value?.sha);
    },
    async createTree(baseTree, path, blob) {
      const value = await request("POST", "/git/trees", {
        base_tree: baseTree, tree: [{ path, mode: "100644", type: "blob", sha: blob }],
      });
      return sha(value?.sha);
    },
    async createCommit(parent, tree, message) {
      const value = await request("POST", "/git/commits", { message, tree, parents: [parent] });
      return sha(value?.sha);
    },
    async updateRef(commit) {
      const value = await request("PATCH", `/git/refs/heads/${ref}`, { sha: commit, force: false });
      const object = value?.object;
      if (typeof object !== "object" || object === null || Array.isArray(object) ||
          sha((object as Record<string, unknown>).sha) !== commit) {
        return fail("GIT_FACT_REF_UPDATE_UNCONFIRMED");
      }
    },
  };
  return Object.freeze(port);
}
