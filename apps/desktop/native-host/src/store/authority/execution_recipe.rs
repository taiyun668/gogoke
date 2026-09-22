//! Durable, immutable ExecutionRecipe versions in the existing Product Authority.
//! A recipe is configuration only: this receipt does not admit Action or prove
//! that its runtime, model, capability, Context, binding, or admission is current.

use std::collections::BTreeMap;

use super::super::atomic::{
    canonical_js_number, json_string, AtomicError, DomainRecordInput, DomainRecordReceipt, Json,
    JsonString, Parser, JS_MAX_SAFE_INTEGER,
};
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;
use super::bootstrap::OwnerIssuer;
use super::catalog::current_profile;
use super::model::{denied, identifier, next_revision, revision};
use super::transaction::{self, Result, Transaction};

const OBJECT_TYPE: &str = "ExecutionRecipe";
const EVENT_TYPE: &str = "ExecutionRecipeVersionCommitted";
const RECEIPT_TYPE: &str = "ExecutionRecipeVersionCommitted";
pub(crate) const CURRENTNESS_STATUS: &str = "PREPARATORY_CURRENTNESS_REQUIRED";
const MAX_SAFE_INTEGER: f64 = JS_MAX_SAFE_INTEGER as f64;
const HEAD_SCHEMA: &str = "CREATE TABLE gogoke_execution_recipe_heads (domain_id TEXT NOT NULL,recipe_id TEXT NOT NULL,object_type TEXT NOT NULL CHECK(object_type='ExecutionRecipe'),recipe_revision TEXT NOT NULL,content_hash TEXT NOT NULL CHECK(length(content_hash)=71),updated_at TEXT NOT NULL,PRIMARY KEY(domain_id,recipe_id),FOREIGN KEY(domain_id,object_type,recipe_id,recipe_revision) REFERENCES gogoke_objects(domain_id,object_type,object_id,object_version) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";

/// UTF-16-backed strings preserve JavaScript lone-surrogate code units.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct RecipeJsonString(Vec<u16>);

impl RecipeJsonString {
    pub(crate) fn from_utf16_units(units: Vec<u16>) -> Self {
        Self(units)
    }

    pub(crate) fn as_utf16_units(&self) -> &[u16] {
        &self.0
    }
}

impl From<&str> for RecipeJsonString {
    fn from(value: &str) -> Self {
        Self(value.encode_utf16().collect())
    }
}

impl From<String> for RecipeJsonString {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

/// JSON data with the same value kinds as Gogoke's public JsonValue contract.
/// Number validity and ECMAScript canonical bytes are checked before storage.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RecipeJsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(RecipeJsonString),
    Array(Vec<RecipeJsonValue>),
    Object(BTreeMap<RecipeJsonString, RecipeJsonValue>),
}

