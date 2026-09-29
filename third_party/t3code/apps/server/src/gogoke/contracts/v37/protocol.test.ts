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
import { V37M1FakePort, V37M1FakeStore } from "./m1Fake.ts";
import { runV37M2ContractCases } from "./m2Conformance.ts";
import { V37M2FakePort, V37M2FakeStore } from "./m2Fake.ts";
import { decodeV37Receipt, decodeV37Request, encodeV37Request, V37_SCHEMA, V37UnwiredPort, type V37Request, type V37TrustedCaller } from "./protocol.ts";

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
  it("runs reusable side, policy and worktree behavior cases on the fake", async () => {
    await runV37M2ContractCases((caseId) => {
      const store = new V37M2FakeStore();
      const principal = caseId.startsWith("worktree-") ?
        { ...caller, seatId: "seatA", role: "seat" as const } : caller;
      const options = {
        caller: () => principal,
        granted: () => true,
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
        reconstruct: () => new V37M2FakePort(store, options) };
    });
  });
  it("runs reusable QCard, seat and instance behavior cases on the fake", async () => {
    await runV37M1ContractCases((caseId) => {
      const store = new V37M1FakeStore();
      store.templates.set("templateA", { instruction: "default" });
      let grant = true;
      const principal = caseId === "seat-lead" ?
        { ...caller, seatId: "leadSeat", role: "lead" as const } : caller;
      const options = {
        caller: () => principal,
        granted: () => grant,
        nativeCardCapability: () => caseId === "card-native",
        verifyMemoryDisabled: () => caseId !== "instance-unverified",
        verifyProgramDigest: (digest: string) =>
          digest === "verifiedDigest" || digest === "newVerifiedDigest",
        isSeatBusy: () => caseId === "seat-busy",
        capacity: () => "3",
      };
      return { port: new V37M1FakePort(store, options),
        reconstruct: () => new V37M1FakePort(store, options), revoke: () => { grant = false; } };
    });
  });
  it("runs operational session, ledger and inbox contract cases on the fake", async () => {
    await runV37CoreContractCases((caseId) => {
      const store = new V37CoreFakeStore();
      let grant = true;
      const calls: string[] = [];
      const options = {
        caller: () => caller,
        granted: (_principal: V37TrustedCaller, r: V37Request) => grant &&
          !(r.family === "K-LEDGER" && r.operation === "scoped-query" && r.payload.scope === "GLOBAL"),
        verifyPinnedBinary: (digest: string) => digest === "verifiedDigest",
        verifyStopProof: (proof: string) => proof === "verifiedProof",
        prepareDelivery: async () => { calls.push("prepare"); if (caseId === "inbox-revoke") grant = false; },
        beginCommitted: async () => { calls.push("beginCommitted"); },
        completeDelivery: async () => { calls.push("completion"); return "unknown" as const; },
      };
      return { port: new V37CoreFakePort(store, options),
        reconstruct: () => new V37CoreFakePort(store, options), deliveryCalls: calls };
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
