//! Typed NativeBinding identity in the existing Product Authority database.
//! The public R4 binding keeps its eight fields; private identity versions bind
//! it to an instance, account state and SessionLineage without changing the wire.

use super::super::atomic::{json_string, DomainRecordInput};
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;
use super::bootstrap::OwnerIssuer;
use super::catalog::current_profile;
use super::model::{denied, identifier, revision};
use super::session_lineage::read_session_lineage_in_transaction;
use super::transaction::{self, Result, Transaction};

const INSTANCE_TYPE: &str = "RuntimeInstanceIdentity";
const BINDING_TYPE: &str = "NativeBinding";
const INSTANCE_SCHEMA: &str = "CREATE TABLE gogoke_runtime_instance_identity_versions (domain_id TEXT NOT NULL,instance_id TEXT NOT NULL,identity_version TEXT NOT NULL,object_type TEXT NOT NULL CHECK(object_type='RuntimeInstanceIdentity'),driver_id TEXT NOT NULL,profile_ref TEXT NOT NULL,profile_revision TEXT NOT NULL,account_state TEXT NOT NULL CHECK(account_state IN ('PRESENT','ABSENT','UNKNOWN')),account_ref TEXT NOT NULL,auth_revision TEXT NOT NULL,content_hash TEXT NOT NULL CHECK(length(content_hash)=71),operation_id TEXT NOT NULL,receipt_id TEXT NOT NULL,PRIMARY KEY(domain_id,instance_id,identity_version),UNIQUE(domain_id,operation_id),FOREIGN KEY(domain_id,object_type,instance_id,identity_version) REFERENCES gogoke_objects(domain_id,object_type,object_id,object_version) ON DELETE RESTRICT ON UPDATE RESTRICT,FOREIGN KEY(domain_id,receipt_id) REFERENCES gogoke_receipts(domain_id,receipt_id) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";
const INSTANCE_HEAD_SCHEMA: &str = "CREATE TABLE gogoke_runtime_instance_identity_heads (domain_id TEXT NOT NULL,instance_id TEXT NOT NULL,identity_version TEXT NOT NULL,content_hash TEXT NOT NULL CHECK(length(content_hash)=71),PRIMARY KEY(domain_id,instance_id),FOREIGN KEY(domain_id,instance_id,identity_version) REFERENCES gogoke_runtime_instance_identity_versions(domain_id,instance_id,identity_version) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";
const BINDING_SCHEMA: &str = "CREATE TABLE gogoke_native_binding_versions (domain_id TEXT NOT NULL,binding_id TEXT NOT NULL,generation TEXT NOT NULL,object_type TEXT NOT NULL CHECK(object_type='NativeBinding'),source_epoch TEXT NOT NULL,instance_id TEXT NOT NULL,instance_version TEXT NOT NULL,native_identity TEXT NOT NULL,lineage_ref TEXT NOT NULL,custody_ref TEXT NOT NULL,content_hash TEXT NOT NULL CHECK(length(content_hash)=71),operation_id TEXT NOT NULL,receipt_id TEXT NOT NULL,PRIMARY KEY(domain_id,binding_id,generation),UNIQUE(domain_id,instance_id,instance_version,native_identity,generation,source_epoch),UNIQUE(domain_id,operation_id),FOREIGN KEY(domain_id,instance_id,instance_version) REFERENCES gogoke_runtime_instance_identity_versions(domain_id,instance_id,identity_version) ON DELETE RESTRICT ON UPDATE RESTRICT,FOREIGN KEY(domain_id,object_type,binding_id,generation) REFERENCES gogoke_objects(domain_id,object_type,object_id,object_version) ON DELETE RESTRICT ON UPDATE RESTRICT,FOREIGN KEY(domain_id,receipt_id) REFERENCES gogoke_receipts(domain_id,receipt_id) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";
const BINDING_HEAD_SCHEMA: &str = "CREATE TABLE gogoke_native_binding_heads (domain_id TEXT NOT NULL,binding_id TEXT NOT NULL,generation TEXT NOT NULL,source_epoch TEXT NOT NULL,content_hash TEXT NOT NULL CHECK(length(content_hash)=71),PRIMARY KEY(domain_id,binding_id),FOREIGN KEY(domain_id,binding_id,generation) REFERENCES gogoke_native_binding_versions(domain_id,binding_id,generation) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AccountRefSnapshot { Present(String), Absent, Unknown }

