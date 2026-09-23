import * as NodeUtilTypes from "node:util/types";

import { PiJsonlFramer } from "./framing.ts";
import type {
  PiAcceptedCommand,
  PiManagedSessionOptions,
  PiNewSessionResult,
  PiPauseResult,
  PiProtectionAdmission,
  PiRpcDiagnostic,
  PiSettledObservation,
} from "./types.ts";

const DEFAULT_MAX_FRAME_BYTES = 1024 * 1024;

type JsonRecord = Record<string, unknown>;

interface PendingResponse {
  readonly command: string;
  readonly resolve: (response: Readonly<JsonRecord>) => void;
  readonly reject: (error: unknown) => void;
}

type PiSessionRuntimeOptions = Omit<PiManagedSessionOptions, "admission">;

export type PiManagedSessionErrorCode =
  | "INVALID_ADMISSION"
  | "PROTECTED_MODE_NOT_QUALIFIED"
  | "DISPATCH_PAUSED"
  | "PAUSE_IN_PROGRESS"
  | "SESSION_CLOSED"
  | "WRITE_FAILED"
  | "REMOTE_REJECTED"
  | "INVALID_RESPONSE"
  | "SETTLEMENT_TIMEOUT";

export class PiManagedSessionError extends Error {
  readonly code: PiManagedSessionErrorCode;

  constructor(code: PiManagedSessionErrorCode, message: string, cause?: unknown) {
    super(message, cause === undefined ? undefined : { cause });
    this.name = "PiManagedSessionError";
    this.code = code;
  }
}

const isRecord = (value: unknown): value is JsonRecord =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const deepFreeze = <T>(value: T): T => {
  if (typeof value !== "object" || value === null || Object.isFrozen(value)) return value;
  for (const child of Object.values(value)) deepFreeze(child);
  return Object.freeze(value);
};

const PI_PROTECTION_ADMISSION_FIELDS = Object.freeze([
  "mode",
  "protocolQualified",
  "protectedDomainQualified",
  "contextExposure",
] as const);

function invalidAdmission(detail: string): never {
  throw new PiManagedSessionError("INVALID_ADMISSION", `admission ${detail}`);
}

/**
 * Inspect an admission record without invoking caller code, then snapshot its
 * scalar fields exactly once.  The authority boundary must never read a
 * caller-supplied getter, Proxy, inherited field, or mutable alias.
 */
function passiveAdmissionRecord(value: unknown): Record<string, unknown> {
  if (
    typeof value !== "object" ||
    value === null ||
    NodeUtilTypes.isProxy(value) ||
    Array.isArray(value) ||
    Object.getPrototypeOf(value) !== Object.prototype
  ) {
    return invalidAdmission("must be a non-Proxy plain object");
  }

  const keys = Reflect.ownKeys(value);
  if (keys.some((key) => typeof key === "symbol")) {
    return invalidAdmission("must not contain symbol keys");
  }
  const names = keys as ReadonlyArray<string>;
  if (
    names.length !== PI_PROTECTION_ADMISSION_FIELDS.length ||
    names.some((name) => !PI_PROTECTION_ADMISSION_FIELDS.some((expected) => expected === name))
  ) {
    return invalidAdmission("must contain exactly the known fields");
  }

  const descriptors = Object.getOwnPropertyDescriptors(value);
  for (const name of PI_PROTECTION_ADMISSION_FIELDS) {
    const descriptor = descriptors[name];
    if (descriptor === undefined || !descriptor.enumerable || !("value" in descriptor)) {
      return invalidAdmission(`${name} must be an enumerable data property`);
    }
  }

  // All structure checks complete before any caller-owned value is read.
  const snapshot: Record<string, unknown> = {};
  for (const name of PI_PROTECTION_ADMISSION_FIELDS) {
    snapshot[name] = descriptors[name]?.value;
  }
  return snapshot;
}

