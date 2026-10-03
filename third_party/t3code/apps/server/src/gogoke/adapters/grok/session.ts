import {
  GROK_BUILD_PINNED_VERSION,
  GROK_BUILD_PROTOCOL_VERSION,
  GrokProtocolError,
  type GrokCapabilityReport,
  type GrokFrame,
} from "./protocol.ts";

type JsonRecord = Record<string, unknown>;
const isRecord = (value: unknown): value is JsonRecord =>
  value !== null && typeof value === "object" && !Array.isArray(value);
const requiredString = (value: unknown, field: string): string => {
  if (typeof value !== "string" || value.length === 0 || value.includes("\0")) {
    throw new GrokProtocolError("INVALID_RESPONSE", field);
  }
  return value;
};
const responseObject = (value: unknown, method: string): JsonRecord => {
  if (!isRecord(value)) throw new GrokProtocolError("INVALID_RESPONSE", method);
  return value;
};

/** H owns process creation, binary identity, environment, LPAC and process custody. */
export interface GrokAcpTransport {
  request(method: string, params: Readonly<JsonRecord>): Promise<unknown>;
  notify(method: string, params: Readonly<JsonRecord>): Promise<void>;
}

export type GrokPromptStopReason = "end_turn" | "cancelled" | "max_turn_requests" | "unknown";
export interface GrokPromptReceipt {
  readonly provider: "grok-build";
  readonly sessionId: string;
  readonly stopReason: GrokPromptStopReason;
  readonly response: Readonly<JsonRecord>;
}
export interface GrokInterruptResumeReceipt {
  readonly provider: "grok-build";
  readonly sessionId: string;
  readonly interrupt: {
    readonly method: "session/cancel";
    readonly requestSent: true;
    readonly promptStopReason: "cancelled";
  };
  readonly resumed: GrokPromptReceipt;
}
export interface GrokRawObservation {
  readonly source: { readonly provider: "grok-build"; readonly version: typeof GROK_BUILD_PINNED_VERSION };
  readonly frame: GrokFrame;
}

type Phase = "new" | "initializing" | "ready" | "recovery-required" | "closed";

/**
 * Protocol state for the pinned public CLI. It deliberately does not launch a
 * process or treat requested configuration as evidence that the vendor used it.
 */
export class GrokBuild041Session {
  private readonly transport: GrokAcpTransport;
  private phaseValue: Phase = "new";
  private observedVersionValue: string | null = null;
  private sessionIdValue: string | null = null;
  private activePrompt: Promise<GrokPromptReceipt> | null = null;
  private uncertainMethodValue: string | null = null;
  private readonly observed = new Set<string>();
  private advertised = new Set<string>();

  constructor(transport: GrokAcpTransport) {
    this.transport = transport;
  }

  get phase(): Phase { return this.phaseValue; }
  get sessionId(): string | null { return this.sessionIdValue; }
  get uncertainMethod(): string | null { return this.uncertainMethodValue; }

  /** Only exact version and ACP fields returned by this process count as observations. */
  capabilityReport(): GrokCapabilityReport {
    const ready = this.phaseValue === "ready" || this.phaseValue === "recovery-required" || this.phaseValue === "closed";
    const methodState = (method: string, advertised = false): "OBSERVED" | "ADVERTISED" | "UNKNOWN" =>
      this.observed.has(method) ? "OBSERVED" : advertised ? "ADVERTISED" : "UNKNOWN";
    return Object.freeze({
      pinnedVersion: GROK_BUILD_PINNED_VERSION,
      observedVersion: this.observedVersionValue,
      versionEvidence: ready && this.observedVersionValue !== null ? "LIVE_INITIALIZE" : "NOT_RUN",
      acpInitialize: methodState("initialize"),
      sessionNew: methodState("session/new", this.advertised.has("session/new")),
      sessionLoad: methodState("session/load", this.advertised.has("loadSession")),
      sessionResume: methodState("session/load", this.advertised.has("resume")),
      sessionCancel: methodState("session/cancel", this.advertised.has("cancel")),
      steer: "UNKNOWN",
      nativeQuestionCard: "UNKNOWN",
      memoryOff: "NOT_RUN",
      memoryCrossProjectIsolation: "NOT_RUN",
      observedFeatures: Object.freeze([...this.observed].sort()),
    });
  }

