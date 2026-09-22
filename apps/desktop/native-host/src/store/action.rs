use super::atomic::Statement;
use super::digest::content_hash;
use super::orchestration::OrchestrationError;
use super::same_open::VerifiedDatabaseConnection;

#[derive(Clone, Debug)]
pub struct ActionReservation {
    pub operation_id: String,
    pub semantic_digest: String,
    pub reservation_id: String,
    pub binding_id: String,
    pub session_id: String,
    pub execution_id: String,
    pub runtime_instance_id: String,
    pub profile_id: String,
    pub auth_revision: String,
    pub generation: String,
    pub lane: String,
    pub action_kind: String,
    pub payload_hex: String,
    pub commitment: ActionCommitment,
}

#[derive(Clone, Debug)]
/// Exact expected Policy package tuple. This closes reservation/begin replay
/// identity, but does not make the TypeScript PolicyAuthorityPort transactional
/// with SQLite; a future durable ceiling primitive must replace that seam.
pub struct ActionCommitment {
    pub package_digest: String,
    pub parent_grant_ref: String,
    pub parent_grant_revision: String,
    pub parent_ceiling_digest: String,
    pub child_ceiling_digest: String,
    pub source_principal_id: String,
    pub source_project_id: String,
    pub source_domain_id: String,
    pub source_role: String,
    pub target_principal_id: String,
    pub target_project_id: String,
    pub target_domain_id: String,
    pub target_role: String,
    pub source_session_id: String,
    pub source_execution_id: String,
    pub source_generation: String,
    pub child_session_id: String,
    pub child_execution_id: String,
    pub child_generation: String,
    pub route: String,
    pub policy_action: String,
    pub sink: String,
    pub material_set_digest: String,
    pub instruction_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReserveDisposition {
    Reserved,
    Replay { state: String },
    Conflict { existing_digest: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BeginDisposition {
    Granted { send_authority: String },
    Replay { state: String },
    Conflict,
}

const ACTION_SCHEMA: &str = "CREATE TABLE gogoke_action_reservations (
              operation_id TEXT PRIMARY KEY,
              semantic_digest TEXT NOT NULL CHECK(length(semantic_digest)=71),
              reservation_id TEXT NOT NULL UNIQUE,
              binding_id TEXT NOT NULL, session_id TEXT NOT NULL, execution_id TEXT,
              runtime_instance_id TEXT NOT NULL, profile_id TEXT NOT NULL,
              auth_revision TEXT NOT NULL, generation TEXT NOT NULL,
              lane TEXT NOT NULL CHECK(lane IN ('control','work')),
              action_kind TEXT NOT NULL CHECK(action_kind IN ('queue','steer','interrupt','close')),
              payload_hex TEXT NOT NULL, commitment_record TEXT, send_authority TEXT,
              state TEXT NOT NULL CHECK(state IN ('reserved','dispatching','legacy-unknown','not-sent','dispatched','rejected','outcome-unknown','completed')),
              outcome_kind TEXT, outcome_detail TEXT, receipt_ref TEXT
            ) STRICT";
const LEGACY_SCHEMA: &str = "CREATE TABLE gogoke_action_reservations (
              operation_id TEXT PRIMARY KEY,
              semantic_digest TEXT NOT NULL CHECK(length(semantic_digest)=71),
              reservation_id TEXT NOT NULL UNIQUE,
              binding_id TEXT NOT NULL, session_id TEXT NOT NULL,
              runtime_instance_id TEXT NOT NULL, profile_id TEXT NOT NULL,
              auth_revision TEXT NOT NULL, generation TEXT NOT NULL,
              lane TEXT NOT NULL CHECK(lane IN ('control','work')),
              action_kind TEXT NOT NULL CHECK(action_kind IN ('queue','steer','interrupt','close')),
              payload_hex TEXT NOT NULL,
              state TEXT NOT NULL CHECK(state IN ('reserved','not-sent','dispatched','rejected','outcome-unknown','completed')),
              outcome_kind TEXT, outcome_detail TEXT, receipt_ref TEXT
            ) STRICT";

const LEGACY_COLUMNS: [&str; 16] = [
    "operation_id","semantic_digest","reservation_id","binding_id","session_id",
    "runtime_instance_id","profile_id","auth_revision","generation","lane","action_kind",
    "payload_hex","state","outcome_kind","outcome_detail","receipt_ref",
];
const ACTION_COLUMNS: [&str; 19] = [
    "operation_id","semantic_digest","reservation_id","binding_id","session_id","execution_id",
    "runtime_instance_id","profile_id","auth_revision","generation","lane","action_kind",
    "payload_hex","commitment_record","send_authority","state","outcome_kind","outcome_detail","receipt_ref",
];

fn action_columns(connection: &VerifiedDatabaseConnection<'_>) -> Result<Vec<String>, OrchestrationError> {
    let statement = Statement::prepare(connection.as_ptr(), "PRAGMA table_info('gogoke_action_reservations')")?;
    let mut columns = Vec::new();
    while statement.step_row()? {
        columns.push(statement.column_text(1)?);
    }
    Ok(columns)
}

fn action_trigger_exists(connection: &VerifiedDatabaseConnection<'_>, schema: &str) -> Result<bool, OrchestrationError> {
    let sql = format!("SELECT name FROM {schema} WHERE type='trigger' AND tbl_name='gogoke_action_reservations' LIMIT 1");
    let statement = Statement::prepare(connection.as_ptr(), &sql)?;
    Ok(statement.step_row()?)
}

fn action_schema_exact(connection: &VerifiedDatabaseConnection<'_>) -> Result<(), OrchestrationError> {
    let statement=Statement::prepare(connection.as_ptr(),"SELECT type,sql FROM sqlite_schema WHERE name='gogoke_action_reservations'")?;
    if !statement.step_row()? || statement.column_text(0)? != "table" || statement.column_text(1)? != ACTION_SCHEMA || statement.step_row()? {
        return Err(OrchestrationError::AccessDenied);
    }
    Ok(())
}

fn action_schema_sql(connection:&VerifiedDatabaseConnection<'_>)->Result<String,OrchestrationError>{
    let statement=Statement::prepare(connection.as_ptr(),"SELECT sql FROM sqlite_schema WHERE type='table' AND name='gogoke_action_reservations'")?;
    if !statement.step_row()?{return Err(OrchestrationError::AccessDenied);}let sql=statement.column_text(0)?;
    if statement.step_row()?{return Err(OrchestrationError::AccessDenied);}Ok(sql)
}

pub fn apply_action_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<(), OrchestrationError> {
    if action_trigger_exists(connection, "sqlite_schema")?
        || action_trigger_exists(connection, "sqlite_temp_schema")?
    {
        return Err(OrchestrationError::AccessDenied);
    }
    let columns = action_columns(connection)?;
    if columns.is_empty() {
        connection.execute(ACTION_SCHEMA).map_err(|error| OrchestrationError::Atomic(error.into()))?;
        return action_schema_exact(connection);
    }
    if columns == ACTION_COLUMNS {
        return action_schema_exact(connection);
    }
    if columns != LEGACY_COLUMNS {
        return Err(OrchestrationError::AccessDenied);
    }
    if action_schema_sql(connection)? != LEGACY_SCHEMA {
        return Err(OrchestrationError::AccessDenied);
    }
    connection.execute("BEGIN IMMEDIATE").map_err(|error| OrchestrationError::Atomic(error.into()))?;
    let migrated = (|| {
        connection.execute("ALTER TABLE gogoke_action_reservations RENAME TO gogoke_action_reservations_legacy")?;
        connection.execute(ACTION_SCHEMA)?;
        connection.execute("INSERT INTO gogoke_action_reservations(operation_id,semantic_digest,reservation_id,binding_id,session_id,runtime_instance_id,profile_id,auth_revision,generation,lane,action_kind,payload_hex,state,outcome_kind,outcome_detail,receipt_ref) SELECT operation_id,semantic_digest,reservation_id,binding_id,session_id,runtime_instance_id,profile_id,auth_revision,generation,lane,action_kind,payload_hex,CASE WHEN state='reserved' THEN 'legacy-unknown' ELSE state END,outcome_kind,outcome_detail,receipt_ref FROM gogoke_action_reservations_legacy")?;
        connection.execute("DROP TABLE gogoke_action_reservations_legacy")?;
        Ok::<(), crate::store::same_open::SameOpenError>(())
    })();
    match migrated {
        Ok(()) => {
            connection.execute("COMMIT").map_err(|error| OrchestrationError::Atomic(error.into()))?;
            action_schema_exact(connection)
        },
        Err(error) => {
            let _ = connection.execute("ROLLBACK");
            Err(OrchestrationError::Atomic(error.into()))
        }
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 300
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

fn valid_u64(value: &str) -> bool {
    value == "0"
        || (!value.starts_with('0')
            && value.bytes().all(|byte| byte.is_ascii_digit())
            && value.parse::<u64>().is_ok())
}

fn valid_operation_id(value: &str) -> bool {
    value.len() == 36
        && value.starts_with("opr_")
        && value[4..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_hex(value: &str) -> bool {
    value.len() <= 8 * 1024 * 1024
        && value.len() % 2 == 0
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn commitment_values(value: &ActionCommitment) -> [&str; 24] {
    [
        &value.package_digest,&value.parent_grant_ref,&value.parent_grant_revision,
        &value.parent_ceiling_digest,&value.child_ceiling_digest,&value.source_principal_id,
        &value.source_project_id,&value.source_domain_id,&value.source_role,&value.target_principal_id,
        &value.target_project_id,&value.target_domain_id,&value.target_role,&value.source_session_id,
        &value.source_execution_id,&value.source_generation,&value.child_session_id,
        &value.child_execution_id,&value.child_generation,&value.route,&value.policy_action,&value.sink,
        &value.material_set_digest,&value.instruction_digest,
    ]
}

pub(crate) fn commitment_record(value: &ActionCommitment) -> String {
    let mut out = String::from("gogoke.action-commitment.v1|24|");
    for field in commitment_values(value) {
        out.push_str(&field.len().to_string());
        out.push(':');
        out.push_str(field);
    }
    out
}

fn validate_commitment(value: &ActionCommitment) -> Result<(), OrchestrationError> {
    for digest in [
        &value.package_digest,&value.parent_ceiling_digest,&value.child_ceiling_digest,
        &value.material_set_digest,&value.instruction_digest,
    ] {
        if !valid_digest(digest) {
            return Err(OrchestrationError::Invalid("action commitment digest"));
        }
    }
    for revision in [&value.parent_grant_revision,&value.source_generation,&value.child_generation] {
        if !valid_u64(revision) {
            return Err(OrchestrationError::Invalid("action commitment revision"));
        }
    }
    for identity in [
        &value.parent_grant_ref,&value.source_principal_id,&value.source_project_id,
        &value.source_domain_id,&value.source_role,&value.target_principal_id,&value.target_project_id,
        &value.target_domain_id,&value.target_role,&value.source_session_id,&value.source_execution_id,
        &value.child_session_id,&value.child_execution_id,&value.route,&value.policy_action,&value.sink,
    ] {
        if !valid_id(identity) {
            return Err(OrchestrationError::Invalid("action commitment identity"));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn test_commitment(session: &str, execution: &str, generation: &str) -> ActionCommitment {
    let digest = format!("sha256:{}", "a".repeat(64));
    ActionCommitment {
        package_digest:digest.clone(),parent_grant_ref:"grant-one".into(),parent_grant_revision:"1".into(),
        parent_ceiling_digest:digest.clone(),child_ceiling_digest:digest.clone(),
        source_principal_id:"principal-source".into(),source_project_id:"project-one".into(),source_domain_id:"domain-source".into(),source_role:"controller".into(),
        target_principal_id:"principal-target".into(),target_project_id:"project-one".into(),target_domain_id:"domain-target".into(),target_role:"worker".into(),
        source_session_id:"source-session".into(),source_execution_id:"source-execution".into(),source_generation:"1".into(),
        child_session_id:session.into(),child_execution_id:execution.into(),child_generation:generation.into(),
        route:"controller-worker".into(),policy_action:"delegate".into(),sink:"task-package".into(),
        material_set_digest:digest.clone(),instruction_digest:digest,
    }
}

fn validate(input: &ActionReservation) -> Result<(), OrchestrationError> {
    if !valid_operation_id(&input.operation_id) {
        return Err(OrchestrationError::Invalid("action operation id"));
    }
    for value in [
        &input.reservation_id,
        &input.binding_id,
        &input.session_id,
        &input.execution_id,
        &input.runtime_instance_id,
        &input.profile_id,
    ] {
        if !valid_id(value) {
            return Err(OrchestrationError::Invalid("action identity"));
        }
    }
    if !valid_digest(&input.semantic_digest)
        || !valid_u64(&input.auth_revision)
        || !valid_u64(&input.generation)
        || !matches!(input.lane.as_str(), "control" | "work")
        || !matches!(
            input.action_kind.as_str(),
            "queue" | "steer" | "interrupt" | "close"
        )
        || !valid_hex(&input.payload_hex)
    {
        return Err(OrchestrationError::Invalid("action reservation"));
    }
    validate_commitment(&input.commitment)?;
    if input.session_id != input.commitment.child_session_id
        || input.execution_id != input.commitment.child_execution_id
        || input.generation != input.commitment.child_generation
    {
        return Err(OrchestrationError::Invalid("action child binding"));
    }
    Ok(())
}

pub fn reserve_action(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: ActionReservation,
) -> Result<ReserveDisposition, OrchestrationError> {
    validate(&input)?;
    apply_action_schema(connection)?;
    connection
        .execute("BEGIN IMMEDIATE")
        .map_err(|error| OrchestrationError::Atomic(error.into()))?;
    let result = reserve_action_in_transaction(connection, input);
    match result {
        Ok(value) => {
            connection
                .execute("COMMIT")
                .map_err(|error| OrchestrationError::Atomic(error.into()))?;
            Ok(value)
        }
        Err(error) => {
            let _ = connection.execute("ROLLBACK");
            Err(error)
        }
    }
}

/// Storage composition on the same verified connection, not a grant or dispatch.
/// The owning Product Authority must recheck current task/binding/permission and
/// capacity before using this group. It owns COMMIT/ROLLBACK and must handle a
/// conflicting tuple without committing an unrelated Decision or resource lease.
/// This primitive never starts/commits a transaction or performs external I/O.
pub(super) fn reserve_action_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: ActionReservation,
) -> Result<ReserveDisposition, OrchestrationError> {
    validate(&input)?;
    unsafe extern "C" {
        fn sqlite3_get_autocommit(database: *mut std::ffi::c_void) -> std::ffi::c_int;
    }
    // SAFETY: the caller exclusively owns this live verified SQLite connection.
    if unsafe { sqlite3_get_autocommit(connection.as_ptr()) } != 0 {
        return Err(OrchestrationError::Invalid("action reservation requires owning transaction"));
    }
    (|| {
        let existing = Statement::prepare(
            connection.as_ptr(),
            "SELECT semantic_digest,reservation_id,binding_id,session_id,execution_id,runtime_instance_id,
                    profile_id,auth_revision,generation,lane,action_kind,payload_hex,commitment_record,state
             FROM gogoke_action_reservations WHERE operation_id=?",
        )?;
        existing.bind_text(1, &input.operation_id)?;
        if existing.step_row()? {
            let digest = existing.column_text(0)?;
            let state = existing.column_text(13)?;
            if state == "legacy-unknown" {
                return Ok(ReserveDisposition::Conflict { existing_digest: digest });
            }
            let encoded_commitment = commitment_record(&input.commitment);
            let exact_replay = digest == input.semantic_digest
                && existing.column_text(1)? == input.reservation_id
                && existing.column_text(2)? == input.binding_id
                && existing.column_text(3)? == input.session_id
                && existing.column_text(4)? == input.execution_id
                && existing.column_text(5)? == input.runtime_instance_id
                && existing.column_text(6)? == input.profile_id
                && existing.column_text(7)? == input.auth_revision
                && existing.column_text(8)? == input.generation
                && existing.column_text(9)? == input.lane
                && existing.column_text(10)? == input.action_kind
                && existing.column_text(11)? == input.payload_hex
                && existing.column_text(12)? == encoded_commitment;
            return if exact_replay {
                Ok(ReserveDisposition::Replay {
                    state,
                })
            } else {
                Ok(ReserveDisposition::Conflict {
                    existing_digest: digest,
                })
            };
        }
        let statement = Statement::prepare(
            connection.as_ptr(),
            "INSERT INTO gogoke_action_reservations (
              operation_id,semantic_digest,reservation_id,binding_id,session_id,execution_id,
              runtime_instance_id,profile_id,auth_revision,generation,lane,action_kind,payload_hex,commitment_record,state
            ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?, 'reserved')",
        )?;
        let encoded_commitment = commitment_record(&input.commitment);
        for (index, value) in [
            input.operation_id.as_str(),
            input.semantic_digest.as_str(),
            input.reservation_id.as_str(),
            input.binding_id.as_str(),
            input.session_id.as_str(),
            input.execution_id.as_str(),
            input.runtime_instance_id.as_str(),
            input.profile_id.as_str(),
            input.auth_revision.as_str(),
            input.generation.as_str(),
            input.lane.as_str(),
            input.action_kind.as_str(),
            input.payload_hex.as_str(),
            encoded_commitment.as_str(),
        ]
        .iter()
        .enumerate()
        {
            statement.bind_text((index + 1) as i32, value)?;
        }
        statement.step_done()?;
        Ok(ReserveDisposition::Reserved)
    })()
}

pub fn begin_action_commitment(
    connection: &mut VerifiedDatabaseConnection<'_>,
    reservation_id: &str,
    input: &ActionReservation,
) -> Result<BeginDisposition, OrchestrationError> {
    validate(input)?;
    if !valid_id(reservation_id) || reservation_id != input.reservation_id {
        return Err(OrchestrationError::Invalid("action begin identity"));
    }
    apply_action_schema(connection)?;
    connection.execute("BEGIN IMMEDIATE")
        .map_err(|error| OrchestrationError::Atomic(error.into()))?;
    let outcome = (|| {
        let current = Statement::prepare(
            connection.as_ptr(),
            "SELECT semantic_digest,reservation_id,binding_id,session_id,execution_id,runtime_instance_id,
                    profile_id,auth_revision,generation,lane,action_kind,payload_hex,commitment_record,state
             FROM gogoke_action_reservations WHERE operation_id=?",
        )?;
        current.bind_text(1, &input.operation_id)?;
        if !current.step_row()? {
            return Err(OrchestrationError::Invalid("action begin missing reservation"));
        }
        let state = current.column_text(13)?;
        if state == "legacy-unknown" {
            drop(current);
            return Ok(BeginDisposition::Replay { state });
        }
        let encoded_commitment = commitment_record(&input.commitment);
        let exact = current.column_text(0)? == input.semantic_digest
            && current.column_text(1)? == reservation_id
            && current.column_text(2)? == input.binding_id
            && current.column_text(3)? == input.session_id
            && current.column_text(4)? == input.execution_id
            && current.column_text(5)? == input.runtime_instance_id
            && current.column_text(6)? == input.profile_id
            && current.column_text(7)? == input.auth_revision
            && current.column_text(8)? == input.generation
            && current.column_text(9)? == input.lane
            && current.column_text(10)? == input.action_kind
            && current.column_text(11)? == input.payload_hex
            && current.column_text(12)? == encoded_commitment;
        drop(current);
        if !exact {
            return Ok(BeginDisposition::Conflict);
        }
        if state != "reserved" {
            return Ok(BeginDisposition::Replay { state });
        }
        let authority_hash = content_hash(
            format!("{}|{}|{}|{}", reservation_id, input.operation_id, input.semantic_digest, encoded_commitment).as_bytes(),
        );
        let send_authority = format!("send_{}", &authority_hash[7..]);
        let update = Statement::prepare(
            connection.as_ptr(),
            "UPDATE gogoke_action_reservations SET state='dispatching',send_authority=? WHERE operation_id=? AND reservation_id=? AND semantic_digest=? AND commitment_record=? AND state='reserved'",
        )?;
        update.bind_text(1, &send_authority)?;
        update.bind_text(2, &input.operation_id)?;
        update.bind_text(3, reservation_id)?;
        update.bind_text(4, &input.semantic_digest)?;
        update.bind_text(5, &encoded_commitment)?;
        update.step_done()?;
        let verify = Statement::prepare(
            connection.as_ptr(),
            "SELECT state,send_authority FROM gogoke_action_reservations WHERE operation_id=? AND reservation_id=?",
        )?;
        verify.bind_text(1, &input.operation_id)?;
        verify.bind_text(2, reservation_id)?;
        if !verify.step_row()? || verify.column_text(0)? != "dispatching" || verify.column_text(1)? != send_authority {
            return Err(OrchestrationError::OperationConflict);
        }
        Ok(BeginDisposition::Granted { send_authority })
    })();
    match outcome {
        Ok(result) => {
            connection.execute("COMMIT").map_err(|error| OrchestrationError::Atomic(error.into()))?;
            Ok(result)
        }
        Err(error) => {
            let _ = connection.execute("ROLLBACK");
            Err(error)
        }
    }
}

pub fn record_action_outcome(
    connection: &mut VerifiedDatabaseConnection<'_>,
    reservation_id: &str,
    operation_id: &str,
    semantic_digest: &str,
    state: &str,
    detail: &str,
    receipt_ref: &str,
) -> Result<(), OrchestrationError> {
    if !valid_id(reservation_id)
        || !valid_operation_id(operation_id)
        || !valid_digest(semantic_digest)
        || !matches!(
            state,
            "not-sent" | "dispatched" | "rejected" | "outcome-unknown" | "completed"
        )
        || detail.len() > 16_384
        || receipt_ref.len() > 512
    {
        return Err(OrchestrationError::Invalid("action outcome"));
    }
    apply_action_schema(connection)?;
    connection
        .execute("BEGIN IMMEDIATE")
        .map_err(|error| OrchestrationError::Atomic(error.into()))?;
    let result = (|| {
        let current = Statement::prepare(
            connection.as_ptr(),
            "SELECT state,outcome_kind,outcome_detail,receipt_ref
             FROM gogoke_action_reservations
             WHERE reservation_id=? AND operation_id=? AND semantic_digest=?",
        )?;
        current.bind_text(1, reservation_id)?;
        current.bind_text(2, operation_id)?;
        current.bind_text(3, semantic_digest)?;
        if !current.step_row()? {
            return Err(OrchestrationError::Invalid("action outcome identity"));
        }
        let existing_state = current.column_text(0)?;
        if existing_state != "reserved" && existing_state != "dispatching" {
            if existing_state == "legacy-unknown" {
                return Err(OrchestrationError::OperationConflict);
            }
            let existing_kind = current.column_text(1)?;
            let existing_detail = current.column_text(2)?;
            let existing_receipt = current.column_text(3)?;
            return if existing_state == state
                && existing_kind == state
                && existing_detail == detail
                && existing_receipt == receipt_ref
            {
                Ok(())
            } else {
                Err(OrchestrationError::OperationConflict)
            };
        }
        drop(current);
        let statement = Statement::prepare(
            connection.as_ptr(),
            "UPDATE gogoke_action_reservations
             SET state=?,outcome_kind=?,outcome_detail=?,receipt_ref=?
             WHERE reservation_id=? AND operation_id=? AND semantic_digest=? AND state IN ('reserved','dispatching')",
        )?;
        for (index, value) in [
            state,
            state,
            detail,
            receipt_ref,
            reservation_id,
            operation_id,
            semantic_digest,
        ]
        .iter()
        .enumerate()
        {
            statement.bind_text((index + 1) as i32, value)?;
        }
        statement.step_done()?;
        Ok(())
    })();
    match result {
        Ok(()) => connection
            .execute("COMMIT")
            .map_err(|error| OrchestrationError::Atomic(error.into())),
        Err(error) => {
            let _ = connection.execute("ROLLBACK");
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn reservation(digest: char) -> ActionReservation {
        ActionReservation {
            operation_id: "opr_11111111111111111111111111111111".into(),
            semantic_digest: format!("sha256:{}", digest.to_string().repeat(64)),
            reservation_id: "reservation-1".into(),
            binding_id: "binding-1".into(),
            session_id: "session-1".into(),
            execution_id: "execution-1".into(),
            runtime_instance_id: "runtime-1".into(),
            profile_id: "profile-1".into(),
            auth_revision: "7".into(),
            generation: "11".into(),
            lane: "work".into(),
            action_kind: "queue".into(),
            payload_hex: "7b7d".into(),
            commitment: test_commitment("session-1", "execution-1", "11"),
        }
    }

    #[test]
    fn reserve_replay_conflict_and_outcome_are_durable() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root_path = std::env::temp_dir().join(format!("gogoke-action-{nonce}"));
        std::fs::create_dir(&root_path).unwrap();
        let root = RootLock::acquire(&root_path).unwrap();
        let database = root_path.join("state.sqlite");
        let mut connection = create_new(&root, &database).unwrap();
        apply_action_schema(&mut connection).unwrap();
        assert_eq!(
            reserve_action(&mut connection, reservation('a')).unwrap(),
            ReserveDisposition::Reserved
        );
        assert_eq!(
            reserve_action(&mut connection, reservation('a')).unwrap(),
            ReserveDisposition::Replay {
                state: "reserved".into()
            }
        );
        let mut changed_identity = reservation('a');
        changed_identity.reservation_id = "reservation-2".into();
        changed_identity.session_id = "session-2".into();
        changed_identity.commitment.child_session_id = "session-2".into();
        assert!(matches!(
            reserve_action(&mut connection, changed_identity).unwrap(),
            ReserveDisposition::Conflict { .. }
        ));
        assert!(matches!(
            reserve_action(&mut connection, reservation('b')).unwrap(),
            ReserveDisposition::Conflict { .. }
        ));
        record_action_outcome(
            &mut connection,
            "reservation-1",
            "opr_11111111111111111111111111111111",
            &format!("sha256:{}", "a".repeat(64)),
            "outcome-unknown",
            "EOF",
            "",
        )
        .unwrap();
        record_action_outcome(
            &mut connection,
            "reservation-1",
            "opr_11111111111111111111111111111111",
            &format!("sha256:{}", "a".repeat(64)),
            "outcome-unknown",
            "EOF",
            "",
        )
        .unwrap();
        assert!(matches!(
            record_action_outcome(
                &mut connection,
                "reservation-1",
                "opr_11111111111111111111111111111111",
                &format!("sha256:{}", "a".repeat(64)),
                "outcome-unknown",
                "different",
                "receipt-B",
            ),
            Err(OrchestrationError::OperationConflict)
        ));
        assert_eq!(
            reserve_action(&mut connection, reservation('a')).unwrap(),
            ReserveDisposition::Replay {
                state: "outcome-unknown".into()
            }
        );
        let mut invalid = reservation('c');
        invalid.operation_id = "x".into();
        assert!(matches!(
            reserve_action(&mut connection, invalid),
            Err(OrchestrationError::Invalid("action operation id"))
        ));
        connection.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(database).ok();
        std::fs::remove_dir(root_path).ok();
    }

    #[test]
    fn begin_is_the_single_send_authority_cas_and_completion_remains_the_only_terminal_path() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root_path = std::env::temp_dir().join(format!("gogoke-action-begin-{nonce}"));
        std::fs::create_dir(&root_path).unwrap();
        let root = RootLock::acquire(&root_path).unwrap();
        let database = root_path.join("state.sqlite");
        let mut connection = create_new(&root, &database).unwrap();
        apply_action_schema(&mut connection).unwrap();
        let input = reservation('a');
        assert_eq!(reserve_action(&mut connection,input.clone()).unwrap(),ReserveDisposition::Reserved);
        let first=begin_action_commitment(&mut connection,&input.reservation_id,&input).unwrap();
        let BeginDisposition::Granted{send_authority}=first else{panic!("first begin must grant");};
        assert!(send_authority.starts_with("send_")&&send_authority.len()==69);
        assert_eq!(begin_action_commitment(&mut connection,&input.reservation_id,&input).unwrap(),BeginDisposition::Replay{state:"dispatching".into()});
        for mutate in [
            |value:&mut ActionReservation|value.commitment.parent_grant_revision="2".into(),
            |value:&mut ActionReservation|value.commitment.instruction_digest=format!("sha256:{}","b".repeat(64)),
            |value:&mut ActionReservation|value.commitment.material_set_digest=format!("sha256:{}","c".repeat(64)),
            |value:&mut ActionReservation|value.commitment.package_digest=format!("sha256:{}","d".repeat(64)),
        ]{
            let mut conflict=input.clone();mutate(&mut conflict);
            assert_eq!(begin_action_commitment(&mut connection,&input.reservation_id,&conflict).unwrap(),BeginDisposition::Conflict);
        }
        record_action_outcome(&mut connection,&input.reservation_id,&input.operation_id,&input.semantic_digest,"completed","native-complete","receipt-one").unwrap();
        record_action_outcome(&mut connection,&input.reservation_id,&input.operation_id,&input.semantic_digest,"completed","native-complete","receipt-one").unwrap();
        assert_eq!(begin_action_commitment(&mut connection,&input.reservation_id,&input).unwrap(),BeginDisposition::Replay{state:"completed".into()});
        connection.close_checked().unwrap();drop(root);std::fs::remove_file(database).ok();std::fs::remove_dir(root_path).ok();
    }

    #[test]
    fn committed_begin_survives_reopen_without_granting_send_again() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root_path = std::env::temp_dir().join(format!("gogoke-action-crash-{nonce}"));
        std::fs::create_dir(&root_path).unwrap();let root=RootLock::acquire(&root_path).unwrap();let database=root_path.join("state.sqlite");
        let input=reservation('a');let mut connection=create_new(&root,&database).unwrap();apply_action_schema(&mut connection).unwrap();
        reserve_action(&mut connection,input.clone()).unwrap();assert!(matches!(begin_action_commitment(&mut connection,&input.reservation_id,&input).unwrap(),BeginDisposition::Granted{..}));
        connection.close_checked().unwrap();
        let mut reopened=super::super::same_open::open_existing(&root,&database).unwrap();apply_action_schema(&mut reopened).unwrap();
        assert_eq!(reserve_action(&mut reopened,input.clone()).unwrap(),ReserveDisposition::Replay{state:"dispatching".into()});
        assert_eq!(begin_action_commitment(&mut reopened,&input.reservation_id,&input).unwrap(),BeginDisposition::Replay{state:"dispatching".into()});
        reopened.close_checked().unwrap();drop(root);std::fs::remove_file(database).ok();std::fs::remove_dir(root_path).ok();
    }

    #[test]
    fn legacy_reserved_rows_migrate_to_unknown_and_schema_triggers_fail_closed() {
        let _guard = route_b_test_guard();let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root_path=std::env::temp_dir().join(format!("gogoke-action-legacy-{nonce}"));std::fs::create_dir(&root_path).unwrap();
        let root=RootLock::acquire(&root_path).unwrap();let database=root_path.join("state.sqlite");let mut connection=create_new(&root,&database).unwrap();
        connection.execute(LEGACY_SCHEMA).unwrap();
        let input=reservation('a');let insert=Statement::prepare(connection.as_ptr(),"INSERT INTO gogoke_action_reservations(operation_id,semantic_digest,reservation_id,binding_id,session_id,runtime_instance_id,profile_id,auth_revision,generation,lane,action_kind,payload_hex,state) VALUES(?,?,?,?,?,?,?,?,?,?,?,?, 'reserved')").unwrap();
        for (index,value) in [&input.operation_id,&input.semantic_digest,&input.reservation_id,&input.binding_id,&input.session_id,&input.runtime_instance_id,&input.profile_id,&input.auth_revision,&input.generation,&input.lane,&input.action_kind,&input.payload_hex].iter().enumerate(){insert.bind_text((index+1)as i32,value).unwrap();}insert.step_done().unwrap();drop(insert);
        apply_action_schema(&mut connection).unwrap();
        assert_eq!(reserve_action(&mut connection,input.clone()).unwrap(),ReserveDisposition::Conflict{existing_digest:input.semantic_digest.clone()});
        connection.execute("CREATE TEMP TRIGGER action_temp_guard BEFORE UPDATE ON gogoke_action_reservations BEGIN SELECT 1; END").unwrap();
        assert!(apply_action_schema(&mut connection).is_err());assert!(begin_action_commitment(&mut connection,&input.reservation_id,&input).is_err());
        connection.close_checked().unwrap();drop(root);std::fs::remove_file(database).ok();std::fs::remove_dir(root_path).ok();
    }

    #[test]
    fn same_columns_with_weakened_action_state_check_are_not_schema_equivalent() {
        let _guard=route_b_test_guard();let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root_path=std::env::temp_dir().join(format!("gogoke-action-schema-{nonce}"));std::fs::create_dir(&root_path).unwrap();
        let root=RootLock::acquire(&root_path).unwrap();let database=root_path.join("state.sqlite");let mut connection=create_new(&root,&database).unwrap();
        connection.execute(&ACTION_SCHEMA.replace("'legacy-unknown',","")).unwrap();
        assert!(matches!(apply_action_schema(&mut connection),Err(OrchestrationError::AccessDenied)));
        connection.close_checked().unwrap();drop(root);std::fs::remove_file(database).ok();std::fs::remove_dir(root_path).ok();
    }
}
