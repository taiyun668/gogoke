import * as assert from "node:assert/strict";
import * as fs from "node:fs";
import { describe, it } from "vite-plus/test";
import { decodePublicObject, encodePublicObject } from "../codec.ts";
import { V37_OPERATIONS } from "./catalog.ts";
import { runV37Conformance } from "./conformance.ts";
import { V37FakePort, V37FakeStore } from "./fake.ts";
import { runV37CoreContractCases } from "./coreConformance.ts";
import { V37CoreFakePort, V37CoreFakeStore } from "./coreFake.ts";
import { runV37M1ContractCases } from "./m1Conformance.ts";
import { V37M1FakePort, V37M1FakeStore, type V37TakeoverContext } from "./m1Fake.ts";
import { runV37M2ContractCases } from "./m2Conformance.ts";
import { V37M2FakePort, V37M2FakeStore } from "./m2Fake.ts";
import { runV37UiContractCases } from "./uiConformance.ts";
import { V37UiForwardingFakePort } from "./uiFake.ts";
import { decodeV37Receipt, decodeV37Request, encodeV37Receipt, encodeV37Request, V37_SCHEMA, V37UnwiredPort, type V37Port, type V37Request, type V37TrustedCaller } from "./protocol.ts";

const caller: V37TrustedCaller = {
  principalId: "owner", seatId: "lead", domainId: "projectA", role: "user",
  policyRevision: "1", revocationHead: "0",
};

function request(family: V37Request["family"], operation: V37Request["operation"],
  requestId: string, expectedRevision = "0"): V37Request {
  return { schema: V37_SCHEMA, family, operation, requestId, targetId: "targetA",
    domainId: "projectA", expectedRevision, payload: {} };
}

