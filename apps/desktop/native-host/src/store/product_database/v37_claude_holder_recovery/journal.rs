//! Durable original metadata/ACL intent for one Claude cold H operation.
//! This table contains no credential or file contents and no StopFact.
use super::*;
use crate::process::ClaudeAclObject;
use crate::store::atomic::Parser;
use crate::store::digest::sha256_hex;

const TABLE: &str = "gogoke_v37_claude_holder_recovery";
const SCHEMA:&str="CREATE TABLE gogoke_v37_claude_holder_recovery(process_operation_id TEXT PRIMARY KEY REFERENCES gogoke_coordination_process_custody(operation_id),instance_id TEXT NOT NULL,domain_id TEXT NOT NULL,session_id TEXT NOT NULL,request_id TEXT NOT NULL UNIQUE,snapshot_hex TEXT NOT NULL,snapshot_digest TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('PREPARED','APPLIED','UNKNOWN')),original_error TEXT,revision INTEGER NOT NULL CHECK(revision>=1)) STRICT";
const MAX_BYTES: usize = 2_097_152;
unsafe extern "C" {
    fn sqlite3_get_autocommit(database: *mut std::ffi::c_void) -> i32;
}
fn require_transaction(db: &VerifiedDatabaseConnection<'_>) -> Result<()> {
    if unsafe { sqlite3_get_autocommit(db.as_ptr()) } != 0 {
        return Err(denied("Claude holder journal requires transaction"));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Capture {
    pub(super) facts: BTreeMap<String, String>,
    pub(super) objects: Vec<ClaudeAclObject>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Record {
    pub(super) operation: String,
    pub(super) instance: String,
    pub(super) domain: String,
    pub(super) session: String,
    pub(super) request_id: String,
    pub(super) capture: Capture,
    pub(super) snapshot_hex: String,
    pub(super) snapshot_digest: String,
    pub(super) phase: String,
    pub(super) original_error: String,
    pub(super) revision: i64,
}

fn denied(why: &'static str) -> OrchestrationError {
    OrchestrationError::Invalid(why)
}
fn text(s: &str) -> Json {
    Json::String(JsonString::from_str(s))
}
fn field(map: &BTreeMap<String, String>, key: &str) -> Result<String> {
    map.get(key)
        .cloned()
        .ok_or_else(|| denied("Claude holder capture field absent"))
}
fn json_string(value: Json) -> Result<String> {
    let Json::String(value) = value else {
        return Err(denied("Claude holder capture non-string"));
    };
    value
        .to_well_formed_string()
        .ok_or_else(|| denied("Claude holder capture malformed UTF16"))
}
fn parse_object(value: Json) -> Result<BTreeMap<String, String>> {
    let Json::Object(values) = value else {
        return Err(denied("Claude holder capture object"));
    };
    values
        .into_iter()
        .map(|(key, value)| {
            Ok((
                key.to_well_formed_string()
                    .ok_or_else(|| denied("Claude holder capture key"))?,
                json_string(value)?,
            ))
        })
        .collect()
}
fn checked_bool(value: &str) -> Result<bool> {
    match value {
        "1" => Ok(true),
        "0" => Ok(false),
        _ => Err(denied("Claude holder object kind")),
    }
}
fn object_fields(object: &ClaudeAclObject) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("root".into(), object.root_index.to_string()),
        ("relative".into(), object.relative_utf16_hex.clone()),
        ("identity".into(), object.identity.clone()),
        (
            "directory".into(),
            if object.directory { "1" } else { "0" }.into(),
        ),
        ("control".into(), object.control.to_string()),
        ("before".into(), object.before_hex.clone()),
        ("after".into(), object.after_hex.clone()),
    ])
}
fn json_fields(map: &BTreeMap<String, String>) -> Json {
    Json::Object(
        map.iter()
            .map(|(k, v)| (JsonString::from_str(k), text(v)))
            .collect(),
    )
}
pub(super) fn encoded(capture: &Capture) -> Result<Vec<u8>> {
    if capture.objects.is_empty() || capture.objects.len() > 8192 || capture.facts.is_empty() {
        return Err(denied("Claude holder capture size"));
    }
    let body = Json::Object(BTreeMap::from([
        (JsonString::from_str("facts"), json_fields(&capture.facts)),
        (
            JsonString::from_str("objects"),
            Json::Array(
                capture
                    .objects
                    .iter()
                    .map(|object| json_fields(&object_fields(object)))
                    .collect(),
            ),
        ),
    ]))
    .canonical()
    .into_bytes();
    if body.len() > MAX_BYTES {
        return Err(denied("Claude holder capture exceeds bound"));
    }
    Ok(body)
}
fn decoded(bytes: &[u8]) -> Result<Capture> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(denied("Claude holder capture bounds"));
    }
    let source = std::str::from_utf8(bytes).map_err(|error| {
        OrchestrationError::V37StoreFailure(format!("Claude holder capture UTF8: {error}"))
    })?;
    let parsed = Parser::parse(source).map_err(|error| {
        OrchestrationError::V37StoreFailure(format!("Claude holder capture JSON: {error:?}"))
    })?;
    if parsed.canonical() != source {
        return Err(denied("Claude holder capture noncanonical"));
    }
    let Json::Object(mut values) = parsed else {
        return Err(denied("Claude holder capture root"));
    };
    if values.len() != 2 {
        return Err(denied("Claude holder capture fields"));
    }
    let facts = parse_object(
        values
            .remove(&JsonString::from_str("facts"))
            .ok_or_else(|| denied("Claude holder facts absent"))?,
    )?;
    let Json::Array(objects) = values
        .remove(&JsonString::from_str("objects"))
        .ok_or_else(|| denied("Claude holder objects absent"))?
    else {
        return Err(denied("Claude holder object array"));
    };
    if objects.is_empty() || objects.len() > 8192 {
        return Err(denied("Claude holder object count"));
    }
    let objects = objects
        .into_iter()
        .map(|value| {
            let mut map = parse_object(value)?;
            if map.len() != 7 {
                return Err(denied("Claude holder object field count"));
            }
            let root_index = field(&map, "root")?
                .parse::<usize>()
                .map_err(|_| denied("Claude holder root index"))?;
            let control = field(&map, "control")?
                .parse::<u16>()
                .map_err(|_| denied("Claude holder control"))?;
            let object = ClaudeAclObject {
                root_index,
                relative_utf16_hex: field(&map, "relative")?,
                identity: field(&map, "identity")?,
                directory: checked_bool(&field(&map, "directory")?)?,
                control,
                before_hex: field(&map, "before")?,
                after_hex: field(&map, "after")?,
            };
            for key in [
                "root",
                "relative",
                "identity",
                "directory",
                "control",
                "before",
                "after",
            ] {
                map.remove(key);
            }
            if !map.is_empty() {
                return Err(denied("Claude holder object extra field"));
            }
            Ok(object)
        })
        .collect::<Result<Vec<_>>>()?;
    let capture = Capture { facts, objects };
    if encoded(&capture)? != bytes {
        return Err(denied("Claude holder capture changed"));
    }
    Ok(capture)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(source: &str) -> Result<Vec<u8>> {
    if source.is_empty()
        || source.len() > MAX_BYTES * 2
        || source.len() % 2 != 0
        || !source
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(denied("Claude holder snapshot hex"));
    }
    source
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(
                std::str::from_utf8(pair).map_err(|_| denied("Claude holder hex UTF8"))?,
                16,
            )
            .map_err(|_| denied("Claude holder hex digit"))
        })
        .collect()
}