export function snapshotPiProtectionAdmission(value: unknown): PiProtectionAdmission {
  const fields = passiveAdmissionRecord(value);
  const mode = fields.mode;
  if (mode !== "ordinary" && mode !== "protected") {
    return invalidAdmission("mode must be exactly ordinary or protected");
  }
  const protocolQualified = fields.protocolQualified;
  if (typeof protocolQualified !== "boolean") {
    return invalidAdmission("protocolQualified must be boolean");
  }
  const protectedDomainQualified = fields.protectedDomainQualified;
  if (typeof protectedDomainQualified !== "boolean") {
    return invalidAdmission("protectedDomainQualified must be boolean");
  }
  const contextExposure = fields.contextExposure;
  if (
    contextExposure !== "OBSERVED_LIMITED" &&
    contextExposure !== "POSSIBLE" &&
    contextExposure !== "UNKNOWN"
  ) {
    return invalidAdmission("contextExposure is not a known value");
  }
  return deepFreeze({
    mode,
    protocolQualified,
    protectedDomainQualified,
    contextExposure,
  });
}

const requireStringArray = (value: unknown, field: string): ReadonlyArray<string> => {
  if (!Array.isArray(value) || !value.every((entry) => typeof entry === "string")) {
    throw new PiManagedSessionError("INVALID_RESPONSE", `${field} must be a string array`);
  }
  return Object.freeze([...value]);
};

/**
 * Managed Pi RPC state machine. It only consumes/injects bytes; the host owns
 * process lifetime, account state, filesystem access, networking, and custody.
 */
