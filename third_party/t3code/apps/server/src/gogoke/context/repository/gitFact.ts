import { createHash } from "node:crypto";

const SHA = /^[0-9a-f]{40}(?:[0-9a-f]{24})?$/u;
const HASH = /^(?:sha256:)?[0-9a-f]{64}$/u;
const REPOSITORY = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u;
const MAX_RESPONSE_BYTES = 96 * 1024;
const MAX_FACT_BYTES = 32 * 1024;

export interface GitFactCoordinate {
  readonly repository: string;
  readonly commit: string;
  readonly path: string;
  readonly contentHash: string;
}

export interface GitFactReadback {
  readonly state: "COMMITTED_BYTES_VERIFIED_NOT_ADOPTED";
  readonly coordinate: GitFactCoordinate;
  readonly gitBlob: string;
  readonly bytes: Uint8Array;
}

export class GitFactReadbackError extends Error {
  override readonly name = "GitFactReadbackError";
  readonly code: string;
  constructor(code: string, detail?: string) {
    super(detail === undefined ? code : `${code}: ${detail}`);
    this.code = code;
  }
}

function reject(code: string): never { throw new GitFactReadbackError(code); }

function validate(coordinate: GitFactCoordinate, authorizedRepository: string): GitFactCoordinate {
  if (!REPOSITORY.test(authorizedRepository) || coordinate.repository !== authorizedRepository ||
      !SHA.test(coordinate.commit) || !HASH.test(coordinate.contentHash) ||
      coordinate.path.length === 0 || coordinate.path.length > 512 ||
      coordinate.path.startsWith("/") || coordinate.path.includes("\\") ||
      coordinate.path.split("/").some((part) => part === "" || part === "." || part === "..")) {
    return reject("INVALID_OR_UNAUTHORIZED_GIT_FACT_COORDINATE");
  }
  return Object.freeze({
    repository: coordinate.repository,
    commit: coordinate.commit,
    path: coordinate.path,
    contentHash: coordinate.contentHash,
  });
}

/** Public fact readback. Caller supplies the repository from current Project authority. */
export async function readGitHubFact(
  coordinate: GitFactCoordinate,
  authorizedRepository: string,
  fetcher: typeof fetch = fetch,
): Promise<GitFactReadback> {
  const exact = validate(coordinate, authorizedRepository);
  const [owner, repo] = exact.repository.split("/");
  const path = exact.path.split("/").map(encodeURIComponent).join("/");
  const url = `https://api.github.com/repos/${encodeURIComponent(owner!)}/${encodeURIComponent(repo!)}/contents/${path}?ref=${exact.commit}`;
  const response = await fetcher(url, {
    headers: { Accept: "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28" },
    redirect: "error",
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) {
    throw new GitFactReadbackError("GIT_FACT_READ_FAILED",
      `http=${response.status} remaining=${response.headers.get("x-ratelimit-remaining") ?? "unknown"}`);
  }
  const contentLength = Number(response.headers.get("content-length"));
  if (Number.isFinite(contentLength) && contentLength > MAX_RESPONSE_BYTES) {
    return reject("GIT_FACT_RESPONSE_TOO_LARGE");
  }
  const bodyBytes = new Uint8Array(await response.arrayBuffer());
  if (bodyBytes.length > MAX_RESPONSE_BYTES) return reject("GIT_FACT_RESPONSE_TOO_LARGE");
  let body: unknown;
  try { body = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bodyBytes)); }
  catch { return reject("GIT_FACT_RESPONSE_INVALID"); }
  if (typeof body !== "object" || body === null || Array.isArray(body)) {
    return reject("GIT_FACT_RESPONSE_INVALID");
  }
  const item = body as Record<string, unknown>;
  const encoded = typeof item.content === "string" ? item.content.replace(/\s/gu, "") : "";
  if (item.type !== "file" || item.path !== exact.path ||
      typeof item.sha !== "string" || !/^[0-9a-f]{40}$/u.test(item.sha) ||
      item.encoding !== "base64" || typeof item.content !== "string" ||
      !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/u.test(encoded)) {
    return reject("GIT_FACT_RESPONSE_INVALID");
  }
  const content = Buffer.from(encoded, "base64");
  if (content.length > MAX_FACT_BYTES || item.size !== content.length) {
    return reject("GIT_FACT_BYTES_INVALID");
  }
  const gitBlob = createHash("sha1").update(`blob ${content.length}\0`).update(content).digest("hex");
  if (gitBlob !== item.sha) return reject("GIT_FACT_BLOB_MISMATCH");
  const actual = createHash("sha256").update(content).digest("hex");
  if (actual !== exact.contentHash.replace(/^sha256:/u, "")) {
    return reject("GIT_FACT_HASH_MISMATCH");
  }
  return Object.freeze({
    state: "COMMITTED_BYTES_VERIFIED_NOT_ADOPTED" as const,
    coordinate: exact,
    gitBlob: item.sha,
    bytes: new Uint8Array(content),
  });
}
