//! Native composition of F's durable metadata with H's exact credential
//! witnesses. Current A/H/Issuer admission is checked by the calling boundary.
//! References: instance credential_registry/private_history; process session
//! credential_binding/isolation. This module runs no CLI and reads no data.

use crate::process::{AppContainerProfile, CredentialAlias, CredentialAliasScope,
    CredentialBinding, CredentialError, PreparedCustody};
use crate::root::RootLock;
use crate::store::atomic::Statement;
use crate::store::digest::sha256_hex;
use crate::store::instance::{self, CredentialAliasAction, CredentialAliasIntent,
    CredentialAliasIntentReceipt, CredentialAliasPhysicalReceipt, CredentialAliasRecord,
    CredentialAliasResult, CredentialIntentDisposition, CredentialObjectInput,
    CredentialObjectRecord, CredentialProfileAction, CredentialProfileIntent,
    CredentialProfileRecord, CredentialProfileResult, PrivateHistoryGeneration,
    PrivateHistoryReceipt};
use crate::store::same_open::VerifiedDatabaseConnection;
use std::sync::Arc;

fn evidence<T, E: std::fmt::Debug>(value: Result<T, E>) -> Result<T, String> {
    value.map_err(|error| format!("credential launch: {error:?}"))
}
fn denied(message: &str) -> String { format!("credential launch: {message}") }
fn step_request(request: &str, step: &str) -> String {
    format!("credential-{}", sha256_hex(format!("{}:{request}{}:{step}", request.len(), step.len()).as_bytes()))
}
fn source_home(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    instance: &str) -> Result<instance::ResolvedDirectory, String> {
    let pin = evidence(Statement::prepare(db.as_ptr(),
        "SELECT program_digest,version FROM main.gogoke_v37_instances WHERE instance_id=?1 AND driver_id='codex'"))?;
    evidence(pin.bind_text(1, instance))?;
    if !evidence(pin.step_row())? { return Err(denied("registered Codex pin absent")); }
    let digest = evidence(pin.column_text(0))?;
    let version = evidence(pin.column_text(1))?;
    if digest.len() != 71 || !digest.starts_with("sha256:")
        || !digest[7..].bytes().all(|byte| byte.is_ascii_hexdigit()) || version != "0.160.0"
        || evidence(pin.step_row())? { return Err(denied("unqualified current Codex pin")); }
    evidence(instance::resolve_codex_instance_home(db, root, instance))
}
fn object_matches(db: &VerifiedDatabaseConnection<'_>, home: &instance::ResolvedDirectory,
    object: &CredentialObjectRecord, observed: &crate::root::RootIdentity) -> Result<(), String> {
    if object.root_identity != *db.root_identity() || object.home_identity != home.identity
        || object.source_parent_identity != home.identity || object.file_identity != *observed {
        return Err(denied("registered source/root/home identity changed"));
    }
    Ok(())
}
fn alias_scopes(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    object: &CredentialObjectRecord, rows: &[CredentialAliasRecord],
    allowed_pending: Option<&str>) -> Result<Vec<CredentialAliasScope>, String> {
    let mut scopes = Vec::new();
    for row in rows {
        if row.state == "REMOVED" { continue; }
        if !matches!(row.state.as_str(), "ACTIVE" | "DORMANT")
            && !(allowed_pending == Some(row.history_id.as_str())
                && matches!(row.state.as_str(), "PREPARING" | "UNKNOWN")) {
            return Err(denied("unresolved alias is retained; no new namespace action"));
        }
        if row.instance_id != object.instance_id || row.source_file_identity != object.file_identity {
            return Err(denied("alias belongs to another source"));
        }
        let directory = evidence(instance::resolve_private_history_directory(db, root, &row.history_id))?;
        if directory.identity != row.directory_identity { return Err(denied("alias scope identity changed")); }
        scopes.push(CredentialAliasScope { root: directory.path, root_identity: directory.identity });
    }
    Ok(scopes)
}

