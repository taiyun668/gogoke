use super::bootstrap::initialize_profile;
use super::catalog::{self, issue_owner_grant};
use super::delegation::*;
use super::model::{GrantRef, GrantSpec};
use super::transaction;
use crate::root::RootLock;
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn scratch() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("gogoke-delegation-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    path
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path.join("state.sqlite"));
    if let Err(error) = std::fs::remove_dir(path) {
        eprintln!("owned delegation fixture retained: {error}");
    }
}

fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, &super::bootstrap::OwnerIssuer)) {
    let _guard = route_b_test_guard();
    let path = scratch();
    let root = RootLock::acquire(&path).unwrap();
    let mut connection = create_new(&root, &path.join("state.sqlite")).unwrap();
    let owner = initialize_profile(&mut connection, &root).unwrap();
    run(&mut connection, &owner);
    connection.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}

fn ceiling() -> AuthorityCeiling {
    AuthorityCeiling {
        allowed_actions: vec!["task.run".into()],
        allowed_target_principal_ids: vec!["principal-worker".into()],
        allowed_target_domain_ids: vec!["domain-project".into()],
        allowed_sinks: vec!["runtime.codex".into()],
        allowed_material_classes: vec!["task-context".into()],
        explicit_private_material_ids: vec!["material-explicit".into()],
        allowed_continuation_responses: vec!["continue".into()],
        max_material_items: 8,
        max_material_bytes: 4096,
        max_response_bytes: 2048,
    }
}

fn input(owner: &super::bootstrap::OwnerIssuer, expires_at_epoch_ms: u64) -> DelegationGrantInput {
    DelegationGrantInput {
        principal: DelegationPrincipal {
            principal_id: owner.principal_id().into(),
            project_id: "project-one".into(),
            domain_id: "domain-project".into(),
            role: "controller".into(),
            seat_id: owner.seat_id().into(),
        },
        binding: DelegationBinding {
            session_id: "session-one".into(),
            execution_id: "execution-one".into(),
            generation: "7".into(),
        },
        expires_at_epoch_ms,
        ceiling: ceiling(),
    }
}

fn future() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 3_600_000
}

#[test]
fn fixed_r2_test_grant_replays_once_without_broadening_or_reissuing() {
    fixture(|connection, owner| {
        let admitted = super::bootstrap::read_product_identity(connection, owner).unwrap();
        let request = || DelegationGrantInput {
            principal: DelegationPrincipal {
                principal_id: owner.principal_id().into(),
                project_id: "project-r2-02-test".into(),
                domain_id: "domain-r2-02-test".into(),
                role: "controller".into(),
                seat_id: owner.seat_id().into(),
            },
            binding: DelegationBinding {
                session_id: "session-r2-02-source".into(),
                execution_id: "execution-r2-02-source".into(),
                generation: "1".into(),
            },
            expires_at_epoch_ms: future(),
            ceiling: AuthorityCeiling {
                allowed_actions: vec!["delegate".into()],
                allowed_target_principal_ids: vec!["principal-r2-02-worker".into()],
                allowed_target_domain_ids: vec!["domain-r2-02-test".into()],
                allowed_sinks: vec!["task-package".into()],
                allowed_material_classes: vec![],
                explicit_private_material_ids: vec![],
                allowed_continuation_responses: vec![],
                max_material_items: 0,
                max_material_bytes: 0,
                max_response_bytes: 32 * 1024,
            },
        };
        let first = issue_r2_test_owner_delegation_once(
            connection, owner, &admitted, "r2-test-operation-one", request()).unwrap();
        let replay = issue_r2_test_owner_delegation_once(
            connection, owner, &admitted, "r2-test-operation-one", request()).unwrap();
        assert_eq!(first, replay);
        assert_eq!(transaction::run(connection, |tx| tx.query(
            "SELECT count(*) FROM main.gogoke_authority_grant_heads", &[], 1)).unwrap()[0][0], "1");
        assert_eq!(transaction::run(connection, |tx| tx.query(
            "SELECT count(*) FROM main.gogoke_authority_events WHERE event_kind='ISSUE'", &[], 1)).unwrap()[0][0], "1");
        let mut broader = request();
        broader.ceiling.max_response_bytes += 1;
        assert!(issue_r2_test_owner_delegation_once(
            connection, owner, &admitted, "r2-test-operation-one", broader).is_err());
        let mut stale = admitted.clone();
        stale.policy_revision = "999".into();
        assert!(issue_r2_test_owner_delegation_once(
            connection, owner, &stale, "r2-test-operation-one", request()).is_err());
        revoke_owner_delegation(connection, owner, &DelegationGrantIdentity {
            grant_id: first.reference.grant_id,
            revision: first.reference.revision,
        }).unwrap();
        assert!(issue_r2_test_owner_delegation_once(
            connection, owner, &admitted, "r2-test-operation-one", request()).is_err(),
            "revocation cannot be bypassed by replaying the operation");
    });
}

