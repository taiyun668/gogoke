//! Native durable Decision replay definitions; zero execution is not PASS.
use super::decision_replay::read_durable_decision_replay;
use crate::root::RootLock;
use crate::store::atomic::{apply_core_schema,commit_domain_record,DomainRecordInput,Statement};
use crate::store::digest::content_hash;
use crate::store::same_open::{create_new,route_b_test_guard};
use std::time::{SystemTime,UNIX_EPOCH};

const ACTION:&str="opr_11111111111111111111111111111111";
fn public_decision()->Vec<u8>{
    format!("{{\"actionId\":\"{ACTION}\",\"backend\":\"FAKE\",\"calibrationRef\":\"NONE\",\"candidateHash\":\"candidates-one\",\"decisionId\":\"decision-one\",\"family\":\"RESOURCE_SELECTION\",\"mode\":\"fixture_bounded_auto\",\"modelRequested\":\"NONE\",\"modelResolved\":\"fake-v1\",\"nativeConfidence\":null,\"probabilities\":{{}},\"questionVersion\":\"1\",\"sourceRevisions\":{{\"bindingGeneration\":\"1\",\"capabilityRevision\":\"1\",\"policyRevision\":\"1\",\"taskRevision\":\"1\"}},\"state\":\"COMMITTED\",\"stateViewHash\":\"view-one\"}}").into_bytes()
}
fn receipt(candidate:&str,action:&str)->Vec<u8>{
    format!("{{\"actionIntentRef\":\"{action}\",\"candidateId\":\"{candidate}\",\"decisionContentHash\":\"{}\",\"engineRecord\":{{\"backendKind\":\"FAKE\",\"bindingGeneration\":\"1\",\"budgetUnits\":\"1\",\"candidateHash\":\"candidates-one\",\"capabilityRevision\":\"1\",\"choice\":\"{candidate}\",\"deadlineEpochMs\":\"1000\",\"family\":\"RESOURCE_SELECTION\",\"modelRequested\":null,\"modelResolved\":\"fake-v1\",\"operationId\":\"operation-one\",\"policyRevision\":\"1\",\"questionVersion\":\"1\",\"reason\":\"QUALIFIED_BOUNDED_SELECTION\",\"rubricVersion\":\"1\",\"scenarioId\":\"DF02\",\"state\":\"COMMITTED\",\"stateViewHash\":\"view-one\",\"taskRevision\":\"1\"}},\"resourceReservationRef\":\"capacity-one\",\"schema\":\"gogoke.decision-commit-receipt.v1\"}}",
        content_hash(&public_decision())).into_bytes()
}
fn fixture(run:impl FnOnce(&mut crate::store::same_open::VerifiedDatabaseConnection<'_>)){
    let _guard=route_b_test_guard();let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path=std::env::temp_dir().join(format!("gogoke-decision-replay-{}-{nonce}",std::process::id()));
    std::fs::create_dir(&path).unwrap();let root=RootLock::acquire(&path).unwrap();let database=path.join("state.sqlite");
    let mut db=create_new(&root,&database).unwrap();apply_core_schema(&mut db).unwrap();run(&mut db);
    db.close_checked().unwrap();drop(root);std::fs::remove_file(database).unwrap();
    if let Err(e)=std::fs::remove_dir(path){eprintln!("owned fixture retained:{e}");}
}
fn seed(db:&mut crate::store::same_open::VerifiedDatabaseConnection<'_>, candidate:&str, action:&str){
    let public=public_decision();
    commit_domain_record(db,DomainRecordInput{domain_id:"domain-one".into(),object_type:"DecisionRecord".into(),
        object_id:"decision-one".into(),object_version:"1".into(),object_bytes:public,native_identity:None,
        event_id:"decision-event".into(),stream_id:"decision-stream".into(),expected_previous_counter:None,counter:"0".into(),
        event_type:"DecisionApplied".into(),occurred_at:"2026-09-21T00:00:00Z".into(),event_bytes:b"{}".to_vec(),
        receipt_id:"decision-receipt".into(),operation_id:"operation-one".into(),receipt_type:"DecisionApplied".into(),
        recorded_at:"2026-09-21T00:00:00Z".into(),receipt_bytes:receipt(candidate,action)}).unwrap();
}
fn replace_receipt_and_rehash(db:&mut crate::store::same_open::VerifiedDatabaseConnection<'_>,from:&str,to:&str){
    let select=Statement::prepare(db.as_ptr(),"SELECT CAST(canonical_json AS TEXT) FROM gogoke_receipts WHERE operation_id='operation-one'").unwrap();
    assert!(select.step_row().unwrap());let original=select.column_text(0).unwrap();drop(select);
    let mutated=original.replace(from,to);assert_ne!(mutated,original,"mutation must reach the intended receipt field");
    let hash=content_hash(mutated.as_bytes());
    let update=Statement::prepare(db.as_ptr(),"UPDATE gogoke_receipts SET canonical_json=?,content_hash=? WHERE operation_id='operation-one'").unwrap();
    update.bind_blob(1,mutated.as_bytes()).unwrap();update.bind_text(2,&hash).unwrap();update.step_done().unwrap();
}
#[test]
fn replay_returns_prior_durable_choice_and_engine_record(){
    fixture(|db|{seed(db,"candidate-old",ACTION);let r=read_durable_decision_replay(db,"domain-one","operation-one").unwrap();
        assert_eq!(r.record.choice,"candidate-old");assert_eq!(r.record.scenario_id,"DF02");
        assert_eq!(r.resource_reservation_ref,"capacity-one");assert_eq!(r.action_intent_ref,ACTION);});
}
#[test]
fn replay_is_domain_and_operation_qualified(){
    fixture(|db|{seed(db,"candidate-old",ACTION);
        assert!(read_durable_decision_replay(db,"domain-other","operation-one").is_err());
        assert!(read_durable_decision_replay(db,"domain-one","operation-other").is_err());});
}
#[test]
fn public_source_revision_mismatch_fails_closed(){
    fixture(|db|{seed(db,"candidate-old",ACTION);
        db.execute("UPDATE gogoke_objects SET canonical_json=CAST(replace(CAST(canonical_json AS TEXT),'\"taskRevision\":\"1\"','\"taskRevision\":\"2\"') AS BLOB) WHERE object_type='DecisionRecord'").unwrap();
        assert!(read_durable_decision_replay(db,"domain-one","operation-one").is_err());});
}
#[test]
fn durable_receipt_choice_cannot_disagree_with_engine_choice(){
    fixture(|db|{seed(db,"candidate-old",ACTION);
        db.execute("UPDATE gogoke_receipts SET canonical_json=CAST(replace(CAST(canonical_json AS TEXT),'\"candidateId\":\"candidate-old\"','\"candidateId\":\"candidate-other\"') AS BLOB)").unwrap();
        assert!(read_durable_decision_replay(db,"domain-one","operation-one").is_err());});
}
#[test]
fn public_action_must_match_durable_action_intent(){
    fixture(|db|{seed(db,"candidate-old",ACTION);
        let forged="opr_22222222222222222222222222222222";
        db.execute(&format!("UPDATE gogoke_objects SET canonical_json=CAST(replace(CAST(canonical_json AS TEXT),'{ACTION}','{forged}') AS BLOB) WHERE object_type='DecisionRecord'")).unwrap();
        assert!(read_durable_decision_replay(db,"domain-one","operation-one").is_err());});
}
#[test]
fn malformed_private_engine_record_is_not_replayed(){
    fixture(|db|{seed(db,"candidate-old",ACTION);
        replace_receipt_and_rehash(db,"\"budgetUnits\":\"1\"","\"budgetUnits\":\"-1\"");
        assert!(read_durable_decision_replay(db,"domain-one","operation-one").is_err());});
}
