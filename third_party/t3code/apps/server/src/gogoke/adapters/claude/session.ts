import { CLAUDE_PINNED_VERSION, ClaudeProtocolError, encodeClaudeUserInput, type ClaudeFrame } from "./protocol.ts";

export type ClaudeSessionStatus = "idle" | "running" | "unknown";
export interface ClaudeTurnTerminal {
  readonly sessionId: string;
  readonly status: "completed" | "failed";
  readonly error: boolean;
  readonly subtype: string | null;
}
export interface ClaudeSessionCapabilities {
  readonly driver: "claude";
  readonly pinnedVersion: typeof CLAUDE_PINNED_VERSION;
  readonly runtimeEvidence: "UNKNOWN" | "STREAM_EVENTS_OBSERVED";
  readonly observedEventTypes: readonly string[];
  readonly features: Readonly<{
    resume: "SOURCE_PRESENT_RUNTIME_UNVERIFIED";
    sameProcessInput: "SOURCE_PRESENT_RUNTIME_UNVERIFIED";
    inTurnSteer: "UNVERIFIED_NO_RECEIPT";
    nativeQuestionCard: "UNSUPPORTED_BY_ADAPTER";
    manualCompaction: "UNSUPPORTED_BY_ADAPTER";
    automaticCompaction: "DOCUMENTED_RUNTIME_UNVERIFIED";
    memoryOffLaunch: "SOURCE_PRESENT_RUNTIME_UNVERIFIED";
    lpac: "HOST_ENFORCED_REQUIRED";
  }>;
}

/** Session state only. The host owns process transport, admission and durable receipts. */
export class ClaudeStreamSession {
  private sessionIdValue: string | null = null;
  private statusValue: ClaudeSessionStatus = "idle";
  private readonly observed = new Set<string>();
  private modelValue: string | null = null;
  private expectedSessionId: string | null = null;
  private terminalValue: ClaudeTurnTerminal | null = null;

  constructor(observedVersion: string) {
    if (observedVersion !== CLAUDE_PINNED_VERSION)
      throw new ClaudeProtocolError("VERSION_MISMATCH", `expected ${CLAUDE_PINNED_VERSION}`);
  }

  get sessionId(): string | null { return this.sessionIdValue; }
  get status(): ClaudeSessionStatus { return this.statusValue; }
  get model(): string | null { return this.modelValue; }
  get lastTerminal(): ClaudeTurnTerminal | null { return this.terminalValue; }

  capabilityReport(): ClaudeSessionCapabilities {
    return Object.freeze({
      driver: "claude",
      pinnedVersion: CLAUDE_PINNED_VERSION,
      runtimeEvidence: this.observed.size === 0 ? "UNKNOWN" : "STREAM_EVENTS_OBSERVED",
      observedEventTypes: Object.freeze([...this.observed].sort()),
      features: Object.freeze({
        resume: "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
        sameProcessInput: "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
        inTurnSteer: "UNVERIFIED_NO_RECEIPT",
        nativeQuestionCard: "UNSUPPORTED_BY_ADAPTER",
        manualCompaction: "UNSUPPORTED_BY_ADAPTER",
        automaticCompaction: "DOCUMENTED_RUNTIME_UNVERIFIED",
        memoryOffLaunch: "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
        lpac: "HOST_ENFORCED_REQUIRED",
      }),
    });
  }

  observe(frame: ClaudeFrame): ClaudeTurnTerminal | null {
    this.observed.add(frame.kind === "other" ? `other:${frame.type ?? "unknown"}` : frame.kind);
    if (frame.kind === "init") {
      if (this.expectedSessionId !== null && this.expectedSessionId !== frame.sessionId)
        throw new ClaudeProtocolError("SESSION_BINDING", "resumed CLI returned a different native session");
      if (this.sessionIdValue !== null && this.sessionIdValue !== frame.sessionId)
        throw new ClaudeProtocolError("SESSION_CHANGED", "init event changed the active session");
      this.sessionIdValue = frame.sessionId;
      this.modelValue = frame.model;
      return null;
    }
    if (frame.kind === "result") {
      if (frame.sessionId !== null && this.sessionIdValue !== null && frame.sessionId !== this.sessionIdValue)
        throw new ClaudeProtocolError("SESSION_CHANGED", "result event belongs to another session");
      if (this.sessionIdValue === null) throw new ClaudeProtocolError("SESSION_BINDING", "result arrived before init");
      const terminal = Object.freeze({ sessionId: this.sessionIdValue,
        status: frame.isError || (frame.subtype !== null && frame.subtype !== "success") ? "failed" as const : "completed" as const,
        error: frame.isError, subtype: frame.subtype });
      this.terminalValue = terminal;
      this.statusValue = "idle";
      return terminal;
    }
    return null;
  }

  /** Resume is a new pinned process with an explicit native session ID. */
  resumeArgs(sessionId: string): readonly string[] {
    if (this.statusValue === "running") throw new ClaudeProtocolError("INVALID_STATE", "cannot resume an active process");
    if (typeof sessionId !== "string" || sessionId.length === 0 || sessionId.includes("\0") || /[\\/]/u.test(sessionId))
      throw new ClaudeProtocolError("INVALID_INPUT", "sessionId must be an opaque native ID");
    if (this.sessionIdValue !== null && this.sessionIdValue !== sessionId)
      throw new ClaudeProtocolError("SESSION_BINDING", "resume ID differs from the current session");
    this.expectedSessionId = sessionId;
    return Object.freeze(["--resume", sessionId]);
  }

  /** Start a turn on this initialized process. The caller records UNKNOWN until an assistant event arrives. */
  startTurn(text: string): Uint8Array {
    if (this.statusValue !== "idle" || this.sessionIdValue === null)
      throw new ClaudeProtocolError("INVALID_STATE", "session is not initialized and idle");
    this.statusValue = "running";
    return encodeClaudeUserInput(text);
  }

  /** Appending input during a turn has no vendor receipt and may race the turn boundary. */
  appendDuringTurn(expectedTurnId: string, activeTurnId: string, text: string): Readonly<{ readonly status: "UNKNOWN"; readonly bytes: Uint8Array }> {
    if (this.statusValue !== "running") throw new ClaudeProtocolError("INVALID_STATE", "no active turn");
    if (!expectedTurnId || expectedTurnId !== activeTurnId)
      throw new ClaudeProtocolError("STALE_TURN", "append target is not the active host turn");
    return Object.freeze({ status: "UNKNOWN", bytes: encodeClaudeUserInput(text) });
  }
}