pub(crate) type RecipeJsonObject = BTreeMap<RecipeJsonString, RecipeJsonValue>;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExecutionRecipe {
    pub recipe_id: String,
    pub revision: String,
    pub seat_id: String,
    pub runtime_instance_id: String,
    pub model_ref: RecipeJsonObject,
    pub tool_profile: RecipeJsonValue,
    pub isolation_profile: RecipeJsonValue,
    pub context_manifest_id: String,
    pub budget_policy: RecipeJsonValue,
    pub admission_ref: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExecutionRecipeVersion {
    pub domain_id: String,
    pub recipe: ExecutionRecipe,
    pub content_hash: String,
}

/// Caller supplies the desired typed content and expected head. Product
/// Authority assigns the immutable revision and commits it with its CAS head.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AppendExecutionRecipe {
    pub operation_id: String,
    pub domain_id: String,
    pub expected_previous_revision: Option<String>,
    pub recipe_id: String,
    pub seat_id: String,
    pub runtime_instance_id: String,
    pub model_ref: RecipeJsonObject,
    pub tool_profile: RecipeJsonValue,
    pub isolation_profile: RecipeJsonValue,
    pub context_manifest_id: String,
    pub budget_policy: RecipeJsonValue,
    pub admission_ref: String,
    pub event_id: String,
    pub receipt_id: String,
    pub recorded_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExecutionRecipeReceipt {
    pub disposition: &'static str,
    pub operation_id: String,
    pub storage_receipt: DomainRecordReceipt,
    pub version: ExecutionRecipeVersion,
    pub currentness_status: &'static str,
}

fn recipe_json_value(value: &RecipeJsonValue) -> Result<Json> {
    Ok(match value {
        RecipeJsonValue::Null => Json::Null,
        RecipeJsonValue::Bool(value) => Json::Bool(*value),
        RecipeJsonValue::Number(value) => {
            if !value.is_finite() || (value.fract() == 0.0 && value.abs() > MAX_SAFE_INTEGER) {
                return Err(OrchestrationError::Invalid("execution recipe JSON number"));
            }
            Json::Number(canonical_js_number(*value))
        }
        RecipeJsonValue::String(value) => {
            Json::String(JsonString::from_units(value.as_utf16_units().to_vec()))
        }
        RecipeJsonValue::Array(values) => Json::Array(
            values
                .iter()
                .map(recipe_json_value)
                .collect::<Result<Vec<_>>>()?,
        ),
        RecipeJsonValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, value)| {
                    Ok((
                        JsonString::from_units(key.as_utf16_units().to_vec()),
                        recipe_json_value(value)?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?,
        ),
    })
}

fn recipe_json_value_from(value: Json) -> Result<RecipeJsonValue> {
    Ok(match value {
        Json::Null => RecipeJsonValue::Null,
        Json::Bool(value) => RecipeJsonValue::Bool(value),
        Json::Number(value) => RecipeJsonValue::Number(
            value
                .parse::<f64>()
                .map_err(|_| OrchestrationError::AccessDenied)?,
        ),
        Json::String(value) => {
            RecipeJsonValue::String(RecipeJsonString::from_utf16_units(value.units().to_vec()))
        }
        Json::Array(values) => RecipeJsonValue::Array(
            values
                .into_iter()
                .map(recipe_json_value_from)
                .collect::<Result<Vec<_>>>()?,
        ),
        Json::Object(fields) => RecipeJsonValue::Object(
            fields
                .into_iter()
                .map(|(key, value)| {
                    Ok((
                        RecipeJsonString::from_utf16_units(key.units().to_vec()),
                        recipe_json_value_from(value)?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?,
        ),
    })
}

fn recipe_to_json(recipe: &ExecutionRecipe) -> Result<Vec<u8>> {
    let mut fields = BTreeMap::new();
    fields.insert(
        "admissionRef".into(),
        Json::String(recipe.admission_ref.clone().into()),
    );
    fields.insert(
        "budgetPolicy".into(),
        recipe_json_value(&recipe.budget_policy)?,
    );
    fields.insert(
        "contextManifestId".into(),
        Json::String(recipe.context_manifest_id.clone().into()),
    );
    fields.insert(
        "isolationProfile".into(),
        recipe_json_value(&recipe.isolation_profile)?,
    );
    fields.insert(
        "modelRef".into(),
        Json::Object(
            recipe
                .model_ref
                .iter()
                .map(|(key, value)| {
                    Ok((
                        JsonString::from_units(key.as_utf16_units().to_vec()),
                        recipe_json_value(value)?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?,
        ),
    );
    fields.insert(
        "recipeId".into(),
        Json::String(recipe.recipe_id.clone().into()),
    );
    fields.insert(
        "revision".into(),
        Json::String(recipe.revision.clone().into()),
    );
    fields.insert(
        "runtimeInstanceId".into(),
        Json::String(recipe.runtime_instance_id.clone().into()),
    );
    fields.insert("seatId".into(), Json::String(recipe.seat_id.clone().into()));
    fields.insert(
        "toolProfile".into(),
        recipe_json_value(&recipe.tool_profile)?,
    );
    Ok(Json::Object(fields).canonical().into_bytes())
}

fn take_string(fields: &mut BTreeMap<JsonString, Json>, key: &str) -> Result<String> {
    match fields.remove(&JsonString::from_str(key)) {
        Some(Json::String(value)) => value
            .to_well_formed_string()
            .ok_or(OrchestrationError::AccessDenied),
        _ => denied(),
    }
}

fn take_value(fields: &mut BTreeMap<JsonString, Json>, key: &str) -> Result<RecipeJsonValue> {
    fields
        .remove(&JsonString::from_str(key))
        .ok_or(OrchestrationError::AccessDenied)
        .and_then(recipe_json_value_from)
}

fn recipe_from_json(bytes: &[u8]) -> Result<ExecutionRecipe> {
    let text = std::str::from_utf8(bytes).map_err(|_| OrchestrationError::AccessDenied)?;
    let value = Parser::parse(text).map_err(|_| OrchestrationError::AccessDenied)?;
    if value.canonical().as_bytes() != bytes {
        return denied();
    }
    let Json::Object(mut fields) = value else {
        return denied();
    };
    const EXPECTED: [&str; 10] = [
        "admissionRef",
        "budgetPolicy",
        "contextManifestId",
        "isolationProfile",
        "modelRef",
        "recipeId",
        "revision",
        "runtimeInstanceId",
        "seatId",
        "toolProfile",
    ];
    if fields.len() != EXPECTED.len()
        || EXPECTED
            .iter()
            .any(|key| !fields.contains_key(&JsonString::from_str(key)))
    {
        return denied();
    }
    let model_ref = match fields.remove(&JsonString::from_str("modelRef")) {
        Some(Json::Object(fields)) => fields
            .into_iter()
            .map(|(key, value)| {
                Ok((
                    RecipeJsonString::from_utf16_units(key.units().to_vec()),
                    recipe_json_value_from(value)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?,
        _ => return denied(),
    };
    let recipe = ExecutionRecipe {
        recipe_id: take_string(&mut fields, "recipeId")?,
        revision: take_string(&mut fields, "revision")?,
        seat_id: take_string(&mut fields, "seatId")?,
        runtime_instance_id: take_string(&mut fields, "runtimeInstanceId")?,
        model_ref,
        tool_profile: take_value(&mut fields, "toolProfile")?,
        isolation_profile: take_value(&mut fields, "isolationProfile")?,
        context_manifest_id: take_string(&mut fields, "contextManifestId")?,
        budget_policy: take_value(&mut fields, "budgetPolicy")?,
        admission_ref: take_string(&mut fields, "admissionRef")?,
    };
    if !fields.is_empty() {
        return denied();
    }
    validate_recipe(&recipe)?;
    Ok(recipe)
}

fn validate_recipe(recipe: &ExecutionRecipe) -> Result<()> {
    for value in [
        recipe.recipe_id.as_str(),
        recipe.seat_id.as_str(),
        recipe.runtime_instance_id.as_str(),
        recipe.context_manifest_id.as_str(),
        recipe.admission_ref.as_str(),
    ] {
        identifier(value)?;
    }
    if revision(&recipe.revision)? == 0 {
        return denied();
    }
    for value in recipe.model_ref.values().chain([
        &recipe.tool_profile,
        &recipe.isolation_profile,
        &recipe.budget_policy,
    ]) {
        let _ = recipe_json_value(value)?;
    }
    Ok(())
}

fn json_optional_string(value: Option<&str>) -> String {
    value.map(json_string).unwrap_or_else(|| "null".to_owned())
}

fn stream_id(recipe_id: &str) -> String {
    format!("gogoke.execution-recipe.v1/{recipe_id}")
}

fn event_bytes(
    domain_id: &str,
    recipe: &ExecutionRecipe,
    expected_previous_revision: Option<&str>,
    hash: &str,
) -> Vec<u8> {
    format!(
        "{{\"contentHash\":{},\"domainId\":{},\"expectedPreviousRevision\":{},\"recipeId\":{},\"revision\":{},\"type\":{}}}",
        json_string(hash),
        json_string(domain_id),
        json_optional_string(expected_previous_revision),
        json_string(&recipe.recipe_id),
        json_string(&recipe.revision),
        json_string(EVENT_TYPE),
    )
    .into_bytes()
}

fn receipt_bytes(
    domain_id: &str,
    operation_id: &str,
    recipe: &ExecutionRecipe,
    hash: &str,
) -> Vec<u8> {
    format!(
        "{{\"contentHash\":{},\"currentnessStatus\":{},\"domainId\":{},\"operationId\":{},\"recipeId\":{},\"revision\":{},\"type\":{}}}",
        json_string(hash),
        json_string(CURRENTNESS_STATUS),
        json_string(domain_id),
        json_string(operation_id),
        json_string(&recipe.recipe_id),
        json_string(&recipe.revision),
        json_string(RECEIPT_TYPE),
    )
    .into_bytes()
}

fn ensure_schema(tx: &mut Transaction<'_, '_>) -> Result<()> {
    tx.validate_product_core_schema()?;
    let rows = tx.query(
        "SELECT type,sql FROM main.sqlite_schema WHERE name='gogoke_execution_recipe_heads'",
        &[],
        2,
    )?;
    if rows.is_empty() {
        tx.write(HEAD_SCHEMA, &[])?;
    } else if rows.len() != 1 || rows[0][0] != "table" || rows[0][1] != HEAD_SCHEMA {
        return denied();
    }
    if !tx
        .query(
            "SELECT name FROM main.sqlite_schema WHERE type='trigger' AND lower(tbl_name)='gogoke_execution_recipe_heads' LIMIT 1",
            &[],
            1,
        )?
        .is_empty()
        || !tx
            .query(
                "SELECT name FROM temp.sqlite_schema WHERE (type IN ('table','view') AND lower(name)='gogoke_execution_recipe_heads') OR (type='trigger' AND lower(tbl_name)='gogoke_execution_recipe_heads') LIMIT 1",
                &[],
                1,
            )?
            .is_empty()
    {
        return denied();
    }
    Ok(())
}

pub(crate) fn initialize_execution_recipe_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<()> {
    transaction::run(connection, ensure_schema)
}

fn build_record(
    input: &AppendExecutionRecipe,
    recipe: &ExecutionRecipe,
) -> Result<DomainRecordInput> {
    let object_bytes = recipe_to_json(recipe)?;
    let hash = content_hash(&object_bytes);
    let expected_counter = input
        .expected_previous_revision
        .as_deref()
        .map(|previous| {
            revision(previous)?
                .checked_sub(1)
                .map(|counter| counter.to_string())
                .ok_or(OrchestrationError::Invalid(
                    "execution recipe previous revision",
                ))
        })
        .transpose()?;
    let counter = revision(&recipe.revision)?
        .checked_sub(1)
        .ok_or(OrchestrationError::Invalid("execution recipe revision"))?
        .to_string();
    Ok(DomainRecordInput {
        domain_id: input.domain_id.clone(),
        object_type: OBJECT_TYPE.to_owned(),
        object_id: recipe.recipe_id.clone(),
        object_version: recipe.revision.clone(),
        object_bytes,
        native_identity: None,
        event_id: input.event_id.clone(),
        stream_id: stream_id(&recipe.recipe_id),
        expected_previous_counter: expected_counter,
        counter,
        event_type: EVENT_TYPE.to_owned(),
        occurred_at: input.recorded_at.clone(),
        event_bytes: event_bytes(
            &input.domain_id,
            recipe,
            input.expected_previous_revision.as_deref(),
            &hash,
        ),
        receipt_id: input.receipt_id.clone(),
        operation_id: input.operation_id.clone(),
        receipt_type: RECEIPT_TYPE.to_owned(),
        recorded_at: input.recorded_at.clone(),
        receipt_bytes: receipt_bytes(&input.domain_id, &input.operation_id, recipe, &hash),
    })
}

fn validate_append(input: &AppendExecutionRecipe) -> Result<ExecutionRecipe> {
    for value in [
        input.operation_id.as_str(),
        input.domain_id.as_str(),
        input.recipe_id.as_str(),
        input.seat_id.as_str(),
        input.runtime_instance_id.as_str(),
        input.context_manifest_id.as_str(),
        input.admission_ref.as_str(),
        input.event_id.as_str(),
        input.receipt_id.as_str(),
    ] {
        identifier(value)?;
    }
    if let Some(previous) = input.expected_previous_revision.as_deref() {
        if revision(previous)? == 0 {
            return denied();
        }
    }
    let recipe = ExecutionRecipe {
        recipe_id: input.recipe_id.clone(),
        revision: input
            .expected_previous_revision
            .as_deref()
            .map(next_revision)
            .transpose()?
            .unwrap_or_else(|| "1".to_owned()),
        seat_id: input.seat_id.clone(),
        runtime_instance_id: input.runtime_instance_id.clone(),
        model_ref: input.model_ref.clone(),
        tool_profile: input.tool_profile.clone(),
        isolation_profile: input.isolation_profile.clone(),
        context_manifest_id: input.context_manifest_id.clone(),
        budget_policy: input.budget_policy.clone(),
        admission_ref: input.admission_ref.clone(),
    };
    validate_recipe(&recipe)?;
    Ok(recipe)
}

struct StoredVersion {
    version: ExecutionRecipeVersion,
    event_id: String,
    operation_id: String,
    receipt_id: String,
}

fn load_revision(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    recipe_id: &str,
    recipe_revision: &str,
) -> Result<Option<StoredVersion>> {
    identifier(domain_id)?;
    identifier(recipe_id)?;
    if revision(recipe_revision)? == 0 {
        return denied();
    }
    let rows = tx.query(
        "SELECT CAST(o.canonical_json AS TEXT),o.content_hash,r.receipt_id,r.operation_id,r.event_id,r.receipt_type,CAST(r.canonical_json AS TEXT),r.content_hash,r.object_type,r.object_id,r.object_version,r.operation_fingerprint,e.event_id,e.stream_id,e.stream_counter,e.event_type,CAST(e.canonical_json AS TEXT),e.content_hash,e.object_type,e.object_id,e.object_version,(SELECT count(*) FROM main.gogoke_receipts rr WHERE rr.domain_id=o.domain_id AND rr.object_type=o.object_type AND rr.object_id=o.object_id AND rr.object_version=o.object_version),(SELECT count(*) FROM main.gogoke_events ee WHERE ee.domain_id=o.domain_id AND ee.object_type=o.object_type AND ee.object_id=o.object_id AND ee.object_version=o.object_version),r.recorded_at,e.occurred_at FROM main.gogoke_objects o JOIN main.gogoke_receipts r ON r.domain_id=o.domain_id AND r.object_type=o.object_type AND r.object_id=o.object_id AND r.object_version=o.object_version JOIN main.gogoke_events e ON e.domain_id=r.domain_id AND e.event_id=r.event_id WHERE o.domain_id=? AND o.object_type='ExecutionRecipe' AND o.object_id=? AND o.object_version=?",
        &[domain_id, recipe_id, recipe_revision],
        25,
    )?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 {
        return denied();
    }
    let row = &rows[0];
    let canonical = row[0].as_bytes();
    let recipe = recipe_from_json(canonical)?;
    if recipe.recipe_id != recipe_id || recipe.revision != recipe_revision {
        return denied();
    }
    let object_hash = content_hash(canonical);
    if row[1] != object_hash
        || row[5] != RECEIPT_TYPE
        || row[7] != content_hash(row[6].as_bytes())
        || row[8] != OBJECT_TYPE
        || row[9] != recipe_id
        || row[10] != recipe_revision
        || row[11].len() != 71
        || row[12] != row[4]
        || row[13] != stream_id(recipe_id)
        || row[14] != (revision(recipe_revision)? - 1).to_string()
        || row[15] != EVENT_TYPE
        || row[17] != content_hash(row[16].as_bytes())
        || row[18] != OBJECT_TYPE
        || row[19] != recipe_id
        || row[20] != recipe_revision
        || row[21] != "1"
        || row[22] != "1"
        || row[23] != row[24]
    {
        return denied();
    }
    let expected_previous = revision(recipe_revision)?
        .checked_sub(1)
        .filter(|previous| *previous > 0)
        .map(|previous| previous.to_string());
    if row[16].as_bytes()
        != event_bytes(
            domain_id,
            &recipe,
            expected_previous.as_deref(),
            &object_hash,
        )
        || row[6].as_bytes() != receipt_bytes(domain_id, &row[3], &recipe, &object_hash)
    {
        return denied();
    }
    let version_number = revision(recipe_revision)?;
    let expected_event = event_bytes(
        domain_id,
        &recipe,
        expected_previous.as_deref(),
        &object_hash,
    );
    let expected_receipt = receipt_bytes(domain_id, &row[3], &recipe, &object_hash);
    let replay = tx.apply_domain_record(DomainRecordInput {
        domain_id: domain_id.to_owned(),
        object_type: OBJECT_TYPE.to_owned(),
        object_id: recipe_id.to_owned(),
        object_version: recipe_revision.to_owned(),
        object_bytes: canonical.to_vec(),
        native_identity: None,
        event_id: row[4].clone(),
        stream_id: stream_id(recipe_id),
        expected_previous_counter: version_number.checked_sub(2).map(|value| value.to_string()),
        counter: (version_number - 1).to_string(),
        event_type: EVENT_TYPE.to_owned(),
        occurred_at: row[24].clone(),
        event_bytes: expected_event,
        receipt_id: row[2].clone(),
        operation_id: row[3].clone(),
        receipt_type: RECEIPT_TYPE.to_owned(),
        recorded_at: row[23].clone(),
        receipt_bytes: expected_receipt,
    })?;
    if replay.operation_fingerprint != row[11]
        || replay.event_id != row[4]
        || replay.receipt_id != row[2]
        || replay.object_hash != row[1]
        || replay.event_hash != row[17]
        || replay.receipt_hash != row[7]
    {
        return denied();
    }
    let version = ExecutionRecipeVersion {
        domain_id: domain_id.to_owned(),
        recipe,
        content_hash: object_hash.clone(),
    };
    Ok(Some(StoredVersion {
        version,
        event_id: row[4].clone(),
        operation_id: row[3].clone(),
        receipt_id: row[2].clone(),
    }))
}

fn load_current(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    recipe_id: &str,
) -> Result<Option<ExecutionRecipeVersion>> {
    let head = tx.query(
        "SELECT recipe_revision,content_hash,object_type FROM main.gogoke_execution_recipe_heads WHERE domain_id=? AND recipe_id=?",
        &[domain_id, recipe_id],
        3,
    )?;
    if head.is_empty() {
        let orphan_object = tx.query(
            "SELECT object_version FROM main.gogoke_objects WHERE domain_id=? AND object_type='ExecutionRecipe' AND object_id=? LIMIT 1",
            &[domain_id, recipe_id],
            1,
        )?;
        let orphan_stream = tx.query(
            "SELECT counter FROM main.gogoke_stream_heads WHERE domain_id=? AND stream_id=?",
            &[domain_id, &stream_id(recipe_id)],
            1,
        )?;
        if !orphan_object.is_empty() || !orphan_stream.is_empty() {
            return denied();
        }
        return Ok(None);
    }
    if head.len() != 1 || head[0][2] != OBJECT_TYPE {
        return denied();
    }
    let current = load_revision(tx, domain_id, recipe_id, &head[0][0])?
        .ok_or(OrchestrationError::AccessDenied)?;
    if current.version.content_hash != head[0][1] {
        return denied();
    }
    let latest = tx.query(
        "SELECT object_version FROM main.gogoke_objects WHERE domain_id=? AND object_type='ExecutionRecipe' AND object_id=? ORDER BY length(object_version) DESC,object_version DESC LIMIT 1",
        &[domain_id, recipe_id],
        1,
    )?;
    let stream = tx.query(
        "SELECT counter FROM main.gogoke_stream_heads WHERE domain_id=? AND stream_id=?",
        &[domain_id, &stream_id(recipe_id)],
        1,
    )?;
    if latest.len() != 1
        || latest[0][0] != head[0][0]
        || stream.len() != 1
        || stream[0][0] != (revision(&head[0][0])? - 1).to_string()
    {
        return denied();
    }
    let version_number = revision(&head[0][0])?;
    let counts = tx.query(
        "SELECT (SELECT count(*) FROM main.gogoke_objects WHERE domain_id=? AND object_type='ExecutionRecipe' AND object_id=?),(SELECT count(*) FROM main.gogoke_events WHERE domain_id=? AND object_type='ExecutionRecipe' AND object_id=?),(SELECT count(*) FROM main.gogoke_receipts WHERE domain_id=? AND object_type='ExecutionRecipe' AND object_id=?)",
        &[domain_id, recipe_id, domain_id, recipe_id, domain_id, recipe_id],
        3,
    )?;
    if counts.len() != 1
        || counts[0][0] != version_number.to_string()
        || counts[0][1] != version_number.to_string()
        || counts[0][2] != version_number.to_string()
    {
        return denied();
    }
    for version in 1..=version_number {
        if load_revision(tx, domain_id, recipe_id, &version.to_string())?.is_none() {
            return denied();
        }
    }
    Ok(Some(current.version))
}

pub(super) fn read_current_execution_recipe_in_transaction(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    recipe_id: &str,
) -> Result<Option<ExecutionRecipeVersion>> {
    identifier(domain_id)?;
    identifier(recipe_id)?;
    ensure_schema(tx)?;
    load_current(tx, domain_id, recipe_id)
}

pub(super) fn model_ref_digest(recipe: &ExecutionRecipe) -> Result<String> {
    let value = RecipeJsonValue::Object(recipe.model_ref.clone());
    let json = recipe_json_value(&value)?;
    Ok(content_hash(json.canonical().as_bytes()))
}

fn append_in_transaction(
    tx: &mut Transaction<'_, '_>,
    input: &AppendExecutionRecipe,
    recipe: &ExecutionRecipe,
    record: DomainRecordInput,
) -> Result<ExecutionRecipeReceipt> {
    ensure_schema(tx)?;
    let existing_operation = tx.query(
        "SELECT receipt_id,event_id,object_type,object_id,object_version FROM main.gogoke_receipts WHERE domain_id=? AND operation_id=?",
        &[&input.domain_id, &input.operation_id],
        5,
    )?;
    if !existing_operation.is_empty() {
        if existing_operation.len() != 1
            || existing_operation[0][0] != input.receipt_id
            || existing_operation[0][1] != input.event_id
            || existing_operation[0][2] != OBJECT_TYPE
            || existing_operation[0][3] != recipe.recipe_id
            || existing_operation[0][4] != recipe.revision
        {
            return Err(OrchestrationError::OperationConflict);
        }
        let storage = tx
            .apply_domain_record(record)
            .map_err(|error| match error {
                OrchestrationError::Atomic(AtomicError::OperationConflict) => {
                    OrchestrationError::OperationConflict
                }
                other => other,
            })?;
        if storage.disposition != "RECONCILED" {
            return denied();
        }
        let stored = load_revision(tx, &input.domain_id, &recipe.recipe_id, &recipe.revision)?
            .ok_or(OrchestrationError::AccessDenied)?;
        let current = load_current(tx, &input.domain_id, &recipe.recipe_id)?
            .ok_or(OrchestrationError::AccessDenied)?;
        if stored.version.recipe != *recipe
            || stored.operation_id != input.operation_id
            || stored.receipt_id != input.receipt_id
            || stored.event_id != input.event_id
            || revision(&current.recipe.revision)? < revision(&recipe.revision)?
        {
            return Err(OrchestrationError::OperationConflict);
        }
        return Ok(ExecutionRecipeReceipt {
            disposition: "RECONCILED",
            operation_id: input.operation_id.clone(),
            storage_receipt: storage,
            version: stored.version,
            currentness_status: CURRENTNESS_STATUS,
        });
    }

    let current = load_current(tx, &input.domain_id, &recipe.recipe_id)?;
    if current.as_ref().map(|value| value.recipe.revision.as_str())
        != input.expected_previous_revision.as_deref()
    {
        return Err(OrchestrationError::OperationConflict);
    }
    let storage = tx.apply_domain_record(record)?;
    if storage.disposition != "COMMITTED" {
        return Err(OrchestrationError::OperationConflict);
    }
    let hash = storage.object_hash.clone();
    if let Some(previous) = input.expected_previous_revision.as_deref() {
        tx.write(
            "UPDATE gogoke_execution_recipe_heads SET recipe_revision=?,content_hash=?,updated_at=? WHERE domain_id=? AND recipe_id=? AND recipe_revision=? AND object_type='ExecutionRecipe'",
            &[&recipe.revision, &hash, &input.recorded_at, &input.domain_id, &recipe.recipe_id, previous],
        )?;
    } else {
        tx.write(
            "INSERT INTO gogoke_execution_recipe_heads(domain_id,recipe_id,object_type,recipe_revision,content_hash,updated_at) VALUES(?,?,'ExecutionRecipe',?,?,?)",
            &[&input.domain_id, &recipe.recipe_id, &recipe.revision, &hash, &input.recorded_at],
        )?;
    }
    let current = load_current(tx, &input.domain_id, &recipe.recipe_id)?
        .ok_or(OrchestrationError::AccessDenied)?;
    if current.recipe != *recipe || current.content_hash != hash {
        return denied();
    }
    Ok(ExecutionRecipeReceipt {
        disposition: "COMMITTED",
        operation_id: input.operation_id.clone(),
        storage_receipt: storage,
        version: current,
        currentness_status: CURRENTNESS_STATUS,
    })
}

pub(crate) fn append_owner_execution_recipe(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    input: &AppendExecutionRecipe,
) -> Result<ExecutionRecipeReceipt> {
    let recipe = validate_append(input)?;
    let record = build_record(input, &recipe)?;
    transaction::run(connection, |tx| {
        owner.check(&current_profile(tx)?)?;
        append_in_transaction(tx, input, &recipe, record)
    })
}

pub(crate) fn read_current_execution_recipe(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    domain_id: &str,
    recipe_id: &str,
) -> Result<Option<ExecutionRecipeVersion>> {
    identifier(domain_id)?;
    identifier(recipe_id)?;
    transaction::run(connection, |tx| {
        owner.check(&current_profile(tx)?)?;
        ensure_schema(tx)?;
        load_current(tx, domain_id, recipe_id)
    })
}

pub(crate) fn read_execution_recipe_revision(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    domain_id: &str,
    recipe_id: &str,
    recipe_revision: &str,
) -> Result<Option<ExecutionRecipeVersion>> {
    identifier(domain_id)?;
    identifier(recipe_id)?;
    revision(recipe_revision)?;
    transaction::run(connection, |tx| {
        owner.check(&current_profile(tx)?)?;
        ensure_schema(tx)?;
        let current = load_current(tx, domain_id, recipe_id)?;
        let Some(current) = current else {
            return Ok(None);
        };
        if revision(recipe_revision)? > revision(&current.recipe.revision)? {
            return Ok(None);
        }
        Ok(load_revision(tx, domain_id, recipe_id, recipe_revision)?.map(|value| value.version))
    })
}

#[cfg(test)]
mod utf16_identity_tests {
    use super::*;

    #[test]
    fn public_identity_fields_reject_unpaired_surrogates_while_json_values_preserve_them() {
        let bytes = br#"{"admissionRef":"admission-one","budgetPolicy":null,"contextManifestId":"manifest-one","isolationProfile":null,"modelRef":{},"recipeId":"\ud800","revision":"1","runtimeInstanceId":"runtime-one","seatId":"seat-one","toolProfile":null}"#;
        let parsed = Parser::parse(std::str::from_utf8(bytes).unwrap()).expect("JSON object");
        assert_eq!(parsed.canonical().as_bytes(), bytes);
        assert!(matches!(
            recipe_from_json(bytes),
            Err(OrchestrationError::AccessDenied)
        ));

        let Json::String(value) =
            Parser::parse(r#""\ud800""#).expect("generic JSON string retains lone surrogate")
        else {
            panic!("expected string");
        };
        assert_eq!(
            value.units(),
            &[0xd800],
            "generic JsonValue preserves the UTF-16 code unit"
        );
    }
}
