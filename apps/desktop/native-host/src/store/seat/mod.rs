//! E.1 seat identity, template copy, and instance binding on the product database connection.
//! This module receives only native capabilities. It is not an IPC dispatcher;
//! the future ingress must authenticate a live lead session before admission.

use super::atomic::{AtomicError, Json, JsonString, Parser, Statement};
use super::authority::{check_owner_in_current_transaction, OwnerIssuer};
use super::same_open::{SameOpenError, VerifiedDatabaseConnection};

#[cfg(all(test, windows))]
mod tests;
mod resource;
mod policy;
mod continuity;
mod orchestration;
mod page_facts;
mod secretary;
mod secretary_routines;
pub(crate) mod secretary_schedule;
pub(crate) use secretary::{configure_secretary, designate_secretary,
    read_secretary_configuration_in_transaction, require_secretary_session, SecretaryConfiguration,
    SecretaryDesignation};
pub(crate) use secretary_routines::{create_secretary_routine,change_secretary_routine,
    create_secretary_routine_from_model_in_transaction,replay_secretary_routine_from_model_in_transaction,
    read_secretary_routines_in_transaction,record_user_presence,record_user_presence_in_transaction,
    configure_absence_policy,
    read_secretary_presence_in_transaction,SecretaryPresenceFact,SecretaryAbsencePolicyFact,
    read_secretary_occurrences_in_transaction,SecretaryOccurrenceFact,
    take_due_secretary_routine_in_transaction,record_secretary_occurrence_outcome_in_transaction,
    SecretaryOccurrenceOutcome,SecretaryRoutine,SecretaryRoutineCreate,
    SecretaryRoutineChange,SecretaryRoutineCommand,SecretaryRoutineDecision,
    UserPresenceKind};
pub(crate) use page_facts::{designate_project_lead,list_page_facts,list_templates,
    rename_seat,PageSeatFacts,SeatActionFacts,SeatPageFacts,TemplateChoice};
pub(crate) use resource::{read_effective_project_parallel_cap,read_host_parallel_fact,
    refresh_host_parallel_fact_in_transaction,HostParallelFact};
pub(crate) use policy::{authorize_current_call,authorize_merge_for_f2,
    HostEscalationProof,observe_host_reject_cap_in_transaction,observe_host_stalled_in_transaction,
    revalidate_host_escalation_in_transaction,read_host_escalation_intent_in_transaction,
    begin_host_escalation_in_transaction,
    apply_owner_policy_configuration,
    begin_escalation,begin_trigger_cancel,begin_trigger_register,configure_call_grant,
    ensure_side_message_pair,
    configure_escalation_route,configure_gate,
    current_call_permission_table,current_policy_revision,gate_decide,gate_submit,initialize_policy,
    policy_revision_for_native_request,
    mark_escalation_unknown,mark_trigger_unknown,recover_trigger,settle_escalation,
    settle_trigger,stage_transition,CallAction,
    CallPermissionRow,EscalationCause,EscalationIntent,GateDecision,NativeDeliveryEvidence,
    NativeCoordinatorTriggerEvidence,NativeSeatCall,OwnerPolicyCommand,PolicyEvent,TriggerTransition};
pub(crate) use continuity::{answer_takeover,answer_takeover_at_seat_revision,answer_takeover_from_written_source,mark_health_requested,observe_health,
    observe_host_health_in_transaction,request_host_health_in_transaction,observe_stalled_host_health_in_transaction,
    read_state_card,settle_health_receipt,takeover_questions,takeover_ready,
    update_state_card,AnswerBasis,HealthObservation,HealthSignal,StateCard,TakeoverAnswer,
    TakeoverQuestion};
pub(crate) use orchestration::{authorize_child_dispatch,current_child_dispatch_context,
    orchestration_scope,validate_new_scope,
    render_codex_instruction,seat_effort,OrchestrationScope,RenderedInstruction};

const LEGACY_SEATS: &str = "CREATE TABLE gogoke_v37_seats(domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,incarnation TEXT NOT NULL UNIQUE,layer TEXT NOT NULL CHECK(layer IN ('USER','LEAD')),parent_seat_id TEXT,kind TEXT NOT NULL CHECK(kind IN ('LONG','SHORT')),instance_id TEXT NOT NULL REFERENCES gogoke_v37_instances(instance_id),state TEXT NOT NULL CHECK(state IN ('IDLE','BUSY','RECLAIMED')),generation INTEGER NOT NULL CHECK(generation >= 1),revision INTEGER NOT NULL CHECK(revision >= 1),CHECK((layer='USER' AND parent_seat_id IS NULL) OR (layer='LEAD' AND parent_seat_id IS NOT NULL)),PRIMARY KEY(domain_id,seat_id),FOREIGN KEY(domain_id,parent_seat_id) REFERENCES gogoke_v37_seats(domain_id,seat_id)) STRICT";
const SEATS: &str = "CREATE TABLE gogoke_v37_seats(domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,incarnation TEXT NOT NULL UNIQUE,layer TEXT NOT NULL CHECK(layer IN ('USER','LEAD')),parent_seat_id TEXT,kind TEXT NOT NULL CHECK(kind IN ('LONG','SHORT')),instance_id TEXT REFERENCES gogoke_v37_instances(instance_id),state TEXT NOT NULL CHECK(state IN ('IDLE','BUSY','RECLAIMED')),generation INTEGER NOT NULL CHECK(generation >= 1),revision INTEGER NOT NULL CHECK(revision >= 1),CHECK((layer='USER' AND parent_seat_id IS NULL) OR (layer='LEAD' AND parent_seat_id IS NOT NULL)),PRIMARY KEY(domain_id,seat_id),FOREIGN KEY(domain_id,parent_seat_id) REFERENCES gogoke_v37_seats(domain_id,seat_id)) STRICT";
const OPERATIONS: &str = "CREATE TABLE gogoke_v37_seat_operations(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,fingerprint TEXT NOT NULL,seat_id TEXT NOT NULL,incarnation TEXT NOT NULL,layer TEXT NOT NULL,parent_seat_id TEXT,kind TEXT NOT NULL,instance_id TEXT NOT NULL,state TEXT NOT NULL,revision INTEGER NOT NULL,generation INTEGER NOT NULL,PRIMARY KEY(domain_id,request_id)) STRICT";
const TEMPLATES: &str = "CREATE TABLE gogoke_v37_seat_templates(domain_id TEXT NOT NULL,template_id TEXT NOT NULL,settings_json TEXT NOT NULL CHECK(length(settings_json) > 0),revision INTEGER NOT NULL CHECK(revision >= 1),PRIMARY KEY(domain_id,template_id)) STRICT";
const SETTINGS: &str = "CREATE TABLE gogoke_v37_seat_settings(domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,template_id TEXT NOT NULL,settings_json TEXT NOT NULL CHECK(length(settings_json) > 0),PRIMARY KEY(domain_id,seat_id),FOREIGN KEY(domain_id,seat_id) REFERENCES gogoke_v37_seats(domain_id,seat_id)) STRICT";
const OPERATION_SNAPSHOTS: &str = "CREATE TABLE gogoke_v37_seat_operation_snapshots(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,template_id TEXT NOT NULL,settings_json TEXT NOT NULL CHECK(length(settings_json) > 0),PRIMARY KEY(domain_id,request_id),FOREIGN KEY(domain_id,request_id) REFERENCES gogoke_v37_seat_operations(domain_id,request_id)) STRICT";
const PROJECT_CAPS: &str = "CREATE TABLE gogoke_v37_seat_project_caps(domain_id TEXT PRIMARY KEY,parallel_cap INTEGER NOT NULL CHECK(parallel_cap > 0)) STRICT";

