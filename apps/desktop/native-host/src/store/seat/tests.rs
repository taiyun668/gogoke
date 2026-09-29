use super::*;
use crate::root::RootLock;
use crate::store::same_open::{create_new, route_b_test_guard};
use std::time::{SystemTime, UNIX_EPOCH};

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
            instance_id: "instanceA",
            kind: Kind::Long,
            request_id,
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
fn binding_refuses_busy_or_stale_generation_and_replay_checks_current_target() {
    fixture(|db, owner| {
        let original = create_user(db, owner, "lead", "createLead");
        let original_replay = create(
            db,
            NativeOrigin::user(owner),
            CreateSeat {
                domain_id: "projectA",
                seat_id: "lead",
                instance_id: "instanceA",
                kind: Kind::Long,
                request_id: "createLead",
            },
        )
        .unwrap();
        assert!(original_replay.replayed);
        assert_eq!(original_replay.seat, original);
        let busy = set_dispatch_state(db, &original, true).unwrap();
        assert!(matches!(
            bind_instance(
                db,
                NativeOrigin::user(owner),
                SeatChange {
                    domain_id: "projectA",
                    seat_id: "lead",
                    expected_generation: busy.generation,
                    request_id: "busyBind"
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
                    instance_id: "instanceA",
                    kind: Kind::Long,
                    request_id: "createLead",
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
                    request_id: "staleBind"
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
                    expected_generation: 1,
                    request_id: "bindOnce"
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
                instance_id: "instanceA",
                kind: Kind::Short,
                request_id: "createWorker",
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
                    request_id: "forbiddenBind"
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
                    instance_id: "instanceA",
                    kind: Kind::Short,
                    request_id: "otherProjectCreate"
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
                    instance_id: "instanceA",
                    kind: Kind::Short,
                    request_id: "reuseWorker"
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
                    instance_id: "instanceA",
                    kind: Kind::Short,
                    request_id: "createWorker"
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
                    instance_id: "instanceA",
                    kind: Kind::Short,
                    request_id: "afterStopCreate"
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
                instance_id: "instanceA",
                kind: Kind::Short,
                request_id: "createWorker",
            },
        )
        .unwrap()
        .seat;
        let change = SeatChange {
            domain_id: "projectA",
            seat_id: "worker",
            expected_generation: worker.generation,
            request_id: "promoteWorker",
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
                }
            ),
            Err(SeatError::Conflict)
        ));
    });
}
