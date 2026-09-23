//! Active process coordination only. Rows never constitute accepted Git facts.
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;
use super::transaction;
use crate::process::PreparedCustody;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS gogoke_coordination_process_custody (operation_id TEXT PRIMARY KEY, ticket TEXT NOT NULL UNIQUE, custodian_nonce TEXT NOT NULL, pid TEXT NOT NULL, creation_time_100ns TEXT NOT NULL, image_path TEXT NOT NULL, binary_digest_sha256 TEXT NOT NULL, profile_id TEXT NOT NULL, domain_id TEXT NOT NULL, generation TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('PREPARED','ACTIVE','STOPPED','UNKNOWN')), stop_proof_hash TEXT) STRICT";

pub(crate) fn initialize(connection: &mut VerifiedDatabaseConnection<'_>) -> Result<(), OrchestrationError> {
    connection.execute(SCHEMA).map_err(|error| OrchestrationError::Atomic(error.into()))?;
    // A new host never inherits proof that a previous Job or writer survived.
    connection.execute("UPDATE gogoke_coordination_process_custody SET state='UNKNOWN' WHERE state IN ('PREPARED','ACTIVE')")
        .map_err(|error| OrchestrationError::Atomic(error.into()))
}

pub(crate) fn record_prepared(
    connection: &mut VerifiedDatabaseConnection<'_>,
    operation_id: &str,
    prepared: &PreparedCustody,
) -> Result<(), OrchestrationError> {
    if operation_id.is_empty() || operation_id.len() > 128 || !operation_id.bytes().all(|byte|
        byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_') {
        return Err(OrchestrationError::Invalid("process operation identity"));
    }
    let pid = prepared.identity.pid.to_string();
    let created = prepared.identity.creation_time_100ns.to_string();
    let image = prepared.identity.image_path.to_string_lossy();
    transaction::run(connection, |tx| {
        tx.write("INSERT INTO gogoke_coordination_process_custody (operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES (?,?,?,?,?,?,?,?,?,?,'PREPARED')",
            &[operation_id, prepared.ticket.opaque(), &prepared.custodian_nonce, &pid, &created,
              &image, &prepared.binding.binary_digest_sha256, &prepared.binding.profile_id,
              &prepared.binding.domain_id, &prepared.binding.generation])
    })
}

pub(crate) fn mark_active(
    connection: &mut VerifiedDatabaseConnection<'_>,
    operation_id: &str,
    prepared: &PreparedCustody,
) -> Result<(), OrchestrationError> {
    transaction::run(connection, |tx| {
        tx.write("UPDATE gogoke_coordination_process_custody SET state='ACTIVE' WHERE operation_id=? AND ticket=? AND custodian_nonce=? AND state='PREPARED'",
            &[operation_id, prepared.ticket.opaque(), &prepared.custodian_nonce])?;
        let rows = tx.query("SELECT changes()", &[], 1)?;
        if rows.len() != 1 || rows[0][0] != "1" {
            return Err(OrchestrationError::OperationConflict);
        }
        Ok(())
    })
}

pub(crate) fn mark_unknown(
    connection: &mut VerifiedDatabaseConnection<'_>,
    operation_id: &str,
    prepared: &PreparedCustody,
) -> Result<(), OrchestrationError> {
    transaction::run(connection, |tx| {
        tx.write("UPDATE gogoke_coordination_process_custody SET state='UNKNOWN' WHERE operation_id=? AND ticket=? AND custodian_nonce=? AND state IN ('PREPARED','ACTIVE')",
            &[operation_id, prepared.ticket.opaque(), &prepared.custodian_nonce])?;
        Ok(())
    })
}
