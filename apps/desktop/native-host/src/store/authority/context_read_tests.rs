//! Real native RootLock/Route-B/Grant Catalog tests, not an alternate SQL model.
//! Only actual Windows execution with a positive count is native evidence.
use super::bootstrap::{initialize_profile, OwnerIssuer};
use super::catalog::{delegate_owner_grant, issue_owner_grant, revise_owner_grant, revoke_owner_grant};
use super::context_read::{read_owner_context, ContextReadRequest};
use super::model::{GrantRef, GrantSpec};
use crate::root::RootLock;
use crate::store::context::{apply_context_schema, commit_context_version, ContextCommand};
use crate::store::same_open::{create_new, open_existing, route_b_test_guard, VerifiedDatabaseConnection};
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(run: impl FnOnce(&RootLock, &mut VerifiedDatabaseConnection<'_>, &OwnerIssuer)) {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-context-read-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut connection = create_new(&root, &database).unwrap();
    apply_context_schema(&mut connection).unwrap();
    let owner = initialize_profile(&mut connection, &root).unwrap();
    run(&root, &mut connection, &owner);
    connection.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    if let Err(error) = std::fs::remove_dir(&path) { eprintln!("owned fixture retained: {error}"); }
}

fn spec(owner: &OwnerIssuer, depth: u8) -> GrantSpec {
    GrantSpec { principal_id: owner.principal_id().into(), seat_id: owner.seat_id().into(),
        permission: "context.read".into(), promotion_kind: "GLOBAL_LESSON".into(),
        source_domain_id: "domain-source".into(), destination_domain_id: "domain-target".into(),
        destination_scope: "GLOBAL".into(), delegable_depth: depth }
}

fn source(connection: &mut VerifiedDatabaseConnection<'_>, domain: &str, visibility: &str, refs: Vec<String>) {
    commit_context_version(connection, ContextCommand {
        operation_id: "source-operation".into(), context_id: "source-context".into(), version: "1".into(),
        scope: "PROJECT".into(), domain_id: domain.into(), kind: "fact".into(),
        content_hash: format!("sha256:{}", "a".repeat(64)), source_ref: "source://one".into(),
        source_hash: format!("sha256:{}", "b".repeat(64)), source_authority_kind: "repository".into(),
        source_authority_ref: "authority://one".into(), derived_from: vec![], supersedes: vec![],
        access_policy_revision: "1".into(), visibility: visibility.into(), read_grant_refs: refs,
        promotion: None,
    }).unwrap();
}

fn request(grant: GrantRef) -> ContextReadRequest {
    ContextReadRequest { source_domain_id: "domain-source".into(), context_id: "source-context".into(),
        version: "1".into(), expected_scope: "PROJECT".into(),
        expected_content_hash: format!("sha256:{}", "a".repeat(64)),
        expected_access_policy_revision: "1".into(), destination_domain_id: "domain-target".into(),
        destination_scope: "GLOBAL".into(), promotion_kind: "GLOBAL_LESSON".into(),
        policy_revision: "1".into(), grant }
}

#[test]
fn owner_read_uses_current_native_grant_and_exact_source_metadata() {
    fixture(|_, connection, owner| {
        source(connection, "domain-source", "OWNER_PRIVATE", vec![]);
        let grant = issue_owner_grant(connection, owner, "1", "0", spec(owner, 0)).unwrap();
        let input = request(grant);
        let result = read_owner_context(connection, owner, &input).unwrap();
        assert_eq!(result.source_domain_id, "domain-source");
        assert_eq!(result.context_id, "source-context");
        assert_eq!(result.version, "1");
        assert_eq!(result.content_hash, input.expected_content_hash);
        assert_eq!(result.source_ref, "source://one");
        assert_eq!(result.source_hash, format!("sha256:{}", "b".repeat(64)));
        assert_eq!(result, read_owner_context(connection, owner, &input).unwrap());
    });
}

#[test]
fn missing_grant_never_falls_back_to_owner_id_or_global_destination_scope() {
    fixture(|_, connection, owner| {
        source(connection, "domain-source", "OWNER_PRIVATE", vec![]);
        let absent = GrantRef { grant_id: "grant:absent".into(), revision: "1".into(), revocation_head: "0".into() };
        assert!(read_owner_context(connection, owner, &request(absent)).is_err());
    });
}

#[test]
fn every_read_grant_subject_permission_and_destination_axis_is_compared() {
    fixture(|_, connection, owner| {
        source(connection, "domain-source", "OWNER_PRIVATE", vec![]);
        let changes: [(&str, fn(&mut GrantSpec)); 8] = [
            ("principal", |x| x.principal_id = "owner-other".into()),
            ("seat", |x| x.seat_id = "seat-other".into()),
            ("permission", |x| x.permission = "context.promote.source".into()),
            ("kind", |x| x.promotion_kind = "PROJECT_ONLY".into()),
            ("source-domain", |x| x.source_domain_id = "domain-other".into()),
            ("destination-domain", |x| x.destination_domain_id = "domain-other".into()),
            ("destination-scope", |x| x.destination_scope = "PROJECT".into()),
            ("target-permission", |x| x.permission = "context.promote.target".into()),
        ];
        for (name, change) in changes {
            let mut wrong = spec(owner, 0);
            change(&mut wrong);
            let grant = issue_owner_grant(connection, owner, "1", "0", wrong).unwrap();
            assert!(read_owner_context(connection, owner, &request(grant)).is_err(), "{name}");
        }
    });
}

#[test]
fn grant_revision_and_revocation_are_rechecked_on_every_repeated_read() {
    fixture(|_, connection, owner| {
        source(connection, "domain-source", "OWNER_PRIVATE", vec![]);
        let grant = issue_owner_grant(connection, owner, "1", "0", spec(owner, 0)).unwrap();
        let old = request(grant);
        assert!(read_owner_context(connection, owner, &old).is_ok());
        let new = revise_owner_grant(connection, owner, "1", &old.grant, spec(owner, 0)).unwrap();
        assert!(read_owner_context(connection, owner, &old).is_err());
        let mut input = request(new);
        assert!(read_owner_context(connection, owner, &input).is_ok());
        let head = revoke_owner_grant(connection, owner, "1", &input.grant).unwrap();
        assert!(read_owner_context(connection, owner, &input).is_err());
        input.grant.revocation_head = head;
        assert!(read_owner_context(connection, owner, &input).is_err());
    });
}

#[test]
fn unrelated_revocation_requires_a_fresh_head_even_for_still_valid_read_grant() {
    fixture(|_, connection, owner| {
        source(connection, "domain-source", "OWNER_PRIVATE", vec![]);
        let grant = issue_owner_grant(connection, owner, "1", "0", spec(owner, 0)).unwrap();
        let other = issue_owner_grant(connection, owner, "1", "0", spec(owner, 0)).unwrap();
        let mut input = request(grant);
        let head = revoke_owner_grant(connection, owner, "1", &other).unwrap();
        assert!(read_owner_context(connection, owner, &input).is_err());
        input.grant.revocation_head = head;
        assert!(read_owner_context(connection, owner, &input).is_ok());
    });
}

#[test]
fn delegated_read_rechecks_ancestor_revision_not_just_the_leaf() {
    fixture(|_, connection, owner| {
        source(connection, "domain-source", "OWNER_PRIVATE", vec![]);
        let parent = issue_owner_grant(connection, owner, "1", "0", spec(owner, 2)).unwrap();
        let child = delegate_owner_grant(connection, owner, "1", &parent, spec(owner, 1)).unwrap();
        let input = request(child);
        assert!(read_owner_context(connection, owner, &input).is_ok());
        revise_owner_grant(connection, owner, "1", &parent, spec(owner, 2)).unwrap();
        assert!(read_owner_context(connection, owner, &input).is_err());
    });
}

#[test]
fn all_nonactive_source_states_block_reads_without_returning_stale_metadata() {
    fixture(|_, connection, owner| {
        source(connection, "domain-source", "OWNER_PRIVATE", vec![]);
        let grant = issue_owner_grant(connection, owner, "1", "0", spec(owner, 0)).unwrap();
        let input = request(grant);
        for state in ["SUPERSEDED", "CONFLICTED", "STALE", "REVOKED", "ARCHIVED"] {
            connection.execute(&format!("UPDATE gogoke_context_states SET state='{state}' WHERE domain_id='domain-source'")).unwrap();
            assert!(read_owner_context(connection, owner, &input).is_err(), "{state}");
        }
        connection.execute("UPDATE gogoke_context_states SET state='ACTIVE' WHERE domain_id='domain-source'").unwrap();
        assert!(read_owner_context(connection, owner, &input).is_ok());
    });
}

#[test]
fn content_scope_and_access_revision_mismatches_fail_independently() {
    fixture(|_, connection, owner| {
        source(connection, "domain-source", "OWNER_PRIVATE", vec![]);
        let grant = issue_owner_grant(connection, owner, "1", "0", spec(owner, 0)).unwrap();
        let original = request(grant);
        let changes: [fn(&mut ContextReadRequest); 5] = [
            |x| x.expected_content_hash = format!("sha256:{}", "c".repeat(64)),
            |x| x.expected_access_policy_revision = "2".into(),
            |x| x.expected_scope = "GLOBAL".into(),
            |x| x.version = "2".into(),
            |x| x.policy_revision = "2".into(),
        ];
        for change in changes {
            let mut wrong = original.clone(); change(&mut wrong);
            assert!(read_owner_context(connection, owner, &wrong).is_err());
        }
        assert!(read_owner_context(connection, owner, &original).is_ok());
    });
}

#[test]
fn destination_context_never_substitutes_for_an_absent_source_domain() {
    fixture(|_, connection, owner| {
        source(connection, "domain-target", "OWNER_PRIVATE", vec![]);
        let grant = issue_owner_grant(connection, owner, "1", "0", spec(owner, 0)).unwrap();
        assert!(read_owner_context(connection, owner, &request(grant)).is_err());
    });
}

#[test]
fn source_state_and_access_joins_do_not_borrow_same_named_destination_rows() {
    fixture(|_, connection, owner| {
        source(connection, "domain-source", "OWNER_PRIVATE", vec![]);
        source(connection, "domain-target", "OWNER_PRIVATE", vec![]);
        let grant = issue_owner_grant(connection, owner, "1", "0", spec(owner, 0)).unwrap();
        let input = request(grant);
        connection.execute("DELETE FROM gogoke_context_state_revisions WHERE domain_id='domain-source'").unwrap();
        connection.execute("DELETE FROM gogoke_context_states WHERE domain_id='domain-source'").unwrap();
        assert!(read_owner_context(connection, owner, &input).is_err());
        connection.execute("INSERT INTO gogoke_context_states VALUES('domain-source','source-context@1','ACTIVE')").unwrap();
        connection.execute("INSERT INTO gogoke_context_state_revisions VALUES('domain-source','source-context@1','1')").unwrap();
        connection.execute("DELETE FROM gogoke_context_access WHERE domain_id='domain-source'").unwrap();
        assert!(read_owner_context(connection, owner, &input).is_err());
    });
}

#[test]
fn domain_granted_reads_require_exact_acl_membership_and_recheck_acl_removal() {
    fixture(|_, connection, owner| {
        let grant = issue_owner_grant(connection, owner, "1", "0", spec(owner, 0)).unwrap();
        source(connection, "domain-source", "DOMAIN_GRANTED", vec![grant.grant_id.clone()]);
        let input = request(grant);
        assert!(read_owner_context(connection, owner, &input).is_ok());
        connection.execute("UPDATE gogoke_context_access SET read_grant_refs='' WHERE domain_id='domain-source'").unwrap();
        assert!(read_owner_context(connection, owner, &input).is_err());
        for refs in [format!("{}-suffix", input.grant.grant_id),
            format!("{},{}", input.grant.grant_id, input.grant.grant_id)] {
            connection.execute(&format!("UPDATE gogoke_context_access SET read_grant_refs='{refs}' WHERE domain_id='domain-source'")).unwrap();
            assert!(read_owner_context(connection, owner, &input).is_err());
        }
    });
}

#[test]
fn a_different_native_owner_profile_cannot_reuse_the_read_request() {
    fixture(|root, connection, owner| {
        source(connection, "domain-source", "OWNER_PRIVATE", vec![]);
        let grant = issue_owner_grant(connection, owner, "1", "0", spec(owner, 0)).unwrap();
        let path = root.canonical_root().canonical_path.join("other.sqlite");
        let mut other = create_new(root, &path).unwrap();
        let other_owner = initialize_profile(&mut other, root).unwrap();
        assert!(read_owner_context(connection, &other_owner, &request(grant)).is_err());
        other.close_checked().unwrap(); std::fs::remove_file(path).unwrap();
    });
}

#[test]
fn reopened_database_keeps_grant_and_context_identity_without_cached_authorization() {
    fixture(|root, connection, _owner| {
        // Use a second owned fixture so it can be closed and reopened inside this test.
        let path = root.canonical_root().canonical_path.join("reopen.sqlite");
        let mut other = create_new(root, &path).unwrap();
        apply_context_schema(&mut other).unwrap();
        let actor = initialize_profile(&mut other, root).unwrap();
        source(&mut other, "domain-source", "OWNER_PRIVATE", vec![]);
        let grant = issue_owner_grant(&mut other, &actor, "1", "0", spec(&actor, 0)).unwrap();
        let input = request(grant);
        let before = read_owner_context(&mut other, &actor, &input).unwrap();
        other.close_checked().unwrap();
        let mut reopened = open_existing(root, &path).unwrap();
        let resumed = initialize_profile(&mut reopened, root).unwrap();
        assert_eq!(before, read_owner_context(&mut reopened, &resumed, &input).unwrap());
        assert!(read_owner_context(connection, &resumed, &input).is_err());
        reopened.close_checked().unwrap(); std::fs::remove_file(path).unwrap();
    });
}

#[test]
fn existing_native_source_reference_syntax_is_preserved() {
    fixture(|_, connection, owner| {
        source(connection, "domain-source", "OWNER_PRIVATE", vec![]);
        connection.execute("UPDATE gogoke_context_versions SET source_ref='source://file@revision',source_authority_ref='authority://repository@sha' WHERE domain_id='domain-source'").unwrap();
        let grant = issue_owner_grant(connection, owner, "1", "0", spec(owner, 0)).unwrap();
        let snapshot = read_owner_context(connection, owner, &request(grant)).unwrap();
        assert_eq!(snapshot.source_ref, "source://file@revision");
        assert_eq!(snapshot.source_authority_ref, "authority://repository@sha");
    });
}
