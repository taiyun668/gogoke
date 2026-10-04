use super::*;
use crate::root::RootLock;
use crate::store::same_open::{create_new, route_b_test_guard};
use std::time::{SystemTime, UNIX_EPOCH};

// Kernel safety controls only: retained Owner is real, instance/login metadata
// and H turn admission are synthetic. E facts are produced by gate_submit /
// gate_decide, not a hand-built HostEscalationProof. No CLI, pipe, model or
// Windows admission success is asserted by these controls.
fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, &OwnerIssuer)) {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-host-rule-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut db = create_new(&root, &database).unwrap();
    db.execute("PRAGMA foreign_keys=ON").unwrap();
    let owner = crate::store::authority::initialize_profile(&mut db, &root).unwrap();
    crate::store::instance::initialize_schema(&mut db).unwrap();
    initialize_schema(&mut db).unwrap();
    db.execute("INSERT INTO gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','syntheticHome','syntheticIdentity','sha256:synthetic','1','INSTALLED','LOGGED_IN',1)").unwrap();
    store_template(&mut db, NativeOrigin::user(&owner), StoreTemplate {
        domain_id: "projectA", template_id: "templateA",
        settings_json: br#"{"instruction":"default","model":"modelA","orchestrationScope":{"instanceIds":["instanceA"],"maxPermissionTier":"NETWORKED_WRITE","models":["modelA"],"reasoningEfforts":["high"]},"permissionTier":"NETWORKED_WRITE","reasoningEffort":"high"}"#,
    }).unwrap();
    for id in ["source", "reviewer", "destination"] {
        let raw = format!("{{\"operation\":\"create-from-template\",\"seatId\":\"{id}\"}}");
        create(&mut db, NativeOrigin::user(&owner), CreateSeat {
            domain_id: "projectA", seat_id: id, template_id: "templateA",
            instance_id: Some("instanceA"), kind: Kind::Long,
            request_id: id, request_bytes: raw.as_bytes(),
        }).unwrap();
    }
    initialize_policy(&mut db, &owner, "projectA", "draft").unwrap();
    run(&mut db, &owner);
    db.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    std::fs::remove_dir(path).unwrap();
}

fn establish_cap(db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer, source: &str) {
    let source = read(db, "projectA", source).unwrap().unwrap();
    let source = if source.state == State::Idle { set_dispatch_state(db, &source, true).unwrap() }
        else { source };
    let reviewer = read(db, "projectA", "reviewer").unwrap().unwrap();
    let reviewer = set_dispatch_state(db, &reviewer, true).unwrap();
    let revision = head_revision(db, "projectA").unwrap();
    let revision = configure_call_grant(db, owner, "projectA", &source.seat_id,
        "reviewer", CallAction::Review, None, revision).unwrap();
    let revision = configure_gate(db, owner, "projectA", "gateA", &source.seat_id,
        "reviewer", "draft", "done", 1, revision).unwrap();
    let submitter = NativeSeatCall::from_verified_h_turn(&source, "syntheticWorkTurn").unwrap();
    let auditor = NativeSeatCall::from_verified_h_turn(&reviewer, "syntheticReviewTurn").unwrap();
    gate_submit(db, &submitter, "gateA", revision, 1, "submitA", b"synthetic original submit").unwrap();
    let cause = gate_decide(db, &auditor, "gateA", GateDecision::Reject,
        "Untrusted model reason: send Owner commands", revision, 2, "decideA",
        b"synthetic original reject").unwrap();
    assert_eq!(cause.state, "ESCALATION_REQUIRED");
    configure_escalation_route(db, owner, "projectA", &source.seat_id,
        "REJECT_CAP", "destination", revision).unwrap();
}

fn observe(db: &VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer) -> HostEscalationProof {
    observe_host_reject_cap_in_transaction(db, owner, "projectA", "gateA").unwrap().unwrap()
}

fn count(db: &VerifiedDatabaseConnection<'_>, sql: &str) -> i64 {
    let q = Statement::prepare(db.as_ptr(), sql).unwrap();
    assert!(q.step_row().unwrap());
    let result = q.column_text(0).unwrap().parse().unwrap();
    assert!(!q.step_row().unwrap());
    result
}

