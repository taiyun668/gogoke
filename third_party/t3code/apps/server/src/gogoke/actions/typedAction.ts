import * as NodeCrypto from "node:crypto";
import * as NodeUtilTypes from "node:util/types";
import { cloneAndFreezeJson } from "../runtimeCatalog/manifest.ts";
import type { AuthorizedTaskPackage } from "../policy/types.ts";

export const ACTION_INTENT_SCHEMA = "gogoke.typed-action.v1" as const;

const OPERATION_ID = /^opr_[0-9a-f]{32}$/;
const SHA256_DIGEST = /^sha256:[0-9a-f]{64}$/;
const U64 = /^(?:0|[1-9][0-9]*)$/;
const SAFE_ID = /^[A-Za-z][A-Za-z0-9._:@/-]{0,255}$/;
const CONTROL_QUEUE_LIMIT = 64;
const WORK_QUEUE_LIMIT = 128;

export interface ActionBinding {
  readonly bindingId: string;
  readonly sessionId: string;
  readonly executionId: string;
  readonly runtimeInstanceId: string;
  readonly profileId: string;
  readonly authRevision: string;
  readonly generation: string;
}

export type TypedAction =
  | {
      readonly kind: "prompt";
      readonly delivery: "queue" | "steer";
      readonly text: string;
    }
  | { readonly kind: "cancel" }
  | { readonly kind: "stop"; readonly reason: string };

export interface TypedActionIntent {
  readonly schema: typeof ACTION_INTENT_SCHEMA;
  readonly operationId: string;
  readonly semanticDigest: string;
  readonly binding: ActionBinding;
  readonly taskPackage: AuthorizedTaskPackage;
  readonly action: TypedAction;
}

export interface ActionCommitment {
  readonly packageDigest: string;
  readonly parentGrantRef: string;
  readonly parentGrantRevision: string;
  readonly parentCeilingDigest: string;
  readonly childCeilingDigest: string;
  readonly sourcePrincipalId: string;
  readonly sourceProjectId: string;
  readonly sourceDomainId: string;
  readonly sourceRole: string;
  readonly targetPrincipalId: string;
  readonly targetProjectId: string;
  readonly targetDomainId: string;
  readonly targetRole: string;
  readonly sourceSessionId: string;
  readonly sourceExecutionId: string;
  readonly sourceGeneration: string;
  readonly childSessionId: string;
  readonly childExecutionId: string;
  readonly childGeneration: string;
  readonly route: string;
  readonly policyAction: string;
  readonly sink: string;
  readonly materialSetDigest: string;
  readonly instructionDigest: string;
}

export type ActionLane = "control" | "work";

/** The only shapes that may cross the provider adapter seam. */
export type MappedNativeAction =
  | { readonly kind: "queue"; readonly text: string }
  | { readonly kind: "steer"; readonly text: string }
  | { readonly kind: "interrupt" }
  | { readonly kind: "close"; readonly reason: string };

export interface DurableActionReservation {
  readonly schema: typeof ACTION_INTENT_SCHEMA;
  readonly operationId: string;
  readonly semanticDigest: string;
  readonly binding: ActionBinding;
  readonly commitment: ActionCommitment;
  readonly lane: ActionLane;
  readonly action: MappedNativeAction;
}

export type DurableActionState =
  | "reserved"
  | "dispatching"
  | "legacy-unknown"
  | "not-sent"
  | "dispatched"
  | "rejected"
  | "outcome-unknown"
  | "completed";

export type ReserveActionResult =
  | {
      readonly kind: "reserved";
      readonly reservationId: string;
      readonly operationId: string;
      readonly semanticDigest: string;
    }
  | {
      readonly kind: "replay";
      readonly reservationId: string;
      readonly operationId: string;
      readonly semanticDigest: string;
      readonly state: DurableActionState;
    }
  | {
      readonly kind: "conflict";
      readonly operationId: string;
      readonly existingSemanticDigest: string;
    }
  | { readonly kind: "unknown"; readonly operationId: string };

export type BeginActionResult =
  | {
      readonly kind: "granted";
      readonly operationId: string;
      readonly reservationId: string;
      readonly sendAuthority: string;
    }
  | {
      readonly kind: "replay";
      readonly operationId: string;
      readonly reservationId: string;
      readonly state: DurableActionState;
    }
  | { readonly kind: "conflict"; readonly operationId: string; readonly reservationId: string }
  | { readonly kind: "unknown"; readonly operationId: string; readonly reservationId: string };

