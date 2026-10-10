//! E read snapshot controls. Synthetic E/F rows do not prove a native launch.
use super::*;
use crate::root::RootLock;
use crate::store::authority;
use crate::store::same_open::route_b_test_guard;
use std::time::{SystemTime,UNIX_EPOCH};

const SETTINGS:&str=r#"{"effort":"high","model":"m","permissionTier":"READ_ONLY"}"#;

fn fixture(action:impl FnOnce(&mut VerifiedDatabaseConnection<'_>,&OwnerIssuer)) {
    let _guard=route_b_test_guard();
    let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let folder=std::env::temp_dir().join(format!("gogoke-secretary-read-{}-{nonce}",std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    let root=RootLock::acquire(&folder).unwrap();
    let mut db=crate::store::session::open_product_database(&root,&folder.join("state.sqlite")).unwrap();
    let owner=authority::initialize_profile(&mut db,&root).unwrap();
    db.execute("INSERT INTO gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceRead','codex','fixture-home','fixture-home-identity','sha256:fixture','0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
    db.execute("INSERT INTO gogoke_v37_instance_profiles(instance_id,display_name,enabled,tombstoned,revision) VALUES('instanceRead','fixture',1,0,1)").unwrap();
    db.execute("INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('global','seatRead','incarnationRead','USER','LONG','instanceRead','BUSY',1,1)").unwrap();
    let settings=Parser::parse(SETTINGS).unwrap().canonical();
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_seat_settings(domain_id,seat_id,template_id,settings_json) VALUES('global','seatRead','templateRead',?1)").unwrap();
    q.bind_text(1,&settings).unwrap();q.step_done().unwrap();drop(q);
    db.execute("INSERT INTO gogoke_v37_seat_secretary VALUES(1,'global','seatRead','incarnationRead','designationRead','fixture-fingerprint')").unwrap();
    action(&mut db,&owner);
    db.close_checked().unwrap();drop(root);
    let target=folder.canonicalize().unwrap();let temp=std::env::temp_dir().canonicalize().unwrap();
    assert!(target.starts_with(&temp)&&target!=temp);
    std::fs::remove_dir_all(target).unwrap();
}

#[test]
fn secretary_read_outside_transaction_returns_complete_e_snapshot() {
    fixture(|db,owner| {
        assert!(autocommit(db));
        let configuration=read_secretary_configuration_in_transaction(db,owner).unwrap();
        let SecretaryConfiguration::Designated {seat_id,incarnation,instance_id,model,effort,
            permission,state,..}=configuration else {panic!("complete designated snapshot")};
        assert_eq!((seat_id.as_str(),incarnation.as_str()),("seatRead","incarnationRead"));
        assert_eq!(instance_id.as_deref(),Some("instanceRead"));
        assert_eq!(model.as_deref(),Some("m"));
        assert_eq!(effort.as_deref(),Some("high"));
        assert_eq!(permission,Some(PermissionTier::ReadOnly));
        assert_eq!(state,State::Busy);
        assert!(autocommit(db));
        let seat=require_secretary_session(db,owner,"seatRead","incarnationRead").unwrap();
        assert_eq!(seat.instance_id,"instanceRead");
        assert_eq!(seat.state,State::Busy);
        assert!(autocommit(db),"owned read commits before returning a seat");
        assert!(matches!(read_secretary_configuration_core_in_transaction(db,owner),Err(SeatError::Denied)));
        assert!(matches!(require_secretary_session_core_in_transaction(db,owner,
            "seatRead","incarnationRead"),Err(SeatError::Denied)));
    });
}

#[test]
fn owned_errors_close_snapshot_and_borrowed_reads_never_end_caller_transaction() {
    fixture(|db,owner| {
        db.execute("UPDATE gogoke_v37_seat_settings SET settings_json='[]' WHERE seat_id='seatRead'").unwrap();
        assert!(matches!(read_secretary_configuration_in_transaction(db,owner),Err(SeatError::SchemaDrift)));
        assert!(autocommit(db),"owned core error must roll back its BEGIN");
        assert!(require_secretary_session(db,owner,"seatRead","incarnationRead").is_err());
        assert!(autocommit(db));
        let q=Statement::prepare(db.as_ptr(),"UPDATE gogoke_v37_seat_settings SET settings_json=?1 WHERE seat_id='seatRead'").unwrap();
        q.bind_text(1,SETTINGS).unwrap();q.step_done().unwrap();drop(q);
        db.execute("UPDATE gogoke_v37_instance_profiles SET enabled=0 WHERE instance_id='instanceRead'").unwrap();
        assert!(matches!(require_secretary_session(db,owner,"seatRead","incarnationRead"),Err(SeatError::Denied)));
        assert!(autocommit(db),"instance eligibility failure cannot leave the owned read open");
        db.execute("UPDATE gogoke_v37_instance_profiles SET enabled=1 WHERE instance_id='instanceRead'").unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        db.execute("UPDATE gogoke_v37_seat_settings SET settings_json='{\"effort\":\"high\",\"model\":\"borrowed\",\"permissionTier\":\"READ_ONLY\"}' WHERE seat_id='seatRead'").unwrap();
        let configuration=read_secretary_configuration_in_transaction(db,owner).unwrap();
        assert!(matches!(configuration,SecretaryConfiguration::Designated {model:Some(value),..} if value=="borrowed"));
        assert!(!autocommit(db),"successful borrowed read must not commit its caller");
        assert!(require_secretary_session(db,owner,"seatRead","incarnationRead").is_ok());
        assert!(!autocommit(db));
        db.execute("UPDATE gogoke_v37_instance_profiles SET enabled=0 WHERE instance_id='instanceRead'").unwrap();
        assert!(matches!(require_secretary_session(db,owner,"seatRead","incarnationRead"),Err(SeatError::Denied)));
        assert!(!autocommit(db),"failed borrowed read must not roll back its caller");
        db.execute("ROLLBACK").unwrap();
        assert!(autocommit(db));
        let recovered=require_secretary_session(db,owner,"seatRead","incarnationRead").unwrap();
        assert_eq!(recovered.instance_id,"instanceRead");
        assert!(matches!(read_secretary_configuration_in_transaction(db,owner),
            Ok(SecretaryConfiguration::Designated {model:Some(value),..}) if value=="m"));
    });
}
