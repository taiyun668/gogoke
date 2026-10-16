import { join } from "node:path";

/**
 * OpenCode ACP adapter contract pinned to upstream release v1.18.32.
 *
 * ACP framing and process ownership belong to the H/native host integration;
 * this module only validates protocol values and never starts OpenCode.
 */
export const OPENCODE_PINNED_VERSION = "1.18.32" as const;
export const OPENCODE_SOURCE_COMMIT = "545f51d" as const;
export const OPENCODE_ACP_VERSION = 1 as const;

export class OpenCodeProtocolError extends Error {
  override readonly name = "OpenCodeProtocolError";
  readonly code: string;

  constructor(code: string, detail: string, cause?: unknown) {
    super(`${code}: ${detail}`, cause === undefined ? undefined : { cause });
    this.code = code;
  }
}

export type OpenCodeAcpMethod =
  | "initialize"
  | "session/new"
  | "session/resume"
  | "session/prompt"
  | "session/cancel";

/**
 * Transport correlation, byte framing, process custody and cancellation are
 * supplied by the host. A notification must preserve ACP's one-way semantics.
 */
export interface OpenCodeAcpTransport {
  request(method: Exclude<OpenCodeAcpMethod, "session/cancel">,
    params: Readonly<Record<string, unknown>>): Promise<unknown>;
  notify(method: "session/cancel", params: Readonly<Record<string, unknown>>): Promise<void>;
  setEventHandler?(handler: (event: unknown) => void): void;
}

export interface OpenCodeAcpEvent {
  readonly provider: "opencode";
  readonly nativeSessionId: string;
  readonly method: string;
  readonly payload: unknown;
}

export interface OpenCodeAcpNotification {
  readonly method: string;
  readonly params: unknown;
}

export interface OpenCodeHostedLoginCommand {
  readonly methodId: "opencode-login";
  readonly args: readonly ["auth", "login"];
  readonly label: "OpenCode Login";
}

export type OpenCodeStopReason =
  | "end_turn"
  | "max_tokens"
  | "max_turn_requests"
  | "refusal"
  | "cancelled"
  | "error";

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function requireText(value: unknown, field: string): string {
  if (typeof value !== "string" || value.length === 0 || value.includes("\0")) {
    throw new OpenCodeProtocolError("INVALID_INPUT", field);
  }
  return value;
}

export function responseRecord(value: unknown, method: string): Record<string, unknown> {
  if (!isRecord(value)) throw new OpenCodeProtocolError("INVALID_RESPONSE", method);
  return value;
}

export function parseStopReason(value: unknown): OpenCodeStopReason {
  if (value === "end_turn" || value === "max_tokens" || value === "max_turn_requests" ||
      value === "refusal" || value === "cancelled" || value === "error") return value;
  throw new OpenCodeProtocolError("INVALID_RESPONSE", "session/prompt.stopReason");
}

/** Instance-private config roots; this does not disable native OpenCode session history. */
export function opencodeInstanceEnvironment(home: string): Readonly<Record<string, string>> {
  const root = requireText(home, "home");
  return Object.freeze({
    HOME: root,
    USERPROFILE: root,
    APPDATA: join(root, "AppData", "Roaming"),
    LOCALAPPDATA: join(root, "AppData", "Local"),
    XDG_CONFIG_HOME: join(root, ".config"),
    XDG_DATA_HOME: join(root, ".local", "share"),
    XDG_CACHE_HOME: join(root, ".cache"),
    XDG_STATE_HOME: join(root, ".local", "state"),
    OPENCODE_CONFIG_DIR: join(root, ".opencode"),
    OPENCODE_CONFIG: join(root, ".opencode", "opencode.json"),
    OPENCODE_CONFIG_CONTENT: "{}",
    OPENCODE_DISABLE_CLAUDE_CODE: "1",
    OPENCODE_DISABLE_CLAUDE_CODE_PROMPT: "1",
    OPENCODE_DISABLE_CLAUDE_CODE_SKILLS: "1",
  });
}

/** ACP command for a host-owned, pinned OpenCode executable. */
export function opencodeAcpArguments(): ReadonlyArray<string> {
  return Object.freeze(["acp"]);
}

/** Command arguments only; the host supplies its already-pinned executable and terminal. */
export function opencodeOfficialLoginArguments(): readonly ["auth", "login"] {
  return Object.freeze(["auth", "login"]);
}
