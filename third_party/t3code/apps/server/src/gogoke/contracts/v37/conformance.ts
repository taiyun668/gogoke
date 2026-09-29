import { canonicalJson } from "../strictJson.ts";
import { V37_READ_OPERATIONS } from "./catalog.ts";
import { decodeV37Receipt, encodeV37Request, V37_SCHEMA, type V37Port, type V37Request } from "./protocol.ts";

/** Both the fake and each real implementation must use these same behavioral cases. */
export async function runV37Conformance(
  port: V37Port,
  request: V37Request,
): Promise<void> {
  if (request.family === "K-UI") {
    throw new Error("V37_CONFORMANCE: K-UI requires exact forwarding cases");
  }
  const first = decodeV37Receipt(await port.execute(encodeV37Request(request)));
  if (first.family !== request.family || first.operation !== request.operation ||
      first.requestId !== request.requestId || first.targetId !== request.targetId ||
      first.schema !== V37_SCHEMA) {
    throw new Error("V37_CONFORMANCE: receipt correlation failed");
  }
  if (first.status !== "APPLIED") throw new Error(`V37_CONFORMANCE: first call ${first.status}`);
  const second = decodeV37Receipt(await port.execute(encodeV37Request(request)));
  if (second.status !== "REPLAYED" || second.revision !== first.revision ||
      canonicalJson(second.result) !== canonicalJson(first.result)) {
    throw new Error("V37_CONFORMANCE: exact replay changed the result");
  }
  if (V37_READ_OPERATIONS[request.family].includes(request.operation)) {
    if (first.revision !== first.previousRevision) {
      throw new Error("V37_CONFORMANCE: read mutated durable revision");
    }
  } else if (BigInt(first.revision) !== BigInt(first.previousRevision) + 1n) {
    throw new Error("V37_CONFORMANCE: write did not advance one durable revision");
  }
}