#[derive(Debug)]
pub(crate) enum SeatError {
    Invalid(&'static str),
    HostResourceObservation(String),
    HostHealthObservation(String),
    NativeAnswerSource(String),
    InstanceManagement(String),
    Denied,
    Conflict,
    Busy,
    Unknown,
    SchemaDrift,
    Store(AtomicError),
    Open(SameOpenError),
    CommitUnknown(SameOpenError),
    RollbackUnknown(SameOpenError),
}
impl From<AtomicError> for SeatError {
    fn from(error: AtomicError) -> Self {
        Self::Store(error)
    }
}
impl From<SameOpenError> for SeatError {
    fn from(error: SameOpenError) -> Self {
        Self::Open(error)
    }
}
impl From<SeatError> for super::orchestration::OrchestrationError {
    fn from(error: SeatError) -> Self {
        use super::orchestration::OrchestrationError;
        match error {
            SeatError::Invalid(field) => OrchestrationError::Invalid(field),
            SeatError::Denied => OrchestrationError::AccessDenied,
            SeatError::Conflict => OrchestrationError::OperationConflict,
            SeatError::Store(source) => OrchestrationError::Atomic(source),
            SeatError::CommitUnknown(source) => OrchestrationError::CommitUnknownWithCause(source),
            // The remaining variants lack an exact typed destination. Retain
            // the full native error and cause rather than collapsing them.
            other => OrchestrationError::V37StoreFailure(format!("seat: {other:?}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Seat {
    pub(crate) domain_id: String,
    pub(crate) seat_id: String,
    pub(crate) incarnation: String,
    pub(crate) layer: Layer,
    pub(crate) parent_seat_id: Option<String>,
    pub(crate) kind: Kind,
    /// Empty is the native view of a SQL NULL until bind_instance succeeds.
    pub(crate) instance_id: String,
    pub(crate) template_id: Option<String>,
    pub(crate) settings_json: Option<String>,
    pub(crate) state: State,
    pub(crate) generation: i64,
    pub(crate) revision: i64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Layer {
    User,
    Lead,
}
impl Layer {
    fn sql(self) -> &'static str {
        match self {
            Self::User => "USER",
            Self::Lead => "LEAD",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Long,
    Short,
}
impl Kind {
    fn sql(self) -> &'static str {
        match self {
            Self::Long => "LONG",
            Self::Short => "SHORT",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum State {
    Idle,
    Busy,
    Reclaimed,
}

/// Persisted seat intent. This is an input to H's native launch witness, never
/// evidence that the requested filesystem or network restriction is enforced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PermissionTier {
    ReadOnly,
    NoNetwork,
    IsolatedWrite,
    NetworkedWrite,
}

impl PermissionTier {
    fn from_json(value: &Json) -> Result<Self, SeatError> {
        let Json::String(value) = value else { return Err(SeatError::Invalid("permissionTier")); };
        match value.to_well_formed_string().as_deref() {
            Some("READ_ONLY") => Ok(Self::ReadOnly),
            Some("NO_NETWORK") => Ok(Self::NoNetwork),
            Some("ISOLATED_WRITE") => Ok(Self::IsolatedWrite),
            Some("NETWORKED_WRITE") => Ok(Self::NetworkedWrite),
            _ => Err(SeatError::Invalid("permissionTier")),
        }
    }
}

/// The current native seat record supplies permission intent; a missing field
/// has no default and cannot be filled from an open request or UI cache.
pub(crate) fn permission_tier(seat: &Seat) -> Result<PermissionTier, SeatError> {
    if seat.state == State::Reclaimed { return Err(SeatError::Denied); }
    let settings = seat.settings_json.as_deref().ok_or(SeatError::Denied)?;
    let Json::Object(fields) = Parser::parse(settings)? else { return Err(SeatError::SchemaDrift); };
    let value = fields.get(&JsonString::from_str("permissionTier")).ok_or(SeatError::Denied)?;
    PermissionTier::from_json(value)
}

/// A native lead admission. No string or wire token constructor is exposed.
/// H must call the constructor only after authenticating the live session and
/// matching its seat, domain and generation to its admission reservation.
#[derive(Clone)]
pub(crate) struct NativeLeadAdmission {
    domain_id: String,
    seat_id: String,
    incarnation: String,
    generation: i64,
    model_call: Option<NativeSeatCall>,
}
impl NativeLeadAdmission {
    pub(crate) fn from_model_call(caller:&NativeSeatCall)->Result<Self,SeatError> {
        if caller.model_proof().is_none() || caller.tool()!=Some("gogoke_seat") {
            return Err(SeatError::Denied);
        }
        Ok(Self {domain_id:caller.domain_id().to_owned(),seat_id:caller.seat_id().to_owned(),
            incarnation:caller.incarnation().to_owned(),generation:caller.generation(),
            model_call:Some(caller.clone())})
    }
    pub(crate) fn model_call(&self)->Option<&NativeSeatCall> {self.model_call.as_ref()}
    pub(crate) fn parent_identity(&self)->(&str,&str,i64) {
        (&self.domain_id,&self.seat_id,self.generation)
    }
    pub(crate) fn parent_incarnation(&self)->&str {&self.incarnation}
    #[cfg(test)]
    pub(crate) fn from_native_runtime_snapshot(seat: &Seat) -> Result<Self, SeatError> {
        if seat.layer != Layer::User || seat.state != State::Busy {
            return Err(SeatError::Denied);
        }
        Ok(Self {
            domain_id: seat.domain_id.clone(),
            seat_id: seat.seat_id.clone(),
            incarnation: seat.incarnation.clone(),
            generation: seat.generation,
            model_call: None,
        })
    }
}

pub(crate) enum NativeOrigin<'a> {
    User(&'a OwnerIssuer),
    Lead(&'a NativeLeadAdmission),
}
impl<'a> NativeOrigin<'a> {
    pub(crate) fn user(issuer: &'a OwnerIssuer) -> Self {
        Self::User(issuer)
    }
    pub(crate) fn lead(admission: &'a NativeLeadAdmission) -> Self {
        Self::Lead(admission)
    }
}

pub(crate) struct CreateSeat<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) seat_id: &'a str,
    pub(crate) template_id: &'a str,
    /// A new seat can remain unbound until the native caller chooses an
    /// instance. Bound creation is retained for existing callers.
    pub(crate) instance_id: Option<&'a str>,
    pub(crate) kind: Kind,
    pub(crate) request_id: &'a str,
    /// Exact ingress bytes, including unknown fields and original whitespace.
    pub(crate) request_bytes: &'a [u8],
}
pub(crate) struct StoreTemplate<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) template_id: &'a str,
    /// Canonical JSON object copied into every seat created from this template.
    pub(crate) settings_json: &'a [u8],
}
pub(crate) struct SeatChange<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) seat_id: &'a str,
    pub(crate) expected_generation: i64,
    pub(crate) expected_revision: i64,
    pub(crate) request_id: &'a str,
    pub(crate) request_bytes: &'a [u8],
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SeatReceipt {
    pub(crate) seat: Seat,
    pub(crate) replayed: bool,
}

fn valid_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'))
        && value.len() <= 128
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}
fn validate(domain: &str, seat: &str, request: &str, raw: &[u8]) -> Result<(), SeatError> {
    if !valid_id(domain) {
        return Err(SeatError::Invalid("domain_id"));
    }
    if !valid_id(seat) {
        return Err(SeatError::Invalid("seat_id"));
    }
    if !valid_id(request) {
        return Err(SeatError::Invalid("request_id"));
    }
    if raw.is_empty() || raw.len() > crate::ipc::MAX_FRAME_BYTES {
        return Err(SeatError::Invalid("request_bytes"));
    }
    Ok(())
}
fn fingerprint(parts: &[&str], raw: &[u8]) -> String {
    let mut bytes = Vec::new();
    for part in parts {
        bytes.extend_from_slice(&(part.len() as u64).to_be_bytes());
        bytes.extend_from_slice(part.as_bytes());
    }
    bytes.extend_from_slice(&(raw.len() as u64).to_be_bytes());
    bytes.extend_from_slice(raw);
    super::digest::content_hash(&bytes)
}
fn transact<T>(
    db: &mut VerifiedDatabaseConnection<'_>,
    f: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T, SeatError>,
) -> Result<T, SeatError> {
    db.execute("BEGIN IMMEDIATE")?;
    match f(db) {
        Ok(value) => {
            db.execute("COMMIT").map_err(SeatError::CommitUnknown)?;
            Ok(value)
        }
        Err(error) => {
            db.execute("ROLLBACK").map_err(SeatError::RollbackUnknown)?;
            Err(error)
        }
    }
}
fn check_current_owner(
    db: &VerifiedDatabaseConnection<'_>,
    issuer: &OwnerIssuer,
) -> Result<(), SeatError> {
    check_owner_in_current_transaction(db, issuer).map_err(|error| match error {
        super::orchestration::OrchestrationError::Atomic(source) => SeatError::Store(source),
        _ => SeatError::Denied,
    })
}
fn schema(db: &VerifiedDatabaseConnection<'_>) -> Result<Vec<(String, String)>, SeatError> {
    let q = Statement::prepare(db.as_ptr(), "SELECT name,sql FROM main.sqlite_schema WHERE lower(name) LIKE 'gogoke_v37_seat%' ORDER BY name")?;
    let mut rows = Vec::new();
    while q.step_row()? {
        rows.push((q.column_text(0)?, q.column_text(1)?));
    }
    Ok(rows)
}
fn reject_shadow_or_effect(db: &VerifiedDatabaseConnection<'_>) -> Result<(), SeatError> {
    for sql in [
        "SELECT 1 FROM temp.sqlite_schema WHERE lower(name) LIKE 'gogoke_v37_seat%' OR lower(tbl_name) LIKE 'gogoke_v37_seat%' LIMIT 1",
        "SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND lower(tbl_name) LIKE 'gogoke_v37_seat%' LIMIT 1",
    ] {
        if Statement::prepare(db.as_ptr(), sql)?.step_row()? { return Err(SeatError::SchemaDrift); }
    }
    Ok(())
}
fn expected_schema() -> Vec<(String, String)> {
    let mut entries = pre_secretary_schema();
    entries.push(("gogoke_v37_seat_secretary".into(),secretary::DESIGNATION.into()));
    entries.extend(secretary_routines::SCHEMA.iter().map(|(name,sql)|(name.to_string(),sql.to_string())));
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}
fn secretary_only_schema() -> Vec<(String, String)> {
    let mut entries = pre_secretary_schema();
    entries.push(("gogoke_v37_seat_secretary".into(),secretary::DESIGNATION.into()));
    entries.sort_by(|left,right|left.0.cmp(&right.0));
    entries
}
fn pre_secretary_schema() -> Vec<(String, String)> {
    let mut entries = e2_schema();
    entries.extend(page_facts::SCHEMA.iter().map(|(name,sql)|(name.to_string(),sql.to_string())));
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}
fn e2_schema() -> Vec<(String, String)> {
    let mut entries = f1_schema();
    entries.push(("gogoke_v37_seat_host_resources".into(),resource::HOST_RESOURCES.into()));
    entries.extend([
        ("gogoke_v37_seat_policy_head".into(),policy::POLICY_HEAD.into()),
        ("gogoke_v37_seat_policy_grants".into(),policy::POLICY_GRANTS.into()),
        ("gogoke_v37_seat_policy_gates".into(),policy::POLICY_GATES.into()),
        ("gogoke_v37_seat_policy_routes".into(),policy::POLICY_ROUTES.into()),
        ("gogoke_v37_seat_policy_escalations".into(),policy::POLICY_ESCALATIONS.into()),
        ("gogoke_v37_seat_policy_events".into(),policy::POLICY_EVENTS.into()),
        ("gogoke_v37_seat_policy_triggers".into(),policy::POLICY_TRIGGERS.into()),
        ("gogoke_v37_seat_cards".into(),continuity::CARDS.into()),
        ("gogoke_v37_seat_takeover_answers".into(),continuity::ANSWERS.into()),
        ("gogoke_v37_seat_continuity_operations".into(),continuity::OPERATIONS.into()),
        ("gogoke_v37_seat_health".into(),continuity::HEALTH.into()),
    ]);
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}
fn f1_schema() -> Vec<(String, String)> {
    let mut entries = previous_schema();
    entries.push(("gogoke_v37_seat_project_caps".into(), PROJECT_CAPS.into()));
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}
fn create_e2_tables(db: &mut VerifiedDatabaseConnection<'_>) -> Result<(), SeatError> {
    db.execute(resource::HOST_RESOURCES)?;
    for sql in [policy::POLICY_HEAD,policy::POLICY_GRANTS,policy::POLICY_GATES,
        policy::POLICY_ROUTES,policy::POLICY_ESCALATIONS,policy::POLICY_EVENTS,
        policy::POLICY_TRIGGERS,
        continuity::CARDS,continuity::ANSWERS,continuity::OPERATIONS,continuity::HEALTH] {
        db.execute(sql)?;
    }
    Ok(())
}
fn previous_schema() -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = vec![
        (
            "gogoke_v37_seat_operation_snapshots".into(),
            OPERATION_SNAPSHOTS.into(),
        ),
        ("gogoke_v37_seat_operations".into(), OPERATIONS.into()),
        ("gogoke_v37_seat_settings".into(), SETTINGS.into()),
        ("gogoke_v37_seat_templates".into(), TEMPLATES.into()),
        ("gogoke_v37_seats".into(), SEATS.into()),
    ];
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}
fn legacy_schema() -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = vec![
        ("gogoke_v37_seat_operations".into(), OPERATIONS.into()),
        ("gogoke_v37_seats".into(), LEGACY_SEATS.into()),
    ];
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}
/// Call after F's instance schema, on the same verified product connection.
pub(crate) fn initialize_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<(), SeatError> {
    reject_shadow_or_effect(db)?;
    let observed = schema(db)?;
    if observed == expected_schema() {
        return Ok(());
    }
    if observed == secretary_only_schema() {
        return transact(db,|db| {
            if schema(db)? != secretary_only_schema() {return Err(SeatError::SchemaDrift);}
            secretary_routines::create_tables(db)?;
            if schema(db)? != expected_schema() {return Err(SeatError::SchemaDrift);}
            Ok(())
        });
    }
    if observed == pre_secretary_schema() {
        return transact(db, |db| {
            if schema(db)? != pre_secretary_schema() {return Err(SeatError::SchemaDrift);}
            db.execute(secretary::DESIGNATION)?;
            secretary_routines::create_tables(db)?;
            if schema(db)? != expected_schema() {return Err(SeatError::SchemaDrift);}
            Ok(())
        });
    }
    if observed == e2_schema() {
        return transact(db, |db| {
            if schema(db)? != e2_schema() {return Err(SeatError::SchemaDrift);}
            page_facts::create_tables(db)?;
            db.execute(secretary::DESIGNATION)?;
            secretary_routines::create_tables(db)?;
            if schema(db)? != expected_schema() {return Err(SeatError::SchemaDrift);}
            Ok(())
        });
    }
    if observed == f1_schema() {
        return migrate_f1_schema(db);
    }
    if observed == legacy_schema() {
        return migrate_legacy_schema(db);
    }
    if observed == previous_schema() {
        return migrate_previous_schema(db);
    }
    if !observed.is_empty() {
        return Err(SeatError::SchemaDrift);
    }
    transact(db, |db| {
        reject_shadow_or_effect(db)?;
        if !schema(db)?.is_empty() {
            return Err(SeatError::SchemaDrift);
        }
        db.execute(SEATS)?;
        db.execute(OPERATIONS)?;
        db.execute(TEMPLATES)?;
        db.execute(SETTINGS)?;
        db.execute(OPERATION_SNAPSHOTS)?;
        db.execute(PROJECT_CAPS)?;
        create_e2_tables(db)?;
        page_facts::create_tables(db)?;
        db.execute(secretary::DESIGNATION)?;
        secretary_routines::create_tables(db)?;
        if schema(db)? != expected_schema() {
            return Err(SeatError::SchemaDrift);
        }
        Ok(())
    })
}

fn migrate_f1_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<(), SeatError> {
    transact(db, |db| {
        reject_shadow_or_effect(db)?;
        if schema(db)? != f1_schema() { return Err(SeatError::SchemaDrift); }
        create_e2_tables(db)?;
        page_facts::create_tables(db)?;
        db.execute(secretary::DESIGNATION)?;
        secretary_routines::create_tables(db)?;
        if schema(db)? != expected_schema() { return Err(SeatError::SchemaDrift); }
        Ok(())
    })
}

fn migrate_previous_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<(), SeatError> {
    transact(db, |db| {
        reject_shadow_or_effect(db)?;
        if schema(db)? != previous_schema() {
            return Err(SeatError::SchemaDrift);
        }
        db.execute(PROJECT_CAPS)?;
        create_e2_tables(db)?;
        page_facts::create_tables(db)?;
        db.execute(secretary::DESIGNATION)?;
        secretary_routines::create_tables(db)?;
        if schema(db)? != expected_schema() {
            return Err(SeatError::SchemaDrift);
        }
        Ok(())
    })
}

fn migrate_legacy_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<(), SeatError> {
    transact(db, |db| {
        if schema(db)? != legacy_schema() {
            return Err(SeatError::SchemaDrift);
        }
        // SQLite cannot make a NOT NULL column nullable in place. Rebuild only
        // this table, preserving every existing row and the self-layer FK.
        db.execute("CREATE TABLE gogoke_v37_seats_v2(domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,incarnation TEXT NOT NULL UNIQUE,layer TEXT NOT NULL CHECK(layer IN ('USER','LEAD')),parent_seat_id TEXT,kind TEXT NOT NULL CHECK(kind IN ('LONG','SHORT')),instance_id TEXT REFERENCES gogoke_v37_instances(instance_id),state TEXT NOT NULL CHECK(state IN ('IDLE','BUSY','RECLAIMED')),generation INTEGER NOT NULL CHECK(generation >= 1),revision INTEGER NOT NULL CHECK(revision >= 1),CHECK((layer='USER' AND parent_seat_id IS NULL) OR (layer='LEAD' AND parent_seat_id IS NOT NULL)),PRIMARY KEY(domain_id,seat_id),FOREIGN KEY(domain_id,parent_seat_id) REFERENCES gogoke_v37_seats_v2(domain_id,seat_id)) STRICT")?;
        db.execute("INSERT INTO gogoke_v37_seats_v2(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) SELECT domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision FROM gogoke_v37_seats")?;
        db.execute("DROP TABLE gogoke_v37_seats")?;
        // CREATE the final table from the canonical SQL instead of renaming
        // the temporary one: SQLite records renamed identifiers with quotes,
        // which would make the exact-schema pin depend on migration history.
        db.execute(SEATS)?;
        db.execute("INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) SELECT domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision FROM gogoke_v37_seats_v2")?;
        db.execute("DROP TABLE gogoke_v37_seats_v2")?;
        db.execute(TEMPLATES)?;
        db.execute(SETTINGS)?;
        db.execute(OPERATION_SNAPSHOTS)?;
        db.execute(PROJECT_CAPS)?;
        create_e2_tables(db)?;
        page_facts::create_tables(db)?;
        db.execute(secretary::DESIGNATION)?;
        secretary_routines::create_tables(db)?;
        if schema(db)? != expected_schema() {
            return Err(SeatError::SchemaDrift);
        }
        Ok(())
    })
}

fn validate_template_settings(raw: &[u8]) -> Result<(), SeatError> {
    if raw.is_empty() || raw.len() > crate::ipc::MAX_FRAME_BYTES {
        return Err(SeatError::Invalid("template_settings"));
    }
    super::atomic::require_canonical_json(raw, "template.settings")?;
    let text = std::str::from_utf8(raw).map_err(|_| SeatError::Invalid("template_settings"))?;
    let parsed=Parser::parse(text)?;
    let Json::Object(fields) = &parsed else {
        return Err(SeatError::Invalid("template_settings"));
    };
    if let Some(value) = fields.get(&JsonString::from_str("permissionTier")) {
        PermissionTier::from_json(value)?;
    }
    continuity::validate_takeover_template(&parsed)?;
    orchestration::validate_template_scope(&parsed)?;
    Ok(())
}

fn seat_template_fields(
    template_id: String,
    settings_json: String,
) -> Result<(Option<String>, Option<String>), SeatError> {
    if template_id.is_empty() != settings_json.is_empty() {
        return Err(SeatError::SchemaDrift);
    }
    if template_id.is_empty() {
        return Ok((None, None));
    }
    if !valid_id(&template_id) {
        return Err(SeatError::SchemaDrift);
    }
    validate_template_settings(settings_json.as_bytes()).map_err(|_| SeatError::SchemaDrift)?;
    Ok((Some(template_id), Some(settings_json)))
}

fn template(
    db: &VerifiedDatabaseConnection<'_>,
    domain: &str,
    template_id: &str,
) -> Result<Option<String>, SeatError> {
    let q = Statement::prepare(
        db.as_ptr(),
        "SELECT settings_json FROM main.gogoke_v37_seat_templates WHERE domain_id=?1 AND template_id=?2",
    )?;
    q.bind_text(1, domain)?;
    q.bind_text(2, template_id)?;
    if !q.step_row()? {
        return Ok(None);
    }
    let settings = q.column_text(0)?;
    if q.step_row()? {
        return Err(SeatError::SchemaDrift);
    }
    validate_template_settings(settings.as_bytes()).map_err(|_| SeatError::SchemaDrift)?;
    Ok(Some(settings))
}

fn read(
    db: &VerifiedDatabaseConnection<'_>,
    domain: &str,
    seat_id: &str,
) -> Result<Option<Seat>, SeatError> {
    let q = Statement::prepare(db.as_ptr(), "SELECT s.incarnation,s.layer,COALESCE(s.parent_seat_id,''),s.kind,COALESCE(s.instance_id,''),s.state,s.generation,s.revision,COALESCE(t.template_id,''),COALESCE(t.settings_json,'') FROM main.gogoke_v37_seats AS s LEFT JOIN main.gogoke_v37_seat_settings AS t ON t.domain_id=s.domain_id AND t.seat_id=s.seat_id WHERE s.domain_id=?1 AND s.seat_id=?2")?;
    q.bind_text(1, domain)?;
    q.bind_text(2, seat_id)?;
    if !q.step_row()? {
        return Ok(None);
    }
    let layer = match q.column_text(1)?.as_str() {
        "USER" => Layer::User,
        "LEAD" => Layer::Lead,
        _ => return Err(SeatError::SchemaDrift),
    };
    let parent = q.column_text(2)?;
    let kind = match q.column_text(3)?.as_str() {
        "LONG" => Kind::Long,
        "SHORT" => Kind::Short,
        _ => return Err(SeatError::SchemaDrift),
    };
    let state = match q.column_text(5)?.as_str() {
        "IDLE" => State::Idle,
        "BUSY" => State::Busy,
        "RECLAIMED" => State::Reclaimed,
        _ => return Err(SeatError::SchemaDrift),
    };
    let generation = q
        .column_text(6)?
        .parse()
        .map_err(|_| SeatError::SchemaDrift)?;
    let revision = q
        .column_text(7)?
        .parse()
        .map_err(|_| SeatError::SchemaDrift)?;
    let (template_id, settings_json) = seat_template_fields(q.column_text(8)?, q.column_text(9)?)?;
    let seat = Seat {
        domain_id: domain.into(),
        seat_id: seat_id.into(),
        incarnation: q.column_text(0)?,
        layer,
        parent_seat_id: if parent.is_empty() {
            None
        } else {
            Some(parent)
        },
        kind,
        instance_id: q.column_text(4)?,
        template_id,
        settings_json,
        state,
        generation,
        revision,
    };
    if q.step_row()? {
        return Err(SeatError::SchemaDrift);
    }
    Ok(Some(seat))
}
pub(crate) fn get(
    db: &VerifiedDatabaseConnection<'_>,
    domain: &str,
    seat_id: &str,
) -> Result<Option<Seat>, SeatError> {
    if !valid_id(domain) || !valid_id(seat_id) {
        return Err(SeatError::Invalid("seat address"));
    }
    read(db, domain, seat_id)
}

/// Owner's project-wide admission limit. The native issuer is checked inside
/// the same write transaction as the update; lead-layer callers have no write
/// capability. No value is installed by schema creation or migration.
pub(crate) fn set_project_parallel_cap(
    db: &mut VerifiedDatabaseConnection<'_>,
    issuer: &OwnerIssuer,
    domain: &str,
    cap: i64,
) -> Result<(), SeatError> {
    if !valid_id(domain) {
        return Err(SeatError::Invalid("domain_id"));
    }
    if cap <= 0 {
        return Err(SeatError::Invalid("project_parallel_cap"));
    }
    transact(db, |db| {
        check_current_owner(db, issuer)?;
        let write = Statement::prepare(
            db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_project_caps(domain_id,parallel_cap) VALUES(?1,?2) ON CONFLICT(domain_id) DO UPDATE SET parallel_cap=excluded.parallel_cap",
        )?;
        write.bind_text(1, domain)?;
        write.bind_i64(2, cap)?;
        write.step_done()?;
        Ok(())
    })
}

/// H reads this required value inside its BEGIN IMMEDIATE admission transaction
/// on the same verified connection. A missing project cap denies admission.
pub(crate) fn read_project_parallel_cap(
    db: &VerifiedDatabaseConnection<'_>,
    domain: &str,
) -> Result<i64, SeatError> {
    if !valid_id(domain) {
        return Err(SeatError::Invalid("domain_id"));
    }
    let query = Statement::prepare(
        db.as_ptr(),
        "SELECT parallel_cap FROM main.gogoke_v37_seat_project_caps WHERE domain_id=?1",
    )?;
    query.bind_text(1, domain)?;
    if !query.step_row()? {
        return Err(SeatError::Denied);
    }
    let cap = query.column_text(0)?.parse::<i64>().map_err(|_| SeatError::SchemaDrift)?;
    if cap <= 0 || query.step_row()? {
        return Err(SeatError::SchemaDrift);
    }
    Ok(cap)
}

/// Store an immutable template definition. Only the owner layer can publish
/// templates; lead seats may copy them but cannot change the source definition.
pub(crate) fn store_template(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: StoreTemplate<'_>,
) -> Result<(), SeatError> {
    let issuer = match origin {
        NativeOrigin::User(issuer) => issuer,
        NativeOrigin::Lead(_) => return Err(SeatError::Denied),
    };
    if !valid_id(input.domain_id) {
        return Err(SeatError::Invalid("domain_id"));
    }
    if !valid_id(input.template_id) {
        return Err(SeatError::Invalid("template_id"));
    }
    validate_template_settings(input.settings_json)?;
    let settings=std::str::from_utf8(input.settings_json)
        .map_err(|_|SeatError::Invalid("template_settings"))?;
    let settings=orchestration::normalized_effort_json(settings)?;
    transact(db, |db| {
        check_current_owner(db, issuer)?;
        let existing = Statement::prepare(
            db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_seat_templates WHERE domain_id=?1 AND template_id=?2",
        )?;
        existing.bind_text(1, input.domain_id)?;
        existing.bind_text(2, input.template_id)?;
        if existing.step_row()? {
            return Err(SeatError::Conflict);
        }
        let insert = Statement::prepare(
            db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_templates(domain_id,template_id,settings_json,revision) VALUES(?1,?2,?3,1)",
        )?;
        insert.bind_text(1, input.domain_id)?;
        insert.bind_text(2, input.template_id)?;
        insert.bind_text(3, &settings)?;
        insert.step_done()?;
        Ok(())
    })
}

fn instance_exists(
    db: &VerifiedDatabaseConnection<'_>,
    instance_id: &str,
) -> Result<bool, SeatError> {
    let q = Statement::prepare(
        db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_instances WHERE instance_id=?1",
    )?;
    q.bind_text(1, instance_id)?;
    Ok(q.step_row()?)
}
fn check_origin(
    db: &VerifiedDatabaseConnection<'_>,
    origin: &NativeOrigin<'_>,
    domain: &str,
    target: Option<&Seat>,
) -> Result<(Layer, Option<String>), SeatError> {
    match origin {
        NativeOrigin::User(issuer) => {
            check_current_owner(db, issuer)?;
            Ok((Layer::User, None))
        }
        NativeOrigin::Lead(admission) => {
            if admission.domain_id != domain {
                return Err(SeatError::Denied);
            }
            let actor = if let Some(caller)=admission.model_call() {
                policy::current_caller(db,caller)?
            } else {
                #[cfg(test)]
                {read(db,domain,&admission.seat_id)?.ok_or(SeatError::Denied)?}
                #[cfg(not(test))]
                {return Err(SeatError::Denied);}
            };
            if actor.layer != Layer::User
                || actor.state != State::Busy
                || actor.incarnation != admission.incarnation
                || actor.generation != admission.generation
            {
                return Err(SeatError::Denied);
            }
            if let Some(target) = target {
                if let Some(caller)=admission.model_call() {
                    orchestration::current_child_dispatch_context(db,caller,target)?;
                }
                if target.layer != Layer::Lead
                    || target.parent_seat_id.as_deref() != Some(&admission.seat_id)
                    || target.seat_id == admission.seat_id
                {
                    return Err(SeatError::Denied);
                }
            }
            Ok((Layer::Lead, Some(admission.seat_id.clone())))
        }
    }
}
fn operation(
    db: &VerifiedDatabaseConnection<'_>,
    domain: &str,
    request: &str,
    fp: &str,
) -> Result<Option<SeatReceipt>, SeatError> {
    let q = Statement::prepare(db.as_ptr(), "SELECT o.fingerprint,o.seat_id,o.incarnation,o.layer,COALESCE(o.parent_seat_id,''),o.kind,o.instance_id,o.state,o.revision,o.generation,COALESCE(s.template_id,''),COALESCE(s.settings_json,'') FROM main.gogoke_v37_seat_operations AS o LEFT JOIN main.gogoke_v37_seat_operation_snapshots AS s ON s.domain_id=o.domain_id AND s.request_id=o.request_id WHERE o.domain_id=?1 AND o.request_id=?2")?;
    q.bind_text(1, domain)?;
    q.bind_text(2, request)?;
    if !q.step_row()? {
        return Ok(None);
    }
    if q.column_text(0)? != fp {
        return Err(SeatError::Conflict);
    }
    let layer = match q.column_text(3)?.as_str() {
        "USER" => Layer::User,
        "LEAD" => Layer::Lead,
        _ => return Err(SeatError::SchemaDrift),
    };
    let parent = q.column_text(4)?;
    let kind = match q.column_text(5)?.as_str() {
        "LONG" => Kind::Long,
        "SHORT" => Kind::Short,
        _ => return Err(SeatError::SchemaDrift),
    };
    let (template_id, settings_json) =
        seat_template_fields(q.column_text(10)?, q.column_text(11)?)?;
    let state = match q.column_text(7)?.as_str() {
        "IDLE" => State::Idle,
        "BUSY" => State::Busy,
        "RECLAIMED" => State::Reclaimed,
        _ => return Err(SeatError::SchemaDrift),
    };
    let seat = Seat {
        domain_id: domain.into(),
        seat_id: q.column_text(1)?,
        incarnation: q.column_text(2)?,
        layer,
        parent_seat_id: if parent.is_empty() {
            None
        } else {
            Some(parent)
        },
        kind,
        instance_id: q.column_text(6)?,
        template_id,
        settings_json,
        state,
        revision: q
            .column_text(8)?
            .parse()
            .map_err(|_| SeatError::SchemaDrift)?,
        generation: q
            .column_text(9)?
            .parse()
            .map_err(|_| SeatError::SchemaDrift)?,
    };
    Ok(Some(SeatReceipt {
        seat,
        replayed: true,
    }))
}
fn create_operation_fingerprint(
    db: &VerifiedDatabaseConnection<'_>, domain: &str, request: &str,
) -> Result<Option<String>, SeatError> {
    let q = Statement::prepare(db.as_ptr(),
        "SELECT fingerprint FROM main.gogoke_v37_seat_operations WHERE domain_id=?1 AND request_id=?2")?;
    q.bind_text(1, domain)?;
    q.bind_text(2, request)?;
    if !q.step_row()? { return Ok(None); }
    let fingerprint = q.column_text(0)?;
    if q.step_row()? { return Err(SeatError::SchemaDrift); }
    Ok(Some(fingerprint))
}
fn authorize_replay(
    db: &VerifiedDatabaseConnection<'_>,
    origin: &NativeOrigin<'_>,
    receipt: &SeatReceipt,
) -> Result<(), SeatError> {
    let current =
        read(db, &receipt.seat.domain_id, &receipt.seat.seat_id)?.ok_or(SeatError::SchemaDrift)?;
    // First recheck the live actor and layer relationship. A historical
    // request fingerprint cannot keep a stopped or revoked lead authorized.
    check_origin(db, origin, &receipt.seat.domain_id, Some(&current))?;
    // Even with a live actor, an old receipt cannot represent a later target
    // generation, reclamation or binding as the current successful result.
    // The exact reclaim receipt itself may replay while that same reclaimed
    // generation remains current; it does not restore dispatch authority.
    if current != receipt.seat {
        return Err(SeatError::Conflict);
    }
    Ok(())
}
fn record_operation(
    db: &VerifiedDatabaseConnection<'_>,
    request: &str,
    fp: &str,
    seat: &Seat,
) -> Result<(), SeatError> {
    let q = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_seat_operations(domain_id,request_id,fingerprint,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,revision,generation) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)")?;
    q.bind_text(1, &seat.domain_id)?;
    q.bind_text(2, request)?;
    q.bind_text(3, fp)?;
    q.bind_text(4, &seat.seat_id)?;
    q.bind_text(5, &seat.incarnation)?;
    q.bind_text(6, seat.layer.sql())?;
    if let Some(parent) = &seat.parent_seat_id {
        q.bind_text(7, parent)?;
    }
    q.bind_text(8, seat.kind.sql())?;
    q.bind_text(9, &seat.instance_id)?;
    q.bind_text(
        10,
        match seat.state {
            State::Idle => "IDLE",
            State::Busy => "BUSY",
            State::Reclaimed => "RECLAIMED",
        },
    )?;
    q.bind_i64(11, seat.revision)?;
    q.bind_i64(12, seat.generation)?;
    q.step_done()?;
    if let (Some(template_id), Some(settings_json)) = (&seat.template_id, &seat.settings_json) {
        let snapshot = Statement::prepare(
            db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_operation_snapshots(domain_id,request_id,template_id,settings_json) VALUES(?1,?2,?3,?4)",
        )?;
        snapshot.bind_text(1, &seat.domain_id)?;
        snapshot.bind_text(2, request)?;
        snapshot.bind_text(3, template_id)?;
        snapshot.bind_text(4, settings_json)?;
        snapshot.step_done()?;
    }
    Ok(())
}

