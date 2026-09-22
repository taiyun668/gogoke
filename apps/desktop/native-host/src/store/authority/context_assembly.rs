//! Safe pre-search Context assembly reads. These operations expose only the
//! durable basis and currently authorized metadata relation; search remains a
//! pure caller-side operation over that bounded relation.
use std::collections::BTreeMap;

use super::catalog::current_profile;
use super::context_manifest::{
    load_context_assembly_snapshot_in_transaction, ContextManifestReplayIdentity,
    ContextPartitionGrantBinding,
};
use super::context_read::read_acl_allows;
use super::model::{denied, identifier, revision};
use super::task_context::{read_current_task_context_in_transaction, MandatoryContextRef};
use super::transaction::{self, Result, Transaction};
use super::super::same_open::VerifiedDatabaseConnection;

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct ContextAssemblyBasis {
 pub operation_id:String,pub binding_generation:String,pub source_epoch:String,pub task_revision:String,
 pub policy_revision:String,pub auth_revision:String,pub revocation_head:String,pub max_content_bytes:u64,
 pub max_candidates:u64,pub partition_grant_bindings:Vec<ContextPartitionGrantBinding>,
 pub mandatory_refs:Vec<MandatoryContextRef>,
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct ContextAssemblySource {
 pub source_domain_id:String,pub context_id:String,pub version:String,pub scope:String,pub kind:String,
 pub content_hash:String,pub source_ref:String,pub source_hash:String,pub source_authority_kind:String,
 pub source_authority_ref:String,pub access_policy_revision:String,pub state_revision:String,
 pub grant_id:String,pub grant_revision:String,pub grant_revocation_head:String,
}

fn context_text(value:&str)->bool{
 !value.is_empty()&&value.len()<=256&&value.bytes().all(|x|x.is_ascii_alphanumeric()||b"._:/-@".contains(&x))
}
fn hash(value:&str)->bool{
 value.len()==71&&value.starts_with("sha256:")
  &&value.as_bytes()[7..].iter().all(|x|x.is_ascii_digit()||(b'a'..=b'f').contains(x))
}
fn validate_source(source:&ContextAssemblySource)->Result<()>{
 if !context_text(&source.context_id)||revision(&source.version).is_err()
  ||!matches!(source.scope.as_str(),"GLOBAL"|"PROJECT"|"SESSION")
  ||!context_text(&source.kind)||!hash(&source.content_hash)||!context_text(&source.source_ref)
  ||!hash(&source.source_hash)||!context_text(&source.source_authority_kind)
  ||!context_text(&source.source_authority_ref)||revision(&source.access_policy_revision).is_err()
  ||revision(&source.state_revision).is_err()||identifier(&source.grant_id).is_err()
  ||revision(&source.grant_revision).is_err()||revision(&source.grant_revocation_head).is_err(){return denied();}
 Ok(())
}

pub(crate) fn read_context_assembly_basis(
 connection:&mut VerifiedDatabaseConnection<'_>,identity:&ContextManifestReplayIdentity,
)->Result<ContextAssemblyBasis>{
 transaction::run(connection,|tx|{
  let snapshot=load_context_assembly_snapshot_in_transaction(tx,identity)?;
  let task=read_current_task_context_in_transaction(tx,&snapshot.domain_id,&snapshot.task_id)?;
  Ok(ContextAssemblyBasis{operation_id:snapshot.operation_id,binding_generation:snapshot.binding_generation,
   source_epoch:snapshot.source_epoch,task_revision:snapshot.task_revision,policy_revision:snapshot.policy_revision,
   auth_revision:snapshot.auth_revision,revocation_head:snapshot.revocation_head,max_content_bytes:snapshot.max_content_bytes,
   max_candidates:snapshot.max_candidates,partition_grant_bindings:snapshot.partition_grant_bindings,
   mandatory_refs:task.mandatory_refs})
 })
}

fn scan_domain(
 tx:&mut Transaction<'_, '_>,domain:&str,bindings:&[ContextPartitionGrantBinding],
 profile:&super::bootstrap::Profile,principal:&str,seat:&str,
 out:&mut BTreeMap<(String,String,String),ContextAssemblySource>,
)->Result<()>{
 let mut last_context=String::new();let mut last_version=String::new();
 loop{
  let rows=tx.query(
   "SELECT v.context_id,v.version,v.scope,v.kind,v.content_hash,v.source_ref,v.source_hash,v.source_authority_kind,v.source_authority_ref,v.access_policy_revision,a.visibility,a.read_grant_refs,s.state FROM gogoke_context_versions v JOIN gogoke_context_states s ON s.domain_id=v.domain_id AND s.version_ref=v.context_id||'@'||v.version JOIN gogoke_context_access a ON a.domain_id=v.domain_id AND a.version_ref=s.version_ref WHERE v.domain_id=? AND s.state='ACTIVE' AND (?='' OR v.context_id>? OR (v.context_id=? AND v.version>?)) ORDER BY v.context_id,v.version LIMIT 64",
   &[domain,&last_context,&last_context,&last_context,&last_version],13)?;
  if rows.is_empty(){break;}
  let page_len=rows.len();
  for row in rows{
   if row.len()!=13||row[12]!="ACTIVE"{return denied();}
   let version_ref=format!("{}@{}",row[0],row[1]);let lifecycle=tx.context_state(domain,&version_ref)?;
   if lifecycle.state!="ACTIVE"{return denied();}
   let mut authorized_binding=None;
   for binding in bindings{
    if read_acl_allows(&row[10],&row[11],&binding.grant.grant_id,profile,principal,seat)?{
     authorized_binding=Some(binding);break;
    }
   }
   let Some(authorized_binding)=authorized_binding else{last_context=row[0].clone();last_version=row[1].clone();continue;};
   let source=ContextAssemblySource{source_domain_id:domain.into(),context_id:row[0].clone(),version:row[1].clone(),
    scope:row[2].clone(),kind:row[3].clone(),content_hash:row[4].clone(),source_ref:row[5].clone(),source_hash:row[6].clone(),
    source_authority_kind:row[7].clone(),source_authority_ref:row[8].clone(),access_policy_revision:row[9].clone(),
    state_revision:lifecycle.revision,grant_id:authorized_binding.grant.grant_id.clone(),
    grant_revision:authorized_binding.grant.revision.clone(),grant_revocation_head:authorized_binding.grant.revocation_head.clone()};
   validate_source(&source)?;
   let key=(source.source_domain_id.clone(),source.context_id.clone(),source.version.clone());
   if let Some(prior)=out.insert(key,source.clone()){
    if prior!=source{return denied();}
   }
   last_context=source.context_id;last_version=source.version;
  }
  if page_len<64{break;}
 }
 Ok(())
}

pub(crate) fn list_context_assembly_sources(
 connection:&mut VerifiedDatabaseConnection<'_>,identity:&ContextManifestReplayIdentity,
)->Result<Vec<ContextAssemblySource>>{
 transaction::run(connection,|tx|{
  let snapshot=load_context_assembly_snapshot_in_transaction(tx,identity)?;let profile=current_profile(tx)?;
  let task=read_current_task_context_in_transaction(tx,&snapshot.domain_id,&snapshot.task_id)?;
  let mut domains:BTreeMap<String,Vec<ContextPartitionGrantBinding>>=BTreeMap::new();
  for binding in &snapshot.partition_grant_bindings{
   domains.entry(binding.source_domain_id.clone()).or_default().push(binding.clone());
  }
  let mut authorized=BTreeMap::new();
  for (domain,bindings) in domains{
   scan_domain(tx,&domain,&bindings,&profile,&snapshot.principal_id,&snapshot.seat_id,&mut authorized)?;
  }
  let mut selected=Vec::new();let mut mandatory_keys=BTreeMap::new();
  for reference in &task.mandatory_refs{
   let key=(reference.source_domain_id.clone(),reference.context_id.clone(),reference.version.clone());
   let Some(source)=authorized.get(&key) else{return denied();};
   mandatory_keys.insert(key,());selected.push(source.clone());
  }
  for (key,source) in authorized{
   if selected.len()>=snapshot.max_candidates as usize{break;}
   if !mandatory_keys.contains_key(&key){selected.push(source);}
  }
  Ok(selected)
 })
}