impl AccountRefSnapshot {
    fn columns(&self) -> (&'static str, &str) {
        match self { Self::Present(value) => ("PRESENT", value), Self::Absent => ("ABSENT", ""), Self::Unknown => ("UNKNOWN", "") }
    }
    fn decode(state: &str, value: &str) -> Result<Self> {
        match (state, value) { ("PRESENT", v) if !v.is_empty() => { identifier(v)?; Ok(Self::Present(v.into())) }, ("ABSENT", "") => Ok(Self::Absent), ("UNKNOWN", "") => Ok(Self::Unknown), _ => denied() }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeInstanceIdentitySnapshot {
    pub domain_id: String,
    pub instance_id: String,
    pub version: String,
    pub driver_id: String,
    pub profile_ref: String,
    pub profile_revision: String,
    pub account_ref: AccountRefSnapshot,
    pub auth_revision: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AppendRuntimeInstanceIdentity {
    pub operation_id: String,
    pub snapshot: RuntimeInstanceIdentitySnapshot,
    pub expected_previous_version: Option<String>,
    pub event_id: String,
    pub receipt_id: String,
    pub recorded_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeInstanceIdentityReceipt { pub disposition: &'static str, pub operation_id: String, pub content_hash: String, pub snapshot: RuntimeInstanceIdentitySnapshot }

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeBindingVersion {
    pub domain_id: String,
    pub binding_id: String,
    pub generation: String,
    pub source_epoch: String,
    pub instance: RuntimeInstanceIdentitySnapshot,
    pub native_identity: String,
    pub lineage_ref: String,
    pub custody_ref: String,
    pub content_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommitNativeBinding {
    pub operation_id: String,
    pub domain_id: String,
    pub binding_id: String,
    pub generation: String,
    pub expected_previous_generation: Option<String>,
    pub source_epoch: String,
    pub instance_id: String,
    pub instance_version: String,
    pub native_identity: String,
    pub lineage_ref: String,
    pub custody_ref: String,
    pub event_id: String,
    pub receipt_id: String,
    pub recorded_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeBindingIdentity { pub domain_id: String, pub binding_id: String, pub generation: String, pub source_epoch: String, pub instance_id: String, pub instance_version: String }

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeBindingReceipt { pub disposition: &'static str, pub operation_id: String, pub version: NativeBindingVersion }

fn checked_id(value: &str) -> Result<()> { identifier(value) }
fn checked_version(value: &str) -> Result<u64> { let number=revision(value)?; if number==0 { return denied(); } Ok(number) }
fn account_json(value: &AccountRefSnapshot) -> String { match value { AccountRefSnapshot::Present(v) => json_string(v), _ => "null".into() } }

fn instance_bytes(value: &RuntimeInstanceIdentitySnapshot) -> Vec<u8> {
    let (state, _) = value.account_ref.columns();
    format!("{{\"accountRef\":{},\"accountState\":{},\"authRevision\":{},\"domainId\":{},\"driverId\":{},\"instanceId\":{},\"profileRef\":{},\"profileRevision\":{},\"version\":{}}}", account_json(&value.account_ref),json_string(state),json_string(&value.auth_revision),json_string(&value.domain_id),json_string(&value.driver_id),json_string(&value.instance_id),json_string(&value.profile_ref),json_string(&value.profile_revision),json_string(&value.version)).into_bytes()
}
fn binding_bytes(value: &CommitNativeBinding) -> Vec<u8> {
    format!("{{\"bindingId\":{},\"custodyRef\":{},\"domainId\":{},\"generation\":{},\"instanceId\":{},\"lineageRef\":{},\"nativeIdentity\":{},\"sourceEpoch\":{}}}",json_string(&value.binding_id),json_string(&value.custody_ref),json_string(&value.domain_id),json_string(&value.generation),json_string(&value.instance_id),json_string(&value.lineage_ref),json_string(&value.native_identity),json_string(&value.source_epoch)).into_bytes()
}

fn ensure_schema(tx: &mut Transaction<'_, '_>) -> Result<()> {
    tx.validate_product_core_schema()?;
    for (name, schema) in [("gogoke_runtime_instance_identity_versions",INSTANCE_SCHEMA),("gogoke_runtime_instance_identity_heads",INSTANCE_HEAD_SCHEMA),("gogoke_native_binding_versions",BINDING_SCHEMA),("gogoke_native_binding_heads",BINDING_HEAD_SCHEMA)] {
        let rows=tx.query("SELECT type,sql FROM main.sqlite_schema WHERE name=?",&[name],2)?;
        if rows.is_empty() { tx.write(schema,&[])?; }
        else if rows.len()!=1 || rows[0][0]!="table" || rows[0][1]!=schema { return denied(); }
        if !tx.query("SELECT name FROM main.sqlite_schema WHERE type='trigger' AND lower(tbl_name)=lower(?) LIMIT 1",&[name],1)?.is_empty() || !tx.query("SELECT name FROM temp.sqlite_schema WHERE (type IN ('table','view') AND lower(name)=lower(?)) OR (type='trigger' AND lower(tbl_name)=lower(?)) LIMIT 1",&[name,name],1)?.is_empty() { return denied(); }
    }
    Ok(())
}

pub(crate) fn initialize_native_binding_schema(connection: &mut VerifiedDatabaseConnection<'_>) -> Result<()> { transaction::run(connection, ensure_schema) }

fn validate_instance(value: &RuntimeInstanceIdentitySnapshot) -> Result<()> {
    for field in [&value.domain_id,&value.instance_id,&value.driver_id,&value.profile_ref] { checked_id(field)?; }
    for field in [&value.version,&value.profile_revision,&value.auth_revision] { checked_version(field)?; }
    if let AccountRefSnapshot::Present(account)=&value.account_ref { checked_id(account)?; }
    Ok(())
}

fn instance_record(input: &AppendRuntimeInstanceIdentity) -> Result<DomainRecordInput> {
    let value=&input.snapshot; validate_instance(value)?;
    let counter=(checked_version(&value.version)?-1).to_string();
    let previous=input.expected_previous_version.as_deref().map(|v| checked_version(v).map(|n| (n-1).to_string())).transpose()?;
    let bytes=instance_bytes(value);let hash=content_hash(&bytes);
    Ok(DomainRecordInput{domain_id:value.domain_id.clone(),object_type:INSTANCE_TYPE.into(),object_id:value.instance_id.clone(),object_version:value.version.clone(),object_bytes:bytes,native_identity:None,event_id:input.event_id.clone(),stream_id:format!("gogoke.runtime-instance-identity.v1/{}",value.instance_id),expected_previous_counter:previous,counter,event_type:"RuntimeInstanceIdentityCommitted".into(),occurred_at:input.recorded_at.clone(),event_bytes:format!("{{\"contentHash\":{},\"domainId\":{},\"instanceId\":{},\"version\":{}}}",json_string(&hash),json_string(&value.domain_id),json_string(&value.instance_id),json_string(&value.version)).into_bytes(),receipt_id:input.receipt_id.clone(),operation_id:input.operation_id.clone(),receipt_type:"RuntimeInstanceIdentityCommitted".into(),recorded_at:input.recorded_at.clone(),receipt_bytes:format!("{{\"contentHash\":{},\"operationId\":{},\"version\":{}}}",json_string(&hash),json_string(&input.operation_id),json_string(&value.version)).into_bytes()})
}

fn load_instance(tx:&mut Transaction<'_, '_>,domain:&str,instance:&str,version:&str)->Result<Option<RuntimeInstanceIdentitySnapshot>> {
    ensure_schema(tx)?;
    let rows=tx.query("SELECT domain_id,instance_id,identity_version,driver_id,profile_ref,profile_revision,account_state,account_ref,auth_revision,content_hash,operation_id,receipt_id FROM main.gogoke_runtime_instance_identity_versions WHERE domain_id=? AND instance_id=? AND identity_version=?",&[domain,instance,version],12)?;
    if rows.is_empty(){return Ok(None)}
    if rows.len()!=1 {return denied()}
    let row=&rows[0];
    let value=RuntimeInstanceIdentitySnapshot{domain_id:row[0].clone(),instance_id:row[1].clone(),version:row[2].clone(),driver_id:row[3].clone(),profile_ref:row[4].clone(),profile_revision:row[5].clone(),account_ref:AccountRefSnapshot::decode(&row[6],&row[7])?,auth_revision:row[8].clone()};
    validate_instance(&value)?;
    if value.domain_id!=domain || value.instance_id!=instance || value.version!=version || content_hash(&instance_bytes(&value))!=row[9] {return denied()}
    let core=tx.query("SELECT content_hash,canonical_json FROM main.gogoke_objects WHERE domain_id=? AND object_type='RuntimeInstanceIdentity' AND object_id=? AND object_version=?",&[domain,instance,version],2)?;
    if core.len()!=1 || core[0][0]!=row[9] || core[0][1].as_bytes()!=instance_bytes(&value) {return denied()}
    let receipt=tx.query("SELECT receipt_id,event_id FROM main.gogoke_receipts WHERE domain_id=? AND operation_id=? AND object_type='RuntimeInstanceIdentity' AND object_id=? AND object_version=?",&[domain,&row[10],instance,version],2)?;
    if receipt.len()!=1 || receipt[0][0]!=row[11] {return denied()}
    let events=tx.query("SELECT event_id FROM main.gogoke_events WHERE domain_id=? AND event_id=? AND object_type='RuntimeInstanceIdentity' AND object_id=? AND object_version=?",&[domain,&receipt[0][1],instance,version],1)?;
    if events.len()!=1 {return denied()}
    Ok(Some(value))
}

fn current_instance(tx:&mut Transaction<'_, '_>,domain:&str,instance:&str)->Result<Option<RuntimeInstanceIdentitySnapshot>> {
    let rows=tx.query("SELECT identity_version,content_hash FROM main.gogoke_runtime_instance_identity_heads WHERE domain_id=? AND instance_id=?",&[domain,instance],2)?;
    if rows.is_empty(){return Ok(None)}
    if rows.len()!=1{return denied()}
    let value=load_instance(tx,domain,instance,&rows[0][0])?.ok_or(OrchestrationError::AccessDenied)?;
    if content_hash(&instance_bytes(&value))!=rows[0][1]{return denied()}
    Ok(Some(value))
}

pub(crate) fn append_runtime_instance_identity(connection:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,input:&AppendRuntimeInstanceIdentity)->Result<RuntimeInstanceIdentityReceipt>{
    checked_id(&input.operation_id)?;checked_id(&input.event_id)?;checked_id(&input.receipt_id)?;
    let record=instance_record(input)?;
    transaction::run(connection,|tx|{
        owner.check(&current_profile(tx)?)?;ensure_schema(tx)?;
        let value=&input.snapshot;
        let current=current_instance(tx,&value.domain_id,&value.instance_id)?;
        let operation=tx.query("SELECT receipt_id FROM main.gogoke_receipts WHERE domain_id=? AND operation_id=?",&[&value.domain_id,&input.operation_id],1)?;
        if !operation.is_empty(){
            if operation.len()!=1 || operation[0][0]!=input.receipt_id{return Err(OrchestrationError::OperationConflict)}
            let stored=load_instance(tx,&value.domain_id,&value.instance_id,&value.version)?.ok_or(OrchestrationError::OperationConflict)?;
            if stored!=*value || current.as_ref()!=Some(value){return Err(OrchestrationError::OperationConflict)}
            let storage=tx.apply_domain_record(record)?;
            if storage.disposition!="RECONCILED"{return denied()}
            return Ok(RuntimeInstanceIdentityReceipt{disposition:"RECONCILED",operation_id:input.operation_id.clone(),content_hash:storage.object_hash,snapshot:stored});
        }
        if current.as_ref().map(|v|v.version.as_str())!=input.expected_previous_version.as_deref(){return Err(OrchestrationError::OperationConflict)}
        if let Some(previous)=&input.expected_previous_version { if checked_version(&value.version)?!=checked_version(previous)?.checked_add(1).ok_or(OrchestrationError::OperationConflict)? {return Err(OrchestrationError::OperationConflict)} }
        else if value.version!="1" {return Err(OrchestrationError::OperationConflict)}
        let storage=tx.apply_domain_record(record)?;
        if storage.disposition!="COMMITTED"{return denied()}
        let (state,account)=value.account_ref.columns();
        tx.write("INSERT INTO gogoke_runtime_instance_identity_versions(domain_id,instance_id,identity_version,object_type,driver_id,profile_ref,profile_revision,account_state,account_ref,auth_revision,content_hash,operation_id,receipt_id) VALUES(?,?,?,'RuntimeInstanceIdentity',?,?,?,?,?,?,?,?,?,?)",&[&value.domain_id,&value.instance_id,&value.version,&value.driver_id,&value.profile_ref,&value.profile_revision,state,account,&value.auth_revision,&storage.object_hash,&input.operation_id,&input.receipt_id])?;
        if let Some(previous)=&input.expected_previous_version {
            tx.write("UPDATE gogoke_runtime_instance_identity_heads SET identity_version=?,content_hash=? WHERE domain_id=? AND instance_id=? AND identity_version=?",&[&value.version,&storage.object_hash,&value.domain_id,&value.instance_id,previous])?;
        } else {tx.write("INSERT INTO gogoke_runtime_instance_identity_heads(domain_id,instance_id,identity_version,content_hash) VALUES(?,?,?,?)",&[&value.domain_id,&value.instance_id,&value.version,&storage.object_hash])?;}
        if current_instance(tx,&value.domain_id,&value.instance_id)?.as_ref()!=Some(value){return denied()}
        Ok(RuntimeInstanceIdentityReceipt{disposition:"COMMITTED",operation_id:input.operation_id.clone(),content_hash:storage.object_hash,snapshot:value.clone()})
    })
}

pub(crate) fn read_runtime_instance_identity(connection:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,domain:&str,instance:&str,version:&str)->Result<Option<RuntimeInstanceIdentitySnapshot>>{
    checked_id(domain)?;checked_id(instance)?;checked_version(version)?;
    transaction::run(connection,|tx|{owner.check(&current_profile(tx)?)?;load_instance(tx,domain,instance,version)})
}

fn validate_binding(input:&CommitNativeBinding)->Result<()> {
    for field in [&input.operation_id,&input.domain_id,&input.binding_id,&input.instance_id,&input.native_identity,&input.lineage_ref,&input.custody_ref,&input.event_id,&input.receipt_id] {checked_id(field)?;}
    checked_version(&input.generation)?;checked_version(&input.instance_version)?;revision(&input.source_epoch)?;
    if let Some(previous)=&input.expected_previous_generation {checked_version(previous)?;}
    Ok(())
}

fn binding_record(input:&CommitNativeBinding,instance_hash:&str)->Result<DomainRecordInput>{
    validate_binding(input)?;
    let bytes=binding_bytes(input);let hash=content_hash(&bytes);
    let counter=(checked_version(&input.generation)?-1).to_string();
    let previous=input.expected_previous_generation.as_deref().map(|v| checked_version(v).map(|n|(n-1).to_string())).transpose()?;
    Ok(DomainRecordInput{domain_id:input.domain_id.clone(),object_type:BINDING_TYPE.into(),object_id:input.binding_id.clone(),object_version:input.generation.clone(),object_bytes:bytes,native_identity:None,event_id:input.event_id.clone(),stream_id:format!("gogoke.native-binding.v1/{}",input.binding_id),expected_previous_counter:previous,counter,event_type:"NativeBindingCommitted".into(),occurred_at:input.recorded_at.clone(),event_bytes:format!("{{\"bindingId\":{},\"contentHash\":{},\"domainId\":{},\"generation\":{},\"instanceVersion\":{},\"runtimeInstanceHash\":{},\"sourceEpoch\":{}}}",json_string(&input.binding_id),json_string(&hash),json_string(&input.domain_id),json_string(&input.generation),json_string(&input.instance_version),json_string(instance_hash),json_string(&input.source_epoch)).into_bytes(),receipt_id:input.receipt_id.clone(),operation_id:input.operation_id.clone(),receipt_type:"NativeBindingCommitted".into(),recorded_at:input.recorded_at.clone(),receipt_bytes:format!("{{\"bindingId\":{},\"contentHash\":{},\"generation\":{},\"operationId\":{}}}",json_string(&input.binding_id),json_string(&hash),json_string(&input.generation),json_string(&input.operation_id)).into_bytes()})
}

fn check_lineage(tx:&mut Transaction<'_, '_>,domain:&str,binding:&str,generation:&str,source_epoch:&str,native:&str,lineage_ref:&str)->Result<()> {
    let lineage=read_session_lineage_in_transaction(tx,domain,lineage_ref)?;
    if lineage.lifecycle!="ACTIVE" || lineage.domain_id!=domain || lineage.session_id!=lineage_ref || lineage.native.domain_id!=domain || lineage.native.binding_id!=binding || lineage.native.generation!=generation || lineage.native.source_epoch!=source_epoch || lineage.native.native_session_id!=native {return denied()}
    Ok(())
}

fn load_binding(tx:&mut Transaction<'_, '_>,identity:&NativeBindingIdentity)->Result<Option<NativeBindingVersion>> {
    ensure_schema(tx)?;
    let rows=tx.query("SELECT domain_id,binding_id,generation,source_epoch,instance_id,instance_version,native_identity,lineage_ref,custody_ref,content_hash,operation_id,receipt_id FROM main.gogoke_native_binding_versions WHERE domain_id=? AND binding_id=? AND generation=?",&[&identity.domain_id,&identity.binding_id,&identity.generation],12)?;
    if rows.is_empty(){return Ok(None)}
    if rows.len()!=1{return denied()}
    let row=&rows[0];
    if row[0]!=identity.domain_id || row[1]!=identity.binding_id || row[2]!=identity.generation || row[3]!=identity.source_epoch || row[4]!=identity.instance_id || row[5]!=identity.instance_version {return denied()}
    let instance=load_instance(tx,&row[0],&row[4],&row[5])?.ok_or(OrchestrationError::AccessDenied)?;
    let current=current_instance(tx,&row[0],&row[4])?.ok_or(OrchestrationError::AccessDenied)?;
    if current!=instance || instance.account_ref==AccountRefSnapshot::Unknown {return denied()}
    check_lineage(tx,&row[0],&row[1],&row[2],&row[3],&row[6],&row[7])?;
    let value=NativeBindingVersion{domain_id:row[0].clone(),binding_id:row[1].clone(),generation:row[2].clone(),source_epoch:row[3].clone(),instance,native_identity:row[6].clone(),lineage_ref:row[7].clone(),custody_ref:row[8].clone(),content_hash:row[9].clone()};
    let object=CommitNativeBinding{operation_id:row[10].clone(),domain_id:row[0].clone(),binding_id:row[1].clone(),generation:row[2].clone(),expected_previous_generation:None,source_epoch:row[3].clone(),instance_id:row[4].clone(),instance_version:row[5].clone(),native_identity:row[6].clone(),lineage_ref:row[7].clone(),custody_ref:row[8].clone(),event_id:String::new(),receipt_id:row[11].clone(),recorded_at:String::new()};
    if content_hash(&binding_bytes(&object))!=row[9]{return denied()}
    let core=tx.query("SELECT content_hash,canonical_json FROM main.gogoke_objects WHERE domain_id=? AND object_type='NativeBinding' AND object_id=? AND object_version=?",&[&row[0],&row[1],&row[2]],2)?;
    if core.len()!=1 || core[0][0]!=row[9] || core[0][1].as_bytes()!=binding_bytes(&object){return denied()}
    let receipt=tx.query("SELECT receipt_id,event_id FROM main.gogoke_receipts WHERE domain_id=? AND operation_id=? AND object_type='NativeBinding' AND object_id=? AND object_version=?",&[&row[0],&row[10],&row[1],&row[2]],2)?;
    if receipt.len()!=1 || receipt[0][0]!=row[11]{return denied()}
    let events=tx.query("SELECT event_id FROM main.gogoke_events WHERE domain_id=? AND event_id=? AND object_type='NativeBinding' AND object_id=? AND object_version=?",&[&row[0],&receipt[0][1],&row[1],&row[2]],1)?;
    if events.len()!=1{return denied()}
    Ok(Some(value))
}

fn current_binding(tx:&mut Transaction<'_, '_>,domain:&str,binding:&str)->Result<Option<NativeBindingVersion>> {
    let rows=tx.query("SELECT generation,source_epoch,content_hash FROM main.gogoke_native_binding_heads WHERE domain_id=? AND binding_id=?",&[domain,binding],3)?;
    if rows.is_empty(){return Ok(None)}
    if rows.len()!=1{return denied()}
    let projection=tx.query("SELECT instance_id,instance_version FROM main.gogoke_native_binding_versions WHERE domain_id=? AND binding_id=? AND generation=?",&[domain,binding,&rows[0][0]],2)?;
    if projection.len()!=1{return denied()}
    let identity=NativeBindingIdentity{domain_id:domain.into(),binding_id:binding.into(),generation:rows[0][0].clone(),source_epoch:rows[0][1].clone(),instance_id:projection[0][0].clone(),instance_version:projection[0][1].clone()};
    let version=load_binding(tx,&identity)?.ok_or(OrchestrationError::AccessDenied)?;
    if version.content_hash!=rows[0][2]{return denied()}
    Ok(Some(version))
}

pub(crate) fn commit_native_binding(connection:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,input:&CommitNativeBinding)->Result<NativeBindingReceipt> {
    validate_binding(input)?;
    transaction::run(connection,|tx|{
        owner.check(&current_profile(tx)?)?;ensure_schema(tx)?;
        let instance=current_instance(tx,&input.domain_id,&input.instance_id)?.ok_or(OrchestrationError::AccessDenied)?;
        if instance.version!=input.instance_version || instance.account_ref==AccountRefSnapshot::Unknown{return denied()}
        check_lineage(tx,&input.domain_id,&input.binding_id,&input.generation,&input.source_epoch,&input.native_identity,&input.lineage_ref)?;
        let record=binding_record(input,&content_hash(&instance_bytes(&instance)))?;
        let current=current_binding(tx,&input.domain_id,&input.binding_id)?;
        let operation=tx.query("SELECT receipt_id FROM main.gogoke_receipts WHERE domain_id=? AND operation_id=?",&[&input.domain_id,&input.operation_id],1)?;
        if !operation.is_empty(){
            if operation.len()!=1 || operation[0][0]!=input.receipt_id{return Err(OrchestrationError::OperationConflict)}
            let identity=NativeBindingIdentity{domain_id:input.domain_id.clone(),binding_id:input.binding_id.clone(),generation:input.generation.clone(),source_epoch:input.source_epoch.clone(),instance_id:input.instance_id.clone(),instance_version:input.instance_version.clone()};
            let stored=load_binding(tx,&identity)?.ok_or(OrchestrationError::OperationConflict)?;
            if current.as_ref()!=Some(&stored) || stored.native_identity!=input.native_identity || stored.lineage_ref!=input.lineage_ref || stored.custody_ref!=input.custody_ref{return Err(OrchestrationError::OperationConflict)}
            let storage=tx.apply_domain_record(record)?;
            if storage.disposition!="RECONCILED"{return denied()}
            return Ok(NativeBindingReceipt{disposition:"RECONCILED",operation_id:input.operation_id.clone(),version:stored});
        }
        if current.as_ref().map(|v|v.generation.as_str())!=input.expected_previous_generation.as_deref(){return Err(OrchestrationError::OperationConflict)}
        if let Some(previous)=&input.expected_previous_generation {if checked_version(&input.generation)?!=checked_version(previous)?.checked_add(1).ok_or(OrchestrationError::OperationConflict)?{return Err(OrchestrationError::OperationConflict)}}
        else if input.generation!="1"{return Err(OrchestrationError::OperationConflict)}
        let storage=tx.apply_domain_record(record)?;
        if storage.disposition!="COMMITTED"{return denied()}
        tx.write("INSERT INTO gogoke_native_binding_versions(domain_id,binding_id,generation,object_type,source_epoch,instance_id,instance_version,native_identity,lineage_ref,custody_ref,content_hash,operation_id,receipt_id) VALUES(?,?,?,'NativeBinding',?,?,?,?,?,?,?,?,?,?)",&[&input.domain_id,&input.binding_id,&input.generation,&input.source_epoch,&input.instance_id,&input.instance_version,&input.native_identity,&input.lineage_ref,&input.custody_ref,&storage.object_hash,&input.operation_id,&input.receipt_id])?;
        if let Some(previous)=&input.expected_previous_generation {tx.write("UPDATE gogoke_native_binding_heads SET generation=?,source_epoch=?,content_hash=? WHERE domain_id=? AND binding_id=? AND generation=?",&[&input.generation,&input.source_epoch,&storage.object_hash,&input.domain_id,&input.binding_id,previous])?;}
        else {tx.write("INSERT INTO gogoke_native_binding_heads(domain_id,binding_id,generation,source_epoch,content_hash) VALUES(?,?,?,?,?)",&[&input.domain_id,&input.binding_id,&input.generation,&input.source_epoch,&storage.object_hash])?;}
        let stored=current_binding(tx,&input.domain_id,&input.binding_id)?.ok_or(OrchestrationError::AccessDenied)?;
        if stored.binding_id!=input.binding_id || stored.generation!=input.generation || stored.native_identity!=input.native_identity || stored.instance!=instance{return denied()}
        Ok(NativeBindingReceipt{disposition:"COMMITTED",operation_id:input.operation_id.clone(),version:stored})
    })
}

pub(crate) fn read_native_binding(connection:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,identity:&NativeBindingIdentity)->Result<Option<NativeBindingVersion>>{
    for field in [&identity.domain_id,&identity.binding_id,&identity.instance_id] {checked_id(field)?;}
    checked_version(&identity.generation)?;checked_version(&identity.instance_version)?;revision(&identity.source_epoch)?;
    transaction::run(connection,|tx|{owner.check(&current_profile(tx)?)?;let current=current_binding(tx,&identity.domain_id,&identity.binding_id)?;if let Some(version)=&current {if version.generation!=identity.generation || version.source_epoch!=identity.source_epoch || version.instance.instance_id!=identity.instance_id || version.instance.version!=identity.instance_version{return denied()}}Ok(current)})
}
