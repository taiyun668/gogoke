//! Private Product Authority facts and capacity leases for Decision commit.
//! This is persistence for the one main-service scheduler, not a second scheduler.
//! No ranking, dispatch, model call or external I/O occurs here.
use super::catalog::current_profile;
use super::model::{denied, identifier, revision};
use super::transaction::{self, Result, Transaction};
use super::super::atomic::Statement;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;

const SNAPSHOT_SCHEMA:&str="CREATE TABLE gogoke_decision_authority_snapshots (operation_id TEXT NOT NULL,candidate_id TEXT NOT NULL,state_view_hash TEXT NOT NULL,candidate_hash TEXT NOT NULL,task_revision TEXT NOT NULL,policy_revision TEXT NOT NULL,capability_revision TEXT NOT NULL,binding_id TEXT NOT NULL,binding_generation TEXT NOT NULL,auth_revision TEXT NOT NULL,resource_ref TEXT NOT NULL,resource_revision TEXT NOT NULL,capacity_total INTEGER NOT NULL CHECK(capacity_total>=0),action_operation_id TEXT NOT NULL,action_digest TEXT NOT NULL,PRIMARY KEY(operation_id,candidate_id)) STRICT";
const POOL_SCHEMA:&str="CREATE TABLE gogoke_decision_capacity_pools (resource_ref TEXT PRIMARY KEY,revision TEXT NOT NULL,total_units INTEGER NOT NULL CHECK(total_units>=0),reserved_units INTEGER NOT NULL CHECK(reserved_units>=0 AND reserved_units<=total_units)) STRICT";
const LEASE_SCHEMA:&str="CREATE TABLE gogoke_decision_capacity_leases (resource_reservation_ref TEXT PRIMARY KEY,operation_id TEXT NOT NULL UNIQUE,candidate_id TEXT NOT NULL,resource_ref TEXT NOT NULL,resource_revision TEXT NOT NULL,units INTEGER NOT NULL CHECK(units>=0),action_operation_id TEXT NOT NULL,FOREIGN KEY(resource_ref) REFERENCES gogoke_decision_capacity_pools(resource_ref) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";
const R2_RELEASE_SCHEMA:&str="CREATE TABLE gogoke_coordination_r2_capacity_releases (lease_ref TEXT PRIMARY KEY,action_operation_id TEXT NOT NULL UNIQUE,receipt_ref TEXT NOT NULL,evidence_hash TEXT NOT NULL,stop_proof_hash TEXT NOT NULL,FOREIGN KEY(lease_ref) REFERENCES gogoke_decision_capacity_leases(resource_reservation_ref) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";
const MAX_SAFE:u64=9_007_199_254_740_991;

#[derive(Clone,Debug)]
pub(crate) struct DecisionAuthoritySnapshot {
 pub operation_id:String,pub candidate_id:String,pub state_view_hash:String,pub candidate_hash:String,
 pub task_revision:String,pub policy_revision:String,pub capability_revision:String,
 pub binding_id:String,pub binding_generation:String,pub auth_revision:String,
 pub resource_ref:String,pub resource_revision:String,pub capacity_total:u64,
 pub action_operation_id:String,pub action_digest:String,
}
#[derive(Clone,Debug)]
pub(crate) struct DecisionCapacityRequest {
 pub operation_id:String,pub candidate_id:String,pub state_view_hash:String,pub candidate_hash:String,
 pub task_revision:String,pub policy_revision:String,pub capability_revision:String,
 pub binding_generation:String,pub resource_reservation_ref:String,pub required_units:u64,
 pub action_operation_id:String,
}
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) enum DecisionCapacityDisposition{Reserved,Replay}

