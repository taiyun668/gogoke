//! Native Decision commit definitions; not execution evidence.
use super::decision_capacity::{publish_decision_snapshot,DecisionAuthoritySnapshot};
use super::decision_commit::{commit_decision,DecisionCommitDisposition,DecisionCommitInput};
use super::decision_replay::DurableDecisionRecord;
use crate::root::RootLock;
use crate::store::action::{apply_action_schema,reserve_action,record_action_outcome,ActionReservation};
use crate::store::atomic::apply_core_schema;
use crate::store::digest::content_hash;
use crate::store::same_open::{create_new,route_b_test_guard};
use std::time::{SystemTime,UNIX_EPOCH};

fn digest(c:char)->String{format!("sha256:{}",c.to_string().repeat(64))}
fn record()->DurableDecisionRecord{DurableDecisionRecord{operation_id:"decision-op".into(),scenario_id:"DF02".into(),family:"RESOURCE_SELECTION".into(),
 state_view_hash:digest('a'),candidate_hash:digest('b'),question_version:"1".into(),rubric_version:"1".into(),model_requested:None,
 model_resolved:Some("fake-v1".into()),task_revision:"1".into(),policy_revision:"1".into(),capability_revision:"3".into(),
 binding_generation:"7".into(),backend_kind:"FAKE".into(),choice:"candidate-one".into(),reason:"QUALIFIED_BOUNDED_SELECTION".into(),
 budget_units:1,deadline_epoch_ms:1000}}
fn input()->DecisionCommitInput{DecisionCommitInput{domain_id:"domain-one".into(),decision_id:"decision-one".into(),
 event_id:"decision-event".into(),receipt_id:"decision-receipt".into(),recorded_at:"2026-09-21T00:00:00Z".into(),
 record:record(),resource_reservation_ref:"capacity-lease-one".into(),action_intent_ref:"opr_11111111111111111111111111111111".into(),
 required_capacity_units:1}}
fn snapshot()->DecisionAuthoritySnapshot{let i=input();DecisionAuthoritySnapshot{operation_id:i.record.operation_id,candidate_id:i.record.choice,
 state_view_hash:i.record.state_view_hash,candidate_hash:i.record.candidate_hash,task_revision:i.record.task_revision,
 policy_revision:i.record.policy_revision,capability_revision:i.record.capability_revision,binding_id:"binding-one".into(),
 binding_generation:i.record.binding_generation,auth_revision:"2".into(),resource_ref:"pool-one".into(),resource_revision:"5".into(),
 capacity_total:2,action_operation_id:i.action_intent_ref,action_digest:digest('c')}}
fn fixture(run:impl FnOnce(&mut crate::store::same_open::VerifiedDatabaseConnection<'_>)){
 let _g=route_b_test_guard();let n=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
 let p=std::env::temp_dir().join(format!("gogoke-decision-commit-{}-{n}",std::process::id()));std::fs::create_dir(&p).unwrap();
 let root=RootLock::acquire(&p).unwrap();let dbp=p.join("state.sqlite");let mut db=create_new(&root,&dbp).unwrap();
 apply_core_schema(&mut db).unwrap();apply_action_schema(&mut db).unwrap();crate::store::authority::initialize_profile(&mut db,&root).unwrap();
 crate::store::authority::initialize_decision_capacity_schema(&mut db).unwrap();
 let s=snapshot();reserve_action(&mut db,ActionReservation{operation_id:s.action_operation_id.clone(),semantic_digest:s.action_digest.clone(),
  reservation_id:"action-reservation".into(),binding_id:s.binding_id.clone(),session_id:"session-one".into(),execution_id:"execution-one".into(),runtime_instance_id:"runtime-one".into(),
  profile_id:"profile-one".into(),auth_revision:s.auth_revision.clone(),generation:s.binding_generation.clone(),lane:"work".into(),
  action_kind:"queue".into(),payload_hex:"7b7d".into(),commitment:crate::store::action::test_commitment("session-one","execution-one","7")}).unwrap();publish_decision_snapshot(&mut db,&s).unwrap();run(&mut db);
 db.close_checked().unwrap();drop(root);std::fs::remove_file(dbp).unwrap();if let Err(e)=std::fs::remove_dir(p){eprintln!("owned:{e}");}
}
#[test]fn commit_is_atomic_with_capacity_and_returns_durable_record(){fixture(|db|{let r=commit_decision(db,&input()).unwrap();
 assert_eq!(r.disposition,DecisionCommitDisposition::Committed);
 assert_eq!(r.replay.record,record());assert_eq!(r.replay.resource_reservation_ref,"capacity-lease-one");});}
#[test]fn replay_returns_same_durable_decision_after_action_progress(){fixture(|db|{let first=commit_decision(db,&input()).unwrap();
 assert_eq!(first.disposition,DecisionCommitDisposition::Committed);
 record_action_outcome(db,"action-reservation","opr_11111111111111111111111111111111",&digest('c'),"dispatched","","receipt-native").unwrap();
 let replay=commit_decision(db,&input()).unwrap();
 assert_eq!(replay.disposition,DecisionCommitDisposition::Replayed);
 assert_eq!(replay.replay.operation_fingerprint,first.replay.operation_fingerprint);
 assert_eq!(replay.replay.record,first.replay.record);});}
#[test]fn changed_choice_or_resource_ref_conflicts_without_second_decision(){fixture(|db|{commit_decision(db,&input()).unwrap();
 let mut changed=input();changed.record.choice="candidate-other".into();assert!(commit_decision(db,&changed).is_err());
 let mut changed=input();changed.resource_reservation_ref="capacity-other".into();assert!(commit_decision(db,&changed).is_err());});}
#[test]fn stale_basis_fails_before_public_decision_write(){fixture(|db|{let mut changed=input();changed.record.capability_revision="4".into();
 assert!(commit_decision(db,&changed).is_err());});}
#[test]fn unqualified_scenario_or_backend_never_commits(){fixture(|db|{for mutate in [0,1]{
 let mut x=input();if mutate==0{x.record.scenario_id="DF15".into();}else{x.record.backend_kind="JEV".into();}
 assert!(commit_decision(db,&x).is_err());}});}
#[test]fn public_decision_uses_frozen_contract_shape_and_private_receipt_keeps_engine_metadata(){fixture(|db|{let r=commit_decision(db,&input()).unwrap();
 let rows=crate::store::authority::transaction::run(db,|tx|tx.query("SELECT CAST(o.canonical_json AS TEXT),CAST(r.canonical_json AS TEXT) FROM gogoke_objects o JOIN gogoke_receipts r ON r.domain_id=o.domain_id AND r.object_type=o.object_type AND r.object_id=o.object_id AND r.object_version=o.object_version WHERE o.object_type='DecisionRecord'",&[],2)).unwrap();
 assert_eq!(rows.len(),1);assert!(rows[0][0].contains("\"sourceRevisions\""));assert!(rows[0][0].contains("\"probabilities\":{}"));
 assert!(rows[0][1].contains("\"scenarioId\":\"DF02\""));
 assert!(rows[0][1].contains("\"budgetUnits\":\"1\""));assert!(rows[0][1].contains("\"deadlineEpochMs\":\"1000\""));
 assert_eq!(content_hash(rows[0][0].as_bytes()),r.replay.decision_content_hash);});}
