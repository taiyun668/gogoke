import { V37_DURABLE_OWNER, V37_READ_OPERATIONS } from "./catalog.ts";
import { decodeV37Request, encodeV37Receipt, type V37Port, type V37Receipt, type V37Request, type V37TrustedCaller, V37_SCHEMA } from "./protocol.ts";
import { rawV37RequestKey } from "./rawRequest.ts";

type Authorize = (caller: V37TrustedCaller, request: V37Request) => boolean;
type ResolveCaller = () => V37TrustedCaller | null;
interface Entry { readonly bytes: string; readonly receipt: V37Receipt; }

/** Test-only store; real persistence and issuer checks belong to the native host. */
export class V37FakeStore {
  readonly revisions = new Map<string, bigint>();
  readonly requests = new Map<string, Entry>();
}

export class V37FakePort implements V37Port {
  private readonly store: V37FakeStore;
  private readonly caller: ResolveCaller;
  private readonly authorize: Authorize;
  constructor(
    store: V37FakeStore,
    caller: ResolveCaller,
    authorize: Authorize,
  ) { this.store = store; this.caller = caller; this.authorize = authorize; }

  async execute(bytes: Uint8Array): Promise<Uint8Array> {
    const request = decodeV37Request(bytes);
    const key = `${request.family}:${request.domainId}:${request.targetId}`;
    const current = this.store.revisions.get(key) ?? 0n;
    const receipt = (status: V37Receipt["status"], revision: bigint,
      result: V37Receipt["result"] = {}): V37Receipt => ({
      schema: V37_SCHEMA, family: request.family, operation: request.operation,
      requestId: request.requestId, targetId: request.targetId, status,
      previousRevision: current.toString(), revision: revision.toString(), result,
    });
    const principal = this.caller();
    if (principal === null || principal.domainId !== request.domainId ||
        !this.authorize(principal, request)) {
      return encodeV37Receipt(receipt("DENIED", current));
    }
    // K-UI forwards an underlying operation's exact receipt; the generic
    // envelope fake cannot truthfully synthesize that behavior.
    if (request.family === "K-UI") {
      return encodeV37Receipt(receipt("UNSUPPORTED", current));
    }
    const replayKey = `${request.family}:${request.domainId}:${request.requestId}`;
    const previous = this.store.requests.get(replayKey);
    const raw = rawV37RequestKey(bytes);
    if (previous !== undefined) {
      return encodeV37Receipt(previous.bytes === raw
        ? { ...previous.receipt, status: "REPLAYED" }
        : receipt("CONFLICT", current));
    }
    if (BigInt(request.expectedRevision) !== current) {
      return encodeV37Receipt(receipt("STALE", current));
    }
    const isRead = V37_READ_OPERATIONS[request.family].includes(request.operation);
    const next = isRead ? current : current + 1n;
    const applied = receipt("APPLIED", next, { durableOwner: V37_DURABLE_OWNER[request.family] });
    this.store.requests.set(replayKey, { bytes: raw, receipt: applied });
    if (!isRead) this.store.revisions.set(key, next);
    return encodeV37Receipt(applied);
  }
}

/** No native issuer means no product execution. */
