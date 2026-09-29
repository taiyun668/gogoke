import { spawnSync } from "node:child_process";
import * as NodeFS from "node:fs";
import * as NodePath from "node:path";

export class GhCredentialError extends Error {
  override readonly name = "GhCredentialError";
  readonly code: string;
  constructor(code: string) {
    super(code);
    this.code = code;
  }
}

type GhRunner = (args: readonly string[]) => string;

function isWithin(
  pathApi: typeof NodePath | typeof NodePath.win32,
  root: string,
  value: string,
): boolean {
  const relative = pathApi.relative(root, value);
  return (
    relative === "" ||
    (relative !== ".." && !relative.startsWith(`..${pathApi.sep}`) && !pathApi.isAbsolute(relative))
  );
}

function trustedGhPaths(platform: NodeJS.Platform): string[] {
  if (platform === "win32") {
    return ["C:\\Program Files\\GitHub CLI\\gh.exe", "C:\\Program Files (x86)\\GitHub CLI\\gh.exe"];
  }
  return ["/usr/bin/gh", "/usr/local/bin/gh", "/bin/gh"];
}

type GhFilesystem = Pick<typeof NodeFS, "realpathSync" | "statSync">;

/** Use fixed system CLI locations; inherited PATH and ProgramFiles are not trust roots. */
export function resolveTrustedGhExecutable(
  platform: NodeJS.Platform = process.platform,
  cwd = process.cwd(),
  executablePath = process.execPath,
  fsApi: GhFilesystem = NodeFS,
): string {
  const pathApi = platform === "win32" ? NodePath.win32 : NodePath;
  const excluded =
    platform === "win32" ? [cwd, pathApi.dirname(executablePath), pathApi.dirname(cwd)] : [cwd];
  const excludedPaths = excluded.map((path) => pathApi.resolve(path));
  for (const candidate of trustedGhPaths(platform)) {
    const absolute = pathApi.resolve(candidate);
    if (excludedPaths.some((directory) => isWithin(pathApi, directory, absolute))) continue;
    try {
      const resolved = fsApi.realpathSync(absolute);
      const stat = fsApi.statSync(resolved);
      if (
        !stat.isFile() ||
        pathApi.normalize(resolved).toLowerCase() !== pathApi.normalize(absolute).toLowerCase() ||
        excludedPaths.some((directory) => isWithin(pathApi, directory, resolved))
      )
        continue;
      return resolved;
    } catch {
      // An absent fixed installation is unavailable; never search writable PATH entries.
    }
  }
  throw new GhCredentialError("GH_AUTH_UNAVAILABLE");
}

const runGh: GhRunner = (args) => {
  const executable = resolveTrustedGhExecutable();
  const result = spawnSync(executable, [...args], {
    encoding: "utf8",
    timeout: 10_000,
    maxBuffer: 4096,
    windowsHide: true,
    stdio: ["ignore", "pipe", "ignore"],
  });
  if (result.error !== undefined || result.status !== 0 || typeof result.stdout !== "string") {
    throw new GhCredentialError("GH_AUTH_UNAVAILABLE");
  }
  return result.stdout.trim();
};

/** Read-only public GitHub fetches may use the current gh credential for rate limits. */
export function currentGhToken(runner: GhRunner = runGh): string {
  // Actions supplies GH_TOKEN directly; the installed product uses the user's
  // existing gh login when that environment credential is absent.
  const token =
    runner === runGh && process.env.GH_TOKEN !== undefined
      ? process.env.GH_TOKEN
      : runner(["auth", "token", "--hostname", "github.com"]);
  if (token.length < 20 || token.includes("\n") || token.includes("\r")) {
    throw new GhCredentialError("GH_AUTH_UNAVAILABLE");
  }
  return token;
}

/** Public readback can fall back to GitHub's unauthenticated API when no gh login exists. */
export function currentGhTokenIfAvailable(runner: GhRunner = runGh): string | undefined {
  try {
    return currentGhToken(runner);
  } catch (error) {
    if (error instanceof GhCredentialError && error.code === "GH_AUTH_UNAVAILABLE")
      return undefined;
    throw error;
  }
}

/** Uses the existing local gh login. Credentials never enter argv or logs. */
export function createR2GhCredentialAccess(
  currentNativeAdmission: () => Promise<void>,
  runner: GhRunner = runGh,
): {
  readonly assertCurrentAuthority: () => Promise<void>;
  readonly credential: () => Promise<string>;
} {
  return Object.freeze({
    async assertCurrentAuthority() {
      await currentNativeAdmission();
      const login = runner(["api", "user", "--hostname", "github.com", "--jq", ".login"]);
      if (login !== "taiyun668") throw new GhCredentialError("GH_OWNER_IDENTITY_MISMATCH");
    },
    async credential() {
      return currentGhToken(runner);
    },
  });
}
