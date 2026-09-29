use super::*;
use crate::root::RootLock;
use crate::store::same_open::{create_new, route_b_test_guard};
use std::time::{SystemTime, UNIX_EPOCH};

// Complete ingress records for the focused native tests. The store receives
// these bytes verbatim; it does not reconstruct them from typed fields.
fn wire(request_id: &str) -> &'static [u8] {
    match request_id {
        "createLead" => br#"{"op":"create-from-template","requestId":"createLead","domainId":"projectA","seatId":"lead","templateId":"templateA","instanceId":"instanceA","kind":"LONG"}"#,
        "createAnother" => br#"{"op":"create-from-template","requestId":"createAnother","domainId":"projectA","seatId":"another","templateId":"templateA","instanceId":"instanceA","kind":"LONG"}"#,
        "busyBind" => br#"{"op":"bind","requestId":"busyBind","domainId":"projectA","seatId":"lead","expectedGeneration":2,"instanceId":"instanceB"}"#,
        "bindOnce" => br#"{"op":"bind","requestId":"bindOnce","domainId":"projectA","seatId":"lead","expectedGeneration":3,"instanceId":"instanceB"}"#,
        "staleBind" => br#"{"op":"bind","requestId":"staleBind","domainId":"projectA","seatId":"lead","expectedGeneration":1,"instanceId":"instanceA"}"#,
        "createWorker" => br#"{"op":"create-from-template","requestId":"createWorker","domainId":"projectA","seatId":"worker","templateId":"templateA","instanceId":"instanceA","kind":"SHORT"}"#,
        "forbiddenBind" => br#"{"op":"bind","requestId":"forbiddenBind","domainId":"projectA","seatId":"another","expectedGeneration":1,"instanceId":"instanceB"}"#,
        "otherProjectCreate" => br#"{"op":"create-from-template","requestId":"otherProjectCreate","domainId":"projectB","seatId":"otherProject","templateId":"templateA","instanceId":"instanceA","kind":"SHORT"}"#,
        "promoteWorker" => br#"{"op":"promote","requestId":"promoteWorker","domainId":"projectA","seatId":"worker","expectedGeneration":1}"#,
        "reclaimWorker" => br#"{"op":"reclaim","requestId":"reclaimWorker","domainId":"projectA","seatId":"worker","expectedGeneration":2}"#,
        "reuseWorker" => br#"{"op":"create-from-template","requestId":"reuseWorker","domainId":"projectA","seatId":"worker","templateId":"templateA","instanceId":"instanceA","kind":"SHORT"}"#,
        "afterStopCreate" => br#"{"op":"create-from-template","requestId":"afterStopCreate","domainId":"projectA","seatId":"afterStop","templateId":"templateA","instanceId":"instanceA","kind":"SHORT"}"#,
        "createUnbound" => br#"{"op":"create-from-template","requestId":"createUnbound","domainId":"projectA","seatId":"unbound","templateId":"templateA","kind":"SHORT"}"#,
        "bindUnbound" => br#"{"op":"bind-instance","requestId":"bindUnbound","domainId":"projectA","seatId":"unbound","expectedGeneration":1,"instanceId":"instanceB"}"#,
        _ => panic!("missing complete test request: {request_id}"),
    }
}

fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, &OwnerIssuer)) {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-v37-seat-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut db = create_new(&root, &database).unwrap();
    db.execute("PRAGMA foreign_keys=ON").unwrap();
    let owner = crate::store::authority::initialize_profile(&mut db, &root).unwrap();
    crate::store::instance::initialize_schema(&mut db).unwrap();
    initialize_schema(&mut db).unwrap();
    store_template(
        &mut db,
        NativeOrigin::user(&owner),
        StoreTemplate {
            domain_id: "projectA",
            template_id: "templateA",
            settings_json: br#"{"instruction":"default"}"#,
        },
    )
    .unwrap();
    for (instance, home, identity) in [
        ("instanceA", "homeA", "identityA"),
        ("instanceB", "homeB", "identityB"),
    ] {
        let insert = Statement::prepare(db.as_ptr(), "INSERT INTO gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES(?1,'codex',?2,?3,'sha256:test','1','INSTALLED','LOGGED_IN',1)").unwrap();
        insert.bind_text(1, instance).unwrap();
        insert.bind_text(2, home).unwrap();
        insert.bind_text(3, identity).unwrap();
        insert.step_done().unwrap();
    }
    run(&mut db, &owner);
    db.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    if let Err(error) = std::fs::remove_dir(&path) {
        eprintln!("owned fixture retained: {} ({error})", path.display());
    }
}

