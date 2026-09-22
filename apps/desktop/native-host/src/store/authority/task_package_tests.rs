use super::bootstrap::initialize_profile;
use super::delegation::{
    issue_owner_delegation, read_current_delegation, AuthorityCeiling, DelegationBinding,
    DelegationGrantIdentity, DelegationGrantInput, DelegationPrincipal,
};
use super::material::{
    append_trusted_task_material, AppendTaskMaterial, MaterialVisibility, TaskMaterial,
};
use super::task_package::{
    digest_canonical, package_from_current, package_json, prepare_in_transaction,
    AuthorizedTaskPackage, AuthorizedTaskPackageDraft, PrepareAuthorizedTaskPackage,
    SelectedMaterial, TaskMaterialReference, TaskPackageBinding, TaskPackagePrincipal,
};
use super::transaction;
use super::{
    initialize_authorized_task_package_schema, prepare_authorized_task_package,
    read_authorized_task_package,
};
use crate::root::RootLock;
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn scratch(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("gogoke-atp-{label}-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    path
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path.join("state.sqlite"));
    if let Err(error) = std::fs::remove_dir(path) {
        eprintln!("owned AuthorizedTaskPackage fixture retained: {error}");
    }
}

fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, &super::bootstrap::OwnerIssuer)) {
    let _guard = route_b_test_guard();
    let path = scratch("authority");
    let root = RootLock::acquire(&path).unwrap();
    let mut connection = create_new(&root, &path.join("state.sqlite")).unwrap();
    crate::store::atomic::apply_core_schema(&mut connection).unwrap();
    crate::store::context::apply_context_schema(&mut connection).unwrap();
    crate::store::context_state::initialize_context_state_schema(&mut connection).unwrap();
    initialize_authorized_task_package_schema(&mut connection).unwrap();
    let owner = initialize_profile(&mut connection, &root).unwrap();
    seed_material(
        &mut connection,
        &owner,
        "material-public",
        "domain-source",
        "project-one",
        "spec",
        MaterialVisibility::Project,
        "fixture snowman ☃",
        None,
    );
    run(&mut connection, &owner);
    connection.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}

fn ceiling() -> AuthorityCeiling {
    AuthorityCeiling {
        allowed_actions: vec!["delegate".into()],
        allowed_target_principal_ids: vec![
            "principal-worker".into(),
            "principal-controller".into(),
            "principal-auditor".into(),
        ],
        allowed_target_domain_ids: vec!["domain-target".into()],
        allowed_sinks: vec!["task-package".into(), "formal-review".into()],
        allowed_material_classes: vec!["spec".into()],
        explicit_private_material_ids: vec![],
        allowed_continuation_responses: vec!["continue".into()],
        max_material_items: 8,
        max_material_bytes: 4096,
        max_response_bytes: 2048,
    }
}

fn expires() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 3_600_000
}

fn issue(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &super::bootstrap::OwnerIssuer,
) -> super::delegation::DelegationGrantSnapshot {
    issue_with_ceiling(connection, owner, ceiling())
}

fn issue_with_ceiling(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &super::bootstrap::OwnerIssuer,
    authority_ceiling: AuthorityCeiling,
) -> super::delegation::DelegationGrantSnapshot {
    issue_owner_delegation(
        connection,
        owner,
        DelegationGrantInput {
            principal: DelegationPrincipal {
                principal_id: owner.principal_id().into(),
                project_id: "project-one".into(),
                domain_id: "domain-source".into(),
                role: "controller".into(),
                seat_id: owner.seat_id().into(),
            },
            binding: DelegationBinding {
                session_id: "session-source".into(),
                execution_id: "execution-source".into(),
                generation: "7".into(),
            },
            expires_at_epoch_ms: expires(),
            ceiling: authority_ceiling,
        },
    )
    .unwrap()
}

fn proposal(grant: &super::delegation::DelegationGrantSnapshot) -> AuthorizedTaskPackageDraft {
    AuthorizedTaskPackageDraft {
        parent_grant_ref: grant.reference.grant_id.clone(),
        parent_grant_revision: grant.reference.revision.clone(),
        parent_grant_revocation_head: grant.reference.revocation_head.clone(),
        parent_policy_revision: grant.policy_revision.clone(),
        parent_seat_id: grant.principal.seat_id.clone(),
        child_ceiling: AuthorityCeiling {
            allowed_actions: vec!["delegate".into()],
            allowed_target_principal_ids: vec!["principal-worker".into()],
            allowed_target_domain_ids: vec!["domain-target".into()],
            allowed_sinks: vec!["task-package".into()],
            allowed_material_classes: vec!["spec".into()],
            explicit_private_material_ids: vec![],
            allowed_continuation_responses: vec![],
            max_material_items: 2,
            max_material_bytes: 512,
            max_response_bytes: 128,
        },
        action: "delegate".into(),
        route: "controller-worker".into(),
        source: TaskPackagePrincipal {
            principal_id: grant.principal.principal_id.clone(),
            project_id: grant.principal.project_id.clone(),
            domain_id: grant.principal.domain_id.clone(),
            role: grant.principal.role.clone(),
        },
        target: TaskPackagePrincipal {
            principal_id: "principal-worker".into(),
            project_id: "project-one".into(),
            domain_id: "domain-target".into(),
            role: "worker".into(),
        },
        source_binding: TaskPackageBinding {
            session_id: grant.binding.session_id.clone(),
            execution_id: grant.binding.execution_id.clone(),
            generation: grant.binding.generation.clone(),
        },
        target_binding: TaskPackageBinding {
            session_id: "session-worker".into(),
            execution_id: "execution-worker".into(),
            generation: "1".into(),
        },
        target_binding_kind: "existing".into(),
        sink: "task-package".into(),
        instruction: "Run bounded task λ".into(),
    }
}