export type DurableDispatchOutcome =
  | { readonly kind: "not-sent"; readonly reason: "STALE_BINDING" }
  | { readonly kind: "dispatched"; readonly receiptRef: string | null }
  | { readonly kind: "rejected"; readonly reason: string }
  | {
      readonly kind: "outcome-unknown";
      readonly reason: "EOF" | "ERROR" | "TRANSPORT_THROW";
      readonly detail: string;
    };

/**
 * This is an injected interface to the shared durable authority. Implementations
 * must not resolve `reserve` until the operation identity and digest are durable.
 * The action seam deliberately owns no fallback or second persistence store.
 */
export interface DurableActionStore {
  reserve(reservation: DurableActionReservation): Promise<ReserveActionResult>;
  begin(reservationId: string, reservation: DurableActionReservation): Promise<BeginActionResult>;
  recordDispatchOutcome(
    reservationId: string,
    operationId: string,
    semanticDigest: string,
    outcome: DurableDispatchOutcome,
  ): Promise<void>;
}

export type ActionTransportResult =
  | { readonly kind: "accepted"; readonly receiptRef?: string }
  | { readonly kind: "rejected"; readonly reason: string }
  | { readonly kind: "eof" }
  | { readonly kind: "error"; readonly detail: string };

export interface TypedActionTransport {
  send(action: MappedNativeAction, binding: ActionBinding): Promise<ActionTransportResult>;
}

export interface ActionBindingAuthority {
  currentBinding(bindingId: string): Promise<ActionBinding | null>;
  /**
   * Preparatory authority-commit seam. Native Begin persists and compares the
   * returned tuple exactly, but the current PolicyAuthorityPort is not yet in
   * the native transaction, so this callback must not be described as atomic
   * grant/ceiling revalidation.
   */
  revalidateTaskPackage(
    taskPackage: AuthorizedTaskPackage,
    binding: ActionBinding,
  ): Promise<ActionCommitment>;
  /** Trusted host owns both the final fence and the actual provider side effect. */
  dispatchIfCurrent(
    binding: ActionBinding,
    action: MappedNativeAction,
    sendAuthority: string,
  ): Promise<ActionTransportResult | { readonly kind: "stale-before-send" }>;
}

export type DispatchActionResult =
  | { readonly status: "replayed"; readonly durableState: DurableActionState }
  | { readonly status: "dispatched"; readonly receiptRef: string | null }
  | { readonly status: "rejected"; readonly reason: string }
  | { readonly status: "outcome-unknown"; readonly reason: "EOF" | "ERROR" };

export type ActionErrorCode =
  | "INVALID_INTENT"
  | "SEMANTIC_DIGEST_MISMATCH"
  | "IDEMPOTENCY_CONFLICT"
  | "QUEUE_CAPACITY_EXCEEDED"
  | "RESERVATION_OUTCOME_UNKNOWN"
  | "BEGIN_OUTCOME_UNKNOWN"
  | "STALE_BINDING"
  | "STORE_PROTOCOL_ERROR"
  | "TRANSPORT_OUTCOME_UNKNOWN"
  | "OUTCOME_RECORD_UNKNOWN";

export class TypedActionError extends Error {
  override readonly name = "TypedActionError";
  readonly code: ActionErrorCode;
  override readonly cause: unknown;

  constructor(code: ActionErrorCode, detail: string, cause?: unknown) {
    super(`${code}: ${detail}`);
    this.code = code;
    this.cause = cause;
  }
}

export type RuntimeActionObservation =
  | {
      readonly kind: "completed";
      readonly operationId: string;
      readonly semanticDigest: string;
    }
  | { readonly kind: "progress" }
  | { readonly kind: "partial-frame" }
  | { readonly kind: "eof" }
  | { readonly kind: "error"; readonly detail: string };

export type ActionObservationState = "completed" | "in-progress" | "outcome-unknown";

interface PendingAction {
  readonly intent: Readonly<TypedActionIntent>;
  readonly resolve: (result: DispatchActionResult) => void;
  readonly reject: (error: unknown) => void;
}

const fail = (code: ActionErrorCode, detail: string, cause?: unknown): never => {
  throw new TypedActionError(code, detail, cause);
};