fn create_user(
    db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    seat_id: &str,
    request_id: &str,
) -> Seat {
    create(
        db,
        NativeOrigin::user(owner),
        CreateSeat {
            domain_id: "projectA",
            seat_id,
            template_id: "templateA",
            instance_id: Some("instanceA"),
            kind: Kind::Long,
            request_id,
            request_bytes: wire(request_id),
        },
    )
    .unwrap()
    .seat
}

#[test]
fn exact_schema_reopens_and_drift_refuses_repair() {
    fixture(|db, _| {
        initialize_schema(db).unwrap();
        db.execute("DROP TABLE gogoke_v37_seat_operations").unwrap();
        assert!(matches!(initialize_schema(db), Err(SeatError::SchemaDrift)));
    });
}

#[test]
fn legacy_bound_seat_migrates_without_losing_identity_or_binding() {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "gogoke-v37-seat-legacy-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut db = create_new(&root, &database).unwrap();
    db.execute("PRAGMA foreign_keys=ON").unwrap();
    let _owner = crate::store::authority::initialize_profile(&mut db, &root).unwrap();
    crate::store::instance::initialize_schema(&mut db).unwrap();
    db.execute(LEGACY_SEATS).unwrap();
    db.execute(OPERATIONS).unwrap();
    let insert_instance = Statement::prepare(db.as_ptr(), "INSERT INTO gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','homeA','identityA','sha256:test','1','INSTALLED','LOGGED_IN',1)").unwrap();
    insert_instance.step_done().unwrap();
    let insert_lead = Statement::prepare(db.as_ptr(), "INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) VALUES('projectA','lead','incarnationA','USER',NULL,'LONG','instanceA','IDLE',1,1)").unwrap();
    insert_lead.step_done().unwrap();
    let insert_child = Statement::prepare(db.as_ptr(), "INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) VALUES('projectA','child','incarnationB','LEAD','lead','SHORT','instanceA','IDLE',1,1)").unwrap();
    insert_child.step_done().unwrap();
    let insert_operation = Statement::prepare(db.as_ptr(), "INSERT INTO gogoke_v37_seat_operations(domain_id,request_id,fingerprint,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,revision,generation) VALUES('projectA','legacyCreate','legacy-fingerprint','lead','incarnationA','USER',NULL,'LONG','instanceA','IDLE',1,1)").unwrap();
    insert_operation.step_done().unwrap();

    initialize_schema(&mut db).unwrap();
    let restored = get(&db, "projectA", "lead").unwrap().unwrap();
    assert_eq!(restored.incarnation, "incarnationA");
    assert_eq!(restored.instance_id, "instanceA");
    assert!(restored.template_id.is_none());
    assert!(restored.settings_json.is_none());
    let restored_child = get(&db, "projectA", "child").unwrap().unwrap();
    assert_eq!(restored_child.parent_seat_id.as_deref(), Some("lead"));
    let replay = operation(&db, "projectA", "legacyCreate", "legacy-fingerprint")
        .unwrap()
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.seat, restored);
    let snapshot_count = Statement::prepare(
        db.as_ptr(),
        "SELECT COUNT(*) FROM gogoke_v37_seat_operation_snapshots",
    )
    .unwrap();
    assert!(snapshot_count.step_row().unwrap());
    assert_eq!(snapshot_count.column_text(0).unwrap(), "0");
    initialize_schema(&mut db).unwrap();

    db.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    if let Err(error) = std::fs::remove_dir(&path) {
        eprintln!("owned fixture retained: {} ({error})", path.display());
    }
}

#[test]
fn empty_or_oversize_raw_request_is_rejected_before_mutation() {
    fixture(|db, owner| {
        let oversized = vec![b'x'; crate::ipc::MAX_FRAME_BYTES + 1];
        for raw in [&b""[..], oversized.as_slice()] {
            assert!(matches!(
                create(
                    db,
                    NativeOrigin::user(owner),
                    CreateSeat {
                        domain_id: "projectA",
                        seat_id: "lead",
                        template_id: "templateA",
                        instance_id: Some("instanceA"),
                        kind: Kind::Long,
                        request_id: "createLead",
                        request_bytes: raw,
                    }
                ),
                Err(SeatError::Invalid("request_bytes"))
            ));
        }
        assert!(get(db, "projectA", "lead").unwrap().is_none());
    });
}

