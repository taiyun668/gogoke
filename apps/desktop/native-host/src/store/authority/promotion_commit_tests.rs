//! Native Route-B cases. Definitions alone and non-Windows zero counts are NOT PASS.
use super::bootstrap::{initialize_profile, OwnerIssuer};
use super::catalog::{issue_owner_grant, revoke_owner_grant, revise_owner_grant};
use super::model::GrantSpec;
use super::promotion::PromotionRequest;
use super::promotion_commit::{apply_authorized_promotion, commit_owner_promotion};
use super::transaction::{self, Result};
use crate::root::RootLock;
use crate::store::context::{apply_context_schema, apply_context_version_in_transaction,
    commit_context_version, ContextCommand, PromotionEvidence};
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use std::time::{SystemTime, UNIX_EPOCH};

fn command(id: &str) -> ContextCommand {
    ContextCommand { operation_id: format!("operation-{id}"), context_id: id.into(), version: "1".into(),
        scope: "PROJECT".into(), domain_id: "domain-one".into(), kind: "fact".into(),
        content_hash: format!("sha256:{}", "a".repeat(64)), source_ref: "source://one".into(),
        source_hash: format!("sha256:{}", "b".repeat(64)), source_authority_kind: "repository".into(),
        source_authority_ref: "authority://one".into(), derived_from: vec![], supersedes: vec![],
        access_policy_revision: "1".into(), visibility: "OWNER_PRIVATE".into(),
        read_grant_refs: vec![], promotion: None }
}
fn spec(owner: &OwnerIssuer, permission: &str) -> GrantSpec {
    GrantSpec { principal_id: owner.principal_id().into(), seat_id: owner.seat_id().into(),
        permission: permission.into(), promotion_kind: "GLOBAL_LESSON".into(),
        source_domain_id: "domain-one".into(), destination_domain_id: "domain-one".into(),
        destination_scope: "GLOBAL".into(), delegable_depth: 0 }
}
fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, &OwnerIssuer,
    PromotionRequest, ContextCommand)) {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-promotion-commit-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut connection = create_new(&root, &database).unwrap();
    apply_context_schema(&mut connection).unwrap();
    let owner = initialize_profile(&mut connection, &root).unwrap();
    commit_context_version(&mut connection, command("source")).unwrap();
    let request = PromotionRequest {
        source_context_id: "source".into(), source_version: "1".into(), source_domain_id: "domain-one".into(),
        source_content_hash: format!("sha256:{}", "a".repeat(64)), source_access_policy_revision: "1".into(),
        destination_domain_id: "domain-one".into(), destination_scope: "GLOBAL".into(),
        promotion_kind: "GLOBAL_LESSON".into(), policy_revision: "1".into(),
        source_grant: issue_owner_grant(&mut connection, &owner, "1", "0", spec(&owner, "context.promote.source")).unwrap(),
        target_grant: issue_owner_grant(&mut connection, &owner, "1", "0", spec(&owner, "context.promote.target")).unwrap(),
        provenance_refs: vec!["evidence://review".into()] };
    let mut target = command("global");
    target.scope = "GLOBAL".into();
    target.derived_from = vec!["source@1".into()];
    target.promotion = Some(PromotionEvidence { source_version_ref: "source@1".into(),
        source_grant_ref: request.source_grant.grant_id.clone(),
        target_grant_ref: request.target_grant.grant_id.clone(), provenance_refs: request.provenance_refs.clone() });
    run(&mut connection, &owner, request, target);
    connection.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    if let Err(error) = std::fs::remove_dir(&path) { eprintln!("owned fixture retained: {error}"); }
}
fn query(connection: &mut VerifiedDatabaseConnection<'_>, sql: &str, args: &[&str], columns: i32) -> Vec<Vec<String>> {
    transaction::run(connection, |tx| tx.query(sql, args, columns)).unwrap()
}
fn target_count(connection: &mut VerifiedDatabaseConnection<'_>) -> String {
    query(connection, "SELECT count(*) FROM gogoke_context_versions WHERE scope='GLOBAL'", &[], 1)[0][0].clone()
}