/// None means no F source registration yet. It is not an auth-absence claim.
/// The initial account observer is separately authorized by Root's quiescent
/// one-link metadata bootstrap, before usable backend evidence exists.
pub(crate) fn registered_credential_binding(db: &VerifiedDatabaseConnection<'_>,
    root: &RootLock, instance: &str)
    -> Result<Option<(Arc<CredentialBinding>, Vec<CredentialAliasScope>)>, String> {
    let Some(object) = evidence(instance::read_credential_object(db, instance))? else { return Ok(None); };
    let home = source_home(db, root, instance)?;
    let source = home.path.join("auth.json");
    let (observed, _) = evidence(CredentialBinding::observe_source_metadata(root, &source, &home.identity))?;
    object_matches(db, &home, &object, &observed)?;
    let rows = evidence(instance::read_credential_aliases(db, instance))?;
    let scopes = alias_scopes(db, root, &object, &rows, None)?;
    let binding = evidence(CredentialBinding::open_registered(root, &source,
        &home.identity, &object.file_identity, &scopes))?;
    Ok(Some((binding, scopes)))
}

fn stopped_instance(db: &VerifiedDatabaseConnection<'_>, instance: &str) -> Result<(), String> {
    for sql in [
        "SELECT 1 FROM main.gogoke_coordination_process_custody WHERE profile_id=?1
           AND (state<>'STOPPED' OR stop_proof_hash IS NULL OR stop_proof_hash='') LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_process_episode e LEFT JOIN main.gogoke_coordination_process_custody c
           ON c.operation_id=e.process_operation_id AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation
          WHERE e.instance_id=?1 AND NOT (e.phase='FAILED' AND e.process_operation_id IS NULL)
           AND (e.phase<>'STOPPED' OR c.state IS NULL OR c.state<>'STOPPED' OR e.stop_fact_id IS NULL
             OR e.stop_fact_id='' OR c.stop_proof_hash IS NULL OR e.stop_fact_id<>c.stop_proof_hash) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_claim a LEFT JOIN main.gogoke_coordination_process_custody c
           ON c.operation_id=a.process_operation_id AND c.profile_id=a.instance_id AND c.domain_id=a.domain_id AND c.generation=a.generation
          WHERE a.instance_id=?1 AND (a.state NOT IN ('STOPPED','RELEASED')
           OR (a.process_operation_id IS NOT NULL AND (c.state IS NULL OR c.state<>'STOPPED'
             OR a.stop_fact_id IS NULL OR a.stop_fact_id='' OR c.stop_proof_hash IS NULL OR a.stop_fact_id<>c.stop_proof_hash))) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_generation_change g JOIN main.gogoke_v37_h_process_episode e
           ON e.process_operation_id=g.old_process_operation_id WHERE e.instance_id=?1
             AND g.stage NOT IN ('APPLIED','CANCELLED','UNSUPPORTED') LIMIT 1",
    ] {
        let query = evidence(Statement::prepare(db.as_ptr(), sql))?;
        evidence(query.bind_text(1, instance))?;
        if evidence(query.step_row())? { return Err(denied("instance has unresolved or unstopped original H custody")); }
    }
    Ok(())
}

