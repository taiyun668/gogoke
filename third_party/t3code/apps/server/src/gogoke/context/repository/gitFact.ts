import { createHash } from "node:crypto";

const SHA = /^[0-9a-f]{40}(?:[0-9a-f]{24})?$/u;
const HASH = /^(?:sha256:)?[0-9a-f]{64}$/u;
const REPOSITORY = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u;
const MAX_RESPONSE_BYTES = 96 * 1024;
const MAX_FACT_BYTES = 32 * 1024;
const MAX_PR_FILE_PAGES = 10;

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

/** These values must come from current product authority, never from the proposed fact. */
export interface GitHubFactAcceptancePolicy {
  readonly repository: string;
  readonly targetBranch: string;
  readonly mergeActor: string;
}

export interface AcceptedGitHubFactReadback {
  readonly state: "PR_MERGE_ACCEPTED_FACT_VERIFIED";
  readonly pullNumber: number;
  readonly mergedAt: string;
  readonly mergeCommit: string;
  readonly mergedBy: string;
  readonly fact: GitFactReadback;
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
  authorizationToken?: string,
): Promise<GitFactReadback> {
  const exact = validate(coordinate, authorizedRepository);
  if (authorizationToken !== undefined &&
      (authorizationToken.length < 20 || authorizationToken.includes("\n") ||
       authorizationToken.includes("\r"))) return reject("GIT_FACT_CREDENTIAL_INVALID");
  const [owner, repo] = exact.repository.split("/");
  const path = exact.path.split("/").map(encodeURIComponent).join("/");
  const url = `https://api.github.com/repos/${encodeURIComponent(owner!)}/${encodeURIComponent(repo!)}/contents/${path}?ref=${exact.commit}`;
  const response = await fetcher(url, {
    headers: { Accept: "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28",
      ...(authorizationToken === undefined ? {} : { Authorization: `Bearer ${authorizationToken}` }) },
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

/** Verify GitHub's merged PR record and read the fact at that exact merge commit. */
export async function readAcceptedGitHubFact(
  coordinate: GitFactCoordinate,
  pullNumber: number,
  policy: GitHubFactAcceptancePolicy,
  fetcher: typeof fetch = fetch,
  authorizationToken?: string,
): Promise<AcceptedGitHubFactReadback> {
  const exact = validate(coordinate, policy.repository);
  if (!Number.isSafeInteger(pullNumber) || pullNumber <= 0 ||
      !REPOSITORY.test(policy.repository) ||
      policy.targetBranch.length === 0 || policy.targetBranch.length > 255 ||
      /[\u0000-\u001f\u007f]/u.test(policy.targetBranch) ||
      !/^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38}[A-Za-z0-9])?$/u.test(policy.mergeActor)) {
    return reject("GIT_FACT_ACCEPTANCE_POLICY_INVALID");
  }
  if (authorizationToken !== undefined &&
      (authorizationToken.length < 20 || authorizationToken.includes("\n") ||
       authorizationToken.includes("\r"))) return reject("GIT_FACT_CREDENTIAL_INVALID");
  const [owner, repo] = policy.repository.split("/");
  const url = `https://api.github.com/repos/${encodeURIComponent(owner!)}/${encodeURIComponent(repo!)}/pulls/${pullNumber}`;
  const response = await fetcher(url, {
    headers: { Accept: "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28",
      ...(authorizationToken === undefined ? {} : { Authorization: `Bearer ${authorizationToken}` }) },
    redirect: "error",
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) {
    throw new GitFactReadbackError("GIT_FACT_ACCEPTANCE_READ_FAILED",
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
  catch { return reject("GIT_FACT_ACCEPTANCE_RESPONSE_INVALID"); }
  if (typeof body !== "object" || body === null || Array.isArray(body)) {
    return reject("GIT_FACT_ACCEPTANCE_RESPONSE_INVALID");
  }
  const pull = body as Record<string, unknown>;
  const base = typeof pull.base === "object" && pull.base !== null && !Array.isArray(pull.base) ?
    pull.base as Record<string, unknown> : null;
  const baseRepo = base !== null && typeof base.repo === "object" && base.repo !== null &&
    !Array.isArray(base.repo) ? base.repo as Record<string, unknown> : null;
  const actor = typeof pull.merged_by === "object" && pull.merged_by !== null &&
    !Array.isArray(pull.merged_by) ? pull.merged_by as Record<string, unknown> : null;
  if (pull.number !== pullNumber || baseRepo?.full_name !== policy.repository ||
      base?.ref !== policy.targetBranch || pull.state !== "closed" ||
      pull.merged !== true || pull.draft !== false ||
      typeof pull.merged_at !== "string" ||
      !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/u.test(pull.merged_at) ||
      !Number.isFinite(Date.parse(pull.merged_at)) ||
      new Date(pull.merged_at).toISOString().replace(".000Z", "Z") !== pull.merged_at ||
      actor?.login !== policy.mergeActor ||
      pull.merge_commit_sha !== exact.commit || !/^[0-9a-f]{40}$/u.test(exact.commit)) {
    return reject("GIT_FACT_ACCEPTANCE_MISMATCH");
  }
  let matchingFiles = 0;
  for (let page = 1; page <= MAX_PR_FILE_PAGES; page += 1) {
    const filesResponse = await fetcher(`${url}/files?per_page=100&page=${page}`, {
      headers: { Accept: "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28",
        ...(authorizationToken === undefined ? {} : { Authorization: `Bearer ${authorizationToken}` }) },
      redirect: "error",
      signal: AbortSignal.timeout(10_000),
    });
    if (!filesResponse.ok) {
      throw new GitFactReadbackError("GIT_FACT_PR_FILES_READ_FAILED",
        `http=${filesResponse.status} remaining=${filesResponse.headers.get("x-ratelimit-remaining") ?? "unknown"}`);
    }
    const filesLength = Number(filesResponse.headers.get("content-length"));
    if (Number.isFinite(filesLength) && filesLength > MAX_RESPONSE_BYTES) {
      return reject("GIT_FACT_RESPONSE_TOO_LARGE");
    }
    const filesBytes = new Uint8Array(await filesResponse.arrayBuffer());
    if (filesBytes.length > MAX_RESPONSE_BYTES) return reject("GIT_FACT_RESPONSE_TOO_LARGE");
    let files: unknown;
    try { files = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(filesBytes)); }
    catch { return reject("GIT_FACT_PR_FILES_RESPONSE_INVALID"); }
    if (!Array.isArray(files) || files.length > 100) return reject("GIT_FACT_PR_FILES_RESPONSE_INVALID");
    for (const entry of files) {
      if (typeof entry !== "object" || entry === null || Array.isArray(entry) ||
          typeof entry.filename !== "string" || typeof entry.status !== "string") {
        return reject("GIT_FACT_PR_FILES_RESPONSE_INVALID");
      }
      if (entry.filename === exact.path) {
        matchingFiles += 1;
        if (matchingFiles > 1 || !["added", "modified", "renamed"].includes(entry.status)) {
          return reject("GIT_FACT_ACCEPTANCE_MISMATCH");
        }
      }
    }
    if (files.length < 100) {
      if (filesResponse.headers.get("link")?.includes('rel="next"')) {
        return reject("GIT_FACT_PR_FILES_RESPONSE_INVALID");
      }
      break;
    }
    if (page === MAX_PR_FILE_PAGES) return reject("GIT_FACT_PR_FILES_PAGE_LIMIT");
  }
  if (matchingFiles !== 1) return reject("GIT_FACT_ACCEPTANCE_MISMATCH");
  const fact = await readGitHubFact(exact, policy.repository, fetcher, authorizationToken);
  return Object.freeze({
    state: "PR_MERGE_ACCEPTED_FACT_VERIFIED" as const,
    pullNumber,
    mergedAt: pull.merged_at,
    mergeCommit: exact.commit,
    mergedBy: policy.mergeActor,
    fact,
  });
}
