//! Product-Authority-owned Action dispatch and completion facts.
//!
//! The existing ActionStore remains a private persistence primitive. This
//! module resolves references against current durable authority and is the only
//! production path allowed to mint a send authority or record completion.

use super::super::action::{ActionCommitment, ActionReservation, ReserveDisposition};
use super::super::atomic::DomainRecordInput;
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;
use super::bootstrap::Profile;
use super::catalog::current_profile;
use super::model::{denied, identifier};
#[cfg(test)]
use super::model::{next_revision, revision};
use super::transaction::{self, Result, Transaction};
use crate::process::{PreparedCustody, ProcessIdentity};

pub(crate) const ACTION_AUTHORITY_STATUS: &str = "PREPARATORY_CURRENT_FACTS_REQUIRED";
pub(crate) const COMPLETION_AUTHORITY_STATUS: &str = "PREPARATORY_TRUSTED_NATIVE_RECEIPT_REQUIRED";

const INTENT_SCHEMA: &str = "CREATE TABLE gogoke_action_authority_intents (domain_id TEXT NOT NULL,target_domain_id TEXT NOT NULL,operation_id TEXT NOT NULL,package_operation_id TEXT NOT NULL,package_digest TEXT NOT NULL,parent_grant_ref TEXT NOT NULL,task_id TEXT NOT NULL,task_revision TEXT NOT NULL,recipe_id TEXT NOT NULL,session_id TEXT NOT NULL,binding_id TEXT NOT NULL,generation TEXT NOT NULL,source_epoch TEXT NOT NULL,runtime_instance_id TEXT NOT NULL,context_manifest_id TEXT NOT NULL,auth_revision TEXT NOT NULL,action_kind TEXT NOT NULL CHECK(action_kind IN ('queue','steer','interrupt','close')),lane TEXT NOT NULL CHECK(lane IN ('control','work')),payload_digest TEXT NOT NULL CHECK(length(payload_digest)=71),attempt_id TEXT,send_authority TEXT,CHECK((attempt_id IS NULL AND send_authority IS NULL) OR (attempt_id IS NOT NULL AND send_authority IS NOT NULL)),PRIMARY KEY(domain_id,operation_id),UNIQUE(domain_id,package_operation_id,operation_id),FOREIGN KEY(operation_id) REFERENCES gogoke_action_reservations(operation_id) ON DELETE RESTRICT ON UPDATE RESTRICT,FOREIGN KEY(domain_id,package_operation_id) REFERENCES gogoke_authorized_task_packages(domain_id,operation_id) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";
const COMPLETION_SCHEMA: &str = "CREATE TABLE gogoke_action_completion_receipts (domain_id TEXT NOT NULL,operation_id TEXT NOT NULL,reservation_id TEXT NOT NULL,semantic_digest TEXT NOT NULL CHECK(length(semantic_digest)=71),attempt_id TEXT NOT NULL,send_authority TEXT NOT NULL,binding_id TEXT NOT NULL,generation TEXT NOT NULL,source_epoch TEXT NOT NULL,runtime_instance_id TEXT NOT NULL,native_request_id TEXT NOT NULL,native_session_id TEXT NOT NULL,trusted_receipt_ref TEXT NOT NULL,evidence_hash TEXT NOT NULL CHECK(length(evidence_hash)=71),disposition TEXT NOT NULL CHECK(disposition IN ('COMPLETED','REJECTED','ACCEPTANCE_UNKNOWN')),receipt_id TEXT NOT NULL,PRIMARY KEY(domain_id,operation_id),UNIQUE(domain_id,reservation_id),FOREIGN KEY(operation_id) REFERENCES gogoke_action_reservations(operation_id) ON DELETE RESTRICT ON UPDATE RESTRICT,FOREIGN KEY(domain_id,receipt_id) REFERENCES gogoke_receipts(domain_id,receipt_id) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";
const CURRENT_FACTS_SCHEMA: &str = "CREATE TABLE gogoke_action_current_facts (domain_id TEXT NOT NULL,operation_id TEXT NOT NULL,facts_revision TEXT NOT NULL,policy_revision TEXT NOT NULL,revocation_head TEXT NOT NULL,binding_id TEXT NOT NULL,generation TEXT NOT NULL,source_epoch TEXT NOT NULL,runtime_instance_id TEXT NOT NULL,model_ref_digest TEXT NOT NULL CHECK(length(model_ref_digest)=71),capability_revision TEXT NOT NULL,context_manifest_id TEXT NOT NULL,context_manifest_hash TEXT NOT NULL CHECK(length(context_manifest_hash)=71),admission_ref TEXT NOT NULL,admission_revision TEXT NOT NULL,expires_at_epoch_ms TEXT NOT NULL,PRIMARY KEY(domain_id,operation_id),FOREIGN KEY(operation_id) REFERENCES gogoke_action_reservations(operation_id) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";
const NATIVE_RECEIPT_SCHEMA: &str = "CREATE TABLE gogoke_action_native_receipts (domain_id TEXT NOT NULL,receipt_ref TEXT NOT NULL,operation_id TEXT NOT NULL,reservation_id TEXT NOT NULL,semantic_digest TEXT NOT NULL,attempt_id TEXT NOT NULL,send_authority TEXT NOT NULL,binding_id TEXT NOT NULL,generation TEXT NOT NULL,source_epoch TEXT NOT NULL,runtime_instance_id TEXT NOT NULL,native_request_id TEXT NOT NULL,native_session_id TEXT NOT NULL,evidence_hash TEXT NOT NULL,disposition TEXT NOT NULL CHECK(disposition IN ('COMPLETED','REJECTED','ACCEPTANCE_UNKNOWN')),receipt_id TEXT NOT NULL,PRIMARY KEY(domain_id,receipt_ref),UNIQUE(domain_id,operation_id),FOREIGN KEY(operation_id) REFERENCES gogoke_action_reservations(operation_id) ON DELETE RESTRICT ON UPDATE RESTRICT,FOREIGN KEY(domain_id,receipt_id) REFERENCES gogoke_receipts(domain_id,receipt_id) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";

fn ensure_schema(tx: &mut Transaction<'_, '_>) -> Result<()> {
    for (name, ddl) in [
        ("gogoke_action_authority_intents", INTENT_SCHEMA),
        ("gogoke_action_completion_receipts", COMPLETION_SCHEMA),
        ("gogoke_action_current_facts", CURRENT_FACTS_SCHEMA),
        ("gogoke_action_native_receipts", NATIVE_RECEIPT_SCHEMA),
    ] {
        let rows = tx.query(
            "SELECT type,sql FROM main.sqlite_schema WHERE name=?",
            &[name],
            2,
        )?;
        let temp = tx.query("SELECT name FROM temp.sqlite_schema WHERE (lower(name)=lower(?) AND type IN ('table','view')) OR (lower(tbl_name)=lower(?) AND type='trigger')", &[name,name], 1)?;
        if !temp.is_empty() {
            return denied();
        }
        if rows.is_empty() {
            tx.write(ddl, &[])?;
        } else if rows.len() != 1 || rows[0][0] != "table" || rows[0][1] != ddl {
            return denied();
        }
        if !tx
            .query(
                "SELECT name FROM main.sqlite_schema WHERE type='trigger' AND lower(tbl_name)=lower(?)",
                &[name],
                1,
            )?
            .is_empty()
        {
            return denied();
        }
    }
    Ok(())
}

fn current_admission_in_transaction(
    tx: &mut Transaction<'_, '_>,
    profile: &Profile,
    grant_id: &str,
) -> Result<super::delegation::DelegationGrantSnapshot> {
    identifier(grant_id)?;
    let heads = tx.query(
        "SELECT revision FROM main.gogoke_authority_grant_heads WHERE grant_id=? AND revoked=0",
        &[grant_id],
        1,
    )?;
    if heads.len() != 1 {
        return denied();
    }
    super::delegation::current_in_transaction(
        tx,
        profile,
        &super::delegation::DelegationGrantIdentity {
            grant_id: grant_id.to_owned(),
            revision: heads[0][0].clone(),
        },
    )
}

fn current_manifest_hash_in_transaction(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    manifest_id: &str,
) -> Result<String> {
    super::context_manifest::resolve_current_manifest_in_transaction(tx, domain_id, manifest_id)
}

fn hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn intent_digest(
    request: &PrepareActionAuthority,
    package_digest: &str,
    task_revision: &str,
    recipe_revision: &str,
    recipe_hash: &str,
    current_policy_revision: &str,
    payload_digest: &str,
) -> String {
    let values = [
        request.domain_id.as_str(),
        request.action_operation_id.as_str(),
        request.package_operation_id.as_str(),
        package_digest,
        request.parent_grant_ref.as_str(),
        request.task_id.as_str(),
        task_revision,
        request.recipe_id.as_str(),
        recipe_revision,
        recipe_hash,
        request.session_id.as_str(),
        request.context_manifest_id.as_str(),
        current_policy_revision,
        request.action_kind.as_str(),
        request.lane.as_str(),
        payload_digest,
    ];
    let mut bytes = String::from("gogoke.action-intent.v1|");
    for value in values {
        bytes.push_str(&value.len().to_string());
        bytes.push(':');
        bytes.push_str(value);
    }
    content_hash(bytes.as_bytes())
}