pub(crate) fn create(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: CreateSeat<'_>,
) -> Result<SeatReceipt, SeatError> {
    create_inner(db,origin,input,None)
}

/// Root/H passes only a caller authenticated from its original live turn.
/// A copied seat snapshot alone can still create an E.1 child, but cannot
/// derive a policy grant. The new grant is part of this first-create commit.
pub(crate) fn create_native_child(db:&mut VerifiedDatabaseConnection<'_>,
    caller:&policy::NativeSeatCall,input:CreateSeat<'_>)->Result<SeatReceipt,SeatError> {
    let parent=policy::current_caller(db,caller)?;
    let admission=if caller.model_proof().is_some() {NativeLeadAdmission::from_model_call(caller)?}
        else {
            #[cfg(test)]
            {NativeLeadAdmission::from_native_runtime_snapshot(&parent)?}
            #[cfg(not(test))]
            {return Err(SeatError::Denied);}
        };
    create_inner(db,NativeOrigin::lead(&admission),input,Some(caller))
}

fn create_inner(
    db:&mut VerifiedDatabaseConnection<'_>,origin:NativeOrigin<'_>,input:CreateSeat<'_>,
    caller:Option<&policy::NativeSeatCall>,
)->Result<SeatReceipt,SeatError> {
    validate(
        input.domain_id,
        input.seat_id,
        input.request_id,
        input.request_bytes,
    )?;
    if !valid_id(input.template_id) {
        return Err(SeatError::Invalid("template_id"));
    }
    if let Some(instance_id) = input.instance_id {
        if !valid_id(instance_id) {
            return Err(SeatError::Invalid("instance_id"));
        }
    }
    let (layer_label, parent_label, origin_incarnation, origin_generation) = match &origin {
        NativeOrigin::User(_) => ("USER", "", "", String::new()),
        NativeOrigin::Lead(a) => (
            "LEAD",
            a.seat_id.as_str(),
            a.incarnation.as_str(),
            a.generation.to_string(),
        ),
    };
    // The frozen create wire does not contain kind. Historical USER and LEAD
    // requests may have either kind; their original fingerprints stay valid.
    let fresh_kind = if caller.is_some() { Kind::Short } else { input.kind };
    let create_fingerprint = |kind: Kind| fingerprint(
        &[
            "create",
            input.domain_id,
            input.seat_id,
            input.template_id,
            input.instance_id.unwrap_or(""),
            kind.sql(),
            layer_label,
            parent_label,
            origin_incarnation,
            &origin_generation,
        ],
        input.request_bytes,
    );
    let fp = create_fingerprint(fresh_kind);
    let prior_kind = match fresh_kind { Kind::Long => Kind::Short, Kind::Short => Kind::Long };
    let legacy_fp = create_fingerprint(prior_kind);
    transact(db, |db| {
        // Authenticate before looking up a request ID as well as before the
        // first seat write. This covers both fresh writes and replay/conflict
        // paths in the same write group.
        check_origin(db, &origin, input.domain_id, None)?;
        if let Some(caller)=caller {
            let verified=policy::current_caller(db,caller)?;
            if verified.layer!=Layer::User || verified.domain_id!=input.domain_id ||
                !matches!(&origin,NativeOrigin::Lead(admission) if
                    admission.seat_id==verified.seat_id &&
                    admission.incarnation==verified.incarnation &&
                    admission.generation==verified.generation) {
                return Err(SeatError::Denied);
            }
        }
        // Resolve a persisted request against its original fingerprint before
        // applying the new default to a fresh child. Neither kind is rewritten.
        if let Some(stored_fp) = create_operation_fingerprint(db, input.domain_id, input.request_id)? {
            if stored_fp != fp && stored_fp != legacy_fp {
                return Err(SeatError::Conflict);
            }
            let receipt = operation(db, input.domain_id, input.request_id, &stored_fp)?
                .ok_or(SeatError::SchemaDrift)?;
            authorize_replay(db, &origin, &receipt)?;
            return Ok(receipt);
        }
        let (layer, parent) = check_origin(db, &origin, input.domain_id, None)?;
        if read(db, input.domain_id, input.seat_id)?.is_some() {
            return Err(SeatError::Conflict);
        }
        let settings_json =
            template(db, input.domain_id, input.template_id)?.ok_or(SeatError::Unknown)?;
        let settings_json=orchestration::normalized_effort_json(&settings_json)?;
        if let NativeOrigin::Lead(admission)=&origin {
            let parent=read(db,input.domain_id,&admission.seat_id)?.ok_or(SeatError::Denied)?;
            let instance=input.instance_id.ok_or(SeatError::Denied)?;
            orchestration::child_within_scope(&parent,&settings_json,instance)?;
            orchestration::require_child_capacity(db,&parent,true)?;
        }
        if let Some(instance_id) = input.instance_id {
            if !instance_exists(db, instance_id)? {
                return Err(SeatError::Unknown);
            }
        }
        if let Some(instance_id) = input.instance_id {
            let q = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) VALUES(?1,?2,lower(hex(randomblob(16))),?3,?4,?5,?6,'IDLE',1,1)")?;
            q.bind_text(1, input.domain_id)?;
            q.bind_text(2, input.seat_id)?;
            q.bind_text(3, layer.sql())?;
            if let Some(parent) = &parent {
                q.bind_text(4, parent)?;
            }
            q.bind_text(5, fresh_kind.sql())?;
            q.bind_text(6, instance_id)?;
            q.step_done()?;
        } else {
            let q = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) VALUES(?1,?2,lower(hex(randomblob(16))),?3,?4,?5,NULL,'IDLE',1,1)")?;
            q.bind_text(1, input.domain_id)?;
            q.bind_text(2, input.seat_id)?;
            q.bind_text(3, layer.sql())?;
            if let Some(parent) = &parent {
                q.bind_text(4, parent)?;
            }
            q.bind_text(5, fresh_kind.sql())?;
            q.step_done()?;
        }
        let settings = Statement::prepare(
            db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_settings(domain_id,seat_id,template_id,settings_json) VALUES(?1,?2,?3,?4)",
        )?;
        settings.bind_text(1, input.domain_id)?;
        settings.bind_text(2, input.seat_id)?;
        settings.bind_text(3, input.template_id)?;
        settings.bind_text(4, &settings_json)?;
        settings.step_done()?;
        let seat = read(db, input.domain_id, input.seat_id)?.ok_or(SeatError::SchemaDrift)?;
        if let Some(caller)=caller {
            policy::derive_new_child_dispatch_grant(db,caller,&seat)?;
        }
        record_operation(db, input.request_id, &fp, &seat)?;
        Ok(SeatReceipt {
            seat,
            replayed: false,
        })
    })
}

