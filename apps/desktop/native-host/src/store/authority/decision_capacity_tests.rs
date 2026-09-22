//! Native Product Authority capacity facts; definitions only until controlled native execution.
use super::decision_capacity::*;
use crate::root::RootLock;
use crate::store::action::{apply_action_schema,reserve_action,ActionReservation};
use crate::store::authority::transaction;
use crate::store::same_open::{create_new,route_b_test_guard};
use std::time::{SystemTime,UNIX_EPOCH};

fn fixture(run:impl FnOnce(&mut crate::store::same_open::VerifiedDatabaseConnection<'_>)){
 let _guard=route_b_test_guard();let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
 let path=std::env::temp_dir().join(format!("gogoke-decision-capacity-{}-{nonce}",std::process::id()));
 std::fs::create_dir(&path).unwrap();let root=RootLock::acquire(&path).unwrap();let database=path.join("state.sqlite");
 let mut db=create_new(&root,&database).unwrap();apply_action_schema(&mut db).unwrap();
 crate::store::authority::initialize_profile(&mut db,&root).unwrap();initialize_decision_capacity_schema(&mut db).unwrap();run(&mut db);
 db.close_checked().unwrap();drop(root);std::fs::remove_file(database).unwrap();if let Err(e)=std::fs::remove_dir(path){eprintln!("owned:{e}");}
}
fn digest(c:char)->String{format!("sha256:{}",c.to_string().repeat(64))}
fn snap()->DecisionAuthoritySnapshot{DecisionAuthoritySnapshot{operation_id:"decision-op".into(),candidate_id:"candidate-one".into(),
 state_view_hash:digest('a'),candidate_hash:digest('b'),task_revision:"1".into(),policy_revision:"1".into(),capability_revision:"3".into(),
 binding_id:"binding-one".into(),binding_generation:"7".into(),auth_revision:"2".into(),resource_ref:"pool-one".into(),
 resource_revision:"5".into(),capacity_total:2,action_operation_id:"opr_11111111111111111111111111111111".into(),action_digest:digest('c')}}
fn action(db:&mut crate::store::same_open::VerifiedDatabaseConnection<'_>){let s=snap();reserve_action(db,ActionReservation{
 operation_id:s.action_operation_id,semantic_digest:s.action_digest,reservation_id:"action-reservation".into(),binding_id:s.binding_id,
 session_id:"session-one".into(),execution_id:"execution-one".into(),runtime_instance_id:"runtime-one".into(),profile_id:"profile-one".into(),auth_revision:s.auth_revision,
 generation:s.binding_generation,lane:"work".into(),action_kind:"queue".into(),payload_hex:"7b7d".into(),commitment:crate::store::action::test_commitment("session-one","execution-one","7")}).unwrap();}
fn req()->DecisionCapacityRequest{let s=snap();DecisionCapacityRequest{operation_id:s.operation_id,candidate_id:s.candidate_id,
 state_view_hash:s.state_view_hash,candidate_hash:s.candidate_hash,task_revision:s.task_revision,policy_revision:s.policy_revision,
 capability_revision:s.capability_revision,binding_generation:s.binding_generation,resource_reservation_ref:"capacity-lease-one".into(),
 required_units:1,action_operation_id:s.action_operation_id}}
#[test]fn exact_snapshot_reserves_capacity_once_and_replays(){fixture(|db|{action(db);publish_decision_snapshot(db,&snap()).unwrap();
 transaction::run(db,|tx|{assert_eq!(reserve_decision_capacity_in_transaction(tx,&req())?,DecisionCapacityDisposition::Reserved);Ok(())}).unwrap();
 transaction::run(db,|tx|{assert_eq!(reserve_decision_capacity_in_transaction(tx,&req())?,DecisionCapacityDisposition::Replay);Ok(())}).unwrap();});}
#[test]fn stale_policy_candidate_capability_or_binding_fails_closed(){fixture(|db|{action(db);publish_decision_snapshot(db,&snap()).unwrap();
 let changes:[fn(&mut DecisionCapacityRequest);4]=[|x|x.policy_revision="2".into(),|x|x.candidate_hash=digest('d'),
  |x|x.capability_revision="4".into(),|x|x.binding_generation="8".into()];
 for change in changes{let mut r=req();change(&mut r);assert!(transaction::run(db,|tx|reserve_decision_capacity_in_transaction(tx,&r)).is_err());}});}
#[test]fn decision_capacity_precedes_action_prepare_without_creating_action_state(){fixture(|db|{publish_decision_snapshot(db,&snap()).unwrap();
 assert_eq!(transaction::run(db,|tx|reserve_decision_capacity_in_transaction(tx,&req())).unwrap(),DecisionCapacityDisposition::Reserved);
 let rows=transaction::run(db,|tx|tx.query("SELECT count(*) FROM main.gogoke_action_reservations",&[],1)).unwrap();
 assert_eq!(rows[0][0],"0");});}
#[test]fn capacity_exhaustion_does_not_create_second_lease(){fixture(|db|{action(db);let mut s=snap();s.capacity_total=0;publish_decision_snapshot(db,&s).unwrap();
 assert!(transaction::run(db,|tx|reserve_decision_capacity_in_transaction(tx,&req())).is_err());});}
#[test]fn conflicting_replay_identity_does_not_consume_more_capacity(){fixture(|db|{action(db);publish_decision_snapshot(db,&snap()).unwrap();
 transaction::run(db,|tx|reserve_decision_capacity_in_transaction(tx,&req()).map(|_|())).unwrap();let mut other=req();other.resource_reservation_ref="capacity-other".into();
 assert!(transaction::run(db,|tx|reserve_decision_capacity_in_transaction(tx,&other)).is_err());});}
#[test]fn stale_snapshot_cannot_be_overwritten_under_same_operation_candidate(){fixture(|db|{publish_decision_snapshot(db,&snap()).unwrap();
 let mut changed=snap();changed.binding_generation="8".into();assert!(publish_decision_snapshot(db,&changed).is_err());});}
#[test]fn pool_revision_can_advance_without_erasing_existing_reserved_units(){fixture(|db|{action(db);publish_decision_snapshot(db,&snap()).unwrap();
 transaction::run(db,|tx|reserve_decision_capacity_in_transaction(tx,&req()).map(|_|())).unwrap();
 let mut next=snap();next.operation_id="decision-op-two".into();next.resource_revision="6".into();next.capacity_total=3;
 publish_decision_snapshot(db,&next).unwrap();
 let rows=transaction::run(db,|tx|tx.query("SELECT revision,CAST(reserved_units AS TEXT) FROM gogoke_decision_capacity_pools WHERE resource_ref='pool-one'",&[],2)).unwrap();
 assert_eq!(rows[0],vec!["6","1"]);});}
#[test]fn pool_revision_cannot_shrink_below_reserved_units(){fixture(|db|{action(db);publish_decision_snapshot(db,&snap()).unwrap();
 transaction::run(db,|tx|reserve_decision_capacity_in_transaction(tx,&req()).map(|_|())).unwrap();
 let mut next=snap();next.operation_id="decision-op-two".into();next.resource_revision="6".into();next.capacity_total=0;
 assert!(publish_decision_snapshot(db,&next).is_err());});}