#[test]
fn create_from_stored_template_copies_settings_before_later_binding() {
    fixture(|db, owner| {
        let created = create(
            db,
            NativeOrigin::user(owner),
            CreateSeat {
                domain_id: "projectA",
                seat_id: "unbound",
                template_id: "templateA",
                instance_id: None,
                kind: Kind::Short,
                request_id: "createUnbound",
                request_bytes: wire("createUnbound"),
            },
        )
        .unwrap();
        assert_eq!(created.seat.instance_id, "");
        assert_eq!(created.seat.template_id.as_deref(), Some("templateA"));
        assert_eq!(
            created.seat.settings_json.as_deref(),
            Some("{\"instruction\":\"default\"}")
        );

        // The seat owns a snapshot. A later template edit cannot rewrite an
        // already-created seat's settings.
        db.execute("UPDATE gogoke_v37_seat_templates SET settings_json='{\"instruction\":\"changed\"}' WHERE domain_id='projectA' AND template_id='templateA'").unwrap();
        let before_bind = get(db, "projectA", "unbound").unwrap().unwrap();
        assert_eq!(
            before_bind.settings_json.as_deref(),
            Some("{\"instruction\":\"default\"}")
        );
        assert_eq!(before_bind.instance_id, "");

        let bound = bind_instance(
            db,
            NativeOrigin::user(owner),
            SeatChange {
                domain_id: "projectA",
                seat_id: "unbound",
                expected_generation: 1,
                request_id: "bindUnbound",
                request_bytes: wire("bindUnbound"),
            },
            "instanceB",
        )
        .unwrap();
        assert_eq!(bound.seat.instance_id, "instanceB");
        assert_eq!(
            bound.seat.settings_json.as_deref(),
            Some("{\"instruction\":\"default\"}")
        );
    });
}

#[test]
fn binding_refuses_busy_or_stale_generation_and_replay_checks_current_target() {
    fixture(|db, owner| {
        let original = create_user(db, owner, "lead", "createLead");
        let original_replay = create(
            db,
            NativeOrigin::user(owner),
            CreateSeat {
                domain_id: "projectA",
                seat_id: "lead",
                template_id: "templateA",
                instance_id: Some("instanceA"),
                kind: Kind::Long,
                request_id: "createLead",
                request_bytes: wire("createLead"),
            },
        )
        .unwrap();
        assert!(original_replay.replayed);
        assert_eq!(original_replay.seat, original);
        assert!(matches!(
            create(
                db,
                NativeOrigin::user(owner),
                CreateSeat {
                    domain_id: "projectA",
                    seat_id: "lead",
                    template_id: "templateA",
                    instance_id: Some("instanceA"),
                    kind: Kind::Long,
                    request_id: "createLead",
                    request_bytes: br#"{ "op":"create-from-template","requestId":"createLead","domainId":"projectA","seatId":"lead","templateId":"templateA","instanceId":"instanceA","kind":"LONG"}"#,
                }
            ),
            Err(SeatError::Conflict)
        ));
        let busy = set_dispatch_state(db, &original, true).unwrap();
        assert!(matches!(
            bind_instance(
                db,
                NativeOrigin::user(owner),
                SeatChange {
                    domain_id: "projectA",
                    seat_id: "lead",
                    expected_generation: busy.generation,
                    request_id: "busyBind",
                    request_bytes: wire("busyBind"),
                },
                "instanceB"
            ),
            Err(SeatError::Busy)
        ));
        let idle = set_dispatch_state(db, &busy, false).unwrap();
        let changed = bind_instance(
            db,
            NativeOrigin::user(owner),
            SeatChange {
                domain_id: "projectA",
                seat_id: "lead",
                expected_generation: idle.generation,
                request_id: "bindOnce",
                request_bytes: wire("bindOnce"),
            },
            "instanceB",
        )
        .unwrap();
        assert_eq!(changed.seat.generation, 4);
        assert_eq!(changed.seat.instance_id, "instanceB");
        assert!(matches!(
            create(
                db,
                NativeOrigin::user(owner),
                CreateSeat {
                    domain_id: "projectA",
                    seat_id: "lead",
                    template_id: "templateA",
                    instance_id: Some("instanceA"),
                    kind: Kind::Long,
                    request_id: "createLead",
                    request_bytes: wire("createLead"),
                }
            ),
            Err(SeatError::Conflict)
        ));
        assert!(matches!(
            bind_instance(
                db,
                NativeOrigin::user(owner),
                SeatChange {
                    domain_id: "projectA",
                    seat_id: "lead",
                    expected_generation: 1,
                    request_id: "staleBind",
                    request_bytes: wire("staleBind"),
                },
                "instanceA"
            ),
            Err(SeatError::Conflict)
        ));
        let replay = bind_instance(
            db,
            NativeOrigin::user(owner),
            SeatChange {
                domain_id: "projectA",
                seat_id: "lead",
                expected_generation: idle.generation,
                request_id: "bindOnce",
                request_bytes: wire("bindOnce"),
            },
            "instanceB",
        )
        .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.seat, changed.seat);
        assert!(matches!(
            bind_instance(
                db,
                NativeOrigin::user(owner),
                SeatChange {
                    domain_id: "projectA",
                    seat_id: "lead",
                    expected_generation: idle.generation,
                    request_id: "bindOnce",
                    request_bytes: br#"{"op":"bind","requestId":"bindOnce","domainId":"projectA","seatId":"lead","expectedGeneration":3,"instanceId":"instanceB","hidden":"payload"}"#,
                },
                "instanceB"
            ),
            Err(SeatError::Conflict)
        ));
        assert!(matches!(
            bind_instance(
                db,
                NativeOrigin::user(owner),
                SeatChange {
                    domain_id: "projectA",
                    seat_id: "lead",
                    expected_generation: 1,
                    request_id: "bindOnce",
                    request_bytes: wire("bindOnce"),
                },
                "instanceA"
            ),
            Err(SeatError::Conflict)
        ));
        assert_eq!(idle.generation, 3);
    });
}