fn operation(package: AuthorizedTaskPackageDraft, id: &str) -> PrepareAuthorizedTaskPackage {
    operation_with_refs(
        package,
        id,
        vec![TaskMaterialReference {
            material_id: "material-public".into(),
            revision: "1".into(),
        }],
    )
}

fn operation_with_refs(
    package: AuthorizedTaskPackageDraft,
    id: &str,
    material_refs: Vec<TaskMaterialReference>,
) -> PrepareAuthorizedTaskPackage {
    PrepareAuthorizedTaskPackage {
        operation_id: id.into(),
        domain_id: "domain-source".into(),
        event_id: format!("event-{id}"),
        receipt_id: format!("receipt-{id}"),
        recorded_at: "2026-09-22T12:00:00Z".into(),
        package,
        material_refs,
    }
}

fn seed_material(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &super::bootstrap::OwnerIssuer,
    material_id: &str,
    domain_id: &str,
    project_id: &str,
    material_class: &str,
    visibility: MaterialVisibility,
    content: &str,
    expected_previous_revision: Option<&str>,
) -> TaskMaterialReference {
    let revision = expected_previous_revision
        .map(|previous| {
            previous
                .parse::<u64>()
                .unwrap()
                .checked_add(1)
                .unwrap()
                .to_string()
        })
        .unwrap_or_else(|| "1".into());
    let operation_id = format!("seed-{material_id}-r{revision}");
    append_trusted_task_material(
        connection,
        owner,
        &AppendTaskMaterial {
            operation_id: operation_id.clone(),
            expected_previous_revision: expected_previous_revision.map(str::to_owned),
            material: TaskMaterial {
                material_id: material_id.into(),
                project_id: project_id.into(),
                domain_id: domain_id.into(),
                owner_principal_id: owner.principal_id().into(),
                material_class: material_class.into(),
                visibility,
                content: content.into(),
            },
            provenance_ref: format!("evidence:{operation_id}"),
            event_id: format!("event-{operation_id}"),
            receipt_id: format!("receipt-{operation_id}"),
            recorded_at: "2026-09-22T12:00:00Z".into(),
        },
    )
    .unwrap();
    TaskMaterialReference {
        material_id: material_id.into(),
        revision,
    }
}

fn draft_for_current(
    value: &AuthorizedTaskPackageDraft,
    current: &super::delegation::DelegationGrantSnapshot,
) -> AuthorizedTaskPackageDraft {
    let mut draft = value.clone();
    draft.parent_grant_ref = current.reference.grant_id.clone();
    draft.parent_grant_revision = current.reference.revision.clone();
    draft.parent_grant_revocation_head = current.reference.revocation_head.clone();
    draft.parent_policy_revision = current.policy_revision.clone();
    draft.parent_seat_id = current.principal.seat_id.clone();
    draft
}

#[test]
fn same_transaction_prepare_read_and_exact_replay_persist_object_event_receipt_and_index() {
    fixture(|connection, owner| {
        let grant = issue(connection, owner);
        let request = operation(proposal(&grant), "operation-one");
        let first = prepare_authorized_task_package(connection, &request).unwrap();
        assert_eq!(first.disposition, "COMMITTED");
        assert_eq!(first.authority_status, "PREPARATORY_TRUSTED_MATERIAL_REFS");
        let read =
            read_authorized_task_package(connection, "domain-source", "operation-one").unwrap();
        assert_eq!(read.canonical_package, first.canonical_package);
        assert_eq!(read.package_digest, first.package_digest);
        assert_eq!(
            prepare_authorized_task_package(connection, &request)
                .unwrap()
                .disposition,
            "REPLAYED"
        );
        let mut conflicting = request.clone();
        conflicting.package.instruction.push('!');
        conflicting.package = draft_for_current(&conflicting.package, &grant);
        assert!(matches!(
            prepare_authorized_task_package(connection, &conflicting),
            Err(OrchestrationError::OperationConflict)
        ));
        let mut different_receipt = request.clone();
        different_receipt.receipt_id = "receipt-alternative".into();
        assert!(matches!(
            prepare_authorized_task_package(connection, &different_receipt),
            Err(OrchestrationError::OperationConflict)
        ));
        let mut different_event = request.clone();
        different_event.event_id = "event-alternative".into();
        assert!(matches!(
            prepare_authorized_task_package(connection, &different_event),
            Err(OrchestrationError::OperationConflict)
        ));
        let mut different_time = request.clone();
        different_time.recorded_at = "2026-09-22T12:00:01Z".into();
        assert!(matches!(
            prepare_authorized_task_package(connection, &different_time),
            Err(OrchestrationError::OperationConflict)
        ));
        transaction::run(connection,|tx|{tx.write("UPDATE main.gogoke_authorized_task_packages SET parent_grant_revision='9' WHERE operation_id=?",&[&request.operation_id])?;Ok(())}).unwrap();
        assert!(
            read_authorized_task_package(connection, "domain-source", "operation-one").is_err()
        );
        transaction::run(connection,|tx|{tx.write("UPDATE main.gogoke_authorized_task_packages SET parent_grant_revision='1' WHERE operation_id=?",&[&request.operation_id])?;Ok(())}).unwrap();
        transaction::run(connection,|tx| {
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_objects WHERE object_type='AuthorizedTaskPackage'",&[],1)?[0][0],"1");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_events WHERE event_type='AuthorizedTaskPackagePrepared'",&[],1)?[0][0],"1");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_receipts WHERE receipt_type='AuthorizedTaskPackagePrepared'",&[],1)?[0][0],"1");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_authorized_task_packages",&[],1)?[0][0],"1"); Ok(())
        }).unwrap();
    });
}

