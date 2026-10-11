import * as assert from "node:assert/strict";
import type { V37TakeoverContext } from "./m1Fake.ts";
import { decodeV37Receipt, encodeV37Request, V37_SCHEMA, type V37Port, type V37Receipt, type V37Request } from "./protocol.ts";

export type V37M1Case = "card-fallback" | "card-native" | "seat-user" | "seat-busy" |
  "seat-lead" | "seat-revoked" | "seat-takeover" | "seat-takeover-unwired" |
  "instance" | "instance-unverified" | "instance-seat-denied" |
  "instance-home" | "instance-home-no-stop" |
  "instance-home-busy" | "instance-home-unknown";
export interface V37M1Harness {
  readonly port: V37Port; reconstruct(): V37Port; revoke(): void;
  setTakeoverContext(context: V37TakeoverContext | null): void;
}
export type V37M1HarnessFactory = (caseId: V37M1Case) => V37M1Harness;

const req = (family: V37Request["family"], operation: V37Request["operation"],
  requestId: string, targetId: string, expectedRevision: string,
  payload: V37Request["payload"] = {}): V37Request => ({
  schema: V37_SCHEMA, family, operation, requestId, targetId,
  domainId: "projectA", expectedRevision, payload,
});
const instanceReq = (...args: Parameters<typeof req>): V37Request =>
  ({ ...req(...args), domainId: "global" });
const call = async (port: V37Port, request: V37Request): Promise<V37Receipt> => {
  const result = decodeV37Receipt(await port.execute(encodeV37Request(request)));
  assert.equal(result.requestId, request.requestId);
  assert.equal(result.targetId, request.targetId);
  assert.equal(result.family, request.family);
  return result;
};