#[test]
fn authorized_commit_and_replay_preserve_explicit_source_and_current_grant_coordinates() {
    fixture(|connection, owner, request, target| {
        let first = commit_owner_promotion(connection, owner, &request, target.clone()).unwrap();
        assert_eq!(first.disposition, "COMMITTED");
        let second = commit_owner_promotion(connection, owner, &request, target).unwrap();
        assert_eq!(second.disposition, "RECONCILED");
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(target_count(connection), "1");
        let rows = query(connection, "SELECT source_domain_id,source_context_id,source_version,source_grant_id,source_grant_revision,target_grant_id,target_grant_revision,policy_revision,revocation_head FROM gogoke_context_promotion_authorizations", &[], 9);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], vec!["domain-one".to_owned(), "source".into(), "1".into(),
            request.source_grant.grant_id, "1".into(), request.target_grant.grant_id, "1".into(), "1".into(), "0".into()]);
    });
}
#[test]
fn each_grant_is_rechecked_before_replay_even_after_refreshing_revocation_head() {
    for source in [true, false] {
        fixture(|connection, owner, mut request, target| {
            commit_owner_promotion(connection, owner, &request, target.clone()).unwrap();
            let revoked = if source { &request.source_grant } else { &request.target_grant };
            let head = revoke_owner_grant(connection, owner, "1", revoked).unwrap();
            assert!(commit_owner_promotion(connection, owner, &request, target.clone()).is_err());
            request.source_grant.revocation_head = head.clone();
            request.target_grant.revocation_head = head;
            assert!(commit_owner_promotion(connection, owner, &request, target).is_err());
            assert_eq!(target_count(connection), "1");
        });
    }
}
#[test]
fn revised_grant_cannot_replay_the_old_authorization_tuple() {
    fixture(|connection, owner, mut request, target| {
        commit_owner_promotion(connection, owner, &request, target.clone()).unwrap();
        let revised = revise_owner_grant(connection, owner, "1", &request.source_grant,
            spec(owner, "context.promote.source")).unwrap();
        assert!(commit_owner_promotion(connection, owner, &request, target.clone()).is_err());
        request.source_grant = revised;
        assert!(commit_owner_promotion(connection, owner, &request, target).is_err());
        assert_eq!(target_count(connection), "1");
    });
}
#[test]
fn source_and_target_current_state_are_rechecked_on_replay() {
    for id in ["source@1", "global@1"] {
        fixture(|connection, owner, request, target| {
            commit_owner_promotion(connection, owner, &request, target.clone()).unwrap();
            transaction::run(connection, |tx| tx.write(
                "UPDATE gogoke_context_states SET state='REVOKED' WHERE domain_id='domain-one' AND version_ref=?", &[id])).unwrap();
            assert!(commit_owner_promotion(connection, owner, &request, target).is_err());
            assert_eq!(target_count(connection), "1");
        });
    }
}
#[test]
fn changed_source_access_revision_cannot_replay_a_previous_authorization_receipt() {
    fixture(|connection, owner, mut request, target| {
        commit_owner_promotion(connection, owner, &request, target.clone()).unwrap();
        connection.execute("UPDATE gogoke_context_versions SET access_policy_revision='2' WHERE context_id='source'").unwrap();
        request.source_access_policy_revision = "2".into();
        assert!(commit_owner_promotion(connection, owner, &request, target).is_err());
        assert_eq!(target_count(connection), "1");
    });
}
#[test]
fn legacy_storage_promotion_cannot_be_upgraded_by_authorized_replay() {
    fixture(|connection, owner, request, target| {
        commit_context_version(connection, target.clone()).unwrap();
        assert!(commit_owner_promotion(connection, owner, &request, target).is_err());
        assert_eq!(target_count(connection), "1");
    });
}
#[test]
fn missing_authorization_receipt_cannot_be_synthesized_from_existing_context() {
    fixture(|connection, owner, request, target| {
        commit_owner_promotion(connection, owner, &request, target.clone()).unwrap();
        connection.execute("DELETE FROM gogoke_context_promotion_authorizations").unwrap();
        assert!(commit_owner_promotion(connection, owner, &request, target).is_err());
        assert_eq!(query(connection, "SELECT count(*) FROM gogoke_context_promotion_authorizations", &[], 1)[0][0], "0");
    });
}
#[test]
fn target_shape_and_provenance_mismatches_do_not_write() {
    let changes: [fn(&mut ContextCommand); 7] = [
        |x| x.domain_id = "other-domain".into(), |x| x.scope = "PROJECT".into(),
        |x| x.visibility = "DOMAIN_GRANTED".into(), |x| x.read_grant_refs.push("grant://forged".into()),
        |x| x.derived_from.clear(), |x| x.supersedes.push("source@1".into()), |x| x.promotion = None,
    ];
    for change in changes {
        fixture(|connection, owner, request, mut target| {
            change(&mut target);
            assert!(commit_owner_promotion(connection, owner, &request, target).is_err());
            assert_eq!(target_count(connection), "0");
        });
    }
}
#[test]
fn explicit_foreign_source_is_not_resolved_in_the_destination_domain() {
    fixture(|connection, owner, mut request, target| {
        request.source_domain_id = "foreign-domain".into();
        assert!(commit_owner_promotion(connection, owner, &request, target).is_err());
        assert_eq!(target_count(connection), "0");
    });
}
#[test]
fn post_write_failure_rolls_back_context_graph_operation_and_authorization_receipt() {
    fixture(|connection, owner, request, target| {
        commit_owner_promotion(connection, owner, &request, target.clone()).unwrap();
        let mut rolled_back = target;
        rolled_back.context_id = "rolled-back".into();
        rolled_back.operation_id = "operation-rolled-back".into();
        let result: Result<()> = transaction::run(connection, |tx| {
            apply_authorized_promotion(tx, owner, &request, rolled_back)?;
            Err(OrchestrationError::Invalid("test fault after complete write group"))
        });
        assert!(result.is_err());
        assert_eq!(target_count(connection), "1");
        assert_eq!(query(connection, "SELECT count(*) FROM gogoke_context_operations WHERE context_id='rolled-back'", &[], 1)[0][0], "0");
        assert_eq!(query(connection, "SELECT count(*) FROM gogoke_context_edges WHERE to_ref='rolled-back@1'", &[], 1)[0][0], "0");
        assert_eq!(query(connection, "SELECT count(*) FROM gogoke_context_promotion_authorizations", &[], 1)[0][0], "1");
    });
}
#[test]
fn storage_group_requires_an_actual_outer_transaction() {
    fixture(|connection, _, _, _| {
        assert!(apply_context_version_in_transaction(connection, command("unowned")).is_err());
        assert_eq!(query(connection, "SELECT count(*) FROM gogoke_context_versions WHERE context_id='unowned'", &[], 1)[0][0], "0");
    });
}
#[test]
fn unexpected_authorization_receipt_trigger_fails_closed() {
    fixture(|connection, owner, request, target| {
        commit_owner_promotion(connection, owner, &request, target.clone()).unwrap();
        connection.execute("CREATE TRIGGER forged_promotion_receipt AFTER INSERT ON gogoke_context_promotion_authorizations BEGIN SELECT 1; END").unwrap();
        assert!(commit_owner_promotion(connection, owner, &request, target).is_err());
    });
}
#[test]
fn stored_profile_root_must_match_the_verified_connection_on_commit() {
    fixture(|connection, owner, request, target| {
        connection.execute("UPDATE gogoke_authority_profile SET root_identity='forged-root'").unwrap();
        assert!(commit_owner_promotion(connection, owner, &request, target).is_err());
        assert_eq!(target_count(connection), "0");
    });
}
