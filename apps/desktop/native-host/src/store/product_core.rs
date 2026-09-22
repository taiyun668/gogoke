//! Initialize the existing canonical record tables inside the same product DB.
//! Other table families keep their own admission. This is not a grant, a generic
//! schema repair API, or a replacement for standalone admit_core_schema.
use super::{assert_durability, core_schema_statements, exec, AtomicError, Statement,
    ADMITTED_TABLES};
use super::super::same_open::VerifiedDatabaseConnection;

type Result<T> = std::result::Result<T, AtomicError>;

struct ExpectedSchema {
    kind: &'static str,
    name: String,
    sql: String,
}

// Derive names and the stored SQL from the SAME seven DDL statements used by the
// original core initializer. Do not maintain a second copy of the table schema.
fn expected_schema() -> Result<Vec<ExpectedSchema>> {
    core_schema_statements().into_iter().map(|sql| {
        let (kind, prefix, tail) = if let Some(tail) = sql.strip_prefix("CREATE TABLE IF NOT EXISTS ") {
            ("table", "CREATE TABLE ", tail)
        } else if let Some(tail) = sql.strip_prefix("CREATE INDEX IF NOT EXISTS ") {
            ("index", "CREATE INDEX ", tail)
        } else {
            return Err(AtomicError::InvalidRecord("unknown core DDL"));
        };
        let name = tail.split_whitespace().next()
            .ok_or(AtomicError::InvalidRecord("missing core DDL name"))?;
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err(AtomicError::InvalidRecord("invalid core DDL name"));
        }
        Ok(ExpectedSchema { kind, name: name.to_owned(), sql: format!("{prefix}{tail}") })
    }).collect()
}

fn reject<T>(detail: &'static str) -> Result<T> {
    Err(AtomicError::InvalidRecord(detail))
}

fn reject_shadow_or_effects(connection: &VerifiedDatabaseConnection<'_>, expected: &[ExpectedSchema]) -> Result<()> {
    // temp objects can shadow main names and temp triggers can attach to main
    // tables. Inspect both catalogs before any schema or row mutation.
    for item in expected {
        let statement = Statement::prepare(connection.as_ptr(),
            "SELECT name FROM temp.sqlite_schema WHERE name=? COLLATE NOCASE OR tbl_name=? COLLATE NOCASE LIMIT 1")?;
        statement.bind_text(1, &item.name)?;
        statement.bind_text(2, &item.name)?;
        if statement.step_row()? { return reject("temporary core schema object"); }
    }
    for table in ADMITTED_TABLES {
        let statement = Statement::prepare(connection.as_ptr(),
            "SELECT type,name FROM main.sqlite_schema WHERE tbl_name=? COLLATE NOCASE AND type IN ('index','trigger') AND sql IS NOT NULL")?;
        statement.bind_text(1, table)?;
        while statement.step_row()? {
            let kind = statement.column_text(0)?;
            let name = statement.column_text(1)?;
            if kind != "index" || !expected.iter().any(|item| item.kind == "index" && item.name == name) {
                return reject("unexpected core schema effect");
            }
        }
    }
    Ok(())
}

fn existing_count(connection: &VerifiedDatabaseConnection<'_>, expected: &[ExpectedSchema]) -> Result<usize> {
    let mut count = 0;
    for item in expected {
        let statement = Statement::prepare(connection.as_ptr(),
            "SELECT type,sql FROM main.sqlite_schema WHERE name=? COLLATE NOCASE")?;
        statement.bind_text(1, &item.name)?;
        if statement.step_row()? {
            if statement.column_text(0)? != item.kind || statement.column_text(1)? != item.sql {
                return reject("product core schema mismatch");
            }
            if statement.step_row()? { return reject("ambiguous product core schema"); }
            count += 1;
        }
    }
    Ok(count)
}

fn ensure_in_transaction(connection: &mut VerifiedDatabaseConnection<'_>) -> Result<()> {
    let expected = expected_schema()?;
    reject_shadow_or_effects(connection, &expected)?;
    let count = existing_count(connection, &expected)?;
    if count == 0 {
        // Introduce the whole known family, never fill holes in existing history.
        for sql in core_schema_statements() { exec(connection, &sql)?; }
    } else if count != expected.len() {
        return reject("incomplete product core schema");
    }
    validate_product_core_schema(connection)?;
    Ok(())
}

pub(in crate::store) fn validate_product_core_schema(connection: &mut VerifiedDatabaseConnection<'_>) -> Result<()> {
    let expected = expected_schema()?;
    reject_shadow_or_effects(connection, &expected)?;
    if existing_count(connection, &expected)? != expected.len() {
        return reject("product core schema did not persist");
    }
    for table in ADMITTED_TABLES {
        // Names are fixed internal vocabulary, never caller-provided SQL.
        let sql = format!("PRAGMA main.foreign_key_check('{table}')");
        if Statement::prepare(connection.as_ptr(), &sql)?.step_row()? {
            return reject("product core foreign key violation");
        }
    }
    Ok(())
}

struct SchemaTransaction<'db, 'root> {
    connection: &'db mut VerifiedDatabaseConnection<'root>,
    active: bool,
}
impl Drop for SchemaTransaction<'_, '_> {
    fn drop(&mut self) {
        if self.active { let _ = self.connection.execute("ROLLBACK"); }
    }
}

/// Native product bootstrap only. No flags are changed, no connection is opened,
/// and no Owner identity or grant is created here. Durable settings must already
/// be established by the owning product bootstrap. A nested BEGIN fails before
/// this function owns any rollback obligation.
pub(in crate::store) fn initialize_product_core_schema(connection: &mut VerifiedDatabaseConnection<'_>) -> Result<()> {
    assert_durability(connection)?;
    exec(connection, "BEGIN IMMEDIATE")?;
    let mut tx = SchemaTransaction { connection, active: true };
    match ensure_in_transaction(tx.connection) {
        Ok(()) => {
            if exec(tx.connection, "COMMIT").is_err() { return Err(AtomicError::CommitUnknown); }
            tx.active = false;
            Ok(())
        }
        Err(error) => {
            if exec(tx.connection, "ROLLBACK").is_err() { return Err(AtomicError::CommitUnknown); }
            tx.active = false;
            Err(error)
        }
    }
}

#[cfg(test)]
#[path = "product_core_tests.rs"]
mod tests;
