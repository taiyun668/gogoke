import { CODEX_PINNED_VERSION, CodexProtocolError, type CodexFrame,
  type CodexQuestionCard, buildQuestionCardResponse } from "./protocol.ts";

type JsonRecord = Record<string, unknown>;
const record = (value: unknown): value is JsonRecord =>
  value !== null && typeof value === "object" && !Array.isArray(value);
const nonempty = (value: string, field: string): string => {
  if (typeof value !== "string" || value.length === 0 || value.includes("\0"))
    throw new CodexProtocolError("INVALID_INPUT", field);
  return value;
};
const object = (value: unknown, method: string): JsonRecord => {
  if (!record(value)) throw new CodexProtocolError("INVALID_RESPONSE", method);
  return value;
};
const empty = (value: unknown, method: string): void => {
  if (!record(value) || Object.keys(value).length !== 0)
    throw new CodexProtocolError("INVALID_RESPONSE", `${method} must return {}`);
};
const threadIdOf = (value: unknown, method: string): string => {
  const thread = object(object(value, method).thread, method).id;
  if (typeof thread !== "string" || thread.length === 0)
    throw new CodexProtocolError("INVALID_RESPONSE", `${method}.thread.id`);
  return thread;
};

/** Persistent stdio, binary pin, approval routing and request-ID correlation belong to H. */
export interface CodexSessionTransport {
  request(method: string, params: Readonly<JsonRecord>): Promise<unknown>;
  notify(method: "initialized", params?: Readonly<JsonRecord>): Promise<void>;
}

export type CodexSessionPhase = "new" | "initializing" | "ready" | "recovery-required" | "closed";
export type CodexTurnStatus = "idle" | "running" | "completed" | "interrupted" | "failed" | "unknown";
export type CodexSessionEvent =
  | { readonly kind: "turn-terminal"; readonly threadId: string; readonly turnId: string;
      readonly status: "completed" | "interrupted" | "failed" }
  | { readonly kind: "question"; readonly card: CodexQuestionCard }
  | { readonly kind: "compaction-item"; readonly threadId: string; readonly itemId: string }
  | { readonly kind: "unhandled"; readonly method: string };

/**
 * Protocol state only. Its effects are request/notification writes through an
 * H-owned transport. A lost response leaves acceptance unknown; callers must
 * use their durable operation journal before attempting a replay.
 */
export class Codex0149Session {
  private readonly transport: CodexSessionTransport;
  private phaseValue: CodexSessionPhase = "new";
  private memoryOffCwd: string | null = null;
  private threadIdValue: string | null = null;
  private turnIdValue: string | null = null;
  private turnStatusValue: CodexTurnStatus = "idle";
  private uncertainMethodValue: string | null = null;
  private questionValue: CodexQuestionCard | null = null;
  private compactionRequested = false;
  private inFlight = false;
  private startingTurn = false;
  private earlyTerminal: { id: string; status: "completed" | "interrupted" | "failed" } | null = null;
  private readonly observed = new Set<string>();

  constructor(transport: CodexSessionTransport, observedVersion: string) {
    if (observedVersion !== CODEX_PINNED_VERSION)
      throw new CodexProtocolError("VERSION_MISMATCH", `expected ${CODEX_PINNED_VERSION}`);
    this.transport = transport;
  }

  get phase(): CodexSessionPhase { return this.phaseValue; }
  get threadId(): string | null { return this.threadIdValue; }
  get turnId(): string | null { return this.turnIdValue; }
  get turnStatus(): CodexTurnStatus { return this.turnStatusValue; }
  get uncertainMethod(): string | null { return this.uncertainMethodValue; }
  get pendingQuestion(): CodexQuestionCard | null { return this.questionValue; }
  get compactionPending(): boolean { return this.compactionRequested; }

  /** Source presence is not runtime support. Only successful methods are observed. */
  capabilityReport(): Readonly<{ pinnedVersion: typeof CODEX_PINNED_VERSION;
    runtimeEvidence: "METHOD_RESPONSES_ONLY" | "UNKNOWN"; observedMethods: readonly string[] }> {
    return Object.freeze({ pinnedVersion: CODEX_PINNED_VERSION,
      runtimeEvidence: this.observed.size === 0 ? "UNKNOWN" : "METHOD_RESPONSES_ONLY",
      observedMethods: Object.freeze([...this.observed].sort()) });
  }