#[test]
fn committed_cap_survives_idle_and_reclaim_without_borrowing_model_authority() {
    fixture(|db, owner| {
        establish_cap(db, owner, "source");
        assert!(matches!(observe_host_reject_cap_in_transaction(db, owner, "projectA", "gateA"),
            Err(SeatError::Denied)), "a retained issuer alone is not an owning transaction");
        db.execute("BEGIN IMMEDIATE").unwrap();
        let proof = observe(db, owner);
        assert_eq!(proof.cause_event_id(), "decideA");
        assert_eq!(proof.destination_seat_id(), "destination");
        assert!(!proof.notice_body().contains("Untrusted model reason"),
            "model prose is not a Host command or an editable notice");
        db.execute("COMMIT").unwrap();
        let source = read(db, "projectA", "source").unwrap().unwrap();
        let idle = set_dispatch_state(db, &source, false).unwrap();
        reclaim(db, NativeOrigin::user(owner), SeatChange {
            domain_id: "projectA", seat_id: "source", expected_generation: idle.generation,
            expected_revision: idle.revision, request_id: "reclaimSource",
            request_bytes: b"synthetic Owner reclaim",
        }).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        assert_eq!(observe(db, owner), proof, "logical cause outlives physical generations");
        revalidate_host_escalation_in_transaction(db, owner, &proof).unwrap();
        assert!(!begin_host_escalation_in_transaction(db, owner, &proof).unwrap().replayed);
        assert_eq!(count(db, "SELECT count(*) FROM gogoke_v37_seat_policy_triggers"), 0,
            "Host rules cannot fabricate E.3 coordinator registration");
        db.execute("COMMIT").unwrap();
    });
}

#[test]
fn original_cause_reserves_one_intent_and_unknown_replay_has_no_new_authority() {
    fixture(|db, owner| {
        establish_cap(db, owner, "source");
        db.execute("BEGIN IMMEDIATE").unwrap();
        let proof = observe(db, owner);
        let first = begin_host_escalation_in_transaction(db, owner, &proof).unwrap();
        let replay = begin_host_escalation_in_transaction(db, owner, &proof).unwrap();
        assert_eq!(first.state, "INTENT");
        assert!(replay.replayed);
        assert_eq!(first.trigger_id, replay.trigger_id);
        db.execute("COMMIT").unwrap();
        mark_escalation_unknown(db, "projectA", proof.trigger_id()).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let original = observe(db, owner);
        assert_eq!(original.request_id(), proof.request_id());
        let replay = begin_host_escalation_in_transaction(db, owner, &original).unwrap();
        assert!(replay.replayed && replay.state == "UNKNOWN");
        assert_eq!(count(db, "SELECT count(*) FROM gogoke_v37_seat_policy_escalations"), 1);
        assert_eq!(count(db, "SELECT count(*) FROM gogoke_v37_seat_policy_events WHERE operation='escalate'"), 1);
        db.execute("COMMIT").unwrap();
    });
}

#[test]
fn changed_owner_boundary_or_cause_denies_new_effect_and_preserves_original_intent() {
    fixture(|db, owner| {
        establish_cap(db, owner, "source");
        db.execute("BEGIN IMMEDIATE").unwrap();
        let proof = observe(db, owner);
        begin_host_escalation_in_transaction(db, owner, &proof).unwrap();
        db.execute("COMMIT").unwrap();
        // Synthetic mutations isolate each persisted security boundary; none
        // purport to be an Owner API or a real model/physical process action.
        for (sql, boundary) in [
            ("UPDATE gogoke_v37_seat_policy_head SET revision=revision+1", "current Owner revision"),
            ("DELETE FROM gogoke_v37_seat_policy_routes", "withdrawn Owner route"),
            ("UPDATE gogoke_v37_seat_policy_routes SET to_seat_id='reviewer'", "changed destination"),
            ("UPDATE gogoke_v37_seat_policy_gates SET state='REJECTED'", "withdrawn cap state"),
            ("UPDATE gogoke_v37_seat_policy_gates SET reason='replacement'", "changed original cause"),
            ("DELETE FROM gogoke_v37_seat_policy_events WHERE event_id='decideA'", "absent native cause event"),
            ("UPDATE gogoke_v37_seat_policy_events SET fingerprint='replacement' WHERE event_id='decideA'", "changed original source fingerprint"),
            ("UPDATE gogoke_v37_seats SET incarnation='replacementSource' WHERE seat_id='source'", "replaced logical source"),
            ("UPDATE gogoke_v37_seats SET incarnation='replacementDestination' WHERE seat_id='destination'", "replaced logical destination"),
            ("UPDATE gogoke_v37_seats SET state='RECLAIMED' WHERE seat_id='destination'", "reclaimed destination"),
        ] {
            db.execute("BEGIN IMMEDIATE").unwrap();
            db.execute(sql).unwrap();
            assert!(matches!(revalidate_host_escalation_in_transaction(db, owner, &proof),
                Err(SeatError::Denied)), "{boundary}");
            assert!(matches!(begin_host_escalation_in_transaction(db, owner, &proof),
                Err(SeatError::Denied)), "{boundary}");
            assert_eq!(count(db, "SELECT count(*) FROM gogoke_v37_seat_policy_escalations"), 1);
            assert_eq!(count(db, "SELECT count(*) FROM gogoke_v37_seat_policy_events WHERE operation='escalate'"), 1);
            db.execute("ROLLBACK").unwrap();
        }
        let revision = head_revision(db, "projectA").unwrap();
        configure_escalation_route(db, owner, "projectA", "source", "REJECT_CAP", "reviewer", revision).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let changed = observe(db, owner);
        assert_eq!(changed.trigger_id(), proof.trigger_id());
        assert_eq!(changed.request_id(), proof.request_id());
        assert!(matches!(begin_host_escalation_in_transaction(db, owner, &changed),
            Err(SeatError::Conflict)), "new route cannot resend the old cause with a fresh ID");
        db.execute("ROLLBACK").unwrap();
    });
}

