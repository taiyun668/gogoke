import { createHash } from "node:crypto";
import { types as utilTypes } from "node:util";
import {
  HOST_STOP_DEADLINE_MS,
  PRODUCTION_STOP_BUDGETS,
  STOP_REFUSED_EXIT_CODE,
  STOP_TIMEOUT_EXIT_CODE,
  type CustodyRecord,
  type DurableCustodyStore,
  type LaunchRequest,
  type NativeBinding,
  type NativeCustodyAdapter,
  type NativePrepareRequest,
  type NativeStopProof,
  type PreparedCustody,
  type ProcessIdentity,
  type StopBudgets,
  stopBudgetsFitHostDeadline,
} from "./model.ts";

const RELEASED_PHASE = "CONFIRMED" as const;

export class CustodyService {
  readonly #native: NativeCustodyAdapter;
  readonly #store: DurableCustodyStore;

  constructor(native: NativeCustodyAdapter, store: DurableCustodyStore) {
    this.#native = Object.freeze({
      prepare: native.prepare.bind(native),
      activate: native.activate.bind(native),
      abortPrepared: native.abortPrepared.bind(native),
      stop: native.stop.bind(native),
      confirmStopped: native.confirmStopped.bind(native),
    });
    this.#store = Object.freeze({
      reservePrepared: store.reservePrepared.bind(store),
      read: store.read.bind(store),
      compareAndSet: store.compareAndSet.bind(store),
      saveStoppedProof: store.saveStoppedProof.bind(store),
    });
    Object.freeze(this);
  }

  async launch(rawRequest: unknown, timeoutMs = HOST_STOP_DEADLINE_MS): Promise<CustodyRecord> {
    const request = snapshotLaunchRequest(rawRequest);
    const deadline = deadlineAfter(timeoutMs);
    const prepared = snapshotPrepared(
      await bounded("NATIVE_PREPARE_DEADLINE", deadline, (signal) =>
        this.#native.prepare(nativeRequest(request), signal),
      ),
    );
    assertBindingEquals(prepared.binding, request.binding, "NATIVE_PREPARE_BINDING_MISMATCH");

    let reservation;
    try {
      reservation = await boundedPromise(
        "PREPARED_RESERVATION_DEADLINE",
        deadline,
        this.#store.reservePrepared(prepared, request.writerDomain),
      );
    } catch (error) {
      await bestEffortAbort(this.#native, prepared, deadline);
      throw error;
    }
    if (reservation.status === "writer-conflict") {
      await bestEffortAbort(this.#native, prepared, deadline);
      throw new Error("WRITER_DOMAIN_ALREADY_RESERVED");
    }
    const reserved = snapshotRecord(reservation.record);
    assertPreparedEquals(reserved, prepared, "PREPARED_RESERVATION_MISMATCH");
    if (reserved.phase !== "PREPARED" || reserved.writerDomain !== request.writerDomain) {
      await bestEffortAbort(this.#native, prepared, deadline);
      throw new Error("PREPARED_RESERVATION_INVALID");
    }
    assertRecordEquals(
      reserved,
      snapshotRecord({
        ...prepared,
        writerDomain: request.writerDomain,
        phase: "PREPARED",
        revision: 1,
        stopAttempt: 0,
        leaseState: "held",
        stopProof: null,
        errors: [],
      }),
      "PREPARED_RESERVATION_RECORD_MISMATCH",
    );

    try {
      await bounded("NATIVE_ACTIVATE_DEADLINE", deadline, (signal) =>
        this.#native.activate(prepared, signal),
      );
    } catch (error) {
      await bestEffortAbort(this.#native, prepared, deadline);
      await bestEffortResidual(this.#store, reserved, "NATIVE_ACTIVATE_FAILED", deadline);
      throw error;
    }
    const active = snapshotRecord({
      ...reserved,
      phase: "ACTIVE",
      revision: reserved.revision + 1,
    });
    const durableActive = snapshotRecord(
      await boundedPromise(
        "ACTIVE_CAS_DEADLINE",
        deadline,
        this.#store.compareAndSet(active, reserved.revision),
      ),
    );
    assertRecordEquals(durableActive, active, "ACTIVE_CAS_RESULT_MISMATCH");
    return durableActive;
  }

  async stop(
    rawTicket: unknown,
    rawBudgets: unknown = PRODUCTION_STOP_BUDGETS,
  ): Promise<CustodyRecord> {
    const ticket = snapshotString(rawTicket, "ticket", false);
    const budgets = snapshotStopBudgets(rawBudgets);
    const deadline = deadlineAfter(budgets.hostDeadlineMs);
    const initial = snapshotRecord(
      await boundedPromise("CUSTODY_READ_DEADLINE", deadline, this.#store.read(ticket)),
    );
    if (initial.ticket !== ticket) throw new Error("CUSTODY_TICKET_MISMATCH");
    if (initial.phase === RELEASED_PHASE) return initial;
    if (initial.phase === "STOPPED_PENDING_CONFIRM") {
      return await this.#confirmSavedProof(initial, deadline);
    }
    if (initial.stopAttempt > 0 || initial.phase === "STOPPING") {
      return await this.#writeResidual(
        initial,
        "SECOND_STOP_REQUIRES_DURABLE_RECONCILIATION",
        deadline,
      );
    }

    const stopping = snapshotRecord({
      ...initial,
      phase: "STOPPING",
      revision: initial.revision + 1,
      stopAttempt: initial.stopAttempt + 1,
    });
    const durableStopping = snapshotRecord(
      await boundedPromise(
        "STOP_INTENT_CAS_DEADLINE",
        deadline,
        this.#store.compareAndSet(stopping, initial.revision),
      ),
    );
    assertRecordEquals(durableStopping, stopping, "STOP_INTENT_CAS_RESULT_MISMATCH");

    let proof: NativeStopProof;
    try {
      proof = snapshotStopProof(
        await bounded("NATIVE_STOP_DEADLINE", deadline, (signal) =>
          this.#native.stop(preparedFromRecord(durableStopping), budgets, signal),
        ),
      );
    } catch (error) {
      if (isDeadlineError(error))
        return residualView(durableStopping, "HOST_STOP_DEADLINE_EXCEEDED");
      return await this.#writeResidual(
        durableStopping,
        "NATIVE_STOP_FAILED_WITHOUT_PROOF",
        deadline,
      );
    }
    assertPreparedEquals(proof, durableStopping, "NATIVE_STOP_PROOF_BINDING_MISMATCH");
    const proofErrors = stopProofErrors(proof);
    if (proofErrors.length > 0) {
      return await this.#writeResidual(durableStopping, proofErrors, deadline, proof);
    }

    const stopped = snapshotRecord(
      await boundedPromise(
        "STOP_PROOF_CAS_DEADLINE",
        deadline,
        this.#store.saveStoppedProof(ticket, durableStopping.revision, proof),
      ),
    );
    if (stopped.phase !== "STOPPED_PENDING_CONFIRM" || stopped.stopProof === null) {
      return residualView(durableStopping, "DURABLE_STOPPED_PROOF_MISSING");
    }
    const savedProof = snapshotStopProof(stopped.stopProof);
    assertProofEquals(savedProof, proof, "DURABLE_STOPPED_PROOF_MISMATCH");
    assertRecordEquals(
      stopped,
      snapshotRecord({
        ...durableStopping,
        phase: "STOPPED_PENDING_CONFIRM",
        revision: durableStopping.revision + 1,
        stopProof: proof,
      }),
      "DURABLE_STOPPED_RECORD_MISMATCH",
    );
    return await this.#confirmSavedProof(stopped, deadline);
  }

  async markLeaseLost(rawTicket: unknown): Promise<CustodyRecord> {
    const ticket = snapshotString(rawTicket, "ticket", false);
    const deadline = deadlineAfter(HOST_STOP_DEADLINE_MS);
    const current = snapshotRecord(
      await boundedPromise("CUSTODY_READ_DEADLINE", deadline, this.#store.read(ticket)),
    );
    if (current.phase === RELEASED_PHASE) return current;
    const residual = snapshotRecord({
      ...current,
      phase: "RESIDUAL",
      revision: current.revision + 1,
      leaseState: "lost",
      errors: [...current.errors, "LEASE_LOST_PROCESS_STATE_UNKNOWN"],
    });
    const durableResidual = snapshotRecord(
      await boundedPromise(
        "LEASE_LOST_CAS_DEADLINE",
        deadline,
        this.#store.compareAndSet(residual, current.revision),
      ),
    );
    assertRecordEquals(durableResidual, residual, "LEASE_LOST_CAS_RESULT_MISMATCH");
    return durableResidual;
  }

  async #confirmSavedProof(record: CustodyRecord, deadline: number): Promise<CustodyRecord> {
    if (record.phase !== "STOPPED_PENDING_CONFIRM" || record.stopProof === null) {
      return residualView(record, "DURABLE_STOPPED_PROOF_MISSING");
    }
    const proof = snapshotStopProof(record.stopProof);
    assertPreparedEquals(proof, record, "DURABLE_STOPPED_PROOF_BINDING_MISMATCH");
    if (stopProofErrors(proof).length > 0) return residualView(record, stopProofErrors(proof));
    await bounded("NATIVE_CONFIRM_DEADLINE", deadline, (signal) =>
      this.#native.confirmStopped(
        {
          ticket: record.ticket,
          custodianNonce: record.custodianNonce,
          identity: record.identity,
          proofHash: hashStopProof(proof),
          durableRevision: record.revision,
        },
        signal,
      ),
    );
    const confirmed = snapshotRecord({
      ...record,
      phase: RELEASED_PHASE,
      revision: record.revision + 1,
    });
    const durableConfirmed = snapshotRecord(
      await boundedPromise(
        "STOP_CONFIRM_CAS_DEADLINE",
        deadline,
        this.#store.compareAndSet(confirmed, record.revision),
      ),
    );
    assertRecordEquals(durableConfirmed, confirmed, "STOP_CONFIRM_CAS_RESULT_MISMATCH");
    return durableConfirmed;
  }

  async #writeResidual(
    record: CustodyRecord,
    errors: string | ReadonlyArray<string>,
    deadline: number,
    proof: NativeStopProof | null = null,
  ): Promise<CustodyRecord> {
    const next = snapshotRecord({
      ...record,
      phase: "RESIDUAL",
      revision: record.revision + 1,
      stopProof: proof,
      errors: [...record.errors, ...(typeof errors === "string" ? [errors] : errors)],
    });
    try {
      const durableResidual = snapshotRecord(
        await boundedPromise(
          "RESIDUAL_CAS_DEADLINE",
          deadline,
          this.#store.compareAndSet(next, record.revision),
        ),
      );
      assertRecordEquals(durableResidual, next, "RESIDUAL_CAS_RESULT_MISMATCH");
      return durableResidual;
    } catch (error) {
      if (isDeadlineError(error)) return residualView(record, "HOST_STOP_DEADLINE_EXCEEDED", proof);
      throw error;
    }
  }
}

export function stopProofErrors(proof: NativeStopProof): ReadonlyArray<string> {
  const errors = [...proof.errors];
  if (!proof.processHandlePresent || !proof.jobHandlePresent) errors.push("CUSTODY_HANDLE_MISSING");
  if (proof.identityStatus !== "exact")
    errors.push(`PROCESS_IDENTITY_${proof.identityStatus.toUpperCase()}`);
  if (proof.killAttempted && !proof.killSucceeded) errors.push("PROCESS_TREE_STOP_UNPROVEN");
  if (!proof.parentExited) errors.push("PARENT_PROCESS_REMAINS");
  if (proof.activeJobProcesses !== 0) errors.push("JOB_DESCENDANTS_REMAIN");
  if (!proof.writerFenceVerified) errors.push("WRITER_FENCE_UNVERIFIED");
  if (proof.deadlineExceeded) errors.push("HOST_STOP_DEADLINE_EXCEEDED");
  if (proof.exitCode === STOP_TIMEOUT_EXIT_CODE)
    errors.push("STOP_EXIT_124_REQUIRES_RECONCILIATION");
  if (proof.exitCode === STOP_REFUSED_EXIT_CODE)
    errors.push("STOP_EXIT_125_REQUIRES_RECONCILIATION");
  return Object.freeze([...new Set(errors)]);
}

export function hashStopProof(rawProof: unknown): string {
  const proof = snapshotStopProof(rawProof);
  const fields = [
    proof.ticket,
    proof.custodianNonce,
    proof.binding.binaryDigestSha256,
    proof.binding.profileId,
    proof.binding.domainId,
    proof.binding.generation,
    String(proof.identity.pid),
    proof.identity.creationTime100ns,
    proof.identity.imagePath,
    String(proof.parentExited),
    proof.activeJobProcesses === null ? "null" : String(proof.activeJobProcesses),
    proof.identityStatus,
    String(proof.processHandlePresent),
    String(proof.jobHandlePresent),
    String(proof.killAttempted),
    String(proof.killSucceeded),
    String(proof.writerFenceVerified),
    proof.exitCode === null ? "null" : String(proof.exitCode),
    String(proof.deadlineExceeded),
    ...proof.errors,
  ];
  const hash = createHash("sha256");
  for (const field of fields) {
    const bytes = Buffer.from(field, "utf8");
    const length = Buffer.allocUnsafe(8);
    length.writeBigUInt64LE(BigInt(bytes.length));
    hash.update(length).update(bytes);
  }
  return `sha256:${hash.digest("hex")}`;
}

function nativeRequest(request: LaunchRequest): NativePrepareRequest {
  return deepFreeze({
    application: request.application,
    arguments: [...request.arguments],
    currentDirectory: request.currentDirectory,
    hideWindow: request.hideWindow,
    binding: request.binding,
  });
}

function preparedFromRecord(record: CustodyRecord): PreparedCustody {
  return deepFreeze({
    ticket: record.ticket,
    custodianNonce: record.custodianNonce,
    binding: record.binding,
    identity: record.identity,
  });
}

async function bestEffortAbort(
  native: NativeCustodyAdapter,
  prepared: PreparedCustody,
  deadline: number,
): Promise<void> {
  await bounded("NATIVE_ABORT_DEADLINE", deadline, (signal) =>
    native.abortPrepared(prepared, signal),
  ).catch(() => undefined);
}

async function bestEffortResidual(
  store: DurableCustodyStore,
  record: CustodyRecord,
  error: string,
  deadline: number,
): Promise<void> {
  const residual = snapshotRecord({
    ...record,
    phase: "RESIDUAL",
    revision: record.revision + 1,
    errors: [...record.errors, error],
  });
  await boundedPromise(
    "RESIDUAL_CAS_DEADLINE",
    deadline,
    store.compareAndSet(residual, record.revision),
  ).catch(() => undefined);
}

function deadlineAfter(timeoutMs: number): number {
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0 || timeoutMs > HOST_STOP_DEADLINE_MS) {
    throw new Error("CUSTODY_DEADLINE_INVALID");
  }
  return performance.now() + timeoutMs;
}

