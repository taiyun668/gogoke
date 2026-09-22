import type { DecisionBackend } from "../engine/engine.ts";

export class DecisionBackendClosedError extends Error {
  override readonly name = "DecisionBackendClosedError";
  readonly code = "NOT_QUALIFIED";
  constructor() { super("NOT_QUALIFIED"); }
}

/** GN is closed. Reject BEFORE observing configuration, reading keys, creating
 * any provider/transport or serializing a view. A model name/boolean is no grant.
 */
export function createExternalDecisionBackend(_configuration: unknown): never {
  throw new DecisionBackendClosedError();
}

/** Non-transport sentinels for current bootstrap/fixture wiring. Even direct
 * evaluate calls reject without observing their input. No SDK or env imports.
 */
function closed(kind: "JEV" | "GENERATIVE"): DecisionBackend {
  return Object.freeze({ kind, async evaluate() { throw new DecisionBackendClosedError(); } });
}
export const CLOSED_JEV_BACKEND: DecisionBackend = closed("JEV");
export const CLOSED_GENERATIVE_BACKEND: DecisionBackend = closed("GENERATIVE");
