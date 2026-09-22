import { performance } from "node:perf_hooks";
import { setTimeout, clearTimeout } from "node:timers";
import { isProxy } from "node:util/types";
import { DecisionEngine, type DecisionBackend, type DecisionCommitPort,
  type DecisionEligibilityPort, type DecisionRequest, type DecisionResult } from "./engine.ts";
import { snapshotRequest, snapshotBackendKind, snapshotEligibility,
  snapshotBackendResult, snapshotCommitResult } from "./decoders.ts";
import { captureMethod, captureValue, DecisionEngineError } from "./passive.ts";

/** Trusted host timing, never deserialized from a request or a model response.
 * Synthetic clocks are test instruments, not product authority or qualification.
 */
export interface DecisionTiming {
  epochMs(): number;
  monotonicMs(): number;
  schedule(delayMs: number, callback: () => void): () => void;
}
// Error objects are data too: never use instanceof on an unknown Proxy or
// return a collaborator-owned Error whose code/name/stack may be active getters.
const errorPrototype = DecisionEngineError.prototype;
const getPrototype = Object.getPrototypeOf;
const getDescriptor = Object.getOwnPropertyDescriptor;
const owns = Object.hasOwn;
function safeFailure(error: unknown, commitStarted: boolean): DecisionEngineError {
  const fallback = commitStarted ? "COMMIT_UNKNOWN" : "INVALID_INPUT";
  if (error === null || typeof error !== "object" || isProxy(error) || getPrototype(error) !== errorPrototype) {
    return new DecisionEngineError(fallback);
  }
  const code = getDescriptor(error, "code");
  if (!code || !owns(code, "value")) return new DecisionEngineError(fallback);
  switch (code.value) {
    case "INVALID_INPUT": case "STALE_VIEW": case "BACKEND_PROTOCOL":
    case "COMMIT_DENIED": case "COMMIT_CONFLICT": case "COMMIT_UNKNOWN":
      return new DecisionEngineError(code.value);
    default: return new DecisionEngineError(fallback);
  }
}
const epochNow = Date.now;
const performanceNow = performance.now.bind(performance);
const HOST_TIMING: DecisionTiming = Object.freeze({
  epochMs: () => epochNow(),
  monotonicMs: () => performanceNow(),
  schedule: (delayMs: number, callback: () => void) => {
    const timer = setTimeout(callback, delayMs);
    return () => clearTimeout(timer);
  },
});

/** Local execution termination, NOT a persisted DecisionRecord or action receipt. */
export type BoundedDecisionResult = DecisionResult | {
  readonly kind: "DEADLINE" | "CANCELLED";
  readonly operationId: string;
  readonly commitStarted: false;
};
export interface DecisionRun {
  readonly result: Promise<BoundedDecisionResult>;
  /** Can only stop this invocation. It cannot grant permission or undo a commit. */
  cancel(): void;
}

/** Deadline/cancellation coordination around the SAME selection kernel.
 * Not a scheduler, permission store, dispatcher or durable replay authority.
 * Native commit must still atomically check revisions/capacity/deadline; once
 * invoked, local cancellation cannot assert that no transaction was committed.
 * No retries and no cancellation of unrelated deterministic control actions.
 */
export class BoundedDecisionExecutor {
  readonly #eligibility: DecisionEligibilityPort;
  readonly #backend: DecisionBackend;
  readonly #commit: DecisionCommitPort;
  readonly #timing: DecisionTiming;
  readonly #limitMs: number;