/** Behavioral cases shared by fake and future native-backed C/E/F implementations. */
export async function runV37M1ContractCases(factory: V37M1HarnessFactory): Promise<void> {
  const cardPayload = { driverId: "driverA", requestRef: "questionA", seatId: "seatA",
    turnId: "turnA", generation: "1", options: [
      { id: "yes", recommended: true }, { id: "no", recommended: false },
    ] } as const;
  {
    const h = factory("card-native");
    const native = await call(h.port, req("K-QCARD", "raise", "nativeCard", "cardA", "0", cardPayload));
    assert.equal(native.status, "UNSUPPORTED");
    assert.equal(native.revision, "0");
  }
  {
    const h = factory("card-fallback");
    const bad = { ...cardPayload, options: [{ id: "yes", recommended: true },
      { id: "no", recommended: true }] };
    await assert.rejects(() => call(h.port, req("K-QCARD", "raise", "badCard", "cardA", "0", bad)),
      /exactly one recommended/);
    const raised = req("K-QCARD", "raise", "raiseA", "cardA", "0", cardPayload);
    assert.equal((await call(h.port, raised)).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), raised)).status, "REPLAYED");
    const recovered = await call(h.reconstruct(), req("K-QCARD", "recover", "recoverA", "cardA", "1"));
    assert.equal(recovered.result.state, "OPEN");
    assert.equal(recovered.result.turnId, "turnA");
    const answerPayload = { requestRef: "questionA", seatId: "seatA", turnId: "turnA",
      generation: "1", optionId: "yes" };
    assert.equal((await call(h.port, req("K-QCARD", "answer", "wrongGeneration", "cardA", "2",
      { ...answerPayload, generation: "2" }))).status, "CONFLICT");
    const race = await Promise.all([
      call(h.port, req("K-QCARD", "answer", "answerA", "cardA", "2", answerPayload)),
      call(h.port, req("K-QCARD", "expire", "expireA", "cardA", "2")),
    ]);
    assert.deepEqual(race.map((r) => r.status).sort(), ["APPLIED", "STALE"]);
    assert.equal((await call(h.port, req("K-QCARD", "answer", "secondAnswer", "cardA", "3",
      answerPayload))).status, "CONFLICT");
    assert.equal((await call(h.reconstruct(), req("K-QCARD", "recover", "recoverTerminal", "cardA", "3"))).status,
      "CONFLICT");
  }
  {
    const h = factory("seat-user");
    const createA = req("K-SEAT", "create-from-template", "seatCreateA", "seatA", "0",
      { layer: "USER", templateId: "templateA" });
    const createdA = await call(h.port, createA);
    assert.equal(createdA.status, "APPLIED");
    assert.equal(createdA.result.kind, "LONG");
    assert.equal(createdA.result.state, "IDLE");
    const replayA = await call(h.reconstruct(), createA);
    assert.equal(replayA.status, "REPLAYED");
    assert.equal(replayA.result.kind, "LONG");
    assert.equal(replayA.result.state, "IDLE");
    assert.equal((await call(h.port, req("K-SEAT", "create-from-template", "seatCreateB", "seatB", "0",
      { layer: "USER", templateId: "templateA" }))).status, "APPLIED");
    assert.equal((await call(h.port, req("K-SEAT", "tune", "tuneA", "seatA", "1",
      { setting: "instruction", value: "changed" }))).status, "APPLIED");
    const other = await call(h.port, req("K-SEAT", "state-card", "otherView", "seatB", "1"));
    assert.equal((other.result.settings as { instruction: string }).instruction, "default");
    assert.equal(other.revision, other.previousRevision);
    assert.equal((await call(h.port, req("K-SEAT", "reclaim", "reclaimA", "seatA", "2"))).status, "APPLIED");
    assert.equal((await call(h.port, req("K-SEAT", "tune", "tuneReclaimed", "seatA", "3",
      { setting: "instruction", value: "forbidden" }))).status, "CONFLICT");
  }
  {
    const h = factory("seat-revoked");
    assert.equal((await call(h.port, req("K-SEAT", "create-from-template", "revokedCreate", "seatR", "0",
      { layer: "USER", templateId: "templateA" }))).status, "APPLIED");
    h.revoke();
    const denied = await call(h.reconstruct(), req("K-SEAT", "state-card", "revokedRead", "seatR", "1"));
    assert.equal(denied.status, "DENIED");
    assert.equal(denied.revision, "1");
  }
  {
    const h = factory("seat-takeover-unwired");
    assert.equal((await call(h.port, req("K-SEAT", "create-from-template", "unwiredLead", "leadA", "0",
      { layer: "USER", templateId: "templateA" }))).status, "APPLIED");
    assert.equal((await call(h.port, req("K-SEAT", "takeover-answers", "unwiredAnswers", "leadA", "1",
      { takeoverEpoch: "epochA", answers: [{ questionId: "purpose", answer: "UNKNOWN", howToFind: "Inspect repository" }] }))).status,
      "UNSUPPORTED");
  }
  {
    const h = factory("seat-takeover");
    const createdLead = await call(h.port, req("K-SEAT", "create-from-template", "takeoverLead", "leadA", "0",
      { layer: "USER", templateId: "templateA" }));
    assert.equal(createdLead.status, "APPLIED");
    assert.equal(createdLead.result.kind, "LONG");
    assert.equal(createdLead.result.state, "IDLE");
    const missing = await call(h.port, req("K-SEAT", "takeover-answers", "missingAnswer", "leadA", "1",
      { takeoverEpoch: "epochA", answers: [{ questionId: "purpose", answer: "Build app", sourceRef: "repo:README" }] }));
    assert.equal(missing.status, "CONFLICT");
    assert.equal(missing.revision, "1");
    assert.equal((await call(h.port, req("K-SEAT", "takeover-answers", "inventedAnswer", "leadA", "1",
      { takeoverEpoch: "epochA", answers: [{ questionId: "purpose", answer: "Build app", sourceRef: "repo:README" },
        { questionId: "authority", answer: "Owner", sourceRef: "" }] }))).status, "CONFLICT");
    const answers = req("K-SEAT", "takeover-answers", "takeoverAnswers", "leadA", "1",
      { takeoverEpoch: "epochA", answers: [{ questionId: "purpose", answer: "Build app", sourceRef: "repo:README" },
        { questionId: "authority", answer: "UNKNOWN", howToFind: "Inspect decision register" }] });
    assert.equal((await call(h.port, answers)).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), answers)).status, "REPLAYED");
    const card = await call(h.reconstruct(), req("K-SEAT", "state-card", "takeoverCard", "leadA", "2"));
    assert.equal(card.result.takeoverReady, true);
    assert.equal((card.result.takeoverAnswers as Record<string, { howToFind?: string }>).authority?.howToFind,
      "Inspect decision register");
    h.setTakeoverContext({ epoch: "epochA", takerSeatId: "lead", instanceId: null,
      questionIds: ["purpose", "authority", "workspace"] });
    assert.equal((await call(h.reconstruct(), req("K-SEAT", "state-card", "takeoverCard", "leadA", "2"))).status,
      "STALE");
    assert.equal((await call(h.reconstruct(), req("K-SEAT", "state-card", "changedQuestions", "leadA", "2"))).result.takeoverReady,
      false);
    h.setTakeoverContext({ epoch: "epochA", takerSeatId: "otherLead", instanceId: null,
      questionIds: ["purpose", "authority"] });
    assert.equal((await call(h.reconstruct(), req("K-SEAT", "state-card", "changedTaker", "leadA", "2"))).result.takeoverReady,
      false);
    assert.equal((await call(h.port, req("K-SEAT", "takeover-answers", "wrongTaker", "leadA", "2",
      { ...answers.payload, takeoverEpoch: "epochA" }))).status, "DENIED");
    h.setTakeoverContext({ epoch: "epochB", takerSeatId: "lead", instanceId: null,
      questionIds: ["purpose", "authority"] });
    assert.equal((await call(h.reconstruct(), req("K-SEAT", "state-card", "changedEpoch", "leadA", "2"))).result.takeoverReady,
      false);
    assert.equal((await call(h.port, req("K-SEAT", "takeover-answers", "oldEpoch", "leadA", "2",
      answers.payload))).status, "STALE");
    assert.equal((await call(h.port, req("K-SEAT", "change-instance", "takeoverSwap", "leadA", "2",
      { instanceId: "instanceB" }))).status, "CONFLICT");
    assert.equal((await call(h.port, req("K-SEAT", "bind-instance", "takeoverBind", "leadA", "2",
      { instanceId: "instanceA" }))).status, "APPLIED");
    const afterBind = await call(h.reconstruct(), req("K-SEAT", "state-card", "afterBind", "leadA", "3"));
    assert.equal(afterBind.result.takeoverReady, false);
    h.setTakeoverContext({ epoch: "epochC", takerSeatId: "lead", instanceId: "instanceA",
      questionIds: ["purpose", "authority"] });
    assert.equal((await call(h.port, req("K-SEAT", "takeover-answers", "answersAfterBind", "leadA", "3",
      { ...answers.payload, takeoverEpoch: "epochC" }))).status, "APPLIED");
    assert.equal((await call(h.port, req("K-SEAT", "change-instance", "successfulSwap", "leadA", "4",
      { instanceId: "instanceB" }))).status, "APPLIED");
    h.setTakeoverContext({ epoch: "epochD", takerSeatId: "lead", instanceId: "instanceB",
      questionIds: ["purpose", "authority"] });
    assert.equal((await call(h.reconstruct(), req("K-SEAT", "state-card", "afterSwap", "leadA", "5"))).result.takeoverReady,
      false);
    assert.equal((await call(h.port, req("K-SEAT", "takeover-answers", "answersAfterSwap", "leadA", "5",
      { ...answers.payload, takeoverEpoch: "epochD" }))).status, "APPLIED");
    const readyBeforeReclaim = req("K-SEAT", "state-card", "readyBeforeReclaim", "leadA", "6");
    assert.equal((await call(h.reconstruct(), readyBeforeReclaim)).result.takeoverReady, true);
    assert.equal((await call(h.port, req("K-SEAT", "reclaim", "reclaimLead", "leadA", "6"))).status,
      "APPLIED");
    const reclaimed = await call(h.reconstruct(), req("K-SEAT", "state-card", "reclaimedCard", "leadA", "7"));
    assert.equal(reclaimed.result.state, "RECLAIMED");
    assert.equal(reclaimed.result.takeoverReady, false);
    assert.equal(reclaimed.result.takeoverAnswers, null);
    assert.equal((await call(h.reconstruct(), readyBeforeReclaim)).status, "STALE");
    assert.equal((await call(h.reconstruct(), req("K-SEAT", "takeover-answers", "answersAfterReclaim", "leadA", "7",
      { ...answers.payload, takeoverEpoch: "epochD" }))).status, "CONFLICT");
    h.revoke();
    assert.equal((await call(h.reconstruct(), req("K-SEAT", "takeover-answers", "revokedAnswers", "leadA", "7",
      answers.payload))).status, "DENIED");
  }
  {
    const h = factory("seat-busy");
    assert.equal((await call(h.port, req("K-SEAT", "create-from-template", "busyCreate", "seatBusy", "0",
      { layer: "USER", templateId: "templateA" }))).status, "APPLIED");
    assert.equal((await call(h.port, req("K-SEAT", "bind-instance", "bindBusy", "seatBusy", "1",
      { instanceId: "instanceA" }))).status, "APPLIED");
    assert.equal((await call(h.port, req("K-SEAT", "change-instance", "changeBusy", "seatBusy", "2",
      { instanceId: "instanceB" }))).status, "CONFLICT");
  }
  {
    const h = factory("seat-lead");
    assert.equal((await call(h.port, req("K-SEAT", "create-from-template", "selfBind", "leadSeat", "0",
      { layer: "LEAD", templateId: "templateA", caller: { role: "user" } }))).status, "DENIED");
    assert.equal((await call(h.port, req("K-SEAT", "create-from-template", "wrongLayer", "userSeat", "0",
      { layer: "USER", templateId: "templateA" }))).status, "DENIED");
    const createChild = req("K-SEAT", "create-from-template", "childSeat", "childSeat", "0",
      { layer: "LEAD", templateId: "templateA" });
    const child = await call(h.port, createChild);
    assert.equal(child.status, "APPLIED");
    assert.equal(child.result.kind, "SHORT");
    assert.equal(child.result.state, "IDLE");
    const childReplay = await call(h.reconstruct(), createChild);
    assert.equal(childReplay.result.kind, "SHORT");
    assert.equal(childReplay.result.state, "IDLE");
    const promoted = await call(h.port, req("K-SEAT", "short-to-long", "keepChild", "childSeat", "1"));
    assert.equal(promoted.status, "APPLIED");
    assert.equal(promoted.result.kind, "LONG");
    assert.equal(promoted.result.state, "IDLE");
    const longChild = await call(h.port, req("K-SEAT", "state-card", "longChild", "childSeat", "2"));
    assert.equal(longChild.result.kind, "LONG");
    assert.equal(longChild.result.state, "IDLE");
  }
  {
    const h = factory("instance-seat-denied");
    assert.equal((await call(h.port, instanceReq("K-INSTANCE", "register", "seatCannotRegister",
      "instanceA", "0", { driverId: "codex" }))).status,
      "DENIED");
  }
  {
    const h = factory("instance-unverified");
    const invalid = await call(h.port, instanceReq("K-INSTANCE", "register", "unverified", "instanceA", "0",
      { driverId: "codex" }));
    assert.equal(invalid.status, "DENIED");
    assert.equal(invalid.revision, "0");
  }
  {
    const h = factory("instance");
    const register = instanceReq("K-INSTANCE", "register", "registerA", "instanceA", "0",
      { driverId: "codex" });
    await assert.rejects(() => call(h.port, { ...register, requestId: "forgedHome",
      payload: { driverId: "codex", homeRef: "borrowed" } }), /registration payload/);
    assert.equal((await call(h.port, register)).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), register)).status, "REPLAYED");
    assert.equal((await call(h.port, instanceReq("K-INSTANCE", "register", "sharedHome", "instanceB", "0",
      { driverId: "codex" }))).status, "DENIED");
    const login = await call(h.port, instanceReq("K-INSTANCE", "login-state", "loginRead", "instanceA", "1"));
    assert.equal(login.result.state, "UNKNOWN");
    assert.equal(login.revision, login.previousRevision);
    assert.equal((await call(h.port, instanceReq("K-INSTANCE", "install-state", "installRead", "instanceA", "1"))).result.installed,
      true);
    assert.equal((await call(h.port, instanceReq("K-INSTANCE", "repin-after-manual-upgrade", "badRepin", "instanceA", "1",
      { programDigest: "unverifiedDigest", version: "2" }))).status, "DENIED");
    const repin = instanceReq("K-INSTANCE", "repin-after-manual-upgrade", "repinA", "instanceA", "1");
    assert.equal((await call(h.port, repin)).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), repin)).status, "REPLAYED");
    const version = await call(h.reconstruct(), instanceReq("K-INSTANCE", "version-and-new-version", "versionRead", "instanceA", "2"));
    assert.equal(version.result.programDigest, "newVerifiedDigest");
    assert.equal(version.result.version, "2");
    assert.equal((await call(h.port, instanceReq("K-INSTANCE", "concurrency-input", "capacityRead", "instanceA", "2"))).result.capacity,
      "3");
    assert.equal(JSON.stringify(version.result).includes("credential"), false);
  }
  for (const caseId of ["instance-home", "instance-home-no-stop", "instance-home-busy",
    "instance-home-unknown"] as const) {
    const h = factory(caseId);
    assert.equal((await call(h.port, instanceReq("K-INSTANCE", "register", `${caseId}Register`, "instanceA", "0",
      { driverId: "codex" }))).status, "APPLIED");
    const create = instanceReq("K-INSTANCE", "home-lifecycle", `${caseId}Create`, "tempA", "0",
      { action: "CREATE", instanceId: "instanceA", ownerDomainId: "projectA",
        kind: "SESSION", ownerId: "sessionA",
        generation: "1" });
    assert.equal((await call(h.port, { ...create, requestId: `${caseId}WrongInstance`,
      payload: { ...create.payload, instanceId: "instanceB" } })).status, "DENIED");
    assert.equal((await call(h.port, { ...create, requestId: `${caseId}WrongOwnerDomain`,
      payload: { ...create.payload, ownerDomainId: "projectB" } })).status, "DENIED");
    assert.equal((await call(h.port, { ...create, requestId: `${caseId}OtherDomain`,
      domainId: "projectB" })).status, "DENIED");
    await assert.rejects(() => call(h.port, { ...create, requestId: `${caseId}ForgedPath`,
      payload: { ...create.payload, path: "untrusted-path" } }), /temporary home payload/);
    const created = await call(h.port, create);
    assert.equal(created.status, caseId === "instance-home-unknown" ? "UNKNOWN" : "APPLIED");
    assert.equal((await call(h.reconstruct(), create)).status,
      caseId === "instance-home-unknown" ? "UNKNOWN" : "REPLAYED");
    assert.equal((await call(h.port, { ...create, requestId: `${caseId}SecondCreate` })).status,
      caseId === "instance-home-unknown" ? "CONFLICT" : "STALE");
    if (caseId === "instance-home-unknown") {
      assert.equal((await call(h.port, instanceReq("K-INSTANCE", "home-lifecycle", "blindCleanup", "tempA", "0",
        { action: "CLEANUP" }))).status, "CONFLICT");
      continue;
    }
    assert.equal(JSON.stringify(created.result).includes("homeA"), false);
    assert.equal(JSON.stringify(created.result).includes("credential"), false);
    const close = instanceReq("K-INSTANCE", "home-lifecycle", `${caseId}Close`, "tempA", "1",
      { action: "CLOSE" });
    const closed = await call(h.port, close);
    assert.equal(closed.status, caseId === "instance-home-no-stop" ? "DENIED" : "APPLIED");
    if (caseId === "instance-home-no-stop") continue;
    assert.equal((await call(h.port, instanceReq("K-INSTANCE", "home-lifecycle", `${caseId}EarlyCleanup`,
      "tempA", "1", { action: "CLEANUP" }))).status, "STALE");
    const cleaned = await call(h.port, instanceReq("K-INSTANCE", "home-lifecycle", `${caseId}Cleanup`,
      "tempA", "2", { action: "CLEANUP" }));
    assert.equal(cleaned.status, caseId === "instance-home-busy" ? "DENIED" : "APPLIED");
    if (caseId === "instance-home") {
      assert.equal((await call(h.reconstruct(), instanceReq("K-INSTANCE", "home-lifecycle", "afterCleanup",
        "tempA", "3", { action: "CLEANUP" }))).status, "CONFLICT");
      assert.equal((await call(h.reconstruct(), close)).status, "REPLAYED");
    }
  }
}