fn change(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: SeatChange<'_>,
    action: &'static str,
    value: &str,
) -> Result<SeatReceipt, SeatError> {
    validate(
        input.domain_id,
        input.seat_id,
        input.request_id,
        input.request_bytes,
    )?;
    if input.expected_generation < 1 {
        return Err(SeatError::Invalid("generation"));
    }
    if input.expected_revision < 1 {
        return Err(SeatError::Invalid("revision"));
    }
    let (origin_id, origin_incarnation, origin_generation) = match &origin {
        NativeOrigin::User(_) => ("", "", String::new()),
        NativeOrigin::Lead(a) => (
            a.seat_id.as_str(),
            a.incarnation.as_str(),
            a.generation.to_string(),
        ),
    };
    let fp = fingerprint(
        &[
            action,
            input.domain_id,
            input.seat_id,
            &input.expected_generation.to_string(),
            &input.expected_revision.to_string(),
            value,
            origin_id,
            origin_incarnation,
            &origin_generation,
        ],
        input.request_bytes,
    );
    transact(db, |db| {
        // Keep request replay lookup behind the current issuer check. A
        // request-ID collision must not become an issuer oracle.
        check_origin(db, &origin, input.domain_id, None)?;
        if let Some(receipt) = operation(db, input.domain_id, input.request_id, &fp)? {
            authorize_replay(db, &origin, &receipt)?;
            return Ok(receipt);
        }
        let before = read(db, input.domain_id, input.seat_id)?.ok_or(SeatError::Unknown)?;
        check_origin(db, &origin, input.domain_id, Some(&before))?;
        if secretary::is_designated_current(db,&before)? && action!="reclaim" {
            return Err(SeatError::Denied);
        }
        if action=="reclaim" && page_facts::is_designated_lead(db,&before)? {
            return Err(SeatError::Denied);
        }
        if before.state == State::Reclaimed {
            return Err(SeatError::Denied);
        }
        if before.generation != input.expected_generation {
            return Err(SeatError::Conflict);
        }
        if before.revision != input.expected_revision {
            return Err(SeatError::Conflict);
        }
        if before.state == State::Busy {
            return Err(SeatError::Busy);
        }
        page_facts::ensure_mutable(db,&before)?;
        if let NativeOrigin::Lead(admission)=&origin {
            if matches!(action,"bind-instance"|"change-instance") {
                let parent=read(db,input.domain_id,&admission.seat_id)?.ok_or(SeatError::Denied)?;
                orchestration::child_within_scope(&parent,
                    before.settings_json.as_deref().ok_or(SeatError::Denied)?,value)?;
            }
        }
        let next_generation = before
            .generation
            .checked_add(1)
            .ok_or(SeatError::Conflict)?;
        let next_revision = before.revision.checked_add(1).ok_or(SeatError::Conflict)?;
        let sql = match action {
            "bind-instance" => {
                if !before.instance_id.is_empty() {
                    return Err(SeatError::Conflict);
                }
                if !instance_exists(db, value)? { return Err(SeatError::Unknown); }
                "UPDATE main.gogoke_v37_seats SET instance_id=?1,generation=?2,revision=?3 WHERE domain_id=?4 AND seat_id=?5 AND generation=?6 AND revision=?7 AND state='IDLE'"
            }
            "change-instance" => {
                if before.instance_id.is_empty() || before.instance_id == value {
                    return Err(SeatError::Conflict);
                }
                if !instance_exists(db, value)? { return Err(SeatError::Unknown); }
                "UPDATE main.gogoke_v37_seats SET instance_id=?1,generation=?2,revision=?3 WHERE domain_id=?4 AND seat_id=?5 AND generation=?6 AND revision=?7 AND state='IDLE'"
            }
            "promote" => {
                if before.kind != Kind::Short { return Err(SeatError::Conflict); }
                "UPDATE main.gogoke_v37_seats SET kind=?1,generation=?2,revision=?3 WHERE domain_id=?4 AND seat_id=?5 AND generation=?6 AND revision=?7 AND state='IDLE'"
            }
            "reclaim" => "UPDATE main.gogoke_v37_seats SET state=?1,generation=?2,revision=?3 WHERE domain_id=?4 AND seat_id=?5 AND generation=?6 AND revision=?7 AND state='IDLE'",
            _ => return Err(SeatError::Invalid("action")),
        };
        let q = Statement::prepare(db.as_ptr(), sql)?;
        q.bind_text(1, value)?;
        q.bind_i64(2, next_generation)?;
        q.bind_i64(3, next_revision)?;
        q.bind_text(4, input.domain_id)?;
        q.bind_text(5, input.seat_id)?;
        q.bind_i64(6, before.generation)?;
        q.bind_i64(7, before.revision)?;
        q.step_done()?;
        let seat = read(db, input.domain_id, input.seat_id)?.ok_or(SeatError::SchemaDrift)?;
        if seat.generation != next_generation || seat.revision != next_revision {
            return Err(SeatError::Conflict);
        }
        if matches!(action,"change-instance"|"reclaim") {
            let clear=Statement::prepare(db.as_ptr(),
                "DELETE FROM main.gogoke_v37_seat_takeover_answers WHERE domain_id=?1 AND seat_id=?2")?;
            clear.bind_text(1,input.domain_id)?;clear.bind_text(2,input.seat_id)?;
            clear.step_done()?;
        }
        record_operation(db, input.request_id, &fp, &seat)?;
        Ok(SeatReceipt {
            seat,
            replayed: false,
        })
    })
}
pub(crate) fn bind_instance(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: SeatChange<'_>,
    instance_id: &str,
) -> Result<SeatReceipt, SeatError> {
    if !valid_id(instance_id) {
        return Err(SeatError::Invalid("instance_id"));
    }
    change(db, origin, input, "bind-instance", instance_id)
}
pub(crate) fn change_instance(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: SeatChange<'_>,
    instance_id: &str,
) -> Result<SeatReceipt, SeatError> {
    if !valid_id(instance_id) {
        return Err(SeatError::Invalid("instance_id"));
    }
    change(db, origin, input, "change-instance", instance_id)
}