#[test]
fn material_reference_resolution_is_atomic_and_checks_project_ceiling_and_revisions() {
    fixture(|connection, owner| {
        let grant = issue(connection, owner);
        let missing_last = operation_with_refs(
            proposal(&grant),
            "operation-material-missing-last",
            vec![
                TaskMaterialReference {
                    material_id: "material-public".into(),
                    revision: "1".into(),
                },
                TaskMaterialReference {
                    material_id: "material-missing".into(),
                    revision: "1".into(),
                },
            ],
        );
        assert!(prepare_authorized_task_package(connection, &missing_last).is_err());
        transaction::run(connection, |tx| {
            assert_eq!(
                tx.query(
                    "SELECT count(*) FROM main.gogoke_objects WHERE object_type='AuthorizedTaskPackage'",
                    &[],
                    1,
                )?[0][0],
                "0"
            );
            assert_eq!(
                tx.query("SELECT count(*) FROM main.gogoke_events WHERE event_type='AuthorizedTaskPackagePrepared'", &[], 1)?[0][0],
                "0"
            );
            assert_eq!(
                tx.query("SELECT count(*) FROM main.gogoke_receipts WHERE operation_id=?", &[&missing_last.operation_id], 1)?[0][0],
                "0"
            );
            assert_eq!(
                tx.query("SELECT count(*) FROM main.gogoke_authorized_task_packages WHERE operation_id=?", &[&missing_last.operation_id], 1)?[0][0],
                "0"
            );
            Ok(())
        })
        .unwrap();

        let cross_domain_material = seed_material(
            connection,
            owner,
            "material-cross-domain",
            "domain-other",
            "project-one",
            "spec",
            MaterialVisibility::Project,
            "root-owned cross-domain content",
            None,
        );
        let child_grant = issue_owner_delegation(
            connection,
            owner,
            DelegationGrantInput {
                principal: DelegationPrincipal {
                    principal_id: "principal-child-controller".into(),
                    project_id: "project-one".into(),
                    domain_id: "domain-source".into(),
                    role: "controller".into(),
                    seat_id: "seat-child-controller".into(),
                },
                binding: DelegationBinding {
                    session_id: "session-child-controller".into(),
                    execution_id: "execution-child-controller".into(),
                    generation: "1".into(),
                },
                expires_at_epoch_ms: expires(),
                ceiling: ceiling(),
            },
        )
        .unwrap();
        let cross_domain = prepare_authorized_task_package(
            connection,
            &operation_with_refs(
                proposal(&child_grant),
                "operation-material-cross-domain-child-share",
                vec![cross_domain_material],
            ),
        )
        .unwrap();
        assert_eq!(cross_domain.disposition, "COMMITTED");
        assert!(String::from_utf8(cross_domain.canonical_package)
            .unwrap()
            .contains("root-owned cross-domain content"));

        let wrong_project = seed_material(
            connection,
            owner,
            "material-wrong-project",
            "domain-source",
            "project-other",
            "spec",
            MaterialVisibility::Project,
            "wrong project",
            None,
        );
        assert!(prepare_authorized_task_package(
            connection,
            &operation_with_refs(
                proposal(&grant),
                "operation-material-wrong-project",
                vec![wrong_project],
            ),
        )
        .is_err());
    });
}

