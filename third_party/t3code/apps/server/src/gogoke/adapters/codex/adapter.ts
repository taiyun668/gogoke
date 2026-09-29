import { CODEX_PINNED_VERSION, CodexProtocolError } from "./protocol.ts";

type RecordValue = Record<string, unknown>;
const record = (value: unknown): value is RecordValue =>
  value !== null && typeof value === "object" && !Array.isArray(value);

export type CodexMethod = "thread/resume" | "turn/steer" | "thread/inject_items" | "thread/compact/start";

/** H owns process, authentication, binary pin, session custody, and request ID correlation. */
export interface CodexAppServerTransport {
  request(method: CodexMethod, params: Readonly<Record<string, unknown>>): Promise<unknown>;
}

export interface CodexCapabilityReport {
  readonly driver: "codex";
  readonly pinnedVersion: typeof CODEX_PINNED_VERSION;
  readonly protocolSource: "openai/codex rust-v0.149.0";
  readonly runtimeEvidence: "UNKNOWN";
  readonly methods: Readonly<Record<CodexMethod, "SOURCE_PRESENT_RUNTIME_UNVERIFIED">>;
  readonly features: Readonly<Record<"resume" | "inTurnSteer" | "appendWithoutTurn" |
    "nativeQuestionCard" | "manualCompaction" | "memoryOffLaunch", "SOURCE_PRESENT_RUNTIME_UNVERIFIED">>;
}

/** Source presence never means an installed binary or login was observed. */
export const CODEX_SOURCE_CAPABILITIES: CodexCapabilityReport = Object.freeze({
  driver: "codex",
  pinnedVersion: CODEX_PINNED_VERSION,
  protocolSource: "openai/codex rust-v0.149.0",
  runtimeEvidence: "UNKNOWN",
  methods: Object.freeze({
    "thread/resume": "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
    "turn/steer": "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
    "thread/inject_items": "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
    "thread/compact/start": "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
  }),
  features: Object.freeze({
    resume: "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
    inTurnSteer: "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
    appendWithoutTurn: "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
    nativeQuestionCard: "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
    manualCompaction: "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
    memoryOffLaunch: "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
  }),
});

/** Host must supply a separate instance CODEX_HOME and verify effective config. */
export function codexMemoryOffAppServerArgs(): readonly string[] {
  return Object.freeze([
    "-c", "features.memories=false",
    "-c", "memories.generate_memories=false",
    "-c", "memories.use_memories=false",
    "app-server",
  ]);
}

const nonempty = (value: string, field: string): string => {
  if (typeof value !== "string" || value.length === 0 || value.includes("\0")) {
    throw new CodexProtocolError("INVALID_INPUT", field);
  }
  return value;
};
const emptyResult = (value: unknown, method: string): void => {
  if (!record(value) || Object.keys(value).length !== 0) {
    throw new CodexProtocolError("INVALID_RESPONSE", `${method} must return {}`);
  }
};

/** Protocol actions only. No fallback creates a fresh thread or changes target. */
export class Codex0149Adapter {
  private readonly transport: CodexAppServerTransport;
  constructor(transport: CodexAppServerTransport, observedVersion: string) {
    if (observedVersion !== CODEX_PINNED_VERSION) {
      throw new CodexProtocolError("VERSION_MISMATCH", `expected ${CODEX_PINNED_VERSION}`);
    }
    this.transport = transport;
  }

  async resume(threadId: string): Promise<{ readonly status: "resumed"; readonly threadId: string }> {
    nonempty(threadId, "threadId");
    const result = await this.transport.request("thread/resume", { threadId });
    if (!record(result) || !record(result.thread) || result.thread.id !== threadId) {
      throw new CodexProtocolError("INVALID_RESPONSE", "thread/resume returned a different thread");
    }
    return Object.freeze({ status: "resumed", threadId });
  }

  async steer(threadId: string, expectedTurnId: string, text: string): Promise<{ readonly status: "accepted"; readonly turnId: string }> {
    nonempty(threadId, "threadId"); nonempty(expectedTurnId, "expectedTurnId"); nonempty(text, "text");
    const result = await this.transport.request("turn/steer", {
      threadId, expectedTurnId, input: [{ type: "text", text }],
    });
    if (!record(result) || result.turnId !== expectedTurnId) {
      throw new CodexProtocolError("INVALID_RESPONSE", "turn/steer did not confirm the active turn");
    }
    return Object.freeze({ status: "accepted", turnId: expectedTurnId });
  }

  async appendWithoutTurn(threadId: string, text: string): Promise<{ readonly status: "append-acknowledged" }> {
    nonempty(threadId, "threadId"); nonempty(text, "text");
    const result = await this.transport.request("thread/inject_items", {
      threadId,
      items: [{ type: "message", role: "user", content: [{ type: "input_text", text }] }],
    });
    emptyResult(result, "thread/inject_items");
    return Object.freeze({ status: "append-acknowledged" });
  }

  async requestCompaction(threadId: string): Promise<{ readonly status: "requested-not-completed" }> {
    nonempty(threadId, "threadId");
    const result = await this.transport.request("thread/compact/start", { threadId });
    emptyResult(result, "thread/compact/start");
    return Object.freeze({ status: "requested-not-completed" });
  }
}