fn package_commitment(package: &super::task_package::AuthorizedTaskPackage) -> ActionCommitment {
    ActionCommitment {
        package_digest: package.package_digest.clone(),
        parent_grant_ref: package.parent_grant_ref.clone(),
        parent_grant_revision: package.parent_grant_revision.clone(),
        parent_ceiling_digest: package.parent_ceiling_digest.clone(),
        child_ceiling_digest: package.child_ceiling_digest.clone(),
        source_principal_id: package.source.principal_id.clone(),
        source_project_id: package.source.project_id.clone(),
        source_domain_id: package.source.domain_id.clone(),
        source_role: package.source.role.clone(),
        target_principal_id: package.target.principal_id.clone(),
        target_project_id: package.target.project_id.clone(),
        target_domain_id: package.target.domain_id.clone(),
        target_role: package.target.role.clone(),
        source_session_id: package.source_binding.session_id.clone(),
        source_execution_id: package.source_binding.execution_id.clone(),
        source_generation: package.source_binding.generation.clone(),
        child_session_id: package.target_binding.session_id.clone(),
        child_execution_id: package.target_binding.execution_id.clone(),
        child_generation: package.target_binding.generation.clone(),
        route: package.route.clone(),
        policy_action: package.action.clone(),
        sink: package.sink.clone(),
        material_set_digest: package.material_set_digest.clone(),
        instruction_digest: package.instruction_digest.clone(),
    }
}

fn current_selection(
    tx: &mut Transaction<'_, '_>,
    request: &PrepareActionAuthority,
) -> Result<(
    super::task_package::AuthorizedTaskPackage,
    super::task_context::TaskContextRequirements,
    super::session_lineage::SessionSnapshot,
    super::execution_recipe::ExecutionRecipeVersion,
    Profile,
)> {
    for value in [
        &request.domain_id,
        &request.parent_grant_ref,
        &request.package_operation_id,
        &request.task_id,
        &request.recipe_id,
        &request.session_id,
        &request.context_manifest_id,
        &request.action_operation_id,
        &request.reservation_id,
    ] {
        identifier(value)?;
    }
    let profile = current_profile(tx)?;
    let package = super::task_package::read_authorized_task_package_in_transaction(
        tx,
        &request.domain_id,
        &request.package_operation_id,
    )?
    .ok_or(OrchestrationError::AccessDenied)?;
    if package.parent_grant_ref != request.parent_grant_ref
        || package.source.domain_id != request.domain_id
        || package.target_binding.session_id != request.session_id
    {
        return denied();
    }
    let task = super::task_context::read_current_task_context_in_transaction(
        tx,
        &request.domain_id,
        &request.task_id,
    )?;
    let lineage = super::session_lineage::read_session_lineage_in_transaction(
        tx,
        &package.target.domain_id,
        &request.session_id,
    )?;
    if lineage.lifecycle != "ACTIVE"
        || lineage.lineage.session_id != request.session_id
        || lineage.native.generation != package.target_binding.generation
    {
        return denied();
    }
    let recipe = super::execution_recipe::read_current_execution_recipe_in_transaction(
        tx,
        &package.target.domain_id,
        &request.recipe_id,
    )?
    .ok_or(OrchestrationError::AccessDenied)?;
    if recipe.recipe.context_manifest_id != request.context_manifest_id
        || recipe.recipe.seat_id.is_empty()
    {
        return denied();
    }
    Ok((package, task, lineage, recipe, profile))
}

/// Preparatory Decision input derived from current Product Authority records.
/// It does not reserve an Action or grant send authority; Action prepare/begin
/// must recheck the same records after the Decision is committed.
pub(crate) fn derive_action_decision_basis(
    connection: &mut VerifiedDatabaseConnection<'_>,
    request: &PrepareActionAuthority,
) -> Result<ActionDecisionBasis> {
    if request.action_kind != "queue" || request.lane != "work" {
        return denied();
    }
    transaction::run(connection, |tx| {
        let (package, task, lineage, recipe, profile) = current_selection(tx, request)?;
        if request.payload.as_slice() != package.instruction.as_bytes() {
            return denied();
        }
        let payload_digest = content_hash(&request.payload);
        let semantic_digest = intent_digest(request, &package.package_digest,
            &task.task_revision, &recipe.recipe.revision, &recipe.content_hash,
            &profile.policy_revision, &payload_digest);
        let state_view_hash = content_hash(format!(
            "r2-02-decision-state:{}:{}:{}:{}",
            package.package_digest, task.task_revision,
            recipe.content_hash, lineage.native.generation,
        ).as_bytes());
        Ok(ActionDecisionBasis {
            semantic_digest,
            state_view_hash,
            task_revision: task.task_revision,
            policy_revision: profile.policy_revision,
            binding_id: lineage.native.binding_id,
            generation: lineage.native.generation,
        })
    })
}

pub(crate) fn prepare_action_authority(
    connection: &mut VerifiedDatabaseConnection<'_>,
    request: &PrepareActionAuthority,
) -> Result<PreparedActionAuthority> {
    if !matches!(
        request.action_kind.as_str(),
        "queue" | "steer" | "interrupt" | "close"
    ) || !matches!(request.lane.as_str(), "control" | "work")
    {
        return denied();
    }
    super::super::action::apply_action_schema(connection)?;
    transaction::run(connection, |tx| {
        ensure_schema(tx)?;
        let (package, task, lineage, recipe, profile) = current_selection(tx, request)?;
        if request.payload.as_slice() != package.instruction.as_bytes() {
            return denied();
        }
        let payload_digest = content_hash(&request.payload);
        let semantic_digest = intent_digest(
            request,
            &package.package_digest,
            &task.task_revision,
            &recipe.recipe.revision,
            &recipe.content_hash,
            &profile.policy_revision,
            &payload_digest,
        );
        let leases = tx.query(
            "SELECT operation_id,candidate_id,resource_ref,resource_revision,resource_reservation_ref FROM main.gogoke_decision_capacity_leases WHERE action_operation_id=?",
            &[&request.action_operation_id],
            5,
        )?;
        if leases.len() != 1 {
            return denied();
        }
        let decision = super::decision_replay::read_in_transaction(
            tx,
            &request.domain_id,
            &leases[0][0],
        )?;
        let decision_snapshot = tx.query(
            "SELECT action_digest,binding_id,binding_generation,policy_revision,task_revision FROM main.gogoke_decision_authority_snapshots WHERE operation_id=? AND candidate_id=?",
            &[&leases[0][0], &leases[0][1]],
            5,
        )?;
        if decision_snapshot.len() != 1
            || decision.action_intent_ref != request.action_operation_id
            || decision.record.choice != leases[0][1]
            || decision.resource_reservation_ref != leases[0][4]
            || decision.record.task_revision != task.task_revision
            || decision.record.policy_revision != profile.policy_revision
            || decision.record.binding_generation != package.target_binding.generation
            || decision_snapshot[0][0] != semantic_digest
            || decision_snapshot[0][1] != lineage.native.binding_id
            || decision_snapshot[0][2] != lineage.native.generation
            || decision_snapshot[0][3] != profile.policy_revision
            || decision_snapshot[0][4] != task.task_revision
        {
            return denied();
        }
        let commitment = package_commitment(&package);
        let reservation = ActionReservation {
            operation_id: request.action_operation_id.clone(),
            semantic_digest: semantic_digest.clone(),
            reservation_id: request.reservation_id.clone(),
            binding_id: lineage.native.binding_id.clone(),
            session_id: package.target_binding.session_id.clone(),
            execution_id: package.target_binding.execution_id.clone(),
            runtime_instance_id: recipe.recipe.runtime_instance_id.clone(),
            profile_id: profile.profile_id,
            auth_revision: profile.policy_revision.clone(),
            generation: package.target_binding.generation.clone(),
            lane: request.lane.clone(),
            action_kind: request.action_kind.clone(),
            payload_hex: hex(&request.payload),
            commitment,
        };
        let storage = tx.reserve_action(reservation)?;
        if matches!(storage, ReserveDisposition::Conflict { .. }) {
            return Err(OrchestrationError::OperationConflict);
        }
        tx.write("INSERT INTO main.gogoke_action_authority_intents(domain_id,target_domain_id,operation_id,package_operation_id,package_digest,parent_grant_ref,task_id,task_revision,recipe_id,session_id,binding_id,generation,source_epoch,runtime_instance_id,context_manifest_id,auth_revision,action_kind,lane,payload_digest) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(domain_id,operation_id) DO NOTHING", &[&request.domain_id,&package.target.domain_id,&request.action_operation_id,&request.package_operation_id,&package.package_digest,&request.parent_grant_ref,&request.task_id,&task.task_revision,&request.recipe_id,&request.session_id,&lineage.native.binding_id,&package.target_binding.generation,&lineage.native.source_epoch,&recipe.recipe.runtime_instance_id,&request.context_manifest_id,&profile.policy_revision,&request.action_kind,&request.lane,&payload_digest])?;
        let rows=tx.query("SELECT package_operation_id,package_digest,parent_grant_ref,task_id,task_revision,recipe_id,session_id,binding_id,generation,source_epoch,runtime_instance_id,context_manifest_id,auth_revision,action_kind,lane,payload_digest,target_domain_id FROM main.gogoke_action_authority_intents WHERE domain_id=? AND operation_id=?", &[&request.domain_id,&request.action_operation_id],17)?;
        if rows.len() != 1
            || rows[0][0] != request.package_operation_id
            || rows[0][1] != package.package_digest
            || rows[0][2] != request.parent_grant_ref
            || rows[0][3] != request.task_id
            || rows[0][4] != task.task_revision
            || rows[0][5] != request.recipe_id
            || rows[0][6] != request.session_id
            || rows[0][7] != lineage.native.binding_id
            || rows[0][8] != package.target_binding.generation
            || rows[0][9] != lineage.native.source_epoch
            || rows[0][10] != recipe.recipe.runtime_instance_id
            || rows[0][11] != request.context_manifest_id
            || rows[0][12] != profile.policy_revision
            || rows[0][13] != request.action_kind
            || rows[0][14] != request.lane
            || rows[0][15] != payload_digest
            || rows[0][16] != package.target.domain_id
        {
            return Err(OrchestrationError::OperationConflict);
        }
        let (disposition, reservation_state) = match storage {
            ReserveDisposition::Reserved => ("COMMITTED", "reserved".to_owned()),
            ReserveDisposition::Replay { state } => ("REPLAYED", state),
            ReserveDisposition::Conflict { .. } => return Err(OrchestrationError::OperationConflict),
        };
        Ok(PreparedActionAuthority {
            disposition,
            reservation_state,
            authority_status: ACTION_AUTHORITY_STATUS,
            operation_id: request.action_operation_id.clone(),
            reservation_id: request.reservation_id.clone(),
            semantic_digest,
            package_digest: package.package_digest,
        })
    })
}

