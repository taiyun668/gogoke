//! A.1 ledger storage on the product's existing verified SQLite connection.
//! Legacy event bodies stay in orchestration_events. This module only owns the
//! shared cursor index and recovery check; caller binding and K-LEDGER ingress
//! still require the native H issuer and L0 product routing.

use super::atomic::{exec, AtomicError, Statement};
use super::same_open::VerifiedDatabaseConnection;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LedgerPosition {
    pub(crate) epoch: String,
    pub(crate) cursor: u64,
}

fn scalar(connection: &VerifiedDatabaseConnection<'_>, sql: &str) -> Result<String, AtomicError> {
    let statement = Statement::prepare(connection.as_ptr(), sql)?;
    if !statement.step_row()? {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 ledger scalar row missing".into(),
        ));
    }
    let value = statement.column_text(0)?;
    if statement.step_row()? {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 ledger scalar returned multiple rows".into(),
        ));
    }
    Ok(value)
}

/// The schema script owns one BEGIN IMMEDIATE/COMMIT. On any error the caller
/// must discard the connection; it must never serve it after uncertain COMMIT.
pub(crate) fn initialize_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<LedgerPosition, AtomicError> {
    exec(connection, include_str!("schema.sql"))?;
    recover(connection)
}

/// Recover the durable epoch/cursor and verify every old event still resolves
/// through exactly the original source ID. No second history is consulted.
pub(crate) fn recover(
    connection: &VerifiedDatabaseConnection<'_>,
) -> Result<LedgerPosition, AtomicError> {
    let missing = scalar(connection,
        "SELECT COUNT(*) FROM orchestration_events AS e
         LEFT JOIN v37_ledger_index AS i
           ON i.source_event_id = e.event_id AND i.source_kind = 'legacy'
         WHERE i.cursor IS NULL")?;
    let orphaned = scalar(connection,
        "SELECT COUNT(*) FROM v37_ledger_index AS i
         LEFT JOIN orchestration_events AS e ON e.event_id = i.source_event_id
         WHERE i.source_kind = 'legacy' AND e.event_id IS NULL")?;
    if missing != "0" || orphaned != "0" {
        return Err(AtomicError::DurabilityContractFailed(format!(
            "A.1 legacy index divergence: missing={missing}, orphaned={orphaned}"
        )));
    }
    let epoch = scalar(connection,
        "SELECT epoch FROM v37_ledger_meta WHERE singleton = 1")?;
    if epoch.is_empty() {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 ledger epoch missing".into(),
        ));
    }
    let cursor = scalar(connection,
        "SELECT COALESCE(MAX(cursor), 0) FROM v37_ledger_index")?
        .parse::<u64>()
        .map_err(|error| AtomicError::DurabilityContractFailed(format!(
            "A.1 invalid ledger cursor: {error}"
        )))?;
    Ok(LedgerPosition { epoch, cursor })
}
