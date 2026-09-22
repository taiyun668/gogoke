export const PUBLIC_SCHEMA_VERSION = 1 as const;
export const DECIMAL_U64_MAX = 18_446_744_073_709_551_615n;

export type JsonValue =
  | null
  | boolean
  | string
  | number
  | bigint
  | JsonValue[]
  | { [key: string]: JsonValue };
export type DecimalU64 = string;
export type OpaqueId = string;
export type SchemaVersion = typeof PUBLIC_SCHEMA_VERSION;

export type SessionLifecycle = "active" | "archived";
export type ContinuationMode = "native" | "fixed_context" | "new";
export type ExecutionState = "created" | "running" | "completed" | "failed" | "cancelled";
export type AcceptanceState =
  | "recorded" | "queued" | "dispatching" | "accepted"
  | "acceptance_unknown" | "rejected" | "cancelled";
export type CapabilityState = "supported" | "unsupported" | "unknown";
export type HumanActionState = "pending" | "resolved" | "expired" | "cancelled";
export type PublicEventKind =
  | "session_updated" | "binding_updated" | "delivery_updated"
  | "execution_started" | "execution_output_delta" | "execution_completed"
  | "execution_failed" | "execution_cancelled" | "human_action_requested"
  | "human_action_resolved" | "context_created" | "delegation_result"
  | "stream_gap" | "diagnostic";
export type ErrorCode =
  | "FEATURE_SEALED" | "UNAUTHORIZED" | "SCOPE_MISMATCH" | "STALE_BINDING"
  | "UNSUPPORTED" | "CAPABILITY_UNKNOWN" | "PROTOCOL_VERSION_MISMATCH"
  | "INVALID_FRAME" | "OPERATION_CONFLICT" | "ACCEPTANCE_UNKNOWN"
  | "ROOT_UNAVAILABLE" | "ROOT_IN_USE" | "ROOT_ID_MISMATCH" | "UNSUPPORTED_ROOT"
  | "STORE_CORRUPT" | "STORE_WRITE_FAILED" | "DURABILITY_UNKNOWN"
  | "PROCESS_IDENTITY_UNKNOWN" | "STOP_UNCONFIRMED" | "LEGACY_ROUTE_DISABLED"
  | "QUEUE_FULL" | "RESYNC_REQUIRED" | "CONFIG_CHANGED" | "CANCELLED";
export type ObservationState = "observed" | "not_observed" | "unknown";
export type RecordedState = "recorded" | "not_recorded" | "unknown";
export type DurabilityAckState = "confirmed" | "failed" | "unknown";
export type CustodyState = "retained" | "released" | "transferred" | "unknown";
export type SideEffectState = "none" | "possible" | "confirmed" | "unknown";

export interface Session { schemaVersion: SchemaVersion; sessionId: OpaqueId; scopeId: OpaqueId; privacyDomainId: OpaqueId; title: string; lifecycle: SessionLifecycle; revision: DecimalU64; }
export interface NativeBinding { schemaVersion: SchemaVersion; bindingId: OpaqueId; sessionId: OpaqueId; driverId: string; instanceId: OpaqueId; profileRevision: DecimalU64; authRevision: DecimalU64; domainId: OpaqueId; generation: DecimalU64; nativeSessionId: string | null; continuationMode: ContinuationMode; }
export interface Execution { schemaVersion: SchemaVersion; executionId: OpaqueId; sessionId: OpaqueId; bindingId: OpaqueId; generation: DecimalU64; state: ExecutionState; createdAt: string; completedAt: string | null; resultRef: string | null; }
export interface Delivery { schemaVersion: SchemaVersion; operationId: OpaqueId; executionId: OpaqueId; sessionId: OpaqueId; bindingId: OpaqueId; generation: DecimalU64; requestFingerprint: string; intentKind: string; acceptanceState: AcceptanceState; durableReceiptId: OpaqueId | null; nativeReceipt: JsonValue; }
export interface PublicEvent { schemaVersion: SchemaVersion; eventId: OpaqueId; sessionId: OpaqueId; executionId: OpaqueId | null; bindingId: OpaqueId | null; generation: DecimalU64; streamEpoch: DecimalU64; sequence: DecimalU64; kind: PublicEventKind; payload: JsonValue; }
export interface ExecutableId { path: string; hash: string; version: string; platform: string; }
export interface CapabilityEvidence { state: CapabilityState; reason: string; evidenceRef: string | null; }
export interface CapabilitySnapshot { schemaVersion: SchemaVersion; snapshotId: OpaqueId; instanceId: OpaqueId; executableId: ExecutableId; mode: string; profileRevision: DecimalU64; authRevision: DecimalU64; observedAt: string; expiresAt: string; evidenceSource: string; capabilities: Record<string, CapabilityEvidence>; }
export interface ContextItem { ref: string; digest: string; visibility: string; }
export interface ContextPackage { schemaVersion: SchemaVersion; packageId: OpaqueId; recipient: string; scopeId: OpaqueId; domainId: OpaqueId; taskId: string; sourceVersion: string; items: ContextItem[]; permissionCeiling: Record<string, JsonValue>; expiresAt: string; }
export interface HumanActionRequest { schemaVersion: SchemaVersion; requestId: OpaqueId; executionId: OpaqueId; bindingId: OpaqueId; generation: DecimalU64; continuationId: string; domainId: OpaqueId; expiresAt: string; allowedAnswers: string[]; permissionCeiling: Record<string, JsonValue>; state: HumanActionState; }
export interface ProcessIdentity { schemaVersion: SchemaVersion; hostId: OpaqueId; instanceId: OpaqueId; processHandleRef: string; pid: number; startedAtTicks: DecimalU64 | null; launchEpoch: DecimalU64; imageDigest: string; containmentId: string | null; }
export interface OwnershipIssue { code: ErrorCode; stage: string; message: string; }
export interface OwnershipResult { schemaVersion: SchemaVersion; observation: ObservationState; inMemoryState: RecordedState; persistedState: RecordedState; durabilityAck: DurabilityAckState; custodyState: CustodyState; errors: OwnershipIssue[]; }
export interface CommandError { code: ErrorCode; stage: string; retryable: boolean; sideEffectState: SideEffectState; correlationId: OpaqueId; message: string; }
export interface CommandSuccess<T = JsonValue> { schemaVersion: SchemaVersion; ok: true; value: T; receipt: JsonValue; }
export interface CommandFailure { schemaVersion: SchemaVersion; ok: false; error: CommandError; }

export type PublicDocument =
  | Session | NativeBinding | Execution | Delivery | PublicEvent | CapabilitySnapshot
  | ContextPackage | HumanActionRequest | ProcessIdentity | OwnershipResult
  | CommandSuccess | CommandFailure;