/// Resolves the already reserved Action's fixture launch coordinates inside
/// Product Authority. This is a pre-commit read: begin still rechecks every
/// current fact immediately before the sole protocol write.
pub(crate) fn read_native_action_fixture_selection(
    connection: &mut VerifiedDatabaseConnection<'_>,
    references: &NativeActionCurrentFactsRefs,
) -> Result<NativeActionFixtureSelection> {
    for value in [
        &references.domain_id,
        &references.operation_id,
        &references.reservation_id,
    ] {
        identifier(value)?;
    }
    transaction::run(connection, |tx| {
        ensure_schema(tx)?;
        let intents = tx.query(
            "SELECT package_operation_id,parent_grant_ref,task_id,recipe_id,session_id,context_manifest_id,action_kind,lane,payload_digest FROM main.gogoke_action_authority_intents WHERE domain_id=? AND operation_id=?",
            &[&references.domain_id, &references.operation_id],
            9,
        )?;
        let actions = tx.query(
            "SELECT semantic_digest,reservation_id,profile_id,generation,payload_hex,state,action_kind,lane,session_id FROM main.gogoke_action_reservations WHERE operation_id=?",
            &[&references.operation_id],
            9,
        )?;
        if intents.len() != 1 || actions.len() != 1
            || actions[0][1] != references.reservation_id
            || actions[0][5] != "reserved"
            || actions[0][6] != intents[0][6]
            || actions[0][7] != intents[0][7]
            || actions[0][8] != intents[0][4]
        {
            return denied();
        }
        let payload = decode_hex(&actions[0][4])?;
        let selection = PrepareActionAuthority {
            domain_id: references.domain_id.clone(),
            parent_grant_ref: intents[0][1].clone(),
            package_operation_id: intents[0][0].clone(),
            task_id: intents[0][2].clone(),
            recipe_id: intents[0][3].clone(),
            session_id: intents[0][4].clone(),
            context_manifest_id: intents[0][5].clone(),
            action_operation_id: references.operation_id.clone(),
            reservation_id: references.reservation_id.clone(),
            action_kind: intents[0][6].clone(),
            lane: intents[0][7].clone(),
            payload,
        };
        let (package, task, lineage, recipe, profile) = current_selection(tx, &selection)?;
        let payload_digest = content_hash(&selection.payload);
        if selection.payload.as_slice() != package.instruction.as_bytes()
            || intents[0][8] != payload_digest
            || actions[0][2] != profile.profile_id
            || actions[0][3] != lineage.native.generation
            || actions[0][0] != intent_digest(
                &selection, &package.package_digest, &task.task_revision,
                &recipe.recipe.revision, &recipe.content_hash,
                &profile.policy_revision, &payload_digest,
            )
        {
            return denied();
        }
        let launch_digest_sha256 = if recipe.recipe.recipe_id == "recipe-r2-02-test"
            && recipe.recipe.runtime_instance_id != super::r2_fixture_driver::FIXED_RUNTIME_INSTANCE_ID {
            super::r2_fixture_driver::resolve_in_transaction(tx, &recipe.recipe.runtime_instance_id)?.launch_digest_sha256
        } else {
            super::r2_fixture_driver::LAUNCH_DIGEST_SHA256.to_owned()
        };
        Ok(NativeActionFixtureSelection {
            profile_id: profile.profile_id,
            target_domain_id: package.target.domain_id,
            generation: lineage.native.generation,
            binding_id: lineage.native.binding_id,
            source_epoch: lineage.native.source_epoch,
            native_session_id: lineage.native.native_session_id,
            runtime_instance_id: recipe.recipe.runtime_instance_id,
            launch_digest_sha256,
            semantic_digest: actions[0][0].clone(),
            payload: selection.payload,
        })
    })
}

#[cfg(test)]
/// Legacy test seam retained only for the pre-existing authority tests. Product
/// code derives these values in `derive_native_action_current_facts`.
pub(super) fn record_trusted_native_action_facts(
    connection: &mut VerifiedDatabaseConnection<'_>,
    facts: &TrustedNativeActionFacts,
) -> Result<String> {
    for value in [
        &facts.domain_id,
        &facts.operation_id,
        &facts.session_id,
        &facts.binding_id,
        &facts.runtime_instance_id,
        &facts.context_manifest_id,
        &facts.admission_ref,
    ] {
        identifier(value)?;
    }
    for value in [
        &facts.generation,
        &facts.source_epoch,
        &facts.capability_revision,
        &facts.admission_revision,
    ] {
        revision(value)?;
    }
    for value in [&facts.model_ref_digest, &facts.context_manifest_hash] {
        if value.len() != 71
            || !value.starts_with("sha256:")
            || !value[7..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return denied();
        }
    }
    if facts.expires_at_epoch_ms <= now_epoch_ms()? {
        return denied();
    }
    transaction::run(connection, |tx| {
        ensure_schema(tx)?;
        let intent=tx.query("SELECT package_operation_id,parent_grant_ref,task_id,recipe_id,session_id,context_manifest_id,action_kind,lane,auth_revision,target_domain_id FROM main.gogoke_action_authority_intents WHERE domain_id=? AND operation_id=?", &[&facts.domain_id,&facts.operation_id],10)?;
        let action=tx.query("SELECT reservation_id,binding_id,session_id,execution_id,runtime_instance_id,generation,lane,action_kind,payload_hex,state FROM main.gogoke_action_reservations WHERE operation_id=?", &[&facts.operation_id],10)?;
        if intent.len() != 1
            || action.len() != 1
            || action[0][9] != "reserved"
            || action[0][1] != facts.binding_id
            || action[0][2] != facts.session_id
            || action[0][4] != facts.runtime_instance_id
            || action[0][5] != facts.generation
            || intent[0][4] != facts.session_id
            || intent[0][5] != facts.context_manifest_id
        {
            return denied();
        }
        let payload = decode_hex(&action[0][8])?;
        let selection = PrepareActionAuthority {
            domain_id: facts.domain_id.clone(),
            parent_grant_ref: intent[0][1].clone(),
            package_operation_id: intent[0][0].clone(),
            task_id: intent[0][2].clone(),
            recipe_id: intent[0][3].clone(),
            session_id: intent[0][4].clone(),
            context_manifest_id: intent[0][5].clone(),
            action_operation_id: facts.operation_id.clone(),
            reservation_id: action[0][0].clone(),
            action_kind: intent[0][6].clone(),
            lane: intent[0][7].clone(),
            payload,
        };
        let (package, task, lineage, recipe, profile) = current_selection(tx, &selection)?;
        let manifest_hash = current_manifest_hash_in_transaction(
            tx,
            &facts.domain_id,
            &facts.context_manifest_id,
        )?;
        let admission =
            current_admission_in_transaction(tx, &profile, &recipe.recipe.admission_ref)?;
        if package.target_binding.session_id != facts.session_id
            || package.target_binding.generation != facts.generation
            || lineage.native.binding_id != facts.binding_id
            || lineage.native.generation != facts.generation
            || lineage.native.source_epoch != facts.source_epoch
            || recipe.recipe.runtime_instance_id != facts.runtime_instance_id
            || recipe.recipe.context_manifest_id != facts.context_manifest_id
            || facts.context_manifest_hash != manifest_hash
            || super::execution_recipe::model_ref_digest(&recipe.recipe)? != facts.model_ref_digest
            || recipe.recipe.admission_ref != facts.admission_ref
            || package.parent_grant_ref != admission.reference.grant_id
            || facts.admission_ref != admission.reference.grant_id
            || facts.admission_revision != admission.reference.revision
        {
            return denied();
        }
        let head=tx.query("SELECT facts_revision FROM main.gogoke_action_current_facts WHERE domain_id=? AND operation_id=?", &[&facts.domain_id,&facts.operation_id],1)?;
        let previous = match (head.first(), facts.expected_previous_revision.as_deref()) {
            (None, None) => None,
            (Some(row), Some(expected)) if row[0] == expected => Some(row[0].clone()),
            _ => return Err(OrchestrationError::StreamConflict),
        };
        let next = previous
            .as_deref()
            .map(next_revision)
            .transpose()?
            .unwrap_or_else(|| "1".into());
        if let Some(previous) = previous {
            tx.write("UPDATE main.gogoke_action_current_facts SET facts_revision=?,policy_revision=?,revocation_head=?,binding_id=?,generation=?,source_epoch=?,runtime_instance_id=?,model_ref_digest=?,capability_revision=?,context_manifest_id=?,context_manifest_hash=?,admission_ref=?,admission_revision=?,expires_at_epoch_ms=? WHERE domain_id=? AND operation_id=? AND facts_revision=?", &[&next,&profile.policy_revision,&profile.revocation_head,&facts.binding_id,&facts.generation,&facts.source_epoch,&facts.runtime_instance_id,&facts.model_ref_digest,&facts.capability_revision,&facts.context_manifest_id,&manifest_hash,&admission.reference.grant_id,&admission.reference.revision,&facts.expires_at_epoch_ms.to_string(),&facts.domain_id,&facts.operation_id,&previous])?;
        } else {
            tx.write("INSERT INTO main.gogoke_action_current_facts(domain_id,operation_id,facts_revision,policy_revision,revocation_head,binding_id,generation,source_epoch,runtime_instance_id,model_ref_digest,capability_revision,context_manifest_id,context_manifest_hash,admission_ref,admission_revision,expires_at_epoch_ms) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", &[&facts.domain_id,&facts.operation_id,&next,&profile.policy_revision,&profile.revocation_head,&facts.binding_id,&facts.generation,&facts.source_epoch,&facts.runtime_instance_id,&facts.model_ref_digest,&facts.capability_revision,&facts.context_manifest_id,&manifest_hash,&admission.reference.grant_id,&admission.reference.revision,&facts.expires_at_epoch_ms.to_string()])?;
        }
        let check=tx.query("SELECT facts_revision,policy_revision,revocation_head,binding_id,generation,source_epoch,runtime_instance_id,model_ref_digest,capability_revision,context_manifest_id,context_manifest_hash,admission_ref,admission_revision,expires_at_epoch_ms FROM main.gogoke_action_current_facts WHERE domain_id=? AND operation_id=?", &[&facts.domain_id,&facts.operation_id],14)?;
        let expected = vec![
            next.clone(),
            profile.policy_revision,
            profile.revocation_head,
            facts.binding_id.clone(),
            facts.generation.clone(),
            facts.source_epoch.clone(),
            facts.runtime_instance_id.clone(),
            facts.model_ref_digest.clone(),
            facts.capability_revision.clone(),
            facts.context_manifest_id.clone(),
            manifest_hash,
            admission.reference.grant_id,
            admission.reference.revision,
            facts.expires_at_epoch_ms.to_string(),
        ];
        if check.len()!=1 || check[0]!=expected || package.package_digest!=tx.query("SELECT package_digest FROM main.gogoke_action_authority_intents WHERE domain_id=? AND operation_id=?", &[&facts.domain_id,&facts.operation_id],1)?[0][0] || task.task_revision.is_empty() { return Err(OrchestrationError::OperationConflict); }
        Ok(next)
    })
}

