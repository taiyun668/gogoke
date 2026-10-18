//! E-only due qualification with no live H. These rows are synthetic product
//! facts; this fixture claims no USER pipe, provider process, or H delivery.
use super::*;
use crate::root::RootLock;
use crate::store::authority;
use crate::store::same_open::route_b_test_guard;
use std::time::{SystemTime,UNIX_EPOCH};

fn count(db:&VerifiedDatabaseConnection<'_>,table:&str)->i64 {
    let q=Statement::prepare(db.as_ptr(),&format!("SELECT CAST(count(*) AS TEXT) FROM main.{table}"))
        .unwrap();
    assert!(q.step_row().unwrap());
    q.column_text(0).unwrap().parse().unwrap()
}

fn with_qualified_e(action:impl FnOnce(&mut VerifiedDatabaseConnection<'_>,&OwnerIssuer)) {
    let _guard=route_b_test_guard();
    let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let folder=std::env::temp_dir().join(format!("gogoke-secretary-due-e-{}-{nonce}",std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    let root=RootLock::acquire(&folder).unwrap();
    let mut db=crate::store::session::open_product_database(&root,&folder.join("state.sqlite")).unwrap();
    let owner=authority::initialize_profile(&mut db,&root).unwrap();
    db.execute("INSERT INTO gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceE','codex','fixture-home','fixture-home-identity','sha256:fixture','0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
    db.execute("INSERT INTO gogoke_v37_instance_profiles(instance_id,display_name,enabled,tombstoned,revision) VALUES('instanceE','fixture',1,0,1)").unwrap();
    db.execute("INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('global','secretaryE','incarnationE','USER','LONG','instanceE','BUSY',1,1)").unwrap();
    let settings=Parser::parse(r#"{"model":"m","effort":"high","permissionTier":"READ_ONLY"}"#).unwrap().canonical();
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_seat_settings(domain_id,seat_id,template_id,settings_json) VALUES('global','secretaryE','templateE',?1)").unwrap();
    q.bind_text(1,&settings).unwrap();q.step_done().unwrap();drop(q);
    db.execute("INSERT INTO gogoke_v37_seat_secretary VALUES(1,'global','secretaryE','incarnationE','designationE','fixture-fingerprint')").unwrap();
    db.execute("INSERT INTO gogoke_v37_seat_secretary_absence_policy VALUES(1,1,20,'original USER policy')").unwrap();
    db.execute("INSERT INTO gogoke_v37_seat_secretary_routines VALUES('routineE','secretaryE','incarnationE','每天 09:00 提醒我','originalUserOperation','sourceEpoch','1','每天 09:00','UTC',100,'ACTIVE',1,'','NONE','')").unwrap();
    assert_eq!(count(&db,"gogoke_v37_h_stdin_journal"),0);
    action(&mut db,&owner);
    db.close_checked().unwrap();drop(root);
    let target=folder.canonicalize().unwrap();
    let temp=std::env::temp_dir().canonicalize().unwrap();
    assert!(target.starts_with(&temp)&&target!=temp);
    std::fs::remove_dir_all(target).unwrap();
}

fn qualify(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,revision:i64,now:i64)
    ->Result<SecretaryRoutineDueQualification,SeatError> {
    db.execute("BEGIN IMMEDIATE").unwrap();
    let result=qualify_secretary_routine_due_in_transaction(db,owner,"routineE",revision,now);
    if result.is_ok() {db.execute("COMMIT").unwrap();} else {db.execute("ROLLBACK").unwrap();}
    result
}

#[test]
fn absence_qualifies_without_live_h_and_preserves_original_e_facts() {
    with_qualified_e(|db,owner| {
        assert_eq!(qualify(db,owner,1,111).unwrap(),SecretaryRoutineDueQualification::MissingFacts);
        assert_eq!(count(db,"gogoke_v37_seat_secretary_occurrences"),0);
        assert_eq!(count(db,"gogoke_v37_h_stdin_journal"),0);
        db.execute("INSERT INTO gogoke_v37_seat_secretary_presence VALUES('futureUser','INPUT','userOperation','epoch','1',120,120)").unwrap();
        assert_eq!(qualify(db,owner,1,111).unwrap(),SecretaryRoutineDueQualification::MissingFacts);
        assert_eq!(count(db,"gogoke_v37_seat_secretary_occurrences"),0);
        db.execute("DELETE FROM gogoke_v37_seat_secretary_presence WHERE source_id='futureUser'").unwrap();
        db.execute("INSERT INTO gogoke_v37_seat_secretary_presence VALUES('oldUser','INPUT','userOperation','epoch','2',90,90)").unwrap();
        assert_eq!(qualify(db,owner,1,111).unwrap(),
            SecretaryRoutineDueQualification::PausedForAbsence {revision:2,elapsed_ms:21});
        let row=routine(db,"routineE").unwrap().unwrap();
        assert_eq!((row.state.as_str(),row.revision,row.next_due_ms,row.last_result.as_str()),
            ("ABSENCE_PAUSED",2,100,"NONE"));
        assert!(row.last_occurrence_id.is_empty());
        assert_eq!(count(db,"gogoke_v37_seat_secretary_occurrences"),0);
        assert_eq!(count(db,"gogoke_v37_h_stdin_journal"),0);
        assert_eq!(qualify(db,owner,2,112).unwrap(),SecretaryRoutineDueQualification::NotDue,
            "polling cannot automatically resume an absence pause");
        assert!(matches!(qualify(db,owner,1,112),Err(SeatError::Conflict)));
    });
}

#[test]
fn ready_without_h_is_read_only_and_unknown_or_wrong_seat_cannot_replay() {
    with_qualified_e(|db,owner| {
        db.execute("INSERT INTO gogoke_v37_seat_secretary_presence VALUES('recentUser','INPUT','userOperation','epoch','2',95,95)").unwrap();
        assert_eq!(qualify(db,owner,1,100).unwrap(),
            SecretaryRoutineDueQualification::NeedsLiveH {due_ms:100,next_revision:2});
        assert_eq!(routine(db,"routineE").unwrap().unwrap().revision,1);
        assert_eq!(count(db,"gogoke_v37_seat_secretary_occurrences"),0);
        assert_eq!(count(db,"gogoke_v37_h_stdin_journal"),0);
        db.execute("UPDATE gogoke_v37_seat_secretary_routines SET seat_id='foreignSeat' WHERE routine_id='routineE'").unwrap();
        assert!(matches!(qualify(db,owner,1,100),Err(SeatError::Denied)));
        db.execute("UPDATE gogoke_v37_seat_secretary_routines SET seat_id='secretaryE',state='WAITING_NEXT',revision=2,last_occurrence_id='occOld',last_result='UNKNOWN' WHERE routine_id='routineE'").unwrap();
        db.execute("INSERT INTO gogoke_v37_seat_secretary_occurrences VALUES('occOld','routineE',100,'UNKNOWN','','')").unwrap();
        assert!(matches!(qualify(db,owner,2,101),Err(SeatError::Denied)),
            "a prior UNKNOWN occurrence cannot be requalified without H");
        assert_eq!(count(db,"gogoke_v37_seat_secretary_occurrences"),1);
        assert_eq!(count(db,"gogoke_v37_h_stdin_journal"),0);
    });
}