async function bounded<T>(
  code: string,
  deadline: number,
  operation: (signal: AbortSignal) => Promise<T>,
): Promise<T> {
  const controller = new AbortController();
  return await boundedPromise(code, deadline, operation(controller.signal), () =>
    controller.abort(),
  );
}

async function boundedPromise<T>(
  code: string,
  deadline: number,
  operation: Promise<T>,
  onTimeout: () => void = () => undefined,
): Promise<T> {
  const remaining = Math.max(0, Math.ceil(deadline - performance.now()));
  if (remaining === 0) {
    onTimeout();
    throw new DeadlineError(code);
  }
  let timer: NodeJS.Timeout | undefined;
  const timeout = new Promise<never>((_resolve, reject) => {
    timer = setTimeout(() => {
      onTimeout();
      reject(new DeadlineError(code));
    }, remaining);
    timer.unref();
  });
  try {
    return await Promise.race([operation, timeout]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

class DeadlineError extends Error {
  constructor(code: string) {
    super(code);
    this.name = "DeadlineError";
  }
}

function isDeadlineError(error: unknown): error is DeadlineError {
  return error instanceof DeadlineError;
}

function snapshotLaunchRequest(value: unknown): LaunchRequest {
  const fields = passiveObject(value, [
    "application",
    "arguments",
    "currentDirectory",
    "hideWindow",
    "binding",
    "writerDomain",
  ]);
  return deepFreeze({
    application: snapshotAbsoluteWindowsPath(fields.application, "application"),
    arguments: snapshotStringArray(fields.arguments, "arguments"),
    currentDirectory:
      fields.currentDirectory === null
        ? null
        : snapshotAbsoluteWindowsPath(fields.currentDirectory, "currentDirectory"),
    hideWindow: snapshotBoolean(fields.hideWindow, "hideWindow"),
    binding: snapshotBinding(fields.binding),
    writerDomain: snapshotString(fields.writerDomain, "writerDomain", false),
  });
}

function snapshotStopBudgets(value: unknown): StopBudgets {
  const fields = passiveObject(value, ["graceMs", "terminateMs", "observeMs", "hostDeadlineMs"]);
  const budgets = deepFreeze({
    graceMs: snapshotSafeInteger(fields.graceMs, "graceMs", 0),
    terminateMs: snapshotSafeInteger(fields.terminateMs, "terminateMs", 0),
    observeMs: snapshotSafeInteger(fields.observeMs, "observeMs", 0),
    hostDeadlineMs: snapshotSafeInteger(fields.hostDeadlineMs, "hostDeadlineMs", 1),
  });
  if (!stopBudgetsFitHostDeadline(budgets)) throw new Error("STOP_BUDGETS_EXCEED_HOST_DEADLINE");
  return budgets;
}

function snapshotBinding(value: unknown): NativeBinding {
  const fields = passiveObject(value, [
    "binaryDigestSha256",
    "profileId",
    "domainId",
    "generation",
  ]);
  const digest = snapshotString(
    fields.binaryDigestSha256,
    "binaryDigestSha256",
    false,
  ).toLowerCase();
  if (!/^sha256:[0-9a-f]{64}$/.test(digest)) throw new Error("BINARY_DIGEST_INVALID");
  const generation = snapshotString(fields.generation, "generation", false);
  if (!/^[1-9]\d*$/.test(generation)) throw new Error("GENERATION_INVALID");
  return deepFreeze({
    binaryDigestSha256: digest,
    profileId: snapshotString(fields.profileId, "profileId", false),
    domainId: snapshotString(fields.domainId, "domainId", false),
    generation,
  });
}

function snapshotIdentity(value: unknown): ProcessIdentity {
  const fields = passiveObject(value, ["pid", "creationTime100ns", "imagePath"]);
  const creationTime100ns = snapshotString(fields.creationTime100ns, "creationTime100ns", false);
  if (!/^\d+$/.test(creationTime100ns)) throw new Error("PROCESS_CREATION_TIME_INVALID");
  return deepFreeze({
    pid: snapshotSafeInteger(fields.pid, "pid", 1),
    creationTime100ns,
    imagePath: snapshotAbsoluteWindowsPath(fields.imagePath, "imagePath"),
  });
}

function snapshotPrepared(value: unknown): PreparedCustody {
  const fields = passiveObject(value, ["ticket", "custodianNonce", "binding", "identity"]);
  const ticket = snapshotString(fields.ticket, "ticket", false);
  const custodianNonce = snapshotString(fields.custodianNonce, "custodianNonce", false);
  if (!/^pct1_[0-9a-f]{64}$/.test(ticket)) throw new Error("NATIVE_TICKET_INVALID");
  if (!/^pcn1_[0-9a-f]{64}$/.test(custodianNonce)) throw new Error("CUSTODIAN_NONCE_INVALID");
  return deepFreeze({
    ticket,
    custodianNonce,
    binding: snapshotBinding(fields.binding),
    identity: snapshotIdentity(fields.identity),
  });
}

function snapshotStopProof(value: unknown): NativeStopProof {
  const fields = passiveObject(value, [
    "ticket",
    "custodianNonce",
    "binding",
    "identity",
    "parentExited",
    "activeJobProcesses",
    "identityStatus",
    "processHandlePresent",
    "jobHandlePresent",
    "killAttempted",
    "killSucceeded",
    "writerFenceVerified",
    "exitCode",
    "deadlineExceeded",
    "errors",
  ]);
  const prepared = snapshotPrepared({
    ticket: fields.ticket,
    custodianNonce: fields.custodianNonce,
    binding: fields.binding,
    identity: fields.identity,
  });
  const identityStatus = snapshotString(fields.identityStatus, "identityStatus", false);
  if (!(["exact", "unknown", "mismatch"] as const).includes(identityStatus as never)) {
    throw new Error("IDENTITY_STATUS_INVALID");
  }
  return deepFreeze({
    ...prepared,
    parentExited: snapshotBoolean(fields.parentExited, "parentExited"),
    activeJobProcesses:
      fields.activeJobProcesses === null
        ? null
        : snapshotSafeInteger(fields.activeJobProcesses, "activeJobProcesses", 0),
    identityStatus: identityStatus as NativeStopProof["identityStatus"],
    processHandlePresent: snapshotBoolean(fields.processHandlePresent, "processHandlePresent"),
    jobHandlePresent: snapshotBoolean(fields.jobHandlePresent, "jobHandlePresent"),
    killAttempted: snapshotBoolean(fields.killAttempted, "killAttempted"),
    killSucceeded: snapshotBoolean(fields.killSucceeded, "killSucceeded"),
    writerFenceVerified: snapshotBoolean(fields.writerFenceVerified, "writerFenceVerified"),
    exitCode: fields.exitCode === null ? null : snapshotSafeInteger(fields.exitCode, "exitCode", 0),
    deadlineExceeded: snapshotBoolean(fields.deadlineExceeded, "deadlineExceeded"),
    errors: snapshotStringArray(fields.errors, "errors"),
  });
}

function snapshotRecord(value: unknown): CustodyRecord {
  if (value === null) throw new Error("CUSTODY_RECORD_NOT_FOUND");
  const fields = passiveObject(value, [
    "ticket",
    "custodianNonce",
    "binding",
    "identity",
    "writerDomain",
    "phase",
    "revision",
    "stopAttempt",
    "leaseState",
    "stopProof",
    "errors",
  ]);
  const prepared = snapshotPrepared({
    ticket: fields.ticket,
    custodianNonce: fields.custodianNonce,
    binding: fields.binding,
    identity: fields.identity,
  });
  const phase = snapshotString(fields.phase, "phase", false);
  if (
    ![
      "PREPARED",
      "ACTIVE",
      "STOPPING",
      "STOPPED_PENDING_CONFIRM",
      "CONFIRMED",
      "RESIDUAL",
    ].includes(phase)
  ) {
    throw new Error("CUSTODY_PHASE_INVALID");
  }
  const leaseState = snapshotString(fields.leaseState, "leaseState", false);
  if (leaseState !== "held" && leaseState !== "lost") throw new Error("LEASE_STATE_INVALID");
  return deepFreeze({
    ...prepared,
    writerDomain: snapshotString(fields.writerDomain, "writerDomain", false),
    phase: phase as CustodyRecord["phase"],
    revision: snapshotSafeInteger(fields.revision, "revision", 1),
    stopAttempt: snapshotSafeInteger(fields.stopAttempt, "stopAttempt", 0),
    leaseState,
    stopProof: fields.stopProof === null ? null : snapshotStopProof(fields.stopProof),
    errors: snapshotStringArray(fields.errors, "errors"),
  });
}

function passiveObject(
  value: unknown,
  expectedKeys: ReadonlyArray<string>,
): Record<string, unknown> {
  if (typeof value !== "object" || value === null || utilTypes.isProxy(value)) {
    throw new Error("PASSIVE_PLAIN_OBJECT_REQUIRED");
  }
  if (Object.getPrototypeOf(value) !== Object.prototype) throw new Error("PLAIN_OBJECT_REQUIRED");
  if (Object.getOwnPropertySymbols(value).length !== 0) throw new Error("SYMBOL_FIELDS_FORBIDDEN");
  const descriptors = Object.getOwnPropertyDescriptors(value);
  const keys = Object.keys(descriptors).sort();
  const expected = [...expectedKeys].sort();
  if (keys.length !== expected.length || keys.some((key, index) => key !== expected[index])) {
    throw new Error("OBJECT_FIELDS_INVALID");
  }
  const output: Record<string, unknown> = {};
  for (const key of expectedKeys) {
    const descriptor = descriptors[key];
    if (descriptor === undefined || !("value" in descriptor) || descriptor.get || descriptor.set) {
      throw new Error("ACCESSOR_FIELDS_FORBIDDEN");
    }
    output[key] = descriptor.value;
  }
  return output;
}

function snapshotString(value: unknown, field: string, allowEmpty: boolean): string {
  if (typeof value !== "string" || (!allowEmpty && value.length === 0) || value.length > 4096) {
    throw new Error(`${field.toUpperCase()}_INVALID`);
  }
  return value;
}

function snapshotStringArray(value: unknown, field: string): ReadonlyArray<string> {
  if (
    !Array.isArray(value) ||
    utilTypes.isProxy(value) ||
    Object.getPrototypeOf(value) !== Array.prototype ||
    Object.getOwnPropertySymbols(value).length !== 0
  ) {
    throw new Error(`${field.toUpperCase()}_INVALID`);
  }
  const descriptors = Object.getOwnPropertyDescriptors(value);
  const keys = Object.keys(descriptors);
  const lengthDescriptor = Object.getOwnPropertyDescriptor(value, "length");
  if (lengthDescriptor === undefined || !("value" in lengthDescriptor)) {
    throw new Error(`${field.toUpperCase()}_LENGTH_INVALID`);
  }
  const length = snapshotSafeInteger(lengthDescriptor.value, `${field}_length`, 0);
  const expected = Array.from({ length }, (_item, index) => String(index))
    .concat("length")
    .sort();
  if (
    keys.sort().some((key, index) => key !== expected[index]) ||
    keys.length !== expected.length
  ) {
    throw new Error(`${field.toUpperCase()}_FIELDS_INVALID`);
  }
  const copy = Array.from({ length }, (_item, index) => {
    const descriptor = descriptors[String(index)];
    if (descriptor === undefined || !("value" in descriptor))
      throw new Error("ARRAY_ACCESSOR_FORBIDDEN");
    return snapshotString(descriptor.value, `${field}_${index}`, true);
  });
  return Object.freeze(copy);
}

function snapshotBoolean(value: unknown, field: string): boolean {
  if (typeof value !== "boolean") throw new Error(`${field.toUpperCase()}_INVALID`);
  return value;
}

function snapshotSafeInteger(value: unknown, field: string, minimum: number): number {
  if (!Number.isSafeInteger(value) || (value as number) < minimum) {
    throw new Error(`${field.toUpperCase()}_INVALID`);
  }
  return value as number;
}

function snapshotAbsoluteWindowsPath(value: unknown, field: string): string {
  const path = snapshotString(value, field, false);
  if (!/^(?:[a-zA-Z]:\\|\\\\)/.test(path) || path.includes("\0")) {
    throw new Error(`${field.toUpperCase()}_NOT_ABSOLUTE_WINDOWS_PATH`);
  }
  return path;
}

function deepFreeze<T>(value: T): T {
  if (typeof value === "object" && value !== null && !Object.isFrozen(value)) {
    for (const item of Object.values(value as Record<string, unknown>)) deepFreeze(item);
    Object.freeze(value);
  }
  return value;
}

function assertPreparedEquals(left: PreparedCustody, right: PreparedCustody, code: string): void {
  const project = (value: PreparedCustody) =>
    snapshotPrepared({
      ticket: value.ticket,
      custodianNonce: value.custodianNonce,
      binding: value.binding,
      identity: value.identity,
    });
  if (JSON.stringify(project(left)) !== JSON.stringify(project(right))) throw new Error(code);
}

function assertBindingEquals(left: NativeBinding, right: NativeBinding, code: string): void {
  if (JSON.stringify(snapshotBinding(left)) !== JSON.stringify(snapshotBinding(right)))
    throw new Error(code);
}

function assertProofEquals(left: NativeStopProof, right: NativeStopProof, code: string): void {
  if (hashStopProof(left) !== hashStopProof(right)) throw new Error(code);
}

function assertRecordEquals(left: CustodyRecord, right: CustodyRecord, code: string): void {
  if (JSON.stringify(snapshotRecord(left)) !== JSON.stringify(snapshotRecord(right))) {
    throw new Error(code);
  }
}

function residualView(
  record: CustodyRecord,
  errors: string | ReadonlyArray<string>,
  proof: NativeStopProof | null = null,
): CustodyRecord {
  return snapshotRecord({
    ...record,
    phase: "RESIDUAL",
    revision: record.revision + 1,
    stopProof: proof,
    errors: [...record.errors, ...(typeof errors === "string" ? [errors] : errors)],
  });
}