export class PiManagedSession {
  readonly #options: PiSessionRuntimeOptions;
  readonly #framer: PiJsonlFramer;
  readonly #pending = new Map<string, PendingResponse>();
  readonly #completedIds = new Set<string>();
  #nextRequest = 0;
  #dispatchOpen = true;
  #closed = false;
  #writeTail: Promise<void> = Promise.resolve();
  #pauseFlight: Promise<PiPauseResult> | undefined;
  #pauseComplete = false;
  #exclusiveTaskUsed = false;
  #priorPromptCommand = false;
  #priorAgentRun = false;
  #settlement: {
    started: boolean;
    resolve: () => void;
    reject: (error: PiManagedSessionError) => void;
  } | undefined;

  constructor(options: PiManagedSessionOptions) {
    const admission = snapshotPiProtectionAdmission(options.admission);
    if (
      admission.mode === "protected" &&
      (!admission.protocolQualified ||
        !admission.protectedDomainQualified ||
        admission.contextExposure !== "OBSERVED_LIMITED")
    ) {
      throw new PiManagedSessionError(
        "PROTECTED_MODE_NOT_QUALIFIED",
        "protected Pi mode requires qualified protocol/domain isolation and observed-limited context",
      );
    }
    this.#options = {
      sink: options.sink,
      ...(options.maxFrameBytes === undefined ? {} : { maxFrameBytes: options.maxFrameBytes }),
      ...(options.onDiagnostic === undefined ? {} : { onDiagnostic: options.onDiagnostic }),
      ...(options.onEvent === undefined ? {} : { onEvent: options.onEvent }),
    };
    this.#framer = new PiJsonlFramer({
      maxFrameBytes: options.maxFrameBytes ?? DEFAULT_MAX_FRAME_BYTES,
      onFrame: (value) => this.#receive(value),
      onDiagnostic: (diagnostic) => this.#diagnostic(diagnostic),
    });
  }

  get dispatchOpen(): boolean {
    return this.#dispatchOpen && !this.#closed;
  }

  acceptStdout(chunk: Uint8Array): void {
    if (this.#closed) return;
    this.#framer.push(chunk);
  }

  prompt(message: string, streamingBehavior?: "steer" | "followUp"): Promise<PiAcceptedCommand> {
    const command: JsonRecord = { type: "prompt", message };
    if (streamingBehavior !== undefined) command.streamingBehavior = streamingBehavior;
    return this.#accepted("prompt", command);
  }

  /** One agent run per controlled task because Pi events have no request id. */
  async promptAndObserveSettlement(message: string, timeoutMs: number): Promise<PiSettledObservation> {
    this.#requireDispatch();
    if (this.#exclusiveTaskUsed || this.#priorPromptCommand || this.#priorAgentRun || this.#pending.size !== 0) {
      throw new PiManagedSessionError("INVALID_ADMISSION", "settlement requires a session without prior agent work");
    }
    if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) {
      throw new PiManagedSessionError("INVALID_ADMISSION", "settlement timeout must be positive");
    }
    let settle!: () => void;
    let rejectSettlement!: (error: PiManagedSessionError) => void;
    const observed = new Promise<void>((resolve, reject) => {
      settle = resolve;
      rejectSettlement = reject;
    });
    this.#settlement = { started: false, resolve: settle, reject: rejectSettlement };
    const accepted = this.#accepted("prompt", { type: "prompt", message });
    this.#exclusiveTaskUsed = true;
    const timer = setTimeout(() => {
      this.#settlement?.reject(new PiManagedSessionError("SETTLEMENT_TIMEOUT", "agent_settled was not observed"));
      this.#settlement = undefined;
    }, timeoutMs);
    try {
      const [command] = await Promise.all([accepted, observed]);
      return Object.freeze({ status: "protocol-settled-not-result" as const, accepted: command });
    } finally {
      clearTimeout(timer);
      this.#settlement = undefined;
    }
  }

  steer(message: string): Promise<PiAcceptedCommand> {
    return this.#accepted("steer", { type: "steer", message });
  }

  followUp(message: string): Promise<PiAcceptedCommand> {
    return this.#accepted("follow_up", { type: "follow_up", message });
  }

  newSession(parentSession?: string): Promise<PiNewSessionResult> {
    this.#requireDispatch();
    const command: JsonRecord = { type: "new_session" };
    if (parentSession !== undefined) command.parentSession = parentSession;
    return this.#send(command).then(async (response) => {
      const data = response.data;
      if (!isRecord(data) || typeof data.cancelled !== "boolean") {
        throw new PiManagedSessionError(
          "INVALID_RESPONSE",
          "new_session response must contain boolean data.cancelled",
        );
      }
      if (data.cancelled) return Object.freeze({ status: "cancelled" });

      const stateResponse = await this.#send({ type: "get_state" });
      const state = stateResponse.data;
      if (!isRecord(state) || typeof state.sessionId !== "string" || state.sessionId.length === 0) {
        throw new PiManagedSessionError(
          "INVALID_RESPONSE",
          "get_state did not return a non-empty sessionId",
        );
      }
      const candidate: {
        status: "candidate";
        sessionId: string;
        sessionFile?: string;
      } = { status: "candidate", sessionId: state.sessionId };
      if (typeof state.sessionFile === "string") candidate.sessionFile = state.sessionFile;
      return Object.freeze(candidate);
    });
  }

  /**
   * Seal dispatch admission synchronously, then serialize clear_queue → abort.
   * The result deliberately cannot be mistaken for process termination.
   */
  pause(): Promise<PiPauseResult> {
    if (this.#pauseFlight !== undefined) return this.#pauseFlight;
    if (this.#closed) return Promise.reject(this.#closedError());
    this.#dispatchOpen = false;
    const flight = (async (): Promise<PiPauseResult> => {
      const clear = await this.#send({ type: "clear_queue" }, true);
      const clearData = clear.data;
      if (!isRecord(clearData)) {
        throw new PiManagedSessionError("INVALID_RESPONSE", "clear_queue data must be an object");
      }
      const cleared = Object.freeze({
        steering: requireStringArray(clearData.steering, "data.steering"),
        followUp: requireStringArray(clearData.followUp, "data.followUp"),
      });
      await this.#send({ type: "abort" }, true);
      return Object.freeze({
        admission: "paused",
        cleared,
        sessionIdle: true,
        processStopped: false,
      });
    })();
    this.#pauseFlight = flight;
    void flight.then(
      () => {
        if (this.#pauseFlight === flight) this.#pauseComplete = true;
      },
      () => undefined,
    );
    return flight;
  }

  resumeDispatch(): void {
    if (this.#closed) throw this.#closedError();
    if (this.#pauseFlight === undefined) return;
    if (!this.#pauseComplete) {
      throw new PiManagedSessionError(
        "PAUSE_IN_PROGRESS",
        "cannot reopen Pi dispatch before clear_queue and abort finish",
      );
    }
    this.#pauseFlight = undefined;
    this.#pauseComplete = false;
    this.#dispatchOpen = true;
  }

  close(reason = "Pi RPC byte stream closed"): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#dispatchOpen = false;
    this.#framer.finish();
    const error = new PiManagedSessionError("SESSION_CLOSED", reason);
    for (const pending of this.#pending.values()) pending.reject(error);
    this.#pending.clear();
    this.#settlement?.reject(error);
    this.#settlement = undefined;
  }

  async #accepted(
    type: "prompt" | "steer" | "follow_up",
    command: JsonRecord,
  ): Promise<PiAcceptedCommand> {
    this.#requireDispatch();
    const response = await this.#send(command);
    return Object.freeze({ status: "accepted", requestId: String(response.id), command: type });
  }

  #send(command: JsonRecord, control = false): Promise<Readonly<JsonRecord>> {
    if (this.#closed) return Promise.reject(this.#closedError());
    if (!control) this.#requireDispatch();
    const type = command.type;
    if (typeof type !== "string" || type.length === 0) {
      return Promise.reject(
        new PiManagedSessionError("INVALID_RESPONSE", "outgoing command type is required"),
      );
    }
    if (type === "prompt" || type === "steer" || type === "follow_up") {
      this.#priorPromptCommand = true;
    }
    const id = `gogoke-pi-${++this.#nextRequest}`;
    const bytes = new TextEncoder().encode(`${JSON.stringify({ ...command, id })}\n`);
    const response = new Promise<Readonly<JsonRecord>>((resolve, reject) => {
      this.#pending.set(id, { command: type, resolve, reject });
    });
    const write = this.#writeTail.then(() => this.#options.sink.write(bytes));
    this.#writeTail = write.then(
      () => undefined,
      () => undefined,
    );
    void write.catch((error: unknown) => {
      const pending = this.#pending.get(id);
      if (pending === undefined) return;
      this.#pending.delete(id);
      pending.reject(new PiManagedSessionError("WRITE_FAILED", `failed to write ${type}`, error));
    });
    return response;
  }

  #receive(value: unknown): void {
    if (!isRecord(value) || typeof value.type !== "string") {
      this.#diagnostic({ code: "INVALID_MESSAGE", detail: "message must be an object with type" });
      return;
    }
    const frozen = deepFreeze(value);
    if (frozen.type !== "response") {
      if (frozen.type === "agent_start" || frozen.type === "agent_end" || frozen.type === "agent_settled") {
        this.#priorAgentRun = true;
      }
      if (frozen.type === "agent_start") {
        if (this.#settlement !== undefined) this.#settlement.started = true;
      } else if (frozen.type === "agent_settled" && this.#settlement?.started === true) {
        this.#settlement.resolve();
        this.#settlement = undefined;
      }
      this.#options.onEvent?.(frozen);
      return;
    }
    if (
      typeof frozen.id !== "string" ||
      typeof frozen.command !== "string" ||
      typeof frozen.success !== "boolean"
    ) {
      this.#diagnostic({
        code: "INVALID_MESSAGE",
        detail: "response requires string id/command and boolean success",
      });
      return;
    }
    if (this.#completedIds.has(frozen.id)) {
      this.#diagnostic({
        code: "DUPLICATE_RESPONSE_ID",
        detail: `duplicate response id ${frozen.id}`,
      });
      return;
    }
    const pending = this.#pending.get(frozen.id);
    if (pending === undefined) {
      this.#diagnostic({ code: "UNKNOWN_RESPONSE_ID", detail: `unknown response id ${frozen.id}` });
      return;
    }
    this.#pending.delete(frozen.id);
    this.#completedIds.add(frozen.id);
    if (pending.command !== frozen.command) {
      const error = new PiManagedSessionError(
        "INVALID_RESPONSE",
        `response command ${frozen.command} does not match ${pending.command}`,
      );
      pending.reject(error);
      this.#diagnostic({
        code: "RESPONSE_COMMAND_MISMATCH",
        detail: error.message,
      });
      return;
    }
    if (!frozen.success) {
      pending.reject(
        new PiManagedSessionError(
          "REMOTE_REJECTED",
          typeof frozen.error === "string" ? frozen.error : `${frozen.command} was rejected`,
        ),
      );
      return;
    }
    pending.resolve(frozen);
  }

  #diagnostic(diagnostic: PiRpcDiagnostic): void {
    this.#options.onDiagnostic?.(Object.freeze({ ...diagnostic }));
  }

  #requireDispatch(): void {
    if (this.#closed) throw this.#closedError();
    if (this.#exclusiveTaskUsed) {
      throw new PiManagedSessionError("DISPATCH_PAUSED", "exclusive Pi task has consumed this session");
    }
    if (!this.#dispatchOpen) {
      throw new PiManagedSessionError("DISPATCH_PAUSED", "Pi dispatch admission is paused");
    }
  }

  #closedError(): PiManagedSessionError {
    return new PiManagedSessionError("SESSION_CLOSED", "Pi RPC session is closed");
  }
}