  /** Read-only discovery. Counts describe the returned page, never account entitlements. */
  async discover(): Promise<Readonly<{ modelPageCount: number; modelNextCursor: string | null;
    featurePageCount: number; featureNextCursor: string | null }>> {
    this.ready("discover");
    const page = (method: string, value: unknown): { count: number; nextCursor: string | null } => {
      const result = object(value, method);
      if (!Array.isArray(result.data) ||
          !(result.nextCursor === undefined || result.nextCursor === null || typeof result.nextCursor === "string"))
        throw new CodexProtocolError("INVALID_RESPONSE", `${method} page`);
      return { count: result.data.length, nextCursor: (result.nextCursor ?? null) as string | null };
    };
    const models = this.confirm("model/list", await this.call("model/list", {}),
      (value) => page("model/list", value));
    const features = this.confirm("experimentalFeature/list",
      await this.call("experimentalFeature/list", this.threadIdValue === null ? {} : { threadId: this.threadIdValue }),
      (value) => page("experimentalFeature/list", value));
    return Object.freeze({ modelPageCount: models.count, modelNextCursor: models.nextCursor,
      featurePageCount: features.count, featureNextCursor: features.nextCursor });
  }

  async initialize(clientVersion: string): Promise<void> {
    if (this.phaseValue !== "new") throw new CodexProtocolError("INVALID_STATE", "initialize");
    this.phaseValue = "initializing";
    try {
      object(await this.transport.request("initialize", {
        clientInfo: { name: "gogoke", title: "gogoke", version: nonempty(clientVersion, "clientVersion") },
        capabilities: { experimentalApi: true },
      }), "initialize");
      if (this.phaseValue !== "initializing") throw new CodexProtocolError("INVALID_STATE", "initialize lost transport");
      await this.transport.notify("initialized");
      if (this.phaseValue !== "initializing") throw new CodexProtocolError("INVALID_STATE", "initialized lost transport");
      this.observed.add("initialize");
      this.phaseValue = "ready";
    } catch (error) {
      this.phaseValue = "recovery-required";
      this.uncertainMethodValue = "initialize";
      throw error;
    }
  }

  /** Effective configuration comes from this exact app-server process. A
   * requested command-line override alone is not a memory-off observation. */
  async verifyMemoryOff(cwd: string): Promise<void> {
    this.ready("config/read");
    const targetCwd = nonempty(cwd, "cwd");
    this.memoryOffCwd = null;
    this.confirm("config/read", await this.call("config/read", {
      cwd: targetCwd, includeLayers: true,
    }), (value) => {
      const config = object(object(value, "config/read").config, "config/read.config");
      const features = object(config.features, "config/read.features");
      const memories = object(config.memories, "config/read.memories");
      if (features.memories !== false || memories.generate_memories !== false ||
          memories.use_memories !== false) {
        throw new CodexProtocolError("MEMORY_NOT_DISABLED", "effective config/read values");
      }
    });
    this.memoryOffCwd = targetCwd;
  }

  /** This read never refreshes a token and never returns account details. A
   * local account object proves only credential presence, not validity. */
  async observeLocalAccount(): Promise<"LOGGED_OUT" | "CREDENTIAL_PRESENT" | "UNKNOWN"> {
    this.ready("account/read");
    return this.confirm("account/read", await this.call("account/read", {}), (value) => {
      const response = object(value, "account/read");
      if (typeof response.requiresOpenaiAuth !== "boolean" ||
          !Object.hasOwn(response, "account")) {
        throw new CodexProtocolError("INVALID_RESPONSE", "account/read fields");
      }
      if (response.account === null) {
        return response.requiresOpenaiAuth ? "LOGGED_OUT" : "UNKNOWN";
      }
      const account = object(response.account, "account/read.account");
      if (account.type !== "apiKey" && account.type !== "chatgpt" &&
          account.type !== "amazonBedrock") {
        throw new CodexProtocolError("INVALID_RESPONSE", "account/read.account.type");
      }
      return "CREDENTIAL_PRESENT";
    });
  }