#[test]
fn typed_delegation_issue_read_delegate_revision_and_revoke_share_the_authority_core() {
    fixture(|connection, owner| {
        let root = issue_owner_delegation(connection, owner, input(owner, future())).unwrap();
        assert_eq!(root.reference.revision, "1");
        assert_eq!(root.reference.revocation_head, "0");
        assert!(root.parent.is_none());
        assert_eq!(root.policy_revision, "1");
        assert_eq!(
            read_current_delegation(connection, &root.reference.grant_id).unwrap(),
            root
        );

        let mut child_input = input(owner, root.expires_at_epoch_ms);
        child_input.principal.principal_id = "principal-worker".into();
        child_input.principal.role = "worker".into();
        child_input.principal.seat_id = "seat-worker".into();
        child_input.ceiling.allowed_actions = vec!["task.run".into()];
        let child = delegate_owner_delegation(
            connection,
            owner,
            &DelegationGrantIdentity {
                grant_id: root.reference.grant_id.clone(),
                revision: root.reference.revision.clone(),
            },
            child_input,
        )
        .unwrap();
        assert_eq!(
            child.parent.as_ref().unwrap().grant_id,
            root.reference.grant_id
        );
        assert_eq!(child.parent.as_ref().unwrap().revocation_head, "0");
        assert_eq!(
            child.issuer_id,
            catalog::seat_issuer(owner.principal_id(), owner.seat_id())
        );
        assert_eq!(child.ceiling.allowed_actions, vec!["task.run"]);

        let revised = revise_owner_delegation(
            connection,
            owner,
            &DelegationGrantIdentity {
                grant_id: root.reference.grant_id.clone(),
                revision: root.reference.revision.clone(),
            },
            input(owner, future()),
        )
        .unwrap();
        assert_eq!(revised.reference.revision, "2");
        assert!(
            read_current_delegation(connection, &child.reference.grant_id).is_err(),
            "parent revision movement invalidates the old child lineage"
        );
        let next_head = revoke_owner_delegation(
            connection,
            owner,
            &DelegationGrantIdentity {
                grant_id: revised.reference.grant_id.clone(),
                revision: revised.reference.revision.clone(),
            },
        )
        .unwrap();
        assert_eq!(next_head, "1");
        assert!(read_current_delegation(connection, &revised.reference.grant_id).is_err());
        transaction::run(connection, |tx| {
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_authority_grant_heads", &[], 1)?[0][0], "2");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_authority_events WHERE event_kind IN ('ISSUE','REVISE','REVOKE')", &[], 1)?[0][0], "4");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_authority_context_grant_payloads", &[], 1)?[0][0], "0");
            Ok(())
        }).unwrap();
    });
}

#[test]
fn every_ceiling_axis_rejects_child_widening() {
    fixture(|connection, owner| {
        let parent = issue_owner_delegation(connection, owner, input(owner, future())).unwrap();
        let changes: [fn(&mut DelegationGrantInput); 10] = [
            |x| x.ceiling.allowed_actions.push("task.delete".into()),
            |x| {
                x.ceiling
                    .allowed_target_principal_ids
                    .push("principal-other".into())
            },
            |x| {
                x.ceiling
                    .allowed_target_domain_ids
                    .push("domain-other".into())
            },
            |x| x.ceiling.allowed_sinks.push("runtime.other".into()),
            |x| {
                x.ceiling
                    .allowed_material_classes
                    .push("private-other".into())
            },
            |x| {
                x.ceiling
                    .explicit_private_material_ids
                    .push("material-other".into())
            },
            |x| {
                x.ceiling
                    .allowed_continuation_responses
                    .push("approve".into())
            },
            |x| x.ceiling.max_material_items += 1,
            |x| x.ceiling.max_material_bytes += 1,
            |x| x.ceiling.max_response_bytes += 1,
        ];
        for widen in changes {
            let mut child = input(owner, parent.expires_at_epoch_ms);
            child.principal.principal_id = "principal-worker".into();
            child.principal.seat_id = "seat-worker".into();
            child.principal.role = "worker".into();
            widen(&mut child);
            assert!(delegate_owner_delegation(
                connection,
                owner,
                &DelegationGrantIdentity {
                    grant_id: parent.reference.grant_id.clone(),
                    revision: parent.reference.revision.clone(),
                },
                child
            )
            .is_err());
        }
    });
}

#[test]
fn array_order_round_trips_without_the_generic_sixty_four_row_limit() {
    fixture(|connection, owner| {
        let mut parent_input = input(owner, future());
        parent_input.ceiling.allowed_actions =
            (0..80).map(|index| format!("action-{index:03}")).collect();
        let parent = issue_owner_delegation(connection, owner, parent_input.clone()).unwrap();
        assert_eq!(
            parent.ceiling.allowed_actions,
            parent_input.ceiling.allowed_actions
        );
        let mut child_input = parent_input.clone();
        child_input.principal.principal_id = "principal-worker".into();
        child_input.principal.seat_id = "seat-worker".into();
        child_input.principal.role = "worker".into();
        child_input.ceiling.allowed_actions.reverse();
        let child = delegate_owner_delegation(
            connection,
            owner,
            &DelegationGrantIdentity {
                grant_id: parent.reference.grant_id,
                revision: parent.reference.revision,
            },
            child_input.clone(),
        )
        .unwrap();
        assert_eq!(
            child.ceiling.allowed_actions,
            child_input.ceiling.allowed_actions
        );
    });
}

