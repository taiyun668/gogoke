/**
 * Antigravity CLI evidence pin. The CLI implementation is distributed as a
 * signed binary; Google's public repository provides release metadata and
 * documentation, not the binary's implementation source.
 */
export const ANTIGRAVITY_PIN = Object.freeze({
  repository: "google-antigravity/antigravity-cli",
  version: "1.2.11",
  tag: "1.2.11",
  commit: "6dadd62",
  headlessDocumentation: "https://antigravity.google/docs/cli/headless/",
  release: "https://github.com/google-antigravity/antigravity-cli/releases/tag/1.2.11",
  transport: "newline-delimited-json",
} as const);

export class AntigravityProtocolError extends Error {
  override readonly name = "AntigravityProtocolError";
  readonly code: string;

  constructor(code: string, detail: string, cause?: unknown) {
    super(`${code}: ${detail}`, cause === undefined ? undefined : { cause });
    this.code = code;
  }
}

const nonempty = (value: string, field: string): string => {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value !== value.trim() ||
    value.includes("\0")
  ) {
    throw new AntigravityProtocolError("INVALID_INPUT", field);
  }
  return value;
};

/**
 * `agy` headless stream mode reads user events from stdin and writes NDJSON to
 * stdout. Conversation selection is explicit; there is deliberately no
 * `--continue` fallback because its workspace-local cache could select a
 * different conversation.
 */
export function antigravityHeadlessArgs(conversationId?: string): readonly string[] {
  return Object.freeze([
    ...(conversationId === undefined
      ? []
      : ["--conversation", nonempty(conversationId, "conversationId")]),
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
  ]);
}

/** Exact documented stdin shape. Prompts are written to stdin, not argv. */
export function antigravityUserEventLine(content: string): string {
  if (typeof content !== "string" || content.length === 0 || content.includes("\0")) {
    throw new AntigravityProtocolError(
      "INVALID_INPUT",
      "content must be a non-empty string without NUL",
    );
  }
  return `${JSON.stringify({ event: "user", message: { content } })}\n`;
}

/**
 * Build an isolated CLI environment from an already host-filtered baseline.
 * This only redirects local files. It cannot isolate Windows Credential
 * Manager or establish account identity; H/F must retain that UNKNOWN state.
 */
export function antigravityInstanceEnvironment(
  hostEnvironment: Readonly<Record<string, string | undefined>>,
  instanceHome: string,
): Readonly<Record<string, string>> {
  nonempty(instanceHome, "instanceHome");
  if (!/^(?:[A-Za-z]:[\\/]|\\\\|\/)/u.test(instanceHome)) {
    throw new AntigravityProtocolError("INVALID_INPUT", "instanceHome must be absolute");
  }
  const env: Record<string, string> = {};
  for (const [name, value] of Object.entries(hostEnvironment)) {
    if (
      value === undefined ||
      /(?:API[_-]?KEY|AUTH(?:ORIZATION)?|ACCESS[_-]?TOKEN|REFRESH[_-]?TOKEN|(?:^|[_-])(?:[A-Z]+[_-]?)?TOKEN(?:$|[_-])|SECRET|PASSWORD|CREDENTIAL|PRIVATE[_-]?KEY)/iu.test(
        name,
      )
    ) {
      continue;
    }
    env[name] = value;
  }
  // `agy` stores CLI history/settings beneath ~/.gemini. USERPROFILE is the
  // Windows home source used by its documented per-user layout. AppData is
  // local to the instance; the browser-facing login flow must not use this env.
  env.HOME = instanceHome;
  env.USERPROFILE = instanceHome;
  const separator = /^[A-Za-z]:[\\/]|^\\\\/u.test(instanceHome) ? "\\" : "/";
  const home = instanceHome.replace(/[\\/]+$/u, "");
  env.APPDATA = [home, "AppData", "Roaming"].join(separator);
  env.LOCALAPPDATA = [home, "AppData", "Local"].join(separator);
  return Object.freeze(env);
}

export interface AntigravityEventEnvelope {
  readonly event: string;
  readonly conversation_id?: string;
  readonly init?: Readonly<Record<string, unknown>>;
  readonly step_update?: Readonly<Record<string, unknown>>;
  readonly result?: Readonly<Record<string, unknown>>;
  readonly [key: string]: unknown;
}

export interface AntigravityCapabilityReport {
  readonly driver: "antigravity";
  readonly pinnedVersion: typeof ANTIGRAVITY_PIN.version;
  readonly upstreamCommit: typeof ANTIGRAVITY_PIN.commit;
  readonly implementationSource: "OFFICIAL_BINARY_CLOSED_SOURCE";
  readonly runtimeEvidence: "RESEARCH_1_2_11_PRESENT_ADAPTER_E2E_NOT_RUN";
  readonly headlessNdjson: "RESEARCHED_SUPPORTED";
  readonly conversationResume: "RESEARCHED_SUPPORTED_EXPLICIT_ID";
  readonly resumeAfterForcedMidTurnStop: "NOT_RUN";
  readonly inTurnSteer: "UNSUPPORTED";
  readonly hostInterruptThenResume: "REQUIRES_H_CONFIRMED_STOP_AND_SAME_CONVERSATION_ID";
  readonly nativeQuestionCard: "NOT_OBSERVED_UNSUPPORTED";
  readonly memoryOff: "UNSUPPORTED";
  readonly localHistoryIsolation: "INSTANCE_HOME_ONLY";
  readonly remoteOrAccountMemoryIsolation: "UNKNOWN";
  readonly instructionFiles: "WORKSPACE_AND_GLOBAL_RULES_MAY_BE_LOADED";
}

export const ANTIGRAVITY_CAPABILITIES: AntigravityCapabilityReport = Object.freeze({
  driver: "antigravity",
  pinnedVersion: ANTIGRAVITY_PIN.version,
  upstreamCommit: ANTIGRAVITY_PIN.commit,
  implementationSource: "OFFICIAL_BINARY_CLOSED_SOURCE",
  runtimeEvidence: "RESEARCH_1_2_11_PRESENT_ADAPTER_E2E_NOT_RUN",
  headlessNdjson: "RESEARCHED_SUPPORTED",
  conversationResume: "RESEARCHED_SUPPORTED_EXPLICIT_ID",
  resumeAfterForcedMidTurnStop: "NOT_RUN",
  inTurnSteer: "UNSUPPORTED",
  hostInterruptThenResume: "REQUIRES_H_CONFIRMED_STOP_AND_SAME_CONVERSATION_ID",
  nativeQuestionCard: "NOT_OBSERVED_UNSUPPORTED",
  memoryOff: "UNSUPPORTED",
  localHistoryIsolation: "INSTANCE_HOME_ONLY",
  remoteOrAccountMemoryIsolation: "UNKNOWN",
  instructionFiles: "WORKSPACE_AND_GLOBAL_RULES_MAY_BE_LOADED",
});