  async initialize(clientVersion: string): Promise<void> {
    if (this.phaseValue !== "new") throw new GrokProtocolError("INVALID_STATE", "initialize");
    this.phaseValue = "initializing";
    try {
      const value = responseObject(await this.transport.request("initialize", {
        protocolVersion: GROK_BUILD_PROTOCOL_VERSION,
        clientCapabilities: { fs: { readTextFile: false, writeTextFile: false }, terminal: false },
        _meta: { clientType: "gogoke", clientVersion: requiredString(clientVersion, "clientVersion") },
      }), "initialize");
      if (value.protocolVersion !== GROK_BUILD_PROTOCOL_VERSION || !isRecord(value.agentCapabilities)) {
        throw new GrokProtocolError("INVALID_RESPONSE", "initialize protocolVersion/agentCapabilities");
      }
      const meta = isRecord(value._meta) ? value._meta : {};
      const observedVersion = requiredString(meta.agentVersion, "initialize._meta.agentVersion");
      if (observedVersion !== GROK_BUILD_PINNED_VERSION) {
        throw new GrokProtocolError("VERSION_MISMATCH", `expected ${GROK_BUILD_PINNED_VERSION}`);
      }
      this.observedVersionValue = observedVersion;
      this.observed.add("initialize");
      const capabilities = value.agentCapabilities;
      if (capabilities.loadSession === true) this.advertised.add("loadSession");
      const sessions = isRecord(capabilities.sessionCapabilities) ? capabilities.sessionCapabilities : {};
      if (isRecord(sessions.resume)) this.advertised.add("resume");
      if (isRecord(sessions.list)) this.advertised.add("list");
      if (isRecord(sessions.close)) this.advertised.add("close");
      this.phaseValue = "ready";
    } catch (error) {
      this.phaseValue = "recovery-required";
      this.uncertainMethodValue = "initialize";
      throw error;
    }
  }

  async start(cwd: string): Promise<string> {
    this.ready("session/new");
    if (this.sessionIdValue !== null) throw new GrokProtocolError("INVALID_STATE", "session already bound");
    const workdir = requiredString(cwd, "cwd");
    try {
      const result = await this.call("session/new", {
        cwd: workdir,
        mcpServers: [],
        _meta: { yoloMode: false },
      });
      const sessionId = requiredString(responseObject(result, "session/new").sessionId, "session/new.sessionId");
      this.sessionIdValue = sessionId;
      this.observed.add("session/new");
      return sessionId;
    } catch (error) {
      if (this.phaseValue === "ready") {
        this.phaseValue = "recovery-required";
        this.uncertainMethodValue = "session/new";
      }
      throw error;
    }
  }

  /** ACP load is the vendor continuation operation; it is not a new session. */
  async load(sessionId: string, cwd: string): Promise<void> {
    this.ready("session/load");
    if (this.sessionIdValue !== null) throw new GrokProtocolError("INVALID_STATE", "session already bound");
    if (!this.advertised.has("loadSession")) throw new GrokProtocolError("UNSUPPORTED", "session/load not advertised");
    const id = requiredString(sessionId, "sessionId");
    try {
      const result = responseObject(await this.call("session/load", {
        sessionId: id,
        cwd: requiredString(cwd, "cwd"),
        mcpServers: [],
      }), "session/load");
      if (result.sessionId !== undefined && result.sessionId !== id) {
        throw new GrokProtocolError("SESSION_BINDING", "session/load returned a different session id");
      }
      this.sessionIdValue = id;
      this.observed.add("session/load");
    } catch (error) {
      if (this.phaseValue === "ready") {
        this.phaseValue = "recovery-required";
        this.uncertainMethodValue = "session/load";
      }
      throw error;
    }
  }