function passiveRecord(
  value: unknown,
  path: string,
  expectedKeys: ReadonlyArray<string>,
): Readonly<Record<string, unknown>> {
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value) ||
    NodeUtilTypes.isProxy(value) ||
    Object.getPrototypeOf(value) !== Object.prototype
  ) {
    return fail("INVALID_INTENT", `${path} must be a non-Proxy plain object`);
  }
  const keys = Reflect.ownKeys(value);
  if (keys.some((key) => typeof key === "symbol")) {
    return fail("INVALID_INTENT", `${path} must not contain symbol keys`);
  }
  const names = keys as ReadonlyArray<string>;
  const extra = names.find((name) => !expectedKeys.includes(name));
  if (extra !== undefined) return fail("INVALID_INTENT", `${path}.${extra} is not allowed`);
  const missing = expectedKeys.find((name) => !names.includes(name));
  if (missing !== undefined) return fail("INVALID_INTENT", `${path}.${missing} is required`);
  const descriptors = Object.getOwnPropertyDescriptors(value);
  const output: Record<string, unknown> = {};
  for (const name of names) {
    const descriptor = descriptors[name];
    if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
      return fail("INVALID_INTENT", `${path}.${name} must be an enumerable data property`);
    }
    output[name] = descriptor.value;
  }
  return output;
}

function passiveDiscriminant(value: unknown, path: string): unknown {
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value) ||
    NodeUtilTypes.isProxy(value) ||
    Object.getPrototypeOf(value) !== Object.prototype
  ) {
    return fail("INVALID_INTENT", `${path} must be a non-Proxy plain object`);
  }
  const descriptor = Object.getOwnPropertyDescriptor(value, "kind");
  if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
    return fail("INVALID_INTENT", `${path}.kind must be an enumerable data property`);
  }
  return descriptor.value;
}

function canonicalString(value: unknown, path: string, maxLength = 1_048_576): string {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > maxLength ||
    value !== value.trim()
  ) {
    return fail("INVALID_INTENT", `${path} must be a canonical non-empty string`);
  }
  return value;
}

function canonicalId(value: unknown, path: string): string {
  const candidate = canonicalString(value, path, 256);
  if (!SAFE_ID.test(candidate)) return fail("INVALID_INTENT", `${path} is not a safe identifier`);
  return candidate;
}

function canonicalU64(value: unknown, path: string): string {
  const candidate = canonicalString(value, path, 20);
  if (!U64.test(candidate) || BigInt(candidate) > 18_446_744_073_709_551_615n) {
    return fail("INVALID_INTENT", `${path} must be a canonical uint64 string`);
  }
  return candidate;
}

function snapshotBinding(value: unknown): Readonly<ActionBinding> {
  const record = passiveRecord(value, "intent.binding", [
    "bindingId",
    "sessionId",
    "executionId",
    "runtimeInstanceId",
    "profileId",
    "authRevision",
    "generation",
  ]);
  return Object.freeze({
    bindingId: canonicalId(record.bindingId, "intent.binding.bindingId"),
    sessionId: canonicalId(record.sessionId, "intent.binding.sessionId"),
    executionId: canonicalId(record.executionId, "intent.binding.executionId"),
    runtimeInstanceId: canonicalId(record.runtimeInstanceId, "intent.binding.runtimeInstanceId"),
    profileId: canonicalId(record.profileId, "intent.binding.profileId"),
    authRevision: canonicalU64(record.authRevision, "intent.binding.authRevision"),
    generation: canonicalU64(record.generation, "intent.binding.generation"),
  });
}