#[test]
fn lead_only_controls_own_layer_and_reclaim_retains_identity() {
    fixture(|db, owner| {
        let lead = create_user(db, owner, "lead", "createLead");
        let another = create_user(db, owner, "another", "createAnother");
        let active = set_dispatch_state(db, &lead, true).unwrap();
        let admission = NativeLeadAdmission::from_native_runtime_snapshot(&active).unwrap();
        let worker = create(
            db,
            NativeOrigin::lead(&admission),
            CreateSeat {
                domain_id: "projectA",
                seat_id: "worker",
                template_id: "templateA",
                instance_id: Some("instanceA"),
                kind: Kind::Short,
                request_id: "createWorker",
                request_bytes: wire("createWorker"),
            },
        )
        .unwrap()
        .seat;
        assert_eq!(worker.layer, Layer::Lead);
        assert_eq!(worker.parent_seat_id.as_deref(), Some("lead"));
        assert!(matches!(
            bind_instance(
                db,
                NativeOrigin::lead(&admission),
                SeatChange {
                    domain_id: "projectA",
                    seat_id: "another",
                    expected_generation: another.generation,
                    request_id: "forbiddenBind",
                    request_bytes: wire("forbiddenBind"),
                },
                "instanceB"
            ),
            Err(SeatError::Denied)
        ));
        assert!(matches!(
            create(
                db,
                NativeOrigin::lead(&admission),
                CreateSeat {
                    domain_id: "projectB",
                    seat_id: "otherProject",
                    template_id: "templateA",
                    instance_id: Some("instanceA"),
                    kind: Kind::Short,
                    request_id: "otherProjectCreate",
                    request_bytes: wire("otherProjectCreate"),
                }
            ),
            Err(SeatError::Denied)
        ));
        let promoted = promote(
            db,
            NativeOrigin::lead(&admission),
            SeatChange {
                domain_id: "projectA",
                seat_id: "worker",
                expected_generation: worker.generation,
                request_id: "promoteWorker",
                request_bytes: wire("promoteWorker"),
            },
        )
        .unwrap()
        .seat;
        assert_eq!(promoted.kind, Kind::Long);
        let reclaimed = reclaim(
            db,
            NativeOrigin::lead(&admission),
            SeatChange {
                domain_id: "projectA",
                seat_id: "worker",
                expected_generation: promoted.generation,
                request_id: "reclaimWorker",
                request_bytes: wire("reclaimWorker"),
            },
        )
        .unwrap()
        .seat;
        assert_eq!(reclaimed.state, State::Reclaimed);
        assert_eq!(reclaimed.incarnation, worker.incarnation);
        assert!(matches!(
            create(
                db,
                NativeOrigin::lead(&admission),
                CreateSeat {
                    domain_id: "projectA",
                    seat_id: "worker",
                    template_id: "templateA",
                    instance_id: Some("instanceA"),
                    kind: Kind::Short,
                    request_id: "reuseWorker",
                    request_bytes: wire("reuseWorker"),
                }
            ),
            Err(SeatError::Conflict)
        ));
        let stopped = set_dispatch_state(db, &active, false).unwrap();
        assert!(matches!(
            create(
                db,
                NativeOrigin::lead(&admission),
                CreateSeat {
                    domain_id: "projectA",
                    seat_id: "worker",
                    template_id: "templateA",
                    instance_id: Some("instanceA"),
                    kind: Kind::Short,
                    request_id: "createWorker",
                    request_bytes: wire("createWorker"),
                }
            ),
            Err(SeatError::Denied)
        ));
        assert!(matches!(
            create(
                db,
                NativeOrigin::lead(&admission),
                CreateSeat {
                    domain_id: "projectA",
                    seat_id: "afterStop",
                    template_id: "templateA",
                    instance_id: Some("instanceA"),
                    kind: Kind::Short,
                    request_id: "afterStopCreate",
                    request_bytes: wire("afterStopCreate"),
                }
            ),
            Err(SeatError::Denied)
        ));
        assert!(matches!(
            NativeLeadAdmission::from_native_runtime_snapshot(&worker),
            Err(SeatError::Denied)
        ));
        assert_eq!(stopped.state, State::Idle);
    });
}

