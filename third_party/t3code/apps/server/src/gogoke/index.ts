/**
 * Gogoke's public service composition boundary.
 *
 * The donor T3 CLI remains legacy/reference code. New Gogoke construction
 * enters only through the sealed bootstrap exported here, so later service
 * wiring cannot accidentally bypass the pre-construction policy fence.
 */
export * from "./bootstrap/index.ts";
export * from "./actions/index.ts";
export * from "./contracts/index.ts";
export * from "./context/repository/index.ts";
export { CustodyService, hashStopProof, stopProofErrors } from "./custody/custody.ts";
export {
  HOST_STOP_DEADLINE_MS,
  PRODUCTION_STOP_BUDGETS,
  STOP_GRACE_MS,
  STOP_OBSERVE_MS,
  STOP_REFUSED_EXIT_CODE,
  STOP_TERMINATE_MS,
  STOP_TIMEOUT_EXIT_CODE,
  stopBudgetsFitHostDeadline,
} from "./custody/model.ts";
export type {
  CustodyPhase,
  CustodyRecord,
  DurableCustodyStore,
  LaunchRequest,
  NativeBinding as CustodyNativeBinding,
  NativeCustodyAdapter,
  NativePrepareRequest,
  NativeStopConfirmation,
  NativeStopProof,
  PreparedCustody,
  ProcessIdentity,
  ReservePreparedResult,
  StopBudgets,
} from "./custody/model.ts";
export * from "./policy/index.ts";
export * from "./releasePolicy.ts";
