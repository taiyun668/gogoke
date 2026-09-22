//! Real native Catalog/Context/Route-B tests; definitions are not execution evidence.
use super::bootstrap::{initialize_profile, OwnerIssuer};
use super::catalog::{issue_owner_grant, revise_owner_grant, revoke_owner_grant};
use super::context_read::ContextReadRequest;
use super::context_read_set::read_owner_context_set;
use super::model::{GrantRef, GrantSpec};
use crate::root::RootLock;
use crate::store::context::{apply_context_schema, commit_context_version, ContextCommand};
use crate::store::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, &OwnerIssuer)) {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-context-set-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut db = create_new(&root, &database).unwrap();
    apply_context_schema(&mut db).unwrap();
    let owner = initialize_profile(&mut db, &root).unwrap();
    run(&mut db, &owner);
    db.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    if let Err(error) = std::fs::remove_dir(&path) { eprintln!("owned fixture retained: {error}"); }
}

fn spec(owner: &OwnerIssuer, domain: &str) -> GrantSpec {
    GrantSpec { principal_id: owner.principal_id().into(), seat_id: owner.seat_id().into(),
        permission: "context.read".into(), promotion_kind: "GLOBAL_LESSON".into(),
        source_domain_id: domain.into(), destination_domain_id: "domain-target".into(),
        destination_scope: "GLOBAL".into(), delegable_depth: 0 }
}
fn source(db: &mut VerifiedDatabaseConnection<'_>, domain: &str, id: &str) {
    commit_context_version(db, ContextCommand {
        operation_id: format!("create-{domain}-{id}"), context_id: id.into(), version: "1".into(),
        scope: "PROJECT".into(), domain_id: domain.into(), kind: "fact".into(),
        content_hash: format!("sha256:{}", "a".repeat(64)), source_ref: "source://one".into(),
        source_hash: format!("sha256:{}", "b".repeat(64)), source_authority_kind: "repository".into(),
        source_authority_ref: "authority://one".into(), derived_from: vec![], supersedes: vec![],
        access_policy_revision: "1".into(), visibility: "OWNER_PRIVATE".into(), read_grant_refs: vec![],
        promotion: None,
    }).unwrap();
}
fn request(domain: &str, id: &str, grant: &GrantRef) -> ContextReadRequest {
    ContextReadRequest { source_domain_id: domain.into(), context_id: id.into(), version: "1".into(),
        expected_scope: "PROJECT".into(), expected_content_hash: format!("sha256:{}", "a".repeat(64)),
        expected_access_policy_revision: "1".into(), destination_domain_id: "domain-target".into(),
        destination_scope: "GLOBAL".into(), promotion_kind: "GLOBAL_LESSON".into(),
        policy_revision: "1".into(), grant: grant.clone() }
}
fn pair(db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer) -> Vec<ContextReadRequest> {
    let grant = issue_owner_grant(db, owner, "1", "0", spec(owner, "domain-source")).unwrap();
    source(db, "domain-source", "one"); source(db, "domain-source", "two");
    vec![request("domain-source", "one", &grant), request("domain-source", "two", &grant)]
}