fn verify_active_process_custody(
    tx: &mut Transaction<'_, '_>,
    operation_id: &str,
    prepared: &PreparedCustody,
    active_identity: &ProcessIdentity,
) -> Result<()> {
    if &prepared.identity != active_identity {
        return denied();
    }
    let rows = tx.query(
        "SELECT ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state FROM main.gogoke_coordination_process_custody WHERE operation_id=?",
        &[operation_id],
        10,
    )?;
    let expected = vec![
        prepared.ticket.opaque().to_owned(),
        prepared.custodian_nonce.clone(),
        prepared.identity.pid.to_string(),
        prepared.identity.creation_time_100ns.to_string(),
        prepared.identity.image_path.to_string_lossy().into_owned(),
        prepared.binding.binary_digest_sha256.clone(),
        prepared.binding.profile_id.clone(),
        prepared.binding.domain_id.clone(),
        prepared.binding.generation.clone(),
        "ACTIVE".to_owned(),
    ];
    if rows.len() != 1 || rows[0] != expected {
        return denied();
    }
    Ok(())
}

/// Private native producer for the current facts consumed by
/// `begin_committed_action`. Callers identify an already prepared Action; all
/// mutable authority values are resolved inside this transaction. The exact
/// in-memory active process identity must match its durable ACTIVE custody row.
pub(crate) fn derive_native_action_current_facts(
    connection: &mut VerifiedDatabaseConnection<'_>,
    references: &NativeActionCurrentFactsRefs,
    prepared: &PreparedCustody,
    active_identity: &ProcessIdentity,
) -> Result<String> {
    for value in [
        &references.domain_id,
        &references.operation_id,
        &references.reservation_id,
    ] {
        identifier(value)?;
    }
    transaction::run(connection, |tx| {
        ensure_schema(tx)?;
        verify_active_process_custody(tx, &references.operation_id, prepared, active_identity)?;
        let intent = tx.query(
            "SELECT package_operation_id,parent_grant_ref,task_id,recipe_id,session_id,context_manifest_id,action_kind,lane,task_revision,package_digest,binding_id,generation,source_epoch,runtime_instance_id,auth_revision,target_domain_id,payload_digest FROM main.gogoke_action_authority_intents WHERE domain_id=? AND operation_id=?",
            &[&references.domain_id, &references.operation_id],
            17,
        )?;
        let action = tx.query(
            "SELECT semantic_digest,reservation_id,binding_id,session_id,execution_id,runtime_instance_id,profile_id,auth_revision,generation,lane,action_kind,payload_hex,commitment_record,state FROM main.gogoke_action_reservations WHERE operation_id=?",
            &[&references.operation_id],
            14,
        )?;
        if intent.len() != 1
            || action.len() != 1
            || action[0][1] != references.reservation_id
            || action[0][13] != "reserved"
            || action[0][2] != intent[0][10]
            || action[0][3] != intent[0][4]
            || action[0][5] != intent[0][13]
            || action[0][7] != intent[0][14]
            || action[0][8] != intent[0][11]
            || action[0][9] != intent[0][7]
            || action[0][10] != intent[0][6]
        {
            return denied();
        }
        let payload = decode_hex(&action[0][11])?;
        let selection = PrepareActionAuthority {
            domain_id: references.domain_id.clone(),
            parent_grant_ref: intent[0][1].clone(),
            package_operation_id: intent[0][0].clone(),
            task_id: intent[0][2].clone(),
            recipe_id: intent[0][3].clone(),
            session_id: intent[0][4].clone(),
            context_manifest_id: intent[0][5].clone(),
            action_operation_id: references.operation_id.clone(),
            reservation_id: references.reservation_id.clone(),
            action_kind: intent[0][6].clone(),
            lane: intent[0][7].clone(),
            payload,
        };
        let (package, task, lineage, recipe, profile) = current_selection(tx, &selection)?;
        let payload_digest = content_hash(&selection.payload);
        let semantic_digest = intent_digest(
            &selection,
            &package.package_digest,
            &task.task_revision,
            &recipe.recipe.revision,
            &recipe.content_hash,
            &profile.policy_revision,
            &payload_digest,
        );
        let commitment = package_commitment(&package);
        let manifest_hash = current_manifest_hash_in_transaction(
            tx,
            &references.domain_id,
            &recipe.recipe.context_manifest_id,
        )?;
        let admission =
            current_admission_in_transaction(tx, &profile, &recipe.recipe.admission_ref)?;
        let leases = tx.query(
            "SELECT operation_id,candidate_id,resource_ref,resource_revision,resource_reservation_ref FROM main.gogoke_decision_capacity_leases WHERE action_operation_id=?",
            &[&references.operation_id],
            5,
        )?;
        if leases.len() != 1 {
            return denied();
        }
        let decision = super::decision_replay::read_in_transaction(
            tx,
            &references.domain_id,
            &leases[0][0],
        )?;
        let snapshot = tx.query(
            "SELECT capability_revision,binding_id,binding_generation,policy_revision,task_revision,action_digest,resource_ref,resource_revision,auth_revision FROM main.gogoke_decision_authority_snapshots WHERE operation_id=? AND candidate_id=?",
            &[&leases[0][0], &leases[0][1]],
            9,
        )?;
        let pool = tx.query(
            "SELECT revision FROM main.gogoke_decision_capacity_pools WHERE resource_ref=?",
            &[&leases[0][2]],
            1,
        )?;
        if selection.payload.as_slice() != package.instruction.as_bytes()
            || intent[0][8] != task.task_revision
            || intent[0][9] != package.package_digest
            || intent[0][10] != lineage.native.binding_id
            || intent[0][11] != lineage.native.generation
            || intent[0][12] != lineage.native.source_epoch
            || intent[0][13] != recipe.recipe.runtime_instance_id
            || intent[0][14] != profile.policy_revision
            || intent[0][15] != package.target.domain_id
            || intent[0][16] != payload_digest
            || action[0][0] != semantic_digest
            || action[0][4] != package.target_binding.execution_id
            || action[0][6] != profile.profile_id
            || action[0][12] != super::super::action::commitment_record(&commitment)
            || package.parent_grant_ref != admission.reference.grant_id
            || recipe.recipe.context_manifest_id != selection.context_manifest_id
            || prepared.binding.profile_id != profile.profile_id
            || prepared.binding.domain_id != package.target.domain_id
            || prepared.binding.generation != lineage.native.generation
            || decision.action_intent_ref != references.operation_id
            || decision.record.choice != leases[0][1]
            || decision.record.task_revision != task.task_revision
            || decision.record.policy_revision != profile.policy_revision
            || decision.record.binding_generation != lineage.native.generation
            || decision.resource_reservation_ref != leases[0][4]
            || snapshot.len() != 1
            || snapshot[0][0] != decision.record.capability_revision
            || snapshot[0][1] != lineage.native.binding_id
            || snapshot[0][2] != lineage.native.generation
            || snapshot[0][3] != profile.policy_revision
            || snapshot[0][4] != task.task_revision
            || snapshot[0][5] != semantic_digest
            || snapshot[0][6] != leases[0][2]
            || snapshot[0][7] != leases[0][3]
            || snapshot[0][8] != profile.policy_revision
            || pool.len() != 1
            || pool[0][0] != leases[0][3]
        {
            return denied();
        }
        let model_ref_digest = super::execution_recipe::model_ref_digest(&recipe.recipe)?;
        let expires_at_epoch_ms = admission.expires_at_epoch_ms;
        if expires_at_epoch_ms <= now_epoch_ms()? {
            return denied();
        }
        let derived = vec![
            profile.policy_revision.clone(),
            profile.revocation_head.clone(),
            lineage.native.binding_id.clone(),
            lineage.native.generation.clone(),
            lineage.native.source_epoch.clone(),
            recipe.recipe.runtime_instance_id.clone(),
            model_ref_digest,
            decision.record.capability_revision.clone(),
            recipe.recipe.context_manifest_id.clone(),
            manifest_hash,
            admission.reference.grant_id.clone(),
            admission.reference.revision.clone(),
            expires_at_epoch_ms.to_string(),
        ];
        let current = tx.query(
            "SELECT facts_revision,policy_revision,revocation_head,binding_id,generation,source_epoch,runtime_instance_id,model_ref_digest,capability_revision,context_manifest_id,context_manifest_hash,admission_ref,admission_revision,expires_at_epoch_ms FROM main.gogoke_action_current_facts WHERE domain_id=? AND operation_id=?",
            &[&references.domain_id, &references.operation_id],
            14,
        )?;
        let next = if current.is_empty() {
            tx.write(
                "INSERT INTO main.gogoke_action_current_facts(domain_id,operation_id,facts_revision,policy_revision,revocation_head,binding_id,generation,source_epoch,runtime_instance_id,model_ref_digest,capability_revision,context_manifest_id,context_manifest_hash,admission_ref,admission_revision,expires_at_epoch_ms) VALUES(?,?,'1',?,?,?,?,?,?,?,?,?,?,?,?,?)",
                &[&references.domain_id, &references.operation_id, &derived[0], &derived[1], &derived[2], &derived[3], &derived[4], &derived[5], &derived[6], &derived[7], &derived[8], &derived[9], &derived[10], &derived[11], &derived[12]],
            )?;
            "1".to_owned()
        } else if current.len() == 1 && current[0][1..] == derived {
            current[0][0].clone()
        } else {
            return Err(OrchestrationError::OperationConflict);
        };
        let check = tx.query(
            "SELECT facts_revision,policy_revision,revocation_head,binding_id,generation,source_epoch,runtime_instance_id,model_ref_digest,capability_revision,context_manifest_id,context_manifest_hash,admission_ref,admission_revision,expires_at_epoch_ms FROM main.gogoke_action_current_facts WHERE domain_id=? AND operation_id=?",
            &[&references.domain_id, &references.operation_id],
            14,
        )?;
        let mut expected = vec![next.clone()];
        expected.extend(derived);
        if check.len() != 1 || check[0] != expected {
            return Err(OrchestrationError::OperationConflict);
        }
        Ok(next)
    })
}

