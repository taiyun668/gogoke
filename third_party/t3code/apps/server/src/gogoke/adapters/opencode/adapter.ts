import {
  OPENCODE_ACP_VERSION,
  OPENCODE_PINNED_VERSION,
  OpenCodeProtocolError,
  isRecord,
  parseStopReason,
  requireText,
  responseRecord,
  type OpenCodeAcpEvent,
  type OpenCodeAcpNotification,
  type OpenCodeHostedLoginCommand,
  type OpenCodeAcpTransport,
  type OpenCodeStopReason,
} from "./protocol.ts";

export type OpenCodeAdapterPhase = "new" | "ready" | "recovery-required" | "closed";

export interface OpenCodeTurnResult {
  readonly status: "settled";
  readonly nativeSessionId: string;
  readonly stopReason: OpenCodeStopReason;
}

export interface OpenCodeInterruptResult {
  readonly status: "interrupted";
  readonly nativeSessionId: string;
  /** Session cancel ends a model turn. H still needs native process stop proof. */
  readonly processStopped: false;
}

export interface OpenCodeCapabilityReport {
  readonly driver: "opencode";
  readonly pinnedVersion: typeof OPENCODE_PINNED_VERSION;
  readonly sourceCommit: "anomalyco/opencode 545f51d";
  readonly runtimeEvidence: "NOT_RUN";
  readonly features: Readonly<Record<
    "acp" | "hostedLoginFlow" | "resume" | "inTurnSteer" | "interruptAndResume" |
    "nativeQuestionCard" | "projectInstructions" | "instanceHomeIsolation" | "vendorMemoryOff",
    "SOURCE_PRESENT_RUNTIME_UNVERIFIED" | "UNSUPPORTED" | "HOST_CONFIGURATION_REQUIRED" | "NOT_SUPPORTED"
  >>;
  readonly knownProtocolQuirks: readonly string[];
}

export const OPENCODE_SOURCE_CAPABILITIES: OpenCodeCapabilityReport = Object.freeze({
  driver: "opencode",
  pinnedVersion: OPENCODE_PINNED_VERSION,
  sourceCommit: "anomalyco/opencode 545f51d",
  runtimeEvidence: "NOT_RUN",
  features: Object.freeze({
    acp: "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
    hostedLoginFlow: "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
    resume: "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
    inTurnSteer: "UNSUPPORTED",
    interruptAndResume: "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
    nativeQuestionCard: "UNSUPPORTED",
    projectInstructions: "SOURCE_PRESENT_RUNTIME_UNVERIFIED",
    instanceHomeIsolation: "HOST_CONFIGURATION_REQUIRED",
    // ACP instance-home isolation does not disable or erase OpenCode's own session history.
    vendorMemoryOff: "NOT_SUPPORTED",
  }),
  knownProtocolQuirks: Object.freeze([
    "v1.18.32 session/resume returns configOptions without echoing sessionId; the request is bound to the supplied ID.",
    "session/cancel is a notification; only the corresponding session/prompt response can confirm stopReason=cancelled.",
  ]),
});

function validateAgentInfo(value: unknown): OpenCodeHostedLoginCommand {
  const response = responseRecord(value, "initialize");
  if (response.protocolVersion !== OPENCODE_ACP_VERSION || !isRecord(response.agentInfo) ||
      response.agentInfo.name !== "OpenCode" || response.agentInfo.version !== OPENCODE_PINNED_VERSION) {
    throw new OpenCodeProtocolError("VERSION_MISMATCH", "initialize must identify OpenCode ACP v1.18.32");
  }
  if (!isRecord(response.agentCapabilities) || response.agentCapabilities.loadSession !== true) {
    throw new OpenCodeProtocolError("CAPABILITY_MISMATCH", "OpenCode ACP loadSession capability is required");
  }
  if (!Array.isArray(response.authMethods)) {
    throw new OpenCodeProtocolError("INVALID_RESPONSE", "initialize.authMethods");
  }
  const login = response.authMethods.find((item) => isRecord(item) && item.id === "opencode-login");
  if (!isRecord(login) || !isRecord(login._meta) || !isRecord(login._meta["terminal-auth"])) {
    throw new OpenCodeProtocolError("CAPABILITY_MISMATCH", "hosted OpenCode login command was not advertised");
  }
  const terminalAuth = login._meta["terminal-auth"];
  if (terminalAuth.command !== "opencode" || !Array.isArray(terminalAuth.args) ||
      terminalAuth.args.length !== 2 || terminalAuth.args[0] !== "auth" || terminalAuth.args[1] !== "login" ||
      terminalAuth.label !== "OpenCode Login") {
    throw new OpenCodeProtocolError("INVALID_RESPONSE", "initialize.authMethods.terminal-auth");
  }
  return Object.freeze({
    methodId: "opencode-login",
    args: Object.freeze(["auth", "login"] as const),
    label: "OpenCode Login",
  });
}

