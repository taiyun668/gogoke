use super::*;
use crate::process::{NativeBinding, PrepareRequest, ProcessCustodian, ProcessLaunch};

const ACTION_OPERATION_ID: &str = "opr_11111111111111111111111111111111";

fn prepare_authorized_action(
    db: &mut VerifiedDatabaseConnection<'_>,
    owner: &super::super::super::OwnerIssuer,
) -> (PrepareActionAuthority, PreparedActionAuthority, u64) {
    super::super::super::initialize_task_context_schema(db).unwrap();
    super::super::super::initialize_authorized_task_package_schema(db).unwrap();
    super::super::super::initialize_session_lineage_schema(db).unwrap();
    super::super::super::initialize_execution_recipe_schema(db).unwrap();
    super::super::super::commit_task_context_requirements(
        db,
        &super::super::super::CommitTaskContextRequirements {
            operation_id: "task-create".into(),
            domain_id: "domain-one".into(),
            task_id: "task-one".into(),
            expected_previous_revision: None,
            mandatory_refs: vec![],
            event_id: "task-event".into(),
            receipt_id: "task-receipt".into(),
            recorded_at: "2026-09-22T00:00:00Z".into(),
        },
    )
    .unwrap();
    let ceiling = super::super::super::AuthorityCeiling {
        allowed_actions: vec!["delegate".into()],
        allowed_target_principal_ids: vec!["principal-worker".into()],
        allowed_target_domain_ids: vec!["domain-one".into()],
        allowed_sinks: vec!["task-package".into()],
        allowed_material_classes: vec![],
        explicit_private_material_ids: vec![],
        allowed_continuation_responses: vec![],
        max_material_items: 0,
        max_material_bytes: 0,
        max_response_bytes: 256,
    };
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 3_600_000;
    let grant = super::super::super::issue_owner_delegation(
        db,
        owner,
        super::super::super::DelegationGrantInput {
            principal: super::super::super::DelegationPrincipal {
                principal_id: owner.principal_id().into(),
                project_id: "project-one".into(),
                domain_id: "domain-one".into(),
                role: "controller".into(),
                seat_id: owner.seat_id().into(),
            },
            binding: super::super::super::DelegationBinding {
                session_id: "session-source".into(),
                execution_id: "execution-source".into(),
                generation: "1".into(),
            },
            expires_at_epoch_ms: expiry,
            ceiling: ceiling.clone(),
        },
    )
    .unwrap();
    let package = super::super::super::prepare_authorized_task_package(
        db,
        &super::super::super::PrepareAuthorizedTaskPackage {
            operation_id: "package-operation".into(),
            domain_id: "domain-one".into(),
            event_id: "package-event".into(),
            receipt_id: "package-receipt".into(),
            recorded_at: "2026-09-22T00:00:00Z".into(),
            package: super::super::super::AuthorizedTaskPackageDraft {
                parent_grant_ref: grant.reference.grant_id.clone(),
                parent_grant_revision: grant.reference.revision.clone(),
                parent_grant_revocation_head: grant.reference.revocation_head.clone(),
                parent_policy_revision: grant.policy_revision.clone(),
                parent_seat_id: grant.principal.seat_id.clone(),
                child_ceiling: ceiling,
                action: "delegate".into(),
                route: "controller-worker".into(),
                source: super::super::super::TaskPackagePrincipal {
                    principal_id: grant.principal.principal_id.clone(),
                    project_id: grant.principal.project_id.clone(),
                    domain_id: grant.principal.domain_id.clone(),
                    role: grant.principal.role.clone(),
                },
                target: super::super::super::TaskPackagePrincipal {
                    principal_id: "principal-worker".into(),
                    project_id: "project-one".into(),
                    domain_id: "domain-one".into(),
                    role: "worker".into(),
                },
                source_binding: super::super::super::TaskPackageBinding {
                    session_id: grant.binding.session_id.clone(),
                    execution_id: grant.binding.execution_id.clone(),
                    generation: grant.binding.generation.clone(),
                },
                target_binding: super::super::super::TaskPackageBinding {
                    session_id: "session-worker".into(),
                    execution_id: "execution-worker".into(),
                    generation: "7".into(),
                },
                target_binding_kind: "existing".into(),
                sink: "task-package".into(),
                instruction: "bounded instruction".into(),
            },
            material_refs: vec![],
        },
    )
    .unwrap();
    assert_eq!(package.disposition, "COMMITTED");
    super::super::super::apply_session_lineage_command(
        db,
        &super::super::super::SessionLineageCommand {
            operation_id: "lineage-operation".into(),
            domain_id: "domain-one".into(),
            event_id: "lineage-event".into(),
            receipt_id: "lineage-receipt".into(),
            recorded_at: "2026-09-22T00:00:00Z".into(),
            operation: super::super::super::SessionLineageOperation::NewClean {
                session_id: "session-worker".into(),
                native: super::super::super::NativeSessionIdentity {
                    native_session_id: "native-worker".into(),
                    binding_id: "binding-worker".into(),
                    generation: "7".into(),
                    source_epoch: "9".into(),
                    domain_id: "domain-one".into(),
                },
            },
        },
    )
    .unwrap();
    super::super::super::append_owner_execution_recipe(
        db,
        owner,
        &super::super::super::AppendExecutionRecipe {
            operation_id: "recipe-operation".into(),
            domain_id: "domain-one".into(),
            expected_previous_revision: None,
            recipe_id: "recipe-one".into(),
            seat_id: "seat-worker".into(),
            runtime_instance_id: "runtime-one".into(),
            model_ref: BTreeMap::new(),
            tool_profile: super::super::super::RecipeJsonValue::Null,
            isolation_profile: super::super::super::RecipeJsonValue::Null,
            context_manifest_id: "manifest-one".into(),
            budget_policy: super::super::super::RecipeJsonValue::Null,
            admission_ref: grant.reference.grant_id.clone(),
            event_id: "recipe-event".into(),
            receipt_id: "recipe-receipt".into(),
            recorded_at: "2026-09-22T00:00:00Z".into(),
        },
    )
    .unwrap();
    let request = PrepareActionAuthority {
        domain_id: "domain-one".into(),
        parent_grant_ref: grant.reference.grant_id,
        package_operation_id: "package-operation".into(),
        task_id: "task-one".into(),
        recipe_id: "recipe-one".into(),
        session_id: "session-worker".into(),
        context_manifest_id: "manifest-one".into(),
        action_operation_id: ACTION_OPERATION_ID.into(),
        reservation_id: "action-reservation".into(),
        action_kind: "queue".into(),
        lane: "work".into(),
        payload: b"bounded instruction".to_vec(),
    };
    let (action_digest, policy_revision) = super::super::super::transaction::run(db, |tx| {
        let (resolved, task, _, recipe, profile) = current_selection(tx, &request)?;
        let payload_digest = crate::store::digest::content_hash(&request.payload);
        Ok((
            intent_digest(
                &request,
                &resolved.package_digest,
                &task.task_revision,
                &recipe.recipe.revision,
                &recipe.content_hash,
                &profile.policy_revision,
                &payload_digest,
            ),
            profile.policy_revision,
        ))
    })
    .unwrap();
    let decision_record = super::super::super::DurableDecisionRecord {
        operation_id: "decision-operation".into(),
        scenario_id: "DF02".into(),
        family: "RESOURCE_SELECTION".into(),
        state_view_hash: format!("sha256:{}", "a".repeat(64)),
        candidate_hash: format!("sha256:{}", "b".repeat(64)),
        question_version: "1".into(),
        rubric_version: "1".into(),
        model_requested: None,
        model_resolved: Some("fixture-fake".into()),
        task_revision: "1".into(),
        policy_revision: policy_revision.clone(),
        capability_revision: "1".into(),
        binding_generation: "7".into(),
        backend_kind: "FAKE".into(),
        choice: "candidate-one".into(),
        reason: "QUALIFIED_BOUNDED_SELECTION".into(),
        budget_units: 1,
        deadline_epoch_ms: 9_007_199_254_740_000,
    };
    super::super::super::publish_decision_snapshot(
        db,
        &super::super::super::DecisionAuthoritySnapshot {
            operation_id: decision_record.operation_id.clone(),
            candidate_id: decision_record.choice.clone(),
            state_view_hash: decision_record.state_view_hash.clone(),
            candidate_hash: decision_record.candidate_hash.clone(),
            task_revision: decision_record.task_revision.clone(),
            policy_revision: decision_record.policy_revision.clone(),
            capability_revision: decision_record.capability_revision.clone(),
            binding_id: "binding-worker".into(),
            binding_generation: decision_record.binding_generation.clone(),
            auth_revision: policy_revision,
            resource_ref: "pool-one".into(),
            resource_revision: "1".into(),
            capacity_total: 2,
            action_operation_id: ACTION_OPERATION_ID.into(),
            action_digest,
        },
    )
    .unwrap();
    super::super::super::commit_decision(
        db,
        &super::super::super::DecisionCommitInput {
            domain_id: "domain-one".into(),
            decision_id: "decision-one".into(),
            event_id: "decision-event".into(),
            receipt_id: "decision-receipt".into(),
            recorded_at: "2026-09-22T00:00:00Z".into(),
            record: decision_record,
            resource_reservation_ref: "capacity-lease-one".into(),
            action_intent_ref: ACTION_OPERATION_ID.into(),
            required_capacity_units: 1,
        },
    )
    .unwrap();
    let prepared = prepare_action_authority(db, &request).unwrap();
    assert_eq!(prepared.disposition, "COMMITTED");
    persist_manifest(db, owner, &request, &prepared.semantic_digest);
    (request, prepared, expiry)
}

