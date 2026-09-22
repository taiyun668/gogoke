//! S1 native Decision commit on the one Product Authority transaction.
//! Capacity lease + canonical Decision object/receipt are atomic. No provider I/O
//! or dispatch occurs here. Live Jev/GENERATIVE are deliberately not qualified.
use super::decision_capacity::{reserve_decision_capacity_in_transaction, DecisionCapacityDisposition, DecisionCapacityRequest};
use super::decision_replay::{read_in_transaction, DurableDecisionRecord, DurableDecisionReplay};
use super::model::{denied, identifier, revision};
use super::transaction::{self, Result, Transaction};
use super::super::atomic::DomainRecordInput;
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) enum DecisionCommitDisposition { Committed, Replayed }

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct DecisionCommitReceipt {
 pub disposition: DecisionCommitDisposition,
 pub replay: DurableDecisionReplay,
}

#[derive(Clone,Debug)]
pub(crate) struct DecisionCommitInput {
 pub domain_id:String,pub decision_id:String,pub event_id:String,pub receipt_id:String,pub recorded_at:String,
 pub record:DurableDecisionRecord,pub resource_reservation_ref:String,pub action_intent_ref:String,
 pub required_capacity_units:u64,
}

fn json_string(value:&str)->String{
 let mut out=String::new();
 out.push('"');
 for ch in value.chars(){match ch{
  '"'=>{out.push('\\');out.push('"');},'\\'=>out.push_str("\\\\"),'\u{0008}'=>out.push_str("\\b"),
  '\u{000c}'=>out.push_str("\\f"),'\n'=>out.push_str("\\n"),'\r'=>out.push_str("\\r"),'\t'=>out.push_str("\\t"),
  ch if (ch as u32)<0x20=>out.push_str(&format!("\\u{:04x}",ch as u32)),ch=>out.push(ch)}}
 out.push('"');out
}
fn optional(value:&Option<String>)->String{value.as_ref().map(|v|json_string(v)).unwrap_or_else(||"null".into())}
fn mode(record:&DurableDecisionRecord)->Result<(&'static str,&'static str)>{
 match record.scenario_id.as_str(){
  "DF02" if record.family=="RESOURCE_SELECTION"=>Ok(("fixture_bounded_auto","RESOURCE_SELECTION")),
  "DF09" if record.family=="SESSION_LIFECYCLE"=>Ok(("fixture_bounded_auto","SESSION_LIFECYCLE")),
  "DF10" if record.family=="CONTEXT_SELECTION"=>Ok(("fixture_bounded_auto","CONTEXT_SELECTION")),
  _=>denied(),
 }
}
fn validate(input:&DecisionCommitInput)->Result<()>{
 for value in [&input.domain_id,&input.decision_id,&input.event_id,&input.receipt_id,&input.record.operation_id,
  &input.record.scenario_id,&input.record.choice,&input.resource_reservation_ref,&input.action_intent_ref]{identifier(value)?;}
 for value in [&input.record.task_revision,&input.record.policy_revision,&input.record.capability_revision,&input.record.binding_generation]{revision(value)?;}
 if input.record.budget_units>9_007_199_254_740_991||input.record.deadline_epoch_ms>9_007_199_254_740_991{return denied();}
 if !matches!(input.record.backend_kind.as_str(),"RULES"|"FAKE"|"REPLAY"){return denied();}
 if input.record.reason!="QUALIFIED_BOUNDED_SELECTION"{return denied();}
 mode(&input.record)?;
 if input.record.state_view_hash.is_empty()||input.record.candidate_hash.is_empty()
  ||input.record.question_version.is_empty()||input.record.rubric_version.is_empty()
  ||input.record.state_view_hash.contains('\0')||input.record.candidate_hash.contains('\0'){return denied();}
 Ok(())
}
fn public_bytes(input:&DecisionCommitInput)->Result<Vec<u8>>{
 let (mode,_)=mode(&input.record)?;
 let requested=input.record.model_requested.as_deref().unwrap_or("NONE");
 let resolved=input.record.model_resolved.as_deref().unwrap_or("NONE");
 let value=format!(
  "{{\"actionId\":{},\"backend\":{},\"calibrationRef\":\"NONE\",\"candidateHash\":{},\"decisionId\":{},\"family\":{},\"mode\":{},\"modelRequested\":{},\"modelResolved\":{},\"nativeConfidence\":null,\"probabilities\":{{}},\"questionVersion\":{},\"sourceRevisions\":{{\"bindingGeneration\":{},\"capabilityRevision\":{},\"policyRevision\":{},\"taskRevision\":{}}},\"state\":\"COMMITTED\",\"stateViewHash\":{}}}",
  json_string(&input.action_intent_ref),json_string(&input.record.backend_kind),json_string(&input.record.candidate_hash),
  json_string(&input.decision_id),json_string(&input.record.family),json_string(mode),json_string(requested),json_string(resolved),
  json_string(&input.record.question_version),json_string(&input.record.binding_generation),json_string(&input.record.capability_revision),
  json_string(&input.record.policy_revision),json_string(&input.record.task_revision),json_string(&input.record.state_view_hash));
 Ok(value.into_bytes())
}
pub(crate) fn durable_record_json(record:&DurableDecisionRecord)->String{
 format!(
  "{{\"backendKind\":{},\"bindingGeneration\":{},\"budgetUnits\":{},\"candidateHash\":{},\"capabilityRevision\":{},\"choice\":{},\"deadlineEpochMs\":{},\"family\":{},\"modelRequested\":{},\"modelResolved\":{},\"operationId\":{},\"policyRevision\":{},\"questionVersion\":{},\"reason\":{},\"rubricVersion\":{},\"scenarioId\":{},\"state\":\"COMMITTED\",\"stateViewHash\":{},\"taskRevision\":{}}}",
  json_string(&record.backend_kind),json_string(&record.binding_generation),json_string(&record.budget_units.to_string()),json_string(&record.candidate_hash),
  json_string(&record.capability_revision),json_string(&record.choice),json_string(&record.deadline_epoch_ms.to_string()),json_string(&record.family),
  optional(&record.model_requested),optional(&record.model_resolved),json_string(&record.operation_id),json_string(&record.policy_revision),
  json_string(&record.question_version),json_string(&record.reason),json_string(&record.rubric_version),json_string(&record.scenario_id),
  json_string(&record.state_view_hash),json_string(&record.task_revision))
}
fn receipt_bytes(input:&DecisionCommitInput,decision_hash:&str)->Vec<u8>{
 format!("{{\"actionIntentRef\":{},\"candidateId\":{},\"decisionContentHash\":{},\"engineRecord\":{},\"resourceReservationRef\":{},\"schema\":\"gogoke.decision-commit-receipt.v1\"}}",
  json_string(&input.action_intent_ref),json_string(&input.record.choice),json_string(decision_hash),durable_record_json(&input.record),
  json_string(&input.resource_reservation_ref)).into_bytes()
}
fn capacity_request(input:&DecisionCommitInput)->DecisionCapacityRequest{
 DecisionCapacityRequest{operation_id:input.record.operation_id.clone(),candidate_id:input.record.choice.clone(),
  state_view_hash:input.record.state_view_hash.clone(),candidate_hash:input.record.candidate_hash.clone(),
  task_revision:input.record.task_revision.clone(),policy_revision:input.record.policy_revision.clone(),
  capability_revision:input.record.capability_revision.clone(),binding_generation:input.record.binding_generation.clone(),
  resource_reservation_ref:input.resource_reservation_ref.clone(),required_units:input.required_capacity_units,
  action_operation_id:input.action_intent_ref.clone()}
}
fn apply(tx:&mut Transaction<'_, '_>,input:&DecisionCommitInput)->Result<DecisionCommitReceipt>{
 validate(input)?;
 let capacity=reserve_decision_capacity_in_transaction(tx,&capacity_request(input))?;
 if capacity==DecisionCapacityDisposition::Replay{
  let prior=read_in_transaction(tx,&input.domain_id,&input.record.operation_id)?;
  if prior.record!=input.record||prior.decision_id!=input.decision_id||prior.resource_reservation_ref!=input.resource_reservation_ref
   ||prior.action_intent_ref!=input.action_intent_ref{return Err(OrchestrationError::OperationConflict);}
  return Ok(DecisionCommitReceipt { disposition: DecisionCommitDisposition::Replayed, replay: prior });
 }
 let preexisting=tx.query("SELECT receipt_id FROM gogoke_receipts WHERE domain_id=? AND operation_id=?",
  &[&input.domain_id,&input.record.operation_id],1)?;
 if !preexisting.is_empty(){return Err(OrchestrationError::OperationConflict);}
 let public=public_bytes(input)?;let decision_hash=content_hash(&public);
 let event=format!("{{\"actionId\":{},\"candidateId\":{},\"decisionId\":{},\"resourceReservationRef\":{}}}",
  json_string(&input.action_intent_ref),json_string(&input.record.choice),json_string(&input.decision_id),json_string(&input.resource_reservation_ref));
 let receipt=receipt_bytes(input,&decision_hash);
 tx.apply_domain_record(DomainRecordInput{domain_id:input.domain_id.clone(),object_type:"DecisionRecord".into(),object_id:input.decision_id.clone(),
  object_version:"1".into(),object_bytes:public,native_identity:None,event_id:input.event_id.clone(),
  stream_id:format!("gogoke.decision.v1/{}",input.decision_id),expected_previous_counter:None,counter:"0".into(),
  event_type:"DecisionApplied".into(),occurred_at:input.recorded_at.clone(),event_bytes:event.into_bytes(),
  receipt_id:input.receipt_id.clone(),operation_id:input.record.operation_id.clone(),receipt_type:"DecisionApplied".into(),
  recorded_at:input.recorded_at.clone(),receipt_bytes:receipt})?;
 let replay=read_in_transaction(tx,&input.domain_id,&input.record.operation_id)?;
 Ok(DecisionCommitReceipt { disposition: DecisionCommitDisposition::Committed, replay })
}
pub(crate) fn commit_decision(
 connection:&mut VerifiedDatabaseConnection<'_>,input:&DecisionCommitInput,
)->Result<DecisionCommitReceipt>{transaction::run(connection,|tx|apply(tx,input))}

#[cfg(test)]
#[path = "decision_encoding_tests.rs"]
mod encoding_tests;