fn now_epoch_ms() -> Result<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .map_err(|_| OrchestrationError::AccessDenied)
}

fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn action_domain_record(
    domain_id: &str,
    object_type: &str,
    operation_id: &str,
    object_id: &str,
    object_bytes: Vec<u8>,
    event_type: &str,
    receipt_type: &str,
    recorded_at: &str,
) -> DomainRecordInput {
    let object_hash = content_hash(&object_bytes);
    let event_id = format!(
        "action-event:{}",
        &content_hash(format!("{domain_id}\0{object_type}\0{operation_id}\0event").as_bytes())[7..]
    );
    let receipt_id = format!(
        "action-receipt:{}",
        &content_hash(format!("{domain_id}\0{object_type}\0{operation_id}\0receipt").as_bytes())
            [7..]
    );
    let event_bytes = format!(
        "{{\"objectHash\":{},\"operationId\":{},\"type\":{}}}",
        quote(&object_hash),
        quote(operation_id),
        quote(event_type)
    )
    .into_bytes();
    let receipt_bytes = format!(
        "{{\"objectHash\":{},\"operationId\":{},\"schema\":{}}}",
        quote(&object_hash),
        quote(operation_id),
        quote(receipt_type)
    )
    .into_bytes();
    DomainRecordInput {
        domain_id: domain_id.into(),
        object_type: object_type.into(),
        object_id: object_id.into(),
        object_version: "1".into(),
        object_bytes,
        native_identity: None,
        event_id,
        stream_id: format!("gogoke.action.v1/{object_type}/{object_id}"),
        expected_previous_counter: None,
        counter: "0".into(),
        event_type: event_type.into(),
        occurred_at: recorded_at.into(),
        event_bytes,
        receipt_id,
        operation_id: operation_id.into(),
        receipt_type: receipt_type.into(),
        recorded_at: recorded_at.into(),
        receipt_bytes,
    }
}

/// Reconcile a replay only when all three canonical DomainRecord rows already
/// exist. apply_domain_record can write if its canonical receipt join is
/// missing; replay must never recreate missing authority evidence from a
/// denormalized Action row.
fn reconcile_existing_action_domain_record(
    tx: &mut Transaction<'_, '_>,
    record: DomainRecordInput,
) -> Result<super::super::atomic::DomainRecordReceipt> {
    let object = tx.query(
        "SELECT CAST(canonical_json AS TEXT),content_hash FROM main.gogoke_objects WHERE domain_id=? AND object_type=? AND object_id=? AND object_version=?",
        &[&record.domain_id, &record.object_type, &record.object_id, &record.object_version],
        2,
    )?;
    let event = tx.query(
        "SELECT CAST(canonical_json AS TEXT),content_hash FROM main.gogoke_events WHERE domain_id=? AND event_id=?",
        &[&record.domain_id, &record.event_id],
        2,
    )?;
    let receipt = tx.query(
        "SELECT receipt_id,CAST(canonical_json AS TEXT),content_hash,event_id,object_type,object_id,object_version,receipt_type FROM main.gogoke_receipts WHERE domain_id=? AND operation_id=?",
        &[&record.domain_id, &record.operation_id],
        8,
    )?;
    if object.len() != 1 || event.len() != 1 || receipt.len() != 1
        || object[0][0].as_bytes() != record.object_bytes.as_slice()
        || content_hash(object[0][0].as_bytes()) != object[0][1]
        || event[0][0].as_bytes() != record.event_bytes.as_slice()
        || content_hash(event[0][0].as_bytes()) != event[0][1]
        || receipt[0][0] != record.receipt_id
        || receipt[0][1].as_bytes() != record.receipt_bytes.as_slice()
        || content_hash(receipt[0][1].as_bytes()) != receipt[0][2]
        || receipt[0][3] != record.event_id
        || receipt[0][4] != record.object_type
        || receipt[0][5] != record.object_id
        || receipt[0][6] != record.object_version
        || receipt[0][7] != record.receipt_type
    {
        return denied();
    }
    tx.apply_domain_record(record)
}

pub(super) fn load_validated_native_receipt(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    operation_id: &str,
) -> Result<Option<Vec<String>>> {
    let rows=tx.query("SELECT receipt_ref,reservation_id,semantic_digest,attempt_id,send_authority,binding_id,generation,source_epoch,runtime_instance_id,native_request_id,native_session_id,evidence_hash,disposition,receipt_id FROM main.gogoke_action_native_receipts WHERE domain_id=? AND operation_id=?", &[domain_id,operation_id],14)?;
    if rows.is_empty() { return Ok(None); }
    if rows.len()!=1 { return denied(); }
    let row=&rows[0];
    let object=format!("{{\"attemptId\":{},\"bindingId\":{},\"disposition\":{},\"evidenceHash\":{},\"generation\":{},\"nativeRequestId\":{},\"nativeSessionId\":{},\"operationId\":{},\"runtimeInstanceId\":{},\"semanticDigest\":{},\"sendAuthority\":{},\"sourceEpoch\":{},\"trustedReceiptRef\":{}}}",quote(&row[3]),quote(&row[5]),quote(&row[12]),quote(&row[11]),quote(&row[6]),quote(&row[9]),quote(&row[10]),quote(operation_id),quote(&row[8]),quote(&row[2]),quote(&row[4]),quote(&row[7]),quote(&row[0])).into_bytes();
    let times=tx.query("SELECT recorded_at FROM main.gogoke_receipts WHERE domain_id=? AND receipt_id=?", &[domain_id,&row[13]],1)?;
    if times.len()!=1 { return denied(); }
    let record = action_domain_record(
        domain_id,"ActionNativeReceipt",&format!("native-receipt:{operation_id}"),operation_id,
        object,"ActionNativeReceiptRecorded","ActionNativeReceiptRecorded",&times[0][0],
    );
    let storage = reconcile_existing_action_domain_record(tx, record)?;
    if storage.disposition!="RECONCILED" || storage.receipt_id!=row[13] { return denied(); }
    Ok(Some(row.clone()))
}

