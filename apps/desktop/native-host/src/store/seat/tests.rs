use super::*;
use crate::root::RootLock;
use crate::store::same_open::{create_new, open_existing, route_b_test_guard};
use std::time::{SystemTime, UNIX_EPOCH};

// Complete ingress records for the focused native tests. The store receives
// these bytes verbatim; it does not reconstruct them from typed fields.
fn wire(request_id: &str) -> &'static [u8] {
    match request_id {
        "createLead" => br#"{"op":"create-from-template","requestId":"createLead","domainId":"projectA","seatId":"lead","templateId":"templateA","instanceId":"instanceA","kind":"LONG"}"#,
        "createOrdinary" => br#"{"op":"create-from-template","requestId":"createOrdinary","domainId":"projectA","seatId":"ordinary","templateId":"templateA","instanceId":"instanceA","kind":"LONG"}"#,
        "createAnother" => br#"{"op":"create-from-template","requestId":"createAnother","domainId":"projectA","seatId":"another","templateId":"templateA","instanceId":"instanceA","kind":"LONG"}"#,
        "reviewerCreate" => br#"{"op":"create-from-template","requestId":"reviewerCreate","domainId":"projectA","seatId":"reviewer","templateId":"templateA","instanceId":"instanceA","kind":"LONG"}"#,
        "busyBind" => br#"{"op":"bind-instance","requestId":"busyBind","domainId":"projectA","seatId":"lead","expectedGeneration":2,"expectedRevision":2,"instanceId":"instanceB"}"#,
        "boundBind" => br#"{"op":"bind-instance","requestId":"boundBind","domainId":"projectA","seatId":"lead","expectedGeneration":3,"expectedRevision":3,"instanceId":"instanceB"}"#,
        "changeOnce" => br#"{"op":"change-instance","requestId":"changeOnce","domainId":"projectA","seatId":"lead","expectedGeneration":3,"expectedRevision":3,"instanceId":"instanceB"}"#,
        "staleBind" => br#"{"op":"bind-instance","requestId":"staleBind","domainId":"projectA","seatId":"lead","expectedGeneration":1,"expectedRevision":1,"instanceId":"instanceA"}"#,
        "staleRevision" => br#"{"op":"change-instance","requestId":"staleRevision","domainId":"projectA","seatId":"lead","expectedGeneration":3,"expectedRevision":2,"instanceId":"instanceB"}"#,
        "createWorker" => br#"{"op":"create-from-template","requestId":"createWorker","domainId":"projectA","seatId":"worker","templateId":"templateA","instanceId":"instanceA","kind":"SHORT"}"#,
        "forbiddenBind" => br#"{"op":"bind-instance","requestId":"forbiddenBind","domainId":"projectA","seatId":"another","expectedGeneration":1,"expectedRevision":1,"instanceId":"instanceB"}"#,
        "otherProjectCreate" => br#"{"op":"create-from-template","requestId":"otherProjectCreate","domainId":"projectB","seatId":"otherProject","templateId":"templateA","instanceId":"instanceA","kind":"SHORT"}"#,
        "promoteWorker" => br#"{"op":"promote","requestId":"promoteWorker","domainId":"projectA","seatId":"worker","expectedGeneration":1,"expectedRevision":1}"#,
        "reclaimWorker" => br#"{"op":"reclaim","requestId":"reclaimWorker","domainId":"projectA","seatId":"worker","expectedGeneration":2,"expectedRevision":2}"#,
        "reuseWorker" => br#"{"op":"create-from-template","requestId":"reuseWorker","domainId":"projectA","seatId":"worker","templateId":"templateA","instanceId":"instanceA","kind":"SHORT"}"#,
        "afterStopCreate" => br#"{"op":"create-from-template","requestId":"afterStopCreate","domainId":"projectA","seatId":"afterStop","templateId":"templateA","instanceId":"instanceA","kind":"SHORT"}"#,
        "createUnbound" => br#"{"op":"create-from-template","requestId":"createUnbound","domainId":"projectA","seatId":"unbound","templateId":"templateA","kind":"SHORT"}"#,
        "bindUnbound" => br#"{"op":"bind-instance","requestId":"bindUnbound","domainId":"projectA","seatId":"unbound","expectedGeneration":1,"expectedRevision":1,"instanceId":"instanceB"}"#,
        _ => panic!("missing complete test request: {request_id}"),
    }
}

