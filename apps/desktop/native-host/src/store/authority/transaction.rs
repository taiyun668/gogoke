//! No authority decision may escape an uncommitted SQLite write group.
use super::super::action::{reserve_action_in_transaction, ActionReservation, ReserveDisposition};
use super::super::atomic::{apply_domain_record_in_transaction, DomainRecordInput, DomainRecordReceipt, Statement};
use super::super::context::{apply_context_version_in_transaction, ContextCommand, ContextReceipt};
use super::super::context_state::{self, StateSnapshot};
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;

pub(super) type Result<T> = std::result::Result<T, OrchestrationError>;

pub(super) struct Transaction<'db, 'root> {
    connection: &'db mut VerifiedDatabaseConnection<'root>,
    active: bool,
}

impl Transaction<'_, '_> {
    pub(super) fn root_identity(&self) -> String {
        self.connection.root_identity().opaque()
    }

    pub(super) fn validate_product_core_schema(&mut self) -> Result<()> {
        if !self.active {
            return Err(OrchestrationError::AccessDenied);
        }
        super::super::atomic::validate_product_core_schema(self.connection)
            .map_err(OrchestrationError::Atomic)
    }

    /// The same connection and outer transaction own grants, Context, and receipts.
    /// No callback, await, second connection, or nested transaction is admitted.
    pub(super) fn apply_context(&mut self, command: ContextCommand) -> Result<ContextReceipt> {
        if !self.active {
            return Err(OrchestrationError::AccessDenied);
        }
        apply_context_version_in_transaction(self.connection, command)
    }

    /// Reuse the existing ActionStore group without nesting BEGIN or COMMIT.
    /// This returns storage disposition, never permission or dispatch evidence.
    pub(super) fn reserve_action(&mut self, input: ActionReservation) -> Result<ReserveDisposition> {
        if !self.active {
            return Err(OrchestrationError::AccessDenied);
        }
        reserve_action_in_transaction(self.connection, input)
    }

    /// Reuse the canonical object/event/receipt group on this same connection.
    /// Current actor/policy/reference checks belong to the typed authority caller;
    /// a storage replay here is not a permission token or an external acceptance.
    pub(super) fn apply_domain_record(&mut self, input: DomainRecordInput) -> Result<DomainRecordReceipt> {
        if !self.active {
            return Err(OrchestrationError::AccessDenied);
        }
        apply_domain_record_in_transaction(self.connection, input).map_err(OrchestrationError::Atomic)
    }

    pub(super) fn context_state(&mut self, domain_id: &str, version_ref: &str) -> Result<StateSnapshot> {
        if !self.active {
            return Err(OrchestrationError::AccessDenied);
        }
        context_state::current(self.connection, domain_id, version_ref)
    }

    pub(super) fn query(&mut self, sql: &str, args: &[&str], columns: i32) -> Result<Vec<Vec<String>>> {
        let statement = Statement::prepare(self.connection.as_ptr(), sql)?;
        for (index, value) in args.iter().enumerate() {
            statement.bind_text((index + 1) as i32, value)?;
        }
        let mut rows = Vec::new();
        while statement.step_row()? {
            if rows.len() >= 64 {
                return Err(OrchestrationError::Invalid("authority query bound"));
            }
            let mut row = Vec::new();
            for column in 0..columns {
                row.push(statement.column_text(column)?);
            }
            rows.push(row);
        }
        Ok(rows)
    }

    pub(super) fn write(&mut self, sql: &str, args: &[&str]) -> Result<()> {
        let statement = Statement::prepare(self.connection.as_ptr(), sql)?;
        for (index, value) in args.iter().enumerate() {
            statement.bind_text((index + 1) as i32, value)?;
        }
        statement.step_done()?;
        Ok(())
    }
}

impl Drop for Transaction<'_, '_> {
    fn drop(&mut self) {
        if self.active {
            // Also runs on early return/unwind. Never report a failed commit as success.
            let _ = self.connection.execute("ROLLBACK");
        }
    }
}

pub(super) fn run<T>(
    connection: &mut VerifiedDatabaseConnection<'_>,
    operation: impl FnOnce(&mut Transaction<'_, '_>) -> Result<T>,
) -> Result<T> {
    connection.execute("BEGIN IMMEDIATE")
        .map_err(|e| OrchestrationError::Atomic(e.into()))?;
    let mut tx = Transaction { connection, active: true };
    match operation(&mut tx) {
        Ok(value) => {
            if tx.connection.execute("COMMIT").is_err() {
                return Err(OrchestrationError::CommitUnknown);
            }
            tx.active = false;
            Ok(value)
        }
        Err(error) => {
            if tx.connection.execute("ROLLBACK").is_err() {
                return Err(OrchestrationError::CommitUnknown);
            }
            tx.active = false;
            Err(error)
        }
    }
}
