//! Pending reference coordination for the fixed R2-02 test ledger only.
//! This table is not an accepted Git fact or a remote-write authority.

use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;
use super::model::denied;
use super::transaction::{self, Result, Transaction};

const REPOSITORY: &str = "taiyun668/gogoke";
const DOMAIN_ID: &str = "domain-r2-02-test";
const BRANCH: &str = "s1-r4-ledger-test/r2-02";
const PATH_ROOT: &str = "apps/desktop/test-fixtures/s1-r4/ledger/r2-02-results";
const TABLE: &str = "gogoke_coordination_r2_fact_pending_refs";
const SCHEMA: &str = "CREATE TABLE gogoke_coordination_r2_fact_pending_refs (operation_id TEXT PRIMARY KEY,domain_id TEXT NOT NULL CHECK(domain_id='domain-r2-02-test'),execution_evidence_sha TEXT NOT NULL,bytes_hash TEXT NOT NULL,repository TEXT NOT NULL,branch TEXT NOT NULL,path TEXT NOT NULL UNIQUE,base_head TEXT,target_commit TEXT,CHECK((base_head IS NULL AND target_commit IS NULL) OR (base_head IS NOT NULL AND target_commit IS NOT NULL))) STRICT";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct R2TestFactIntent {
    pub operation_id: String,
    pub execution_evidence_sha: String,
    pub bytes_hash: String,
    pub repository: String,
    pub branch: String,
    pub path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct R2TestFactJournalEntry {
    pub intent: R2TestFactIntent,
    pub base_head: Option<String>,
    pub target_commit: Option<String>,
}

fn sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn bytes_hash(value: &str) -> bool {
    value.starts_with("sha256:") && value.len() == 71
        && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn validate(intent: &R2TestFactIntent) -> Result<()> {
    let operation = intent.operation_id.as_bytes();
    if operation.is_empty() || operation.len() > 64
        || !operation[0].is_ascii_lowercase() && !operation[0].is_ascii_digit()
        || !operation.iter().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
        || !sha(&intent.execution_evidence_sha) || !bytes_hash(&intent.bytes_hash)
        || intent.repository != REPOSITORY || intent.branch != BRANCH
        || intent.path != format!("{PATH_ROOT}/{}.json", intent.operation_id)
    { return denied(); }
    Ok(())
}

fn ensure_schema(tx: &mut Transaction<'_, '_>) -> Result<()> {
    let rows = tx.query("SELECT type,sql FROM main.sqlite_schema WHERE name=?", &[TABLE], 2)?;
    let temp = tx.query("SELECT name FROM temp.sqlite_schema WHERE (lower(name)=lower(?) AND type IN ('table','view')) OR (lower(tbl_name)=lower(?) AND type='trigger')", &[TABLE,TABLE], 1)?;
    if !temp.is_empty() { return denied(); }
    if rows.is_empty() { tx.write(SCHEMA, &[])?; }
    else if rows.len()!=1 || rows[0][0]!="table" || rows[0][1]!=SCHEMA { return denied(); }
    if !tx.query("SELECT name FROM main.sqlite_schema WHERE type='trigger' AND lower(tbl_name)=lower(?)", &[TABLE], 1)?.is_empty() {
        return denied();
    }
    Ok(())
}

fn read(tx: &mut Transaction<'_, '_>, operation_id: &str) -> Result<Option<R2TestFactJournalEntry>> {
    let rows = tx.query("SELECT domain_id,execution_evidence_sha,bytes_hash,repository,branch,path,COALESCE(base_head,''),COALESCE(target_commit,'') FROM main.gogoke_coordination_r2_fact_pending_refs WHERE operation_id=?", &[operation_id], 8)?;
    if rows.is_empty() { return Ok(None); }
    if rows.len()!=1 { return denied(); }
    let row = &rows[0];
    if row[0] != DOMAIN_ID { return denied(); }
    let intent = R2TestFactIntent {
        operation_id: operation_id.into(), execution_evidence_sha: row[1].clone(),
        bytes_hash: row[2].clone(), repository: row[3].clone(), branch: row[4].clone(), path: row[5].clone(),
    };
    validate(&intent)?;
    let (base_head, target_commit) = match (row[6].as_str(), row[7].as_str()) {
        ("", "") => (None, None),
        (base, target) if sha(base) && sha(target) => (Some(base.into()), Some(target.into())),
        _ => return denied(),
    };
    Ok(Some(R2TestFactJournalEntry { intent, base_head, target_commit }))
}

pub(crate) fn begin(connection: &mut VerifiedDatabaseConnection<'_>, intent: &R2TestFactIntent) -> Result<R2TestFactJournalEntry> {
    validate(intent)?;
    transaction::run(connection, |tx| {
        ensure_schema(tx)?;
        tx.write("INSERT INTO main.gogoke_coordination_r2_fact_pending_refs(operation_id,domain_id,execution_evidence_sha,bytes_hash,repository,branch,path) VALUES(?,?,?,?,?,?,?) ON CONFLICT(operation_id) DO NOTHING",
            &[&intent.operation_id,DOMAIN_ID,&intent.execution_evidence_sha,&intent.bytes_hash,&intent.repository,&intent.branch,&intent.path])?;
        let entry = read(tx, &intent.operation_id)?.ok_or(OrchestrationError::OperationConflict)?;
        if entry.intent != *intent { return Err(OrchestrationError::OperationConflict); }
        Ok(entry)
    })
}

pub(crate) fn bind(connection: &mut VerifiedDatabaseConnection<'_>, intent: &R2TestFactIntent,
    base_head: &str, target_commit: &str) -> Result<R2TestFactJournalEntry> {
    validate(intent)?;
    if !sha(base_head) || !sha(target_commit) { return denied(); }
    transaction::run(connection, |tx| {
        ensure_schema(tx)?;
        let existing = read(tx, &intent.operation_id)?.ok_or(OrchestrationError::OperationConflict)?;
        if existing.intent != *intent { return Err(OrchestrationError::OperationConflict); }
        if let (Some(base), Some(target)) = (&existing.base_head, &existing.target_commit) {
            if base != base_head || target != target_commit { return Err(OrchestrationError::OperationConflict); }
            return Ok(existing);
        }
        tx.write("UPDATE main.gogoke_coordination_r2_fact_pending_refs SET base_head=?,target_commit=? WHERE operation_id=? AND base_head IS NULL AND target_commit IS NULL",
            &[base_head,target_commit,&intent.operation_id])?;
        let changed = tx.query("SELECT changes()", &[], 1)?;
        if changed.len()!=1 || changed[0][0]!="1" { return Err(OrchestrationError::OperationConflict); }
        let bound = read(tx, &intent.operation_id)?.ok_or(OrchestrationError::OperationConflict)?;
        if bound.base_head.as_deref()!=Some(base_head) || bound.target_commit.as_deref()!=Some(target_commit) {
            return Err(OrchestrationError::OperationConflict);
        }
        Ok(bound)
    })
}

pub(crate) fn reject(connection: &mut VerifiedDatabaseConnection<'_>, intent: &R2TestFactIntent,
    base_head: &str, target_commit: &str) -> Result<R2TestFactJournalEntry> {
    validate(intent)?;
    if !sha(base_head) || !sha(target_commit) { return denied(); }
    transaction::run(connection, |tx| {
        ensure_schema(tx)?;
        let existing = read(tx, &intent.operation_id)?.ok_or(OrchestrationError::OperationConflict)?;
        if existing.intent != *intent || existing.base_head.as_deref() != Some(base_head)
            || existing.target_commit.as_deref() != Some(target_commit) {
            return Err(OrchestrationError::OperationConflict);
        }
        tx.write("UPDATE main.gogoke_coordination_r2_fact_pending_refs SET base_head=NULL,target_commit=NULL WHERE operation_id=? AND base_head=? AND target_commit=?",
            &[&intent.operation_id,base_head,target_commit])?;
        let changed = tx.query("SELECT changes()", &[], 1)?;
        if changed.len()!=1 || changed[0][0]!="1" { return Err(OrchestrationError::OperationConflict); }
        read(tx, &intent.operation_id)?.ok_or(OrchestrationError::OperationConflict)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, open_existing, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn exact_intent_and_target_survive_reopen_and_reject_conflicts() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root_path = std::env::temp_dir().join(format!("gogoke-r2-fact-journal-{nonce}"));
        std::fs::create_dir(&root_path).unwrap();
        let root = RootLock::acquire(&root_path).unwrap();
        let database = root_path.join("state.sqlite");
        let mut db = create_new(&root, &database).unwrap();
        let intent = R2TestFactIntent {
            operation_id: "r2-02-one".into(), execution_evidence_sha: "a".repeat(40),
            bytes_hash: format!("sha256:{}", "b".repeat(64)), repository: REPOSITORY.into(),
            branch: BRANCH.into(), path: format!("{PATH_ROOT}/r2-02-one.json"),
        };
        let first = begin(&mut db, &intent).unwrap();
        assert_eq!(first.base_head, None);
        assert_eq!(begin(&mut db, &intent).unwrap(), first);
        let mut changed = intent.clone(); changed.bytes_hash = format!("sha256:{}", "c".repeat(64));
        assert!(begin(&mut db, &changed).is_err());
        assert!(bind(&mut db, &changed, &"d".repeat(40), &"e".repeat(40)).is_err());
        let bound = bind(&mut db, &intent, &"d".repeat(40), &"e".repeat(40)).unwrap();
        assert_eq!(bound.target_commit.as_deref(), Some("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"));
        assert_eq!(bind(&mut db, &intent, &"d".repeat(40), &"e".repeat(40)).unwrap(), bound);
        assert!(bind(&mut db, &intent, &"d".repeat(40), &"f".repeat(40)).is_err());
        assert!(reject(&mut db, &intent, &"d".repeat(40), &"f".repeat(40)).is_err());
        let released = reject(&mut db, &intent, &"d".repeat(40), &"e".repeat(40)).unwrap();
        assert_eq!(released.base_head, None);
        assert_eq!(released.target_commit, None);
        assert!(reject(&mut db, &intent, &"d".repeat(40), &"e".repeat(40)).is_err());
        let rebound = bind(&mut db, &intent, &"f".repeat(40), &"1".repeat(40)).unwrap();
        assert_eq!(rebound.base_head.as_deref(), Some("ffffffffffffffffffffffffffffffffffffffff"));
        assert_eq!(rebound.target_commit.as_deref(), Some("1111111111111111111111111111111111111111"));
        db.close_checked().unwrap();
        let mut reopened = open_existing(&root, &database).unwrap();
        assert_eq!(begin(&mut reopened, &intent).unwrap(), rebound);
        assert!(bind(&mut reopened, &intent, &"d".repeat(40), &"f".repeat(40)).is_err());
        reopened.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(database).unwrap();
        std::fs::remove_dir(root_path).unwrap();
    }
}
