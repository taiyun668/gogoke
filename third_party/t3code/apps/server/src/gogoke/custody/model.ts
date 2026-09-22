export const STOP_GRACE_MS = 10_000;
export const STOP_TERMINATE_MS = 5_000;
export const STOP_OBSERVE_MS = 5_000;
export const HOST_STOP_DEADLINE_MS = 30_000;
export const STOP_TIMEOUT_EXIT_CODE = 124;
export const STOP_REFUSED_EXIT_CODE = 125;

export interface StopBudgets {
  readonly graceMs: number;
  readonly terminateMs: number;
  readonly observeMs: number;
  readonly hostDeadlineMs: number;
}

export const PRODUCTION_STOP_BUDGETS: StopBudgets = Object.freeze({
  graceMs: STOP_GRACE_MS,
  terminateMs: STOP_TERMINATE_MS,
  observeMs: STOP_OBSERVE_MS,
  hostDeadlineMs: HOST_STOP_DEADLINE_MS,
});

export interface NativeBinding {
  readonly binaryDigestSha256: string;
  readonly profileId: string;
  readonly domainId: string;
  readonly generation: string;
}

export interface ProcessIdentity {
  readonly pid: number;
  readonly creationTime100ns: string;
  readonly imagePath: string;
}

export interface PreparedCustody {
  readonly ticket: string;
  readonly custodianNonce: string;
  readonly binding: NativeBinding;
  readonly identity: ProcessIdentity;
}

export interface NativeStopProof extends PreparedCustody {
  readonly parentExited: boolean;
  readonly activeJobProcesses: number | null;
  readonly identityStatus: "exact" | "unknown" | "mismatch";
  readonly processHandlePresent: boolean;
  readonly jobHandlePresent: boolean;
  readonly killAttempted: boolean;
  readonly killSucceeded: boolean;
  readonly writerFenceVerified: boolean;
  readonly exitCode: number | null;
  readonly deadlineExceeded: boolean;
  readonly errors: ReadonlyArray<string>;
}

export type CustodyPhase =
  | "PREPARED"
  | "ACTIVE"
  | "STOPPING"
  | "STOPPED_PENDING_CONFIRM"
  | "CONFIRMED"
  | "RESIDUAL";

export interface CustodyRecord extends PreparedCustody {
  readonly writerDomain: string;
  readonly phase: CustodyPhase;
  readonly revision: number;
  readonly stopAttempt: number;
  readonly leaseState: "held" | "lost";
  readonly stopProof: NativeStopProof | null;
  readonly errors: ReadonlyArray<string>;
}

export type ReservePreparedResult =
  | { readonly status: "reserved"; readonly record: CustodyRecord }
  | { readonly status: "writer-conflict" };

/**
 * Each method is one durable transaction/CAS. reservePrepared atomically
 * checks the writer domain and inserts PREPARED; it is never list-then-put.
 * Every phase except CONFIRMED keeps the writer reservation. In particular a
 * STOPPED_PENDING_CONFIRM or RESIDUAL row never permits a replacement writer.
 */
export interface DurableCustodyStore {
  reservePrepared(prepared: PreparedCustody, writerDomain: string): Promise<ReservePreparedResult>;
  read(ticket: string): Promise<CustodyRecord | null>;
  compareAndSet(record: CustodyRecord, expectedRevision: number): Promise<CustodyRecord>;
  saveStoppedProof(
    ticket: string,
    expectedRevision: number,
    proof: NativeStopProof,
  ): Promise<CustodyRecord>;
}

export interface NativePrepareRequest {
  readonly application: string;
  readonly arguments: ReadonlyArray<string>;
  readonly currentDirectory: string | null;
  readonly hideWindow: boolean;
  readonly binding: NativeBinding;
}

export interface NativeStopConfirmation {
  readonly ticket: string;
  readonly custodianNonce: string;
  readonly identity: ProcessIdentity;
  readonly proofHash: string;
  readonly durableRevision: number;
}

/** Trusted process-local adapter. Request callers can never provide one. */
export interface NativeCustodyAdapter {
  prepare(request: NativePrepareRequest, signal: AbortSignal): Promise<PreparedCustody>;
  activate(prepared: PreparedCustody, signal: AbortSignal): Promise<void>;
  abortPrepared(prepared: PreparedCustody, signal: AbortSignal): Promise<void>;
  stop(
    prepared: PreparedCustody,
    budgets: StopBudgets,
    signal: AbortSignal,
  ): Promise<NativeStopProof>;
  confirmStopped(confirmation: NativeStopConfirmation, signal: AbortSignal): Promise<void>;
}

export interface LaunchRequest extends NativePrepareRequest {
  readonly writerDomain: string;
}

export function stopBudgetsFitHostDeadline(budgets: StopBudgets): boolean {
  const phases = budgets.graceMs + budgets.terminateMs + budgets.observeMs;
  return (
    Number.isSafeInteger(phases) &&
    budgets.graceMs >= 0 &&
    budgets.terminateMs >= 0 &&
    budgets.observeMs >= 0 &&
    budgets.hostDeadlineMs > 0 &&
    phases <= budgets.hostDeadlineMs
  );
}