/// Called under Root's instance quiescence boundary after all original grants
/// are revoked and their holders are released. It never stops a model, removes
/// a history directory, or opens credential data. One name is removed per
/// durable intent; pending work receives only a read-only physical readback.
pub(crate) fn quiescent_cleanup(db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
    instance: &str, request: &str) -> Result<(), String> {
    let profiles = evidence(instance::read_credential_profiles(db, instance))?;
    if profiles.iter().any(|profile| profile.state != "REVOKED") {
        return Err(denied("cleanup requires all original F profiles revoked"));
    }
    let Some(object) = evidence(instance::read_credential_object(db, instance))? else {
        if !profiles.is_empty() || !evidence(instance::read_credential_aliases(db, instance))?.is_empty() {
            return Err(denied("credential inventory without source"));
        }
        return Ok(());
    };
    let home = source_home(db, root, instance)?;
    let source = home.path.join("auth.json");
    let (identity, _) = evidence(CredentialBinding::observe_source_metadata(root, &source, &home.identity))?;
    object_matches(db, &home, &object, &identity)?;
    loop {
        stopped_instance(db, instance)?;
        let rows = evidence(instance::read_credential_aliases(db, instance))?;
        if rows.iter().any(|row| !matches!(row.state.as_str(), "DORMANT" | "REMOVED" | "REMOVE_PENDING")) {
            return Err(denied("cleanup retains active or unknown alias intents"));
        }
        // A pending original remove is reconciled before reserving any new
        // side effect. Other unresolved journal entries remain blockers.
        let selected = rows.iter().find(|row| row.state == "REMOVE_PENDING")
            .or_else(|| rows.iter().find(|row| row.state == "DORMANT"));
        let target = format!("credential-instance-{}", sha256_hex(instance.as_bytes()));
        let query = evidence(Statement::prepare(db.as_ptr(),
            "SELECT request_id,phase FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase IN ('PREPARING','UNKNOWN')"))?;
        evidence(query.bind_text(1, &target))?;
        while evidence(query.step_row())? {
            let key = evidence(query.column_text(0))?;
            let phase = evidence(query.column_text(1))?;
            if !selected.is_some_and(|row| row.state == "REMOVE_PENDING"
                && key == row.intent_request)
                || phase != "PREPARING" {
                return Err(denied("cleanup retains another unresolved original journal"));
            }
        }
        let Some(row) = selected else { return Ok(()); };
        let recovering = row.state == "REMOVE_PENDING";
        let mut scopes = Vec::new();
        let mut scope = None;
        for alias in rows.iter().filter(|alias| alias.state != "REMOVED") {
            if alias.instance_id != object.instance_id || alias.source_file_identity != object.file_identity {
                return Err(denied("cleanup source association changed"));
            }
            let directory = evidence(instance::resolve_private_history_directory(db, root, &alias.history_id))?;
            if directory.identity != alias.directory_identity { return Err(denied("cleanup directory identity changed")); }
            let exact = CredentialAliasScope { root: directory.path, root_identity: directory.identity };
            if alias.history_id == row.history_id { scope = Some(exact.clone()); }
            scopes.push(exact);
        }
        let scope = scope.ok_or_else(|| denied("cleanup original scope absent"))?;
        let revision = if recovering { row.revision.checked_sub(1).ok_or_else(|| denied("remove revision invalid"))? }
            else { row.revision };
        let input = CredentialAliasIntent { request_id: step_request(request, &format!("remove:{}", row.history_id)),
            instance_id: instance.into(), history_id: row.history_id.clone(), directory_identity: row.directory_identity.clone(),
            source_file_identity: object.file_identity.clone(), expected_revision: revision, action: CredentialAliasAction::Remove };
        let binding = if recovering { None } else {
            let binding = evidence(CredentialBinding::open_registered(root, &source, &home.identity, &object.file_identity, &scopes))?;
            if Arc::strong_count(&binding) != 1 { return Err(denied("cleanup credential holder is still shared")); }
            Some(binding)
        };
        let intent = evidence(instance::begin_credential_alias(db, &input))?;
        let remaining: Vec<_> = scopes.iter().filter(|entry| **entry != scope).cloned().collect();
        let physical = if recovering {
            if intent.disposition != CredentialIntentDisposition::Pending {
                return Err(denied("cleanup recovery did not select original remove intent"));
            }
            evidence(CredentialBinding::verify_removed_alias(root, &source, &home.identity,
                &object.file_identity, &scope, &remaining))?
        } else {
            if intent.disposition != CredentialIntentDisposition::New { return Err(denied("remove was not newly reserved")); }
            stopped_instance(db, instance)?;
            let binding = binding.ok_or_else(|| denied("cleanup held source absent"))?;
            let alias = evidence(binding.alias(&scope, &scopes))?;
            evidence(CredentialBinding::remove_quiescent_alias(binding, root, &alias, &scopes))?
        };
        if physical.source_identity != object.file_identity || physical.removed != scope || physical.remaining_aliases != remaining {
            return Err(denied("cleanup physical receipt changed original namespace"));
        }
        evidence(instance::complete_credential_alias(db, &intent, &CredentialAliasPhysicalReceipt {
            source_file_identity: physical.source_identity, directory_identity: physical.removed.root_identity,
            observed_nlink: u64::from(physical.remaining_links), result: CredentialAliasResult::Removed }))?;
    }
}