pub(super) fn load_validated_completion(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    operation_id: &str,
) -> Result<Option<Vec<String>>> {
    let rows=tx.query("SELECT reservation_id,semantic_digest,attempt_id,send_authority,binding_id,generation,source_epoch,runtime_instance_id,native_request_id,native_session_id,trusted_receipt_ref,evidence_hash,disposition,receipt_id FROM main.gogoke_action_completion_receipts WHERE domain_id=? AND operation_id=?", &[domain_id,operation_id],14)?;
    if rows.is_empty() { return Ok(None); }
    if rows.len()!=1 { return denied(); }
    let row=&rows[0];
    let object=format!("{{\"attemptId\":{},\"bindingId\":{},\"disposition\":{},\"evidenceHash\":{},\"generation\":{},\"nativeRequestId\":{},\"nativeSessionId\":{},\"operationId\":{},\"reservationId\":{},\"runtimeInstanceId\":{},\"semanticDigest\":{},\"sendAuthority\":{},\"sourceEpoch\":{},\"trustedReceiptRef\":{}}}",quote(&row[2]),quote(&row[4]),quote(&row[12]),quote(&row[11]),quote(&row[5]),quote(&row[8]),quote(&row[9]),quote(operation_id),quote(&row[0]),quote(&row[7]),quote(&row[1]),quote(&row[3]),quote(&row[6]),quote(&row[10])).into_bytes();
    let times=tx.query("SELECT recorded_at FROM main.gogoke_receipts WHERE domain_id=? AND receipt_id=?", &[domain_id,&row[13]],1)?;
    if times.len()!=1 { return denied(); }
    let record = action_domain_record(
        domain_id,"ActionCompletion",&format!("complete:{operation_id}"),operation_id,
        object,"ActionCompletionRecorded","ActionCompletionRecorded",&times[0][0],
    );
    let storage = reconcile_existing_action_domain_record(tx, record)?;
    if storage.disposition!="RECONCILED" || storage.receipt_id!=row[13] { return denied(); }
    Ok(Some(row.clone()))
}

/// Trusted native-host receipt ingress. There is intentionally no equivalent
/// session/IPC operation; the host calls this only after validating its native
/// request/session receipt and exact process binding.
pub(crate) fn record_trusted_native_action_receipt(
    connection: &mut VerifiedDatabaseConnection<'_>,
    evidence: &TrustedActionCompletionEvidence,
) -> Result<String> {
    if matches!(
        evidence.disposition,
        ActionCompletionDisposition::AcceptanceUnknown
    ) {
        // Unknown is represented by the Action state without fabricating a
        // trusted receipt; later reliable evidence may still resolve it.
        return denied();
    }
    for value in [
        &evidence.domain_id,
        &evidence.operation_id,
        &evidence.reservation_id,
        &evidence.attempt_id,
        &evidence.send_authority,
        &evidence.binding_id,
        &evidence.generation,
        &evidence.source_epoch,
        &evidence.runtime_instance_id,
        &evidence.native_request_id,
        &evidence.native_session_id,
        &evidence.trusted_receipt_ref,
    ] {
        identifier(value)?;
    }
    if evidence.semantic_digest.len() != 71
        || !evidence.semantic_digest.starts_with("sha256:")
        || evidence.evidence_hash.len() != 71
        || !evidence.evidence_hash.starts_with("sha256:")
    {
        return denied();
    }
    transaction::run(connection, |tx| {
        ensure_schema(tx)?;
        let expected_disposition = match evidence.disposition {
            ActionCompletionDisposition::Completed => "COMPLETED",
            ActionCompletionDisposition::Rejected => "REJECTED",
            ActionCompletionDisposition::AcceptanceUnknown => return denied(),
        };
        let existing=load_validated_native_receipt(tx,&evidence.domain_id,&evidence.operation_id)?;
        if let Some(existing) = existing {
            let expected = vec![
                evidence.trusted_receipt_ref.clone(), evidence.reservation_id.clone(),
                evidence.semantic_digest.clone(), evidence.attempt_id.clone(),
                evidence.send_authority.clone(), evidence.binding_id.clone(),
                evidence.generation.clone(), evidence.source_epoch.clone(),
                evidence.runtime_instance_id.clone(), evidence.native_request_id.clone(),
                evidence.native_session_id.clone(), evidence.evidence_hash.clone(),
                expected_disposition.to_owned(),
            ];
            if existing[..13]!=expected {
                return Err(OrchestrationError::OperationConflict);
            }
            return Ok(evidence.trusted_receipt_ref.clone());
        }
        let current=tx.query("SELECT semantic_digest,reservation_id,binding_id,session_id,runtime_instance_id,generation,state,COALESCE(send_authority,'') FROM main.gogoke_action_reservations WHERE operation_id=?", &[&evidence.operation_id],8)?;
        let intent=tx.query("SELECT attempt_id,send_authority,binding_id,generation,source_epoch,runtime_instance_id,session_id,target_domain_id FROM main.gogoke_action_authority_intents WHERE domain_id=? AND operation_id=?", &[&evidence.domain_id,&evidence.operation_id],8)?;
        if current.len() != 1
            || intent.len() != 1
            || current[0][0] != evidence.semantic_digest
            || current[0][1] != evidence.reservation_id
            || current[0][2] != evidence.binding_id
            || current[0][3] != intent[0][6]
            || current[0][4] != evidence.runtime_instance_id
            || current[0][5] != evidence.generation
            || !matches!(current[0][6].as_str(), "dispatching" | "outcome-unknown")
            || current[0][7] != evidence.send_authority
            || intent[0][0] != evidence.attempt_id
            || intent[0][1] != evidence.send_authority
            || intent[0][2] != evidence.binding_id
            || intent[0][3] != evidence.generation
            || intent[0][4] != evidence.source_epoch
            || intent[0][5] != evidence.runtime_instance_id
        {
            return denied();
        }
        let lineage = super::session_lineage::read_session_lineage_in_transaction(
            tx,
            &intent[0][7],
            &intent[0][6],
        )?;
        if lineage.native.native_session_id != evidence.native_session_id
            || lineage.native.binding_id != evidence.binding_id
            || lineage.native.generation != evidence.generation
            || lineage.native.source_epoch != evidence.source_epoch
        {
            return denied();
        }
        let disposition = expected_disposition;
        let op = format!("native-receipt:{}", evidence.operation_id);
        let object=format!("{{\"attemptId\":{},\"bindingId\":{},\"disposition\":{},\"evidenceHash\":{},\"generation\":{},\"nativeRequestId\":{},\"nativeSessionId\":{},\"operationId\":{},\"runtimeInstanceId\":{},\"semanticDigest\":{},\"sendAuthority\":{},\"sourceEpoch\":{},\"trustedReceiptRef\":{}}}",quote(&evidence.attempt_id),quote(&evidence.binding_id),quote(disposition),quote(&evidence.evidence_hash),quote(&evidence.generation),quote(&evidence.native_request_id),quote(&evidence.native_session_id),quote(&evidence.operation_id),quote(&evidence.runtime_instance_id),quote(&evidence.semantic_digest),quote(&evidence.send_authority),quote(&evidence.source_epoch),quote(&evidence.trusted_receipt_ref)).into_bytes();
        let time = tx.query("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", &[], 1)?;
        let record = action_domain_record(
            &evidence.domain_id,
            "ActionNativeReceipt",
            &op,
            &evidence.operation_id,
            object,
            "ActionNativeReceiptRecorded",
            "ActionNativeReceiptRecorded",
            &time[0][0],
        );
        let storage = tx.apply_domain_record(record)?;
        tx.write("INSERT INTO main.gogoke_action_native_receipts(domain_id,receipt_ref,operation_id,reservation_id,semantic_digest,attempt_id,send_authority,binding_id,generation,source_epoch,runtime_instance_id,native_request_id,native_session_id,evidence_hash,disposition,receipt_id) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(domain_id,operation_id) DO NOTHING", &[&evidence.domain_id,&evidence.trusted_receipt_ref,&evidence.operation_id,&evidence.reservation_id,&evidence.semantic_digest,&evidence.attempt_id,&evidence.send_authority,&evidence.binding_id,&evidence.generation,&evidence.source_epoch,&evidence.runtime_instance_id,&evidence.native_request_id,&evidence.native_session_id,&evidence.evidence_hash,disposition,&storage.receipt_id])?;
        let rows=tx.query("SELECT receipt_ref,receipt_id,evidence_hash FROM main.gogoke_action_native_receipts WHERE domain_id=? AND operation_id=?", &[&evidence.domain_id,&evidence.operation_id],3)?;
        if rows.len() != 1
            || rows[0][0] != evidence.trusted_receipt_ref
            || rows[0][1] != storage.receipt_id
            || rows[0][2] != evidence.evidence_hash
        {
            return Err(OrchestrationError::OperationConflict);
        }
        Ok(evidence.trusted_receipt_ref.clone())
    })
}