#[test]
fn replay_rechecks_exact_material_revision_and_keeps_historical_package_bytes() {
    fixture(|connection, owner| {
        let grant = issue(connection, owner);
        let request = operation(proposal(&grant), "operation-material-revision-replay");
        let committed = prepare_authorized_task_package(connection, &request).unwrap();
        let original_bytes = committed.canonical_package.clone();

        let revision_two = seed_material(
            connection,
            owner,
            "material-public",
            "domain-source",
            "project-one",
            "spec",
            MaterialVisibility::Project,
            "fixture snowman ☃",
            Some("1"),
        );
        assert_eq!(revision_two.revision, "2");
        assert!(prepare_authorized_task_package(connection, &request).is_err());

        let unchanged = transaction::run(connection, |tx| {
            let rows = tx.query(
                "SELECT CAST(canonical_json AS TEXT) FROM main.gogoke_objects WHERE domain_id=? AND object_type='AuthorizedTaskPackage' AND object_id=? AND object_version='1'",
                &[&request.domain_id, &committed.package_id],
                1,
            )?;
            if rows.len() != 1 {
                return Err(OrchestrationError::AccessDenied);
            }
            Ok(rows[0][0].as_bytes().to_vec())
        })
        .unwrap();
        assert_eq!(unchanged, original_bytes);

        let altered_refs =
            super::task_package::material_refs_json(std::slice::from_ref(&revision_two));
        transaction::run(connection, |tx| {
            tx.write(
                "UPDATE main.gogoke_authorized_task_packages SET material_refs_json=? WHERE domain_id=? AND operation_id=?",
                &[&altered_refs, &request.domain_id, &request.operation_id],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(read_authorized_task_package(
            connection,
            &request.domain_id,
            &request.operation_id
        )
        .is_err());
    });
}

#[test]
fn tampered_material_source_record_fails_closed_before_package_write() {
    fixture(|connection, owner| {
        let grant = issue(connection, owner);
        let request = operation(proposal(&grant), "operation-material-source-tamper");
        transaction::run(connection, |tx| {
            tx.write(
                "UPDATE main.gogoke_objects SET canonical_json=CAST('{}' AS BLOB) WHERE domain_id='domain-source' AND object_type='TaskMaterial' AND object_id='material-public' AND object_version='1'",
                &[],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(prepare_authorized_task_package(connection, &request).is_err());
        transaction::run(connection, |tx| {
            assert_eq!(
                tx.query(
                    "SELECT count(*) FROM main.gogoke_objects WHERE object_type='AuthorizedTaskPackage'",
                    &[],
                    1,
                )?[0][0],
                "0"
            );
            Ok(())
        })
        .unwrap();
    });
}

#[test]
fn replay_fingerprint_event_stream_head_and_receipt_coordinates_are_cross_bound() {
    fixture(|connection, owner| {
        let grant = issue(connection, owner);
        let first = operation(proposal(&grant), "operation-coordinate-one");
        let first_receipt = prepare_authorized_task_package(connection, &first).unwrap();
        let mut second_package = proposal(&grant);
        second_package.instruction = "second durable package".into();
        let second = operation(second_package, "operation-coordinate-two");
        prepare_authorized_task_package(connection, &second).unwrap();

        let fingerprints = transaction::run(connection, |tx| {
            let rows = tx.query(
                "SELECT r.operation_fingerprint FROM main.gogoke_receipts r WHERE r.domain_id=? AND r.operation_id=?",
                &[&first.domain_id, &first.operation_id],
                1,
            )?;
            if rows.len() != 1 { return Err(OrchestrationError::AccessDenied); }
            Ok(rows[0][0].clone())
        }).unwrap();

        transaction::run(connection, |tx| {
            tx.write("UPDATE main.gogoke_receipts SET operation_fingerprint=? WHERE domain_id=? AND operation_id=?", &[&format!("sha256:{}", "a".repeat(64)), &first.domain_id, &first.operation_id])?;
            Ok(())
        }).unwrap();
        assert!(
            read_authorized_task_package(connection, &first.domain_id, &first.operation_id)
                .is_err()
        );
        transaction::run(connection, |tx| { tx.write("UPDATE main.gogoke_receipts SET operation_fingerprint=? WHERE domain_id=? AND operation_id=?", &[&fingerprints, &first.domain_id, &first.operation_id])?; Ok(()) }).unwrap();

        transaction::run(connection, |tx| {
            tx.write("UPDATE main.gogoke_authorized_task_packages SET operation_fingerprint=? WHERE domain_id=? AND operation_id=?", &[&format!("sha256:{}", "b".repeat(64)), &first.domain_id, &first.operation_id])?;
            Ok(())
        }).unwrap();
        assert!(
            read_authorized_task_package(connection, &first.domain_id, &first.operation_id)
                .is_err()
        );
        transaction::run(connection, |tx| { tx.write("UPDATE main.gogoke_authorized_task_packages SET operation_fingerprint=? WHERE domain_id=? AND operation_id=?", &[&fingerprints, &first.domain_id, &first.operation_id])?; Ok(()) }).unwrap();

        transaction::run(connection, |tx| {
            tx.write(
                "UPDATE main.gogoke_events SET stream_counter='1' WHERE domain_id=? AND event_id=?",
                &[&first.domain_id, &first.event_id],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(
            read_authorized_task_package(connection, &first.domain_id, &first.operation_id)
                .is_err()
        );
        transaction::run(connection, |tx| {
            tx.write(
                "UPDATE main.gogoke_events SET stream_counter='0' WHERE domain_id=? AND event_id=?",
                &[&first.domain_id, &first.event_id],
            )?;
            Ok(())
        })
        .unwrap();

        let stream_id = format!(
            "gogoke.authorized-task-package.v1/{}",
            first_receipt.package_id
        );
        transaction::run(connection, |tx| {
            tx.write(
                "UPDATE main.gogoke_stream_heads SET counter='1' WHERE domain_id=? AND stream_id=?",
                &[&first.domain_id, &stream_id],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(
            read_authorized_task_package(connection, &first.domain_id, &first.operation_id)
                .is_err()
        );
        transaction::run(connection, |tx| {
            tx.write(
                "UPDATE main.gogoke_stream_heads SET counter='0' WHERE domain_id=? AND stream_id=?",
                &[&first.domain_id, &stream_id],
            )?;
            Ok(())
        })
        .unwrap();

        transaction::run(connection, |tx| { tx.write("UPDATE main.gogoke_receipts SET receipt_id='receipt-altered' WHERE domain_id=? AND operation_id=?", &[&first.domain_id,&first.operation_id])?; Ok(()) }).unwrap();
        assert!(
            read_authorized_task_package(connection, &first.domain_id, &first.operation_id)
                .is_err()
        );
        transaction::run(connection, |tx| {
            tx.write(
                "UPDATE main.gogoke_receipts SET receipt_id=? WHERE domain_id=? AND operation_id=?",
                &[&first.receipt_id, &first.domain_id, &first.operation_id],
            )?;
            Ok(())
        })
        .unwrap();

        transaction::run(connection, |tx| {
            tx.write(
                "UPDATE main.gogoke_receipts SET event_id=? WHERE domain_id=? AND operation_id=?",
                &[&second.event_id, &first.domain_id, &first.operation_id],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(
            read_authorized_task_package(connection, &first.domain_id, &first.operation_id)
                .is_err()
        );
    });
}

#[test]
fn ten_child_ceiling_axes_cannot_be_widened_even_after_all_package_hashes_are_recomputed() {
    fixture(|connection, owner| {
        let grant = issue(connection, owner);
        let base = proposal(&grant);
        let changes: [fn(&mut AuthorityCeiling); 10] = [
            |c| c.allowed_actions.push("cancel-continuation".into()),
            |c| {
                c.allowed_target_principal_ids
                    .push("principal-other".into())
            },
            |c| c.allowed_target_domain_ids.push("domain-other".into()),
            |c| c.allowed_sinks.push("files".into()),
            |c| c.allowed_material_classes.push("secret".into()),
            |c| {
                c.explicit_private_material_ids
                    .push("material-private".into())
            },
            |c| c.allowed_continuation_responses.push("approve".into()),
            |c| c.max_material_items = 9,
            |c| c.max_material_bytes = 4097,
            |c| c.max_response_bytes = 2049,
        ];
        for (i, change) in changes.into_iter().enumerate() {
            let mut malicious = base.clone();
            change(&mut malicious.child_ceiling);
            malicious = draft_for_current(&malicious, &grant);
            let mut request = operation(malicious, &format!("operation-widen-{i}"));
            assert!(
                prepare_authorized_task_package(connection, &request).is_err(),
                "ceiling axis {i} was accepted"
            );
            request.package.target.principal_id = "principal-other".into();
            request.package = draft_for_current(&request.package, &grant);
            assert!(prepare_authorized_task_package(connection, &request).is_err());
        }
    });
}

#[test]
fn action_target_principal_target_domain_and_sink_must_also_be_in_child_ceiling() {
    fixture(|connection, owner| {
        let mut parent_ceiling = ceiling();
        parent_ceiling.allowed_actions = vec!["delegate".into(), "request-review".into()];
        parent_ceiling.allowed_target_principal_ids = vec![
            "principal-worker".into(),
            "principal-other".into(),
            "principal-auditor".into(),
        ];
        parent_ceiling.allowed_target_domain_ids =
            vec!["domain-target".into(), "domain-other".into()];
        parent_ceiling.allowed_sinks = vec!["task-package".into(), "formal-review".into()];
        let grant = issue_with_ceiling(connection, owner, parent_ceiling);

        let mut action_outside_child = proposal(&grant);
        action_outside_child.action = "request-review".into();
        action_outside_child.route = "controller-clean-review".into();
        action_outside_child.target = TaskPackagePrincipal {
            principal_id: "principal-auditor".into(),
            project_id: "project-one".into(),
            domain_id: "domain-target".into(),
            role: "auditor".into(),
        };
        action_outside_child.target_binding_kind = "new-clean".into();
        action_outside_child.sink = "formal-review".into();
        action_outside_child
            .child_ceiling
            .allowed_target_principal_ids = vec!["principal-auditor".into()];
        action_outside_child.child_ceiling.allowed_sinks = vec!["formal-review".into()];
        assert!(prepare_authorized_task_package(
            connection,
            &operation(action_outside_child.clone(), "operation-child-action"),
        )
        .is_err());

        let mut principal_outside_child = proposal(&grant);
        principal_outside_child.target.principal_id = "principal-other".into();
        assert!(prepare_authorized_task_package(
            connection,
            &operation(principal_outside_child, "operation-child-principal"),
        )
        .is_err());

        let mut domain_outside_child = proposal(&grant);
        domain_outside_child.target.domain_id = "domain-other".into();
        assert!(prepare_authorized_task_package(
            connection,
            &operation(domain_outside_child, "operation-child-domain"),
        )
        .is_err());

        let mut sink_outside_child = action_outside_child;
        sink_outside_child.child_ceiling.allowed_actions = vec!["request-review".into()];
        sink_outside_child.child_ceiling.allowed_sinks = vec!["task-package".into()];
        assert!(prepare_authorized_task_package(
            connection,
            &operation(sink_outside_child, "operation-child-sink"),
        )
        .is_err());
    });
}

#[test]
fn arrays_and_materials_over_sixty_four_follow_authority_ceiling_and_round_trip_in_order() {
    fixture(|connection, owner| {
        let continuation_values = (0..65)
            .map(|index| format!("response-{index:03}"))
            .collect::<Vec<_>>();
        let mut authority_ceiling = ceiling();
        authority_ceiling.allowed_continuation_responses = continuation_values.clone();
        authority_ceiling.max_material_items = 128;
        authority_ceiling.max_material_bytes = 16_384;
        let grant = issue_with_ceiling(connection, owner, authority_ceiling.clone());
        let mut package = proposal(&grant);
        package.child_ceiling.allowed_continuation_responses = continuation_values;
        package.child_ceiling.max_material_items = 80;
        package.child_ceiling.max_material_bytes = 8_192;
        let material_refs = (0..65)
            .map(|index| {
                seed_material(
                    connection,
                    owner,
                    &format!("material-{index:03}"),
                    "domain-source",
                    "project-one",
                    "spec",
                    MaterialVisibility::Project,
                    &format!("payload:{index:03}, λ☃\0"),
                    None,
                )
            })
            .collect::<Vec<_>>();
        let request = operation_with_refs(
            package.clone(),
            "operation-over-sixty-four",
            material_refs.clone(),
        );
        let committed = prepare_authorized_task_package(connection, &request).unwrap();
        assert_eq!(committed.disposition, "COMMITTED");
        let read = read_authorized_task_package(connection, "domain-source", &request.operation_id)
            .unwrap();
        assert_eq!(read.canonical_package, committed.canonical_package);
        transaction::run(connection, |tx| {
            let counts = tx.query(
                "SELECT json_array_length(CAST(canonical_json AS TEXT),'$.materials'),json_array_length(CAST(canonical_json AS TEXT),'$.childCeiling.allowedContinuationResponses') FROM main.gogoke_objects WHERE domain_id=? AND object_type='AuthorizedTaskPackage' AND object_id=? AND object_version='1'",
                &[&request.domain_id, &committed.package_id],
                2,
            )?;
            assert_eq!(counts, vec![vec![String::from("65"), String::from("65")]]);
            Ok(())
        })
        .unwrap();

        let mut over_child_refs = material_refs;
        for index in 65..81 {
            over_child_refs.push(seed_material(
                connection,
                owner,
                &format!("material-{index:03}"),
                "domain-source",
                "project-one",
                "spec",
                MaterialVisibility::Project,
                &format!("payload:{index:03}, λ☃"),
                None,
            ));
        }
        let over_child_count = package.clone();
        assert!(prepare_authorized_task_package(
            connection,
            &operation_with_refs(
                over_child_count,
                "operation-over-child-item-count",
                over_child_refs,
            ),
        )
        .is_err());

        let mut too_many_responses = package.clone();
        too_many_responses
            .child_ceiling
            .allowed_continuation_responses
            .push("outside-parent-response".into());
        assert!(prepare_authorized_task_package(
            connection,
            &operation(too_many_responses, "operation-over-ceiling-response"),
        )
        .is_err());

        let secret_ref = seed_material(
            connection,
            owner,
            "material-secret",
            "domain-source",
            "project-one",
            "secret",
            MaterialVisibility::Project,
            "not approved",
            None,
        );
        assert!(prepare_authorized_task_package(
            connection,
            &operation_with_refs(
                proposal(&grant),
                "operation-over-ceiling-material",
                vec![secret_ref],
            ),
        )
        .is_err());
    });
}

#[test]
fn instruction_larger_than_one_mebibyte_has_no_private_text_cap() {
    fixture(|connection, owner| {
        let grant = issue(connection, owner);
        let mut package = proposal(&grant);
        package.instruction = "x".repeat(1_048_577);
        let request = operation(package.clone(), "operation-large-instruction");
        let committed = prepare_authorized_task_package(connection, &request).unwrap();
        assert_eq!(committed.disposition, "COMMITTED");
        let read = read_authorized_task_package(connection, "domain-source", &request.operation_id)
            .unwrap();
        assert_eq!(read.canonical_package, committed.canonical_package);
    });
}

#[test]
fn material_content_round_trips_legal_nul_escape_but_identity_nuls_fail_closed() {
    fixture(|connection, owner| {
        let grant = issue(connection, owner);
        let nul_ref = seed_material(
            connection,
            owner,
            "material-nul",
            "domain-source",
            "project-one",
            "spec",
            MaterialVisibility::Project,
            "left\0right",
            None,
        );
        let package = proposal(&grant);
        let request = operation_with_refs(package, "operation-nul-material", vec![nul_ref]);
        assert_eq!(
            prepare_authorized_task_package(connection, &request)
                .unwrap()
                .disposition,
            "COMMITTED"
        );
        let nul_read =
            read_authorized_task_package(connection, &request.domain_id, &request.operation_id)
                .unwrap();
        assert!(String::from_utf8(nul_read.canonical_package)
            .unwrap()
            .contains("left\\u0000right"));

        let mut bad_binding = proposal(&grant);
        bad_binding.target_binding.session_id = "session\0worker".into();
        assert!(prepare_authorized_task_package(
            connection,
            &operation(bad_binding, "operation-nul-binding-id"),
        )
        .is_err());

        let bad_material_id = proposal(&grant);
        assert!(prepare_authorized_task_package(
            connection,
            &operation_with_refs(
                bad_material_id,
                "operation-nul-material-id",
                vec![TaskMaterialReference {
                    material_id: "material\0public".into(),
                    revision: "1".into(),
                }],
            ),
        )
        .is_err());
    });
}

#[test]
fn stale_revoked_policy_expired_and_source_binding_mismatches_fail_closed() {
    fixture(|connection, owner| {
        let expiring = issue(connection, owner);
        let expired_request = operation(proposal(&expiring), "operation-expired");
        transaction::run(connection,|tx|{tx.write("UPDATE main.gogoke_authority_delegation_grant_payloads SET expires_at_epoch_ms=1 WHERE grant_id=? AND revision=?",&[&expiring.reference.grant_id,&expiring.reference.revision])?;Ok(())}).unwrap();
        assert!(prepare_authorized_task_package(connection, &expired_request).is_err());
        let grant = issue(connection, owner);
        let valid = operation(proposal(&grant), "operation-current");
        assert_eq!(
            prepare_authorized_task_package(connection, &valid)
                .unwrap()
                .disposition,
            "COMMITTED"
        );
        let revised = super::delegation::revise_owner_delegation(
            connection,
            owner,
            &DelegationGrantIdentity {
                grant_id: grant.reference.grant_id.clone(),
                revision: grant.reference.revision.clone(),
            },
            DelegationGrantInput {
                principal: grant.principal.clone(),
                binding: grant.binding.clone(),
                expires_at_epoch_ms: expires(),
                ceiling: grant.ceiling.clone(),
            },
        )
        .unwrap();
        assert!(
            prepare_authorized_task_package(connection, &valid).is_err(),
            "a durable replay bypassed current grant resolution"
        );
        let stale = operation(proposal(&revised), "operation-revised");
        let revoked_head = super::delegation::revoke_owner_delegation(
            connection,
            owner,
            &DelegationGrantIdentity {
                grant_id: revised.reference.grant_id.clone(),
                revision: revised.reference.revision.clone(),
            },
        )
        .unwrap();
        assert_eq!(revoked_head, "1");
        assert!(prepare_authorized_task_package(connection, &stale).is_err());
        let newgrant = issue(connection, owner);
        let mut wrong_binding = operation(proposal(&newgrant), "operation-binding");
        wrong_binding.package.source_binding.generation = "8".into();
        wrong_binding.package = draft_for_current(&wrong_binding.package, &newgrant);
        assert!(prepare_authorized_task_package(connection, &wrong_binding).is_err());
        transaction::run(connection, |tx| {
            tx.write(
                "UPDATE main.gogoke_authority_profile SET policy_revision='2' WHERE singleton=1",
                &[],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(prepare_authorized_task_package(
            connection,
            &operation(proposal(&newgrant), "operation-policy")
        )
        .is_err());
    });
}

#[test]
fn payload_hash_tampering_and_temp_or_main_trigger_are_rejected() {
    fixture(|connection, owner| {
        let grant = issue(connection, owner);
        let request = operation(proposal(&grant), "operation-tamper");
        prepare_authorized_task_package(connection, &request).unwrap();
        transaction::run(connection,|tx|{tx.write("UPDATE main.gogoke_objects SET content_hash='sha256:0000000000000000000000000000000000000000000000000000000000000000' WHERE object_type='AuthorizedTaskPackage'",&[])?;Ok(())}).unwrap();
        assert!(
            read_authorized_task_package(connection, "domain-source", "operation-tamper").is_err()
        );
        connection.execute("CREATE TEMP TRIGGER atp_temp_guard AFTER INSERT ON main.gogoke_authorized_task_packages BEGIN SELECT 1; END").unwrap();
        assert!(
            read_authorized_task_package(connection, "domain-source", "operation-tamper").is_err()
        );
    });
}

#[test]
fn main_and_temp_triggers_on_every_write_table_fail_before_a_record_is_written() {
    fixture(|connection, owner| {
        let grant = issue(connection, owner);
        let request = operation(proposal(&grant), "operation-trigger-guard");
        let tables = [
            "gogoke_authorized_task_packages",
            "gogoke_objects",
            "gogoke_events",
            "gogoke_receipts",
            "gogoke_stream_heads",
        ];
        for (index, table) in tables.into_iter().enumerate() {
            let main_trigger = format!("atp_main_guard_{index}");
            connection.execute(&format!("CREATE TRIGGER {main_trigger} BEFORE INSERT ON main.{table} BEGIN SELECT RAISE(ABORT,'main guard'); END")).unwrap();
            assert!(
                matches!(
                    prepare_authorized_task_package(connection, &request),
                    Err(OrchestrationError::AccessDenied)
                ),
                "MAIN trigger on {table} was not rejected by schema preflight"
            );
            connection
                .execute(&format!("DROP TRIGGER {main_trigger}"))
                .unwrap();

            let temp_trigger = format!("atp_temp_guard_{index}");
            connection.execute(&format!("CREATE TEMP TRIGGER {temp_trigger} BEFORE INSERT ON main.{table} BEGIN SELECT RAISE(ABORT,'temp guard'); END")).unwrap();
            assert!(
                matches!(
                    prepare_authorized_task_package(connection, &request),
                    Err(OrchestrationError::AccessDenied)
                ),
                "TEMP trigger on {table} was not rejected by schema preflight"
            );
            connection
                .execute(&format!("DROP TRIGGER {temp_trigger}"))
                .unwrap();
        }
        transaction::run(connection,|tx| {
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_objects WHERE object_type='AuthorizedTaskPackage'",&[],1)?[0][0],"0");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_events WHERE event_type='AuthorizedTaskPackagePrepared'",&[],1)?[0][0],"0");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_receipts WHERE operation_id=?",&[&request.operation_id],1)?[0][0],"0");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_authorized_task_packages WHERE operation_id=?",&[&request.operation_id],1)?[0][0],"0");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_stream_heads WHERE stream_id LIKE 'gogoke.authorized-task-package.v1/%'",&[],1)?[0][0],"0");
            Ok(())
        }).unwrap();
    });
}

#[test]
fn postwrite_cross_read_succeeds_inside_the_outer_transaction_and_rolls_back_with_it() {
    fixture(|connection, owner| {
        let grant = issue(connection, owner);
        let request = operation(proposal(&grant), "operation-cross-read-rollback");
        let aborted: transaction::Result<()> = transaction::run(connection, |tx| {
            let receipt = prepare_in_transaction(tx, &request)?;
            assert_eq!(receipt.disposition, "COMMITTED");
            assert_eq!(
                receipt.authority_status,
                "PREPARATORY_TRUSTED_MATERIAL_REFS"
            );
            assert!(!receipt.canonical_package.is_empty());
            Err(OrchestrationError::AccessDenied)
        });
        assert!(aborted.is_err());
        transaction::run(connection,|tx| {
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_objects WHERE object_type='AuthorizedTaskPackage'",&[],1)?[0][0],"0");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_events WHERE event_type='AuthorizedTaskPackagePrepared'",&[],1)?[0][0],"0");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_receipts WHERE operation_id=?",&[&request.operation_id],1)?[0][0],"0");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_authorized_task_packages WHERE operation_id=?",&[&request.operation_id],1)?[0][0],"0");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_stream_heads WHERE stream_id LIKE 'gogoke.authorized-task-package.v1/%'",&[],1)?[0][0],"0");
            Ok(())
        }).unwrap();
    });
}

#[test]
fn resolver_uses_exact_current_revision_not_a_caller_reconstructed_grant() {
    fixture(|connection, owner| {
        let grant = issue(connection, owner);
        assert_eq!(
            read_current_delegation(connection, &grant.reference.grant_id).unwrap(),
            grant
        );
        let request = operation(proposal(&grant), "operation-grant-current");
        let mut forged = request.clone();
        forged.package.parent_grant_revision = "2".into();
        assert!(prepare_authorized_task_package(connection, &forged).is_err());
    });
}

#[test]
fn rust_canonical_package_matches_the_dedicated_typescript_golden_fixture() {
    let golden = include_str!("fixtures/authorized_task_package.canonical.json").trim();
    let parent = super::delegation::DelegationGrantSnapshot {
        reference: super::model::GrantRef {
            grant_id: "grant-one".into(),
            revision: "3".into(),
            revocation_head: "8".into(),
        },
        issuer_id: "issuer-one".into(),
        parent: None,
        policy_revision: "5".into(),
        principal: DelegationPrincipal {
            principal_id: "principal-owner".into(),
            project_id: "project-one".into(),
            domain_id: "domain-source".into(),
            role: "controller".into(),
            seat_id: "seat-owner".into(),
        },
        binding: DelegationBinding {
            session_id: "session-source".into(),
            execution_id: "execution-source".into(),
            generation: "7".into(),
        },
        expires_at_epoch_ms: 1_800_000_000_000,
        ceiling: AuthorityCeiling {
            allowed_actions: vec!["delegate".into(), "share-material".into()],
            allowed_target_principal_ids: vec!["principal-worker".into()],
            allowed_target_domain_ids: vec!["domain-target".into()],
            allowed_sinks: vec!["task-package".into()],
            allowed_material_classes: vec!["spec".into()],
            explicit_private_material_ids: vec![],
            allowed_continuation_responses: vec!["continue".into()],
            max_material_items: 4,
            max_material_bytes: 1024,
            max_response_bytes: 128,
        },
    };
    let mut p = AuthorizedTaskPackage {
        package_digest: String::new(),
        parent_grant_ref: parent.reference.grant_id.clone(),
        parent_grant_revision: parent.reference.revision.clone(),
        parent_grant_revocation_head: parent.reference.revocation_head.clone(),
        parent_policy_revision: parent.policy_revision.clone(),
        parent_seat_id: parent.principal.seat_id.clone(),
        parent_grant_digest: String::new(),
        parent_ceiling_digest: String::new(),
        child_ceiling: AuthorityCeiling {
            allowed_actions: vec!["delegate".into()],
            allowed_target_principal_ids: vec!["principal-worker".into()],
            allowed_target_domain_ids: vec!["domain-target".into()],
            allowed_sinks: vec!["task-package".into()],
            allowed_material_classes: vec!["spec".into()],
            explicit_private_material_ids: vec![],
            allowed_continuation_responses: vec![],
            max_material_items: 2,
            max_material_bytes: 512,
            max_response_bytes: 64,
        },
        child_ceiling_digest: String::new(),
        action: "delegate".into(),
        route: "controller-worker".into(),
        source: TaskPackagePrincipal {
            principal_id: "principal-owner".into(),
            project_id: "project-one".into(),
            domain_id: "domain-source".into(),
            role: "controller".into(),
        },
        target: TaskPackagePrincipal {
            principal_id: "principal-worker".into(),
            project_id: "project-one".into(),
            domain_id: "domain-target".into(),
            role: "worker".into(),
        },
        source_binding: TaskPackageBinding {
            session_id: "session-source".into(),
            execution_id: "execution-source".into(),
            generation: "7".into(),
        },
        target_binding: TaskPackageBinding {
            session_id: "session-worker".into(),
            execution_id: "execution-worker".into(),
            generation: "1".into(),
        },
        target_binding_kind: "existing".into(),
        sink: "task-package".into(),
        instruction: "Build λ".into(),
        instruction_digest: String::new(),
        material_set_digest: String::new(),
        materials: vec![SelectedMaterial {
            material_id: "material-one".into(),
            material_class: "spec".into(),
            visibility: "project".into(),
            content: "snowman ☃".into(),
            content_digest: digest_canonical("\"snowman ☃\""),
        }],
    };
    p = package_from_current(&p, &parent);
    assert_eq!(package_json(&p), golden.as_bytes());
}
