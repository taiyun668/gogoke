//! These cases execute the actual Windows RootLock/Route-B/native catalog.
//! They must not be reported as passed on a non-Windows or zero-test run.
use super::bootstrap::{initialize_profile, OwnerIssuer};
use super::catalog::*;
use super::model::{GrantRef, GrantSpec};
use super::promotion::{authorize_owner_promotion, PromotionRequest};
use super::transaction::{self, Result};
use crate::root::RootLock;
use crate::store::context::{apply_context_schema, commit_context_version, ContextCommand};
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::{create_new, open_existing, route_b_test_guard, VerifiedDatabaseConnection};
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(run: impl FnOnce(&RootLock, &mut VerifiedDatabaseConnection<'_>, &OwnerIssuer)) {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-authority-{}-{nonce}", std::process::id()));
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
    if let Err(e) = std::fs::remove_dir(&path) { eprintln!("owned fixture retained: {e}"); }
}

fn spec(owner: &OwnerIssuer, permission: &str, depth: u8) -> GrantSpec {
    GrantSpec { principal_id: owner.principal_id().into(), seat_id: owner.seat_id().into(),
        permission: permission.into(), promotion_kind: "GLOBAL_LESSON".into(),
        source_domain_id: "domain-source".into(), destination_domain_id: "domain-global".into(),
        destination_scope: "GLOBAL".into(), delegable_depth: depth }
}

fn accepted(connection: &mut VerifiedDatabaseConnection<'_>, reference: &GrantRef) -> bool {
    transaction::run(connection, |tx| {
        let profile = current_profile(tx)?;
        resolve_current(tx, &profile, reference).map(|_| ())
    }).is_ok()
}

fn source(connection: &mut VerifiedDatabaseConnection<'_>) {
    commit_context_version(connection, ContextCommand {
        operation_id: "source-operation".into(), context_id: "source-context".into(), version: "1".into(),
        scope: "PROJECT".into(), domain_id: "domain-source".into(), kind: "fact".into(),
        content_hash: format!("sha256:{}", "a".repeat(64)), source_ref: "source://one".into(),
        source_hash: format!("sha256:{}", "b".repeat(64)), source_authority_kind: "repository".into(),
        source_authority_ref: "repository://one".into(), derived_from: vec![], supersedes: vec![],
        access_policy_revision: "1".into(), visibility: "OWNER_PRIVATE".into(), read_grant_refs: vec![], promotion: None,
    }).unwrap();
}

fn promotion(connection: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer) -> PromotionRequest {
    source(connection);
    let source_grant = issue_owner_grant(connection, owner, "1", "0", spec(owner, "context.promote.source", 0)).unwrap();
    let target_grant = issue_owner_grant(connection, owner, "1", "0", spec(owner, "context.promote.target", 0)).unwrap();
    PromotionRequest { source_context_id: "source-context".into(), source_version: "1".into(),
        source_domain_id: "domain-source".into(), source_content_hash: format!("sha256:{}", "a".repeat(64)),
        source_access_policy_revision: "1".into(), destination_domain_id: "domain-global".into(),
        destination_scope: "GLOBAL".into(), promotion_kind: "GLOBAL_LESSON".into(), policy_revision: "1".into(),
        source_grant, target_grant, provenance_refs: vec!["evidence://review".into()] }
}

fn authorize(connection: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer, request: &PromotionRequest) -> Result<()> {
    transaction::run(connection, |tx| authorize_owner_promotion(tx, owner, request))
}

#[test]
fn bootstrap_reuses_persistent_owner_and_creates_no_automatic_grants() {
    fixture(|root, connection, owner| {
        let resumed = initialize_profile(connection, root).unwrap();
        assert_eq!(resumed.principal_id(), owner.principal_id());
        assert_eq!(resumed.seat_id(), owner.seat_id());
        transaction::run(connection, |tx| {
            assert_eq!(tx.query("SELECT count(*) FROM gogoke_authority_grants", &[], 1)?[0][0], "0");
            assert_eq!(tx.query("SELECT count(*) FROM gogoke_authority_events", &[], 1)?[0][0], "1");
            Ok(())
        }).unwrap();
    });
}

#[test]
fn missing_profile_is_not_silently_bootstrapped_again() {
    fixture(|root, connection, _| {
        connection.execute("DELETE FROM gogoke_authority_profile").unwrap();
        assert!(initialize_profile(connection, root).is_err());
    });
}

#[test]
fn unexpected_authority_schema_or_trigger_fails_closed() {
    fixture(|root, connection, _| {
        connection.execute("CREATE TRIGGER forged_authority AFTER INSERT ON gogoke_authority_events BEGIN SELECT 1; END").unwrap();
        assert!(initialize_profile(connection, root).is_err());
    });
}

#[test]
fn owner_capability_cannot_be_reused_in_another_database_profile() {
    fixture(|root, connection, owner| {
        let path = root.canonical_root().canonical_path.join("other.sqlite");
        let mut other = create_new(root, &path).unwrap();
        let other_owner = initialize_profile(&mut other, root).unwrap();
        assert_ne!(owner.principal_id(), other_owner.principal_id());
        assert!(issue_owner_grant(&mut other, owner, "1", "0", spec(owner, "context.read", 0)).is_err());
        assert!(issue_owner_grant(connection, &other_owner, "1", "0", spec(&other_owner, "context.read", 0)).is_err());
        other.close_checked().unwrap();
        std::fs::remove_file(path).unwrap();
    });
}

#[test]
fn grant_revision_changes_current_head_but_preserves_old_revision() {
    fixture(|_, connection, owner| {
        let original = issue_owner_grant(connection, owner, "1", "0", spec(owner, "context.read", 0)).unwrap();
        let revised = revise_owner_grant(connection, owner, "1", &original, spec(owner, "context.read", 0)).unwrap();
        assert_eq!(revised.revision, "2");
        assert!(!accepted(connection, &original));
        assert!(accepted(connection, &revised));
        transaction::run(connection, |tx| {
            assert_eq!(tx.query("SELECT count(*) FROM gogoke_authority_grants WHERE grant_id=?", &[&original.grant_id], 1)?[0][0], "2");
            Ok(())
        }).unwrap();
    });
}

#[test]
fn revocation_invalidates_stale_snapshot_and_revoked_grant_after_refresh() {
    fixture(|_, connection, owner| {
        let original = issue_owner_grant(connection, owner, "1", "0", spec(owner, "context.read", 0)).unwrap();
        let other = issue_owner_grant(connection, owner, "1", "0", spec(owner, "context.read", 0)).unwrap();
        let head = revoke_owner_grant(connection, owner, "1", &original).unwrap();
        assert_eq!(head, "1");
        assert!(!accepted(connection, &original));
        assert!(!accepted(connection, &other));
        assert!(!accepted(connection, &GrantRef { revocation_head: head.clone(), ..original }));
        assert!(accepted(connection, &GrantRef { revocation_head: head, ..other }));
    });
}

#[test]
fn each_delegation_ceiling_axis_is_checked_independently() {
    fixture(|_, connection, owner| {
        let parent = issue_owner_grant(connection, owner, "1", "0", spec(owner, "context.read", 2)).unwrap();
        let mutations: [fn(&mut GrantSpec); 6] = [
            |x| x.permission = "context.promote.source".into(),
            |x| x.promotion_kind = "RULE".into(),
            |x| x.source_domain_id = "source-other".into(),
            |x| x.destination_domain_id = "target-other".into(),
            |x| x.destination_scope = "PROJECT".into(),
            |x| x.delegable_depth = 2,
        ];
        for change in mutations {
            let mut child = spec(owner, "context.read", 1);
            change(&mut child);
            assert!(delegate_owner_grant(connection, owner, "1", &parent, child).is_err());
        }
        let child = delegate_owner_grant(connection, owner, "1", &parent, spec(owner, "context.read", 1)).unwrap();
        assert!(accepted(connection, &child));
    });
}

#[test]
fn cannot_impersonate_another_grantee_to_delegate_their_parent() {
    fixture(|_, connection, owner| {
        let mut other = spec(owner, "context.read", 2);
        other.principal_id = "another-principal".into();
        other.seat_id = "another-seat".into();
        let parent = issue_owner_grant(connection, owner, "1", "0", other).unwrap();
        assert!(delegate_owner_grant(connection, owner, "1", &parent, spec(owner, "context.read", 1)).is_err());
    });
}

#[test]
fn parent_revocation_is_rechecked_even_with_fresh_child_reference() {
    fixture(|_, connection, owner| {
        let parent = issue_owner_grant(connection, owner, "1", "0", spec(owner, "context.read", 2)).unwrap();
        let child = delegate_owner_grant(connection, owner, "1", &parent, spec(owner, "context.read", 1)).unwrap();
        let head = revoke_owner_grant(connection, owner, "1", &parent).unwrap();
        assert!(!accepted(connection, &GrantRef { revocation_head: head, ..child }));
    });
}

#[test]
fn parent_revision_change_invalidates_existing_child_lineage() {
    fixture(|_, connection, owner| {
        let parent = issue_owner_grant(connection, owner, "1", "0", spec(owner, "context.read", 2)).unwrap();
        let child = delegate_owner_grant(connection, owner, "1", &parent, spec(owner, "context.read", 1)).unwrap();
        revise_owner_grant(connection, owner, "1", &parent, spec(owner, "context.read", 2)).unwrap();
        assert!(!accepted(connection, &child));
    });
}

#[test]
fn stale_policy_and_forged_root_issuer_do_not_resolve() {
    fixture(|_, connection, owner| {
        let grant = issue_owner_grant(connection, owner, "1", "0", spec(owner, "context.read", 0)).unwrap();
        connection.execute("UPDATE gogoke_authority_grants SET issuer_id='forged-issuer'").unwrap();
        assert!(!accepted(connection, &grant));
        assert!(issue_owner_grant(connection, owner, "2", "0", spec(owner, "context.read", 0)).is_err());
    });
}

#[test]
fn malformed_or_missing_grant_references_never_authorize() {
    fixture(|_, connection, _| {
        for revision in ["", "01", "-1", "18446744073709551616", "1"] {
            assert!(!accepted(connection, &GrantRef { grant_id: "not-present".into(), revision: revision.into(), revocation_head: "0".into() }));
        }
    });
}

#[test]
fn promotion_checks_real_source_and_both_current_grants() {
    fixture(|_, connection, owner| {
        let request = promotion(connection, owner);
        authorize(connection, owner, &request).unwrap();
        let head = revoke_owner_grant(connection, owner, "1", &request.target_grant).unwrap();
        let mut fresh = request;
        fresh.source_grant.revocation_head = head.clone();
        fresh.target_grant.revocation_head = head;
        assert!(authorize(connection, owner, &fresh).is_err());
    });
}

#[test]
fn promotion_binding_axes_and_source_identity_fail_closed() {
    fixture(|_, connection, owner| {
        let request = promotion(connection, owner);
        let mutations: [fn(&mut PromotionRequest); 9] = [
            |x| x.source_context_id = "other".into(),
            |x| x.source_version = "2".into(),
            |x| x.source_domain_id = "domain-global".into(),
            |x| x.destination_domain_id = "other".into(),
            |x| x.destination_scope = "PROJECT".into(),
            |x| x.source_content_hash = format!("sha256:{}", "c".repeat(64)),
            |x| x.source_access_policy_revision = "2".into(),
            |x| x.promotion_kind = "RULE".into(),
            |x| x.provenance_refs.clear(),
        ];
        for mutate in mutations {
            let mut changed = request.clone();
            mutate(&mut changed);
            assert!(authorize(connection, owner, &changed).is_err());
        }
    });
}

#[test]
fn source_state_is_reread_inside_promotion_transaction() {
    fixture(|_, connection, owner| {
        let request = promotion(connection, owner);
        connection.execute("UPDATE gogoke_context_states SET state='REVOKED'").unwrap();
        assert!(authorize(connection, owner, &request).is_err());
    });
}

#[test]
fn failed_authority_transaction_rolls_back_and_reopen_preserves_committed_grants() {
    // Own the connection in this fixture so close_checked consumes it before
    // acquiring the next same-open pin. Two simultaneous opens are not a reopen.
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-authority-reopen-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut connection = create_new(&root, &database).unwrap();
    apply_context_schema(&mut connection).unwrap();
    let owner = initialize_profile(&mut connection, &root).unwrap();
    let grant = issue_owner_grant(&mut connection, &owner, "1", "0", spec(&owner, "context.read", 0)).unwrap();
    let result: Result<()> = transaction::run(&mut connection, |tx| {
        tx.write("UPDATE gogoke_authority_grant_heads SET revoked=1", &[])?;
        Err(OrchestrationError::AccessDenied)
    });
    assert!(result.is_err());
    assert!(accepted(&mut connection, &grant));
    connection.close_checked().unwrap();
    let mut reopened = open_existing(&root, &database).unwrap();
    assert!(accepted(&mut reopened, &grant));
    let resumed_owner = initialize_profile(&mut reopened, &root).unwrap();
    assert_eq!(resumed_owner.principal_id(), owner.principal_id());
    assert_eq!(resumed_owner.seat_id(), owner.seat_id());
    reopened.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    if let Err(e) = std::fs::remove_dir(&path) { eprintln!("owned fixture retained: {e}"); }
}
