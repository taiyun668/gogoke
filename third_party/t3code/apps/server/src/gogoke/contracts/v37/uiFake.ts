import type { JsonObject } from "../model.ts";
import { V37_READ_OPERATIONS } from "./catalog.ts";
import { decodeV37Receipt, decodeV37Request, encodeV37Receipt, encodeV37Request,
  V37_SCHEMA, type V37Port, type V37Receipt, type V37Request, type V37TrustedCaller } from "./protocol.ts";

export interface V37UiFakeOptions {
  readonly caller: () => V37TrustedCaller | null;
  readonly granted: (caller: V37TrustedCaller, request: V37Request) => boolean;
  /** Host-owned mapping of a UI view/action to its exact source operation. */
  readonly resolve: (request: V37Request, caller: V37TrustedCaller) => V37Request | null;
  readonly source: V37Port;
}

/** Test-only stateless integrator. It returns the source receipt bytes unchanged. */
export class V37UiForwardingFakePort implements V37Port {
  readonly options: V37UiFakeOptions;
  constructor(options: V37UiFakeOptions) { this.options = options; }

  async execute(bytes: Uint8Array): Promise<Uint8Array> {
    const request = decodeV37Request(bytes);
    if (request.family !== "K-UI") throw new Error("V37_UI_ONLY");
    const reply = (status: V37Receipt["status"], result: JsonObject = {}): Uint8Array =>
      encodeV37Receipt({ schema: V37_SCHEMA, family: request.family,
        operation: request.operation, requestId: request.requestId, targetId: request.targetId,
        status, previousRevision: request.expectedRevision,
        revision: request.expectedRevision, result });
    const caller = this.options.caller();
    if (!caller || caller.domainId !== request.domainId ||
        !this.options.granted(caller, request)) return reply("DENIED");
    const sourceRequest = this.options.resolve(request, caller);
    if (!sourceRequest) return reply("UNSUPPORTED");
    // The UI cannot expand scope or create a new operation through a view mapping.
    if (sourceRequest.domainId !== request.domainId || sourceRequest.family === "K-UI" ||
        (request.operation === "read-models" &&
          !V37_READ_OPERATIONS[sourceRequest.family].includes(sourceRequest.operation))) {
      return reply("DENIED");
    }
    if (!this.options.granted(caller, sourceRequest)) return reply("DENIED");
    const sourceBytes = await this.options.source.execute(encodeV37Request(sourceRequest));
    const sourceReceipt = decodeV37Receipt(sourceBytes);
    if (sourceReceipt.family !== sourceRequest.family ||
        sourceReceipt.operation !== sourceRequest.operation ||
        sourceReceipt.requestId !== sourceRequest.requestId ||
        sourceReceipt.targetId !== sourceRequest.targetId) {
      throw new Error("V37_UI_SOURCE_RECEIPT_MISMATCH");
    }
    return sourceBytes;
  }
}