/// Commit a complete runnable configuration with its instance binding. This
/// also repairs a previously partial configuration on the same instance.
pub(crate) fn configure_instance(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: SeatChange<'_>,
    instance_id: &str,
    model: &str,
    effort: &str,
    permission_json: &str,
) -> Result<SeatReceipt, SeatError> {
    validate(input.domain_id, input.seat_id, input.request_id, input.request_bytes)?;
    if input.expected_generation < 1 || input.expected_revision < 1 {
        return Err(SeatError::Invalid("seat revision"));
    }
    if !valid_id(instance_id) { return Err(SeatError::Invalid("instance_id")); }
    if model.is_empty() || model.len() > 256 || effort.is_empty() || effort.len() > 256 {
        return Err(SeatError::Invalid("model or effort"));
    }
    super::atomic::require_canonical_json(permission_json.as_bytes(), "seat.permissionTier")?;
    let permission = Parser::parse(permission_json)?;
    PermissionTier::from_json(&permission)?;
    let (origin_id, origin_incarnation, origin_generation) = match &origin {
        NativeOrigin::User(_) => ("", "", String::new()),
        NativeOrigin::Lead(admission) => (
            admission.seat_id.as_str(), admission.incarnation.as_str(),
            admission.generation.to_string(),
        ),
    };
    let fp = fingerprint(&[
        "configure-instance", input.domain_id, input.seat_id,
        &input.expected_generation.to_string(), &input.expected_revision.to_string(),
        instance_id, model, effort, permission_json,
        origin_id, origin_incarnation, &origin_generation,
    ], input.request_bytes);
    transact(db, |db| {
        check_origin(db, &origin, input.domain_id, None)?;
        if let Some(receipt) = operation(db, input.domain_id, input.request_id, &fp)? {
            authorize_replay(db, &origin, &receipt)?;
            return Ok(receipt);
        }
        let before = read(db, input.domain_id, input.seat_id)?.ok_or(SeatError::Unknown)?;
        check_origin(db, &origin, input.domain_id, Some(&before))?;
        if before.state == State::Reclaimed { return Err(SeatError::Denied); }
        if before.generation != input.expected_generation || before.revision != input.expected_revision {
            return Err(SeatError::Conflict);
        }
        if before.state == State::Busy { return Err(SeatError::Busy); }
        page_facts::ensure_mutable(db, &before)?;
        // Only the Owner-designated global secretary may be configured from
        // an unbound seat in one transaction. This is configuration data, not
        // H admission or global ledger authority.
        if before.instance_id.is_empty() && !secretary::is_designated_current(db,&before)? {
            return Err(SeatError::Conflict);
        }
        if !instance_exists(db, instance_id)? { return Err(SeatError::Unknown); }
        if secretary::is_designated_current(db,&before)? {
            secretary::require_enabled_instance(db,instance_id)?;
        }
        let evidence = super::instance::read_instance_evidence(db, instance_id)
            .map_err(|error| SeatError::InstanceManagement(format!("{error:?}")))?
            .ok_or(SeatError::Denied)?;
        let models_json = evidence.available_models_json.ok_or(SeatError::Denied)?;
        if evidence.models_source.is_none() || evidence.models_observed_at.is_none() {
            return Err(SeatError::Denied);
        }
        let Json::Array(models) = Parser::parse(&models_json)? else { return Err(SeatError::Denied); };
        if models.is_empty() || !models.iter().all(|item| matches!(item, Json::String(_)))
            || !models.iter().any(|item| matches!(item, Json::String(value)
                if value.to_well_formed_string().as_deref() == Some(model))) {
            return Err(SeatError::Denied);
        }
        let settings_json = before.settings_json.as_deref().ok_or(SeatError::SchemaDrift)?;
        let Json::Object(mut settings) = Parser::parse(settings_json)? else {
            return Err(SeatError::SchemaDrift);
        };
        settings.remove(&JsonString::from_str("reasoningEffort"));
        settings.insert(JsonString::from_str("model"), Json::String(JsonString::from_str(model)));
        settings.insert(JsonString::from_str("effort"), Json::String(JsonString::from_str(effort)));
        settings.insert(JsonString::from_str("permissionTier"), permission);
        let updated_settings = orchestration::normalized_effort_json(&Json::Object(settings).canonical())?;
        validate_template_settings(updated_settings.as_bytes())?;
        if let NativeOrigin::Lead(admission) = &origin {
            let parent = read(db, input.domain_id, &admission.seat_id)?.ok_or(SeatError::Denied)?;
            orchestration::child_within_scope(&parent, &updated_settings, instance_id)?;
        }
        let next_generation = before.generation.checked_add(1).ok_or(SeatError::Conflict)?;
        let next_revision = before.revision.checked_add(1).ok_or(SeatError::Conflict)?;
        let update_settings = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_settings SET settings_json=?1 WHERE domain_id=?2 AND seat_id=?3")?;
        update_settings.bind_text(1, &updated_settings)?;
        update_settings.bind_text(2, input.domain_id)?;
        update_settings.bind_text(3, input.seat_id)?;
        update_settings.step_done()?;
        let update_seat = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seats SET instance_id=?1,generation=?2,revision=?3 WHERE domain_id=?4 AND seat_id=?5 AND generation=?6 AND revision=?7 AND state='IDLE'")?;
        update_seat.bind_text(1, instance_id)?;
        update_seat.bind_i64(2, next_generation)?;
        update_seat.bind_i64(3, next_revision)?;
        update_seat.bind_text(4, input.domain_id)?;
        update_seat.bind_text(5, input.seat_id)?;
        update_seat.bind_i64(6, before.generation)?;
        update_seat.bind_i64(7, before.revision)?;
        update_seat.step_done()?;
        let seat = read(db, input.domain_id, input.seat_id)?.ok_or(SeatError::SchemaDrift)?;
        if seat.instance_id != instance_id || seat.settings_json.as_deref() != Some(updated_settings.as_str())
            || seat.generation != next_generation || seat.revision != next_revision {
            return Err(SeatError::Conflict);
        }
        let clear = Statement::prepare(db.as_ptr(),
            "DELETE FROM main.gogoke_v37_seat_takeover_answers WHERE domain_id=?1 AND seat_id=?2")?;
        clear.bind_text(1, input.domain_id)?;
        clear.bind_text(2, input.seat_id)?;
        clear.step_done()?;
        record_operation(db, input.request_id, &fp, &seat)?;
        Ok(SeatReceipt { seat, replayed: false })
    })
}
pub(crate) fn promote(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: SeatChange<'_>,
) -> Result<SeatReceipt, SeatError> {
    change(db, origin, input, "promote", "LONG")
}
pub(crate) fn reclaim(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: SeatChange<'_>,
) -> Result<SeatReceipt, SeatError> {
    change(db, origin, input, "reclaim", "RECLAIMED")
}

