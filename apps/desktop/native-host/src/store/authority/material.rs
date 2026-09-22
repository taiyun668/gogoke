//! Durable TaskMaterial records in the existing Product Authority database.
//!
//! This private in-process Owner ingress is preparatory. It does not establish
//! a production material source, worker authorization, or an IPC privilege.
use super::super::atomic::{DomainRecordInput, DomainRecordReceipt};
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;
use super::bootstrap::OwnerIssuer;
use super::catalog::current_profile;
use super::model::{denied, identifier, next_revision, revision};
use super::transaction::{self, Result, Transaction};

const AUTHORITY_STATUS: &str = "PREPARATORY_TRUSTED_INGRESS_REQUIRED";
const OBJECT_TYPE: &str = "TaskMaterial";
const STREAM_PREFIX: &str = "gogoke.task-material.v1/";
const CORE_TABLES: [&str; 4] = [
    "gogoke_objects",
    "gogoke_events",
    "gogoke_receipts",
    "gogoke_stream_heads",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MaterialVisibility {
    Project,
    Private,
}

impl MaterialVisibility {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Private => "private",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "project" => Ok(Self::Project),
            "private" => Ok(Self::Private),
            _ => denied(),
        }
    }
}

/// Field shape matches the existing TypeScript `TaskMaterial` contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskMaterial {
    pub material_id: String,
    pub project_id: String,
    pub domain_id: String,
    pub owner_principal_id: String,
    pub material_class: String,
    pub visibility: MaterialVisibility,
    /// Content is opaque text; Unicode and embedded NUL are valid.
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskMaterialVersion {
    pub material: TaskMaterial,
    pub revision: String,
    pub content_hash: String,
    pub provenance_ref: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskMaterialReceipt {
    pub disposition: &'static str,
    pub authority_status: &'static str,
    pub operation_id: String,
    pub current: TaskMaterialVersion,
    pub storage_receipt: DomainRecordReceipt,
}

#[derive(Clone, Debug)]
pub(crate) struct AppendTaskMaterial {
    pub operation_id: String,
    pub expected_previous_revision: Option<String>,
    pub material: TaskMaterial,
    pub provenance_ref: String,
    pub event_id: String,
    pub receipt_id: String,
    pub recorded_at: String,
}

#[derive(Clone)]
struct PreparedMaterial {
    revision: String,
    stream_counter: String,
    previous_counter: Option<String>,
    object: Vec<u8>,
    hash: String,
    event: Vec<u8>,
    receipt: Vec<u8>,
}

fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
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

fn hash_valid(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

fn validate_material(value: &TaskMaterial, provenance_ref: &str) -> Result<()> {
    for field in [
        &value.material_id,
        &value.project_id,
        &value.domain_id,
        &value.owner_principal_id,
        &value.material_class,
        provenance_ref,
    ] {
        identifier(field)?;
    }
    Ok(())
}

fn preimage(material: &TaskMaterial, revision: &str, provenance_ref: &str) -> Vec<u8> {
    format!(
        "{{\"content\":{},\"domainId\":{},\"materialClass\":{},\"materialId\":{},\"ownerPrincipalId\":{},\"projectId\":{},\"provenanceRef\":{},\"revision\":{},\"visibility\":{}}}",
        quote(&material.content), quote(&material.domain_id), quote(&material.material_class),
        quote(&material.material_id), quote(&material.owner_principal_id), quote(&material.project_id),
        quote(provenance_ref), quote(revision), quote(material.visibility.as_str()),
    ).into_bytes()
}

fn object_bytes(
    material: &TaskMaterial,
    revision: &str,
    content_hash: &str,
    provenance_ref: &str,
) -> Vec<u8> {
    format!(
        "{{\"content\":{},\"contentHash\":{},\"domainId\":{},\"materialClass\":{},\"materialId\":{},\"ownerPrincipalId\":{},\"projectId\":{},\"provenanceRef\":{},\"revision\":{},\"visibility\":{}}}",
        quote(&material.content), quote(content_hash), quote(&material.domain_id),
        quote(&material.material_class), quote(&material.material_id),
        quote(&material.owner_principal_id), quote(&material.project_id),
        quote(provenance_ref), quote(revision), quote(material.visibility.as_str()),
    ).into_bytes()
}

fn event_bytes(
    content_hash: &str,
    previous_revision: Option<&str>,
    material_id: &str,
    operation_id: &str,
    revision: &str,
) -> Vec<u8> {
    let prior = previous_revision
        .map(quote)
        .unwrap_or_else(|| "null".into());
    format!(
        "{{\"contentHash\":{},\"expectedPreviousRevision\":{},\"materialId\":{},\"operationId\":{},\"revision\":{},\"type\":\"TaskMaterialAppended\"}}",
        quote(content_hash), prior, quote(material_id), quote(operation_id), quote(revision),
    ).into_bytes()
}

fn receipt_bytes(
    content_hash: &str,
    issuer_principal_id: &str,
    material_id: &str,
    operation_id: &str,
    provenance_ref: &str,
    revision: &str,
) -> Vec<u8> {
    format!(
        "{{\"authorityStatus\":{},\"contentHash\":{},\"issuerPrincipalId\":{},\"materialId\":{},\"operationId\":{},\"provenanceRef\":{},\"revision\":{}}}",
        quote(AUTHORITY_STATUS), quote(content_hash), quote(issuer_principal_id),
        quote(material_id), quote(operation_id), quote(provenance_ref), quote(revision),
    ).into_bytes()
}

fn prepare(input: &AppendTaskMaterial) -> Result<PreparedMaterial> {
    for field in [&input.operation_id, &input.event_id, &input.receipt_id] {
        identifier(field)?;
    }
    validate_material(&input.material, &input.provenance_ref)?;
    let version = match &input.expected_previous_revision {
        None => "1".to_owned(),
        Some(previous) => next_revision(previous)?,
    };
    let version_number = revision(&version)?;
    let previous_counter = input
        .expected_previous_revision
        .as_ref()
        .map(|previous| {
            revision(previous).and_then(|number| {
                number
                    .checked_sub(1)
                    .map(|n| n.to_string())
                    .ok_or(OrchestrationError::Invalid("authority revision"))
            })
        })
        .transpose()?;
    let counter = version_number
        .checked_sub(1)
        .ok_or(OrchestrationError::Invalid("authority revision"))?
        .to_string();
    let preimage = preimage(&input.material, &version, &input.provenance_ref);
    let hash = content_hash(&preimage);
    let object = object_bytes(&input.material, &version, &hash, &input.provenance_ref);
    let event = event_bytes(
        &hash,
        input.expected_previous_revision.as_deref(),
        &input.material.material_id,
        &input.operation_id,
        &version,
    );
    let receipt = receipt_bytes(
        &hash,
        &input.material.owner_principal_id,
        &input.material.material_id,
        &input.operation_id,
        &input.provenance_ref,
        &version,
    );
    Ok(PreparedMaterial {
        revision: version,
        stream_counter: counter,
        previous_counter,
        object,
        hash,
        event,
        receipt,
    })
}

fn json_string(tx: &mut Transaction<'_, '_>, json: &str, path: &str) -> Result<String> {
    let rows = tx.query(
        "SELECT json_type(?,?),json_extract(?,?)",
        &[json, path, json, path],
        2,
    )?;
    if rows.len() != 1 || rows[0][0] != "text" {
        return denied();
    }
    Ok(rows[0][1].clone())
}

fn exact_keys(tx: &mut Transaction<'_, '_>, json: &str, expected: &[&str]) -> Result<()> {
    let rows = tx.query("SELECT key FROM json_each(?)", &[json], 1)?;
    if rows.len() != expected.len() {
        return denied();
    }
    let keys = rows
        .into_iter()
        .map(|row| row[0].clone())
        .collect::<std::collections::BTreeSet<_>>();
    if expected.iter().any(|key| !keys.contains(*key)) {
        return denied();
    }
    Ok(())
}

fn ensure_material_storage(tx: &mut Transaction<'_, '_>) -> Result<()> {
    for table in CORE_TABLES {
        let main = tx.query(
            "SELECT type,name FROM main.sqlite_schema WHERE name=? COLLATE NOCASE",
            &[table],
            2,
        )?;
        let temp = tx.query(
            "SELECT type,name FROM temp.sqlite_schema WHERE name=? COLLATE NOCASE",
            &[table],
            2,
        )?;
        let main_triggers = tx.query(
            "SELECT name FROM main.sqlite_schema WHERE type='trigger' AND lower(tbl_name)=lower(?)",
            &[table],
            1,
        )?;
        let temp_triggers = tx.query(
            "SELECT name FROM temp.sqlite_schema WHERE type='trigger' AND lower(tbl_name)=lower(?)",
            &[table],
            1,
        )?;
        if main.len() != 1
            || main[0][0] != "table"
            || main[0][1] != table
            || !temp.is_empty()
            || !main_triggers.is_empty()
            || !temp_triggers.is_empty()
        {
            return denied();
        }
    }
    Ok(())
}

fn validate_historical_revision(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    material_id: &str,
    revision_number: u64,
) -> Result<()> {
    let version = revision_number.to_string();
    let objects = tx.query(
        "SELECT content_hash,CAST(canonical_json AS TEXT) FROM main.gogoke_objects WHERE domain_id=? AND object_type='TaskMaterial' AND object_id=? AND object_version=?",
        &[domain_id, material_id, &version],
        2,
    )?;
    if objects.len() != 1 {
        return denied();
    }
    let object = &objects[0];
    let object_valid = tx.query("SELECT json_valid(?)", &[&object[1]], 1)?;
    if !hash_valid(&object[0])
        || content_hash(object[1].as_bytes()) != object[0]
        || object_valid.len() != 1
        || object_valid[0][0] != "1"
    {
        return denied();
    }
    exact_keys(
        tx,
        &object[1],
        &[
            "content",
            "contentHash",
            "domainId",
            "materialClass",
            "materialId",
            "ownerPrincipalId",
            "projectId",
            "provenanceRef",
            "revision",
            "visibility",
        ],
    )?;
    let material = TaskMaterial {
        material_id: json_string(tx, &object[1], "$.materialId")?,
        project_id: json_string(tx, &object[1], "$.projectId")?,
        domain_id: json_string(tx, &object[1], "$.domainId")?,
        owner_principal_id: json_string(tx, &object[1], "$.ownerPrincipalId")?,
        material_class: json_string(tx, &object[1], "$.materialClass")?,
        visibility: MaterialVisibility::parse(&json_string(tx, &object[1], "$.visibility")?)?,
        content: json_string(tx, &object[1], "$.content")?,
    };
    let embedded_hash = json_string(tx, &object[1], "$.contentHash")?;
    let provenance_ref = json_string(tx, &object[1], "$.provenanceRef")?;
    if material.domain_id != domain_id
        || material.material_id != material_id
        || json_string(tx, &object[1], "$.revision")? != version
        || !hash_valid(&embedded_hash)
        || embedded_hash != content_hash(&preimage(&material, &version, &provenance_ref))
        || object_bytes(&material, &version, &embedded_hash, &provenance_ref).as_slice()
            != object[1].as_bytes()
    {
        return denied();
    }

    let stream_id = format!("{STREAM_PREFIX}{material_id}");
    let counter = (revision_number - 1).to_string();
    let events = tx.query(
        "SELECT event_id,stream_counter,event_type,object_type,object_id,object_version,CAST(canonical_json AS TEXT),content_hash,occurred_at FROM main.gogoke_events WHERE domain_id=? AND stream_id=? AND stream_counter=?",
        &[domain_id, &stream_id, &counter],
        9,
    )?;
    if events.len() != 1 {
        return denied();
    }
    let event = &events[0];
    let event_valid = tx.query("SELECT json_valid(?)", &[&event[6]], 1)?;
    if event[2] != "TaskMaterialAppended"
        || event[3] != OBJECT_TYPE
        || event[4] != material_id
        || event[5] != version
        || !hash_valid(&event[7])
        || content_hash(event[6].as_bytes()) != event[7]
        || event_valid.len() != 1
        || event_valid[0][0] != "1"
    {
        return denied();
    }
    exact_keys(
        tx,
        &event[6],
        &[
            "contentHash",
            "expectedPreviousRevision",
            "materialId",
            "operationId",
            "revision",
            "type",
        ],
    )?;
    let operation_id = json_string(tx, &event[6], "$.operationId")?;
    identifier(&operation_id)?;
    let previous_version = (revision_number > 1).then(|| (revision_number - 1).to_string());
    if event[6].as_bytes()
        != event_bytes(
            &embedded_hash,
            previous_version.as_deref(),
            material_id,
            &operation_id,
            &version,
        )
        .as_slice()
    {
        return denied();
    }

    let receipts = tx.query(
        "SELECT receipt_id,operation_id,event_id,object_type,object_id,object_version,receipt_type,CAST(canonical_json AS TEXT),content_hash,operation_fingerprint,recorded_at FROM main.gogoke_receipts WHERE domain_id=? AND event_id=? AND object_type='TaskMaterial' AND object_id=? AND object_version=?",
        &[domain_id, &event[0], material_id, &version],
        11,
    )?;
    if receipts.len() != 1 {
        return denied();
    }
    let receipt = &receipts[0];
    let receipt_valid = tx.query("SELECT json_valid(?)", &[&receipt[7]], 1)?;
    if receipt[0].is_empty()
        || receipt[1] != operation_id
        || receipt[2] != event[0]
        || receipt[3] != OBJECT_TYPE
        || receipt[4] != material_id
        || receipt[5] != version
        || receipt[6] != "TaskMaterialAppended"
        || !hash_valid(&receipt[8])
        || content_hash(receipt[7].as_bytes()) != receipt[8]
        || receipt_valid.len() != 1
        || receipt_valid[0][0] != "1"
    {
        return denied();
    }
    exact_keys(
        tx,
        &receipt[7],
        &[
            "authorityStatus",
            "contentHash",
            "issuerPrincipalId",
            "materialId",
            "operationId",
            "provenanceRef",
            "revision",
        ],
    )?;
    if receipt[7].as_bytes()
        != receipt_bytes(
            &embedded_hash,
            &material.owner_principal_id,
            material_id,
            &operation_id,
            &provenance_ref,
            &version,
        )
        .as_slice()
    {
        return denied();
    }

    let record = tx.apply_domain_record(DomainRecordInput {
        domain_id: domain_id.to_owned(),
        object_type: OBJECT_TYPE.to_owned(),
        object_id: material_id.to_owned(),
        object_version: version.clone(),
        object_bytes: object[1].as_bytes().to_vec(),
        native_identity: None,
        event_id: event[0].clone(),
        stream_id,
        expected_previous_counter: (revision_number > 1).then(|| (revision_number - 2).to_string()),
        counter,
        event_type: event[2].clone(),
        occurred_at: event[8].clone(),
        event_bytes: event[6].as_bytes().to_vec(),
        receipt_id: receipt[0].clone(),
        operation_id,
        receipt_type: receipt[6].clone(),
        recorded_at: receipt[10].clone(),
        receipt_bytes: receipt[7].as_bytes().to_vec(),
    })?;
    if record.disposition != "RECONCILED"
        || record.operation_fingerprint != receipt[9]
        || record.event_hash != event[7]
        || record.receipt_hash != receipt[8]
    {
        return denied();
    }
    Ok(())
}

fn load_current(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    material_id: &str,
) -> Result<Option<TaskMaterialVersion>> {
    ensure_material_storage(tx)?;
    identifier(domain_id)?;
    identifier(material_id)?;
    let rows = tx.query(
        "SELECT object_version,content_hash,CAST(canonical_json AS TEXT) FROM main.gogoke_objects WHERE domain_id=? AND object_type='TaskMaterial' AND object_id=? ORDER BY length(object_version) DESC,object_version DESC LIMIT 1",
        &[domain_id, material_id], 3)?;
    if rows.is_empty() {
        let stream = format!("{STREAM_PREFIX}{material_id}");
        if !tx.query("SELECT counter FROM main.gogoke_stream_heads WHERE domain_id=? AND stream_id=?", &[domain_id,&stream], 1)?.is_empty()
            || !tx.query("SELECT event_id FROM main.gogoke_events WHERE domain_id=? AND object_type='TaskMaterial' AND object_id=?", &[domain_id,material_id], 1)?.is_empty()
            || !tx.query("SELECT receipt_id FROM main.gogoke_receipts WHERE domain_id=? AND object_type='TaskMaterial' AND object_id=?", &[domain_id,material_id], 1)?.is_empty() {
            return denied();
        }
        return Ok(None);
    }
    if rows.len() != 1 {
        return denied();
    }
    let row = &rows[0];
    let current_revision = revision(&row[0])?;
    if current_revision == 0 || !hash_valid(&row[1]) || content_hash(row[2].as_bytes()) != row[1] {
        return denied();
    }
    let json = &row[2];
    let valid = tx.query("SELECT json_valid(?)", &[json], 1)?;
    if valid.len() != 1 || valid[0][0] != "1" {
        return denied();
    }
    exact_keys(
        tx,
        json,
        &[
            "content",
            "contentHash",
            "domainId",
            "materialClass",
            "materialId",
            "ownerPrincipalId",
            "projectId",
            "provenanceRef",
            "revision",
            "visibility",
        ],
    )?;
    let material = TaskMaterial {
        material_id: json_string(tx, json, "$.materialId")?,
        project_id: json_string(tx, json, "$.projectId")?,
        domain_id: json_string(tx, json, "$.domainId")?,
        owner_principal_id: json_string(tx, json, "$.ownerPrincipalId")?,
        material_class: json_string(tx, json, "$.materialClass")?,
        visibility: MaterialVisibility::parse(&json_string(tx, json, "$.visibility")?)?,
        content: json_string(tx, json, "$.content")?,
    };
    let embedded_hash = json_string(tx, json, "$.contentHash")?;
    let provenance_ref = json_string(tx, json, "$.provenanceRef")?;
    let embedded_revision = json_string(tx, json, "$.revision")?;
    validate_material(&material, &provenance_ref)?;
    if material.domain_id != domain_id
        || material.material_id != material_id
        || embedded_revision != row[0]
        || !hash_valid(&embedded_hash)
        || embedded_hash != content_hash(&preimage(&material, &row[0], &provenance_ref))
        || object_bytes(&material, &row[0], &embedded_hash, &provenance_ref).as_slice()
            != row[2].as_bytes()
    {
        return denied();
    }
    let history = tx.query(
        "SELECT (SELECT count(*) FROM main.gogoke_objects WHERE domain_id=? AND object_type='TaskMaterial' AND object_id=?),(SELECT count(*) FROM main.gogoke_events WHERE domain_id=? AND stream_id=?),(SELECT count(*) FROM main.gogoke_receipts WHERE domain_id=? AND object_type='TaskMaterial' AND object_id=?),(SELECT count(*) FROM main.gogoke_objects o WHERE o.domain_id=? AND o.object_type='TaskMaterial' AND o.object_id=? AND ((SELECT count(*) FROM main.gogoke_events e WHERE e.domain_id=o.domain_id AND e.object_type=o.object_type AND e.object_id=o.object_id AND e.object_version=o.object_version AND e.stream_id=? AND e.event_type='TaskMaterialAppended')<>1 OR (SELECT count(*) FROM main.gogoke_receipts r WHERE r.domain_id=o.domain_id AND r.object_type=o.object_type AND r.object_id=o.object_id AND r.object_version=o.object_version)<>1)),(SELECT count(*) FROM main.gogoke_events WHERE domain_id=? AND object_type='TaskMaterial' AND object_id=?)",
        &[domain_id,material_id,domain_id,&format!("{STREAM_PREFIX}{material_id}"),domain_id,material_id,domain_id,material_id,&format!("{STREAM_PREFIX}{material_id}"),domain_id,material_id], 5)?;
    if history.len() != 1
        || history[0][0] != current_revision.to_string()
        || history[0][1] != current_revision.to_string()
        || history[0][2] != current_revision.to_string()
        || history[0][3] != "0"
        || history[0][4] != current_revision.to_string()
    {
        return denied();
    }
    let stream_id = format!("{STREAM_PREFIX}{material_id}");
    let expected_counter = (current_revision - 1).to_string();
    let head = tx.query(
        "SELECT counter FROM main.gogoke_stream_heads WHERE domain_id=? AND stream_id=?",
        &[domain_id, &stream_id],
        1,
    )?;
    let event_rows = tx.query(
        "SELECT event_id,stream_counter,event_type,object_type,object_id,object_version,CAST(canonical_json AS TEXT),content_hash,occurred_at FROM main.gogoke_events WHERE domain_id=? AND stream_id=? AND stream_counter=?",
        &[domain_id,&stream_id,&expected_counter], 9)?;
    if head.len() != 1 || head[0][0] != expected_counter || event_rows.len() != 1 {
        return denied();
    }
    let event = &event_rows[0];
    if event[2] != "TaskMaterialAppended"
        || event[3] != OBJECT_TYPE
        || event[4] != material_id
        || event[5] != row[0]
        || !hash_valid(&event[7])
        || content_hash(event[6].as_bytes()) != event[7]
    {
        return denied();
    }
    let receipt_rows = tx.query(
        "SELECT receipt_id,operation_id,event_id,object_type,object_id,object_version,receipt_type,CAST(canonical_json AS TEXT),content_hash,operation_fingerprint,recorded_at FROM main.gogoke_receipts WHERE domain_id=? AND event_id=? AND object_type='TaskMaterial' AND object_id=? AND object_version=?",
        &[domain_id,&event[0],material_id,&row[0]], 11)?;
    if receipt_rows.len() != 1 {
        return denied();
    }
    let receipt = &receipt_rows[0];
    if receipt[2] != event[0]
        || receipt[3] != OBJECT_TYPE
        || receipt[4] != material_id
        || receipt[5] != row[0]
        || receipt[6] != "TaskMaterialAppended"
        || !hash_valid(&receipt[8])
        || content_hash(receipt[7].as_bytes()) != receipt[8]
        || !hash_valid(&receipt[9])
    {
        return denied();
    }
    let event_valid = tx.query("SELECT json_valid(?)", &[&event[6]], 1)?;
    let receipt_valid = tx.query("SELECT json_valid(?)", &[&receipt[7]], 1)?;
    if event_valid.len() != 1
        || event_valid[0][0] != "1"
        || receipt_valid.len() != 1
        || receipt_valid[0][0] != "1"
    {
        return denied();
    }
    exact_keys(
        tx,
        &event[6],
        &[
            "contentHash",
            "expectedPreviousRevision",
            "materialId",
            "operationId",
            "revision",
            "type",
        ],
    )?;
    let expected_prior = if current_revision == 1 {
        "null".to_owned()
    } else {
        quote(&(current_revision - 1).to_string())
    };
    let event_operation = json_string(tx, &event[6], "$.operationId")?;
    let event_previous_type = tx.query(
        "SELECT json_type(?,'$.expectedPreviousRevision')",
        &[&event[6]],
        1,
    )?;
    let expected_previous_matches = if expected_prior == "null" {
        event_previous_type.len() == 1 && event_previous_type[0][0] == "null"
    } else if event_previous_type.len() == 1 && event_previous_type[0][0] == "text" {
        let event_previous = tx.query(
            "SELECT json_extract(?,'$.expectedPreviousRevision')",
            &[&event[6]],
            1,
        )?;
        event_previous.len() == 1 && quote(&event_previous[0][0]) == expected_prior
    } else {
        false
    };
    if json_string(tx, &event[6], "$.contentHash")? != embedded_hash
        || json_string(tx, &event[6], "$.materialId")? != material_id
        || json_string(tx, &event[6], "$.revision")? != row[0]
        || json_string(tx, &event[6], "$.type")? != "TaskMaterialAppended"
        || !expected_previous_matches
    {
        return denied();
    }
    exact_keys(
        tx,
        &receipt[7],
        &[
            "authorityStatus",
            "contentHash",
            "issuerPrincipalId",
            "materialId",
            "operationId",
            "provenanceRef",
            "revision",
        ],
    )?;
    if json_string(tx, &receipt[7], "$.authorityStatus")? != AUTHORITY_STATUS
        || json_string(tx, &receipt[7], "$.contentHash")? != embedded_hash
        || json_string(tx, &receipt[7], "$.issuerPrincipalId")? != material.owner_principal_id
        || json_string(tx, &receipt[7], "$.materialId")? != material_id
        || json_string(tx, &receipt[7], "$.operationId")? != event_operation
        || json_string(tx, &receipt[7], "$.provenanceRef")? != provenance_ref
        || json_string(tx, &receipt[7], "$.revision")? != row[0]
    {
        return denied();
    }
    if event_operation != receipt[1] {
        return denied();
    }
    let expected_event = event_bytes(
        &embedded_hash,
        (current_revision > 1)
            .then(|| (current_revision - 1).to_string())
            .as_deref(),
        material_id,
        &event_operation,
        &row[0],
    );
    let expected_receipt = receipt_bytes(
        &embedded_hash,
        &material.owner_principal_id,
        material_id,
        &event_operation,
        &provenance_ref,
        &row[0],
    );
    if event[6].as_bytes() != expected_event.as_slice()
        || receipt[7].as_bytes() != expected_receipt.as_slice()
    {
        return denied();
    }
    let record = tx.apply_domain_record(DomainRecordInput {
        domain_id: domain_id.to_owned(),
        object_type: OBJECT_TYPE.to_owned(),
        object_id: material_id.to_owned(),
        object_version: row[0].clone(),
        object_bytes: row[2].as_bytes().to_vec(),
        native_identity: None,
        event_id: event[0].clone(),
        stream_id: stream_id.clone(),
        expected_previous_counter: (current_revision > 1)
            .then(|| (current_revision - 2).to_string()),
        counter: expected_counter,
        event_type: event[2].clone(),
        occurred_at: event[8].clone(),
        event_bytes: event[6].as_bytes().to_vec(),
        receipt_id: receipt[0].clone(),
        operation_id: receipt[1].clone(),
        receipt_type: receipt[6].clone(),
        recorded_at: receipt[10].clone(),
        receipt_bytes: receipt[7].as_bytes().to_vec(),
    })?;
    if record.disposition != "RECONCILED"
        || record.operation_fingerprint != receipt[9]
        || record.event_hash != event[7]
        || record.receipt_hash != receipt[8]
    {
        return denied();
    }
    for previous_revision in 1..current_revision {
        validate_historical_revision(tx, domain_id, material_id, previous_revision)?;
    }
    Ok(Some(TaskMaterialVersion {
        material,
        revision: row[0].clone(),
        content_hash: embedded_hash,
        provenance_ref,
    }))
}

fn apply(
    tx: &mut Transaction<'_, '_>,
    actor: &OwnerIssuer,
    input: &AppendTaskMaterial,
    prepared: &PreparedMaterial,
) -> Result<TaskMaterialReceipt> {
    let profile = current_profile(tx)?;
    actor.check(&profile)?;
    if input.material.owner_principal_id != actor.principal_id() {
        return denied();
    }
    let current = load_current(tx, &input.material.domain_id, &input.material.material_id)?;
    let next = prepared.revision.as_str();
    let current_revision = current.as_ref().map(|v| v.revision.as_str());
    let normal = current_revision == input.expected_previous_revision.as_deref();
    let replay = current_revision == Some(next);
    if !normal && !replay {
        return Err(OrchestrationError::OperationConflict);
    }
    if replay {
        let value = current.as_ref().expect("replay has current");
        if value.material != input.material
            || value.revision != prepared.revision
            || value.content_hash != prepared.hash
            || value.provenance_ref != input.provenance_ref
        {
            return Err(OrchestrationError::OperationConflict);
        }
        let receipt = tx.query("SELECT object_type,object_id,object_version FROM main.gogoke_receipts WHERE domain_id=? AND operation_id=?",
            &[&input.material.domain_id,&input.operation_id],3)?;
        if receipt.len() != 1
            || receipt[0]
                != vec![
                    OBJECT_TYPE.to_owned(),
                    input.material.material_id.clone(),
                    prepared.revision.clone(),
                ]
        {
            return Err(OrchestrationError::OperationConflict);
        }
    } else {
        let existing = tx.query(
            "SELECT receipt_id FROM main.gogoke_receipts WHERE domain_id=? AND operation_id=?",
            &[&input.material.domain_id, &input.operation_id],
            1,
        )?;
        if !existing.is_empty() {
            return Err(OrchestrationError::OperationConflict);
        }
    }
    let stored = tx.apply_domain_record(DomainRecordInput {
        domain_id: input.material.domain_id.clone(),
        object_type: OBJECT_TYPE.into(),
        object_id: input.material.material_id.clone(),
        object_version: prepared.revision.clone(),
        object_bytes: prepared.object.clone(),
        native_identity: None,
        event_id: input.event_id.clone(),
        stream_id: format!("{STREAM_PREFIX}{}", input.material.material_id),
        expected_previous_counter: prepared.previous_counter.clone(),
        counter: prepared.stream_counter.clone(),
        event_type: "TaskMaterialAppended".into(),
        occurred_at: input.recorded_at.clone(),
        event_bytes: prepared.event.clone(),
        receipt_id: input.receipt_id.clone(),
        operation_id: input.operation_id.clone(),
        receipt_type: "TaskMaterialAppended".into(),
        recorded_at: input.recorded_at.clone(),
        receipt_bytes: prepared.receipt.clone(),
    })?;
    let current = load_current(tx, &input.material.domain_id, &input.material.material_id)?
        .ok_or(OrchestrationError::AccessDenied)?;
    if current.material != input.material
        || current.revision != prepared.revision
        || current.content_hash != prepared.hash
        || current.provenance_ref != input.provenance_ref
    {
        return denied();
    }
    Ok(TaskMaterialReceipt {
        disposition: stored.disposition,
        authority_status: AUTHORITY_STATUS,
        operation_id: input.operation_id.clone(),
        current,
        storage_receipt: stored,
    })
}

/// Appends an immutable revision through the private native Owner capability.
/// Object, event and receipt commit on the same existing BEGIN IMMEDIATE.
pub(crate) fn append_trusted_task_material(
    connection: &mut VerifiedDatabaseConnection<'_>,
    actor: &OwnerIssuer,
    input: &AppendTaskMaterial,
) -> Result<TaskMaterialReceipt> {
    let prepared = prepare(input)?;
    transaction::run(connection, |tx| apply(tx, actor, input, &prepared))
}

/// Reads only through the same private in-process Owner capability. This is not
/// the production worker material resolver; its ingress status remains explicit.
pub(crate) fn read_trusted_task_material(
    connection: &mut VerifiedDatabaseConnection<'_>,
    actor: &OwnerIssuer,
    domain_id: &str,
    material_id: &str,
) -> Result<TaskMaterialVersion> {
    transaction::run(connection, |tx| {
        let profile = current_profile(tx)?;
        actor.check(&profile)?;
        load_current(tx, domain_id, material_id)?.ok_or(OrchestrationError::AccessDenied)
    })
}

/// Resolves the current typed material record while the caller already owns
/// the Product Authority write transaction. This deliberately exposes no
/// caller-supplied body and cannot open or commit a second transaction.
pub(super) fn read_current_for_authorized_task_package(
    tx: &mut Transaction<'_, '_>,
    material_id: &str,
) -> Result<TaskMaterialVersion> {
    identifier(material_id)?;
    let rows = tx.query(
        "SELECT domain_id FROM (SELECT domain_id FROM main.gogoke_objects WHERE object_type='TaskMaterial' AND object_id=? UNION SELECT domain_id FROM main.gogoke_events WHERE object_type='TaskMaterial' AND object_id=? UNION SELECT domain_id FROM main.gogoke_receipts WHERE object_type='TaskMaterial' AND object_id=? UNION SELECT domain_id FROM main.gogoke_stream_heads WHERE stream_id=?) ORDER BY domain_id LIMIT 2",
        &[
            material_id,
            material_id,
            material_id,
            &format!("{STREAM_PREFIX}{material_id}"),
        ],
        1,
    )?;
    if rows.len() != 1 {
        return denied();
    }
    load_current(tx, &rows[0][0], material_id)?.ok_or(OrchestrationError::AccessDenied)
}

#[cfg(test)]
#[path = "material_tests.rs"]
mod tests;