fn hash(value:&str)->Result<()>{
 if value.len()!=71||!value.starts_with("sha256:")||!value.as_bytes()[7..].iter().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(b)){return denied();} Ok(())
}
fn units(value:u64)->Result<i64>{if value>MAX_SAFE{return denied();}Ok(value as i64)}
fn exec(tx:&mut Transaction<'_, '_>,sql:&str)->Result<()>{tx.write(sql,&[])}
fn schema(tx:&mut Transaction<'_, '_>,name:&str,expected:&str)->Result<bool>{
 let rows=tx.query("SELECT type,sql FROM sqlite_schema WHERE name=?",&[name],2)?;
 if rows.is_empty(){return Ok(false);}
 if rows.len()!=1||rows[0][0]!="table"||rows[0][1]!=expected{return denied();}
 Ok(true)
}
fn ensure_schema(tx:&mut Transaction<'_, '_>)->Result<()>{
 for (name,ddl) in [("gogoke_decision_authority_snapshots",SNAPSHOT_SCHEMA),
  ("gogoke_decision_capacity_pools",POOL_SCHEMA),("gogoke_decision_capacity_leases",LEASE_SCHEMA),
  ("gogoke_coordination_r2_capacity_releases",R2_RELEASE_SCHEMA)]{
   if !schema(tx,name,ddl)?{exec(tx,ddl)?;}
 }
 let main_triggers=tx.query("SELECT name FROM main.sqlite_schema WHERE type='trigger' AND (tbl_name GLOB 'gogoke_decision_*' OR tbl_name='gogoke_coordination_r2_capacity_releases') LIMIT 1",&[],1)?;
 let temp_triggers=tx.query("SELECT name FROM temp.sqlite_schema WHERE type='trigger' AND (tbl_name GLOB 'gogoke_decision_*' OR tbl_name='gogoke_coordination_r2_capacity_releases') LIMIT 1",&[],1)?;
 if !main_triggers.is_empty()||!temp_triggers.is_empty(){return denied();} Ok(())
}
pub(crate) fn initialize_decision_capacity_schema(connection:&mut VerifiedDatabaseConnection<'_>)->Result<()>{
 transaction::run(connection,ensure_schema)
}
fn validate_snapshot(input:&DecisionAuthoritySnapshot)->Result<()>{
 for value in [&input.operation_id,&input.candidate_id,&input.binding_id,&input.resource_ref,&input.action_operation_id]{identifier(value)?;}
 for value in [&input.task_revision,&input.policy_revision,&input.capability_revision,&input.binding_generation,&input.auth_revision,&input.resource_revision]{revision(value)?;}
 hash(&input.state_view_hash)?;hash(&input.candidate_hash)?;hash(&input.action_digest)?;units(input.capacity_total)?;Ok(())
}
pub(crate) fn publish_decision_snapshot(connection:&mut VerifiedDatabaseConnection<'_>,input:&DecisionAuthoritySnapshot)->Result<()>{
 validate_snapshot(input)?;
 transaction::run(connection,|tx|{
  ensure_schema(tx)?;
  let profile=current_profile(tx)?;
  if input.policy_revision!=profile.policy_revision{return denied();}
  let pool=tx.query("SELECT revision,CAST(total_units AS TEXT),CAST(reserved_units AS TEXT) FROM gogoke_decision_capacity_pools WHERE resource_ref=?",&[&input.resource_ref],3)?;
  if pool.is_empty(){
   tx.write("INSERT INTO gogoke_decision_capacity_pools(resource_ref,revision,total_units,reserved_units) VALUES(?,?,?,0)",
    &[&input.resource_ref,&input.resource_revision,&input.capacity_total.to_string()])?;
  }else if pool.len()==1{
   let current=revision(&pool[0][0])?;let next=revision(&input.resource_revision)?;
   let reserved=pool[0][2].parse::<u64>().map_err(|_|OrchestrationError::AccessDenied)?;
   if next<current||reserved>input.capacity_total{return denied();}
   if next==current {
    if pool[0][1]!=input.capacity_total.to_string(){return denied();}
   } else {
    tx.write("UPDATE gogoke_decision_capacity_pools SET revision=?,total_units=? WHERE resource_ref=? AND revision=?",
      &[&input.resource_revision,&input.capacity_total.to_string(),&input.resource_ref,&pool[0][0]])?;
   }
  }else{return denied();}
  let existing=tx.query("SELECT state_view_hash,candidate_hash,task_revision,policy_revision,capability_revision,binding_id,binding_generation,auth_revision,resource_ref,resource_revision,CAST(capacity_total AS TEXT),action_operation_id,action_digest FROM gogoke_decision_authority_snapshots WHERE operation_id=? AND candidate_id=?",
   &[&input.operation_id,&input.candidate_id],13)?;
  let expected=vec![input.state_view_hash.clone(),input.candidate_hash.clone(),input.task_revision.clone(),input.policy_revision.clone(),
   input.capability_revision.clone(),input.binding_id.clone(),input.binding_generation.clone(),input.auth_revision.clone(),
   input.resource_ref.clone(),input.resource_revision.clone(),input.capacity_total.to_string(),input.action_operation_id.clone(),input.action_digest.clone()];
  if existing.is_empty(){
   tx.write("INSERT INTO gogoke_decision_authority_snapshots(operation_id,candidate_id,state_view_hash,candidate_hash,task_revision,policy_revision,capability_revision,binding_id,binding_generation,auth_revision,resource_ref,resource_revision,capacity_total,action_operation_id,action_digest) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    &[&input.operation_id,&input.candidate_id,&input.state_view_hash,&input.candidate_hash,&input.task_revision,&input.policy_revision,
      &input.capability_revision,&input.binding_id,&input.binding_generation,&input.auth_revision,&input.resource_ref,&input.resource_revision,
      &input.capacity_total.to_string(),&input.action_operation_id,&input.action_digest])?;
  }else if existing.len()!=1||existing[0]!=expected{return denied();}
  Ok(())
 })
}
fn reserve_in_transaction(tx:&mut Transaction<'_, '_>,request:&DecisionCapacityRequest)->Result<DecisionCapacityDisposition>{
 ensure_schema(tx)?;
 for value in [&request.operation_id,&request.candidate_id,&request.resource_reservation_ref,&request.action_operation_id]{identifier(value)?;}
 for value in [&request.task_revision,&request.policy_revision,&request.capability_revision,&request.binding_generation]{revision(value)?;}
 hash(&request.state_view_hash)?;hash(&request.candidate_hash)?;let required=units(request.required_units)?;
 let profile=current_profile(tx)?;if request.policy_revision!=profile.policy_revision{return denied();}
 let rows=tx.query("SELECT state_view_hash,candidate_hash,task_revision,policy_revision,capability_revision,binding_id,binding_generation,auth_revision,resource_ref,resource_revision,CAST(capacity_total AS TEXT),action_operation_id,action_digest FROM gogoke_decision_authority_snapshots WHERE operation_id=? AND candidate_id=?",
  &[&request.operation_id,&request.candidate_id],13)?;
 if rows.len()!=1{return denied();}let row=&rows[0];
 if row[0]!=request.state_view_hash||row[1]!=request.candidate_hash||row[2]!=request.task_revision||row[3]!=request.policy_revision
  ||row[4]!=request.capability_revision||row[6]!=request.binding_generation||row[11]!=request.action_operation_id{return denied();}
 let existing=tx.query("SELECT candidate_id,resource_ref,resource_revision,CAST(units AS TEXT),action_operation_id,resource_reservation_ref FROM gogoke_decision_capacity_leases WHERE operation_id=?",
  &[&request.operation_id],6)?;
 if !existing.is_empty(){
  if existing.len()==1&&existing[0][0]==request.candidate_id&&existing[0][1]==row[8]&&existing[0][2]==row[9]
   &&existing[0][3]==required.to_string()&&existing[0][4]==request.action_operation_id&&existing[0][5]==request.resource_reservation_ref{
    // Decision/capacity is committed before Action prepare. Replay validates
    // its immutable intended Action coordinates without requiring Action state.
    return Ok(DecisionCapacityDisposition::Replay);
  }
  return Err(OrchestrationError::OperationConflict);
 }
 let pool=tx.query("SELECT revision,CAST(total_units AS TEXT),CAST(reserved_units AS TEXT) FROM gogoke_decision_capacity_pools WHERE resource_ref=?",&[&row[8]],3)?;
 if pool.len()!=1||pool[0][0]!=row[9]||pool[0][1]!=row[10]{return denied();}
 let total=pool[0][1].parse::<u64>().map_err(|_|OrchestrationError::AccessDenied)?;
 let reserved=pool[0][2].parse::<u64>().map_err(|_|OrchestrationError::AccessDenied)?;
 if required as u64>total.saturating_sub(reserved){return denied();}
 let next=reserved+required as u64;
 tx.write("UPDATE gogoke_decision_capacity_pools SET reserved_units=? WHERE resource_ref=? AND revision=? AND reserved_units=?",
  &[&next.to_string(),&row[8],&row[9],&reserved.to_string()])?;
 let verify=tx.query("SELECT CAST(reserved_units AS TEXT) FROM gogoke_decision_capacity_pools WHERE resource_ref=?",&[&row[8]],1)?;
 if verify.len()!=1||verify[0][0]!=next.to_string(){return Err(OrchestrationError::OperationConflict);}
 tx.write("INSERT INTO gogoke_decision_capacity_leases(resource_reservation_ref,operation_id,candidate_id,resource_ref,resource_revision,units,action_operation_id) VALUES(?,?,?,?,?,?,?)",
  &[&request.resource_reservation_ref,&request.operation_id,&request.candidate_id,&row[8],&row[9],&required.to_string(),&request.action_operation_id])?;
 Ok(DecisionCapacityDisposition::Reserved)
}
pub(super) fn reserve_decision_capacity_in_transaction(tx:&mut Transaction<'_, '_>,request:&DecisionCapacityRequest)->Result<DecisionCapacityDisposition>{
 reserve_in_transaction(tx,request)
}