  constructor(eligibility: DecisionEligibilityPort, backend: DecisionBackend, commit: DecisionCommitPort,
    mode: "INTERACTIVE" | "BACKGROUND" = "INTERACTIVE", timing: DecisionTiming = HOST_TIMING) {
    if (mode !== "INTERACTIVE" && mode !== "BACKGROUND") throw new DecisionEngineError("INVALID_INPUT");
    this.#limitMs = mode === "INTERACTIVE" ? 2000 : 10000;
    this.#eligibility = Object.freeze({ resolveEligibility:
      captureMethod<DecisionEligibilityPort["resolveEligibility"]>(eligibility, "resolveEligibility") });
    this.#backend = Object.freeze({ kind: snapshotBackendKind(captureValue(backend, "kind")),
      evaluate: captureMethod<DecisionBackend["evaluate"]>(backend, "evaluate") });
    this.#commit = Object.freeze({ commitDecisionReservation:
      captureMethod<DecisionCommitPort["commitDecisionReservation"]>(commit, "commitDecisionReservation") });
    this.#timing = Object.freeze({ epochMs: captureMethod<DecisionTiming["epochMs"]>(timing, "epochMs"),
      monotonicMs: captureMethod<DecisionTiming["monotonicMs"]>(timing, "monotonicMs"),
      schedule: captureMethod<DecisionTiming["schedule"]>(timing, "schedule") });
  }

  start(input: DecisionRequest): DecisionRun {
    const original = snapshotRequest(input);
    const readClock = (): { epoch: number; mono: number } => {
      try {
        const epoch = this.#timing.epochMs();
        const mono = this.#timing.monotonicMs();
        if (!Number.isSafeInteger(epoch) || epoch < 0 || !Number.isFinite(mono) || mono < 0 || mono > Number.MAX_SAFE_INTEGER) {
          throw new DecisionEngineError("INVALID_INPUT");
        }
        return { epoch, mono };
      } catch {
        // Initial reads happen before the result Promise exists. Later reads
        // are reclassified by requireOpen when a native commit has started.
        throw new DecisionEngineError("INVALID_INPUT");
      }
    };
    const started = readClock();
    // The caller can shorten but never raise the host's bounded time window.
    const duration = Math.max(0, Math.min(this.#limitMs, original.deadlineEpochMs - started.epoch));
    const deadline = started.epoch + duration;
    const monoDeadline = started.mono + duration;
    if (!Number.isSafeInteger(deadline) || monoDeadline > Number.MAX_SAFE_INTEGER) throw new DecisionEngineError("INVALID_INPUT");
    const request = Object.freeze({ ...original, deadlineEpochMs: deadline });
    let lastMono = started.mono;
    let settled = false;
    let commitStarted = false;
    let cancelTimer: (() => void) | undefined;
    let resolve!: (value: BoundedDecisionResult) => void;
    let reject!: (reason: unknown) => void;
    const result = new Promise<BoundedDecisionResult>((yes, no) => { resolve = yes; reject = no; });
    const cleanup = () => {
      const cancel = cancelTimer;
      cancelTimer = undefined;
      if (cancel) { try { cancel(); } catch { /* No new work or result reclassification. */ } }
    };
    const succeed = (value: BoundedDecisionResult) => {
      if (settled) return;
      settled = true;
      cleanup();
      resolve(value);
    };
    const fail = (error: unknown) => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(safeFailure(error, commitStarted));
    };
    const stop = (kind: "DEADLINE" | "CANCELLED") => {
      if (settled) return;
      if (commitStarted) { fail(new DecisionEngineError("COMMIT_UNKNOWN")); return; }
      succeed(Object.freeze({ kind, operationId: request.operationId, commitStarted: false }));
    };
    const requireOpen = () => {
      if (!settled) {
        try {
          const now = readClock();
          if (now.mono < lastMono) throw new DecisionEngineError("INVALID_INPUT");
          lastMono = now.mono;
          if (now.epoch >= deadline || now.mono >= monoDeadline) stop("DEADLINE");
        } catch { fail(new DecisionEngineError(commitStarted ? "COMMIT_UNKNOWN" : "INVALID_INPUT")); }
      }
      if (settled) throw new DecisionEngineError(commitStarted ? "COMMIT_UNKNOWN" : "STALE_VIEW");
    };
    const handle = Object.freeze({ result, cancel: () => stop("CANCELLED") });
    if (duration === 0) { stop("DEADLINE"); return handle; }
    try {
      const cancel = this.#timing.schedule(duration, () => stop("DEADLINE"));
      if (typeof cancel !== "function" || isProxy(cancel)) throw new DecisionEngineError("INVALID_INPUT");
      cancelTimer = cancel;
      if (settled) { cleanup(); return handle; }
      requireOpen();
      const engine = new DecisionEngine({ resolveEligibility: async request => {
        requireOpen();
        const value = await this.#eligibility.resolveEligibility(request);
        requireOpen();
        // Decode before returning from this async wrapper: returning an untrusted
        // root value would run Promise thenable assimilation a second time.
        return snapshotEligibility(value);
      } }, { kind: this.#backend.kind, evaluate: async input => {
        requireOpen();
        const value = await this.#backend.evaluate(input);
        requireOpen();
        return snapshotBackendResult(value);
      } }, { commitDecisionReservation: async command => {
        requireOpen();
        if (commitStarted) throw new DecisionEngineError("COMMIT_UNKNOWN");
        // This is entry to a trusted native COMMIT port, not an external I/O fence.
        commitStarted = true;
        const value = await this.#commit.commitDecisionReservation(command);
        requireOpen();
        return snapshotCommitResult(value);
      } });
      // Install both handlers immediately; late fulfillment/rejection cannot
      // restart the kernel, disclose a receipt after close, or become unhandled.
      void engine.decide(request).then(value => {
        try { requireOpen(); succeed(value); } catch { /* Already terminal. */ }
      }, fail);
    } catch (error) { fail(error); }
    return handle;
  }
}