  private ready(method: string): void {
    if (this.phaseValue !== "ready") throw new CodexProtocolError("INVALID_STATE", method);
    if (this.inFlight) throw new CodexProtocolError("INVALID_STATE", "another request is in flight");
  }
  private bound(method: string): string {
    this.ready(method);
    if (this.threadIdValue === null) throw new CodexProtocolError("INVALID_STATE", `${method}: no thread`);
    return this.threadIdValue;
  }
  private requireMemoryOff(method: string, cwd: string): void {
    // H supplies one canonical directory string for both operations.
    if (this.memoryOffCwd !== cwd) throw new CodexProtocolError("MEMORY_NOT_VERIFIED", method);
  }
  private async call(method: string, params: Readonly<JsonRecord>): Promise<unknown> {
    if (this.inFlight) throw new CodexProtocolError("INVALID_STATE", "another request is in flight");
    this.inFlight = true;
    try {
      const result = await this.transport.request(method, params);
      if (this.phaseValue !== "ready") throw new CodexProtocolError("INVALID_STATE", "response after transport loss");
      return result;
    } catch (error) {
      this.phaseValue = "recovery-required";
      this.turnStatusValue = "unknown";
      this.uncertainMethodValue = method;
      throw error;
    } finally {
      this.inFlight = false;
    }
  }
  private confirm<T>(method: string, value: unknown, validate: (value: unknown) => T): T {
    try {
      const result = validate(value);
      this.observed.add(method);
      return result;
    } catch (error) {
      // A malformed ACK can follow an accepted mutation. Replaying is unsafe.
      this.phaseValue = "recovery-required";
      this.turnStatusValue = "unknown";
      this.uncertainMethodValue = method;
      throw error;
    }
  }
  private takeEarlyTerminal(): { id: string; status: "completed" | "interrupted" | "failed" } | null {
    const terminal = this.earlyTerminal;
    this.earlyTerminal = null;
    return terminal;
  }

  async startThread(cwd: string): Promise<string> {
    this.ready("thread/start");
    const targetCwd = nonempty(cwd, "cwd");
    this.requireMemoryOff("thread/start", targetCwd);
    if (this.threadIdValue !== null) throw new CodexProtocolError("INVALID_STATE", "thread already bound");
    const id = this.confirm("thread/start", await this.call("thread/start", { cwd: targetCwd }),
      (value) => threadIdOf(value, "thread/start"));
    this.threadIdValue = id;
    return id;
  }

  async resume(threadId: string, cwd: string): Promise<void> {
    this.ready("thread/resume");
    const targetCwd = nonempty(cwd, "cwd");
    this.requireMemoryOff("thread/resume", targetCwd);
    if (this.threadIdValue !== null) throw new CodexProtocolError("INVALID_STATE", "thread already bound");
    const wanted = nonempty(threadId, "threadId");
    this.confirm("thread/resume", await this.call("thread/resume", { threadId: wanted }), (value) => {
      if (threadIdOf(value, "thread/resume") !== wanted)
        throw new CodexProtocolError("INVALID_RESPONSE", "thread/resume returned a different thread");
      if (object(object(value, "thread/resume").thread, "thread/resume.thread").cwd !== targetCwd)
        throw new CodexProtocolError("INVALID_RESPONSE", "thread/resume returned a different cwd");
    });
    this.threadIdValue = wanted;
  }

  async startTurn(text: string): Promise<string> {
    const threadId = this.bound("turn/start");
    if (this.turnStatusValue === "running" || this.questionValue !== null)
      throw new CodexProtocolError("INVALID_STATE", "turn already active");
    this.earlyTerminal = null;
    this.startingTurn = true;
    let response: unknown;
    try {
      response = await this.call("turn/start", {
        threadId, input: [{ type: "text", text: nonempty(text, "text") }],
      });
    } finally { this.startingTurn = false; }
    const turn = this.confirm("turn/start", response, (value) => {
      const turn = object(object(value, "turn/start").turn, "turn/start");
      const turnId = nonempty(turn.id as string, "turn.id");
      if (turn.status !== "inProgress" && turn.status !== "completed" &&
          turn.status !== "interrupted" && turn.status !== "failed")
        throw new CodexProtocolError("INVALID_RESPONSE", "turn/start status");
      return { id: turnId, status: turn.status as "inProgress" | "completed" | "interrupted" | "failed" };
    });
    this.turnIdValue = turn.id;
    this.turnStatusValue = turn.status === "inProgress" ? "running" : turn.status;
    const earlyTerminal = this.takeEarlyTerminal();
    if (earlyTerminal?.id === turn.id) this.turnStatusValue = earlyTerminal.status;
    return turn.id;
  }

  async steer(expectedTurnId: string, text: string): Promise<void> {
    const threadId = this.bound("turn/steer");
    const wanted = nonempty(expectedTurnId, "expectedTurnId");
    if (this.turnStatusValue !== "running" || this.turnIdValue !== wanted)
      throw new CodexProtocolError("INVALID_STATE", "turn/steer requires current active turn");
    this.confirm("turn/steer", await this.call("turn/steer", { threadId, expectedTurnId: wanted,
      input: [{ type: "text", text: nonempty(text, "text") }] }), (value) => {
      if (object(value, "turn/steer").turnId !== wanted)
        throw new CodexProtocolError("INVALID_RESPONSE", "turn/steer returned a different turn");
    });
  }

