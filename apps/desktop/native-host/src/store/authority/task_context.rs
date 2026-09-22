//! Current Task authority for mandatory Context references.
//! The immutable Task objects, events, and receipts live in the existing Product
//! Authority record family; this table is only the CAS current-head pointer.
use std::collections::BTreeSet;

use super::model::{denied, identifier, next_revision, revision};
use super::transaction::{self, Result, Transaction};
use super::super::atomic::{canonical_object_without_string_field, DomainRecordInput};
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;

const MAX_MANDATORY_REFS: usize = 64;
const HEAD_SCHEMA: &str = "CREATE TABLE gogoke_task_context_heads (domain_id TEXT NOT NULL,task_id TEXT NOT NULL,object_type TEXT NOT NULL CHECK(object_type='Task'),task_revision TEXT NOT NULL,content_hash TEXT NOT NULL,updated_at TEXT NOT NULL,PRIMARY KEY(domain_id,task_id),FOREIGN KEY(domain_id,object_type,task_id,task_revision) REFERENCES gogoke_objects(domain_id,object_type,object_id,object_version) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct MandatoryContextRef {
    pub source_domain_id: String,
    pub context_id: String,
    pub version: String,
}

#[derive(Clone, Debug)]
pub(crate) struct CommitTaskContextRequirements {
    pub operation_id: String,
    pub domain_id: String,
    pub task_id: String,
    pub expected_previous_revision: Option<String>,
    pub mandatory_refs: Vec<MandatoryContextRef>,
    pub event_id: String,
    pub receipt_id: String,
    pub recorded_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskContextRequirements {
    pub domain_id: String,
    pub task_id: String,
    pub task_revision: String,
    pub content_hash: String,
    pub mandatory_refs: Vec<MandatoryContextRef>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskContextRequirementsReceipt {
    pub disposition: &'static str,
    pub operation_id: String,
    pub current: TaskContextRequirements,
}

fn ensure_schema(tx: &mut Transaction<'_, '_>) -> Result<()> {
    let rows = tx.query(
        "SELECT type,sql FROM sqlite_schema WHERE name='gogoke_task_context_heads'",
        &[],
        2,
    )?;
    if rows.is_empty() {
        tx.write(HEAD_SCHEMA, &[])?;
    } else if rows.len() != 1 || rows[0][0] != "table" || rows[0][1] != HEAD_SCHEMA {
        return denied();
    }
    if !tx.query(
        "SELECT name FROM sqlite_schema WHERE type='trigger' AND tbl_name='gogoke_task_context_heads' LIMIT 1",
        &[],
        1,
    )?.is_empty() {
        return denied();
    }
    if !tx.query(
        "SELECT name FROM sqlite_temp_schema WHERE type='trigger' AND tbl_name='gogoke_task_context_heads' LIMIT 1",
        &[],
        1,
    )?.is_empty() {
        return denied();
    }
    Ok(())
}

pub(crate) fn initialize_task_context_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<()> {
    transaction::run(connection, ensure_schema)
}

fn json_quote(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character < '\u{0020}' => {
                use std::fmt::Write as _;
                write!(&mut output, "\\u{:04x}", character as u32).expect("string write");
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn validate_refs(values: &[MandatoryContextRef]) -> Result<Vec<MandatoryContextRef>> {
    if values.len() > MAX_MANDATORY_REFS {
        return denied();
    }
    let mut refs = values.to_vec();
    refs.sort();
    let mut seen = BTreeSet::new();
    for reference in &refs {
        identifier(&reference.source_domain_id)?;
        identifier(&reference.context_id)?;
        revision(&reference.version)?;
        if !seen.insert((reference.source_domain_id.as_str(), reference.context_id.as_str())) {
            return denied();
        }
    }
    Ok(refs)
}

fn refs_json(values: &[MandatoryContextRef]) -> String {
    let mut output = String::from("[");
    for (index, reference) in values.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&format!(
            "{{\"contextId\":{},\"sourceDomainId\":{},\"version\":{}}}",
            json_quote(&reference.context_id),
            json_quote(&reference.source_domain_id),
            json_quote(&reference.version),
        ));
    }
    output.push(']');
    output
}

fn task_bytes(
    domain_id: &str,
    task_id: &str,
    task_revision: &str,
    refs: &[MandatoryContextRef],
) -> (Vec<u8>, String) {
    let refs = refs_json(refs);
    let body = format!(
        "{{\"domainId\":{},\"mandatoryContextRefs\":{},\"taskId\":{},\"taskRevision\":{}}}",
        json_quote(domain_id),
        refs,
        json_quote(task_id),
        json_quote(task_revision),
    );
    let hash = content_hash(body.as_bytes());
    let canonical = format!(
        "{{\"contentHash\":{},\"domainId\":{},\"mandatoryContextRefs\":{},\"taskId\":{},\"taskRevision\":{}}}",
        json_quote(&hash),
        json_quote(domain_id),
        refs,
        json_quote(task_id),
        json_quote(task_revision),
    );
    (canonical.into_bytes(), hash)
}

fn exact_keys(tx: &mut Transaction<'_, '_>, json: &str, path: Option<&str>, expected: &[&str]) -> Result<()> {
    let rows = match path {
        None => tx.query("SELECT key FROM json_each(?)", &[json], 1)?,
        Some(path) => tx.query("SELECT key FROM json_each(?,?)", &[json, path], 1)?,
    };
    if rows.len() != expected.len() {
        return denied();
    }
    let actual = rows.into_iter().map(|row| row[0].clone()).collect::<BTreeSet<_>>();
    if expected.iter().any(|key| !actual.contains(*key)) {
        return denied();
    }
    Ok(())
}

fn load_current(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    task_id: &str,
) -> Result<TaskContextRequirements> {
    identifier(domain_id)?;
    identifier(task_id)?;
    ensure_schema(tx)?;
    let rows = tx.query(
        "SELECT h.task_revision,h.content_hash,CAST(o.canonical_json AS TEXT),o.content_hash FROM gogoke_task_context_heads h JOIN gogoke_objects o ON o.domain_id=h.domain_id AND o.object_type=h.object_type AND o.object_id=h.task_id AND o.object_version=h.task_revision WHERE h.domain_id=? AND h.task_id=? AND h.object_type='Task'",
        &[domain_id, task_id],
        4,
    )?;
    if rows.len() != 1 {
        return denied();
    }
    let row = &rows[0];
    let current_revision = revision(&row[0])?;
    let expected_counter = current_revision.checked_sub(1)
        .ok_or(OrchestrationError::Invalid("authority revision"))?.to_string();
    let stream_id = format!("Task:{task_id}");
    let stream = tx.query(
        "SELECT h.counter,e.object_type,e.object_id,e.object_version,(SELECT count(*) FROM gogoke_receipts r WHERE r.domain_id=e.domain_id AND r.event_id=e.event_id AND r.object_type=e.object_type AND r.object_id=e.object_id AND r.object_version=e.object_version) FROM gogoke_stream_heads h JOIN gogoke_events e ON e.domain_id=h.domain_id AND e.stream_id=h.stream_id AND e.stream_counter=h.counter WHERE h.domain_id=? AND h.stream_id=?",
        &[domain_id, &stream_id],
        5,
    )?;
    if stream.len() != 1 || stream[0][0] != expected_counter || stream[0][1] != "Task"
        || stream[0][2] != task_id || stream[0][3] != row[0] || stream[0][4] != "1"
    {
        return denied();
    }
    let latest_object = tx.query(
        "SELECT object_version FROM gogoke_objects WHERE domain_id=? AND object_type='Task' AND object_id=? ORDER BY length(object_version) DESC,object_version DESC LIMIT 1",
        &[domain_id, task_id],
        1,
    )?;
    if latest_object.len() != 1 || revision(&latest_object[0][0])? != current_revision
        || latest_object[0][0] != row[0]
    {
        return denied();
    }
    let canonical = row[2].as_bytes();
    if content_hash(canonical) != row[3] {
        return denied();
    }
    let (body, embedded_hash) = canonical_object_without_string_field(
        canonical,
        "contentHash",
        "Task",
    ).map_err(OrchestrationError::Atomic)?;
    if embedded_hash != row[1] || content_hash(&body) != embedded_hash {
        return denied();
    }
    let json = &row[2];
    exact_keys(tx, json, None, &["contentHash", "domainId", "mandatoryContextRefs", "taskId", "taskRevision"])?;
    let identity = tx.query(
        "SELECT json_type(?,'$.domainId'),json_extract(?,'$.domainId'),json_type(?,'$.taskId'),json_extract(?,'$.taskId'),json_type(?,'$.taskRevision'),json_extract(?,'$.taskRevision'),json_type(?,'$.mandatoryContextRefs')",
        &[json, json, json, json, json, json, json],
        7,
    )?;
    if identity.len() != 1 || identity[0][0] != "text" || identity[0][1] != domain_id
        || identity[0][2] != "text" || identity[0][3] != task_id
        || identity[0][4] != "text" || identity[0][5] != row[0]
        || identity[0][6] != "array"
    {
        return denied();
    }
    let objects = tx.query(
        "SELECT CAST(value AS TEXT) FROM json_each(?,'$.mandatoryContextRefs') ORDER BY CAST(key AS INTEGER)",
        &[json],
        1,
    )?;
    let mut refs = Vec::with_capacity(objects.len());
    for object in objects {
        exact_keys(tx, &object[0], None, &["contextId", "sourceDomainId", "version"])?;
        let fields = tx.query(
            "SELECT json_type(?,'$.sourceDomainId'),json_extract(?,'$.sourceDomainId'),json_type(?,'$.contextId'),json_extract(?,'$.contextId'),json_type(?,'$.version'),json_extract(?,'$.version')",
            &[&object[0], &object[0], &object[0], &object[0], &object[0], &object[0]],
            6,
        )?;
        if fields.len() != 1 || fields[0][0] != "text" || fields[0][2] != "text" || fields[0][4] != "text" {
            return denied();
        }
        refs.push(MandatoryContextRef {
            source_domain_id: fields[0][1].clone(),
            context_id: fields[0][3].clone(),
            version: fields[0][5].clone(),
        });
    }
    let refs = validate_refs(&refs)?;
    let (expected, expected_hash) = task_bytes(domain_id, task_id, &row[0], &refs);
    if expected != canonical || expected_hash != embedded_hash {
        return denied();
    }
    Ok(TaskContextRequirements {
        domain_id: domain_id.to_owned(),
        task_id: task_id.to_owned(),
        task_revision: row[0].clone(),
        content_hash: embedded_hash,
        mandatory_refs: refs,
    })
}

pub(super) fn read_current_task_context_in_transaction(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    task_id: &str,
) -> Result<TaskContextRequirements> {
    load_current(tx, domain_id, task_id)
}

pub(crate) fn read_task_context_requirements(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    task_id: &str,
) -> Result<TaskContextRequirements> {
    transaction::run(connection, |tx| load_current(tx, domain_id, task_id))
}

pub(crate) fn commit_task_context_requirements(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &CommitTaskContextRequirements,
) -> Result<TaskContextRequirementsReceipt> {
    for value in [&input.operation_id, &input.domain_id, &input.task_id, &input.event_id, &input.receipt_id] {
        identifier(value)?;
    }
    let refs = validate_refs(&input.mandatory_refs)?;
    let task_revision = match &input.expected_previous_revision {
        None => "1".to_owned(),
        Some(previous) => next_revision(previous)?,
    };
    let counter = (revision(&task_revision)? - 1).to_string();
    let previous_counter = match &input.expected_previous_revision {
        None => None,
        Some(previous) => Some(revision(previous)?.checked_sub(1)
            .ok_or(OrchestrationError::Invalid("authority revision"))?.to_string()),
    };
    let (object_bytes, hash) = task_bytes(&input.domain_id, &input.task_id, &task_revision, &refs);
    let expected = input.expected_previous_revision.as_ref()
        .map(|value| json_quote(value)).unwrap_or_else(|| "null".into());
    let event_bytes = format!(
        "{{\"contentHash\":{},\"domainId\":{},\"expectedPreviousTaskRevision\":{},\"mandatoryContextRefs\":{},\"taskId\":{},\"taskRevision\":{},\"type\":\"TaskContextRequirementsCommitted\"}}",
        json_quote(&hash), json_quote(&input.domain_id), expected, refs_json(&refs),
        json_quote(&input.task_id), json_quote(&task_revision),
    ).into_bytes();
    let receipt_bytes = format!(
        "{{\"contentHash\":{},\"domainId\":{},\"operationId\":{},\"taskId\":{},\"taskRevision\":{},\"type\":\"TaskContextRequirementsCommitted\"}}",
        json_quote(&hash), json_quote(&input.domain_id), json_quote(&input.operation_id),
        json_quote(&input.task_id), json_quote(&task_revision),
    ).into_bytes();
    transaction::run(connection, |tx| {
        ensure_schema(tx)?;
        let head = tx.query(
            "SELECT task_revision FROM gogoke_task_context_heads WHERE domain_id=? AND task_id=?",
            &[&input.domain_id, &input.task_id],
            1,
        )?;
        if head.len() > 1 {
            return denied();
        }
        let current = if head.is_empty() {
            let stream_id = format!("Task:{}", input.task_id);
            if !tx.query(
                "SELECT object_version FROM gogoke_objects WHERE domain_id=? AND object_type='Task' AND object_id=? LIMIT 1",
                &[&input.domain_id, &input.task_id],
                1,
            )?.is_empty() || !tx.query(
                "SELECT counter FROM gogoke_stream_heads WHERE domain_id=? AND stream_id=?",
                &[&input.domain_id, &stream_id],
                1,
            )?.is_empty() {
                return denied();
            }
            None
        } else {
            Some(load_current(tx, &input.domain_id, &input.task_id)?)
        };
        let current_revision = current.as_ref().map(|value| value.task_revision.as_str());
        let normal_cas = current_revision == input.expected_previous_revision.as_deref();
        let replay_head = current_revision == Some(task_revision.as_str());

        if !normal_cas && !replay_head {
            return Err(OrchestrationError::OperationConflict);
        }
        if replay_head {
            let operation = tx.query(
                "SELECT object_type,object_id,object_version FROM gogoke_receipts WHERE domain_id=? AND operation_id=?",
                &[&input.domain_id, &input.operation_id],
                3,
            )?;
            if operation.len() != 1 || operation[0] != vec!["Task".to_owned(), input.task_id.clone(), task_revision.clone()] {
                return Err(OrchestrationError::OperationConflict);
            }
        }
        let storage = tx.apply_domain_record(DomainRecordInput {
            domain_id: input.domain_id.clone(),
            object_type: "Task".into(),
            object_id: input.task_id.clone(),
            object_version: task_revision.clone(),
            object_bytes,
            native_identity: None,
            event_id: input.event_id.clone(),
            stream_id: format!("Task:{}", input.task_id),
            expected_previous_counter: previous_counter,
            counter,
            event_type: "TaskContextRequirementsCommitted".into(),
            occurred_at: input.recorded_at.clone(),
            event_bytes,
            receipt_id: input.receipt_id.clone(),
            operation_id: input.operation_id.clone(),
            receipt_type: "TaskContextRequirementsCommitted".into(),
            recorded_at: input.recorded_at.clone(),
            receipt_bytes,
        })?;
        if storage.disposition == "COMMITTED" {
            if input.expected_previous_revision.is_none() {
                tx.write(
                    "INSERT INTO gogoke_task_context_heads(domain_id,task_id,object_type,task_revision,content_hash,updated_at) VALUES(?,?,'Task',?,?,?)",
                    &[&input.domain_id, &input.task_id, &task_revision, &hash, &input.recorded_at],
                )?;
            } else {
                tx.write(
                    "UPDATE gogoke_task_context_heads SET task_revision=?,content_hash=?,updated_at=? WHERE domain_id=? AND task_id=? AND task_revision=?",
                    &[&task_revision, &hash, &input.recorded_at, &input.domain_id, &input.task_id,
                      input.expected_previous_revision.as_deref().expect("previous")],
                )?;
            }
        }
        let current = load_current(tx, &input.domain_id, &input.task_id)?;
        if current.task_revision != task_revision || current.content_hash != hash || current.mandatory_refs != refs {
            return denied();
        }
        Ok(TaskContextRequirementsReceipt {
            disposition: storage.disposition,
            operation_id: input.operation_id.clone(),
            current,
        })
    })
}