  /** The caller waits for the actual ACP prompt response, never process exit alone. */
  prompt(text: string): Promise<GrokPromptReceipt> {
    this.ready("session/prompt");
    const sessionId = this.bound("session/prompt");
    if (this.activePrompt !== null) throw new GrokProtocolError("INVALID_STATE", "prompt already active");
    const promptText = requiredString(text, "prompt");
    const operation = this.call("session/prompt", {
      sessionId,
      prompt: [{ type: "text", text: promptText }],
    }).then((value) => this.promptReceipt(sessionId, value));
    this.activePrompt = operation;
    void operation.then(
      () => { if (this.activePrompt === operation) this.activePrompt = null; },
      () => { if (this.activePrompt === operation) this.activePrompt = null; },
    );
    return operation;
  }

  /**
   * ACP has no in-turn steer here. Send cancel, wait for the original prompt's
   * explicit `cancelled` terminal receipt, then submit the replacement. If the
   * original turn completes normally or becomes unknown, the replacement is not sent.
   */
  async interruptAndResume(replacement: string): Promise<GrokInterruptResumeReceipt> {
    this.ready("session/cancel");
    const sessionId = this.bound("session/cancel");
    const active = this.activePrompt;
    if (active === null) throw new GrokProtocolError("INVALID_STATE", "no active prompt");
    try {
      await this.transport.notify("session/cancel", { sessionId });
      const stopped = await active;
      if (stopped.stopReason !== "cancelled") {
        throw new GrokProtocolError("CANCEL_UNCONFIRMED", `original prompt ended with ${stopped.stopReason}`);
      }
      if (this.activePrompt === active) this.activePrompt = null;
      const resumed = await this.prompt(replacement);
      this.observed.add("session/cancel");
      return Object.freeze({
        provider: "grok-build",
        sessionId,
        interrupt: Object.freeze({ method: "session/cancel", requestSent: true, promptStopReason: "cancelled" }),
        resumed,
      });
    } catch (error) {
      this.phaseValue = "recovery-required";
      this.uncertainMethodValue = "session/cancel";
      throw error;
    }
  }

  /** Vendor frames remain tagged/raw; A owns filtering and normalized ledger projection. */
  observe(frame: GrokFrame): GrokRawObservation {
    return Object.freeze({
      source: Object.freeze({ provider: "grok-build", version: GROK_BUILD_PINNED_VERSION }),
      frame,
    });
  }

  transportLost(): Readonly<{ sessionId: string | null; uncertainMethod: string | null }> {
    if (this.phaseValue !== "closed") this.phaseValue = "recovery-required";
    return Object.freeze({ sessionId: this.sessionIdValue, uncertainMethod: this.uncertainMethodValue });
  }

  close(): void {
    this.phaseValue = "closed";
  }

  private ready(method: string): void {
    if (this.phaseValue !== "ready") throw new GrokProtocolError("INVALID_STATE", method);
  }

  private bound(method: string): string {
    const sessionId = this.sessionIdValue;
    if (sessionId === null) throw new GrokProtocolError("INVALID_STATE", `${method}: no session`);
    return sessionId;
  }

  private async call(method: string, params: Readonly<JsonRecord>): Promise<unknown> {
    try {
      const result = await this.transport.request(method, params);
      if (this.phaseValue !== "ready") throw new GrokProtocolError("INVALID_STATE", `response after loss: ${method}`);
      return result;
    } catch (error) {
      this.phaseValue = "recovery-required";
      this.uncertainMethodValue = method;
      throw error;
    }
  }

  private promptReceipt(sessionId: string, value: unknown): GrokPromptReceipt {
    const response = responseObject(value, "session/prompt");
    const raw = response.stopReason;
    const stopReason: GrokPromptStopReason = raw === "end_turn" || raw === "cancelled" || raw === "max_turn_requests"
      ? raw : "unknown";
    this.observed.add("session/prompt");
    if (stopReason === "unknown") {
      this.phaseValue = "recovery-required";
      this.uncertainMethodValue = "session/prompt";
    }
    // Keep only the terminal acknowledgement. Never persist or forward arbitrary
    // vendor response fields such as account metadata or private transcript data.
    return Object.freeze({ provider: "grok-build", sessionId, stopReason,
      response: Object.freeze({ stopReason: typeof raw === "string" ? raw : null }) });
  }
}