#[test]
fn derives_current_facts_from_authority_and_exact_active_process_identity() {
    fixture(|_, db, owner| {
        let (action, _, admission_expiry) = prepare_authorized_action(db, owner);
        let product = super::super::super::read_product_identity(db, owner).unwrap();
        super::super::super::initialize_process_custody_schema(db).unwrap();

        let windows = std::env::var_os("WINDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
        let command = windows.join("System32").join("cmd.exe");
        let digest = crate::store::digest::content_hash(&std::fs::read(&command).unwrap());
        let mut launch = ProcessLaunch::new(&command);
        launch.arguments = vec!["/D".into(), "/C".into(), "ping -n 30 127.0.0.1 >nul".into()];
        let request = PrepareRequest {
            launch,
            binding: NativeBinding {
                binary_digest_sha256: digest,
                profile_id: product.profile_id,
                domain_id: "domain-one".into(),
                generation: "7".into(),
            },
        };
        let mut custodian = ProcessCustodian::new().unwrap();
        let prepared = custodian.prepare(&request).unwrap();
        super::super::super::record_prepared_process(db, ACTION_OPERATION_ID, &prepared).unwrap();
        let references = NativeActionCurrentFactsRefs {
            domain_id: action.domain_id.clone(),
            operation_id: action.action_operation_id.clone(),
            reservation_id: action.reservation_id.clone(),
        };
        assert!(
            derive_native_action_current_facts(db, &references, &prepared, &prepared.identity,)
                .is_err()
        );
        custodian.activate(&prepared).unwrap();
        super::super::super::mark_process_active(db, ACTION_OPERATION_ID, &prepared).unwrap();
        let active_identity = custodian
            .active(&prepared.ticket)
            .unwrap()
            .identity()
            .clone();

        let mut wrong_identity = active_identity.clone();
        wrong_identity.pid = wrong_identity.pid.saturating_add(1);
        assert!(
            derive_native_action_current_facts(db, &references, &prepared, &wrong_identity,)
                .is_err()
        );
        assert_eq!(
            count(db, "SELECT count(*) FROM main.gogoke_action_current_facts",),
            "0"
        );

        assert_eq!(
            derive_native_action_current_facts(db, &references, &prepared, &active_identity)
                .unwrap(),
            "1"
        );
        assert_eq!(
            derive_native_action_current_facts(db, &references, &prepared, &active_identity)
                .unwrap(),
            "1",
            "an exact native replay does not manufacture a new facts revision"
        );
        assert_eq!(
            count(
                db,
                "SELECT capability_revision FROM main.gogoke_action_current_facts",
            ),
            "1"
        );
        assert_eq!(
            count(
                db,
                "SELECT expires_at_epoch_ms FROM main.gogoke_action_current_facts",
            ),
            admission_expiry.to_string()
        );
        assert!(matches!(
            begin_committed_action(
                db,
                &BeginCommittedAction {
                    domain_id: references.domain_id,
                    operation_id: references.operation_id,
                    reservation_id: references.reservation_id,
                },
            )
            .unwrap(),
            BeginCommittedDisposition::Granted { .. }
        ));
    });
}
