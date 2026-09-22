//! Canonical ContextManifest authority on the existing Product DB.
//! Operation-scoped snapshots/read bindings are comparison and replay inputs;
//! immutable Context, lifecycle, grants and canonical object/receipt tables remain
//! their existing single sources of truth. No content bytes or model generation.
use std::collections::{BTreeMap, BTreeSet};

use super::catalog::{current_profile, resolve_current};
use super::context_read::{GranteeContextReadRequest, ContextReadRequest};
use super::context_read_set::{read_grantee_context_set_in_transaction, AuthorizedContextReadSet};
use super::decision_replay::validate_public_decision;
use super::model::{denied, identifier, revision, GrantRef};
use super::task_context::read_current_task_context_in_transaction;
use super::transaction::{self, Result, Transaction};
use super::super::atomic::{canonical_object_without_string_field, DomainRecordInput};
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;

const SNAPSHOT_SCHEMA:&str="CREATE TABLE gogoke_context_assembly_snapshots (operation_id TEXT PRIMARY KEY,principal_id TEXT NOT NULL,seat_id TEXT NOT NULL,task_id TEXT NOT NULL,session_id TEXT NOT NULL,domain_id TEXT NOT NULL,binding_id TEXT NOT NULL,binding_generation TEXT NOT NULL,source_epoch TEXT NOT NULL,runtime_instance_id TEXT NOT NULL,task_revision TEXT NOT NULL,policy_revision TEXT NOT NULL,auth_revision TEXT NOT NULL,revocation_head TEXT NOT NULL,selection_decision_id TEXT NOT NULL,manifest_id TEXT NOT NULL,admission_action_operation_id TEXT NOT NULL,admission_digest TEXT NOT NULL,max_content_bytes TEXT NOT NULL,max_candidates TEXT NOT NULL) STRICT";
const BINDING_SCHEMA:&str="CREATE TABLE gogoke_context_manifest_read_bindings (operation_id TEXT NOT NULL,ordinal TEXT NOT NULL,principal_id TEXT NOT NULL,seat_id TEXT NOT NULL,source_domain_id TEXT NOT NULL,context_id TEXT NOT NULL,version TEXT NOT NULL,expected_scope TEXT NOT NULL,expected_content_hash TEXT NOT NULL,expected_access_policy_revision TEXT NOT NULL,destination_domain_id TEXT NOT NULL,destination_scope TEXT NOT NULL,promotion_kind TEXT NOT NULL,policy_revision TEXT NOT NULL,grant_id TEXT NOT NULL,grant_revision TEXT NOT NULL,grant_revocation_head TEXT NOT NULL,included TEXT NOT NULL CHECK(included IN ('0','1')),expected_state_revision TEXT NOT NULL,PRIMARY KEY(operation_id,ordinal),UNIQUE(operation_id,source_domain_id,context_id),FOREIGN KEY(operation_id) REFERENCES gogoke_context_assembly_snapshots(operation_id) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";
const PARTITION_BINDING_SCHEMA:&str="CREATE TABLE gogoke_context_assembly_partition_grant_bindings (operation_id TEXT NOT NULL,source_domain_id TEXT NOT NULL,destination_scope TEXT NOT NULL,promotion_kind TEXT NOT NULL,grant_id TEXT NOT NULL,grant_revision TEXT NOT NULL,grant_revocation_head TEXT NOT NULL,PRIMARY KEY(operation_id,source_domain_id,grant_id),FOREIGN KEY(operation_id) REFERENCES gogoke_context_assembly_snapshots(operation_id) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct ContextPartitionGrantBinding {
 pub source_domain_id:String,pub destination_scope:String,pub promotion_kind:String,pub grant:GrantRef,
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct ContextAssemblySnapshot {
 pub operation_id:String,pub principal_id:String,pub seat_id:String,pub task_id:String,pub session_id:String,
 pub domain_id:String,pub binding_id:String,pub binding_generation:String,pub source_epoch:String,pub runtime_instance_id:String,
 pub task_revision:String,pub policy_revision:String,pub auth_revision:String,pub revocation_head:String,
 pub selection_decision_id:String,pub manifest_id:String,pub admission_action_operation_id:String,pub admission_digest:String,
 pub max_content_bytes:u64,pub max_candidates:u64,pub partition_grant_bindings:Vec<ContextPartitionGrantBinding>,
}
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct ManifestExpectedVersion {
 pub source_domain_id:String,pub context_id:String,pub version:String,pub content_hash:String,
 pub state_revision:String,pub access_policy_revision:String,
}
#[derive(Clone,Debug)]
pub(crate) struct ContextManifestCommitInput {
 pub operation_id:String,pub request_digest:String,pub event_id:String,pub receipt_id:String,pub recorded_at:String,
 pub read_requests:Vec<GranteeContextReadRequest>,pub expected_versions:Vec<ManifestExpectedVersion>,
 pub canonical_manifest:Vec<u8>,
}
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct ContextManifestReplayIdentity {
 pub operation_id:String,pub principal_id:String,pub seat_id:String,pub task_id:String,pub session_id:String,
 pub domain_id:String,pub binding_id:String,pub binding_generation:String,pub source_epoch:String,pub runtime_instance_id:String,
}
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct ContextManifestAuthorityReceipt {
 pub disposition:&'static str,pub operation_id:String,pub manifest_id:String,pub manifest_hash:String,
 pub canonical_manifest:Vec<u8>,
}
fn hash(value:&str)->Result<()>{
 if value.len()!=71||!value.starts_with("sha256:")||!value.as_bytes()[7..].iter().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(b)){return denied();}Ok(())
}
fn count(value:u64)->Result<String>{if value>9_007_199_254_740_991{return denied();}Ok(value.to_string())}
fn parse_count(value:&str)->Result<u64>{
 if value.is_empty()||value.len()>16||(value.len()>1&&value.starts_with('0'))
  ||!value.bytes().all(|b|b.is_ascii_digit()){return denied();}
 let parsed=value.parse::<u64>().map_err(|_|OrchestrationError::AccessDenied)?;
 if parsed>9_007_199_254_740_991{return denied();}Ok(parsed)
}
fn ensure_schema(tx:&mut Transaction<'_, '_>)->Result<()>{
 let expected=[("gogoke_context_assembly_snapshots",SNAPSHOT_SCHEMA),("gogoke_context_manifest_read_bindings",BINDING_SCHEMA),
  ("gogoke_context_assembly_partition_grant_bindings",PARTITION_BINDING_SCHEMA)];
 let mut present=0usize;
 for (name,ddl) in expected{
  let rows=tx.query("SELECT type,sql FROM sqlite_schema WHERE name=?",&[name],2)?;
  if !rows.is_empty(){
   if rows.len()!=1||rows[0][0]!="table"||rows[0][1]!=ddl{return denied();}
   present+=1;
  }
 }
 if present==0{
  tx.write(SNAPSHOT_SCHEMA,&[])?;
  tx.write(BINDING_SCHEMA,&[])?;
  tx.write(PARTITION_BINDING_SCHEMA,&[])?;
 }else if present!=expected.len(){return denied();}
 let triggers=tx.query("SELECT name FROM sqlite_schema WHERE type='trigger' AND (tbl_name='gogoke_context_assembly_snapshots' OR tbl_name='gogoke_context_manifest_read_bindings' OR tbl_name='gogoke_context_assembly_partition_grant_bindings') LIMIT 1",&[],1)?;
 if !triggers.is_empty(){return denied();}Ok(())
}
pub(crate) fn initialize_context_manifest_schema(connection:&mut VerifiedDatabaseConnection<'_>)->Result<()>{
 transaction::run(connection,ensure_schema)
}
fn validate_snapshot(input:&ContextAssemblySnapshot)->Result<()>{
 for value in [&input.operation_id,&input.principal_id,&input.seat_id,&input.task_id,&input.session_id,&input.domain_id,
  &input.binding_id,&input.runtime_instance_id,&input.selection_decision_id,&input.manifest_id,&input.admission_action_operation_id]{identifier(value)?;}
 for value in [&input.binding_generation,&input.source_epoch,&input.task_revision,&input.policy_revision,&input.auth_revision,&input.revocation_head]{revision(value)?;}
 hash(&input.admission_digest)?;count(input.max_content_bytes)?;count(input.max_candidates)?;
 if input.max_candidates==0||input.max_candidates>64{return denied();}
 validate_partition_bindings(&input.partition_grant_bindings)?;Ok(())
}
fn validate_partition_bindings(values:&[ContextPartitionGrantBinding])->Result<BTreeMap<(String,String),ContextPartitionGrantBinding>>{
 if values.is_empty()||values.len()>64{return denied();}
 let mut out=BTreeMap::new();let mut contract:Option<(&str,&str)>=None;
 for value in values{
  identifier(&value.source_domain_id)?;identifier(&value.promotion_kind)?;value.grant.validate()?;
  if !matches!(value.destination_scope.as_str(),"GLOBAL"|"PROJECT"|"SESSION"){return denied();}
  if let Some((scope,promotion))=contract{
   if scope!=value.destination_scope||promotion!=value.promotion_kind{return denied();}
  }else{contract=Some((&value.destination_scope,&value.promotion_kind));}
  if out.insert((value.source_domain_id.clone(),value.grant.grant_id.clone()),value.clone()).is_some(){return denied();}
 }
 Ok(out)
}
fn validate_current_partition_bindings(tx:&mut Transaction<'_, '_>,snapshot:&ContextAssemblySnapshot)->Result<BTreeMap<(String,String),ContextPartitionGrantBinding>>{
 let bindings=validate_partition_bindings(&snapshot.partition_grant_bindings)?;let profile=current_profile(tx)?;
 if snapshot.policy_revision!=profile.policy_revision||snapshot.revocation_head!=profile.revocation_head{return denied();}
 for binding in bindings.values(){
  let current=resolve_current(tx,&profile,&binding.grant)?;let spec=current.spec;
  if spec.principal_id!=snapshot.principal_id||spec.seat_id!=snapshot.seat_id||spec.permission!="context.read"
   ||spec.source_domain_id!=binding.source_domain_id||spec.destination_domain_id!=snapshot.domain_id
   ||spec.destination_scope!=binding.destination_scope||spec.promotion_kind!=binding.promotion_kind{return denied();}
 }
 Ok(bindings)
}
fn stored_partition_bindings(tx:&mut Transaction<'_, '_>,operation:&str)->Result<Vec<ContextPartitionGrantBinding>>{
 let rows=tx.query("SELECT source_domain_id,destination_scope,promotion_kind,grant_id,grant_revision,grant_revocation_head FROM gogoke_context_assembly_partition_grant_bindings WHERE operation_id=? ORDER BY source_domain_id,grant_id",&[operation],6)?;
 if rows.is_empty(){return denied();}
 let values=rows.into_iter().map(|r|ContextPartitionGrantBinding{source_domain_id:r[0].clone(),destination_scope:r[1].clone(),
  promotion_kind:r[2].clone(),grant:GrantRef{grant_id:r[3].clone(),revision:r[4].clone(),revocation_head:r[5].clone()}}).collect::<Vec<_>>();
 validate_partition_bindings(&values)?;Ok(values)
}
fn verify_action(tx:&mut Transaction<'_, '_>,s:&ContextAssemblySnapshot,first:bool)->Result<()>{
 let rows=tx.query("SELECT semantic_digest,binding_id,auth_revision,generation,runtime_instance_id,session_id,state FROM gogoke_action_reservations WHERE operation_id=?",
  &[&s.admission_action_operation_id],7)?;
 if rows.len()!=1||rows[0][0]!=s.admission_digest||rows[0][1]!=s.binding_id||rows[0][2]!=s.auth_revision
  ||rows[0][3]!=s.binding_generation||rows[0][4]!=s.runtime_instance_id||rows[0][5]!=s.session_id{return denied();}
 if first&&rows[0][6]!="reserved"{return denied();}Ok(())
}
pub(crate) fn publish_context_assembly_snapshot(connection:&mut VerifiedDatabaseConnection<'_>,input:&ContextAssemblySnapshot)->Result<()>{
 validate_snapshot(input)?;
 transaction::run(connection,|tx|{
   ensure_schema(tx)?;let task=read_current_task_context_in_transaction(tx,&input.domain_id,&input.task_id)?;
   if task.task_revision!=input.task_revision||task.mandatory_refs.len()>input.max_candidates as usize{return denied();}
   let profile=current_profile(tx)?;
  if input.policy_revision!=profile.policy_revision||input.revocation_head!=profile.revocation_head{return denied();}
  let partition_bindings=validate_current_partition_bindings(tx,input)?;
  verify_action(tx,input,true)?;
  let fields=[input.principal_id.as_str(),input.seat_id.as_str(),input.task_id.as_str(),input.session_id.as_str(),input.domain_id.as_str(),
   input.binding_id.as_str(),input.binding_generation.as_str(),input.source_epoch.as_str(),input.runtime_instance_id.as_str(),
   input.task_revision.as_str(),input.policy_revision.as_str(),input.auth_revision.as_str(),input.revocation_head.as_str(),
   input.selection_decision_id.as_str(),input.manifest_id.as_str(),input.admission_action_operation_id.as_str(),input.admission_digest.as_str()];
  let rows=tx.query("SELECT principal_id,seat_id,task_id,session_id,domain_id,binding_id,binding_generation,source_epoch,runtime_instance_id,task_revision,policy_revision,auth_revision,revocation_head,selection_decision_id,manifest_id,admission_action_operation_id,admission_digest, max_content_bytes,max_candidates FROM gogoke_context_assembly_snapshots WHERE operation_id=?",&[&input.operation_id],19)?;
  let max_bytes=count(input.max_content_bytes)?;let max_candidates=count(input.max_candidates)?;
  if rows.is_empty(){
   tx.write("INSERT INTO gogoke_context_assembly_snapshots(operation_id,principal_id,seat_id,task_id,session_id,domain_id,binding_id,binding_generation,source_epoch,runtime_instance_id,task_revision,policy_revision,auth_revision,revocation_head,selection_decision_id,manifest_id,admission_action_operation_id,admission_digest,max_content_bytes,max_candidates) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    &[&input.operation_id,&input.principal_id,&input.seat_id,&input.task_id,&input.session_id,&input.domain_id,&input.binding_id,
      &input.binding_generation,&input.source_epoch,&input.runtime_instance_id,&input.task_revision,&input.policy_revision,
      &input.auth_revision,&input.revocation_head,&input.selection_decision_id,&input.manifest_id,
      &input.admission_action_operation_id,&input.admission_digest,&max_bytes,&max_candidates])?;
   for binding in partition_bindings.values(){
    tx.write("INSERT INTO gogoke_context_assembly_partition_grant_bindings(operation_id,source_domain_id,destination_scope,promotion_kind,grant_id,grant_revision,grant_revocation_head) VALUES(?,?,?,?,?,?,?)",
     &[&input.operation_id,&binding.source_domain_id,&binding.destination_scope,&binding.promotion_kind,&binding.grant.grant_id,
       &binding.grant.revision,&binding.grant.revocation_head])?;
   }
  }else{
   if rows.len()!=1{return denied();}let mut expected=fields.iter().map(|v|(*v).to_owned()).collect::<Vec<_>>();
   expected.push(max_bytes);expected.push(max_candidates);
   if rows[0]!=expected{return Err(OrchestrationError::OperationConflict);}
   let stored=stored_partition_bindings(tx,&input.operation_id)?;
   let exact=partition_bindings.into_values().collect::<Vec<_>>();
   if stored!=exact{return Err(OrchestrationError::OperationConflict);}
  }Ok(())
 })
}
fn load_snapshot(tx:&mut Transaction<'_, '_>,operation:&str)->Result<ContextAssemblySnapshot>{
 let rows=tx.query("SELECT principal_id,seat_id,task_id,session_id,domain_id,binding_id,binding_generation,source_epoch,runtime_instance_id,task_revision,policy_revision,auth_revision,revocation_head,selection_decision_id,manifest_id,admission_action_operation_id,admission_digest,max_content_bytes,max_candidates FROM gogoke_context_assembly_snapshots WHERE operation_id=?",&[operation],19)?;
 if rows.len()!=1{return denied();}let r=&rows[0];
 let partition_grant_bindings=stored_partition_bindings(tx,operation)?;
 let snapshot=ContextAssemblySnapshot{operation_id:operation.into(),principal_id:r[0].clone(),seat_id:r[1].clone(),task_id:r[2].clone(),session_id:r[3].clone(),
  domain_id:r[4].clone(),binding_id:r[5].clone(),binding_generation:r[6].clone(),source_epoch:r[7].clone(),runtime_instance_id:r[8].clone(),
  task_revision:r[9].clone(),policy_revision:r[10].clone(),auth_revision:r[11].clone(),revocation_head:r[12].clone(),
  selection_decision_id:r[13].clone(),manifest_id:r[14].clone(),admission_action_operation_id:r[15].clone(),admission_digest:r[16].clone(),
  max_content_bytes:parse_count(&r[17])?,max_candidates:parse_count(&r[18])?,partition_grant_bindings};
  validate_snapshot(&snapshot)?;
  validate_current_partition_bindings(tx,&snapshot)?;
  let task=read_current_task_context_in_transaction(tx,&snapshot.domain_id,&snapshot.task_id)?;
  if task.task_revision!=snapshot.task_revision{return denied();}
  Ok(snapshot)
}
fn expected_key(v:&ManifestExpectedVersion)->(String,String){(v.source_domain_id.clone(),v.context_id.clone())}
fn read_key(v:&GranteeContextReadRequest)->(String,String){(v.source.source_domain_id.clone(),v.source.context_id.clone())}
fn validate_expected(values:&[ManifestExpectedVersion])->Result<BTreeMap<(String,String),ManifestExpectedVersion>>{
 let mut out=BTreeMap::new();for v in values{
  identifier(&v.source_domain_id)?;if v.context_id.is_empty(){return denied();}revision(&v.version)?;hash(&v.content_hash)?;
  revision(&v.state_revision)?;revision(&v.access_policy_revision)?;
  if out.insert(expected_key(v),v.clone()).is_some(){return denied();}
 }Ok(out)
}
fn exact_keys(tx:&mut Transaction<'_, '_>,json:&str,path:Option<&str>,expected:&[&str])->Result<()>{
 let rows=match path{None=>tx.query("SELECT key FROM json_each(?)",&[json],1)?,Some(p)=>tx.query("SELECT key FROM json_each(?,?)",&[json,p],1)?};
 if rows.len()!=expected.len(){return denied();}
 let actual=rows.into_iter().map(|r|r[0].clone()).collect::<BTreeSet<_>>();
 if expected.iter().any(|key|!actual.contains(*key)){return denied();}Ok(())
}
fn jt(tx:&mut Transaction<'_, '_>,json:&str,path:&str)->Result<String>{
 let rows=tx.query("SELECT COALESCE(json_type(?,?),'missing')",&[json,path],1)?;if rows.len()!=1{return denied();}Ok(rows[0][0].clone())
}
fn jtext(tx:&mut Transaction<'_, '_>,json:&str,path:&str)->Result<String>{
 if jt(tx,json,path)?!="text"{return denied();}let rows=tx.query("SELECT json_extract(?,?)",&[json,path],1)?;
 if rows.len()!=1||rows[0][0].is_empty(){return denied();}Ok(rows[0][0].clone())
}
fn exact_object_array(
 tx:&mut Transaction<'_, '_>,json:&str,path:&str,expected:&[&str],
)->Result<Vec<String>>{
 if jt(tx,json,path)?!="array"{return denied();}
 let rows=tx.query("SELECT CAST(value AS TEXT) FROM json_each(?,?) ORDER BY CAST(key AS INTEGER)",&[json,path],1)?;
 let mut values=Vec::with_capacity(rows.len());
 for row in rows{
  if row.len()!=1{return denied();}
  exact_keys(tx,&row[0],None,expected)?;
  values.push(row[0].clone());
 }
 Ok(values)
}
pub(super) fn compare_replay_identity(s:&ContextAssemblySnapshot,id:&ContextManifestReplayIdentity)->Result<()>{
 if s.operation_id!=id.operation_id||s.principal_id!=id.principal_id||s.seat_id!=id.seat_id||s.task_id!=id.task_id
  ||s.session_id!=id.session_id||s.domain_id!=id.domain_id||s.binding_id!=id.binding_id
  ||s.binding_generation!=id.binding_generation||s.source_epoch!=id.source_epoch||s.runtime_instance_id!=id.runtime_instance_id{return denied();}Ok(())
}
pub(super) fn load_context_assembly_snapshot_in_transaction(
 tx:&mut Transaction<'_, '_>,identity:&ContextManifestReplayIdentity,
)->Result<ContextAssemblySnapshot>{
 identifier(&identity.operation_id)?;ensure_schema(tx)?;
 let snapshot=load_snapshot(tx,&identity.operation_id)?;
 compare_replay_identity(&snapshot,identity)?;verify_action(tx,&snapshot,true)?;
 Ok(snapshot)
}
fn authorized_map(set:&AuthorizedContextReadSet)->BTreeMap<(String,String),&super::context_read::AuthorizedContextReadSnapshot>{
 set.sources.iter().map(|v|((v.source.source_domain_id.clone(),v.source.context_id.clone()),v)).collect()
}
fn validate_manifest(
 tx:&mut Transaction<'_, '_>,snapshot:&ContextAssemblySnapshot,reads:&[GranteeContextReadRequest],
 expected:&[ManifestExpectedVersion],canonical:&[u8],request_digest:&str,first:bool,
)->Result<String>{
 hash(request_digest)?;verify_action(tx,snapshot,first)?;
  let task=read_current_task_context_in_transaction(tx,&snapshot.domain_id,&snapshot.task_id)?;
  if task.task_revision!=snapshot.task_revision{return denied();}
  let mandatory=task.mandatory_refs.iter().map(|value|
   (value.source_domain_id.clone(),value.context_id.clone(),value.version.clone())).collect::<BTreeSet<_>>();
  let profile=current_profile(tx)?;if profile.policy_revision!=snapshot.policy_revision||profile.revocation_head!=snapshot.revocation_head{return denied();}
 let decision=tx.query("SELECT content_hash,CAST(canonical_json AS TEXT) FROM gogoke_objects WHERE domain_id=? AND object_type='DecisionRecord' AND object_id=? AND object_version='1'",
  &[&snapshot.domain_id,&snapshot.selection_decision_id],2)?;
 if decision.len()!=1||content_hash(decision[0][1].as_bytes())!=decision[0][0]{return denied();}
 let decision_json=&decision[0][1];validate_public_decision(tx,decision_json)?;
 if jtext(tx,decision_json,"$.decisionId")?!=snapshot.selection_decision_id
  ||jtext(tx,decision_json,"$.family")?!="CONTEXT_SELECTION"
  ||jtext(tx,decision_json,"$.state")?!="COMMITTED"
  ||jtext(tx,decision_json,"$.actionId")?!=snapshot.admission_action_operation_id
  ||jtext(tx,decision_json,"$.sourceRevisions.taskRevision")?!=snapshot.task_revision
  ||jtext(tx,decision_json,"$.sourceRevisions.policyRevision")?!=snapshot.policy_revision
  ||jtext(tx,decision_json,"$.sourceRevisions.bindingGeneration")?!=snapshot.binding_generation{return denied();}
 if reads.is_empty()||reads.len()>64{return denied();}
 for read in reads{
  let r=&read.source;
  if r.destination_domain_id!=snapshot.domain_id||!snapshot.partition_grant_bindings.iter().any(|binding|
   binding.source_domain_id==r.source_domain_id&&binding.destination_scope==r.destination_scope
    &&binding.promotion_kind==r.promotion_kind&&binding.grant==r.grant){return denied();}
 }
 let set=read_grantee_context_set_in_transaction(tx,reads)?;
 if set.principal_id!=snapshot.principal_id||set.seat_id!=snapshot.seat_id||set.policy_revision!=snapshot.policy_revision
  ||set.revocation_head!=snapshot.revocation_head||set.destination_domain_id!=snapshot.domain_id{return denied();}
 let authorized=authorized_map(&set);let expected=validate_expected(expected)?;
 if expected.len()>snapshot.max_candidates as usize{return denied();}
 for (key,value) in &expected{
  let Some(current)=authorized.get(key) else{return denied();};
  if current.source.version!=value.version||current.source.content_hash!=value.content_hash
   ||current.state_revision!=value.state_revision||current.source.access_policy_revision!=value.access_policy_revision{return denied();}
 }
 let (body,manifest_hash)=canonical_object_without_string_field(canonical,"manifestHash","ContextManifest")
  .map_err(OrchestrationError::Atomic)?;
 hash(&manifest_hash)?;if content_hash(&body)!=manifest_hash{return denied();}
 let json=std::str::from_utf8(canonical).map_err(|_|OrchestrationError::AccessDenied)?;
 exact_keys(tx,json,None,&["bindingGeneration","domainId","includedVersions","manifestHash","manifestId","policyRevision",
  "redactions","requiredConstraints","seatId","selectionDecisionId","sourceSnapshot","taskId"])?;
 if jtext(tx,json,"$.manifestId")?!=snapshot.manifest_id||jtext(tx,json,"$.taskId")?!=snapshot.task_id
  ||jtext(tx,json,"$.seatId")?!=snapshot.seat_id||jtext(tx,json,"$.bindingGeneration")?!=snapshot.binding_generation
  ||jtext(tx,json,"$.domainId")?!=snapshot.domain_id||jtext(tx,json,"$.policyRevision")?!=snapshot.policy_revision
  ||jtext(tx,json,"$.selectionDecisionId")?!=snapshot.selection_decision_id{return denied();}
 if jt(tx,json,"$.redactions")?!="array"{return denied();}
 if !tx.query("SELECT 1 FROM json_each(?,'$.redactions') LIMIT 1",&[json],1)?.is_empty(){return denied();}
 exact_keys(tx,json,Some("$.sourceSnapshot"),&["assemblySchema","operationId","requestDigest","principalId","sessionId",
  "bindingId","sourceEpoch","runtimeInstanceId","taskRevision","authRevision","revocationHead","partitions","mode","excluded"])?;
 for (path,value) in [("$.sourceSnapshot.assemblySchema","gogoke.context-assembly.v1"),("$.sourceSnapshot.operationId",&snapshot.operation_id),
  ("$.sourceSnapshot.requestDigest",request_digest),("$.sourceSnapshot.principalId",&snapshot.principal_id),
  ("$.sourceSnapshot.sessionId",&snapshot.session_id),("$.sourceSnapshot.bindingId",&snapshot.binding_id),
  ("$.sourceSnapshot.sourceEpoch",&snapshot.source_epoch),("$.sourceSnapshot.runtimeInstanceId",&snapshot.runtime_instance_id),
  ("$.sourceSnapshot.taskRevision",&snapshot.task_revision),("$.sourceSnapshot.authRevision",&snapshot.auth_revision),
  ("$.sourceSnapshot.revocationHead",&snapshot.revocation_head),("$.sourceSnapshot.mode","FIXED_SOURCE_RULES")]{
   if jtext(tx,json,path)?!=value{return denied();}
 }
 let included_objects=exact_object_array(tx,json,"$.includedVersions",&["sourceDomainId","contextId","version","contentHash","stateRevision","accessPolicyRevision","reason"])?;
 let required_objects=exact_object_array(tx,json,"$.requiredConstraints",&["sourceDomainId","contextId","version","contentHash","stateRevision","accessPolicyRevision"])?;
 let partition_objects=exact_object_array(tx,json,"$.sourceSnapshot.partitions",&["sourceDomainId"])?;
 let excluded_objects=exact_object_array(tx,json,"$.sourceSnapshot.excluded",&["sourceDomainId","contextId","version","reason"])?;
 let included=tx.query("SELECT json_extract(value,'$.sourceDomainId'),json_extract(value,'$.contextId'),json_extract(value,'$.version'),json_extract(value,'$.contentHash'),json_extract(value,'$.stateRevision'),json_extract(value,'$.accessPolicyRevision'),json_extract(value,'$.reason') FROM json_each(?,'$.includedVersions') ORDER BY CAST(key AS INTEGER)",&[json],7)?;
 if included.len()!=expected.len()||included.len()!=included_objects.len(){return denied();}
 let required=tx.query("SELECT json_extract(value,'$.sourceDomainId'),json_extract(value,'$.contextId'),json_extract(value,'$.version'),json_extract(value,'$.contentHash'),json_extract(value,'$.stateRevision'),json_extract(value,'$.accessPolicyRevision') FROM json_each(?,'$.requiredConstraints') ORDER BY CAST(key AS INTEGER)",&[json],6)?;
 if required.len()!=required_objects.len(){return denied();}
  let mut required_keys=BTreeSet::new();let mut required_refs=BTreeSet::new();
  for r in required{
   if r.len()!=6{return denied();}let key=(r[0].clone(),r[1].clone());let Some(e)=expected.get(&key) else{return denied();};
   if r[2]!=e.version||r[3]!=e.content_hash||r[4]!=e.state_revision||r[5]!=e.access_policy_revision
    ||!required_keys.insert(key)||!required_refs.insert((r[0].clone(),r[1].clone(),r[2].clone())){return denied();}
  }
  if required_refs!=mandatory{return denied();}
 let mut included_keys=BTreeSet::new();
 for r in included{
  if r.len()!=7{return denied();}let key=(r[0].clone(),r[1].clone());let Some(e)=expected.get(&key) else{return denied();};
  if r[2]!=e.version||r[3]!=e.content_hash||r[4]!=e.state_revision||r[5]!=e.access_policy_revision
   ||!matches!(r[6].as_str(),"MANDATORY_CONSTRAINT"|"AUTHORIZED_RETRIEVAL")||!included_keys.insert(key.clone()){return denied();}
  if (r[6]=="MANDATORY_CONSTRAINT")!=required_keys.contains(&key){return denied();}
 }
 if included_keys.len()!=expected.len(){return denied();}
 let excluded=tx.query("SELECT json_extract(value,'$.sourceDomainId'),json_extract(value,'$.contextId'),json_extract(value,'$.version'),json_extract(value,'$.reason') FROM json_each(?,'$.sourceSnapshot.excluded') ORDER BY CAST(key AS INTEGER)",&[json],4)?;
 if excluded.len()!=excluded_objects.len(){return denied();}
 let mut disclosed=included_keys.clone();
 for r in excluded{
  if r.len()!=4||!matches!(r[3].as_str(),"NEEDS_EVIDENCE"|"NEEDS_BUDGET"|"VERSION_NOT_SELECTED"){return denied();}
  let key=(r[0].clone(),r[1].clone());let Some(current)=authorized.get(&key) else{return denied();};
  if current.source.version!=r[2]||!disclosed.insert(key){return denied();}
 }
 if disclosed.len()!=reads.len()||authorized.len()!=reads.len(){return denied();}
 let partitions=tx.query("SELECT json_extract(value,'$.sourceDomainId') FROM json_each(?,'$.sourceSnapshot.partitions') ORDER BY json_extract(value,'$.sourceDomainId')",&[json],1)?;
 if partitions.len()!=partition_objects.len(){return denied();}
 let partition_set=partitions.into_iter().map(|r|r[0].clone()).collect::<BTreeSet<_>>();
 let disclosed_domains=disclosed.iter().map(|(domain,_)|domain.clone()).collect::<BTreeSet<_>>();
 if partition_set!=disclosed_domains{return denied();}
 Ok(manifest_hash)
}
fn insert_bindings(tx:&mut Transaction<'_, '_>,operation:&str,reads:&[GranteeContextReadRequest],expected:&[ManifestExpectedVersion])->Result<()>{
 let expected=validate_expected(expected)?;
 for (index,read) in reads.iter().enumerate(){
  let key=read_key(read);let state=expected.get(&key).map(|x|x.state_revision.as_str()).unwrap_or("");
  let included=if expected.contains_key(&key){"1"}else{"0"};let ordinal=index.to_string();let r=&read.source;
  tx.write("INSERT INTO gogoke_context_manifest_read_bindings(operation_id,ordinal,principal_id,seat_id,source_domain_id,context_id,version,expected_scope,expected_content_hash,expected_access_policy_revision,destination_domain_id,destination_scope,promotion_kind,policy_revision,grant_id,grant_revision,grant_revocation_head,included,expected_state_revision) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
   &[operation,&ordinal,&read.principal_id,&read.seat_id,&r.source_domain_id,&r.context_id,&r.version,&r.expected_scope,&r.expected_content_hash,
    &r.expected_access_policy_revision,&r.destination_domain_id,&r.destination_scope,&r.promotion_kind,&r.policy_revision,
    &r.grant.grant_id,&r.grant.revision,&r.grant.revocation_head,included,state])?;
 }Ok(())
}
fn load_bindings(tx:&mut Transaction<'_, '_>,operation:&str)->Result<(Vec<GranteeContextReadRequest>,Vec<ManifestExpectedVersion>)>{
 let rows=tx.query("SELECT principal_id,seat_id,source_domain_id,context_id,version,expected_scope,expected_content_hash,expected_access_policy_revision,destination_domain_id,destination_scope,promotion_kind,policy_revision,grant_id,grant_revision,grant_revocation_head,included,expected_state_revision FROM gogoke_context_manifest_read_bindings WHERE operation_id=? ORDER BY CAST(ordinal AS INTEGER)",&[operation],17)?;
 if rows.is_empty(){return denied();}let mut reads=Vec::new();let mut expected=Vec::new();
 for r in rows{
  let request=GranteeContextReadRequest{principal_id:r[0].clone(),seat_id:r[1].clone(),source:ContextReadRequest{
   source_domain_id:r[2].clone(),context_id:r[3].clone(),version:r[4].clone(),expected_scope:r[5].clone(),expected_content_hash:r[6].clone(),
   expected_access_policy_revision:r[7].clone(),destination_domain_id:r[8].clone(),destination_scope:r[9].clone(),
   promotion_kind:r[10].clone(),policy_revision:r[11].clone(),grant:GrantRef{grant_id:r[12].clone(),revision:r[13].clone(),revocation_head:r[14].clone()}}};
  if r[15]=="1"{expected.push(ManifestExpectedVersion{source_domain_id:r[2].clone(),context_id:r[3].clone(),version:r[4].clone(),
   content_hash:r[6].clone(),state_revision:r[16].clone(),access_policy_revision:r[7].clone()});}
  else if r[15]!="0"||!r[16].is_empty(){return denied();}
  reads.push(request);
 }Ok((reads,expected))
}
struct StoredManifest {
 canonical_manifest:Vec<u8>,
 receipt_id:String,
 event_id:String,
 recorded_at:String,
 receipt_json:String,
}
fn load_stored(tx:&mut Transaction<'_, '_>,domain:&str,operation:&str)->Result<Option<StoredManifest>>{
 let rows=tx.query("SELECT CAST(o.canonical_json AS TEXT),o.content_hash,r.receipt_id,r.event_id,r.recorded_at,CAST(r.canonical_json AS TEXT),r.content_hash FROM gogoke_receipts r JOIN gogoke_objects o ON o.domain_id=r.domain_id AND o.object_type=r.object_type AND o.object_id=r.object_id AND o.object_version=r.object_version WHERE r.domain_id=? AND r.operation_id=? AND r.object_type='ContextManifest' AND r.receipt_type='ContextManifestCommitted'",
  &[domain,operation],7)?;
 if rows.is_empty(){return Ok(None);}if rows.len()!=1{return denied();}
 if content_hash(rows[0][0].as_bytes())!=rows[0][1]||content_hash(rows[0][5].as_bytes())!=rows[0][6]{return denied();}
 Ok(Some(StoredManifest{canonical_manifest:rows[0][0].as_bytes().to_vec(),receipt_id:rows[0][2].clone(),
  event_id:rows[0][3].clone(),recorded_at:rows[0][4].clone(),receipt_json:rows[0][5].clone()}))
}
fn receipt_fields(tx:&mut Transaction<'_, '_>,json:&str)->Result<(String,String)>{
 exact_keys(tx,json,None,&["manifestHash","requestDigest","schema"])?;
 if jtext(tx,json,"$.schema")?!="gogoke.context-manifest-commit.v1"{return denied();}
 let manifest_hash=jtext(tx,json,"$.manifestHash")?;let request_digest=jtext(tx,json,"$.requestDigest")?;
 hash(&manifest_hash)?;hash(&request_digest)?;Ok((manifest_hash,request_digest))
}
pub(crate) fn commit_context_manifest(connection:&mut VerifiedDatabaseConnection<'_>,input:&ContextManifestCommitInput)->Result<ContextManifestAuthorityReceipt>{
 identifier(&input.operation_id)?;identifier(&input.event_id)?;identifier(&input.receipt_id)?;hash(&input.request_digest)?;
 transaction::run(connection,|tx|{
  ensure_schema(tx)?;let snapshot=load_snapshot(tx,&input.operation_id)?;
  if let Some(stored)=load_stored(tx,&snapshot.domain_id,&input.operation_id)?{
   if stored.receipt_id!=input.receipt_id||stored.event_id!=input.event_id||stored.recorded_at!=input.recorded_at{return Err(OrchestrationError::OperationConflict);}
   let (receipt_manifest_hash,stored_digest)=receipt_fields(tx,&stored.receipt_json)?;
   if stored_digest!=input.request_digest{return Err(OrchestrationError::OperationConflict);}
   let (reads,expected)=load_bindings(tx,&input.operation_id)?;
   let manifest_hash=validate_manifest(tx,&snapshot,&reads,&expected,&stored.canonical_manifest,&stored_digest,false)?;
   if manifest_hash!=receipt_manifest_hash{return denied();}
   if reads!=input.read_requests||expected!=input.expected_versions||stored.canonical_manifest!=input.canonical_manifest{return Err(OrchestrationError::OperationConflict);}
   return Ok(ContextManifestAuthorityReceipt{disposition:"REPLAYED",operation_id:input.operation_id.clone(),manifest_id:snapshot.manifest_id,manifest_hash,canonical_manifest:stored.canonical_manifest});
  }
  let any=tx.query("SELECT 1 FROM gogoke_receipts WHERE domain_id=? AND operation_id=? LIMIT 1",&[&snapshot.domain_id,&input.operation_id],1)?;
  if !any.is_empty(){return Err(OrchestrationError::OperationConflict);}
  let manifest_hash=validate_manifest(tx,&snapshot,&input.read_requests,&input.expected_versions,&input.canonical_manifest,&input.request_digest,true)?;
  insert_bindings(tx,&input.operation_id,&input.read_requests,&input.expected_versions)?;
  let event=format!("{{\"manifestHash\":\"{}\",\"manifestId\":\"{}\",\"operationId\":\"{}\"}}",manifest_hash,snapshot.manifest_id,input.operation_id);
  let receipt=format!("{{\"manifestHash\":\"{}\",\"requestDigest\":\"{}\",\"schema\":\"gogoke.context-manifest-commit.v1\"}}",manifest_hash,input.request_digest);
  tx.apply_domain_record(DomainRecordInput{domain_id:snapshot.domain_id.clone(),object_type:"ContextManifest".into(),object_id:snapshot.manifest_id.clone(),
   object_version:"1".into(),object_bytes:input.canonical_manifest.clone(),native_identity:None,event_id:input.event_id.clone(),
   stream_id:format!("gogoke.context-manifest.v1/{}",snapshot.manifest_id),expected_previous_counter:None,counter:"0".into(),
   event_type:"ContextManifestCommitted".into(),occurred_at:input.recorded_at.clone(),event_bytes:event.into_bytes(),
   receipt_id:input.receipt_id.clone(),operation_id:input.operation_id.clone(),receipt_type:"ContextManifestCommitted".into(),
   recorded_at:input.recorded_at.clone(),receipt_bytes:receipt.into_bytes()})?;
  Ok(ContextManifestAuthorityReceipt{disposition:"COMMITTED",operation_id:input.operation_id.clone(),manifest_id:snapshot.manifest_id,
   manifest_hash,canonical_manifest:input.canonical_manifest.clone()})
 })
}
pub(crate) fn read_context_manifest(connection:&mut VerifiedDatabaseConnection<'_>,identity:&ContextManifestReplayIdentity)->Result<ContextManifestAuthorityReceipt>{
 identifier(&identity.operation_id)?;transaction::run(connection,|tx|{
  ensure_schema(tx)?;let snapshot=load_snapshot(tx,&identity.operation_id)?;compare_replay_identity(&snapshot,identity)?;
  let Some(stored)=load_stored(tx,&snapshot.domain_id,&identity.operation_id)? else{return denied();};
  let (receipt_manifest_hash,request_digest)=receipt_fields(tx,&stored.receipt_json)?;
  let (reads,expected)=load_bindings(tx,&identity.operation_id)?;
  let manifest_hash=validate_manifest(tx,&snapshot,&reads,&expected,&stored.canonical_manifest,&request_digest,false)?;
  if manifest_hash!=receipt_manifest_hash{return denied();}
  Ok(ContextManifestAuthorityReceipt{disposition:"REPLAYED",operation_id:identity.operation_id.clone(),manifest_id:snapshot.manifest_id,
   manifest_hash,canonical_manifest:stored.canonical_manifest})
 })
}