fn schema_state(db: &VerifiedDatabaseConnection<'_>) -> Result<bool> {
    for sql in [
        "SELECT 1 FROM temp.sqlite_schema WHERE lower(name)=?1 OR lower(tbl_name)=?1 LIMIT 1",
        "SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND lower(tbl_name)=?1 LIMIT 1",
    ]{
        let q=Statement::prepare(db.as_ptr(),sql)?;q.bind_text(1,TABLE)?;
        if q.step_row()?{return Err(denied("Claude holder schema side effect"));}
    }
    let q = Statement::prepare(
        db.as_ptr(),
        "SELECT name,sql,type FROM main.sqlite_schema WHERE lower(name)=?1",
    )?;
    q.bind_text(1, TABLE)?;
    if !q.step_row()? {
        return Ok(false);
    }
    if q.column_text(0)? != TABLE
        || q.column_text(1)? != SCHEMA
        || q.column_text(2)? != "table"
        || q.step_row()?
    {
        return Err(denied("Claude holder schema drift"));
    }
    Ok(true)
}
pub(super) fn initialize(db: &mut VerifiedDatabaseConnection<'_>) -> Result<()> {
    if schema_state(db)? {
        return Ok(());
    }
    db.execute("BEGIN IMMEDIATE")
        .map_err(OrchestrationError::CommitUnknownWithCause)?;
    let result = (|| -> Result<()> {
        if schema_state(db)? {
            return Err(denied("Claude holder schema creation raced"));
        }
        db.execute(SCHEMA).map_err(|error| {
            OrchestrationError::V37StoreFailure(format!("Claude holder schema creation: {error:?}"))
        })?;
        if !schema_state(db)? {
            return Err(denied("Claude holder schema not created"));
        }
        Ok(())
    })();
    match result {
        Ok(()) => db
            .execute("COMMIT")
            .map_err(OrchestrationError::CommitUnknownWithCause),
        Err(error) => {
            db.execute("ROLLBACK")
                .map_err(OrchestrationError::CommitUnknownWithCause)?;
            Err(error)
        }
    }
}
pub(super) fn read(db: &VerifiedDatabaseConnection<'_>, operation: &str) -> Result<Option<Record>> {
    if !schema_state(db)? {
        return Err(denied("Claude holder schema absent"));
    }
    let q = Statement::prepare(
        db.as_ptr(),
        "SELECT instance_id,domain_id,session_id,request_id,snapshot_hex,snapshot_digest,phase,
                COALESCE(original_error,''),revision
         FROM main.gogoke_v37_claude_holder_recovery WHERE process_operation_id=?1",
    )?;
    q.bind_text(1, operation)?;
    if !q.step_row()? {
        return Ok(None);
    }
    let instance = q.column_text(0)?;
    let domain = q.column_text(1)?;
    let session = q.column_text(2)?;
    let request_id = q.column_text(3)?;
    let snapshot_hex = q.column_text(4)?;
    let snapshot_digest = q.column_text(5)?;
    let phase = q.column_text(6)?;
    let original_error = q.column_text(7)?;
    let revision = q
        .column_text(8)?
        .parse::<i64>()
        .map_err(|_| denied("Claude holder journal revision"))?;
    if q.step_row()?
        || !matches!(phase.as_str(), "PREPARED" | "APPLIED" | "UNKNOWN")
        || revision < 1
    {
        return Err(denied("Claude holder journal row"));
    }
    let bytes = unhex(&snapshot_hex)?;
    if sha256_hex(&bytes) != snapshot_digest {
        return Err(denied("Claude holder snapshot digest"));
    }
    let capture = decoded(&bytes)?;
    for (key, value) in [
        ("operation", operation),
        ("instance", &instance),
        ("domain", &domain),
        ("session", &session),
        ("request", &request_id),
    ] {
        if field(&capture.facts, key)? != value {
            return Err(denied("Claude holder journal identity"));
        }
    }
    let operation_digest = sha256_hex(operation.as_bytes());
    if request_id != format!("claude-gone-{}", &operation_digest[..40])
        || (phase == "PREPARED" && revision != 1)
        || (phase == "APPLIED" && revision != 2)
        || (phase == "UNKNOWN" && (revision != 2 || original_error.is_empty()))
        || (phase != "UNKNOWN" && !original_error.is_empty())
    {
        return Err(denied("Claude holder journal phase/request binding"));
    }
    Ok(Some(Record {
        operation: operation.into(),
        instance,
        domain,
        session,
        request_id,
        capture,
        snapshot_hex,
        snapshot_digest,
        phase,
        original_error,
        revision,
    }))
}
pub(super) fn insert_in_transaction(
    db: &VerifiedDatabaseConnection<'_>,
    capture: Capture,
) -> Result<Record> {
    require_transaction(db)?;
    let operation = field(&capture.facts, "operation")?;
    if read(db, &operation)?.is_some() {
        return Err(denied("Claude holder journal already exists"));
    }
    let instance = field(&capture.facts, "instance")?;
    let domain = field(&capture.facts, "domain")?;
    let session = field(&capture.facts, "session")?;
    let bytes = encoded(&capture)?;
    let snapshot_hex = hex(&bytes);
    let snapshot_digest = sha256_hex(&bytes);
    let operation_digest = sha256_hex(operation.as_bytes());
    let request_id = format!("claude-gone-{}", &operation_digest[..40]);
    if field(&capture.facts, "request")? != request_id {
        return Err(denied("Claude holder request digest"));
    }
    let q=Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_claude_holder_recovery(process_operation_id,instance_id,domain_id,session_id,
         request_id,snapshot_hex,snapshot_digest,phase,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,'PREPARED',1)")?;
    for (i, value) in [
        &operation,
        &instance,
        &domain,
        &session,
        &request_id,
        &snapshot_hex,
        &snapshot_digest,
    ]
    .iter()
    .enumerate()
    {
        q.bind_text(i as i32 + 1, value)?;
    }
    q.step_done()?;
    Ok(Record {
        operation,
        instance,
        domain,
        session,
        request_id,
        capture,
        snapshot_hex,
        snapshot_digest,
        phase: "PREPARED".into(),
        original_error: String::new(),
        revision: 1,
    })
}
pub(super) fn applied_in_transaction(
    db: &VerifiedDatabaseConnection<'_>,
    record: &Record,
) -> Result<()> {
    require_transaction(db)?;
    if record.phase != "PREPARED" {
        return Err(denied("Claude holder apply phase"));
    }
    let q = Statement::prepare(
        db.as_ptr(),
        "UPDATE main.gogoke_v37_claude_holder_recovery SET phase='APPLIED',revision=revision+1
         WHERE process_operation_id=?1 AND snapshot_hex=?2 AND snapshot_digest=?3
           AND phase='PREPARED' AND revision=?4",
    )?;
    q.bind_text(1, &record.operation)?;
    q.bind_text(2, &record.snapshot_hex)?;
    q.bind_text(3, &record.snapshot_digest)?;
    q.bind_i64(4, record.revision)?;
    q.step_done()?;
    let changes = Statement::prepare(db.as_ptr(), "SELECT changes()")?;
    if !changes.step_row()? || changes.column_text(0)? != "1" {
        return Err(denied("Claude holder journal CAS"));
    }
    Ok(())
}