/// Change one copied template setting on the seat. The source template is
/// immutable; the seat revision and copied settings advance in one transaction.
pub(crate) fn tune(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: SeatChange<'_>,
    setting: &str,
    value_json: &str,
) -> Result<SeatReceipt, SeatError> {
    validate(input.domain_id, input.seat_id, input.request_id, input.request_bytes)?;
    if input.expected_generation < 1 || input.expected_revision < 1 {
        return Err(SeatError::Invalid("seat revision"));
    }
    if !valid_id(setting) { return Err(SeatError::Invalid("setting")); }
    if value_json.is_empty() || value_json.len() > crate::ipc::MAX_FRAME_BYTES {
        return Err(SeatError::Invalid("value"));
    }
    super::atomic::require_canonical_json(value_json.as_bytes(), "seat.value")?;
    let value = Parser::parse(value_json)?;
    let (origin_id, origin_incarnation, origin_generation) = match &origin {
        NativeOrigin::User(_) => ("", "", String::new()),
        NativeOrigin::Lead(admission) => (
            admission.seat_id.as_str(), admission.incarnation.as_str(),
            admission.generation.to_string(),
        ),
    };
    let fp = fingerprint(&[
        "tune", input.domain_id, input.seat_id,
        &input.expected_generation.to_string(), &input.expected_revision.to_string(),
        setting, value_json, origin_id, origin_incarnation, &origin_generation,
    ], input.request_bytes);
    transact(db, |db| {
        check_origin(db, &origin, input.domain_id, None)?;
        if let Some(receipt) = operation(db, input.domain_id, input.request_id, &fp)? {
            authorize_replay(db, &origin, &receipt)?;
            return Ok(receipt);
        }
        let before = read(db, input.domain_id, input.seat_id)?.ok_or(SeatError::Unknown)?;
        check_origin(db, &origin, input.domain_id, Some(&before))?;
        if secretary::is_designated_current(db,&before)? {return Err(SeatError::Denied);}
        if before.state == State::Reclaimed { return Err(SeatError::Denied); }
        if before.generation != input.expected_generation || before.revision != input.expected_revision {
            return Err(SeatError::Conflict);
        }
        if before.state == State::Busy { return Err(SeatError::Busy); }
        page_facts::ensure_mutable(db,&before)?;
        if setting=="orchestrationScope" {
            if !matches!(&origin,NativeOrigin::User(_)) || before.layer!=Layer::User {
                return Err(SeatError::Denied);
            }
            orchestration::validate_new_scope(&value)?;
        }
        let Some(settings_json) = &before.settings_json else { return Err(SeatError::SchemaDrift); };
        let Json::Object(mut settings) = Parser::parse(settings_json)? else {
            return Err(SeatError::SchemaDrift);
        };
        let reset_takeover_answers=setting=="takeoverQuestions" &&
            settings.get(&JsonString::from_str("takeoverQuestions")).map(Json::canonical)
                != Some(value.canonical());
        if setting=="reasoningEffort" {
            settings.remove(&JsonString::from_str("effort"));
        } else if setting=="effort" {
            settings.remove(&JsonString::from_str("reasoningEffort"));
        }
        settings.insert(JsonString::from_str(setting), value);
        let updated_settings = orchestration::normalized_effort_json(&Json::Object(settings).canonical())?;
        validate_template_settings(updated_settings.as_bytes())?;
        if let NativeOrigin::Lead(admission)=&origin {
            let parent=read(db,input.domain_id,&admission.seat_id)?.ok_or(SeatError::Denied)?;
            orchestration::child_within_scope(&parent,&updated_settings,&before.instance_id)?;
        }
        let next_generation = before.generation.checked_add(1).ok_or(SeatError::Conflict)?;
        let next_revision = before.revision.checked_add(1).ok_or(SeatError::Conflict)?;
        let update_settings = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_settings SET settings_json=?1 WHERE domain_id=?2 AND seat_id=?3")?;
        update_settings.bind_text(1, &updated_settings)?;
        update_settings.bind_text(2, input.domain_id)?;
        update_settings.bind_text(3, input.seat_id)?;
        update_settings.step_done()?;
        let update_seat = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seats SET generation=?1,revision=?2 WHERE domain_id=?3 AND seat_id=?4 AND generation=?5 AND revision=?6 AND state='IDLE'")?;
        update_seat.bind_i64(1, next_generation)?;
        update_seat.bind_i64(2, next_revision)?;
        update_seat.bind_text(3, input.domain_id)?;
        update_seat.bind_text(4, input.seat_id)?;
        update_seat.bind_i64(5, before.generation)?;
        update_seat.bind_i64(6, before.revision)?;
        update_seat.step_done()?;
        let seat = read(db, input.domain_id, input.seat_id)?.ok_or(SeatError::SchemaDrift)?;
        if seat.generation != next_generation || seat.revision != next_revision
            || seat.settings_json.as_deref() != Some(updated_settings.as_str()) {
            return Err(SeatError::Conflict);
        }
        if reset_takeover_answers {
            let clear=Statement::prepare(db.as_ptr(),
                "DELETE FROM main.gogoke_v37_seat_takeover_answers WHERE domain_id=?1 AND seat_id=?2")?;
            clear.bind_text(1,input.domain_id)?;clear.bind_text(2,input.seat_id)?;
            clear.step_done()?;
        }
        record_operation(db, input.request_id, &fp, &seat)?;
        Ok(SeatReceipt { seat, replayed: false })
    })
}

