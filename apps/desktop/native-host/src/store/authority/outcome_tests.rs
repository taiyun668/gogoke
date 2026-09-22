//! Native Owner Outcome regressions; definitions are not execution evidence.
//! Decision seed is an explicit fixture, not a qualified production Decision.
use super::bootstrap::{initialize_profile, OwnerIssuer};
use super::outcome::{append_owner_override_outcome, apply_owner_override_outcome, OwnerOutcomeAppend, OutcomeVersionRef};
use super::transaction;
use crate::root::RootLock;
use crate::store::action::{apply_action_schema, reserve_action, ActionReservation};
use crate::store::atomic::{apply_core_schema, commit_domain_record, DomainRecordInput, Statement};
use crate::store::digest::content_hash;
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::{create_new, open_existing, route_b_test_guard, VerifiedDatabaseConnection};
use std::time::{SystemTime, UNIX_EPOCH};

const ACTION: &str = "opr_11111111111111111111111111111111";
const DECISION: &[u8] = br#"{"actionId":"opr_11111111111111111111111111111111","backend":"RULES","calibrationRef":"NONE","candidateHash":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","decisionId":"decision-one","family":"RESOURCE_SELECTION","mode":"fixture_bounded_auto","modelRequested":"NONE","modelResolved":"NONE","nativeConfidence":null,"probabilities":{},"questionVersion":"1","sourceRevisions":{"bindingGeneration":"1","capabilityRevision":"1","policyRevision":"1","taskRevision":"1"},"state":"COMMITTED","stateViewHash":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#;
fn document(version: &str, status: &str) -> Vec<u8> {
    format!("{{\"actionId\":\"{ACTION}\",\"censorStatus\":\"{status}\",\"cost\":null,\"decisionId\":\"decision-one\",\"evidenceRefs\":[\"evidence-one\"],\"labelSource\":\"OWNER_OVERRIDE\",\"latency\":null,\"observationWindow\":null,\"outcomeId\":\"outcome-one\",\"quality\":null,\"revision\":\"{version}\",\"rework\":null,\"safetyEvents\":[]}}").into_bytes()
}
fn request(version: &str, previous: Option<OutcomeVersionRef>) -> OwnerOutcomeAppend {
    OwnerOutcomeAppend { domain_id:"domain-one".into(), operation_id:format!("outcome-operation-{version}"),
        event_id:format!("outcome-event-{version}"), receipt_id:format!("outcome-receipt-{version}"),
        recorded_at:"2026-09-21T00:00:00Z".into(), policy_revision:"1".into(), revocation_head:"0".into(),
        decision_version:"1".into(), decision_hash:content_hash(DECISION), action_operation_id:ACTION.into(),
        action_digest:format!("sha256:{}","a".repeat(64)), previous,
        canonical_outcome:document(version,if version=="1" {"PENDING"} else {"CORRECTED"}) }
}
fn seed(connection: &mut VerifiedDatabaseConnection<'_>) {
    reserve_action(connection,ActionReservation { operation_id:ACTION.into(), semantic_digest:format!("sha256:{}","a".repeat(64)),
        reservation_id:"reservation-one".into(),binding_id:"binding-one".into(),session_id:"session-one".into(),execution_id:"execution-one".into(),
        runtime_instance_id:"runtime-one".into(),profile_id:"profile-one".into(),auth_revision:"1".into(),generation:"1".into(),
        lane:"work".into(),action_kind:"queue".into(),payload_hex:"7b7d".into(),commitment:crate::store::action::test_commitment("session-one","execution-one","1") }).unwrap();
    commit_domain_record(connection,DomainRecordInput { domain_id:"domain-one".into(),object_type:"DecisionRecord".into(),
        object_id:"decision-one".into(),object_version:"1".into(),object_bytes:DECISION.to_vec(),native_identity:None,
        event_id:"decision-event".into(),stream_id:"decision-stream".into(),expected_previous_counter:None,counter:"0".into(),
        event_type:"FixtureDecision".into(),occurred_at:"2026-09-21T00:00:00Z".into(),event_bytes:b"{}".to_vec(),
        receipt_id:"decision-receipt".into(),operation_id:"decision-operation".into(),receipt_type:"FixtureDecision".into(),
        recorded_at:"2026-09-21T00:00:00Z".into(),receipt_bytes:b"{}".to_vec() }).unwrap();
}
fn scratch() -> std::path::PathBuf {
    let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path=std::env::temp_dir().join(format!("gogoke-owner-outcome-{}-{nonce}",std::process::id()));
    std::fs::create_dir(&path).unwrap();path
}
fn cleanup(path:&std::path::Path) {
    std::fs::remove_file(path.join("state.sqlite")).unwrap();
    for suffix in ["-wal","-shm"] { match std::fs::remove_file(path.join(format!("state.sqlite{suffix}"))) {
        Ok(())=>{},Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},Err(e)=>panic!("owned fixture cleanup:{e}"),
    } }
    std::fs::remove_dir(path).unwrap();
}
fn fixture(run:impl FnOnce(&RootLock,&mut VerifiedDatabaseConnection<'_>,&OwnerIssuer)) {
    let _guard=route_b_test_guard();let path=scratch();let root=RootLock::acquire(&path).unwrap();
    let mut connection=create_new(&root,&path.join("state.sqlite")).unwrap();
    apply_core_schema(&mut connection).unwrap();apply_action_schema(&mut connection).unwrap();
    let owner=initialize_profile(&mut connection,&root).unwrap();seed(&mut connection);
    run(&root,&mut connection,&owner);connection.close_checked().unwrap();drop(root);cleanup(&path);
}
fn outcome_count(connection:&mut VerifiedDatabaseConnection<'_>)->String {
    let query=Statement::prepare(connection.as_ptr(),"SELECT count(*) FROM gogoke_objects WHERE object_type='OutcomeRecord'").unwrap();
    assert!(query.step_row().unwrap());query.column_text(0).unwrap()
}
#[test]
fn native_owner_append_and_replay_do_not_duplicate_or_dispatch() {
    fixture(|_,db,owner| {
        let input=request("1",None);
        assert_eq!(append_owner_override_outcome(db,owner,&input).unwrap().disposition,"COMMITTED");
        assert_eq!(append_owner_override_outcome(db,owner,&input).unwrap().disposition,"RECONCILED");
        assert_eq!(outcome_count(db),"1");
        let query=Statement::prepare(db.as_ptr(),"SELECT state FROM gogoke_action_reservations").unwrap();
        assert!(query.step_row().unwrap());assert_eq!(query.column_text(0).unwrap(),"reserved");
    });
}
#[test]
fn native_owner_correction_appends_immutable_prior_revision() {
    fixture(|_,db,owner| {
        let first=request("1",None);append_owner_override_outcome(db,owner,&first).unwrap();
        let second=request("2",Some(OutcomeVersionRef {revision:"1".into(),content_hash:content_hash(&first.canonical_outcome)}));
        append_owner_override_outcome(db,owner,&second).unwrap();assert_eq!(outcome_count(db),"2");
        let query=Statement::prepare(db.as_ptr(),"SELECT CAST(canonical_json AS TEXT) FROM gogoke_objects WHERE object_type='OutcomeRecord' AND object_version='1'").unwrap();
        assert!(query.step_row().unwrap());assert_eq!(query.column_text(0).unwrap().as_bytes(),first.canonical_outcome.as_slice());
    });
}
#[test]
fn stale_outcome_head_cannot_create_another_revision_two() {
    fixture(|_,db,owner| {
        let first=request("1",None);append_owner_override_outcome(db,owner,&first).unwrap();
        let mut second=request("2",Some(OutcomeVersionRef {revision:"1".into(),content_hash:content_hash(&first.canonical_outcome)}));
        append_owner_override_outcome(db,owner,&second).unwrap();second.operation_id="new-operation".into();
        second.event_id="new-event".into();second.receipt_id="new-receipt".into();
        assert!(matches!(append_owner_override_outcome(db,owner,&second),Err(OrchestrationError::OperationConflict)));
        assert_eq!(outcome_count(db),"2");
    });
}
#[test]
fn replay_rechecks_current_policy_and_revocation_head() {
    fixture(|_,db,owner| {
        let input=request("1",None);append_owner_override_outcome(db,owner,&input).unwrap();
        db.execute("UPDATE gogoke_authority_profile SET policy_revision='2'").unwrap();
        assert!(append_owner_override_outcome(db,owner,&input).is_err());
        db.execute("UPDATE gogoke_authority_profile SET policy_revision='1',revocation_head='1'").unwrap();
        assert!(append_owner_override_outcome(db,owner,&input).is_err());assert_eq!(outcome_count(db),"1");
    });
}
#[test]
fn wrong_source_domain_decision_hash_or_action_digest_never_appends() {
    fixture(|_,db,owner| {
        let mutations:[fn(&mut OwnerOutcomeAppend);4]=[
            |x|x.domain_id="other-domain".into(),|x|x.decision_hash=format!("sha256:{}","b".repeat(64)),
            |x|x.action_digest=format!("sha256:{}","b".repeat(64)),|x|x.decision_version="2".into(),
        ];
        for mutate in mutations {let mut input=request("1",None);mutate(&mut input);assert!(append_owner_override_outcome(db,owner,&input).is_err());}
        assert_eq!(outcome_count(db),"0");
    });
}
#[test]
fn owner_override_cannot_claim_another_label_producer() {
    fixture(|_,db,owner| {
        for label in ["OBJECTIVE","INDEPENDENT_SEMANTIC","SELF_REPORT"] {
            let mut input=request("1",None);
            input.canonical_outcome=String::from_utf8(input.canonical_outcome).unwrap().replace("OWNER_OVERRIDE",label).into_bytes();
            assert!(append_owner_override_outcome(db,owner,&input).is_err());
        }
        assert_eq!(outcome_count(db),"0");
    });
}
#[test]
fn malformed_duplicate_or_noncanonical_outcome_is_not_persisted() {
    fixture(|_,db,owner| {
        for bytes in [b"{}".to_vec(),b"[]".to_vec(),b"invalid".to_vec(),
            String::from_utf8(document("1","PENDING")).unwrap().replacen("{","{\"outcomeId\":\"forged\",",1).into_bytes(),
            String::from_utf8(document("1","PENDING")).unwrap().replacen("{","{ ",1).into_bytes()] {
            let mut input=request("1",None);input.canonical_outcome=bytes;
            assert!(append_owner_override_outcome(db,owner,&input).is_err());
        }
        assert_eq!(outcome_count(db),"0");
    });
}
#[test]
fn changed_replay_body_conflicts_without_overwriting_the_receipt() {
    fixture(|_,db,owner| {
        let input=request("1",None);let receipt=append_owner_override_outcome(db,owner,&input).unwrap();
        let mut changed=input.clone();changed.canonical_outcome=document("1","CENSORED");
        assert!(append_owner_override_outcome(db,owner,&changed).is_err());
        assert_eq!(append_owner_override_outcome(db,owner,&input).unwrap().operation_fingerprint,receipt.operation_fingerprint);
        assert_eq!(outcome_count(db),"1");
    });
}
#[test]
fn caller_error_after_outcome_append_rolls_back_the_entire_write_group() {
    fixture(|_,db,owner| {
        let result:transaction::Result<()>=transaction::run(db,|tx| {
            apply_owner_override_outcome(tx,owner,&request("1",None))?;Err(OrchestrationError::AccessDenied)
        });
        assert!(result.is_err());assert_eq!(outcome_count(db),"0");
        assert_eq!(append_owner_override_outcome(db,owner,&request("1",None)).unwrap().disposition,"COMMITTED");
    });
}
#[test]
fn checked_close_reopen_keeps_owner_outcome_and_append_history() {
    let _guard=route_b_test_guard();let path=scratch();let root=RootLock::acquire(&path).unwrap();
    let database=path.join("state.sqlite");let mut db=create_new(&root,&database).unwrap();
    apply_core_schema(&mut db).unwrap();apply_action_schema(&mut db).unwrap();let owner=initialize_profile(&mut db,&root).unwrap();seed(&mut db);
    let first=request("1",None);let receipt=append_owner_override_outcome(&mut db,&owner,&first).unwrap();
    db.close_checked().unwrap();let mut reopened=open_existing(&root,&database).unwrap();
    let resumed=initialize_profile(&mut reopened,&root).unwrap();
    assert_eq!(append_owner_override_outcome(&mut reopened,&resumed,&first).unwrap().operation_fingerprint,receipt.operation_fingerprint);
    let second=request("2",Some(OutcomeVersionRef {revision:"1".into(),content_hash:content_hash(&first.canonical_outcome)}));
    append_owner_override_outcome(&mut reopened,&resumed,&second).unwrap();assert_eq!(outcome_count(&mut reopened),"2");
    reopened.close_checked().unwrap();drop(root);cleanup(&path);
}