describe("design 37 closed operation protocol", () => {
  it("runs exact UI source-forwarding cases without a UI state store", async () => {
    await runV37UiContractCases((caseId) => {
      const forwarded: V37Request[] = [];
      const sourceRequest: V37Request = { schema: V37_SCHEMA,
        family: caseId === "action" ? "K-INBOX" : caseId === "bad-family" ? "K-SIDE" : "K-SEAT",
        operation: caseId === "action" ? "steer" : caseId === "bad-family" ? "resume" : "state-card",
        requestId: "sourceA", targetId: "seatA", domainId: "projectA",
        expectedRevision: "4", payload: {} };
      const sourceBytes = encodeV37Receipt({ schema: V37_SCHEMA,
        family: sourceRequest.family, operation: sourceRequest.operation,
        requestId: sourceRequest.requestId, targetId: sourceRequest.targetId,
        status: caseId === "action" ? "UNKNOWN" : "APPLIED",
        previousRevision: "4", revision: "4", result: { state: "UNKNOWN" } });
      const source = { execute: async (bytes: Uint8Array) => {
        forwarded.push(decodeV37Request(bytes));
        const receipt = decodeV37Receipt(sourceBytes);
        return caseId === "bad-family" ? encodeV37Receipt({ ...receipt, family: "K-SESSION" })
          : caseId === "bad-operation" ? encodeV37Receipt({ ...receipt, operation: "tune" })
          : caseId === "bad-request-id" ? encodeV37Receipt({ ...receipt, requestId: "wrongId" })
          : caseId === "bad-target-id" ? encodeV37Receipt({ ...receipt, targetId: "wrongSeat" })
          : sourceBytes;
      } };
      const port = new V37UiForwardingFakePort({
        caller: () => caller,
        granted: (_principal, r) =>
          !(caseId === "outer-denied" && r.family === "K-UI") &&
          !(caseId === "source-denied" && r.family !== "K-UI"),
        resolve: () => caseId === "unmapped" ? null : {
          ...sourceRequest,
          domainId: caseId === "cross-domain" ? "projectB" : "projectA",
          operation: caseId === "read-write" ? "tune" : sourceRequest.operation,
        },
        source,
      });
      return { port, sourceBytes, expectedSourceRequest: sourceRequest, forwarded };
    });
  });
  it("runs reusable side, policy and worktree behavior cases on the fake", async () => {
    await runV37M2ContractCases((caseId) => {
      const store = new V37M2FakeStore();
      let grant = true;
      const permissionEntries = { seatA: "seatB" };
      let policyRevision = "1";
      const principal = caseId.startsWith("worktree-") ?
        { ...caller, seatId: "seatA", role: "seat" as const } : caller;
      const options = {
        caller: () => principal,
        granted: () => grant,
        permissionTable: () => caseId === "permission-table" ?
          { revision: policyRevision, entries: permissionEntries } : null,
        verifyRepository: (repositoryId: string) => repositoryId === "verifiedRepo",
        verifyIsolation: () => true,
        stopConfirmed: () => caseId !== "worktree-no-stop",
        activeAdmissions: () => caseId === "worktree-active-reservation" ? 1 : 0,
        mergeGranted: () => caseId !== "worktree-no-grant",
        performMerge: () => caseId === "worktree-merge-unknown" ? "unknown" as const : "merged" as const,
        classify: () => "SINGLE" as const,
        removeSideLedgerTier: () => caseId !== "side-delete-denied",
        scheduleTrigger: () => true,
        cancelTrigger: () => true,
      };
      return { port: new V37M2FakePort(store, options),
        reconstruct: () => new V37M2FakePort(store, options),
        revoke: () => { grant = false; },
        mutatePermissionTable: () => { permissionEntries.seatA = "seatC"; },
        advancePolicy: () => { policyRevision = "2"; } };
    });
  });
  it("runs reusable QCard, seat and instance behavior cases on the fake", async () => {
    let reclaimedStore: V37M1FakeStore | undefined;
    await runV37M1ContractCases((caseId) => {
      const store = new V37M1FakeStore();
      if (caseId === "seat-takeover") reclaimedStore = store;
      store.templates.set("templateA", { instruction: "default" });
      let grant = true;
      let takeoverContext: V37TakeoverContext | null = caseId === "seat-takeover-unwired" ? null :
        { epoch: "epochA", takerSeatId: "lead", instanceId: null,
          questionIds: ["purpose", "authority"] };
      const principal = caseId === "seat-lead" ?
        { ...caller, seatId: "leadSeat", role: "lead" as const } :
        caseId === "instance-seat-denied" ?
          { ...caller, domainId: "global", role: "seat" as const } :
          caseId.startsWith("instance") ? { ...caller, domainId: "global" } : caller;
      const options = {
        caller: () => principal,
        granted: () => grant,
        nativeCardCapability: () => caseId === "card-native",
        verifyMemoryDisabled: () => caseId !== "instance-unverified",
        verifyProgramDigest: (digest: string) =>
          digest === "verifiedDigest" || digest === "newVerifiedDigest",
        hostRegistration: () => ({ homeRef: "homeA", programDigest: "verifiedDigest", version: "1" }),
        hostProgramUpgrade: () => ({ programDigest: "newVerifiedDigest", version: "2" }),
        isSeatBusy: () => caseId === "seat-busy",
        capacity: () => "3",
        isTakeoverLead: (seatId: string) => seatId === "leadA",
        takeoverContext: () => takeoverContext,
        createTemporaryHome: () => caseId === "instance-home-unknown" ? "UNKNOWN" as const :
          { directoryRef: "opaqueTempA", nativeReceiptId: "createdA" },
        verifyTemporaryHomeOwner: (_instanceId: string, ownerDomainId: string,
          ownerId: string, generation: string) => ownerDomainId === "projectA" &&
          ownerId === "sessionA" && generation === "1",
        closeTemporaryHome: () => caseId === "instance-home-no-stop" ? null : "stoppedA",
        cleanupTemporaryHome: () => "cleanedA",
        activeInstanceAdmissions: () => caseId === "instance-home-busy" ? 1 : 0,
        verifyTemporaryHomeIdentity: () => true,
      };
      return { port: new V37M1FakePort(store, options),
        reconstruct: () => new V37M1FakePort(store, options), revoke: () => { grant = false; },
        setTakeoverContext: (context: V37TakeoverContext | null) => { takeoverContext = context; } };
    });
    assert.equal(reclaimedStore?.seats.get("projectA:leadA")?.takeover, undefined);
  });
  it("runs operational session, ledger and inbox contract cases on the fake", async () => {
    await runV37CoreContractCases((caseId) => {
      const store = new V37CoreFakeStore();
      if (caseId.startsWith("session") && !caseId.startsWith("session-cap-")) {
        // Every ordinary session case has explicit E/F fixture facts; no fake default.
        store.setProjectParallelCap("projectA", 1n);
        store.setInstanceConcurrencyCap("instanceA", 1n);
      }
      let grant = true;
      let activeTurn: string | null = caseId === "inbox-steer-ended" ? null : "turnA";
      let ledgerScopeReads = 0;
      const calls: string[] = [];
      const resumeCalls: string[] = [];
      let bindingReads = 0;
      let resumeReconciliations = 0;
      const options = {
        caller: () => caller,
        granted: (_principal: V37TrustedCaller, r: V37Request) => grant &&
          !(r.family === "K-LEDGER" && r.operation === "scoped-query" && r.payload.scope === "GLOBAL"),
        canReadLedgerScope: (_principal: V37TrustedCaller, scope: string) => {
          ledgerScopeReads += 1;
          return scope !== "GLOBAL" &&
            !(caseId === "ledger-subscription-revoked" && ledgerScopeReads > 1);
        },
        verifyPinnedBinary: (digest: string) => digest === "verifiedDigest",
        verifyStopProof: (proof: string) => proof === "verifiedProof",
        admissionInstance: () => "instanceA",
        sessionCapabilities: () => ({ compact: caseId !== "session-unsupported",
          "renew-session": true,
          resume: caseId.startsWith("session-resume") && caseId !== "session-resume-unsupported" }),
        sessionBinding: () => {
          bindingReads += 1;
          return { driverId: "driverA",
            instanceId: caseId === "session-resume-binding-mismatch" && bindingReads > 1 ?
              "instanceB" : "instanceA", pinnedBinaryDigest: "verifiedDigest" };
        },
        resumeCustody: () => caseId === "session-resume-custody-unknown" ?
          "unknown" as const : "confirmed" as const,
        resumeGeneration: (sessionId: string, oldGeneration: string) => {
          resumeCalls.push(`${sessionId}:${oldGeneration}`);
          return caseId === "session-resume-vendor-unknown" ? "unknown" as const :
            { newGeneration: "2", receiptId: "resumeReceipt" };
        },
        reconcileResume: () => {
          resumeReconciliations += 1;
          return caseId === "session-resume-vendor-unknown" && resumeReconciliations > 1 ?
            { newGeneration: "2", receiptId: "reconciledReceipt" } : "unknown" as const;
        },
        readOutput: () => ({ cursor: "1", events: [{ eventId: "outputA", kind: "message" }] }),
        sendInput: (_sessionId: string, operation: "send" | "append-without-turn") =>
          ({ receiptId: "inputReceipt", createdTurn: operation === "send" }),
        changeGeneration: (_sessionId: string, _operation: "compact" | "renew-session", oldGeneration: string) =>
          caseId === "session-unknown" || caseId === "session-cap-conservative" ? "unknown" as const :
            { newGeneration: (BigInt(oldGeneration) + 1n).toString(), receiptId: "generationReceipt" },
        reconnectGeneration: (_sessionId: string, claimedGeneration: string) =>
          ({ generation: caseId === "session-unknown" ? "2" : claimedGeneration,
            receiptId: "reconnectReceipt" }),
        currentTurn: () => activeTurn,
        steerMode: () => caseId === "inbox-steer-fallback" ? "INTERRUPT_RESUME" as const : "NATIVE" as const,
        canRequeueTarget: (seatId: string, turnId: string, generation: string) =>
          seatId === "seatA" && turnId === "turnA" && generation === "1",
        prepareDelivery: async () => {
          calls.push("prepare");
          if (caseId === "inbox-revoke") grant = false;
          if (caseId === "inbox-steer-race" || caseId === "inbox-steer-abort-unknown") activeTurn = null;
        },
        abortPreparedDelivery: async () => {
          calls.push("abort"); return caseId !== "inbox-steer-abort-unknown";
        },
        beginCommitted: async () => { calls.push("beginCommitted"); },
        completeDelivery: async () => {
          calls.push("completion");
          return caseId === "inbox-failed" ?
            { state: "failed" as const, error: "native delivery rejected" } :
            caseId === "inbox-unknown" ? "unknown" as const : "completed" as const;
        },
      };
      return { port: new V37CoreFakePort(store, options),
        reconstruct: () => new V37CoreFakePort(store, options), deliveryCalls: calls,
        resumeCalls,
        writeProjectCap: (domainId: string, cap: bigint) => store.setProjectParallelCap(domainId, cap),
        writeInstanceCap: (instanceId: string, cap: bigint) => store.setInstanceConcurrencyCap(instanceId, cap) };
    });
  });
  it("exercises the common envelope for non-UI fake operations", async () => {
    let number = 0;
    for (const [family, operations] of Object.entries(V37_OPERATIONS)) {
      if (family === "K-UI") continue;
      for (const operation of operations) {
        number += 1;
        const port = new V37FakePort(new V37FakeStore(), () => caller, () => true);
        await runV37Conformance(port, request(family as V37Request["family"],
          operation as V37Request["operation"], `request${number}`));
      }
    }
    assert.equal(number, 66);
  });

  it("does not pretend the unwired UI integrator can forward a receipt", async () => {
    const port = new V37FakePort(new V37FakeStore(), () => caller, () => true);
    for (const operation of ["actions", "read-models"] as const) {
      const result = decodeV37Receipt(await port.execute(encodeV37Request(
        request("K-UI", operation, `ui${operation}`))));
      assert.equal(result.status, "UNSUPPORTED");
      assert.equal(result.revision, result.previousRevision);
    }
    await assert.rejects(() => runV37Conformance(port,
      request("K-UI", "actions", "uiActionB")), /requires exact forwarding cases/);
  });

  it("fails closed without a native caller or an explicit grant", async () => {
    const target = request("K-WORKTREE", "merge", "mergeA");
    const store = new V37FakeStore();
    for (const port of [new V37FakePort(store, () => null, () => true),
      new V37FakePort(store, () => caller, () => false)]) {
      const result = decodeV37Receipt(await port.execute(encodeV37Request(target)));
      assert.equal(result.status, "DENIED");
      assert.equal(result.revision, "0");
    }
    await assert.rejects(() => new V37UnwiredPort().execute(encodeV37Request(target)), /UNWIRED/);
  });

  it("serializes same-target races and distinguishes replay, conflict and stale", async () => {
    const port = new V37FakePort(new V37FakeStore(), () => caller, () => true);
    const a = request("K-INBOX", "enqueue", "requestA");
    const b = request("K-INBOX", "enqueue", "requestB");
    const results = await Promise.all([a, b].map(async (r) =>
      decodeV37Receipt(await port.execute(encodeV37Request(r)))));
    assert.deepEqual(results.map((r) => r.status), ["APPLIED", "STALE"]);
    const replay = decodeV37Receipt(await port.execute(encodeV37Request(a)));
    assert.equal(replay.status, "REPLAYED");
    const conflict = decodeV37Receipt(await port.execute(encodeV37Request({ ...a, payload: { other: true } })));
    assert.equal(conflict.status, "CONFLICT");
    const retry = decodeV37Receipt(await port.execute(encodeV37Request({ ...b, expectedRevision: "1" })));
    assert.equal(retry.status, "APPLIED");
    assert.equal(retry.revision, "2");
  });

  it("treats reordered valid wire bytes as a request-ID collision", async () => {
    const m1Store = new V37M1FakeStore();
    m1Store.templates.set("templateA", { instruction: "default" });
    const coreStore = new V37CoreFakeStore();
    coreStore.setProjectParallelCap("projectA", 1n);
    coreStore.setInstanceConcurrencyCap("instanceA", 1n);
    const cases: readonly { name: string; port: V37Port; request: V37Request }[] = [
      { name: "generic", port: new V37FakePort(new V37FakeStore(), () => caller, () => true),
        request: request("K-INBOX", "enqueue", "rawBytesGeneric") },
      { name: "core", port: new V37CoreFakePort(coreStore,
        { caller: () => caller, granted: () => true, admissionInstance: () => "instanceA" }),
      request: { ...request("K-SESSION", "admission-reserve", "rawBytesCore"),
        payload: { generation: "1" } } },
      { name: "M1", port: new V37M1FakePort(m1Store,
        { caller: () => caller, granted: () => true }),
      request: { ...request("K-SEAT", "create-from-template", "rawBytesM1"),
        payload: { layer: "USER", templateId: "templateA" } } },
      { name: "M2", port: new V37M2FakePort(new V37M2FakeStore(),
        { caller: () => caller, granted: () => true }),
      request: request("K-POLICY", "gate-submit", "rawBytesM2") },
    ];
    for (const { name, port, request: r } of cases) {
      const original = encodeV37Request(r);
      const reordered = new TextEncoder().encode(JSON.stringify({ payload: r.payload,
        expectedRevision: r.expectedRevision, domainId: r.domainId, targetId: r.targetId,
        requestId: r.requestId, operation: r.operation, family: r.family, schema: r.schema }));
      assert.deepEqual(decodeV37Request(reordered), decodeV37Request(original), name);
      assert.notDeepEqual(reordered, original, name);
      assert.equal(decodeV37Receipt(await port.execute(original)).status, "APPLIED", name);
      assert.equal(decodeV37Receipt(await port.execute(original)).status, "REPLAYED", name);
      assert.equal(decodeV37Receipt(await port.execute(reordered)).status, "CONFLICT", name);
    }
  });

  it("retains deduplication and revisions across fake port reconstruction", async () => {
    const store = new V37FakeStore();
    const initial = new V37FakePort(store, () => caller, () => true);
    const r = request("K-SESSION", "open", "openA");
    assert.equal(decodeV37Receipt(await initial.execute(encodeV37Request(r))).status, "APPLIED");
    const recovered = new V37FakePort(store, () => caller, () => true);
    assert.equal(decodeV37Receipt(await recovered.execute(encodeV37Request(r))).status, "REPLAYED");
    assert.equal(store.revisions.get("K-SESSION:projectA:targetA"), 1n);
  });

  it("rejects caller identity in wire, unknown operations and unsafe revisions", () => {
    const bytes = encodeV37Request(request("K-SEAT", "reclaim", "reclaimA"));
    assert.equal(decodeV37Request(bytes).operation, "reclaim");
    const text = new TextDecoder().decode(bytes);
    assert.throws(() => decodeV37Request(new TextEncoder().encode(text.replace('"payload":{}', '"payload":{},"caller":{"role":"user"}'))));
    assert.throws(() => encodeV37Request({ ...request("K-SEAT", "reclaim", "reclaimB"), operation: "invented" as V37Request["operation"] }));
    assert.throws(() => encodeV37Request(request("K-SEAT", "reclaim", "reclaimC", "01")));
  });

  it("keeps the existing stored-object codec compatible and separate", () => {
    const fixture = fs.readFileSync(new URL(
      "../../../../../../../../apps/desktop/contracts/s1-r4/fixtures/runtime-instance.valid.json",
      import.meta.url));
    const oldObject = decodePublicObject(fixture);
    assert.deepEqual(decodePublicObject(encodePublicObject(oldObject)), oldObject);
    assert.throws(() => decodeV37Request(fixture));
    assert.throws(() => decodePublicObject(encodeV37Request(
      request("K-INSTANCE", "register", "registerA"))));
  });
});