#[test]
fn permission_intent_is_persisted_validated_and_never_defaulted() {
    fixture(|db, owner| {
        let initial = create(db, NativeOrigin::user(owner), CreateSeat {
            domain_id: "projectA", seat_id: "lead", template_id: "templateA",
            instance_id: Some("instanceA"), kind: Kind::Long,
            request_id: "createLead", request_bytes: wire("createLead"),
        }).unwrap().seat;
        assert!(matches!(permission_tier(&initial), Err(SeatError::Denied)));
        for invalid in [br#"{"permissionTier":true}"#.as_slice(),
            br#"{"permissionTier":"FULL_USER"}"#.as_slice()] {
            assert!(matches!(store_template(db, NativeOrigin::user(owner), StoreTemplate {
                domain_id: "projectA", template_id: "invalidTier", settings_json: invalid,
            }), Err(SeatError::Invalid("permissionTier"))));
        }
        let change = SeatChange { domain_id: "projectA", seat_id: "lead",
            expected_generation: initial.generation, expected_revision: initial.revision,
            request_id: "setPermission", request_bytes: br#"{"setting":"permissionTier","value":"NETWORKED_WRITE"}"# };
        let changed = tune(db, NativeOrigin::user(owner), change,
            "permissionTier", "\"NETWORKED_WRITE\"").unwrap().seat;
        let current = get(db, "projectA", "lead").unwrap().unwrap();
        assert_eq!(current, changed);
        assert_eq!(permission_tier(&current).unwrap(), PermissionTier::NetworkedWrite);
        assert!(matches!(tune(db, NativeOrigin::user(owner), SeatChange {
            domain_id: "projectA", seat_id: "lead",
            expected_generation: current.generation, expected_revision: current.revision,
            request_id: "invalidPermission", request_bytes: br#"{"setting":"permissionTier","value":"FULL_USER"}"#,
        }, "permissionTier", "\"FULL_USER\""), Err(SeatError::Invalid("permissionTier"))));
        assert_eq!(get(db, "projectA", "lead").unwrap().unwrap(), current);
        for (value, expected) in [("READ_ONLY", PermissionTier::ReadOnly),
            ("NO_NETWORK", PermissionTier::NoNetwork), ("ISOLATED_WRITE", PermissionTier::IsolatedWrite),
            ("NETWORKED_WRITE", PermissionTier::NetworkedWrite)] {
            assert_eq!(PermissionTier::from_json(&Json::String(JsonString::from_str(value))).unwrap(), expected);
        }
        let mut reclaimed = current;
        reclaimed.state = State::Reclaimed;
        assert!(matches!(permission_tier(&reclaimed), Err(SeatError::Denied)));
    });
}

#[test]
fn tune_copies_settings_and_keeps_native_cas_replay_and_state_guards() {
    fixture(|db, owner| {
        let make = |db: &mut VerifiedDatabaseConnection<'_>, seat_id: &'static str,
            request_id: &'static str, raw: &'static [u8]| {
            create(db, NativeOrigin::user(owner), CreateSeat {
                domain_id: "projectA", seat_id, template_id: "templateA",
                instance_id: Some("instanceA"), kind: Kind::Long,
                request_id, request_bytes: raw,
            }).unwrap().seat
        };
        let seat_a = make(db, "seatA", "createA",
            br#"{"operation":"create-from-template","requestId":"createA","domainId":"projectA","seatId":"seatA","templateId":"templateA","instanceId":"instanceA","kind":"LONG"}"#);
        let seat_b = make(db, "seatB", "createB",
            br#"{"operation":"create-from-template","requestId":"createB","domainId":"projectA","seatId":"seatB","templateId":"templateA","instanceId":"instanceA","kind":"LONG"}"#);
        let raw = br#"{"operation":"tune","requestId":"tuneA","setting":"instruction","value":"changed"}"#;
        let change = || SeatChange {
            domain_id: "projectA", seat_id: "seatA",
            expected_generation: seat_a.generation, expected_revision: seat_a.revision,
            request_id: "tuneA", request_bytes: raw,
        };
        let tuned = tune(db, NativeOrigin::user(owner), change(),
            "instruction", "\"changed\"").unwrap();
        assert!(!tuned.replayed);
        assert_eq!(tuned.seat.settings_json.as_deref(), Some(r#"{"instruction":"changed"}"#));
        assert_eq!(get(db, "projectA", "seatB").unwrap().unwrap().settings_json, seat_b.settings_json);
        let replay = tune(db, NativeOrigin::user(owner), change(),
            "instruction", "\"changed\"").unwrap();
        assert!(replay.replayed);
        assert!(matches!(tune(db, NativeOrigin::user(owner), SeatChange {
            request_bytes: br#"{"operation":"tune","requestId":"tuneA","setting":"instruction","value":"different"}"#,
            ..change()
        }, "instruction", "\"different\""), Err(SeatError::Conflict)));
        assert!(matches!(tune(db, NativeOrigin::user(owner), SeatChange {
            request_id: "staleTune", request_bytes: b"staleTune", ..change()
        }, "instruction", "\"late\""), Err(SeatError::Conflict)));
        let busy = set_dispatch_state(db, &tuned.seat, true).unwrap();
        assert!(matches!(tune(db, NativeOrigin::user(owner), SeatChange {
            expected_generation: busy.generation, expected_revision: busy.revision,
            request_id: "busyTune", request_bytes: b"busyTune", ..change()
        }, "instruction", "\"busy\""), Err(SeatError::Busy)));
        let idle = set_dispatch_state(db, &busy, false).unwrap();
        let reclaimed = reclaim(db, NativeOrigin::user(owner), SeatChange {
            domain_id: "projectA", seat_id: "seatA", expected_generation: idle.generation,
            expected_revision: idle.revision, request_id: "reclaimTune",
            request_bytes: b"reclaimTune",
        }).unwrap().seat;
        assert!(matches!(tune(db, NativeOrigin::user(owner), SeatChange {
            domain_id: "projectA", seat_id: "seatA", expected_generation: reclaimed.generation,
            expected_revision: reclaimed.revision, request_id: "afterReclaimTune",
            request_bytes: b"afterReclaimTune",
        }, "instruction", "\"forbidden\""), Err(SeatError::Denied)));
    });
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

const E2_SETTINGS: &[u8] = br#"{"instruction":"default","model":"modelA","orchestrationScope":{"instanceIds":["instanceA","instanceB"],"maxConcurrent":4,"maxPermissionTier":"NETWORKED_WRITE","models":["modelA"],"reasoningEfforts":["high"]},"permissionTier":"NETWORKED_WRITE","reasoningEffort":"high","takeoverQuestions":[{"id":"q","prompt":"What is the project scope?"}]}"#;

fn create_e2_lead(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer)->Seat {
    store_template(db,NativeOrigin::user(owner),StoreTemplate {domain_id:"projectA",
        template_id:"templateE2",settings_json:E2_SETTINGS}).unwrap();
    create(db,NativeOrigin::user(owner),CreateSeat {domain_id:"projectA",seat_id:"lead",
        template_id:"templateE2",instance_id:Some("instanceA"),kind:Kind::Long,
        request_id:"createLead",request_bytes:wire("createLead")}).unwrap().seat
}

fn set_verified_models(db:&VerifiedDatabaseConnection<'_>, instance_id:&str,
    models_json:&str, digest:&str) {
    let write=Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_instance_evidence(instance_id,available_models_json,models_source,models_observed_at,models_program_digest) VALUES(?1,?2,'codex-model/list:OBSERVED:fixture','100',?3) ON CONFLICT(instance_id) DO UPDATE SET available_models_json=excluded.available_models_json,models_source=excluded.models_source,models_observed_at=excluded.models_observed_at,models_program_digest=excluded.models_program_digest").unwrap();
    write.bind_text(1,instance_id).unwrap();
    write.bind_text(2,models_json).unwrap();
    write.bind_text(3,digest).unwrap();
    write.step_done().unwrap();
}

fn secretary_fact(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer)
    ->SecretaryConfiguration {
    db.execute("BEGIN IMMEDIATE").unwrap();
    let fact=read_secretary_configuration_in_transaction(db,owner).unwrap();
    db.execute("COMMIT").unwrap();
    fact
}

#[test]
fn owner_secretary_designation_is_singleton_and_settings_remain_explicit() {
    fixture(|db,owner| {
        assert_eq!(secretary_fact(db,owner),SecretaryConfiguration::Unset);
        store_template(db,NativeOrigin::user(owner),StoreTemplate {domain_id:"global",
            template_id:"secretaryBase",settings_json:br#"{}"#}).unwrap();
        let seat=create(db,NativeOrigin::user(owner),CreateSeat {domain_id:"global",
            seat_id:"globalSeatA",template_id:"secretaryBase",instance_id:None,
            kind:Kind::Long,request_id:"createGlobalA",request_bytes:b"create global A"})
            .unwrap().seat;
        let first=designate_secretary(db,owner,&seat.seat_id,&seat.incarnation,
            "designateA",b"original designation bytes").unwrap();
        assert!(!first.replayed);
        assert!(designate_secretary(db,owner,&seat.seat_id,&seat.incarnation,
            "designateA",b"original designation bytes").unwrap().replayed);
        let other=create(db,NativeOrigin::user(owner),CreateSeat {domain_id:"global",
            seat_id:"secretary",template_id:"secretaryBase",instance_id:None,
            kind:Kind::Long,request_id:"createGlobalB",request_bytes:b"create global B"})
            .unwrap().seat;
        assert!(matches!(designate_secretary(db,owner,&other.seat_id,&other.incarnation,
            "designateB",b"different global seat"),Err(SeatError::Conflict)));
        assert!(matches!(designate_secretary(db,owner,&seat.seat_id,&seat.incarnation,
            "designateA",b"changed bytes"),Err(SeatError::Conflict)));
        assert!(matches!(secretary_fact(db,owner),SecretaryConfiguration::Designated {
            instance_id:None,model:None,effort:None,permission:None,..}));
        assert!(matches!(configure_secretary(db,owner,seat.generation,seat.revision,
            "configA",b"original config bytes","instanceA","modelA","high",
            "\"READ_ONLY\""),Err(SeatError::Denied)));
        db.execute("INSERT INTO main.gogoke_v37_instance_profiles(instance_id,display_name,enabled,tombstoned,revision) VALUES('instanceA','A',1,0,1)").unwrap();
        set_verified_models(db,"instanceA",r#"["modelA"]"#,"sha256:test");
        let configured=configure_secretary(db,owner,seat.generation,seat.revision,
            "configA",b"original config bytes","instanceA","modelA","high",
            "\"READ_ONLY\"").unwrap();
        assert!(!configured.replayed);
        assert_eq!(configured.seat.instance_id,"instanceA");
        assert!(matches!(secretary_fact(db,owner),SecretaryConfiguration::Designated {
            instance_id:Some(ref id),model:Some(ref model),effort:Some(ref effort),
            permission:Some(PermissionTier::ReadOnly),..}
            if id=="instanceA"&&model=="modelA"&&effort=="high"));
        assert!(configure_secretary(db,owner,seat.generation,seat.revision,
            "configA",b"original config bytes","instanceA","modelA","high",
            "\"READ_ONLY\"").unwrap().replayed);
        assert!(matches!(configure_secretary(db,owner,seat.generation,seat.revision,
            "configA",b"changed config bytes","instanceA","modelA","high",
            "\"READ_ONLY\""),Err(SeatError::Conflict)));
        assert!(matches!(tune(db,NativeOrigin::user(owner),SeatChange {domain_id:"global",
            seat_id:&seat.seat_id,expected_generation:configured.seat.generation,
            expected_revision:configured.seat.revision,request_id:"bypassTune",
            request_bytes:b"direct tune"},"model","\"unverified\""),Err(SeatError::Denied)));
        let revoked=reclaim(db,NativeOrigin::user(owner),SeatChange {domain_id:"global",
            seat_id:&seat.seat_id,expected_generation:configured.seat.generation,
            expected_revision:configured.seat.revision,request_id:"revokeSecretary",
            request_bytes:b"revoke secretary"}).unwrap().seat;
        assert_eq!(revoked.state,State::Reclaimed);
        assert_eq!(secretary_fact(db,owner),SecretaryConfiguration::Revoked);
        assert!(matches!(configure_secretary(db,owner,seat.generation,seat.revision,
            "configA",b"original config bytes","instanceA","modelA","high",
            "\"READ_ONLY\""),Err(SeatError::Conflict)|Err(SeatError::Denied)));
    });
}

#[test]
fn secretary_config_refuses_unreleased_h_occupation() {
    fixture(|db,owner| {
        store_template(db,NativeOrigin::user(owner),StoreTemplate {domain_id:"global",
            template_id:"secretaryBase",settings_json:br#"{}"#}).unwrap();
        let seat=create(db,NativeOrigin::user(owner),CreateSeat {domain_id:"global",
            seat_id:"globalSeatA",template_id:"secretaryBase",instance_id:None,
            kind:Kind::Long,request_id:"createGlobalA",request_bytes:b"create global A"})
            .unwrap().seat;
        designate_secretary(db,owner,&seat.seat_id,&seat.incarnation,
            "designateA",b"original designation bytes").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_instance_profiles(instance_id,display_name,enabled,tombstoned,revision) VALUES('instanceA','A',1,0,1)").unwrap();
        set_verified_models(db,"instanceA",r#"["modelA"]"#,"sha256:test");
        db.execute("CREATE TABLE gogoke_v37_h_claim(domain_id TEXT,session_id TEXT,state TEXT,instance_id TEXT)").unwrap();
        db.execute("CREATE TABLE gogoke_v37_h_seat_binding(domain_id TEXT,session_id TEXT,seat_id TEXT,seat_incarnation TEXT)").unwrap();
        let insert=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_h_seat_binding VALUES('global','sessionA','globalSeatA',?1)").unwrap();
        insert.bind_text(1,&seat.incarnation).unwrap();insert.step_done().unwrap();
        db.execute("INSERT INTO gogoke_v37_h_claim VALUES('global','sessionA','STOPPED','instanceA')").unwrap();
        assert!(matches!(configure_secretary(db,owner,seat.generation,seat.revision,
            "configBusy",b"config while stopped but unreleased","instanceA","modelA",
            "high","\"READ_ONLY\""),Err(SeatError::Busy)));
        assert!(matches!(secretary_fact(db,owner),SecretaryConfiguration::Designated {
            instance_id:None,model:None,effort:None,permission:None,..}));
    });
}

#[test]
fn secretary_routines_replay_absence_and_unknown_are_fail_closed() {
    fixture(|db,owner| {
        store_template(db,NativeOrigin::user(owner),StoreTemplate {domain_id:"global",
            template_id:"secretaryRoutineBase",settings_json:br#"{}"#}).unwrap();
        let seat=create(db,NativeOrigin::user(owner),CreateSeat {domain_id:"global",
            seat_id:"routineSecretary",template_id:"secretaryRoutineBase",instance_id:None,
            kind:Kind::Long,request_id:"createRoutineSeat",request_bytes:b"create routine seat"})
            .unwrap().seat;
        designate_secretary(db,owner,&seat.seat_id,&seat.incarnation,
            "designateRoutineSeat",b"designate routine seat").unwrap();
        let create_input=|raw:&'static [u8]| SecretaryRoutineCreate {
            routine_id:"routineA",request_id:"routineCreate",request_bytes:raw,
            original_text:"每天九点提醒我",source_operation_id:"inputA",source_epoch:"epochA",
            source_cursor:"1",schedule_raw:"每天九点",timezone:"Asia/Shanghai",next_due_ms:100,now_ms:50,
        };
        let (first,replayed)=create_secretary_routine(db,owner,create_input(b"original request")).unwrap();
        assert!(!replayed);assert_eq!(first.original_text,"每天九点提醒我");
        assert!(create_secretary_routine(db,owner,create_input(b"original request")).unwrap().1);
        assert!(matches!(create_secretary_routine(db,owner,create_input(b"changed request")),Err(SeatError::Conflict)));
        let create_b=||SecretaryRoutineCreate {
            routine_id:"routineB",request_id:"routineBCreate",request_bytes:b"create B",
            original_text:"每天十点汇总",source_operation_id:"inputB",source_epoch:"epochA",
            source_cursor:"2",schedule_raw:"每天十点",timezone:"Asia/Shanghai",
            next_due_ms:100,now_ms:50,
        };
        let (other,_)=create_secretary_routine(db,owner,create_b()).unwrap();
        let pause_b=||SecretaryRoutineChange {routine_id:"routineB",expected_revision:1,
            request_id:"pauseB",request_bytes:b"pause B",command:SecretaryRoutineCommand::Pause,
            next_due_ms:None,now_ms:60};
        assert_eq!(other.revision,1);
        let (paused_b,_)=change_secretary_routine(db,owner,pause_b()).unwrap();
        assert_eq!(paused_b.state,"PAUSED");
        assert!(change_secretary_routine(db,owner,pause_b()).unwrap().1);
        let (resumed_b,_)=change_secretary_routine(db,owner,SecretaryRoutineChange {
            routine_id:"routineB",expected_revision:2,request_id:"resumeB",
            request_bytes:b"resume B",command:SecretaryRoutineCommand::Resume,
            next_due_ms:Some(200),now_ms:120}).unwrap();
        assert_eq!(resumed_b.state,"ACTIVE");
        assert!(matches!(change_secretary_routine(db,owner,pause_b()),Err(SeatError::Conflict)));
        assert!(matches!(create_secretary_routine(db,owner,create_b()),Err(SeatError::Conflict)));
        db.execute("BEGIN IMMEDIATE").unwrap();
        assert_eq!(take_due_secretary_routine_in_transaction(db,owner,"routineA",1,100).unwrap(),
            SecretaryRoutineDecision::MissingFacts);
        db.execute("COMMIT").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_instance_profiles(instance_id,display_name,enabled,tombstoned,revision) VALUES('instanceA','A',1,0,1)").unwrap();
        set_verified_models(db,"instanceA",r#"["modelA"]"#,"sha256:test");
        configure_secretary(db,owner,seat.generation,seat.revision,"configureRoutineSeat",
            b"configure routine seat","instanceA","modelA","high","\"READ_ONLY\"").unwrap();
        // Real designation records an explicit product default. The USER
        // replaces only that observed revision; replay must not restore it.
        db.execute("BEGIN IMMEDIATE").unwrap();
        let default_policy=read_secretary_presence_in_transaction(db,owner).unwrap().1.unwrap();
        db.execute("COMMIT").unwrap();
        assert_eq!(default_policy.max_absent_ms,86400000);
        configure_absence_policy(db,owner,Some(1),20,"policyInputA").unwrap();
        designate_secretary(db,owner,&seat.seat_id,&seat.incarnation,
            "designateRoutineSeat",b"designate routine seat").unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let updated_policy=read_secretary_presence_in_transaction(db,owner).unwrap().1.unwrap();
        db.execute("COMMIT").unwrap();
        assert_eq!(updated_policy.max_absent_ms,20);
        record_user_presence(db,owner,"presenceA",UserPresenceKind::Input,
            "userInputA","epochA","2",90,90).unwrap();
        assert!(record_user_presence(db,owner,"presenceA",UserPresenceKind::Input,
            "userInputA","epochA","2",90,90).unwrap());
        assert!(matches!(record_user_presence(db,owner,"presenceA",UserPresenceKind::Input,
            "userInputA","epochA","2",91,91),Err(SeatError::Conflict)));
        db.execute("BEGIN IMMEDIATE").unwrap();
        let (presence,policy)=read_secretary_presence_in_transaction(db,owner).unwrap();
        db.execute("COMMIT").unwrap();
        assert_eq!(presence.unwrap().occurred_at_ms,90);
        assert_eq!(policy.unwrap().max_absent_ms,20);
        db.execute("SAVEPOINT future_user_clock").unwrap();
        record_user_presence_in_transaction(db,owner,"futureClock",UserPresenceKind::Input,
            "futureInput","futureEpoch","1",120,120).unwrap();
        assert_eq!(take_due_secretary_routine_in_transaction(db,owner,"routineA",1,111).unwrap(),
            SecretaryRoutineDecision::MissingFacts);
        db.execute("ROLLBACK TO future_user_clock").unwrap();
        db.execute("RELEASE future_user_clock").unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        assert_eq!(take_due_secretary_routine_in_transaction(db,owner,"routineA",1,111).unwrap(),
            SecretaryRoutineDecision::PausedForAbsence {revision:2,elapsed_ms:21});
        db.execute("COMMIT").unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        assert_eq!(take_due_secretary_routine_in_transaction(db,owner,"routineA",2,112).unwrap(),
            SecretaryRoutineDecision::NotDue);
        db.execute("COMMIT").unwrap();
        assert!(matches!(change_secretary_routine(db,owner,SecretaryRoutineChange {
            routine_id:"routineA",expected_revision:1,request_id:"staleResume",
            request_bytes:b"stale resume",command:SecretaryRoutineCommand::Resume,
            next_due_ms:Some(200),now_ms:120}),Err(SeatError::Conflict)));
        let (resumed,_)=change_secretary_routine(db,owner,SecretaryRoutineChange {
            routine_id:"routineA",expected_revision:2,request_id:"resumeA",
            request_bytes:b"resume A",command:SecretaryRoutineCommand::Resume,
            next_due_ms:Some(200),now_ms:120}).unwrap();
        assert_eq!(resumed.state,"ACTIVE");
        record_user_presence(db,owner,"presenceB",UserPresenceKind::Foreground,
            "userForegroundB","epochA","3",195,195).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let reserved=take_due_secretary_routine_in_transaction(db,owner,"routineA",3,200).unwrap();
        let SecretaryRoutineDecision::Reserved {occurrence_id,revision}=reserved else {panic!("expected reserved");};
        assert_eq!(revision,4);
        assert!(valid_id(&occurrence_id));
        assert!(occurrence_id.starts_with("occ-"));
        db.execute("COMMIT").unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        assert!(matches!(take_due_secretary_routine_in_transaction(db,owner,"routineA",4,201),
            Err(SeatError::Denied)));
        db.execute("COMMIT").unwrap();
        let (paused_a,_)=change_secretary_routine(db,owner,SecretaryRoutineChange {
            routine_id:"routineA",expected_revision:4,request_id:"pausePendingA",
            request_bytes:b"pause pending A",command:SecretaryRoutineCommand::Pause,
            next_due_ms:None,now_ms:201}).unwrap();
        assert_eq!(paused_a.state,"PAUSED");
        let pending_id=paused_a.last_occurrence_id.clone();
        assert!(matches!(change_secretary_routine(db,owner,SecretaryRoutineChange {
            routine_id:"routineA",expected_revision:5,request_id:"resumePendingA",
            request_bytes:b"resume pending A",command:SecretaryRoutineCommand::Resume,
            next_due_ms:Some(300),now_ms:202}),Err(SeatError::Denied)));
        db.execute("BEGIN IMMEDIATE").unwrap();
        assert!(matches!(take_due_secretary_routine_in_transaction(db,owner,"routineA",5,202),
            Err(SeatError::Denied)));
        let unknown=record_secretary_occurrence_outcome_in_transaction(db,owner,"routineA",
            &occurrence_id,5,SecretaryOccurrenceOutcome::Unknown,"","H outcome unknown",None,202).unwrap();
        assert_eq!(unknown.last_result,"UNKNOWN");
        assert_eq!(unknown.state,"PAUSED");
        assert_eq!(unknown.last_occurrence_id,pending_id);
        db.execute("COMMIT").unwrap();
        assert!(matches!(change_secretary_routine(db,owner,SecretaryRoutineChange {
            routine_id:"routineA",expected_revision:6,request_id:"resumeUnknownA",
            request_bytes:b"resume unknown A",command:SecretaryRoutineCommand::Resume,
            next_due_ms:Some(300),now_ms:203}),Err(SeatError::Denied)));
        let (deleted,_)=change_secretary_routine(db,owner,SecretaryRoutineChange {
            routine_id:"routineA",expected_revision:6,request_id:"deleteA",
            request_bytes:b"delete A",command:SecretaryRoutineCommand::Delete,
            next_due_ms:None,now_ms:203}).unwrap();
        assert_eq!(deleted.state,"DELETED");
        db.execute("BEGIN IMMEDIATE").unwrap();
        let settled=record_secretary_occurrence_outcome_in_transaction(db,owner,"routineA",
            &occurrence_id,7,SecretaryOccurrenceOutcome::Failed,"hReceiptA",
            "vendor failure original",None,204).unwrap();
        assert_eq!(settled.state,"DELETED");
        assert_eq!(settled.last_occurrence_id,pending_id);
        let history=read_secretary_routines_in_transaction(db,owner).unwrap();
        let occurrences=read_secretary_occurrences_in_transaction(db,owner,"routineA").unwrap();
        db.execute("COMMIT").unwrap();
        // Both original routine identities were created above; the B replay
        // control must not change A's deletion or original terminal reason.
        assert_eq!(history.len(),2);
        let history_a=history.iter().find(|row|row.routine_id=="routineA").unwrap();
        let history_b=history.iter().find(|row|row.routine_id=="routineB").unwrap();
        assert_eq!(history_a.state,"DELETED");
        assert_eq!(history_a.last_reason,"vendor failure original");
        assert_eq!(history_b.state,"ACTIVE");
        assert_eq!(history_b.revision,3);
        assert_eq!(occurrences.len(),1);
        assert_eq!(occurrences[0].state,"FAILED");
        assert_eq!(occurrences[0].original_reason,"vendor failure original");
        record_user_presence(db,owner,"presenceC",UserPresenceKind::Open,
            "userOpenC","epochA","4",295,295).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let reserved_b=take_due_secretary_routine_in_transaction(db,owner,"routineB",3,300).unwrap();
        let SecretaryRoutineDecision::Reserved {occurrence_id:occurrence_b,revision:revision_b}=reserved_b else {panic!("expected B reservation");};
        assert_eq!(revision_b,4);
        db.execute("COMMIT").unwrap();
        change_secretary_routine(db,owner,SecretaryRoutineChange {
            routine_id:"routineB",expected_revision:4,request_id:"pausePendingB",
            request_bytes:b"pause pending B",command:SecretaryRoutineCommand::Pause,
            next_due_ms:None,now_ms:301}).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let settled_b=record_secretary_occurrence_outcome_in_transaction(db,owner,"routineB",
            &occurrence_b,5,SecretaryOccurrenceOutcome::Failed,"hReceiptB",
            "B original failure",None,302).unwrap();
        db.execute("COMMIT").unwrap();
        assert_eq!(settled_b.state,"PAUSED");
        assert_eq!(settled_b.last_reason,"B original failure");
        let (resumed_b,_)=change_secretary_routine(db,owner,SecretaryRoutineChange {
            routine_id:"routineB",expected_revision:6,request_id:"resumeSettledB",
            request_bytes:b"resume settled B",command:SecretaryRoutineCommand::Resume,
            next_due_ms:Some(400),now_ms:303}).unwrap();
        assert_eq!(resumed_b.state,"ACTIVE");
        record_user_presence(db,owner,"presenceD",UserPresenceKind::Input,
            "userInputD","epochA","5",395,395).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let reserved_b=take_due_secretary_routine_in_transaction(db,owner,"routineB",7,400).unwrap();
        let SecretaryRoutineDecision::Reserved {occurrence_id:second_b,revision:second_revision}=reserved_b else {panic!("expected B second reservation");};
        assert_eq!(second_revision,8);
        let without_next=record_secretary_occurrence_outcome_in_transaction(db,owner,"routineB",
            &second_b,8,SecretaryOccurrenceOutcome::Failed,"hReceiptB2",
            "B second original failure",None,401).unwrap();
        assert_eq!(without_next.state,"WAITING_NEXT");
        assert_eq!(without_next.next_due_ms,0);
        assert_eq!(without_next.last_result,"FAILED");
        assert_eq!(without_next.last_reason,"B second original failure");
        assert_eq!(take_due_secretary_routine_in_transaction(db,owner,"routineB",9,402).unwrap(),
            SecretaryRoutineDecision::NotDue);
        db.execute("COMMIT").unwrap();
        let (explicit_b,_)=change_secretary_routine(db,owner,SecretaryRoutineChange {
            routine_id:"routineB",expected_revision:9,request_id:"resumeNoNextB",
            request_bytes:b"resume B after no next",command:SecretaryRoutineCommand::Resume,
            next_due_ms:Some(500),now_ms:403}).unwrap();
        assert_eq!(explicit_b.state,"ACTIVE");
        assert_eq!(explicit_b.next_due_ms,500);
        let current=get(db,"global","routineSecretary").unwrap().unwrap();
        reclaim(db,NativeOrigin::user(owner),SeatChange {domain_id:"global",
            seat_id:&current.seat_id,expected_generation:current.generation,
            expected_revision:current.revision,request_id:"reclaimRoutineSecretary",
            request_bytes:b"reclaim routine secretary"}).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        assert!(matches!(read_secretary_routines_in_transaction(db,owner),Err(SeatError::Denied)));
        db.execute("COMMIT").unwrap();
    });
}

#[test]
fn configure_instance_commits_binding_and_full_settings_and_replays_exact_snapshot() {
    fixture(|db,owner| {
        let original=create_e2_lead(db,owner);
        set_verified_models(db,"instanceB",r#"["modelB","modelC"]"#,"sha256:test");
        db.execute("INSERT INTO main.gogoke_v37_seat_takeover_answers(domain_id,seat_id,question_id,instance_id,answer,basis,source_ref,how_to_find,revision) VALUES('projectA','lead','q','instanceA','old answer','CITED','repo:old','',1)").unwrap();
        let raw=br#"{"op":"configure-instance","requestId":"configureA","instanceId":"instanceB","model":"modelB","effort":"low","permissionTier":"READ_ONLY"}"#;
        let input=||SeatChange {domain_id:"projectA",seat_id:"lead",
            expected_generation:original.generation,expected_revision:original.revision,
            request_id:"configureA",request_bytes:raw};
        let receipt=configure_instance(db,NativeOrigin::user(owner),input(),
            "instanceB","modelB","low","\"READ_ONLY\"").unwrap();
        assert!(!receipt.replayed);
        assert_eq!(receipt.seat.instance_id,"instanceB");
        assert_eq!(receipt.seat.generation,original.generation+1);
        assert_eq!(receipt.seat.revision,original.revision+1);
        assert_eq!(seat_effort(&receipt.seat).unwrap(),"low");
        assert_eq!(permission_tier(&receipt.seat).unwrap(),PermissionTier::ReadOnly);
        let settings=receipt.seat.settings_json.as_deref().unwrap();
        assert!(settings.contains("\"model\":\"modelB\""));
        assert!(settings.contains("\"instruction\":\"default\""));
        assert!(settings.contains("\"takeoverQuestions\""));
        let Json::Object(settings_object) = Parser::parse(settings).unwrap() else {
            panic!("configured settings must be an object");
        };
        assert!(!settings_object.contains_key(&JsonString::from_str("reasoningEffort")));
        assert_eq!(get(db,"projectA","lead").unwrap().unwrap(),receipt.seat);
        let answers=Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_seat_takeover_answers WHERE domain_id='projectA' AND seat_id='lead'").unwrap();
        assert!(!answers.step_row().unwrap());
        drop(answers);
        let replay=configure_instance(db,NativeOrigin::user(owner),input(),
            "instanceB","modelB","low","\"READ_ONLY\"").unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.seat,receipt.seat);
        assert!(matches!(configure_instance(db,NativeOrigin::user(owner),SeatChange {
            request_bytes:b"different request bytes",..input()
        },"instanceB","modelB","low","\"READ_ONLY\""),Err(SeatError::Conflict)));
        let repair=configure_instance(db,NativeOrigin::user(owner),SeatChange {
            expected_generation:receipt.seat.generation,expected_revision:receipt.seat.revision,
            request_id:"repairSameInstance",request_bytes:b"complete same instance repair",..input()
        },"instanceB","modelC","high","\"NO_NETWORK\"").unwrap().seat;
        assert_eq!(repair.instance_id,"instanceB");
        assert!(repair.settings_json.as_deref().unwrap().contains("\"model\":\"modelC\""));
        assert_eq!(permission_tier(&repair).unwrap(),PermissionTier::NoNetwork);
        assert!(matches!(configure_instance(db,NativeOrigin::user(owner),input(),
            "instanceB","modelB","low","\"READ_ONLY\""),Err(SeatError::Conflict)));
    });
}

#[test]
fn configure_instance_requires_current_verified_model_list_and_preserves_old_state_on_failure() {
    fixture(|db,owner| {
        let before=create_e2_lead(db,owner);
        let input=||SeatChange {domain_id:"projectA",seat_id:"lead",
            expected_generation:before.generation,expected_revision:before.revision,
            request_id:"noEvidence",request_bytes:b"full configuration request"};
        let attempt=|db:&mut VerifiedDatabaseConnection<'_>|
            configure_instance(db,NativeOrigin::user(owner),input(),
                "instanceB","modelB","high","\"READ_ONLY\"");
        assert!(matches!(attempt(db),Err(SeatError::Denied)));
        set_verified_models(db,"instanceB",r#"["modelB"]"#,"sha256:old");
        assert!(matches!(attempt(db),Err(SeatError::Denied)),"old program digest is not current evidence");
        set_verified_models(db,"instanceB",r#"{"model":"modelB"}"#,"sha256:test");
        assert!(matches!(attempt(db),Err(SeatError::Denied)),"non-list evidence must fail closed");
        set_verified_models(db,"instanceB",r#"["another"]"#,"sha256:test");
        assert!(matches!(attempt(db),Err(SeatError::Denied)),"unverified model must fail closed");
        set_verified_models(db,"instanceB",r#"["modelB"]"#,"sha256:test");
        db.execute("UPDATE main.gogoke_v37_instance_evidence SET models_source=NULL WHERE instance_id='instanceB'").unwrap();
        assert!(matches!(attempt(db),Err(SeatError::Denied)),"source-less model list must fail closed");
        set_verified_models(db,"instanceB",r#"["modelB"]"#,"sha256:test");
        db.execute("UPDATE main.gogoke_v37_instances SET login_state='LOGGED_OUT' WHERE instance_id='instanceB'").unwrap();
        assert!(matches!(attempt(db),Err(SeatError::Denied)),"logged-out evidence must fail closed");
        assert_eq!(get(db,"projectA","lead").unwrap().unwrap(),before);
    });
}

#[test]
fn configure_instance_repairs_legacy_partial_change_on_same_instance() {
    fixture(|db,owner| {
        let original=create_e2_lead(db,owner);
        let partial=change_instance(db,NativeOrigin::user(owner),SeatChange {
            domain_id:"projectA",seat_id:"lead",expected_generation:original.generation,
            expected_revision:original.revision,request_id:"legacyPartial",
            request_bytes:b"legacy instance only change",
        },"instanceB").unwrap().seat;
        assert_eq!(partial.instance_id,"instanceB");
        assert!(partial.settings_json.as_deref().unwrap().contains("\"model\":\"modelA\""));
        set_verified_models(db,"instanceB",r#"["modelB"]"#,"sha256:test");
        let restored=configure_instance(db,NativeOrigin::user(owner),SeatChange {
            domain_id:"projectA",seat_id:"lead",expected_generation:partial.generation,
            expected_revision:partial.revision,request_id:"repairPartial",
            request_bytes:b"complete same instance repair",
        },"instanceB","modelB","high","\"READ_ONLY\"").unwrap().seat;
        assert_eq!(restored.instance_id,"instanceB");
        assert!(restored.settings_json.as_deref().unwrap().contains("\"model\":\"modelB\""));
        assert_eq!(permission_tier(&restored).unwrap(),PermissionTier::ReadOnly);
    });
}

#[test]
fn configure_instance_checks_complete_lead_target_scope_and_idle_cas() {
    fixture(|db,owner| {
        let parent=create_e2_lead(db,owner);
        let active=set_dispatch_state(db,&parent,true).unwrap();
        let admission=NativeLeadAdmission::from_native_runtime_snapshot(&active).unwrap();
        let child=create(db,NativeOrigin::lead(&admission),CreateSeat {
            domain_id:"projectA",seat_id:"worker",template_id:"templateE2",
            instance_id:Some("instanceA"),kind:Kind::Short,
            request_id:"createWorker",request_bytes:wire("createWorker"),
        }).unwrap().seat;
        set_verified_models(db,"instanceB",r#"["modelA","modelB"]"#,"sha256:test");
        let input=||SeatChange {domain_id:"projectA",seat_id:"worker",
            expected_generation:child.generation,expected_revision:child.revision,
            request_id:"configureWorker",request_bytes:b"complete child configuration"};
        assert!(matches!(configure_instance(db,NativeOrigin::lead(&admission),input(),
            "instanceB","modelB","high","\"NETWORKED_WRITE\""),Err(SeatError::Denied)));
        assert_eq!(get(db,"projectA","worker").unwrap().unwrap(),child);
        let busy=set_dispatch_state(db,&child,true).unwrap();
        assert!(matches!(configure_instance(db,NativeOrigin::lead(&admission),SeatChange {
            expected_generation:busy.generation,expected_revision:busy.revision,
            request_id:"configureBusyWorker",..input()
        },"instanceB","modelA","high","\"NETWORKED_WRITE\""),Err(SeatError::Busy)));
        let idle=set_dispatch_state(db,&busy,false).unwrap();
        assert!(matches!(configure_instance(db,NativeOrigin::lead(&admission),input(),
            "instanceB","modelA","high","\"NETWORKED_WRITE\""),Err(SeatError::Conflict)));
        let configured=configure_instance(db,NativeOrigin::lead(&admission),SeatChange {
            expected_generation:idle.generation,expected_revision:idle.revision,
            request_id:"configureIdleWorker",..input()
        },"instanceB","modelA","high","\"NETWORKED_WRITE\"").unwrap().seat;
        assert_eq!(configured.instance_id,"instanceB");
        assert_eq!(get(db,"projectA","worker").unwrap().unwrap(),configured);
    });
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
fn measured_host_limit_never_writes_the_owner_project_cap() {
    fixture(|db,owner| {
        assert!(matches!(read_host_parallel_fact(db),Err(SeatError::Denied)));
        set_project_parallel_cap(db,owner,"projectA",4).unwrap();
        assert!(matches!(read_effective_project_parallel_cap(db,"projectA"),
            Err(SeatError::Denied)));
        let fact=transact(db,|db|refresh_host_parallel_fact_in_transaction(db)).unwrap();
        assert!(fact.observed_parallelism>0);
        assert_eq!(fact.machine_limit,fact.observed_parallelism);
        let (effective,recorded)=read_effective_project_parallel_cap(db,"projectA").unwrap();
        assert_eq!(effective,4_i64.min(recorded.machine_limit));
        assert_eq!(read_project_parallel_cap(db,"projectA").unwrap(),4);
    });
}

#[test]
fn project_parallel_cap_requires_explicit_valid_owner_value() {
    fixture(|db, owner| {
        assert!(matches!(read_project_parallel_cap(db, "projectA"), Err(SeatError::Denied)));
        for invalid in [0, -1, i64::MIN] {
            assert!(matches!(
                set_project_parallel_cap(db, owner, "projectA", invalid),
                Err(SeatError::Invalid("project_parallel_cap"))
            ));
        }
        assert!(matches!(read_project_parallel_cap(db, "projectA"), Err(SeatError::Denied)));
        set_project_parallel_cap(db, owner, "projectA", 4).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        assert_eq!(read_project_parallel_cap(db, "projectA").unwrap(), 4);
        db.execute("COMMIT").unwrap();
        assert!(matches!(
            set_project_parallel_cap(db, owner, "projectA", 0),
            Err(SeatError::Invalid("project_parallel_cap"))
        ));
        assert_eq!(read_project_parallel_cap(db, "projectA").unwrap(), 4);
        assert!(matches!(read_project_parallel_cap(db, "projectB"), Err(SeatError::Denied)));
        set_project_parallel_cap(db, owner, "projectA", 2).unwrap();
        assert_eq!(read_project_parallel_cap(db, "projectA").unwrap(), 2);
    });
}

#[test]
fn previous_seat_schema_migrates_without_inventing_a_cap() {
    fixture(|db, owner| {
        let before = create_user(db, owner, "lead", "createLead");
        // Build the actual old schema, including absence of later E.2 tables.
        // Removing only the cap from the current schema creates schema drift.
        let old_tables = previous_schema();
        for (name, _) in expected_schema() {
            if !old_tables.iter().any(|(old, _)| old == &name) {
                db.execute(&format!("DROP TABLE {name}")).unwrap();
            }
        }
        assert_eq!(schema(db).unwrap(), previous_schema());
        initialize_schema(db).unwrap();
        assert_eq!(schema(db).unwrap(), expected_schema());
        assert_eq!(get(db, "projectA", "lead").unwrap(), Some(before));
        assert!(matches!(read_project_parallel_cap(db, "projectA"), Err(SeatError::Denied)));
        assert_eq!(template(db, "projectA", "templateA").unwrap().as_deref(),
            Some("{\"instruction\":\"default\"}"));
        db.execute("CREATE TABLE gogoke_v37_seat_unknown(x INTEGER) STRICT").unwrap();
        assert!(matches!(initialize_schema(db), Err(SeatError::SchemaDrift)));
    });
}

#[test]
fn project_parallel_cap_survives_verified_database_reopen() {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!(
        "gogoke-v37-seat-cap-{}-{nonce}", std::process::id()
    ));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut db = create_new(&root, &database).unwrap();
    db.execute("PRAGMA foreign_keys=ON").unwrap();
    let owner = crate::store::authority::initialize_profile(&mut db, &root).unwrap();
    crate::store::instance::initialize_schema(&mut db).unwrap();
    initialize_schema(&mut db).unwrap();
    set_project_parallel_cap(&mut db, &owner, "projectA", 4).unwrap();
    db.close_checked().unwrap();

    let mut reopened = open_existing(&root, &database).unwrap();
    reopened.execute("PRAGMA foreign_keys=ON").unwrap();
    initialize_schema(&mut reopened).unwrap();
    assert_eq!(read_project_parallel_cap(&reopened, "projectA").unwrap(), 4);
    assert!(matches!(read_project_parallel_cap(&reopened, "projectB"), Err(SeatError::Denied)));
    reopened.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    if let Err(error) = std::fs::remove_dir(&path) {
        eprintln!("owned fixture retained: {} ({error})", path.display());
    }
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
    drop(insert_instance);
    let insert_lead = Statement::prepare(db.as_ptr(), "INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) VALUES('projectA','lead','incarnationA','USER',NULL,'LONG','instanceA','IDLE',1,1)").unwrap();
    insert_lead.step_done().unwrap();
    drop(insert_lead);
    let insert_child = Statement::prepare(db.as_ptr(), "INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) VALUES('projectA','child','incarnationB','LEAD','lead','SHORT','instanceA','IDLE',1,1)").unwrap();
    insert_child.step_done().unwrap();
    drop(insert_child);
    let insert_operation = Statement::prepare(db.as_ptr(), "INSERT INTO gogoke_v37_seat_operations(domain_id,request_id,fingerprint,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,revision,generation) VALUES('projectA','legacyCreate','legacy-fingerprint','lead','incarnationA','USER',NULL,'LONG','instanceA','IDLE',1,1)").unwrap();
    insert_operation.step_done().unwrap();
    drop(insert_operation);

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
    // Keep the close path free of live SQLite statements.  `sqlite3_close`
    // returns BUSY while this readback statement is alive, so the native
    // close ledger has no entry for this generation.
    drop(snapshot_count);
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
fn user_issuer_is_checked_against_current_profile_inside_write_group() {
    fixture(|_db_a, owner_a| {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path_b = std::env::temp_dir().join(format!(
            "gogoke-v37-seat-owner-seam-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path_b).unwrap();
        let root_b = RootLock::acquire(&path_b).unwrap();
        let database_b = path_b.join("state.sqlite");
        let mut db_b = create_new(&root_b, &database_b).unwrap();
        db_b.execute("PRAGMA foreign_keys=ON").unwrap();
        let owner_b = crate::store::authority::initialize_profile(&mut db_b, &root_b).unwrap();
        crate::store::instance::initialize_schema(&mut db_b).unwrap();
        initialize_schema(&mut db_b).unwrap();

        // The seam is deliberately unusable outside an active write group.
        assert!(
            crate::store::authority::check_owner_in_current_transaction(&db_b, &owner_b).is_err()
        );

        // The current owner succeeds in the same write group for both the
        // template publisher and the seat creator.
        store_template(
            &mut db_b,
            NativeOrigin::user(&owner_b),
            StoreTemplate {
                domain_id: "projectA",
                template_id: "templateA",
                settings_json: br#"{"instruction":"default"}"#,
            },
        )
        .unwrap();
        create(
            &mut db_b,
            NativeOrigin::user(&owner_b),
            CreateSeat {
                domain_id: "projectA",
                seat_id: "currentOwner",
                template_id: "templateA",
                instance_id: None,
                kind: Kind::Short,
                request_id: "currentOwnerCreate",
                request_bytes: br#"{"op":"create-from-template","requestId":"currentOwnerCreate","domainId":"projectA","seatId":"currentOwner","templateId":"templateA","kind":"SHORT"}"#,
            },
        )
        .unwrap();

        // An issuer from another verified database is not accepted by Owner
        // user-layer writes, including the project admission cap.
        assert!(matches!(
            set_project_parallel_cap(&mut db_b, owner_a, "projectA", 4),
            Err(SeatError::Denied)
        ));
        assert!(matches!(read_project_parallel_cap(&db_b, "projectA"), Err(SeatError::Denied)));
        assert!(matches!(
            store_template(
                &mut db_b,
                NativeOrigin::user(owner_a),
                StoreTemplate {
                    domain_id: "projectA",
                    template_id: "crossDb",
                    settings_json: br#"{"instruction":"cross"}"#,
                },
            ),
            Err(SeatError::Denied)
        ));
        assert!(matches!(
            create(
                &mut db_b,
                NativeOrigin::user(owner_a),
                CreateSeat {
                    domain_id: "projectA",
                    seat_id: "crossDbSeat",
                    template_id: "templateA",
                    instance_id: None,
                    kind: Kind::Short,
                    request_id: "crossDbCreate",
                    request_bytes: br#"{"op":"create-from-template","requestId":"crossDbCreate","domainId":"projectA","seatId":"crossDbSeat","templateId":"templateA","kind":"SHORT"}"#,
                },
            ),
            Err(SeatError::Denied)
        ));

        // A previously valid issuer becomes stale when the current profile's
        // issuer changes; the write group must reject it before mutation.
        db_b.execute(
            "UPDATE gogoke_authority_profile SET issuer_id='forged-issuer' WHERE singleton=1",
        )
        .unwrap();
        assert!(matches!(
            set_project_parallel_cap(&mut db_b, &owner_b, "projectA", 4),
            Err(SeatError::Denied)
        ));
        assert!(matches!(
            create(
                &mut db_b,
                NativeOrigin::user(&owner_b),
                CreateSeat {
                    domain_id: "projectA",
                    seat_id: "currentOwner",
                    template_id: "templateA",
                    instance_id: None,
                    kind: Kind::Short,
                    request_id: "currentOwnerCreate",
                    request_bytes: br#"{"op":"create-from-template","requestId":"currentOwnerCreate","domainId":"projectA","seatId":"currentOwner","templateId":"templateA","kind":"SHORT"}"#,
                },
            ),
            Err(SeatError::Denied)
        ));
        assert!(matches!(
            store_template(
                &mut db_b,
                NativeOrigin::user(&owner_b),
                StoreTemplate {
                    domain_id: "projectA",
                    template_id: "staleIssuer",
                    settings_json: br#"{"instruction":"stale"}"#,
                },
            ),
            Err(SeatError::Denied)
        ));
        assert!(matches!(
            create(
                &mut db_b,
                NativeOrigin::user(&owner_b),
                CreateSeat {
                    domain_id: "projectA",
                    seat_id: "staleIssuerSeat",
                    template_id: "templateA",
                    instance_id: None,
                    kind: Kind::Short,
                    request_id: "staleIssuerCreate",
                    request_bytes: br#"{"op":"create-from-template","requestId":"staleIssuerCreate","domainId":"projectA","seatId":"staleIssuerSeat","templateId":"templateA","kind":"SHORT"}"#,
                },
            ),
            Err(SeatError::Denied)
        ));
        assert!(get(&db_b, "projectA", "staleIssuerSeat").unwrap().is_none());

        db_b.close_checked().unwrap();
        drop(root_b);
        std::fs::remove_file(database_b).unwrap();
        if let Err(error) = std::fs::remove_dir(&path_b) {
            eprintln!("owned fixture retained: {} ({error})", path_b.display());
        }
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
                expected_revision: 1,
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
fn instance_binding_and_change_require_distinct_operations_and_revision() {
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
                    expected_revision: busy.revision,
                    request_id: "busyBind",
                    request_bytes: wire("busyBind"),
                },
                "instanceB"
            ),
            Err(SeatError::Busy)
        ));
        let idle = set_dispatch_state(db, &busy, false).unwrap();

        // A bound seat cannot be routed through the first-bind operation.
        assert!(matches!(
            bind_instance(
                db,
                NativeOrigin::user(owner),
                SeatChange {
                    domain_id: "projectA",
                    seat_id: "lead",
                    expected_generation: idle.generation,
                    expected_revision: idle.revision,
                    request_id: "boundBind",
                    request_bytes: wire("boundBind"),
                },
                "instanceB"
            ),
            Err(SeatError::Conflict)
        ));

        // The revision CAS is independent of the generation CAS.
        assert!(matches!(
            change_instance(
                db,
                NativeOrigin::user(owner),
                SeatChange {
                    domain_id: "projectA",
                    seat_id: "lead",
                    expected_generation: idle.generation,
                    expected_revision: idle.revision - 1,
                    request_id: "staleRevision",
                    request_bytes: wire("staleRevision"),
                },
                "instanceB"
            ),
            Err(SeatError::Conflict)
        ));
        let changed = change_instance(
            db,
            NativeOrigin::user(owner),
            SeatChange {
                domain_id: "projectA",
                seat_id: "lead",
                expected_generation: idle.generation,
                expected_revision: idle.revision,
                request_id: "changeOnce",
                request_bytes: wire("changeOnce"),
            },
            "instanceB",
        )
        .unwrap();
        assert_eq!(changed.seat.generation, 4);
        assert_eq!(changed.seat.revision, 4);
        assert_eq!(changed.seat.instance_id, "instanceB");

        assert!(matches!(
            bind_instance(
                db,
                NativeOrigin::user(owner),
                SeatChange {
                    domain_id: "projectA",
                    seat_id: "lead",
                    expected_generation: 1,
                    expected_revision: 1,
                    request_id: "staleBind",
                    request_bytes: wire("staleBind"),
                },
                "instanceA"
            ),
            Err(SeatError::Conflict)
        ));
        let replay = change_instance(
            db,
            NativeOrigin::user(owner),
            SeatChange {
                domain_id: "projectA",
                seat_id: "lead",
                expected_generation: idle.generation,
                expected_revision: idle.revision,
                request_id: "changeOnce",
                request_bytes: wire("changeOnce"),
            },
            "instanceB",
        )
        .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.seat, changed.seat);
        assert!(matches!(
            change_instance(
                db,
                NativeOrigin::user(owner),
                SeatChange {
                    domain_id: "projectA",
                    seat_id: "lead",
                    expected_generation: idle.generation,
                    expected_revision: idle.revision,
                    request_id: "changeOnce",
                    request_bytes: br#"{"op":"change-instance","requestId":"changeOnce","domainId":"projectA","seatId":"lead","expectedGeneration":3,"expectedRevision":3,"instanceId":"instanceB","hidden":"payload"}"#,
                },
                "instanceB"
            ),
            Err(SeatError::Conflict)
        ));
        // Reusing a request ID for the other operation identity is also a
        // fingerprint conflict, even when the typed target is otherwise valid.
        assert!(matches!(
            bind_instance(
                db,
                NativeOrigin::user(owner),
                SeatChange {
                    domain_id: "projectA",
                    seat_id: "lead",
                    expected_generation: idle.generation,
                    expected_revision: idle.revision,
                    request_id: "changeOnce",
                    request_bytes: wire("changeOnce"),
                },
                "instanceB"
            ),
            Err(SeatError::Conflict)
        ));
        assert_eq!(get(db, "projectA", "lead").unwrap().unwrap(), changed.seat);
    });
}

#[test]
fn change_instance_rejects_unbound_seat() {
    fixture(|db, owner| {
        let created = create(
            db,
            NativeOrigin::user(owner),
            CreateSeat {
                domain_id: "projectA",
                seat_id: "unboundChange",
                template_id: "templateA",
                instance_id: None,
                kind: Kind::Short,
                request_id: "createUnboundChange",
                request_bytes: br#"{"op":"create-from-template","requestId":"createUnboundChange","domainId":"projectA","seatId":"unboundChange","templateId":"templateA","kind":"SHORT"}"#,
            },
        )
        .unwrap()
        .seat;
        assert_eq!(created.instance_id, "");
        assert!(matches!(
            change_instance(
                db,
                NativeOrigin::user(owner),
                SeatChange {
                    domain_id: "projectA",
                    seat_id: "unboundChange",
                    expected_generation: created.generation,
                    expected_revision: created.revision,
                    request_id: "unboundChange",
                    request_bytes: br#"{"op":"change-instance","requestId":"unboundChange","domainId":"projectA","seatId":"unboundChange","expectedRevision":1,"instanceId":"instanceB"}"#,
                },
                "instanceB"
            ),
            Err(SeatError::Conflict)
        ));
        assert_eq!(
            get(db, "projectA", "unboundChange").unwrap().unwrap(),
            created
        );
    });
}

#[test]
fn lead_only_controls_own_layer_and_reclaim_retains_identity() {
    fixture(|db, owner| {
        let lead = create_e2_lead(db, owner);
        let another = create_user(db, owner, "another", "createAnother");
        let active = set_dispatch_state(db, &lead, true).unwrap();
        let admission = NativeLeadAdmission::from_native_runtime_snapshot(&active).unwrap();
        let worker = create(
            db,
            NativeOrigin::lead(&admission),
            CreateSeat {
                domain_id: "projectA",
                seat_id: "worker",
                template_id: "templateE2",
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
                    expected_revision: another.revision,
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
                    template_id: "templateE2",
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
                expected_revision: worker.revision,
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
                expected_revision: promoted.revision,
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
                    template_id: "templateE2",
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
                    template_id: "templateE2",
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
fn e2_takeover_and_current_policy_grant_are_required_for_child_dispatch() {
    fixture(|db,owner| {
        let lead=create_e2_lead(db,owner);
        let active=set_dispatch_state(db,&lead,true).unwrap();
        let native=NativeLeadAdmission::from_native_runtime_snapshot(&active).unwrap();
        let child=create(db,NativeOrigin::lead(&native),CreateSeat {domain_id:"projectA",
            seat_id:"worker",template_id:"templateE2",instance_id:Some("instanceA"),
            kind:Kind::Short,request_id:"createWorker",request_bytes:wire("createWorker")}).unwrap().seat;
        let caller=NativeSeatCall::from_verified_h_turn(&active,"turnA").unwrap();
        assert!(matches!(authorize_child_dispatch(db,&caller,&child),Err(SeatError::Denied)));
        initialize_policy(db,owner,"projectA","draft").unwrap();
        configure_call_grant(db,owner,"projectA","lead","worker",CallAction::Dispatch,None,1).unwrap();
        assert!(matches!(authorize_child_dispatch(db,&caller,&child),Err(SeatError::Denied)),
            "a grant cannot replace configured takeover answers");
        answer_takeover(db,&caller,"q","Known scope",AnswerBasis::Cited {
            source_ref:"repo:PLAN".into()},0,"answerA",b"original answer").unwrap();
        assert!(takeover_ready(db,&active).unwrap());
        authorize_child_dispatch(db,&caller,&child).unwrap();
        assert!(matches!(authorize_current_call(db,&caller,"projectB","worker",
            CallAction::Dispatch),Err(SeatError::Denied)));
        assert!(matches!(authorize_merge_for_f2(db,&caller,"projectA","worker"),Err(SeatError::Denied)));
        assert!(matches!(authorize_merge_for_f2(db,&caller,"projectB","lead"),Ok(None)));
        assert!(matches!(authorize_merge_for_f2(db,&caller,"projectA","lead"),Err(SeatError::Denied)));
        configure_call_grant(db,owner,"projectA","lead","MAIN",CallAction::Merge,None,2).unwrap();
        assert_eq!(authorize_merge_for_f2(db,&caller,"projectA","lead").unwrap(),
            Some("turnA".into()));
        assert_eq!(authorize_merge_for_f2(db,&caller,"projectA","worker").unwrap(),
            Some("turnA".into()),"reviewer caller may merge another seat's stopped work");
        assert!(matches!(authorize_merge_for_f2(db,&caller,"projectA","missingSource"),
            Err(SeatError::Denied)));
        configure_call_grant(db,owner,"projectA","lead","MAIN",CallAction::Merge,Some(1),3).unwrap();
        assert!(matches!(authorize_merge_for_f2(db,&caller,"projectA","lead"),
            Err(SeatError::Denied)),"expired merge grant cannot be reused");
    });
}

#[test]
fn native_child_create_derives_only_first_exact_dispatch_grant() {
    fixture(|db,owner| {
        let lead=create_e2_lead(db,owner);
        assert_eq!(seat_effort(&lead).unwrap(),"high");
        let active=set_dispatch_state(db,&lead,true).unwrap();
        let caller=NativeSeatCall::from_verified_h_turn(&active,"turnA").unwrap();
        initialize_policy(db,owner,"projectA","draft").unwrap();
        answer_takeover(db,&caller,"q","Known scope",AnswerBasis::Cited {
            source_ref:"repo:PLAN".into()},0,"answerNative",b"original answer").unwrap();
        let input=||CreateSeat {domain_id:"projectA",seat_id:"workerNative",
            template_id:"templateE2",instance_id:Some("instanceA"),kind:Kind::Long,
            request_id:"createNative",request_bytes:b"original native create"};
        let child=create_native_child(db,&caller,input()).unwrap().seat;
        assert_eq!(child.layer,Layer::Lead);
        assert_eq!(child.kind,Kind::Short);
        let expected=fingerprint(&["create","projectA","workerNative","templateE2",
            "instanceA","SHORT","LEAD","lead",&active.incarnation,
            &active.generation.to_string()],b"original native create");
        assert_eq!(operation(db,"projectA","createNative",&expected).unwrap().unwrap().seat,child);
        authorize_child_dispatch(db,&caller,&child).unwrap();
        assert!(matches!(authorize_current_call(db,&caller,"projectA","MAIN",
            CallAction::Dispatch),Err(SeatError::Denied)));
        configure_call_grant(db,owner,"projectA","lead","workerNative",
            CallAction::Dispatch,Some(1),2).unwrap();
        assert!(matches!(authorize_child_dispatch(db,&caller,&child),Err(SeatError::Denied)));
        assert!(create_native_child(db,&caller,input()).unwrap().replayed);
        assert!(matches!(authorize_child_dispatch(db,&caller,&child),Err(SeatError::Denied)),
            "create replay must not restore an expired child grant");
    });
}

#[test]
fn native_child_create_derives_only_first_exact_dispatch_grant_preserves_legacy_kinds() {
    fixture(|db,owner| {
        let lead=create_e2_lead(db,owner);
        let active=set_dispatch_state(db,&lead,true).unwrap();
        let caller=NativeSeatCall::from_verified_h_turn(&active,"turnLegacy").unwrap();
        initialize_policy(db,owner,"projectA","draft").unwrap();
        answer_takeover(db,&caller,"q","Known scope",AnswerBasis::Cited {
            source_ref:"repo:PLAN".into()},0,"answerLegacy",b"original answer").unwrap();
        let admission=NativeLeadAdmission::from_native_runtime_snapshot(&active).unwrap();
        for (seat_id,request_id,kind,raw) in [
            ("legacyLong","legacyLongCreate",Kind::Long,b"old long create".as_slice()),
            ("legacyShort","legacyShortCreate",Kind::Short,b"old short create".as_slice()),
        ] {
            let input=||CreateSeat {domain_id:"projectA",seat_id,template_id:"templateE2",
                instance_id:Some("instanceA"),kind:Kind::Long,request_id,request_bytes:raw};
            let old=create(db,NativeOrigin::lead(&admission),CreateSeat {
                kind,..input()
            }).unwrap().seat;
            assert_eq!(old.kind,kind);
            let stored=fingerprint(&["create","projectA",seat_id,"templateE2",
                "instanceA",kind.sql(),"LEAD","lead",&active.incarnation,
                &active.generation.to_string()],raw);
            assert_eq!(operation(db,"projectA",request_id,&stored).unwrap().unwrap().seat,old);
            let replay=create_native_child(db,&caller,input()).unwrap();
            assert!(replay.replayed);
            assert_eq!(replay.seat,old);
            assert_eq!(operation(db,"projectA",request_id,&stored).unwrap().unwrap().seat,old);
            let grants=Statement::prepare(db.as_ptr(),
                "SELECT COUNT(*) FROM main.gogoke_v37_seat_policy_grants WHERE domain_id='projectA' AND target_id=?1").unwrap();
            grants.bind_text(1,seat_id).unwrap();
            assert!(grants.step_row().unwrap());
            assert_eq!(grants.column_text(0).unwrap(),"0",
                "legacy replay must not derive a new child grant");
        }
        assert!(matches!(create_native_child(db,&caller,CreateSeat {
            request_bytes:b"changed long create",..CreateSeat {domain_id:"projectA",
                seat_id:"legacyLong",template_id:"templateE2",instance_id:Some("instanceA"),
                kind:Kind::Long,request_id:"legacyLongCreate",request_bytes:b"old long create"}
        }),Err(SeatError::Conflict)));
    });
}

#[test]
fn native_child_scope_cap_refuses_a_second_seat_without_minting_a_grant() {
    fixture(|db,owner| {
        let lead=create_e2_lead(db,owner);
        let mut settings=match Parser::parse(lead.settings_json.as_deref().unwrap()).unwrap() {
            Json::Object(settings)=>settings,_=>panic!("original settings object"),
        };
        let Json::Object(scope)=settings.get_mut(&JsonString::from_str("orchestrationScope")).unwrap()
            else {panic!("original scope object")};
        scope.insert(JsonString::from_str("maxConcurrent"),Json::Number("1".into()));
        let value=settings.get(&JsonString::from_str("orchestrationScope")).unwrap().canonical();
        let lead=tune(db,NativeOrigin::user(owner),SeatChange {domain_id:"projectA",
            seat_id:"lead",expected_generation:lead.generation,expected_revision:lead.revision,
            request_id:"limitLead",request_bytes:b"original limit"},"orchestrationScope",&value).unwrap().seat;
        let active=set_dispatch_state(db,&lead,true).unwrap();
        let caller=NativeSeatCall::from_verified_h_turn(&active,"turnCap").unwrap();
        initialize_policy(db,owner,"projectA","draft").unwrap();
        answer_takeover(db,&caller,"q","Known scope",AnswerBasis::Cited {
            source_ref:"repo:PLAN".into()},0,"answerCap",b"original cap answer").unwrap();
        let first=CreateSeat {domain_id:"projectA",seat_id:"capChildA",template_id:"templateE2",
            instance_id:Some("instanceA"),kind:Kind::Short,request_id:"createCapA",request_bytes:b"original cap A"};
        let child=create_native_child(db,&caller,first).unwrap().seat;
        authorize_child_dispatch(db,&caller,&child).unwrap();
        assert!(matches!(create_native_child(db,&caller,CreateSeat {domain_id:"projectA",
            seat_id:"capChildB",template_id:"templateE2",instance_id:Some("instanceA"),kind:Kind::Short,
            request_id:"createCapB",request_bytes:b"original cap B"}),Err(SeatError::Denied)));
        assert!(get(db,"projectA","capChildB").unwrap().is_none());
        let grants=Statement::prepare(db.as_ptr(),
            "SELECT count(*) FROM main.gogoke_v37_seat_policy_grants WHERE domain_id='projectA' AND target_id='capChildB'").unwrap();
        assert!(grants.step_row().unwrap());
        assert_eq!(grants.column_text(0).unwrap(),"0");
    });
}

#[test]
fn native_child_scope_and_policy_head_fail_closed() {
    fixture(|db,owner| {
        let user=create_user(db,owner,"ordinary","createOrdinary");
        let ordinary=set_dispatch_state(db,&user,true).unwrap();
        let caller=NativeSeatCall::from_verified_h_turn(&ordinary,"turnOrdinary").unwrap();
        let input=||CreateSeat {domain_id:"projectA",seat_id:"outside",
            template_id:"templateA",instance_id:Some("instanceA"),kind:Kind::Short,
            request_id:"outsideCreate",request_bytes:b"outside create"};
        assert!(matches!(create_native_child(db,&caller,input()),Err(SeatError::Denied)));
        initialize_policy(db,owner,"projectA","draft").unwrap();
        assert!(matches!(create_native_child(db,&caller,input()),Err(SeatError::Denied)),
            "an ordinary M1 User seat has no orchestration scope");
        assert!(get(db,"projectA","outside").unwrap().is_none());
        let grants=Statement::prepare(db.as_ptr(),
            "SELECT count(*) FROM main.gogoke_v37_seat_policy_grants WHERE domain_id='projectA'").unwrap();
        assert!(grants.step_row().unwrap());
        assert_eq!(grants.column_text(0).unwrap(),"0",
            "ordinary User create cannot grant MAIN or any child");
    });
}

#[test]
fn native_child_create_requires_existing_policy_head() {
    fixture(|db,owner| {
        let lead=create_e2_lead(db,owner);
        let active=set_dispatch_state(db,&lead,true).unwrap();
        let caller=NativeSeatCall::from_verified_h_turn(&active,"turnA").unwrap();
        answer_takeover(db,&caller,"q","Known scope",AnswerBasis::Cited {
            source_ref:"repo:PLAN".into()},0,"answerNoHead",b"original answer").unwrap();
        let result=create_native_child(db,&caller,CreateSeat {domain_id:"projectA",
            seat_id:"noPolicy",template_id:"templateE2",instance_id:Some("instanceA"),
            kind:Kind::Short,request_id:"createNoHead",request_bytes:b"no policy head"});
        assert!(matches!(result,Err(SeatError::Denied)));
        assert!(get(db,"projectA","noPolicy").unwrap().is_none());
    });
}

#[test]
fn conflicting_effort_alias_and_child_model_outside_parent_scope_are_denied() {
    fixture(|db,owner| {
        let conflicting=br#"{"effort":"high","reasoningEffort":"low"}"#;
        assert!(store_template(db,NativeOrigin::user(owner),StoreTemplate {domain_id:"projectA",
            template_id:"badEffort",settings_json:conflicting}).is_err());
        let lead=create_e2_lead(db,owner);
        let active=set_dispatch_state(db,&lead,true).unwrap();
        let caller=NativeSeatCall::from_verified_h_turn(&active,"turnA").unwrap();
        initialize_policy(db,owner,"projectA","draft").unwrap();
        answer_takeover(db,&caller,"q","Known scope",AnswerBasis::Cited {
            source_ref:"repo:PLAN".into()},0,"answerScope",b"original answer").unwrap();
        let child_settings=br#"{"effort":"high","model":"modelB","permissionTier":"NETWORKED_WRITE"}"#;
        store_template(db,NativeOrigin::user(owner),StoreTemplate {domain_id:"projectA",
            template_id:"outsideTemplate",settings_json:child_settings}).unwrap();
        let result=create_native_child(db,&caller,CreateSeat {domain_id:"projectA",
            seat_id:"outsideModel",template_id:"outsideTemplate",instance_id:Some("instanceA"),
            kind:Kind::Short,request_id:"outsideModelCreate",request_bytes:b"outside model"});
        assert!(matches!(result,Err(SeatError::Denied)));
        assert!(get(db,"projectA","outsideModel").unwrap().is_none());
    });
}

#[test]
fn tuned_takeover_question_content_invalidates_old_answer_in_same_transaction() {
    fixture(|db,owner| {
        let lead=create_e2_lead(db,owner);
        let active=set_dispatch_state(db,&lead,true).unwrap();
        let caller=NativeSeatCall::from_verified_h_turn(&active,"turnA").unwrap();
        answer_takeover(db,&caller,"q","Original answer",AnswerBasis::Cited {
            source_ref:"repo:PLAN".into()},0,"answerA",b"original answer").unwrap();
        assert!(takeover_ready(db,&active).unwrap());
        let idle=set_dispatch_state(db,&active,false).unwrap();
        let unrelated=tune(db,NativeOrigin::user(owner),SeatChange {
            domain_id:"projectA",seat_id:"lead",expected_generation:idle.generation,
            expected_revision:idle.revision,request_id:"tuneInstruction",
            request_bytes:b"original instruction tune",
        },"instruction","\"revised\"").unwrap().seat;
        assert!(takeover_ready(db,&unrelated).unwrap(),
            "unrelated copied-setting change preserves cited answers");
        let changed=tune(db,NativeOrigin::user(owner),SeatChange {
            domain_id:"projectA",seat_id:"lead",expected_generation:unrelated.generation,
            expected_revision:unrelated.revision,request_id:"tuneQuestion",
            request_bytes:b"original question tune",
        },"takeoverQuestions",r#"[{"id":"q","prompt":"What changed?"}]"#).unwrap().seat;
        assert!(!takeover_ready(db,&changed).unwrap());
        assert!(read_state_card(db,&changed).unwrap().takeover_answers.is_empty(),
            "same question ID with changed content has no inherited answer");
    });
}

#[test]
fn e2_gate_rejection_stops_stage_and_reserves_one_escalation() {
    fixture(|db,owner| {
        let lead=create_e2_lead(db,owner);
        let reviewer=create_user(db,owner,"reviewer","reviewerCreate");
        let lead=set_dispatch_state(db,&lead,true).unwrap();
        let reviewer=set_dispatch_state(db,&reviewer,true).unwrap();
        let submitter=NativeSeatCall::from_verified_h_turn(&lead,"turnLead").unwrap();
        let auditor=NativeSeatCall::from_verified_h_turn(&reviewer,"turnReview").unwrap();
        assert_eq!(initialize_policy(db,owner,"projectA","draft").unwrap(),1);
        assert_eq!(configure_call_grant(db,owner,"projectA","lead","reviewer",
            CallAction::Review,None,1).unwrap(),2);
        assert_eq!(configure_gate(db,owner,"projectA","gateA","lead","reviewer",
            "draft","done",1,2).unwrap(),3);
        gate_submit(db,&submitter,"gateA",3,1,"submitA",b"original submit").unwrap();
        let rejected=gate_decide(db,&auditor,"gateA",GateDecision::Reject,"needs source",
            3,2,"decideA",b"original decision").unwrap();
        assert_eq!(rejected.state,"ESCALATION_REQUIRED");
        assert!(matches!(stage_transition(db,&submitter,"gateA",3,3,"stageA",
            b"original stage"),Err(SeatError::Denied)));
        configure_escalation_route(db,owner,"projectA","lead","REJECT_CAP","reviewer",3).unwrap();
        assert!(begin_trigger_register(db,&submitter,"triggerA","gateA","triggerRegisterA",
            b"original trigger register",4).unwrap().external_action_authorized);
        let scheduled=NativeCoordinatorTriggerEvidence::from_verified_coordinator("projectA",
            "triggerA","triggerRegisterA","scheduledA",true).unwrap();
        assert_eq!(settle_trigger(db,&scheduled).unwrap().state,"REGISTERED");
        let first=begin_escalation(db,&submitter,EscalationCause::RejectCap {
            gate_id:"gateA".into()},"triggerA","escalateA",b"original escalate",4).unwrap();
        assert_eq!(first.to_seat_id,"reviewer");
        assert!(begin_escalation(db,&submitter,EscalationCause::RejectCap {
            gate_id:"gateA".into()},"triggerA","escalateA",b"original escalate",4).unwrap().replayed);
        mark_escalation_unknown(db,"projectA","triggerA").unwrap();
        let uncertain=begin_escalation(db,&submitter,EscalationCause::RejectCap {
            gate_id:"gateA".into()},"triggerA","escalateA",b"original escalate",4).unwrap();
        assert!(uncertain.replayed && uncertain.state=="UNKNOWN");
        let wrong=NativeDeliveryEvidence::from_verified_c_delivery("projectA","triggerA",
            "otherRequest","reviewer","receiptA").unwrap();
        assert!(matches!(settle_escalation(db,&wrong),Err(SeatError::Denied)));
        let evidence=NativeDeliveryEvidence::from_verified_c_delivery("projectA","triggerA",
            "escalateA","reviewer","receiptA").unwrap();
        assert_eq!(settle_escalation(db,&evidence).unwrap().state,"DELIVERED");
        assert!(settle_escalation(db,&evidence).unwrap().replayed);
        let conflicting=NativeDeliveryEvidence::from_verified_c_delivery("projectA","triggerA",
            "escalateA","reviewer","differentReceipt").unwrap();
        assert!(matches!(settle_escalation(db,&conflicting),Err(SeatError::Unknown)),
            "a second different receipt cannot replace the original");
        let recovered=recover_trigger(db,&submitter,&scheduled,2,"triggerRecoverA",
            b"original trigger recover").unwrap();
        assert_eq!(recovered.revision,3);
        let cancel=begin_trigger_cancel(db,&submitter,"triggerA","triggerCancelA",
            b"original trigger cancel",3,4).unwrap();
        assert!(cancel.external_action_authorized);
        let cancelled=NativeCoordinatorTriggerEvidence::from_verified_coordinator("projectA",
            "triggerA","triggerCancelA","cancelledA",false).unwrap();
        assert_eq!(settle_trigger(db,&cancelled).unwrap().state,"CANCELLED");
        assert!(matches!(recover_trigger(db,&submitter,&scheduled,5,"lateRecover",
            b"late recover"),Err(SeatError::Denied)));
        configure_gate(db,owner,"projectA","gateB","lead","reviewer",
            "draft","done",2,4).unwrap();
        gate_submit(db,&submitter,"gateB",5,1,"submitB",b"original submit B").unwrap();
        assert_eq!(gate_decide(db,&auditor,"gateB",GateDecision::Pass,"",5,2,
            "decideB",b"original decision B").unwrap().state,"PASSED");
        assert_eq!(stage_transition(db,&submitter,"gateB",5,3,"stageB",
            b"original stage B").unwrap().state,"ADVANCED");
        assert_eq!(current_policy_revision(db,&submitter).unwrap(),6);
        assert_eq!(policy_revision_for_native_request(db,&submitter,"gate-submit","submitB").unwrap(),5);
        assert_eq!(policy_revision_for_native_request(db,&submitter,"stage-transition","stageB").unwrap(),5,
            "native replay must use the historical CAS revision after stage advances");
    });
}

#[test]
fn old_lead_change_replay_is_denied_after_admission_generation_changes() {
    fixture(|db, owner| {
        let lead = create_e2_lead(db, owner);
        let active = set_dispatch_state(db, &lead, true).unwrap();
        let admission = NativeLeadAdmission::from_native_runtime_snapshot(&active).unwrap();
        let worker = create(
            db,
            NativeOrigin::lead(&admission),
            CreateSeat {
                domain_id: "projectA",
                seat_id: "worker",
                template_id: "templateE2",
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
            expected_revision: worker.revision,
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
                expected_revision: worker.revision,
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
                    expected_revision: worker.revision,
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
                    expected_revision: worker.revision,
                    request_id: "promoteWorker",
                    request_bytes: wire("promoteWorker"),
                }
            ),
            Err(SeatError::Conflict)
        ));
    });
}