/// Settles only the two bounded R2 fixture leases after a native completion
/// receipt, exact transport evidence and durable STOPPED custody all agree.
/// The release row makes completion replay unable to subtract capacity twice.
pub(super) fn settle_r2_test_capacity_after_completion(
 tx:&mut Transaction<'_, '_>,domain:&str,action_id:&str,receipt_ref:&str,evidence_hash:&str,
)->Result<()> {
 if domain!="domain-r2-02-test" { return Ok(()); }
 let (decision_id,candidate_id,lease_ref)=match action_id {
  "opr_22222222222222222222222222222222" =>
   ("decision-r2-02-test","candidate-r2-02-fixture","capacity-lease-r2-02"),
  "opr_33333333333333333333333333333333" =>
   ("decision-r2-03-test","candidate-r2-03-fixture","capacity-lease-r2-03"),
  _ => return Ok(()),
 };
 ensure_schema(tx)?;
 hash(evidence_hash)?;
 let completion=tx.query("SELECT trusted_receipt_ref,evidence_hash,disposition FROM main.gogoke_action_completion_receipts WHERE domain_id=? AND operation_id=?",&[domain,action_id],3)?;
 if completion.len()!=1||completion[0][0]!=receipt_ref||completion[0][1]!=evidence_hash||completion[0][2]!="COMPLETED" { return denied(); }
 let transport=tx.query("SELECT receipt_ref,evidence_hash,stop_proof_hash,pid,creation_time_100ns,binary_digest_sha256 FROM main.gogoke_action_transport_evidence WHERE domain_id=? AND operation_id=?",&[domain,action_id],6)?;
 if transport.is_empty() { return Ok(()); } // Legacy receipt has no recoverable transport.
 if transport.len()!=1||transport[0][0]!=receipt_ref||transport[0][1]!=evidence_hash { return denied(); }
 let custody=tx.query("SELECT state,stop_proof_hash,pid,creation_time_100ns,binary_digest_sha256,domain_id FROM main.gogoke_coordination_process_custody WHERE operation_id=?",&[action_id],6)?;
 if custody.len()!=1||custody[0][0]!="STOPPED"||custody[0][1]!=transport[0][2]
  ||custody[0][2]!=transport[0][3]||custody[0][3]!=transport[0][4]
  ||custody[0][4]!=transport[0][5]||custody[0][5]!=domain { return denied(); }
 let lease=tx.query("SELECT operation_id,candidate_id,resource_ref,resource_revision,CAST(units AS TEXT) FROM main.gogoke_decision_capacity_leases WHERE resource_reservation_ref=? AND action_operation_id=?",&[lease_ref,action_id],5)?;
 if lease.len()!=1||lease[0][0]!=decision_id||lease[0][1]!=candidate_id
  ||lease[0][2]!="capacity-r2-02-fixture"||lease[0][3]!="1"||lease[0][4]!="1" { return denied(); }
 let released=tx.query("SELECT action_operation_id,receipt_ref,evidence_hash,stop_proof_hash FROM main.gogoke_coordination_r2_capacity_releases WHERE lease_ref=?",&[lease_ref],4)?;
 if !released.is_empty() {
  if released.len()!=1||released[0][0]!=action_id||released[0][1]!=receipt_ref
   ||released[0][2]!=evidence_hash||released[0][3]!=transport[0][2] { return denied(); }
  return Ok(());
 }
 let pool=tx.query("SELECT revision,CAST(total_units AS TEXT),CAST(reserved_units AS TEXT) FROM main.gogoke_decision_capacity_pools WHERE resource_ref='capacity-r2-02-fixture'",&[],3)?;
 if pool.len()!=1||pool[0][0]!="1"||pool[0][1]!="1"||pool[0][2]!="1" { return denied(); }
 tx.write("UPDATE main.gogoke_decision_capacity_pools SET reserved_units=0 WHERE resource_ref='capacity-r2-02-fixture' AND revision='1' AND total_units=1 AND reserved_units=1",&[])?;
 let changed=tx.query("SELECT changes()",&[],1)?;
 if changed.len()!=1||changed[0][0]!="1" { return Err(OrchestrationError::OperationConflict); }
 tx.write("INSERT INTO main.gogoke_coordination_r2_capacity_releases(lease_ref,action_operation_id,receipt_ref,evidence_hash,stop_proof_hash) VALUES(?,?,?,?,?)",&[lease_ref,action_id,receipt_ref,evidence_hash,&transport[0][2]])?;
 let settled=tx.query("SELECT CAST(reserved_units AS TEXT) FROM main.gogoke_decision_capacity_pools WHERE resource_ref='capacity-r2-02-fixture'",&[],1)?;
 if settled.len()!=1||settled[0][0]!="0" { return Err(OrchestrationError::OperationConflict); }
 Ok(())
}