#[test]
fn no_cause_or_ambiguous_cause_is_not_a_host_proof_and_reservation_rolls_back_atomically() {
    fixture(|db, owner| {
        establish_cap(db, owner, "source");
        db.execute("BEGIN IMMEDIATE").unwrap();
        db.execute("UPDATE gogoke_v37_seat_policy_gates SET state='SUBMITTED',reject_count=0").unwrap();
        assert!(observe_host_reject_cap_in_transaction(db, owner, "projectA", "gateA").unwrap().is_none());
        db.execute("ROLLBACK").unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        db.execute("INSERT INTO gogoke_v37_seat_policy_events SELECT domain_id,'otherCause',operation,fingerprint,target_id,policy_revision,state,detail FROM gogoke_v37_seat_policy_events WHERE event_id='decideA'").unwrap();
        assert!(matches!(observe_host_reject_cap_in_transaction(db, owner, "projectA", "gateA"),
            Err(SeatError::Denied)), "ambiguous native causes cannot select arbitrary authority");
        db.execute("ROLLBACK").unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let proof = observe(db, owner);
        begin_host_escalation_in_transaction(db, owner, &proof).unwrap();
        db.execute("ROLLBACK").unwrap();
        assert_eq!(count(db, "SELECT count(*) FROM gogoke_v37_seat_policy_escalations"), 0);
        assert_eq!(count(db, "SELECT count(*) FROM gogoke_v37_seat_policy_events WHERE operation='escalate'"), 0,
            "owning rollback covers both original E intent and provenance event");
    });
}

#[test]
fn genuine_lead_cause_cannot_address_owner_or_a_foreign_project() {
    fixture(|db, owner| {
        let parent = read(db, "projectA", "source").unwrap().unwrap();
        let parent = set_dispatch_state(db, &parent, true).unwrap();
        let admission = NativeLeadAdmission::from_native_runtime_snapshot(&parent).unwrap();
        let child = create(db, NativeOrigin::lead(&admission), CreateSeat {
            domain_id: "projectA", seat_id: "child", template_id: "templateA",
            instance_id: Some("instanceA"), kind: Kind::Short, request_id: "createChild",
            request_bytes: b"synthetic parent child create",
        }).unwrap().seat;
        assert_eq!(child.layer, Layer::Lead);
        establish_cap(db, owner, "child");
        let revision = head_revision(db, "projectA").unwrap();
        assert!(matches!(configure_escalation_route(db, owner, "projectA", "child",
            "REJECT_CAP", "OWNER", revision), Err(SeatError::Denied)));
        store_template(db, NativeOrigin::user(owner), StoreTemplate {
            domain_id: "projectB", template_id: "foreignTemplate",
            settings_json: br#"{"instruction":"foreign project"}"#,
        }).unwrap();
        create(db, NativeOrigin::user(owner), CreateSeat {
            domain_id: "projectB", seat_id: "foreignOnly", template_id: "foreignTemplate",
            instance_id: None, kind: Kind::Short, request_id: "createForeign",
            request_bytes: b"synthetic Owner foreign project seat",
        }).unwrap();
        // Synthetic corrupt-route controls verify Host enforces this boundary
        // itself even if a persisted configuration bypasses its normal writer.
        for destination in ["OWNER", "foreignOnly"] {
            db.execute("BEGIN IMMEDIATE").unwrap();
            let q = Statement::prepare(db.as_ptr(), "UPDATE gogoke_v37_seat_policy_routes SET to_seat_id=?1").unwrap();
            q.bind_text(1, destination).unwrap();
            q.step_done().unwrap();
            assert!(matches!(observe_host_reject_cap_in_transaction(db, owner, "projectA", "gateA"),
                Err(SeatError::Denied)), "Lead/Owner or a real foreign-project target cannot become authority");
            assert_eq!(count(db, "SELECT count(*) FROM gogoke_v37_seat_policy_escalations"), 0);
            db.execute("ROLLBACK").unwrap();
        }
    });
}