/// A third ACL image, changed physical inventory, or lost original authority
/// permanently fences this captured intent. The original error is retained;
/// no later caller can recapture or restore an earlier before image.
pub(super) fn unknown_in_transaction(
    db: &VerifiedDatabaseConnection<'_>,
    record: &Record,
    error: &str,
) -> Result<()> {
    require_transaction(db)?;
    if record.phase != "PREPARED" || error.is_empty() {
        return Err(denied("Claude holder UNKNOWN transition"));
    }
    let bounded: String = error.chars().take(2048).collect();
    let q = Statement::prepare(
        db.as_ptr(),
        "UPDATE main.gogoke_v37_claude_holder_recovery
         SET phase='UNKNOWN',original_error=?1,revision=revision+1
         WHERE process_operation_id=?2 AND snapshot_hex=?3 AND snapshot_digest=?4
           AND phase='PREPARED' AND revision=?5 AND original_error IS NULL",
    )?;
    q.bind_text(1, &bounded)?;
    q.bind_text(2, &record.operation)?;
    q.bind_text(3, &record.snapshot_hex)?;
    q.bind_text(4, &record.snapshot_digest)?;
    q.bind_i64(5, record.revision)?;
    q.step_done()?;
    let changes = Statement::prepare(db.as_ptr(), "SELECT changes()")?;
    if !changes.step_row()? || changes.column_text(0)? != "1" {
        return Err(denied("Claude holder UNKNOWN CAS"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::same_open::route_b_test_guard;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn claude_holder_snapshot_codec_preserves_ordered_acl_and_exact_object_identity() {
        let capture = Capture {
            facts: BTreeMap::from([
                ("operation".into(), "original-operation".into()),
                ("instance".into(), "claude-instance".into()),
            ]),
            objects: vec![ClaudeAclObject {
                root_index: 2,
                relative_utf16_hex: "6100".into(),
                identity: "volume:0000000000000001/file:00000000000000000000000000000002".into(),
                directory: false,
                control: 0x0404,
                before_hex: "00000004aabbccdd".into(),
                after_hex: "00000004eeff0011".into(),
            }],
        };
        let bytes = encoded(&capture).unwrap();
        assert_eq!(decoded(&bytes).unwrap(), capture);
        let mut changed = bytes;
        changed.extend_from_slice(b" ");
        assert!(decoded(&changed).is_err());
    }

    #[test]
    fn claude_holder_unknown_journal_retains_original_error_and_never_reopens_intent() {
        let _guard = route_b_test_guard();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "claude-holder-journal-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir(&base).unwrap();
        let root = crate::root::RootLock::acquire(&base).unwrap();
        let mut product = ProductDatabase::open(&root, &base.join("state.sqlite")).unwrap();
        let operation = "original-synthetic-journal-operation";
        let request = format!("claude-gone-{}", &sha256_hex(operation.as_bytes())[..40]);
        let capture = Capture {
            facts: BTreeMap::from([
                ("operation".into(), operation.into()),
                ("instance".into(), "fixture-instance".into()),
                ("domain".into(), "fixture-domain".into()),
                ("session".into(), "fixture-session".into()),
                ("request".into(), request),
            ]),
            objects: vec![ClaudeAclObject {
                root_index: 0,
                relative_utf16_hex: String::new(),
                identity: "volume:0000000000000001/file:00000000000000000000000000000002".into(),
                directory: true,
                control: 0x0404,
                before_hex: "00000004aabbccdd".into(),
                after_hex: "00000004eeff0011".into(),
            }],
        };
        let custody=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,
             pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state)
             VALUES(?1,'fixture-ticket','fixture-nonce','1','1','fixture-image','fixture-digest',
             'fixture-instance','fixture-domain','1','UNKNOWN')").unwrap();
        custody.bind_text(1, operation).unwrap();
        custody.step_done().unwrap();
        drop(custody);
        product.connection.execute("BEGIN IMMEDIATE").unwrap();
        let record = insert_in_transaction(&product.connection, capture).unwrap();
        product.connection.execute("COMMIT").unwrap();
        assert_eq!(
            read(&product.connection, operation).unwrap().unwrap(),
            record
        );
        product.connection.execute("BEGIN IMMEDIATE").unwrap();
        unknown_in_transaction(&product.connection, &record, "original third ACL image").unwrap();
        product.connection.execute("COMMIT").unwrap();
        let unknown = read(&product.connection, operation).unwrap().unwrap();
        assert_eq!(unknown.phase, "UNKNOWN");
        assert_eq!(unknown.original_error, "original third ACL image");
        assert_eq!(unknown.snapshot_hex, record.snapshot_hex);
        product.connection.execute("BEGIN IMMEDIATE").unwrap();
        assert!(unknown_in_transaction(&product.connection, &record, "new error").is_err());
        assert!(insert_in_transaction(&product.connection, record.capture).is_err());
        product.connection.execute("ROLLBACK").unwrap();
        product.close_checked().unwrap();
        drop(root);
        std::fs::remove_dir_all(base).unwrap();
    }
}