  async appendWithoutTurn(text: string): Promise<void> {
    const threadId = this.bound("thread/inject_items");
    this.confirm("thread/inject_items", await this.call("thread/inject_items", { threadId,
      items: [{ type: "message", role: "user", content: [{ type: "input_text", text: nonempty(text, "text") }] }],
    }), (value) => empty(value, "thread/inject_items"));
  }

  async requestCompaction(): Promise<void> {
    const threadId = this.bound("thread/compact/start");
    if (this.compactionRequested) throw new CodexProtocolError("INVALID_STATE", "compaction already requested");
    this.confirm("thread/compact/start", await this.call("thread/compact/start", { threadId }),
      (value) => empty(value, "thread/compact/start"));
    this.compactionRequested = true; // ACK is not completion.
  }

  /** Only H may send the response to this exact server request ID. */
  answerQuestion(requestId: string | number, answers: Readonly<Record<string, readonly string[]>>):
    Readonly<{ id: string | number; result: ReturnType<typeof buildQuestionCardResponse> }> {
    const card = this.questionValue;
    if (this.phaseValue !== "ready" || card === null || card.requestId !== requestId)
      throw new CodexProtocolError("INVALID_STATE", "no matching native question");
    return Object.freeze({ id: requestId, result: buildQuestionCardResponse(card, answers) });
  }

  /** H invokes this only after it has sent and correlated the answer response. */
  questionAnswered(requestId: string | number): void {
    if (this.questionValue === null || this.questionValue.requestId !== requestId)
      throw new CodexProtocolError("INVALID_STATE", "question response mismatch");
    this.questionValue = null;
  }

  observe(frame: CodexFrame): CodexSessionEvent | null {
    if (this.phaseValue !== "ready") return null;
    if (frame.kind === "response") return null; // H owns request-ID correlation.
    if (frame.kind === "server-request") {
      if (frame.method !== "item/tool/requestUserInput" || frame.questionCard === undefined)
        return { kind: "unhandled", method: frame.method };
      const card = frame.questionCard;
      if (card.threadId !== this.threadIdValue || card.turnId !== this.turnIdValue ||
          this.questionValue !== null || this.turnStatusValue !== "running")
        throw new CodexProtocolError("QUESTION_BINDING", "question outside active turn");
      this.questionValue = card;
      return { kind: "question", card };
    }
    if (!record(frame.params) || frame.params.threadId !== this.threadIdValue) return null;
    if (frame.method === "turn/started") {
      const turn = object(frame.params.turn, "turn/started");
      if (typeof turn.id === "string" && turn.id === this.turnIdValue) this.turnStatusValue = "running";
      return null;
    }
    if (frame.method === "turn/completed") {
      const turn = object(frame.params.turn, "turn/completed");
      if (turn.status !== "completed" && turn.status !== "interrupted" && turn.status !== "failed")
        throw new CodexProtocolError("INVALID_NOTIFICATION", "terminal turn status");
      if (this.startingTurn && turn.id !== this.turnIdValue && typeof turn.id === "string") {
        this.earlyTerminal = { id: turn.id, status: turn.status };
        return null;
      }
      if (turn.id !== this.turnIdValue || this.turnStatusValue !== "running") return null;
      this.turnStatusValue = turn.status;
      this.questionValue = null;
      return { kind: "turn-terminal", threadId: this.threadIdValue!, turnId: this.turnIdValue!, status: turn.status };
    }
    if (frame.method === "item/completed" && this.compactionRequested) {
      const item = object(frame.params.item, "item/completed");
      if (item.type === "contextCompaction" && typeof item.id === "string") {
        this.compactionRequested = false;
        return { kind: "compaction-item", threadId: this.threadIdValue!, itemId: item.id };
      }
    }
    return null;
  }

  /** Transport loss has unknown acceptance; resume happens on a new initialized connection. */
  transportLost(): Readonly<{ threadId: string | null; uncertainMethod: string | null }> {
    if (this.phaseValue !== "closed") {
      this.phaseValue = "recovery-required";
      this.turnStatusValue = "unknown";
      this.questionValue = null;
    }
    return Object.freeze({ threadId: this.threadIdValue, uncertainMethod: this.uncertainMethodValue });
  }

  close(): void { this.phaseValue = "closed"; this.questionValue = null; }
}