/// H must mark BUSY before launching a seat, and mark IDLE only after its
/// durable stop fact. BUSY survives restart and blocks a second binding until
/// H reconciles the stop; an absent process is not itself a stop fact.
pub(crate) fn set_dispatch_state(
    db: &mut VerifiedDatabaseConnection<'_>,
    seat: &Seat,
    busy: bool,
) -> Result<Seat, SeatError> {
    transact(db, |db| set_dispatch_state_in_transaction(db, seat, busy))
}

/// H calls this only inside its admission or proven-stop transaction on this
/// same connection, so seat state cannot commit independently of that fact.
pub(crate) fn set_dispatch_state_in_transaction(
    db: &mut VerifiedDatabaseConnection<'_>,
    seat: &Seat,
    busy: bool,
) -> Result<Seat, SeatError> {
        let current = read(db, &seat.domain_id, &seat.seat_id)?.ok_or(SeatError::Unknown)?;
        if current.incarnation != seat.incarnation
            || current.generation != seat.generation
            || current.revision != seat.revision
        {
            return Err(SeatError::Conflict);
        }
        if current.state != if busy { State::Idle } else { State::Busy } {
            return Err(SeatError::Conflict);
        }
        // Admission and stop both revoke tokens from the preceding runtime.
        // Otherwise an old lead token could become valid on a later BUSY turn.
        let generation = current
            .generation
            .checked_add(1)
            .ok_or(SeatError::Conflict)?;
        let revision = current.revision.checked_add(1).ok_or(SeatError::Conflict)?;
        let q = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_v37_seats SET state=?1,generation=?2,revision=?3 WHERE domain_id=?4 AND seat_id=?5 AND generation=?6 AND revision=?7")?;
        q.bind_text(1, if busy { "BUSY" } else { "IDLE" })?;
        q.bind_i64(2, generation)?;
        q.bind_i64(3, revision)?;
        q.bind_text(4, &seat.domain_id)?;
        q.bind_text(5, &seat.seat_id)?;
        q.bind_i64(6, seat.generation)?;
        q.bind_i64(7, seat.revision)?;
        q.step_done()?;
        let updated = read(db, &seat.domain_id, &seat.seat_id)?.ok_or(SeatError::SchemaDrift)?;
        if updated.revision != revision || updated.generation != generation {
            return Err(SeatError::Conflict);
        }
        Ok(updated)
}
