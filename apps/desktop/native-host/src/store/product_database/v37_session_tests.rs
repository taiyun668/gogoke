use super::*;
use crate::root::RootLock;
use crate::store::same_open::route_b_test_guard;
use crate::store::seat::{CreateSeat, Kind, StoreTemplate};
use std::time::{SystemTime, UNIX_EPOCH};

fn request(operation: &str, id: &str, session: &str, seat: &str, revision: u64) -> V37Request {
    decode_request(format!(r#"{{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"{operation}","requestId":"{id}","targetId":"{session}","domainId":"projectA","expectedRevision":"{revision}","payload":{{"seatId":"{seat}","generation":"2"}}}}"#).as_bytes()).unwrap()
}

fn status(product: &mut ProductDatabase<'_>, r: &V37Request) -> V37Status {
    h::decode_receipt(&product.dispatch_user_request(r).unwrap()).unwrap().status
}

#[test]
fn product_admission_enforces_persisted_caps_and_rolls_back_busy_on_denial() {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-v37-product-admission-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    // This controlled observation is used only for storage/admission. No CLI
    // launch, login validity, vendor capability or Win11 result is asserted.
    let binary = path.join("pin-fixture.bin");
    std::fs::write(&binary, b"admission-only native pin fixture").unwrap();
    let pin = instance::ProgramObservation::observe(&binary, "0.149.0").unwrap();
    instance::register_instance(&mut product.connection, &root, &instance::Registration {
        request_id: "regA", request_bytes: b"admission-only registration fixture",
        instance_id: "instanceA", driver_id: "codex", program: &pin,
    }).unwrap();
    for (id, revision, observation) in [
        ("installA", 1, instance::InstanceObservation::Installed),
        ("loginA", 2, instance::InstanceObservation::LoggedIn),
    ] {
        instance::record_observation(&mut product.connection, &root, &instance::ObservationRequest {
            request_id: id, request_bytes: id.as_bytes(), instance_id: "instanceA",
            expected_revision: revision, observation,
        }).unwrap();
    }
    seat::store_template(&mut product.connection, NativeOrigin::user(&product.owner), StoreTemplate {
        domain_id: "projectA", template_id: "templateA", settings_json: b"{}",
    }).unwrap();
    for name in ["seatA", "seatB", "seatC"] {
        seat::create(&mut product.connection, NativeOrigin::user(&product.owner), CreateSeat {
            domain_id: "projectA", seat_id: name, template_id: "templateA", instance_id: Some("instanceA"),
            kind: Kind::Long, request_id: name, request_bytes: name.as_bytes(),
        }).unwrap();
    }
    let reserve_a = request("admission-reserve", "reserveA", "sessionA", "seatA", 0);
    assert_eq!(status(&mut product, &reserve_a), V37Status::Denied);
    seat::set_project_parallel_cap(&mut product.connection, &product.owner, "projectA", 1).unwrap();
    assert_eq!(status(&mut product, &reserve_a), V37Status::Denied);
    instance::set_instance_concurrency_cap(&mut product.connection, &product.owner, "instanceA", 2).unwrap();
    assert_eq!(status(&mut product, &reserve_a), V37Status::Applied);
    assert_eq!(status(&mut product, &reserve_a), V37Status::Replayed);
    let reserve_b = request("admission-reserve", "reserveB", "sessionB", "seatB", 0);
    assert_eq!(status(&mut product, &reserve_b), V37Status::Denied);
    assert_eq!(seat::get(&product.connection, "projectA", "seatB").unwrap().unwrap().state, State::Idle);
    seat::set_project_parallel_cap(&mut product.connection, &product.owner, "projectA", 3).unwrap();
    assert_eq!(status(&mut product, &reserve_b), V37Status::Applied);
    let reserve_c = request("admission-reserve", "reserveC", "sessionC", "seatC", 0);
    assert_eq!(status(&mut product, &reserve_c), V37Status::Denied);
    assert_eq!(seat::get(&product.connection, "projectA", "seatC").unwrap().unwrap().state, State::Idle);
    assert_eq!(status(&mut product, &request("admission-release", "releaseA", "sessionA", "seatA", 1)), V37Status::Applied);
    assert_eq!(seat::get(&product.connection, "projectA", "seatA").unwrap().unwrap().state, State::Idle);
    assert_eq!(status(&mut product, &reserve_c), V37Status::Applied);
    // A peer or caller cannot select another same-instance, same-generation
    // seat to commit this session's reservation.
    assert_eq!(status(&mut product, &request("admission-commit", "wrongSeat", "sessionB", "seatC", 1)), V37Status::Conflict);
    product.close_checked().unwrap();
    let mut reopened = ProductDatabase::open(&root, &database).unwrap();
    assert_eq!(seat::read_project_parallel_cap(&reopened.connection, "projectA").unwrap(), 3);
    assert_eq!(instance::read_instance_concurrency_cap(&reopened.connection, "instanceA").unwrap(), 2);
    assert_eq!(status(&mut reopened, &request("admission-commit", "commitB", "sessionB", "seatB", 1)), V37Status::Applied);
    reopened.close_checked().unwrap();
    drop(root);
    // The paths below are all inside this test-created fixture root.
    std::fs::remove_dir_all(path).unwrap();
}
