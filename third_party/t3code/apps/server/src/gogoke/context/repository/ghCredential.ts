import { spawnSync } from "node:child_process";

export class GhCredentialError extends Error {
  override readonly name = "GhCredentialError";
  readonly code: string;
  constructor(code: string) { super(code); this.code = code; }
}

type GhRunner = (args: readonly string[]) => string;

const runGh: GhRunner = (args) => {
  const result = spawnSync("gh", [...args], {
    encoding: "utf8", timeout: 10_000, maxBuffer: 4096, windowsHide: true,
    stdio: ["ignore", "pipe", "ignore"],
  });
  if (result.error !== undefined || result.status !== 0 || typeof result.stdout !== "string") {
    throw new GhCredentialError("GH_AUTH_UNAVAILABLE");
  }
  return result.stdout.trim();
};

/** Read-only public GitHub fetches may use the current gh credential for rate limits. */
export function currentGhToken(runner: GhRunner = runGh): string {
  const token = runner(["auth", "token", "--hostname", "github.com"]);
  if (token.length < 20 || token.includes("\n") || token.includes("\r")) {
    throw new GhCredentialError("GH_AUTH_UNAVAILABLE");
  }
  return token;
}

/** Uses the existing local gh login. Credentials never enter argv or logs. */
export function createR2GhCredentialAccess(
  currentNativeAdmission: () => Promise<void>,
  runner: GhRunner = runGh,
): { readonly assertCurrentAuthority: () => Promise<void>; readonly credential: () => Promise<string> } {
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