fn known_profile_custody(db: &VerifiedDatabaseConnection<'_>, profile: &CredentialProfileRecord) -> Result<(), String> {
    let generation = evidence(instance::read_private_history_generation(db, &profile.binding_id, &profile.generation))?
        .ok_or_else(|| denied("old profile F generation absent"))?;
    if generation.instance_id != profile.instance_id || generation.history_id != profile.history_id {
        return Err(denied("old profile F generation changed"));
    }
    let query = evidence(Statement::prepare(db.as_ptr(),
        "SELECT c.operation_id,c.ticket,c.custodian_nonce FROM main.gogoke_v37_h_process_episode e
           JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id
             AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation
          WHERE e.binding_id=?1 AND e.generation=?2 AND e.instance_id=?3 AND e.domain_id=?4 AND e.session_id=?5
            AND ((e.phase='ACTIVE' AND c.state='ACTIVE') OR (e.phase='PREPARED' AND c.state='PREPARED')
             OR (e.phase='STOPPED' AND c.state='STOPPED' AND e.stop_fact_id=c.stop_proof_hash
                 AND e.stop_fact_id IS NOT NULL AND e.stop_fact_id<>''))"))?;
    for (index, value) in [&profile.binding_id, &profile.generation, &profile.instance_id,
        &generation.domain_id, &generation.session_id].iter().enumerate() {
        evidence(query.bind_text((index + 1) as i32, value))?;
    }
    if !evidence(query.step_row())? { return Err(denied("old profile custody is unresolved")); }
    if let Some(source) = &generation.source {
        if evidence(query.column_text(0))? != source.process_operation_id
            || evidence(query.column_text(1))? != source.ticket
            || evidence(query.column_text(2))? != source.custodian_nonce {
            return Err(denied("old profile original custody source changed"));
        }
    } else { return Err(denied("old profile original F custody source absent")); }
    if evidence(query.step_row())? { return Err(denied("ambiguous old profile custody")); }
    Ok(())
}
fn permit_new_grant(db: &VerifiedDatabaseConnection<'_>, binding: &CredentialBinding,
    instance: &str, profiles: &[CredentialProfileRecord]) -> Result<(), String> {
    for profile in profiles {
        match profile.state.as_str() {
            "REVOKED" => (), "ACTIVE" => known_profile_custody(db, profile)?,
            _ => return Err(denied("old pending/unknown profile forbids a new grant")),
        }
    }
    if !evidence(binding.acl_prepared_in_this_holder())? {
        if profiles.iter().any(|row| row.state != "REVOKED") {
            return Err(denied("fresh holder cannot rebuild an old active profile DACL"));
        }
        let query = evidence(Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_coordination_process_custody WHERE profile_id=?1 AND state<>'STOPPED' LIMIT 1"))?;
        evidence(query.bind_text(1, instance))?;
        if evidence(query.step_row())? { return Err(denied("fresh holder requires stopped instance custody")); }
        let query = evidence(Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_h_claim WHERE instance_id=?1 AND
             (state='UNKNOWN' OR (process_operation_id IS NOT NULL AND state<>'STOPPED')) LIMIT 1"))?;
        evidence(query.bind_text(1, instance))?;
        if evidence(query.step_row())? { return Err(denied("fresh holder requires resolved stopped H objects")); }
    }
    Ok(())
}

fn alias_input(history: &PrivateHistoryGeneration, directory: &instance::ResolvedDirectory,
    object: &CredentialObjectRecord, request: &str, revision: i64,
    action: CredentialAliasAction) -> CredentialAliasIntent {
    CredentialAliasIntent { request_id: step_request(request, "alias"), instance_id: history.instance_id.clone(),
        history_id: history.history_id.clone(), directory_identity: directory.identity.clone(),
        source_file_identity: object.file_identity.clone(), expected_revision: revision, action }
}
fn recover_alias_intent(db: &mut VerifiedDatabaseConnection<'_>, history: &PrivateHistoryGeneration,
    directory: &instance::ResolvedDirectory, object: &CredentialObjectRecord,
    request: &str, row: &CredentialAliasRecord) -> Result<CredentialAliasIntentReceipt, String> {
    let revision = row.revision.checked_sub(1).ok_or_else(|| denied("alias revision invalid"))?;
    // F compares the complete original journal fingerprint. These two legal
    // open operations share one request key; neither may create a new intent
    // against a PREPARING/UNKNOWN row. There is no filesystem retry here.
    for action in [CredentialAliasAction::Create, CredentialAliasAction::Reactivate] {
        let input = alias_input(history, directory, object, request, revision, action);
        match instance::begin_credential_alias(db, &input) {
            Ok(receipt) if matches!(receipt.disposition, CredentialIntentDisposition::Pending | CredentialIntentDisposition::Unknown) => return Ok(receipt),
            Ok(_) => return Err(denied("alias recovery did not select the pending original intent")),
            Err(instance::CredentialRegistryError::Conflict) => (),
            Err(error) => return Err(format!("credential original alias intent: {error:?}")),
        }
    }
    Err(denied("pending alias is owned by another original request"))
}

pub(crate) struct CredentialLaunch {
    pub(crate) binding: Arc<CredentialBinding>,
    pub(crate) alias: CredentialAlias,
    history: PrivateHistoryGeneration,
    object: CredentialObjectRecord,
    profile_record: CredentialProfileRecord,
}
impl CredentialLaunch {
    pub(crate) fn prepare(db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
        profile: &AppContainerProfile, history: &PrivateHistoryReceipt, request: &str)
        -> Result<Option<Self>, String> {
        let home = source_home(db, root, &history.generation.instance_id)?;
        let source = home.path.join("auth.json");
        let existing = evidence(instance::read_credential_object(db, &history.generation.instance_id))?;
        let rows = evidence(instance::read_credential_aliases(db, &history.generation.instance_id))?;
        let profiles = evidence(instance::read_credential_profiles(db, &history.generation.instance_id))?;
        let (identity, links) = match CredentialBinding::observe_source_metadata(root, &source, &home.identity) {
            Ok(metadata) => metadata,
            Err(CredentialError::Io { operation: "observe source metadata", source: error })
                if error.raw_os_error() == Some(2) && existing.is_none()
                    && rows.is_empty() && profiles.is_empty() => return Ok(None),
            Err(error) => return Err(format!("credential original source metadata: {error:?}")),
        };
        evidence(instance::read_usable_credential_backend(db, &history.generation.instance_id))?;
        let object = if let Some(object) = existing { object } else {
            let input = CredentialObjectInput {
                request_id: step_request(request, "source"), instance_id: history.generation.instance_id.clone(),
                root_identity: db.root_identity().clone(), home_identity: home.identity.clone(),
                file_identity: identity.clone(), source_parent_identity: home.identity.clone(),
                observed_nlink: u64::from(links), expected_revision: 0, rebind_from: None };
            evidence(instance::bind_credential_object(db, &input))?
        };
        object_matches(db, &home, &object, &identity)?;
        let registered = evidence(instance::resolve_private_history_directory(db, root, &history.generation.history_id))?;
        if registered != history.directory { return Err(denied("current private history physical identity changed")); }
        let current = rows.iter().find(|row| row.history_id == history.generation.history_id);
        let pending = current.is_some_and(|row| matches!(row.state.as_str(), "PREPARING" | "UNKNOWN"));
        let scopes = alias_scopes(db, root, &object, &rows,
            if pending { Some(history.generation.history_id.as_str()) } else { None })?;
        let binding = evidence(CredentialBinding::open_registered(root, &source, &home.identity, &object.file_identity, &scopes))?;
        let scope = CredentialAliasScope { root: registered.path.clone(), root_identity: registered.identity.clone() };
        let alias = if pending {
            let intent = recover_alias_intent(db, &history.generation, &registered, &object, request, current.ok_or_else(|| denied("pending alias absent"))?)?;
            let exact = evidence(binding.alias(&scope, &scopes))?;
            evidence(instance::complete_credential_alias(db, &intent, &CredentialAliasPhysicalReceipt {
                source_file_identity: object.file_identity.clone(), directory_identity: registered.identity.clone(),
                observed_nlink: u64::from(links), result: CredentialAliasResult::Active }))?;
            exact
        } else if current.is_some_and(|row| row.state == "ACTIVE") {
            evidence(binding.alias(&scope, &scopes))?
        } else {
            let (action, revision) = match current {
                None => (CredentialAliasAction::Create, 0),
                Some(row) if row.state == "REMOVED" => (CredentialAliasAction::Create, row.revision),
                Some(row) if row.state == "DORMANT" => (CredentialAliasAction::Reactivate, row.revision),
                _ => return Err(denied("unresolved current alias")),
            };
            let intent = evidence(instance::begin_credential_alias(db, &alias_input(&history.generation, &registered, &object, request, revision, action)))?;
            if intent.disposition != CredentialIntentDisposition::New { return Err(denied("new alias operation was not newly reserved")); }
            let exact = if action == CredentialAliasAction::Create {
                evidence(binding.create_alias(root, scope.clone(), &scopes))?
            } else { evidence(binding.alias(&scope, &scopes))? };
            let (_, links) = evidence(CredentialBinding::observe_source_metadata(root, &source, &home.identity))?;
            evidence(instance::complete_credential_alias(db, &intent, &CredentialAliasPhysicalReceipt {
                source_file_identity: object.file_identity.clone(), directory_identity: registered.identity.clone(),
                observed_nlink: u64::from(links), result: CredentialAliasResult::Active }))?;
            exact
        };
        let sid = evidence(profile.sid_identity())?;
        let prior = profiles.iter().find(|row| row.binding_id == history.generation.binding_id);
        let profile_record = if let Some(row) = prior.filter(|row| row.state == "ACTIVE") {
            if row.generation != history.generation.generation || row.history_id != history.generation.history_id
                || row.profile_sid != sid || row.source_file_identity != object.file_identity {
                return Err(denied("current profile original binding changed"));
            }
            evidence(profile.verify_credential_alias(&binding, &alias))?;
            row.clone()
        } else {
            let recovering = prior.is_some_and(|row| matches!(row.state.as_str(), "GRANT_PENDING" | "UNKNOWN"));
            let revision = if recovering { prior.ok_or_else(|| denied("profile absent"))?.revision.checked_sub(1)
                .ok_or_else(|| denied("profile revision invalid"))? } else { prior.map_or(0, |row| row.revision) };
            if !recovering { permit_new_grant(db, &binding, &object.instance_id, &profiles)?; }
            let intent = evidence(instance::begin_credential_profile(db, &CredentialProfileIntent {
                request_id: step_request(request, "grant"), instance_id: object.instance_id.clone(),
                history_id: history.generation.history_id.clone(), binding_id: history.generation.binding_id.clone(),
                generation: history.generation.generation.clone(), profile_sid: sid,
                source_file_identity: object.file_identity.clone(), expected_revision: revision, action: CredentialProfileAction::Grant }))?;
            if recovering {
                if !matches!(intent.disposition, CredentialIntentDisposition::Pending | CredentialIntentDisposition::Unknown) {
                    return Err(denied("profile recovery did not select original pending grant"));
                }
                evidence(profile.verify_credential_alias(&binding, &alias))?;
            } else {
                if intent.disposition != CredentialIntentDisposition::New { return Err(denied("grant was not newly reserved")); }
                evidence(profile.grant_credential_alias(&binding, &alias))?;
                evidence(profile.verify_credential_alias(&binding, &alias))?;
            }
            evidence(instance::complete_credential_profile(db, &intent, CredentialProfileResult::Active))?
        };
        let result = Self { binding, alias, history: history.generation.clone(), object, profile_record };
        result.verify(db, root, profile)?;
        Ok(Some(result))
    }
    pub(crate) fn verify(&self, db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
        profile: &AppContainerProfile) -> Result<(), String> {
        evidence(instance::read_usable_credential_backend(db, &self.object.instance_id))?;
        let generation = evidence(instance::read_private_history_generation(db,
            &self.history.binding_id, &self.history.generation))?
            .ok_or_else(|| denied("current F generation absent"))?;
        if generation.history_id != self.history.history_id || generation.instance_id != self.history.instance_id
            || generation.domain_id != self.history.domain_id || generation.session_id != self.history.session_id
            || generation.seat_id != self.history.seat_id || generation.seat_incarnation != self.history.seat_incarnation
            || generation.request_id != self.history.request_id {
            return Err(denied("current F generation association changed"));
        }
        let object = evidence(instance::read_credential_object(db, &self.object.instance_id))?
            .ok_or_else(|| denied("registered object absent"))?;
        if object.file_identity != self.object.file_identity || object.home_identity != self.object.home_identity
            || object.root_identity != self.object.root_identity || object.source_parent_identity != self.object.source_parent_identity {
            return Err(denied("registered credential object changed"));
        }
        let (_, scopes) = registered_credential_binding(db, root, &self.object.instance_id)?
            .ok_or_else(|| denied("registered source absent"))?;
        evidence(self.binding.verify_registered_aliases(&scopes))?;
        let directory = evidence(instance::resolve_private_history_directory(db, root, &self.history.history_id))?;
        if directory.path != self.alias.root() || &directory.identity != self.alias.root_identity() {
            return Err(denied("retained credential scope changed"));
        }
        let alias_row = evidence(instance::read_credential_aliases(db, &self.object.instance_id))?
            .into_iter().find(|row| row.history_id == self.history.history_id)
            .ok_or_else(|| denied("registered alias absent"))?;
        if alias_row.state != "ACTIVE" || alias_row.directory_identity != directory.identity
            || alias_row.source_file_identity != self.object.file_identity {
            return Err(denied("registered active alias changed"));
        }
        let profile_row = evidence(instance::read_credential_profiles(db, &self.object.instance_id))?
            .into_iter().find(|row| row.binding_id == self.profile_record.binding_id)
            .ok_or_else(|| denied("registered profile absent"))?;
        if profile_row != self.profile_record || profile_row.state != "ACTIVE"
            || profile_row.profile_sid != evidence(profile.sid_identity())? {
            return Err(denied("registered profile/current SID changed"));
        }
        evidence(profile.verify_credential_alias(&self.binding, &self.alias))
    }
    pub(crate) fn revoke(&self, db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
        profile: &AppContainerProfile, operation: &str, custody: &PreparedCustody) -> Result<(), String> {
        if custody.binding.profile_id != self.object.instance_id || custody.binding.domain_id != self.history.domain_id
            || custody.binding.generation != self.history.generation
            || evidence(profile.sid_identity())? != self.profile_record.profile_sid { return Err(denied("stop custody/profile changed")); }
        let query = evidence(Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_coordination_process_custody c JOIN main.gogoke_v37_h_process_episode e
               ON e.process_operation_id=c.operation_id AND e.instance_id=c.profile_id AND e.domain_id=c.domain_id AND e.generation=c.generation
              WHERE c.operation_id=?1 AND c.ticket=?2 AND c.custodian_nonce=?3 AND c.profile_id=?4 AND c.domain_id=?5 AND c.generation=?6
                AND e.binding_id=?7 AND e.session_id=?8 AND c.binary_digest_sha256=?9 AND c.pid=?10 AND c.creation_time_100ns=?11 AND c.image_path=?12
                AND c.state='STOPPED' AND e.phase='STOPPED' AND c.stop_proof_hash=e.stop_fact_id
                AND e.stop_fact_id IS NOT NULL AND e.stop_fact_id<>''"))?;
        let ticket = custody.ticket.opaque(); let pid = custody.identity.pid.to_string();
        let created = custody.identity.creation_time_100ns.to_string();
        let image = custody.identity.image_path.to_string_lossy();
        for (index, value) in [operation, ticket, custody.custodian_nonce.as_str(), self.object.instance_id.as_str(),
            self.history.domain_id.as_str(), self.history.generation.as_str(), self.history.binding_id.as_str(),
            self.history.session_id.as_str(), custody.binding.binary_digest_sha256.as_str(), pid.as_str(), created.as_str(), image.as_ref()].iter().enumerate() {
            evidence(query.bind_text((index + 1) as i32, value))?;
        }
        if !evidence(query.step_row())? || evidence(query.step_row())? { return Err(denied("exact original STOPPED custody absent")); }
        let home = source_home(db, root, &self.object.instance_id)?;
        let object = evidence(instance::read_credential_object(db, &self.object.instance_id))?
            .ok_or_else(|| denied("original registered source absent"))?;
        let (identity, _) = evidence(CredentialBinding::observe_source_metadata(root, &home.path.join("auth.json"), &home.identity))?;
        object_matches(db, &home, &object, &identity)?;
        if object.file_identity != self.object.file_identity { return Err(denied("original revoke source changed")); }
        let rows = evidence(instance::read_credential_aliases(db, &self.object.instance_id))?;
        if rows.iter().any(|row| row.state == "UNKNOWN") { return Err(denied("unknown alias retained during revoke")); }
        let scopes = alias_scopes(db, root, &object, &rows, Some(&self.history.history_id))?;
        evidence(self.binding.verify_registered_aliases(&scopes))?;
        let current = evidence(instance::read_credential_profiles(db, &self.object.instance_id))?.into_iter()
            .find(|row| row.binding_id == self.profile_record.binding_id).ok_or_else(|| denied("original profile absent"))?;
        if current.history_id != self.history.history_id || current.generation != self.history.generation
            || current.profile_sid != self.profile_record.profile_sid || current.source_file_identity != self.object.file_identity {
            return Err(denied("original revoke profile association changed"));
        }
        if current.state != "REVOKED" {
        let recovering = current.state == "REVOKE_PENDING";
        if !recovering && current.state != "ACTIVE" { return Err(denied("unknown original revoke retained")); }
        let revision = if recovering { current.revision.checked_sub(1).ok_or_else(|| denied("revoke revision invalid"))? }
            else { current.revision };
        let intent = evidence(instance::begin_credential_profile(db, &CredentialProfileIntent {
            request_id: step_request(operation, "revoke"), instance_id: self.object.instance_id.clone(),
            history_id: self.history.history_id.clone(), binding_id: self.history.binding_id.clone(),
            generation: self.history.generation.clone(), profile_sid: current.profile_sid,
            source_file_identity: self.object.file_identity.clone(), expected_revision: revision,
            action: CredentialProfileAction::Revoke }))?;
        if recovering {
            if intent.disposition != CredentialIntentDisposition::Pending { return Err(denied("revoke did not select original pending intent")); }
            evidence(profile.verify_revoked_credential_alias(&self.binding, &self.alias))?;
        } else {
            if intent.disposition != CredentialIntentDisposition::New { return Err(denied("revoke was not newly reserved")); }
            evidence(profile.revoke_credential_alias(&self.binding, &self.alias))?;
            evidence(profile.verify_revoked_credential_alias(&self.binding, &self.alias))?;
        }
        evidence(instance::complete_credential_profile(db, &intent, CredentialProfileResult::Revoked))?;
        } else { evidence(profile.verify_revoked_credential_alias(&self.binding, &self.alias))?; }
        let others = evidence(instance::read_credential_profiles(db, &self.object.instance_id))?;
        if !others.iter().any(|row| row.history_id == self.history.history_id && row.state != "REVOKED") {
            let aliases = evidence(instance::read_credential_aliases(db, &self.object.instance_id))?;
            let row = aliases.iter().find(|row| row.history_id == self.history.history_id).ok_or_else(|| denied("original alias absent"))?;
            if matches!(row.state.as_str(), "ACTIVE" | "PREPARING") {
                let recovering = row.state == "PREPARING";
                let revision = if recovering { row.revision.checked_sub(1).ok_or_else(|| denied("dormant revision invalid"))? }
                    else { row.revision };
                let input = CredentialAliasIntent { request_id: step_request(operation, "dormant"), instance_id: self.object.instance_id.clone(),
                    history_id: self.history.history_id.clone(), directory_identity: self.alias.root_identity().clone(),
                    source_file_identity: self.object.file_identity.clone(), expected_revision: revision, action: CredentialAliasAction::Dormant };
                let intent = evidence(instance::begin_credential_alias(db, &input))?;
                if (!recovering && intent.disposition != CredentialIntentDisposition::New)
                    || (recovering && intent.disposition != CredentialIntentDisposition::Pending) {
                    return Err(denied("dormant did not select its exact original intent"));
                }
                let home = source_home(db, root, &self.object.instance_id)?;
                let (_, links) = evidence(CredentialBinding::observe_source_metadata(root, &home.path.join("auth.json"), &home.identity))?;
                evidence(instance::complete_credential_alias(db, &intent, &CredentialAliasPhysicalReceipt {
                    source_file_identity: self.object.file_identity.clone(), directory_identity: self.alias.root_identity().clone(),
                    observed_nlink: u64::from(links), result: CredentialAliasResult::Dormant }))?;
            } else if row.state != "DORMANT" { return Err(denied("original alias dormant transition unresolved")); }
        }
        Ok(())
    }
}