/**
 * ACP state machine only. The host owns executable pinning, login, home,
 * Content-Length framing, RPC correlation, session generation and process custody.
 */
export class OpenCode11832Adapter {
  readonly #transport: OpenCodeAcpTransport;
  readonly #onEvent: ((event: OpenCodeAcpEvent) => void) | undefined;
  #phase: OpenCodeAdapterPhase = "new";
  #cwd: string | null = null;
  #nativeSessionId: string | null = null;
  #activePrompt: Promise<OpenCodeTurnResult> | null = null;
  #eventFailure: OpenCodeProtocolError | null = null;
  #bindingEvents: OpenCodeAcpNotification[] = [];
  #loginCommand: OpenCodeHostedLoginCommand | null = null;

  constructor(input: {
    readonly transport: OpenCodeAcpTransport;
    readonly observedVersion: string;
    readonly onEvent?: (event: OpenCodeAcpEvent) => void;
  }) {
    if (input.observedVersion !== OPENCODE_PINNED_VERSION) {
      throw new OpenCodeProtocolError("VERSION_MISMATCH", `expected ${OPENCODE_PINNED_VERSION}`);
    }
    this.#transport = input.transport;
    this.#onEvent = input.onEvent;
    this.#transport.setEventHandler?.((event) => this.#receiveEvent(event));
  }

  get phase(): OpenCodeAdapterPhase { return this.#phase; }
  get nativeSessionId(): string | null { return this.#nativeSessionId; }
  get hostedLoginCommand(): OpenCodeHostedLoginCommand | null { return this.#loginCommand; }

  async initialize(clientVersion: string): Promise<void> {
    this.#requirePhase("new", "initialize");
    try {
      this.#loginCommand = validateAgentInfo(await this.#transport.request("initialize", {
        protocolVersion: OPENCODE_ACP_VERSION,
        clientInfo: { name: "gogoke", version: requireText(clientVersion, "clientVersion") },
        clientCapabilities: {
          fs: { readTextFile: false, writeTextFile: false },
          _meta: { "terminal-auth": true },
        },
      }));
      this.#phase = "ready";
    } catch (error) {
      this.#uncertain("initialize", error);
    }
  }