#[test]
fn old_lead_change_replay_is_denied_after_admission_generation_changes() {
    fixture(|db, owner| {
        let lead = create_user(db, owner, "lead", "createLead");
        let active = set_dispatch_state(db, &lead, true).unwrap();
        let admission = NativeLeadAdmission::from_native_runtime_snapshot(&active).unwrap();
        let worker = create(
            db,
            NativeOrigin::lead(&admission),
            CreateSeat {
                domain_id: "projectA",
                seat_id: "worker",
                template_id: "templateA",
                instance_id: Some("instanceA"),
                kind: Kind::Short,
                request_id: "createWorker",
                request_bytes: wire("createWorker"),
            },
        )
        .unwrap()
        .seat;
        let change = SeatChange {
            domain_id: "projectA",
            seat_id: "worker",
            expected_generation: worker.generation,
            request_id: "promoteWorker",
            request_bytes: wire("promoteWorker"),
        };
        let promoted = promote(db, NativeOrigin::lead(&admission), change).unwrap();
        let immediate_replay = promote(
            db,
            NativeOrigin::lead(&admission),
            SeatChange {
                domain_id: "projectA",
                seat_id: "worker",
                expected_generation: worker.generation,
                request_id: "promoteWorker",
                request_bytes: wire("promoteWorker"),
            },
        )
        .unwrap();
        assert!(immediate_replay.replayed);
        assert_eq!(immediate_replay.seat, promoted.seat);
        let idle = set_dispatch_state(db, &active, false).unwrap();
        let new_active = set_dispatch_state(db, &idle, true).unwrap();
        assert_eq!(new_active.state, State::Busy);
        assert!(matches!(
            promote(
                db,
                NativeOrigin::lead(&admission),
                SeatChange {
                    domain_id: "projectA",
                    seat_id: "worker",
                    expected_generation: worker.generation,
                    request_id: "promoteWorker",
                    request_bytes: wire("promoteWorker"),
                }
            ),
            Err(SeatError::Denied)
        ));
        let fresh_admission =
            NativeLeadAdmission::from_native_runtime_snapshot(&new_active).unwrap();
        assert!(matches!(
            promote(
                db,
                NativeOrigin::lead(&fresh_admission),
                SeatChange {
                    domain_id: "projectA",
                    seat_id: "worker",
                    expected_generation: worker.generation,
                    request_id: "promoteWorker",
                    request_bytes: wire("promoteWorker"),
                }
            ),
            Err(SeatError::Conflict)
        ));
    });
}