/// Commits only a receipt previously persisted by the trusted native-host
/// ingress. Without such evidence, the durable state becomes
/// `ACCEPTANCE_UNKNOWN`; this is not Action completion and cannot be resent.
pub(crate) fn complete_action_from_native_receipt(
    connection: &mut VerifiedDatabaseConnection<'_>,
    request: &BeginCommittedAction,
) -> Result<ActionCompletionReceipt> {
    identifier(&request.domain_id)?;
    identifier(&request.operation_id)?;
    identifier(&request.reservation_id)?;
    transaction::run(connection, |tx| {
        ensure_schema(tx)?;
        let action=tx.query("SELECT a.semantic_digest,a.reservation_id,a.state,COALESCE(a.send_authority,'') FROM main.gogoke_action_reservations a JOIN main.gogoke_action_authority_intents i ON i.operation_id=a.operation_id WHERE i.domain_id=? AND a.operation_id=?", &[&request.domain_id,&request.operation_id],4)?;
        if action.len() != 1 || action[0][1] != request.reservation_id {
            return denied();
        }
        if !matches!(
            action[0][2].as_str(),
            "dispatching" | "outcome-unknown" | "completed" | "rejected"
        ) {
            return denied();
        }
        let native=load_validated_native_receipt(tx,&request.domain_id,&request.operation_id)?;
        if native.is_none() {
            tx.write("UPDATE main.gogoke_action_reservations SET state='outcome-unknown',outcome_kind='outcome-unknown' WHERE operation_id=? AND reservation_id=? AND state IN ('dispatching','outcome-unknown')", &[&request.operation_id,&request.reservation_id])?;
            return Ok(ActionCompletionReceipt {
                disposition: "ACCEPTANCE_UNKNOWN",
                terminal_state: "outcome-unknown",
                authority_status: COMPLETION_AUTHORITY_STATUS,
                operation_id: request.operation_id.clone(),
                receipt_id: String::new(),
                semantic_digest: action[0][0].clone(),
            });
        }
        let native=vec![native.expect("checked native receipt")];
        if native[0][1] != request.reservation_id
            || native[0][2] != action[0][0]
            || native[0][4] != action[0][3]
        {
            return denied();
        }
        let existing=load_validated_completion(tx,&request.domain_id,&request.operation_id)?;
        if let Some(existing) = existing {
            let terminal = match native[0][12].as_str() {
                "COMPLETED" => "completed",
                "REJECTED" => "rejected",
                "ACCEPTANCE_UNKNOWN" => "outcome-unknown",
                _ => return denied(),
            };
            if existing[1] != native[0][2]
                || existing[12] != native[0][12]
                || action[0][2] != terminal
            {
                return denied();
            }
            return Ok(ActionCompletionReceipt {
                disposition: "REPLAYED",
                terminal_state: terminal,
                authority_status: COMPLETION_AUTHORITY_STATUS,
                operation_id: request.operation_id.clone(),
                receipt_id: existing[13].clone(),
                semantic_digest: existing[1].clone(),
            });
        }
        let facts=tx.query("SELECT facts_revision,policy_revision,revocation_head,binding_id,generation,source_epoch,runtime_instance_id,model_ref_digest,capability_revision,context_manifest_id,context_manifest_hash,admission_ref,admission_revision,expires_at_epoch_ms FROM main.gogoke_action_current_facts WHERE domain_id=? AND operation_id=?", &[&request.domain_id,&request.operation_id],14)?;
        if facts.len() != 1
            || facts[0][4] != native[0][6]
            || facts[0][5] != native[0][7]
            || facts[0][6] != native[0][8]
            || facts[0][3] != native[0][5]
        {
            return denied();
        }
        let disposition = native[0][12].as_str();
        let operation_id = format!("complete:{}", request.operation_id);
        let object=format!("{{\"attemptId\":{},\"bindingId\":{},\"disposition\":{},\"evidenceHash\":{},\"generation\":{},\"nativeRequestId\":{},\"nativeSessionId\":{},\"operationId\":{},\"reservationId\":{},\"runtimeInstanceId\":{},\"semanticDigest\":{},\"sendAuthority\":{},\"sourceEpoch\":{},\"trustedReceiptRef\":{}}}",quote(&native[0][3]),quote(&native[0][5]),quote(disposition),quote(&native[0][11]),quote(&native[0][6]),quote(&native[0][9]),quote(&native[0][10]),quote(&request.operation_id),quote(&request.reservation_id),quote(&native[0][8]),quote(&native[0][2]),quote(&native[0][4]),quote(&native[0][7]),quote(&native[0][0])).into_bytes();
        let time = tx.query("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", &[], 1)?;
        let record = action_domain_record(
            &request.domain_id,
            "ActionCompletion",
            &operation_id,
            &request.operation_id,
            object,
            "ActionCompletionRecorded",
            "ActionCompletionRecorded",
            &time[0][0],
        );
        let storage = tx.apply_domain_record(record)?;
        tx.write("INSERT INTO main.gogoke_action_completion_receipts(domain_id,operation_id,reservation_id,semantic_digest,attempt_id,send_authority,binding_id,generation,source_epoch,runtime_instance_id,native_request_id,native_session_id,trusted_receipt_ref,evidence_hash,disposition,receipt_id) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(domain_id,operation_id) DO NOTHING", &[&request.domain_id,&request.operation_id,&request.reservation_id,&native[0][2],&native[0][3],&native[0][4],&native[0][5],&native[0][6],&native[0][7],&native[0][8],&native[0][9],&native[0][10],&native[0][0],&native[0][11],disposition,&storage.receipt_id])?;
        let terminal = match disposition {
            "COMPLETED" => "completed",
            "REJECTED" => "rejected",
            "ACCEPTANCE_UNKNOWN" => "outcome-unknown",
            _ => return denied(),
        };
        tx.write("UPDATE main.gogoke_action_reservations SET state=?,outcome_kind=?,receipt_ref=? WHERE operation_id=? AND reservation_id=? AND state IN ('dispatching','outcome-unknown')", &[terminal,terminal,&native[0][0],&request.operation_id,&request.reservation_id])?;
        let check=tx.query("SELECT receipt_id,semantic_digest FROM main.gogoke_action_completion_receipts WHERE domain_id=? AND operation_id=?", &[&request.domain_id,&request.operation_id],2)?;
        if check.len() != 1 || check[0][0] != storage.receipt_id || check[0][1] != native[0][2] {
            return Err(OrchestrationError::OperationConflict);
        }
        Ok(ActionCompletionReceipt {
            disposition: terminal,
            terminal_state: terminal,
            authority_status: COMPLETION_AUTHORITY_STATUS,
            operation_id: request.operation_id.clone(),
            receipt_id: storage.receipt_id,
            semantic_digest: native[0][2].clone(),
        })
    })
}