/// Resolve a manifest from the canonical Product DB record while remaining in
/// the caller's authority transaction. The returned hash is derived from the
/// validated immutable object and receipt, never from caller input.
pub(super) fn resolve_current_manifest_in_transaction(
 tx:&mut Transaction<'_, '_>,domain_id:&str,manifest_id:&str,
)->Result<String>{
 identifier(domain_id)?;identifier(manifest_id)?;ensure_schema(tx)?;
 let rows=tx.query("SELECT r.operation_id FROM main.gogoke_objects o JOIN main.gogoke_receipts r ON r.domain_id=o.domain_id AND r.object_type=o.object_type AND r.object_id=o.object_id AND r.object_version=o.object_version WHERE o.domain_id=? AND o.object_type='ContextManifest' AND o.object_id=? AND o.object_version='1' AND r.receipt_type='ContextManifestCommitted'",&[domain_id,manifest_id],1)?;
 if rows.len()!=1{return denied();}
 let operation=&rows[0][0];let snapshot=load_snapshot(tx,operation)?;
 if snapshot.domain_id!=domain_id||snapshot.manifest_id!=manifest_id{return denied();}
 let Some(stored)=load_stored(tx,domain_id,operation)? else{return denied();};
 let (receipt_hash,request_digest)=receipt_fields(tx,&stored.receipt_json)?;
 let (reads,expected)=load_bindings(tx,operation)?;
 let resolved=validate_manifest(tx,&snapshot,&reads,&expected,&stored.canonical_manifest,&request_digest,false)?;
 if resolved!=receipt_hash{return denied();}
 Ok(resolved)
}
