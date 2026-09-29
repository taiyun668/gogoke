import * as assert from "node:assert/strict";
import type { V37TakeoverContext } from "./m1Fake.ts";
import { decodeV37Receipt, encodeV37Request, V37_SCHEMA, type V37Port, type V37Receipt, type V37Request } from "./protocol.ts";

export type V37M1Case = "card-fallback" | "card-native" | "seat-user" | "seat-busy" |
  "seat-lead" | "seat-revoked" | "seat-takeover" | "seat-takeover-unwired" |
  "instance" | "instance-unverified";
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
      { layer: "LEAD", templateId: "templateA" });
    assert.equal((await call(h.port, createA)).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), createA)).status, "REPLAYED");
    assert.equal((await call(h.port, req("K-SEAT", "create-from-template", "seatCreateB", "seatB", "0",
      { layer: "LEAD", templateId: "templateA" }))).status, "APPLIED");
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
      { layer: "LEAD", templateId: "templateA" }))).status, "APPLIED");
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
    assert.equal((await call(h.port, req("K-SEAT", "create-from-template", "takeoverLead", "leadA", "0",
      { layer: "USER", templateId: "templateA" }))).status, "APPLIED");
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
    h.revoke();
    assert.equal((await call(h.reconstruct(), req("K-SEAT", "takeover-answers", "revokedAnswers", "leadA", "7",
      answers.payload))).status, "DENIED");
  }
  {
    const h = factory("seat-busy");
    assert.equal((await call(h.port, req("K-SEAT", "create-from-template", "busyCreate", "seatBusy", "0",
      { layer: "LEAD", templateId: "templateA" }))).status, "APPLIED");
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
    assert.equal((await call(h.port, req("K-SEAT", "create-from-template", "childSeat", "childSeat", "0",
      { layer: "LEAD", templateId: "templateA" }))).status, "APPLIED");
  }
  {
    const h = factory("instance-unverified");
    const invalid = await call(h.port, req("K-INSTANCE", "register", "unverified", "instanceA", "0",
      { homeRef: "homeA", programDigest: "verifiedDigest", version: "1" }));
    assert.equal(invalid.status, "DENIED");
    assert.equal(invalid.revision, "0");
  }
  {
    const h = factory("instance");
    const register = req("K-INSTANCE", "register", "registerA", "instanceA", "0",
      { homeRef: "homeA", programDigest: "verifiedDigest", version: "1" });
    assert.equal((await call(h.port, register)).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), register)).status, "REPLAYED");
    assert.equal((await call(h.port, req("K-INSTANCE", "register", "sharedHome", "instanceB", "0",
      { homeRef: "homeA", programDigest: "verifiedDigest", version: "1" }))).status, "DENIED");
    const login = await call(h.port, req("K-INSTANCE", "login-state", "loginRead", "instanceA", "1"));
    assert.equal(login.result.loggedIn, false);
    assert.equal(login.revision, login.previousRevision);
    assert.equal((await call(h.port, req("K-INSTANCE", "install-state", "installRead", "instanceA", "1"))).result.installed,
      true);
    assert.equal((await call(h.port, req("K-INSTANCE", "repin-after-manual-upgrade", "badRepin", "instanceA", "1",
      { programDigest: "unverifiedDigest", version: "2" }))).status, "DENIED");
    assert.equal((await call(h.port, req("K-INSTANCE", "repin-after-manual-upgrade", "repinA", "instanceA", "1",
      { programDigest: "newVerifiedDigest", version: "2" }))).status, "APPLIED");
    const version = await call(h.reconstruct(), req("K-INSTANCE", "version-and-new-version", "versionRead", "instanceA", "2"));
    assert.equal(version.result.programDigest, "newVerifiedDigest");
    assert.equal(version.result.version, "2");
    assert.equal((await call(h.port, req("K-INSTANCE", "concurrency-input", "capacityRead", "instanceA", "2"))).result.capacity,
      "3");
    assert.equal(JSON.stringify(version.result).includes("credential"), false);
  }
}