/// Rechecks the durable selection and the native-derived current facts before
/// the one-way `reserved` to `dispatching` transition. The caller cannot supply
/// runtime, capability, admission, or binding authority through IPC.
pub(crate) fn begin_committed_action(
    connection: &mut VerifiedDatabaseConnection<'_>,
    request: &BeginCommittedAction,
) -> Result<BeginCommittedDisposition> {
    identifier(&request.domain_id)?;
    identifier(&request.operation_id)?;
    identifier(&request.reservation_id)?;
    transaction::run(connection, |tx| {
        ensure_schema(tx)?;
        let intent = tx.query(
            "SELECT package_operation_id,parent_grant_ref,task_id,recipe_id,session_id,context_manifest_id,action_kind,lane,task_revision,package_digest,binding_id,generation,source_epoch,runtime_instance_id,auth_revision,target_domain_id,payload_digest FROM main.gogoke_action_authority_intents WHERE domain_id=? AND operation_id=?",
            &[&request.domain_id, &request.operation_id],
            17,
        )?;
        let action = tx.query(
            "SELECT semantic_digest,reservation_id,binding_id,session_id,execution_id,runtime_instance_id,profile_id,auth_revision,generation,lane,action_kind,payload_hex,commitment_record,state,COALESCE(send_authority,'') FROM main.gogoke_action_reservations WHERE operation_id=?",
            &[&request.operation_id],
            15,
        )?;
        if intent.len() != 1
            || action.len() != 1
            || action[0][1] != request.reservation_id
            || action[0][2] != intent[0][10]
            || action[0][3] != intent[0][4]
            || action[0][5] != intent[0][13]
            || action[0][7] != intent[0][14]
            || action[0][8] != intent[0][11]
            || action[0][9] != intent[0][7]
            || action[0][10] != intent[0][6]
        {
            return denied();
        }
        if action[0][13] != "reserved" {
            let state = if matches!(action[0][13].as_str(), "dispatching" | "outcome-unknown") {
                "ACCEPTANCE_UNKNOWN".to_owned()
            } else {
                action[0][13].clone()
            };
            return Ok(BeginCommittedDisposition::Replay { state });
        }
        let payload = decode_hex(&action[0][11])?;
        let profile = current_profile(tx)?;
        if profile.policy_revision != intent[0][14] {
            return denied();
        }
        let selected = PrepareActionAuthority {
            domain_id: request.domain_id.clone(),
            parent_grant_ref: intent[0][1].clone(),
            package_operation_id: intent[0][0].clone(),
            task_id: intent[0][2].clone(),
            recipe_id: intent[0][3].clone(),
            session_id: intent[0][4].clone(),
            context_manifest_id: intent[0][5].clone(),
            action_operation_id: request.operation_id.clone(),
            reservation_id: request.reservation_id.clone(),
            action_kind: intent[0][6].clone(),
            lane: intent[0][7].clone(),
            payload,
        };
        let (package, task, lineage, recipe, current_profile) = current_selection(tx, &selected)?;
        if recipe.recipe.recipe_id == "recipe-r2-02-test"
            && recipe.recipe.runtime_instance_id != super::r2_fixture_driver::FIXED_RUNTIME_INSTANCE_ID {
            super::r2_fixture_driver::resolve_in_transaction(tx, &recipe.recipe.runtime_instance_id)?;
        }
        let payload_digest = content_hash(&selected.payload);
        let expected_semantic_digest = intent_digest(
            &selected,
            &package.package_digest,
            &task.task_revision,
            &recipe.recipe.revision,
            &recipe.content_hash,
            &current_profile.policy_revision,
            &payload_digest,
        );
        let expected_commitment = package_commitment(&package);
        let manifest_hash = current_manifest_hash_in_transaction(
            tx,
            &request.domain_id,
            &recipe.recipe.context_manifest_id,
        )?;
        let admission =
            current_admission_in_transaction(tx, &current_profile, &recipe.recipe.admission_ref)?;
        if selected.payload.as_slice() != package.instruction.as_bytes()
            || intent[0][16] != payload_digest
            || action[0][0] != expected_semantic_digest
            || action[0][4] != package.target_binding.execution_id
            || action[0][12] != super::super::action::commitment_record(&expected_commitment)
            || package.package_digest != intent[0][9]
            || task.task_revision != intent[0][8]
            || lineage.native.binding_id != intent[0][10]
            || lineage.native.generation != intent[0][11]
            || lineage.native.source_epoch != intent[0][12]
            || recipe.recipe.runtime_instance_id != intent[0][13]
            || current_profile.policy_revision != intent[0][14]
        {
            return denied();
        }
        let leases = tx.query(
            "SELECT operation_id,candidate_id,resource_ref,resource_revision,action_operation_id,resource_reservation_ref FROM main.gogoke_decision_capacity_leases WHERE action_operation_id=?",
            &[&request.operation_id], 6,
        )?;
        if leases.len() != 1 || leases[0][4] != request.operation_id {
            return denied();
        }
        let decision =
            super::decision_replay::read_in_transaction(tx, &request.domain_id, &leases[0][0])?;
        if decision.action_intent_ref != request.operation_id
            || decision.record.choice != leases[0][1]
            || decision.record.task_revision != task.task_revision
            || decision.record.policy_revision != profile.policy_revision
            || decision.record.binding_generation != package.target_binding.generation
            || decision.resource_reservation_ref != leases[0][5]
        {
            return denied();
        }
        let pool = tx.query(
            "SELECT revision FROM main.gogoke_decision_capacity_pools WHERE resource_ref=?",
            &[&leases[0][2]],
            1,
        )?;
        if pool.len() != 1 || pool[0][0] != leases[0][3] {
            return denied();
        }

        let facts=tx.query("SELECT facts_revision,policy_revision,revocation_head,binding_id,generation,source_epoch,runtime_instance_id,model_ref_digest,capability_revision,context_manifest_id,context_manifest_hash,admission_ref,admission_revision,expires_at_epoch_ms FROM main.gogoke_action_current_facts WHERE domain_id=? AND operation_id=?", &[&request.domain_id,&request.operation_id],14)?;
        if facts.len() != 1 {
            return Ok(BeginCommittedDisposition::CurrentFactsUnavailable {
                axes: vec!["trusted_native_current_facts".into()],
            });
        }
        let row = &facts[0];
        let expiry = row[13]
            .parse::<u64>()
            .map_err(|_| OrchestrationError::AccessDenied)?;
        let recipe_model_digest = super::execution_recipe::model_ref_digest(&recipe.recipe)?;
        let facts_current = row[1] == current_profile.policy_revision
            && row[2] == current_profile.revocation_head
            && row[3] == lineage.native.binding_id
            && row[4] == lineage.native.generation
            && row[5] == lineage.native.source_epoch
            && row[6] == recipe.recipe.runtime_instance_id
            && row[7] == recipe_model_digest
            && row[8] == decision.record.capability_revision
            && row[9] == recipe.recipe.context_manifest_id
            && row[10] == manifest_hash
            && row[11] == recipe.recipe.admission_ref
            && row[11] == admission.reference.grant_id
            && row[12] == admission.reference.revision
            && package.parent_grant_ref == admission.reference.grant_id
            && row[8].parse::<u64>().is_ok()
            && row[12].parse::<u64>().is_ok()
            && expiry > now_epoch_ms()?;
        if !facts_current {
            return Ok(BeginCommittedDisposition::CurrentFactsUnavailable {
                axes: vec!["native_facts_stale_or_mismatched".into()],
            });
        }
        let attempt_id = super::bootstrap::random_id("attempt")?;
        let send_authority = super::bootstrap::random_id("send")?;
        tx.write("UPDATE main.gogoke_action_reservations SET state='dispatching',send_authority=? WHERE operation_id=? AND reservation_id=? AND semantic_digest=? AND state='reserved'", &[&send_authority,&request.operation_id,&request.reservation_id,&action[0][0]])?;
        tx.write("UPDATE main.gogoke_action_authority_intents SET attempt_id=?,send_authority=? WHERE domain_id=? AND operation_id=? AND attempt_id IS NULL AND send_authority IS NULL", &[&attempt_id,&send_authority,&request.domain_id,&request.operation_id])?;
        let committed=tx.query("SELECT a.state,a.send_authority,i.attempt_id,i.send_authority FROM main.gogoke_action_reservations a JOIN main.gogoke_action_authority_intents i ON i.operation_id=a.operation_id AND i.domain_id=? WHERE a.operation_id=? AND a.reservation_id=?", &[&request.domain_id,&request.operation_id,&request.reservation_id],4)?;
        if committed.len() != 1
            || committed[0][0] != "dispatching"
            || committed[0][1] != send_authority
            || committed[0][2] != attempt_id
            || committed[0][3] != send_authority
        {
            return Err(OrchestrationError::OperationConflict);
        }
        Ok(BeginCommittedDisposition::Granted {
            attempt_id,
            send_authority,
        })
    })
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    if value.len() % 2 != 0
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return denied();
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte: u8| -> Result<u8> {
                match byte {
                    b'0'..=b'9' => Ok(byte - b'0'),
                    b'a'..=b'f' => Ok(byte - b'a' + 10),
                    _ => denied(),
                }
            };
            Ok((digit(pair[0])? << 4) | digit(pair[1])?)
        })
        .collect()
}

#[cfg(test)]
pub(super) mod tests {
    include!("action_authority_tests.rs");

    mod native_current_facts {
        include!("action_current_facts_tests.rs");
    }
}

/// Only durable references and Action identity cross the Product Authority
/// boundary. Grant, Task, Decision, capacity, lineage, and recipe values are
/// resolved inside the same transaction; callers provide no authority snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrepareActionAuthority {
    pub domain_id: String,
    pub parent_grant_ref: String,
    pub package_operation_id: String,
    pub task_id: String,
    pub recipe_id: String,
    pub session_id: String,
    pub context_manifest_id: String,
    /// Expected identity coordinate only; begin rechecks a trusted current head.
    pub action_operation_id: String,
    pub reservation_id: String,
    pub action_kind: String,
    pub lane: String,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedActionAuthority {
    pub disposition: &'static str,
    pub reservation_state: String,
    pub authority_status: &'static str,
    pub operation_id: String,
    pub reservation_id: String,
    pub semantic_digest: String,
    pub package_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BeginCommittedAction {
    pub domain_id: String,
    pub operation_id: String,
    pub reservation_id: String,
}

/// Constructed only by trusted native-host code after it has resolved current
/// runtime/model/capability/manifest/admission facts. Never accepted from IPC.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(test)]
pub(crate) struct TrustedNativeActionFacts {
    pub domain_id: String,
    pub operation_id: String,
    pub expected_previous_revision: Option<String>,
    pub session_id: String,
    pub binding_id: String,
    pub generation: String,
    pub source_epoch: String,
    pub runtime_instance_id: String,
    pub model_ref_digest: String,
    pub capability_revision: String,
    pub context_manifest_id: String,
    pub context_manifest_hash: String,
    pub admission_ref: String,
    pub admission_revision: String,
    pub expires_at_epoch_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeActionCurrentFactsRefs {
    pub domain_id: String,
    pub operation_id: String,
    pub reservation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeActionFixtureSelection {
    pub profile_id: String,
    pub target_domain_id: String,
    pub generation: String,
    pub binding_id: String,
    pub source_epoch: String,
    pub native_session_id: String,
    pub runtime_instance_id: String,
    pub launch_digest_sha256: String,
    pub semantic_digest: String,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActionDecisionBasis {
    pub semantic_digest: String,
    pub state_view_hash: String,
    pub task_revision: String,
    pub policy_revision: String,
    pub binding_id: String,
    pub generation: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BeginCommittedDisposition {
    /// Returned only from the original successful commit, never from replay.
    Granted {
        attempt_id: String,
        send_authority: String,
    },
    /// An already committed begin is never a second send permission.
    Replay { state: String },
    /// The contracts currently stored in the Product DB are preparatory, so no
    /// current runtime/capability/admission fact can authorize this transition.
    CurrentFactsUnavailable { axes: Vec<String> },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActionCompletionDisposition {
    Completed,
    Rejected,
    AcceptanceUnknown,
}

/// Evidence accepted only from a trusted native/runtime receipt ingress. This
/// is not the session IPC `RecordActionOutcome` frame or provider/model text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrustedActionCompletionEvidence {
    pub domain_id: String,
    pub operation_id: String,
    pub reservation_id: String,
    pub semantic_digest: String,
    pub attempt_id: String,
    pub send_authority: String,
    pub binding_id: String,
    pub generation: String,
    pub source_epoch: String,
    pub runtime_instance_id: String,
    pub native_request_id: String,
    pub native_session_id: String,
    pub trusted_receipt_ref: String,
    pub evidence_hash: String,
    pub disposition: ActionCompletionDisposition,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActionCompletionReceipt {
    pub disposition: &'static str,
    pub terminal_state: &'static str,
    pub authority_status: &'static str,
    pub operation_id: String,
    pub receipt_id: String,
    pub semantic_digest: String,
}