#[test]
fn set_returns_all_sources_with_current_native_catalog_coordinates() {
    fixture(|db, owner| {
        let requests = pair(db, owner);
        let result = read_owner_context_set(db, owner, &requests).unwrap();
        assert_eq!(result.policy_revision, "1"); assert_eq!(result.revocation_head, "0");
        assert_eq!(result.destination_domain_id, "domain-target");
        assert_eq!(result.sources.len(), 2);
        assert_eq!(result.sources[0].context_id, "one"); assert_eq!(result.sources[1].context_id, "two");
        assert_eq!(result, read_owner_context_set(db, owner, &requests).unwrap());
    });
}
#[test]
fn same_context_name_in_two_explicit_source_domains_is_not_collapsed() {
    fixture(|db, owner| {
        let mut requests = Vec::new();
        for domain in ["source-a", "source-b"] {
            source(db, domain, "same");
            let grant = issue_owner_grant(db, owner, "1", "0", spec(owner, domain)).unwrap();
            requests.push(request(domain, "same", &grant));
        }
        let result = read_owner_context_set(db, owner, &requests).unwrap();
        assert_eq!(result.sources.len(), 2);
        assert_eq!(result.sources[0].source_domain_id, "source-a");
        assert_eq!(result.sources[1].source_domain_id, "source-b");
    });
}
#[test]
fn repeated_identity_and_competing_versions_are_rejected_without_selection() {
    fixture(|db, owner| {
        let requests = pair(db, owner);
        assert!(read_owner_context_set(db, owner, &[requests[0].clone(), requests[0].clone()]).is_err());
        let mut different_version = requests[0].clone(); different_version.version = "2".into();
        assert!(read_owner_context_set(db, owner, &[requests[0].clone(), different_version]).is_err());
    });
}
#[test]
fn empty_and_over_limit_sets_fail_without_truncation_but_sixty_four_are_read() {
    fixture(|db, owner| {
        assert!(read_owner_context_set(db, owner, &[]).is_err());
        let grant = issue_owner_grant(db, owner, "1", "0", spec(owner, "domain-source")).unwrap();
        let mut requests = Vec::new();
        for index in 0..65 {
            let id = format!("item-{index}"); source(db, "domain-source", &id);
            requests.push(request("domain-source", &id, &grant));
        }
        assert_eq!(read_owner_context_set(db, owner, &requests[..64]).unwrap().sources.len(), 64);
        assert!(read_owner_context_set(db, owner, &requests).is_err());
    });
}
#[test]
fn mixed_destination_policy_and_revocation_coordinates_fail_independently() {
    fixture(|db, owner| {
        let requests = pair(db, owner);
        let changes: [fn(&mut ContextReadRequest); 5] = [
            |x| x.destination_domain_id = "other-target".into(),
            |x| x.destination_scope = "PROJECT".into(),
            |x| x.promotion_kind = "PROJECT_ONLY".into(),
            |x| x.policy_revision = "2".into(),
            |x| x.grant.revocation_head = "1".into(),
        ];
        for change in changes {
            let mut wrong = requests.clone(); change(&mut wrong[1]);
            assert!(read_owner_context_set(db, owner, &wrong).is_err());
        }
        assert_eq!(read_owner_context_set(db, owner, &requests).unwrap().sources.len(), 2);
    });
}
#[test]
fn inaccessible_last_source_never_returns_a_partial_successful_set() {
    fixture(|db, owner| {
        let requests = pair(db, owner);
        let before = read_owner_context_set(db, owner, &requests).unwrap();
        db.execute("UPDATE gogoke_context_states SET state='STALE' WHERE domain_id='domain-source' AND version_ref='two@1'").unwrap();
        assert!(read_owner_context_set(db, owner, &requests).is_err());
        assert_eq!(before.sources.len(), 2);
        assert_eq!(read_owner_context_set(db, owner, &requests[..1]).unwrap().sources.len(), 1);
    });
}
#[test]
fn every_retry_rechecks_catalog_revision_and_revocation() {
    fixture(|db, owner| {
        let mut requests = pair(db, owner);
        assert!(read_owner_context_set(db, owner, &requests).is_ok());
        let new = revise_owner_grant(db, owner, "1", &requests[0].grant, spec(owner, "domain-source")).unwrap();
        assert!(read_owner_context_set(db, owner, &requests).is_err());
        for request in &mut requests { request.grant = new.clone(); }
        assert!(read_owner_context_set(db, owner, &requests).is_ok());
        let head = revoke_owner_grant(db, owner, "1", &new).unwrap();
        for request in &mut requests { request.grant.revocation_head = head.clone(); }
        assert!(read_owner_context_set(db, owner, &requests).is_err());
    });
}
#[test]
fn every_expected_source_hash_is_checked_not_just_the_first_source() {
    fixture(|db, owner| {
        let requests = pair(db, owner);
        for index in 0..requests.len() {
            let mut wrong = requests.clone(); wrong[index].expected_content_hash = format!("sha256:{}", "c".repeat(64));
            assert!(read_owner_context_set(db, owner, &wrong).is_err());
        }
        assert_eq!(read_owner_context_set(db, owner, &requests).unwrap().sources.len(), 2);
    });
}