function snapshotTaskPackage(value: unknown): AuthorizedTaskPackage {
  try {
    return cloneAndFreezeJson(value as never, "intent.taskPackage") as AuthorizedTaskPackage;
  } catch (error) {
    return fail(
      "INVALID_INTENT",
      `intent.taskPackage must be passive JSON: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
}

export function actionCommitmentForPackage(
  taskPackage: AuthorizedTaskPackage,
): Readonly<ActionCommitment> {
  const source = taskPackage.source;
  const target = taskPackage.target;
  const commitment = Object.freeze({
    packageDigest: canonicalString(taskPackage.packageDigest, "intent.taskPackage.packageDigest", 71),
    parentGrantRef: canonicalId(taskPackage.parentGrantRef, "intent.taskPackage.parentGrantRef"),
    parentGrantRevision: canonicalU64(taskPackage.parentGrantRevision, "intent.taskPackage.parentGrantRevision"),
    parentCeilingDigest: canonicalString(taskPackage.parentCeilingDigest, "intent.taskPackage.parentCeilingDigest", 71),
    childCeilingDigest: canonicalString(taskPackage.childCeilingDigest, "intent.taskPackage.childCeilingDigest", 71),
    sourcePrincipalId: canonicalId(source.principalId, "intent.taskPackage.source.principalId"),
    sourceProjectId: canonicalId(source.projectId, "intent.taskPackage.source.projectId"),
    sourceDomainId: canonicalId(source.domainId, "intent.taskPackage.source.domainId"),
    sourceRole: canonicalId(source.role, "intent.taskPackage.source.role"),
    targetPrincipalId: canonicalId(target.principalId, "intent.taskPackage.target.principalId"),
    targetProjectId: canonicalId(target.projectId, "intent.taskPackage.target.projectId"),
    targetDomainId: canonicalId(target.domainId, "intent.taskPackage.target.domainId"),
    targetRole: canonicalId(target.role, "intent.taskPackage.target.role"),
    sourceSessionId: canonicalId(taskPackage.sourceBinding.sessionId, "intent.taskPackage.sourceBinding.sessionId"),
    sourceExecutionId: canonicalId(taskPackage.sourceBinding.executionId, "intent.taskPackage.sourceBinding.executionId"),
    sourceGeneration: canonicalU64(taskPackage.sourceBinding.generation, "intent.taskPackage.sourceBinding.generation"),
    childSessionId: canonicalId(taskPackage.targetBinding.sessionId, "intent.taskPackage.targetBinding.sessionId"),
    childExecutionId: canonicalId(taskPackage.targetBinding.executionId, "intent.taskPackage.targetBinding.executionId"),
    childGeneration: canonicalU64(taskPackage.targetBinding.generation, "intent.taskPackage.targetBinding.generation"),
    route: canonicalId(taskPackage.route, "intent.taskPackage.route"),
    policyAction: canonicalId(taskPackage.action, "intent.taskPackage.action"),
    sink: canonicalId(taskPackage.sink, "intent.taskPackage.sink"),
    materialSetDigest: canonicalString(taskPackage.materialSetDigest, "intent.taskPackage.materialSetDigest", 71),
    instructionDigest: canonicalString(taskPackage.instructionDigest, "intent.taskPackage.instructionDigest", 71),
  });
  for (const candidate of [
    commitment.packageDigest,
    commitment.parentCeilingDigest,
    commitment.childCeilingDigest,
    commitment.materialSetDigest,
    commitment.instructionDigest,
  ]) {
    if (!SHA256_DIGEST.test(candidate)) return fail("INVALID_INTENT", "task package digest is invalid");
  }
  return commitment;
}

function snapshotAction(value: unknown): Readonly<TypedAction> {
  const kind = passiveDiscriminant(value, "intent.action");
  if (kind === "cancel") {
    passiveRecord(value, "intent.action", ["kind"]);
    return Object.freeze({ kind: "cancel" });
  }
  if (kind === "prompt") {
    const record = passiveRecord(value, "intent.action", ["kind", "delivery", "text"]);
    if (record.delivery !== "queue" && record.delivery !== "steer") {
      return fail("INVALID_INTENT", "intent.action.delivery must be queue or steer");
    }
    return Object.freeze({
      kind: "prompt",
      delivery: record.delivery,
      text: canonicalString(record.text, "intent.action.text"),
    });
  }
  if (kind === "stop") {
    const record = passiveRecord(value, "intent.action", ["kind", "reason"]);
    return Object.freeze({
      kind: "stop",
      reason: canonicalString(record.reason, "intent.action.reason", 1_024),
    });
  }
  return fail("INVALID_INTENT", "intent.action.kind is unsupported");
}

function digestPreimage(
  operationId: string,
  binding: Readonly<ActionBinding>,
  commitment: Readonly<ActionCommitment>,
  action: Readonly<TypedAction>,
): string {
  const actionTuple =
    action.kind === "prompt"
      ? [action.kind, action.delivery, action.text]
      : action.kind === "stop"
        ? [action.kind, action.reason]
        : [action.kind];
  return JSON.stringify([
    ACTION_INTENT_SCHEMA,
    operationId,
    binding.bindingId,
    binding.sessionId,
    binding.executionId,
    binding.runtimeInstanceId,
    binding.profileId,
    binding.authRevision,
    binding.generation,
    commitment,
    actionTuple,
  ]);
}

export function semanticDigestForAction(input: Omit<TypedActionIntent, "semanticDigest">): string {
  const record = passiveRecord(input, "intent", ["schema", "operationId", "binding", "taskPackage", "action"]);
  if (record.schema !== ACTION_INTENT_SCHEMA) {
    return fail("INVALID_INTENT", `intent.schema must be ${ACTION_INTENT_SCHEMA}`);
  }
  const operationId = canonicalString(record.operationId, "intent.operationId", 36);
  if (!OPERATION_ID.test(operationId)) {
    return fail("INVALID_INTENT", "intent.operationId must match opr_<32 lowercase hex>");
  }
  const binding = snapshotBinding(record.binding);
  const taskPackage = snapshotTaskPackage(record.taskPackage);
  const commitment = actionCommitmentForPackage(taskPackage);
  if (binding.sessionId !== commitment.childSessionId || binding.executionId !== commitment.childExecutionId || binding.generation !== commitment.childGeneration) {
    return fail("INVALID_INTENT", "action binding does not match the authorized child binding");
  }
  const action = snapshotAction(record.action);
  return `sha256:${NodeCrypto.createHash("sha256")
    .update(digestPreimage(operationId, binding, commitment, action), "utf8")
    .digest("hex")}`;
}

export function snapshotActionIntent(value: unknown): Readonly<TypedActionIntent> {
  const record = passiveRecord(value, "intent", [
    "schema",
    "operationId",
    "semanticDigest",
    "binding",
    "taskPackage",
    "action",
  ]);
  if (record.schema !== ACTION_INTENT_SCHEMA) {
    return fail("INVALID_INTENT", `intent.schema must be ${ACTION_INTENT_SCHEMA}`);
  }
  const operationId = canonicalString(record.operationId, "intent.operationId", 36);
  if (!OPERATION_ID.test(operationId)) {
    return fail("INVALID_INTENT", "intent.operationId must match opr_<32 lowercase hex>");
  }
  const semanticDigest = canonicalString(record.semanticDigest, "intent.semanticDigest", 71);
  if (!SHA256_DIGEST.test(semanticDigest)) {
    return fail("INVALID_INTENT", "intent.semanticDigest must be a lowercase sha256 digest");
  }
  const binding = snapshotBinding(record.binding);
  const taskPackage = snapshotTaskPackage(record.taskPackage);
  const commitment = actionCommitmentForPackage(taskPackage);
  if (binding.sessionId !== commitment.childSessionId || binding.executionId !== commitment.childExecutionId || binding.generation !== commitment.childGeneration) {
    return fail("INVALID_INTENT", "action binding does not match the authorized child binding");
  }
  const action = snapshotAction(record.action);
  const expectedDigest = `sha256:${NodeCrypto.createHash("sha256")
    .update(digestPreimage(operationId, binding, commitment, action), "utf8")
    .digest("hex")}`;
  if (semanticDigest !== expectedDigest) {
    return fail(
      "SEMANTIC_DIGEST_MISMATCH",
      "intent.semanticDigest does not cover the typed action",
    );
  }
  return Object.freeze({
    schema: ACTION_INTENT_SCHEMA,
    operationId,
    semanticDigest,
    binding,
    taskPackage,
    action,
  });
}

export function mapTypedAction(action: Readonly<TypedAction>): Readonly<MappedNativeAction> {
  if (action.kind === "prompt") {
    return Object.freeze({ kind: action.delivery, text: action.text });
  }
  if (action.kind === "stop") return Object.freeze({ kind: "close", reason: action.reason });
  return Object.freeze({ kind: "interrupt" });
}

export function actionLane(action: Readonly<TypedAction>): ActionLane {
  return action.kind === "prompt" ? "work" : "control";
}

function sameBinding(left: Readonly<ActionBinding>, right: Readonly<ActionBinding>): boolean {
  return (
    left.bindingId === right.bindingId &&
    left.sessionId === right.sessionId &&
    left.executionId === right.executionId &&
    left.runtimeInstanceId === right.runtimeInstanceId &&
    left.profileId === right.profileId &&
    left.authRevision === right.authRevision &&
    left.generation === right.generation
  );
}

export function classifyActionObservation(
  intent: Readonly<TypedActionIntent>,
  observation: RuntimeActionObservation,
): ActionObservationState {
  if (
    observation.kind === "eof" ||
    observation.kind === "error" ||
    observation.kind === "partial-frame"
  ) {
    return "outcome-unknown";
  }
  if (observation.kind === "progress") return "in-progress";
  return observation.operationId === intent.operationId &&
    observation.semanticDigest === intent.semanticDigest
    ? "completed"
    : "in-progress";
}

export class TypedActionDispatcher {
  readonly #store: DurableActionStore;
  readonly #authority: ActionBindingAuthority;
  readonly #controlQueue: PendingAction[] = [];
  readonly #workQueue: PendingAction[] = [];
  #controlDraining = false;
  #workDraining = false;

  constructor(store: DurableActionStore, authority: ActionBindingAuthority) {
    this.#store = store;
    this.#authority = authority;
  }

  dispatch(value: TypedActionIntent): Promise<DispatchActionResult> {
    let intent: Readonly<TypedActionIntent>;
    try {
      intent = snapshotActionIntent(value);
    } catch (error) {
      return Promise.reject(error);
    }
    return new Promise<DispatchActionResult>((resolve, reject) => {
      const lane = actionLane(intent.action);
      const queue = lane === "control" ? this.#controlQueue : this.#workQueue;
      const limit = lane === "control" ? CONTROL_QUEUE_LIMIT : WORK_QUEUE_LIMIT;
      if (queue.length >= limit) {
        reject(
          new TypedActionError(
            "QUEUE_CAPACITY_EXCEEDED",
            `${lane} action queue reached its ${limit}-item limit`,
          ),
        );
        return;
      }
      queue.push({ intent, resolve, reject });
      this.#scheduleDrain(lane);
    });
  }

  #scheduleDrain(lane: ActionLane): void {
    if (lane === "control") {
      if (this.#controlDraining) return;
      this.#controlDraining = true;
    } else {
      if (this.#workDraining) return;
      this.#workDraining = true;
    }
    queueMicrotask(() => void this.#drain(lane));
  }

  async #drain(lane: ActionLane): Promise<void> {
    const queue = lane === "control" ? this.#controlQueue : this.#workQueue;
    while (true) {
      const pending = queue.shift();
      if (pending === undefined) {
        if (lane === "control") this.#controlDraining = false;
        else this.#workDraining = false;
        if (queue.length > 0) this.#scheduleDrain(lane);
        return;
      }
      try {
        pending.resolve(await this.#execute(pending.intent));
      } catch (error) {
        pending.reject(error);
      }
    }
  }

  async #assertCurrent(binding: Readonly<ActionBinding>): Promise<void> {
    const current = await this.#authority.currentBinding(binding.bindingId);
    if (current === null || !sameBinding(binding, current)) {
      return fail(
        "STALE_BINDING",
        `binding ${binding.bindingId} generation ${binding.generation} is no longer current`,
      );
    }
  }

  async #revalidate(intent: Readonly<TypedActionIntent>): Promise<Readonly<ActionCommitment>> {
    const expected = actionCommitmentForPackage(intent.taskPackage);
    const current = await this.#authority.revalidateTaskPackage(intent.taskPackage, intent.binding);
    if (JSON.stringify(current) !== JSON.stringify(expected)) {
      return fail("STALE_BINDING", "task package commitment is no longer current");
    }
    return current;
  }

  async #record(
    reservationId: string,
    intent: Readonly<TypedActionIntent>,
    outcome: DurableDispatchOutcome,
  ): Promise<void> {
    try {
      await this.#store.recordDispatchOutcome(
        reservationId,
        intent.operationId,
        intent.semanticDigest,
        outcome,
      );
    } catch (error) {
      return fail(
        "OUTCOME_RECORD_UNKNOWN",
        `durable outcome for ${intent.operationId} could not be established; do not resend`,
        error,
      );
    }
  }

  async #execute(intent: Readonly<TypedActionIntent>): Promise<DispatchActionResult> {
    const commitment = await this.#revalidate(intent);
    await this.#assertCurrent(intent.binding);
    const action = mapTypedAction(intent.action);
    const reservation: DurableActionReservation = Object.freeze({
      schema: ACTION_INTENT_SCHEMA,
      operationId: intent.operationId,
      semanticDigest: intent.semanticDigest,
      binding: intent.binding,
      commitment,
      lane: actionLane(intent.action),
      action,
    });

    let reserved: ReserveActionResult;
    try {
      reserved = await this.#store.reserve(reservation);
    } catch (error) {
      return fail(
        "RESERVATION_OUTCOME_UNKNOWN",
        `reservation for ${intent.operationId} is unknown; no send was attempted`,
        error,
      );
    }
    if (reserved.operationId !== intent.operationId) {
      return fail("STORE_PROTOCOL_ERROR", "store returned a different operation identity");
    }
    if (reserved.kind === "unknown") {
      return fail(
        "RESERVATION_OUTCOME_UNKNOWN",
        `reservation for ${intent.operationId} is unknown; no send was attempted`,
      );
    }
    if (reserved.kind === "conflict") {
      await this.#assertCurrent(intent.binding);
      return fail(
        "IDEMPOTENCY_CONFLICT",
        `${intent.operationId} already exists with ${reserved.existingSemanticDigest}`,
      );
    }
    if (reserved.semanticDigest !== intent.semanticDigest) {
      return fail(
        "IDEMPOTENCY_CONFLICT",
        `${intent.operationId} was reused with a different semantic digest`,
      );
    }
    if (reserved.kind === "replay" && reserved.state !== "reserved") {
      await this.#revalidate(intent);
      await this.#assertCurrent(intent.binding);
      return Object.freeze({ status: "replayed", durableState: reserved.state });
    }
    const reservationId = reserved.reservationId;
    if (reservationId.length === 0) {
      return fail("STORE_PROTOCOL_ERROR", "store returned an empty reservation identity");
    }
    try {
      await this.#revalidate(intent);
      await this.#assertCurrent(intent.binding);
    } catch (error) {
      await this.#record(reservationId, intent, {
        kind: "not-sent",
        reason: "STALE_BINDING",
      });
      throw error;
    }

    let begun: BeginActionResult;
    try {
      begun = await this.#store.begin(reservationId, reservation);
    } catch (error) {
      return fail(
        "BEGIN_OUTCOME_UNKNOWN",
        `begin commitment for ${intent.operationId} is unknown; do not send`,
        error,
      );
    }
    if (begun.operationId !== intent.operationId || begun.reservationId !== reservationId) {
      return fail("STORE_PROTOCOL_ERROR", "begin returned a different reservation identity");
    }
    if (begun.kind === "unknown") {
      return fail("BEGIN_OUTCOME_UNKNOWN", `begin commitment for ${intent.operationId} is unknown; do not send`);
    }
    if (begun.kind === "conflict") {
      return fail("IDEMPOTENCY_CONFLICT", `begin commitment for ${intent.operationId} conflicted`);
    }
    if (begun.kind === "replay") {
      return Object.freeze({ status: "replayed", durableState: begun.state });
    }
    if (begun.sendAuthority.length === 0) {
      return fail("STORE_PROTOCOL_ERROR", "begin returned an empty send authority");
    }

    let dispatched: ActionTransportResult | { readonly kind: "stale-before-send" };
    try {
      dispatched = await this.#authority.dispatchIfCurrent(intent.binding, action, begun.sendAuthority);
    } catch (error) {
      await this.#record(reservationId, intent, {
        kind: "outcome-unknown",
        reason: "TRANSPORT_THROW",
        detail: error instanceof Error ? error.message : "transport threw a non-Error value",
      });
      return fail(
        "TRANSPORT_OUTCOME_UNKNOWN",
        `transport outcome for ${intent.operationId} is unknown; do not resend`,
        error,
      );
    }
    if (dispatched.kind === "stale-before-send") {
      await this.#record(reservationId, intent, {
        kind: "not-sent",
        reason: "STALE_BINDING",
      });
      return fail("STALE_BINDING", "binding changed before the provider side effect");
    }
    const transportResult = dispatched;

    if (transportResult.kind === "accepted") {
      const receiptRef = transportResult.receiptRef ?? null;
      await this.#record(reservationId, intent, { kind: "dispatched", receiptRef });
      return Object.freeze({ status: "dispatched", receiptRef });
    }
    if (transportResult.kind === "rejected") {
      await this.#record(reservationId, intent, {
        kind: "rejected",
        reason: transportResult.reason,
      });
      return Object.freeze({ status: "rejected", reason: transportResult.reason });
    }
    const reason = transportResult.kind === "eof" ? "EOF" : "ERROR";
    await this.#record(reservationId, intent, {
      kind: "outcome-unknown",
      reason,
      detail: transportResult.kind === "error" ? transportResult.detail : "transport reached EOF",
    });
    return Object.freeze({ status: "outcome-unknown", reason });
  }
}
