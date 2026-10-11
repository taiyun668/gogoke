import * as assert from "node:assert/strict";
import { decodeV37Receipt, encodeV37Request, V37_SCHEMA,
  type V37Port, type V37Request } from "./protocol.ts";

export type V37UiCase = "read" | "action" | "outer-denied" | "source-denied" |
  "unmapped" | "cross-domain" | "read-write" | "bad-family" |
  "bad-operation" | "bad-request-id" | "bad-target-id";
export interface V37UiHarness {
  readonly port: V37Port;
  readonly sourceBytes: Uint8Array;
  readonly expectedSourceRequest: V37Request;
  readonly forwarded: V37Request[];
}
export type V37UiHarnessFactory = (caseId: V37UiCase) => V37UiHarness;

export async function runV37UiContractCases(factory: V37UiHarnessFactory): Promise<void> {
  for (const caseId of ["read", "action", "outer-denied", "source-denied",
    "unmapped", "cross-domain", "read-write", "bad-family",
    "bad-operation", "bad-request-id", "bad-target-id"] as const) {
    const h = factory(caseId);
    const request: V37Request = { schema: V37_SCHEMA, family: "K-UI",
      operation: caseId === "action" || caseId === "bad-family" ? "actions" : "read-models",
      requestId: `ui${caseId}`, targetId: "viewA", domainId: "projectA",
      expectedRevision: "0", payload: { caller: "forged", key: "viewA" } };
    if (caseId.startsWith("bad-")) {
      await assert.rejects(() => h.port.execute(encodeV37Request(request)),
        /SOURCE_RECEIPT_MISMATCH/);
      assert.equal(h.forwarded.length, 1);
      assert.deepEqual(encodeV37Request(h.forwarded[0]!), encodeV37Request(h.expectedSourceRequest));
      continue;
    }
    const bytes = await h.port.execute(encodeV37Request(request));
    if (caseId === "read" || caseId === "action") {
      assert.deepEqual(bytes, h.sourceBytes);
      const receipt = decodeV37Receipt(bytes);
      assert.equal(receipt.status, caseId === "action" ? "UNKNOWN" : "APPLIED");
      assert.equal(receipt.family, caseId === "action" ? "K-INBOX" : "K-SEAT");
      assert.equal(h.forwarded.length, 1);
      assert.deepEqual(encodeV37Request(h.forwarded[0]!), encodeV37Request(h.expectedSourceRequest));
    } else {
      const receipt = decodeV37Receipt(bytes);
      assert.equal(receipt.status, caseId === "unmapped" ? "UNSUPPORTED" : "DENIED");
      assert.equal(h.forwarded.length, 0);
    }
  }
}
