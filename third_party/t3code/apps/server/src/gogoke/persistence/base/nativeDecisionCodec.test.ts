import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";
import {
  decodeDecisionCommitReply, encodeDecisionCommitFrame, encodeDecisionReplayFrame,
  encodeDecisionSnapshotFrame, NativeHostClientError,
  type NativeDecisionCommitRequest,
} from "./nativeHostClient.ts";

const commit = (): NativeDecisionCommitRequest => ({
  domainId:"domain-one", decisionId:"decision-one", eventId:"event-one", receiptId:"receipt-one",
  recordedAt:"2026-09-21T00:00:00Z", resourceReservationRef:"capacity-one",
  actionIntentRef:"opr_11111111111111111111111111111111", requiredCapacityUnits:1,
  record:{operationId:"decision-op",scenarioId:"DF02",family:"RESOURCE_SELECTION",state:"COMMITTED",
    stateViewHash:"sha256:"+"a".repeat(64),candidateHash:"sha256:"+"b".repeat(64),questionVersion:"1",
    rubricVersion:"1",modelRequested:null,modelResolved:"fake-v1",taskRevision:"1",policyRevision:"1",
    capabilityRevision:"3",bindingGeneration:"7",backendKind:"FAKE",choice:"candidate-one",
    reason:"QUALIFIED_BOUNDED_SELECTION",budgetUnits:1,deadlineEpochMs:1000}
});

describe("native Decision typed codec",()=>{
  it("encodes an exact scalar current-fact snapshot",()=>{
    const frame=JSON.parse(encodeDecisionSnapshotFrame({operationId:"decision-op",candidateId:"candidate-one",
      stateViewHash:"sha256:"+"a".repeat(64),candidateHash:"sha256:"+"b".repeat(64),taskRevision:"1",
      policyRevision:"1",capabilityRevision:"3",bindingId:"binding-one",bindingGeneration:"7",authRevision:"2",
      resourceRef:"pool-one",resourceRevision:"5",capacityTotal:2,
      actionOperationId:"opr_11111111111111111111111111111111",actionDigest:"sha256:"+"c".repeat(64)}));
    Assert.deepEqual(Object.keys(frame).sort(),["actionDigest","actionOperationId","authRevision","bindingGeneration",
      "bindingId","candidateHash","candidateId","capabilityRevision","capacityTotal","operation","operationId",
      "policyRevision","resourceRef","resourceRevision","stateViewHash","taskRevision"].sort());
    Assert.equal(frame.operation,"PublishDecisionSnapshot"); Assert.equal(frame.capacityTotal,"2");
  });

  it("encodes null models as explicit empty transport values, not missing fields",()=>{
    const frame=JSON.parse(encodeDecisionCommitFrame(commit()));
    Assert.equal(frame.operation,"CommitDecision"); Assert.equal(frame.modelRequested,"");
    Assert.equal(frame.modelResolved,"fake-v1"); Assert.equal(frame.requiredCapacityUnits,"1");
    Assert.equal(Object.hasOwn(frame,"actionIntentRef"),true);
  });

  it("rejects unsafe numeric values before native transport",()=>{
    const value=commit();
    Assert.throws(()=>encodeDecisionCommitFrame({...value,requiredCapacityUnits:Number.MAX_SAFE_INTEGER+1}),NativeHostClientError);
    Assert.throws(()=>encodeDecisionSnapshotFrame({operationId:"o",candidateId:"c",stateViewHash:"h",candidateHash:"c",
      taskRevision:"1",policyRevision:"1",capabilityRevision:"1",bindingId:"b",bindingGeneration:"1",authRevision:"1",
      resourceRef:"r",resourceRevision:"1",capacityTotal:-1,actionOperationId:"a",actionDigest:"d"}),NativeHostClientError);
  });

  it("decodes committed and durable replay replies without rewriting history",()=>{
    const committed=decodeDecisionCommitReply('{"kind":"committed","operationId":"decision-op","decisionReceiptId":"receipt-one"}');
    Assert.equal(committed.kind,"committed");
    const durable={...commit().record,budgetUnits:"1",deadlineEpochMs:"1000"};
    const replay=decodeDecisionCommitReply(JSON.stringify({kind:"replayed",operationId:"decision-op",decisionReceiptId:"receipt-one",record:durable}));
    Assert.equal(replay.kind,"replayed");
    if(replay.kind==="replayed"){Assert.equal(replay.record.choice,"candidate-one");Assert.equal(replay.record.budgetUnits,1);Assert.equal(replay.record.deadlineEpochMs,1000);Assert.equal(Object.isFrozen(replay.record),true);}
  });

  it("rejects replay record mismatch, missing fields and extra aliases",()=>{
    const record={...commit().record,budgetUnits:"1",deadlineEpochMs:"1000"};
    Assert.throws(()=>decodeDecisionCommitReply(JSON.stringify({kind:"replayed",operationId:"other",decisionReceiptId:"r",record})),NativeHostClientError);
    Assert.throws(()=>decodeDecisionCommitReply(JSON.stringify({kind:"replayed",operationId:"decision-op",decisionReceiptId:"r",record:{...record,choice:null}})),NativeHostClientError);
    Assert.throws(()=>decodeDecisionCommitReply(JSON.stringify({kind:"replayed",operationId:"decision-op",decisionReceiptId:"r",record:{...record,budgetUnits:1}})),NativeHostClientError);
    Assert.throws(()=>decodeDecisionCommitReply(JSON.stringify({kind:"replayed",operationId:"decision-op",decisionReceiptId:"r",record:{...record,deadlineEpochMs:"01"}})),NativeHostClientError);
    Assert.throws(()=>decodeDecisionCommitReply(JSON.stringify({kind:"committed",operationId:"decision-op",decisionReceiptId:"r",extra:true})),NativeHostClientError);
  });

  it("encodes durable replay reads as domain-qualified requests",()=>{
    Assert.deepEqual(JSON.parse(encodeDecisionReplayFrame("domain-one","decision-op")),
      {domainId:"domain-one",operation:"ReadDecisionReplay",operationId:"decision-op"});
  });
});