  async open(cwd: string): Promise<string> {
    this.#ready("session/new");
    if (this.#nativeSessionId !== null) throw new OpenCodeProtocolError("INVALID_STATE", "session already bound");
    const target = requireText(cwd, "cwd");
    try {
      const response = responseRecord(await this.#transport.request("session/new", {
        cwd: target,
        mcpServers: [],
      }), "session/new");
      const sessionId = requireText(response.sessionId, "session/new.sessionId");
      this.#nativeSessionId = sessionId;
      this.#cwd = target;
      this.#flushBindingEvents(sessionId);
      return sessionId;
    } catch (error) {
      this.#uncertain("session/new", error);
    }
  }

  /**
   * Rebind the same native transcript after H has confirmed the prior process
   * generation stopped and opened this pinned instance again.
   */
  async resume(nativeSessionId: string, cwd: string): Promise<void> {
    this.#ready("session/resume");
    if (this.#nativeSessionId !== null) throw new OpenCodeProtocolError("INVALID_STATE", "session already bound");
    const wanted = requireText(nativeSessionId, "nativeSessionId");
    const target = requireText(cwd, "cwd");
    try {
      const response = responseRecord(await this.#transport.request("session/resume", {
        sessionId: wanted,
        cwd: target,
        mcpServers: [],
      }), "session/resume");
      // Upstream v1.18.32 omits sessionId from this success result. If a future
      // build does echo one, it must be the exact caller-bound native session.
      if (response.sessionId !== undefined && response.sessionId !== wanted) {
        throw new OpenCodeProtocolError("SESSION_MISMATCH", "session/resume returned a different session");
      }
      if (!Array.isArray(response.configOptions)) {
        throw new OpenCodeProtocolError("INVALID_RESPONSE", "session/resume.configOptions");
      }
      this.#nativeSessionId = wanted;
      this.#cwd = target;
      this.#flushBindingEvents(wanted);
    } catch (error) {
      this.#uncertain("session/resume", error);
    }
  }

  /** ACP has no in-turn steering operation; this is intentionally unsupported. */
  async steer(_expectedNativeSessionId: string, _text: string): Promise<never> {
    this.#ready("steer");
    throw new OpenCodeProtocolError("UNSUPPORTED", "OpenCode ACP has no in-turn steer method");
  }

  send(text: string): Promise<OpenCodeTurnResult> {
    this.#ready("session/prompt");
    const nativeSessionId = this.#bound("session/prompt");
    if (this.#activePrompt !== null) throw new OpenCodeProtocolError("INVALID_STATE", "prompt already active");
    const body = requireText(text, "text");
    const flight = this.#sendPrompt(nativeSessionId, body);
    this.#activePrompt = flight;
    void flight.then(
      () => { if (this.#activePrompt === flight) this.#activePrompt = null; },
      () => { if (this.#activePrompt === flight) this.#activePrompt = null; },
    );
    return flight;
  }

  /**
   * Interrupt the exact active native session and wait for its prompt result.
   * An ACP notification alone is never called a successful interruption.
   */
  async interrupt(expectedNativeSessionId: string): Promise<OpenCodeInterruptResult> {
    this.#ready("session/cancel");
    const nativeSessionId = this.#bound("session/cancel");
    if (requireText(expectedNativeSessionId, "expectedNativeSessionId") !== nativeSessionId ||
        this.#activePrompt === null) {
      throw new OpenCodeProtocolError("SESSION_MISMATCH", "interrupt requires the exact active session");
    }
    const prompt = this.#activePrompt;
    try {
      await this.#transport.notify("session/cancel", { sessionId: nativeSessionId });
    } catch (error) {
      this.#uncertain("session/cancel", error);
    }
    const result = await prompt;
    if (result.stopReason !== "cancelled") {
      throw new OpenCodeProtocolError("INTERRUPT_NOT_CONFIRMED", `prompt settled with ${result.stopReason}`);
    }
    return Object.freeze({ status: "interrupted", nativeSessionId, processStopped: false });
  }

  /** Interrupt, await confirmed cancellation, then send direction in the same ACP session. */
  async interruptAndResume(expectedNativeSessionId: string, text: string): Promise<OpenCodeTurnResult> {
    await this.interrupt(expectedNativeSessionId);
    return this.send(text);
  }

  capabilityReport(): OpenCodeCapabilityReport {
    return OPENCODE_SOURCE_CAPABILITIES;
  }

  /** Mark transport close by the host; this adapter does not kill a process. */
  markClosed(): void {
    this.#phase = "closed";
    this.#nativeSessionId = null;
    this.#cwd = null;
    this.#activePrompt = null;
    this.#bindingEvents = [];
  }

  async #sendPrompt(nativeSessionId: string, text: string): Promise<OpenCodeTurnResult> {
    try {
      const response = responseRecord(await this.#transport.request("session/prompt", {
        sessionId: nativeSessionId,
        prompt: [{ type: "text", text }],
      }), "session/prompt");
      if (this.#eventFailure !== null) throw this.#eventFailure;
      return Object.freeze({
        status: "settled",
        nativeSessionId,
        stopReason: parseStopReason(response.stopReason),
      });
    } catch (error) {
      this.#uncertain("session/prompt", error);
    }
  }

  #receiveEvent(value: unknown): void {
    if (!isRecord(value) || typeof value.method !== "string" || !isRecord(value.params) ||
        typeof value.params.sessionId !== "string") {
      this.#eventFailure = new OpenCodeProtocolError("INVALID_EVENT", "ACP notification must carry method and sessionId");
      return;
    }
    const notification = Object.freeze({ method: value.method, params: value.params });
    if (this.#nativeSessionId === null) {
      if (this.#bindingEvents.length >= 128) {
        this.#eventFailure = new OpenCodeProtocolError("EVENT_OVERFLOW", "too many ACP events before session binding");
        return;
      }
      this.#bindingEvents.push(notification);
      return;
    }
    this.#emitBoundEvent(notification, this.#nativeSessionId);
  }

  #flushBindingEvents(expectedSessionId: string): void {
    const queued = this.#bindingEvents;
    this.#bindingEvents = [];
    for (const notification of queued) this.#emitBoundEvent(notification, expectedSessionId);
    if (this.#eventFailure !== null) this.#uncertain("session binding event", this.#eventFailure);
  }

  #emitBoundEvent(notification: OpenCodeAcpNotification, expectedSessionId: string): void {
    const params = notification.params as Record<string, unknown>;
    if (params.sessionId !== expectedSessionId) {
      this.#eventFailure = new OpenCodeProtocolError("SESSION_MISMATCH", "ACP event is outside the bound session");
      return;
    }
    try {
      this.#onEvent?.(Object.freeze({ provider: "opencode", nativeSessionId: expectedSessionId,
        method: notification.method, payload: notification.params }));
    } catch {
      this.#eventFailure = new OpenCodeProtocolError("EVENT_HANDLER_FAILED", "event consumer rejected ACP event");
    }
  }

  #bound(method: string): string {
    if (this.#nativeSessionId === null || this.#cwd === null) {
      throw new OpenCodeProtocolError("INVALID_STATE", `${method}: no bound session`);
    }
    return this.#nativeSessionId;
  }

  #ready(method: string): void { this.#requirePhase("ready", method); }

  #requirePhase(expected: OpenCodeAdapterPhase, method: string): void {
    if (this.#phase !== expected) throw new OpenCodeProtocolError("INVALID_STATE", `${method}: ${this.#phase}`);
  }

  #uncertain(method: string, cause: unknown): never {
    this.#phase = "recovery-required";
    throw cause instanceof OpenCodeProtocolError
      ? cause
      : new OpenCodeProtocolError("OUTCOME_UNKNOWN", `${method} failed; consult the host journal before retry`, cause);
  }
}
