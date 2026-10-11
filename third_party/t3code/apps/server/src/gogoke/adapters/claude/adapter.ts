import { CLAUDE_PINNED_VERSION, ClaudeProtocolError } from "./protocol.ts";

export interface ClaudeHostBoundary {
  /** This fact must come from H's native admission receipt, never a UI/caller flag. */
  readonly executionBoundary: "LPAC";
  readonly admissionReceiptId: string;
  readonly permissionTier: string;
}

export interface ClaudeLaunchInput {
  readonly observedVersion: string;
  readonly boundary: ClaudeHostBoundary;
  readonly instanceHome: string;
  readonly configDirectory: string;
  readonly workingDirectory: string;
  readonly sessionId?: string;
  readonly hostEnvironment: NodeJS.ProcessEnv;
}

export interface ClaudeLaunchPlan {
  readonly args: readonly string[];
  readonly cwd: string;
  readonly env: NodeJS.ProcessEnv;
  readonly executionBoundary: "LPAC";
  readonly permissionTier: string;
  readonly admissionReceiptId: string;
}

const REQUIRED_SYSTEM_ENV = ["PATH", "SystemRoot", "WINDIR", "COMSPEC", "PATHEXT"] as const;
const SECRET_ENV = /(?:^|_)(?:API_?KEY|AUTH_?TOKEN|ACCESS_?TOKEN|SECRET|PASSWORD|CREDENTIALS?|PRIVATE_?KEY)(?:$|_)/iu;

function nonempty(value: string, field: string): string {
  if (typeof value !== "string" || value.length === 0 || value.includes("\0"))
    throw new ClaudeProtocolError("INVALID_INPUT", field);
  return value;
}

/** Login is a separate, explicit host operation; this plan never reads credentials. */
export function claudeLoginCommand(): readonly string[] {
  return Object.freeze(["auth", "login", "--claudeai"]);
}

export function claudeLoginStatusCommand(): readonly string[] {
  return Object.freeze(["auth", "status"]);
}

/** Build a model-session request only when H attests the required LPAC admission. */
export function buildClaudeLaunchPlan(input: ClaudeLaunchInput): ClaudeLaunchPlan {
  if (input.observedVersion !== CLAUDE_PINNED_VERSION)
    throw new ClaudeProtocolError("VERSION_MISMATCH", `expected ${CLAUDE_PINNED_VERSION}`);
  if (input.boundary?.executionBoundary !== "LPAC" ||
      typeof input.boundary.admissionReceiptId !== "string" || input.boundary.admissionReceiptId.length === 0 ||
      typeof input.boundary.permissionTier !== "string" || input.boundary.permissionTier.length === 0) {
    throw new ClaudeProtocolError("LPAC_ADMISSION_REQUIRED", "no ordinary-user model fallback");
  }
  const home = nonempty(input.instanceHome, "instanceHome");
  const config = nonempty(input.configDirectory, "configDirectory");
  const cwd = nonempty(input.workingDirectory, "workingDirectory");
  const args = ["--print", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"];
  if (input.sessionId !== undefined) {
    nonempty(input.sessionId, "sessionId");
    if (/[\\/]/u.test(input.sessionId)) throw new ClaudeProtocolError("INVALID_INPUT", "sessionId");
    args.push("--resume", input.sessionId);
  }

  const env: NodeJS.ProcessEnv = {};
  for (const key of REQUIRED_SYSTEM_ENV) {
    const value = input.hostEnvironment[key];
    if (value !== undefined && !SECRET_ENV.test(key)) env[key] = value;
  }
  env.HOME = home;
  env.USERPROFILE = home;
  env.CLAUDE_CONFIG_DIR = config;
  env.CLAUDE_CODE_DISABLE_AUTO_MEMORY = "1";
  env.APPDATA = `${home}\\AppData\\Roaming`;
  env.LOCALAPPDATA = `${home}\\AppData\\Local`;
  env.TEMP = `${home}\\Temp`;
  env.TMP = `${home}\\Temp`;

  return Object.freeze({ args: Object.freeze(args), cwd, env: Object.freeze(env),
    executionBoundary: "LPAC", permissionTier: input.boundary.permissionTier,
    admissionReceiptId: input.boundary.admissionReceiptId });
}

export const CLAUDE_LOGIN_REFERENCE = Object.freeze({
  commandSource: "gogo-party PROVIDER_LOGIN.claude; official docs describe browser login, /login and /status",
  automaticLogin: false,
  credentialsReadByHost: false,
  fixedCliOrSystemSettingsModified: false,
});