#[test]
fn context_and_delegation_payloads_never_resolve_as_each_other() {
    fixture(|connection, owner| {
        let context = issue_owner_grant(
            connection,
            owner,
            "1",
            "0",
            GrantSpec {
                principal_id: owner.principal_id().into(),
                seat_id: owner.seat_id().into(),
                permission: "context.read".into(),
                promotion_kind: "NONE".into(),
                source_domain_id: "domain-one".into(),
                destination_domain_id: "domain-one".into(),
                destination_scope: "PROJECT".into(),
                delegable_depth: 0,
            },
        )
        .unwrap();
        assert!(read_current_delegation(connection, &context.grant_id).is_err());

        let delegation = issue_owner_delegation(connection, owner, input(owner, future())).unwrap();
        let as_context = GrantRef {
            grant_id: delegation.reference.grant_id.clone(),
            revision: delegation.reference.revision.clone(),
            revocation_head: "0".into(),
        };
        assert!(transaction::run(connection, |tx| {
            let profile = catalog::current_profile(tx)?;
            catalog::resolve_current(tx, &profile, &as_context).map(|_| ())
        })
        .is_err());
    });
}

#[test]
fn expired_unsafe_or_missing_payload_delegations_fail_closed() {
    fixture(|connection, owner| {
        let expired = input(owner, 1);
        assert!(issue_owner_delegation(connection, owner, expired).is_err());
        let unsafe_expiry = input(owner, 9_007_199_254_740_992);
        assert!(issue_owner_delegation(connection, owner, unsafe_expiry).is_err());
        let expired_current = issue_owner_delegation(connection, owner, input(owner, future())).unwrap();
        connection.execute(&format!(
            "UPDATE main.gogoke_authority_delegation_grant_payloads SET expires_at_epoch_ms='1' WHERE grant_id='{}'",
            expired_current.reference.grant_id,
        )).unwrap();
        assert!(read_current_delegation(connection, &expired_current.reference.grant_id).is_err());
        let grant = issue_owner_delegation(connection, owner, input(owner, future())).unwrap();
        connection.execute(&format!(
            "DELETE FROM gogoke_authority_delegation_ceiling_entries WHERE grant_id='{}';DELETE FROM gogoke_authority_delegation_grant_payloads WHERE grant_id='{}'",
            grant.reference.grant_id, grant.reference.grant_id,
        )).unwrap();
        assert!(matches!(
            read_current_delegation(connection, &grant.reference.grant_id),
            Err(OrchestrationError::AccessDenied)
        ));
    });
}

#[test]
fn a_grant_revision_with_both_payload_kinds_is_rejected() {
    fixture(|connection, owner| {
        let grant = issue_owner_delegation(connection, owner, input(owner, future())).unwrap();
        connection.execute(&format!(
            "INSERT INTO main.gogoke_authority_context_grant_payloads(grant_id,revision,principal_id,seat_id,permission,promotion_kind,source_domain_id,destination_domain_id,destination_scope,delegable_depth) VALUES('{}','1','principal-worker','seat-worker','context.read','NONE','domain-project','domain-project','PROJECT',0)",
            grant.reference.grant_id,
        )).unwrap();
        assert!(read_current_delegation(connection, &grant.reference.grant_id).is_err());
    });
}

#[test]
fn current_authority_access_fails_when_foreign_keys_are_off_or_broken() {
    fixture(|connection, owner| {
        let grant = issue_owner_delegation(connection, owner, input(owner, future())).unwrap();
        let grant_id = grant.reference.grant_id;
        connection.execute("PRAGMA foreign_keys=OFF").unwrap();
        assert!(read_current_delegation(connection, &grant_id).is_err());
        assert!(issue_owner_delegation(connection, owner, input(owner, future())).is_err());
        connection.execute("PRAGMA foreign_keys=ON").unwrap();

        connection.execute("PRAGMA foreign_keys=OFF").unwrap();
        connection.execute(
            "INSERT INTO main.gogoke_authority_context_grant_payloads(grant_id,revision,principal_id,seat_id,permission,promotion_kind,source_domain_id,destination_domain_id,destination_scope,delegable_depth) VALUES('orphan-grant','1','principal-orphan','seat-orphan','context.read','NONE','domain-project','domain-project','PROJECT',0)",
        ).unwrap();
        connection.execute("PRAGMA foreign_keys=ON").unwrap();
        transaction::run(connection, |tx| {
            assert!(!tx
                .query("PRAGMA main.foreign_key_check", &[], 4)?
                .is_empty());
            Ok(())
        })
        .unwrap();
        connection
            .execute("CREATE TEMP TABLE pragma_foreign_key_check(dummy TEXT)")
            .unwrap();
        assert!(read_current_delegation(connection, &grant_id).is_err());
        assert!(issue_owner_delegation(connection, owner, input(owner, future())).is_err());
    });
}
