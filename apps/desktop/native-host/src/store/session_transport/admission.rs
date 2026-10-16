//! Native H admission ledger. App-server output is never an authority source.
//! All entrypoints here require the existing verified connection; product v37
//! ingress and a native per-seat caller issuer are still unwired.

use crate::store::atomic::{AtomicError, Statement};
use crate::store::same_open::{SameOpenError, VerifiedDatabaseConnection};

#[derive(Debug)]
pub(crate) enum AdmissionError {
    Invalid(&'static str),
    Denied,
    Conflict,
    Stale,
    Unknown,
    UnsupportedCapacity,
    CapacityExceeded { project_active: i64, project_cap: i64, instance_active: i64, instance_cap: i64 },
    Identity(crate::store::orchestration::OrchestrationError),
    Seat(crate::store::seat::SeatError),
    ProjectCapacity(crate::store::seat::SeatError),
    InstanceCapacity(crate::store::orchestration::OrchestrationError),
    Catalog(crate::store::instance::CatalogError),
    ProgramSource(crate::store::instance::ProgramSourceError),
    Store(AtomicError),
    Relationship(super::session_binding::BindingError),
    Sqlite(SameOpenError),
    CommitUnknown(SameOpenError),
    RollbackUnknown(SameOpenError),
    Native(String),
}
impl From<AtomicError> for AdmissionError {
    fn from(error: AtomicError) -> Self {
        Self::Store(error)
    }
}
impl From<SameOpenError> for AdmissionError {
    fn from(error: SameOpenError) -> Self {
        Self::Sqlite(error)
    }
}

const SCHEMA: [(&str, &str); 10] = [
    ("gogoke_v37_h_owner_binding",
     "CREATE TABLE gogoke_v37_h_owner_binding(binding_id TEXT PRIMARY KEY,instance_id TEXT NOT NULL,domain_id TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('SESSION','CALL')),owner_id TEXT NOT NULL,generation TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('ACTIVE','REVOKED')),UNIQUE(instance_id,domain_id,kind,owner_id,generation)) STRICT"),
    ("gogoke_v37_h_claim",
     "CREATE TABLE gogoke_v37_h_claim(domain_id TEXT NOT NULL,session_id TEXT NOT NULL,instance_id TEXT NOT NULL,home_id TEXT NOT NULL,binding_id TEXT NOT NULL REFERENCES gogoke_v37_h_owner_binding(binding_id),generation TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('RESERVED','COMMITTED','STOPPED','UNKNOWN','RELEASED')),revision INTEGER NOT NULL CHECK(revision >= 1),process_operation_id TEXT UNIQUE,stop_fact_id TEXT,PRIMARY KEY(domain_id,session_id)) STRICT"),
    ("gogoke_v37_h_operation",
     "CREATE TABLE gogoke_v37_h_operation(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,raw_hex TEXT NOT NULL,operation TEXT NOT NULL,session_id TEXT NOT NULL,status TEXT NOT NULL CHECK(status IN ('APPLIED','UNKNOWN')),previous_revision INTEGER NOT NULL,revision INTEGER NOT NULL,PRIMARY KEY(domain_id,request_id)) STRICT"),
    ("gogoke_v37_h_home_fence",
     "CREATE TABLE gogoke_v37_h_home_fence(home_id TEXT PRIMARY KEY,instance_id TEXT NOT NULL,domain_id TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('SESSION','CALL')),owner_id TEXT NOT NULL,generation TEXT NOT NULL,fence_id TEXT NOT NULL UNIQUE) STRICT"),
    ("gogoke_v37_h_stdin_journal",
     "CREATE TABLE gogoke_v37_h_stdin_journal(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,operation TEXT NOT NULL,ticket TEXT NOT NULL,process_operation_id TEXT NOT NULL,custodian_nonce TEXT NOT NULL,session_id TEXT NOT NULL,generation TEXT NOT NULL,request_hex TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('PREPARED','UNKNOWN','RECEIPTED')),receipt_hex TEXT,receipt_status TEXT CHECK(receipt_status IS NULL OR receipt_status IN ('APPLIED','REPLAYED','DENIED','STALE','CONFLICT','UNSUPPORTED','UNKNOWN','FAILED')),expected_revision TEXT NOT NULL,receipt_previous_revision TEXT,receipt_revision TEXT,PRIMARY KEY(domain_id,request_id),CHECK((phase='PREPARED' AND receipt_hex IS NULL AND receipt_status IS NULL AND receipt_previous_revision IS NULL AND receipt_revision IS NULL) OR (phase='UNKNOWN' AND ((receipt_hex IS NULL AND receipt_status IS NULL AND receipt_previous_revision IS NULL AND receipt_revision IS NULL) OR (receipt_hex IS NOT NULL AND receipt_status='UNKNOWN' AND receipt_previous_revision IS NOT NULL AND receipt_revision IS NOT NULL))) OR (phase='RECEIPTED' AND receipt_hex IS NOT NULL AND receipt_status IS NOT NULL AND receipt_status<>'UNKNOWN' AND receipt_previous_revision IS NOT NULL AND receipt_revision IS NOT NULL))) STRICT"),
    ("gogoke_v37_h_seat_binding",
     "CREATE TABLE gogoke_v37_h_seat_binding(domain_id TEXT NOT NULL,session_id TEXT NOT NULL,seat_id TEXT NOT NULL,seat_incarnation TEXT NOT NULL,generation TEXT NOT NULL,PRIMARY KEY(domain_id,session_id),UNIQUE(domain_id,seat_incarnation,generation),FOREIGN KEY(domain_id,seat_id) REFERENCES gogoke_v37_seats(domain_id,seat_id)) STRICT"),
    ("gogoke_v37_h_process_episode", super::episodes::PROCESS_SCHEMA),
    ("gogoke_v37_h_generation", super::episodes::GENERATION_SCHEMA),
    ("gogoke_v37_h_generation_change", super::generation_change::SCHEMA),
    ("gogoke_v37_h_dos_alias",
     "CREATE TABLE gogoke_v37_h_dos_alias(domain_id TEXT NOT NULL,session_id TEXT NOT NULL,request_id TEXT NOT NULL,request_sha256 TEXT NOT NULL,generation TEXT NOT NULL,origin_kind TEXT NOT NULL CHECK(origin_kind IN ('OPEN','RESUME')),process_operation_id TEXT NOT NULL,instance_id TEXT NOT NULL,binary_digest_sha256 TEXT NOT NULL,binding_id TEXT NOT NULL,worktree_id TEXT NOT NULL,namespace_id TEXT NOT NULL,alias_name TEXT NOT NULL,requested_target TEXT NOT NULL,observed_target TEXT,volume_serial TEXT NOT NULL,file_id TEXT NOT NULL,host_pid TEXT NOT NULL,host_creation_time_100ns TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('INTENT','MAPPED','PREPARE_STARTED','CUSTODY_BOUND','UNKNOWN')),child_operation_id TEXT,child_ticket TEXT,child_nonce TEXT,child_pid TEXT,child_creation_time_100ns TEXT,cleanup_status TEXT NOT NULL CHECK(cleanup_status IN ('NOT_REQUESTED','UNKNOWN','REMOVED','ABSENT')),cleanup_result TEXT,revision INTEGER NOT NULL CHECK(revision>=1),PRIMARY KEY(domain_id,session_id,generation),CHECK((phase='INTENT' AND observed_target IS NULL) OR (phase<>'INTENT' AND observed_target IS NOT NULL)),CHECK((child_operation_id IS NULL AND child_ticket IS NULL AND child_nonce IS NULL AND child_pid IS NULL AND child_creation_time_100ns IS NULL) OR (child_operation_id IS NOT NULL AND child_ticket IS NOT NULL AND child_nonce IS NOT NULL AND child_pid IS NOT NULL AND child_creation_time_100ns IS NOT NULL)),CHECK((cleanup_status='NOT_REQUESTED' AND cleanup_result IS NULL) OR (cleanup_status<>'NOT_REQUESTED' AND cleanup_result IS NOT NULL))) STRICT"),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DosAliasIntent {
    pub(super) domain_id: String,
    pub(super) session_id: String,
    pub(super) request_id: String,
    pub(super) request_sha256: String,
    pub(super) generation: String,
    pub(super) origin_kind: String,
    pub(super) process_operation_id: String,
    pub(super) instance_id: String,
    pub(super) binary_digest_sha256: String,
    pub(super) binding_id: String,
    pub(super) worktree_id: String,
    pub(super) namespace_id: String,
    pub(super) alias_name: String,
    pub(super) protocol_path: String,
    pub(super) requested_target: String,
    pub(super) observed_target: Option<String>,
    pub(super) volume_serial: String,
    pub(super) file_id: String,
    pub(super) host_pid: String,
    pub(super) host_creation_time_100ns: String,
    pub(super) phase: String,
    pub(super) child_operation_id: Option<String>,
    pub(super) child_ticket: Option<String>,
    pub(super) child_nonce: Option<String>,
    pub(super) child_pid: Option<String>,
    pub(super) child_creation_time_100ns: Option<String>,
    pub(super) cleanup_status: String,
    pub(super) cleanup_result: Option<String>,
    pub(super) revision: i64,
}

pub(super) fn verify_opencode_dos_alias_record(
    connection: &VerifiedDatabaseConnection<'_>, alias: &DosAliasIntent,
) -> Result<(), AdmissionError> {
    let row=Statement::prepare(connection.as_ptr(),
        "SELECT request_sha256,generation,origin_kind,process_operation_id,instance_id,
          binary_digest_sha256,binding_id,worktree_id,namespace_id,alias_name,requested_target,
          COALESCE(observed_target,''),volume_serial,file_id,host_pid,
          host_creation_time_100ns,phase,COALESCE(child_operation_id,''),
          COALESCE(child_ticket,''),COALESCE(child_nonce,''),COALESCE(child_pid,''),
          COALESCE(child_creation_time_100ns,''),cleanup_status,COALESCE(cleanup_result,''),revision
          FROM main.gogoke_v37_h_dos_alias WHERE domain_id=?1 AND session_id=?2
            AND request_id=?3 AND generation=?4")?;
    for (index,value) in [alias.domain_id.as_str(),alias.session_id.as_str(),
        alias.request_id.as_str(),alias.generation.as_str()].iter().enumerate(){
        row.bind_text(index as i32+1,value)?;
    }
    if !row.step_row()? {return Err(AdmissionError::Unknown);}
    let observed=alias.observed_target.as_deref().unwrap_or("");
    let fields=[alias.request_sha256.as_str(),alias.generation.as_str(),
        alias.origin_kind.as_str(),alias.process_operation_id.as_str(),
        alias.instance_id.as_str(),alias.binary_digest_sha256.as_str(),
        alias.binding_id.as_str(),alias.worktree_id.as_str(),
        alias.namespace_id.as_str(),alias.alias_name.as_str(),alias.requested_target.as_str(),
        observed,alias.volume_serial.as_str(),alias.file_id.as_str(),alias.host_pid.as_str(),
        alias.host_creation_time_100ns.as_str(),alias.phase.as_str(),
        alias.child_operation_id.as_deref().unwrap_or(""),alias.child_ticket.as_deref().unwrap_or(""),
        alias.child_nonce.as_deref().unwrap_or(""),alias.child_pid.as_deref().unwrap_or(""),
        alias.child_creation_time_100ns.as_deref().unwrap_or(""),alias.cleanup_status.as_str(),
        alias.cleanup_result.as_deref().unwrap_or("")];
    for (index,expected) in fields.iter().enumerate(){
        if row.column_text(index as i32)?!=*expected {return Err(AdmissionError::Conflict);}
    }
    if row.column_text(24)?.parse::<i64>().ok()!=Some(alias.revision) || row.step_row()? {
        return Err(AdmissionError::Conflict);
    }
    verify_opencode_alias_source(connection,alias)?;
    Ok(())
}

fn verify_opencode_alias_source(
    connection:&VerifiedDatabaseConnection<'_>,alias:&DosAliasIntent,
)->Result<(),AdmissionError>{
    let (sql,parameters):(&str,[&str;4])=match alias.origin_kind.as_str(){
        "OPEN"=>("SELECT raw_hex FROM main.gogoke_v37_h_operation
            WHERE domain_id=?1 AND request_id=?2 AND session_id=?3 AND operation='open'
              AND status IN ('UNKNOWN','APPLIED')",[
                alias.domain_id.as_str(),alias.request_id.as_str(),alias.session_id.as_str(),""]),
        "RESUME"=>("SELECT raw_hex FROM main.gogoke_v37_h_process_episode
            WHERE domain_id=?1 AND request_id=?2 AND session_id=?3 AND generation=?4",
            [alias.domain_id.as_str(),alias.request_id.as_str(),alias.session_id.as_str(),alias.generation.as_str()]),
        _=>return Err(AdmissionError::Denied),
    };
    let row=Statement::prepare(connection.as_ptr(),sql)?;
    for(index,value)in parameters.iter().enumerate(){
        if !value.is_empty(){row.bind_text(index as i32+1,value)?;}
    }
    if !row.step_row()? {return Err(AdmissionError::Conflict);}
    let raw_hex=row.column_text(0)?;
    if row.step_row()? {return Err(AdmissionError::Conflict);}
    let raw=decode_hex_bytes(&raw_hex)?;
    let request=super::wire::decode_request(&raw).map_err(|_|AdmissionError::Denied)?;
    if request.family!="K-SESSION" || request.request_id!=alias.request_id
        || request.domain_id!=alias.domain_id || request.target_id!=alias.session_id
        || request.operation!=(if alias.origin_kind=="OPEN"{"open"}else{"resume"}) {
        return Err(AdmissionError::Conflict);
    }
    let digest=crate::store::digest::sha256_hex(&raw);
    if digest!=alias.request_sha256
        || alias.process_operation_id!=format!("h-{}-{}",request.operation,&digest[..40]) {
        return Err(AdmissionError::Conflict);
    }
    let binding=if alias.origin_kind=="OPEN" {
        let episode=count(connection,
            "SELECT COUNT(*) FROM main.gogoke_v37_h_process_episode
              WHERE domain_id=?1 AND session_id=?2 AND request_id=?3 AND generation=?4
                AND raw_hex=?5 AND instance_id=?6 AND binding_id=?7
                AND (process_operation_id IS NULL OR process_operation_id=?8)",
            &[&alias.domain_id,&alias.session_id,&alias.request_id,&alias.generation,
                &raw_hex,&alias.instance_id,&alias.binding_id,&alias.process_operation_id])?;
        if episode==1 {1} else {count(connection,
            "SELECT COUNT(*) FROM main.gogoke_v37_h_claim
              WHERE domain_id=?1 AND session_id=?2 AND generation=?3 AND instance_id=?4
                AND binding_id=?5 AND state IN ('COMMITTED','UNKNOWN','STOPPED')
                AND (process_operation_id IS NULL OR process_operation_id=?6)",
            &[&alias.domain_id,&alias.session_id,&alias.generation,&alias.instance_id,
                &alias.binding_id,&alias.process_operation_id])?}
    }else{
        count(connection,
            "SELECT COUNT(*) FROM main.gogoke_v37_h_process_episode
              WHERE domain_id=?1 AND session_id=?2 AND request_id=?3 AND generation=?4
                AND raw_hex=?5 AND instance_id=?6 AND binding_id=?7
                AND (process_operation_id IS NULL OR process_operation_id=?8)",
            &[&alias.domain_id,&alias.session_id,&alias.request_id,&alias.generation,
                &raw_hex,&alias.instance_id,&alias.binding_id,&alias.process_operation_id])?
    };
    if binding!=1 {return Err(AdmissionError::Conflict);}
    Ok(())
}

fn decode_hex_bytes(value:&str)->Result<Vec<u8>,AdmissionError>{
    if value.is_empty() || value.len()%2!=0
        || !value.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)) {
        return Err(AdmissionError::Denied);
    }
    value.as_bytes().chunks_exact(2).map(|pair|{
        let byte=|v:u8|if v.is_ascii_digit(){v-b'0'}else{v-b'a'+10};
        Ok((byte(pair[0])<<4)|byte(pair[1]))
    }).collect()
}

/// The alias mapping is a side effect of the same original child whose H
/// episode and C Job custody must already have one exact durable StopFact.
/// This is shared by the pre-remove check and the cleanup-outcome CAS. It
/// deliberately does not join the current claim: an unadopted resume
/// candidate can have a valid stopped episode while the old claim remains
/// pointed at its prior generation.
pub(super) fn verify_opencode_dos_alias_stop_source(
    connection:&VerifiedDatabaseConnection<'_>,alias:&DosAliasIntent,stop_fact:&str,
)->Result<(),AdmissionError>{
    require(stop_fact,"stop_fact")?;
    if alias.phase!="CUSTODY_BOUND" {
        return Err(AdmissionError::Denied);
    }
    let matches=count(connection,
        "SELECT COUNT(*) FROM main.gogoke_v37_h_dos_alias a
          JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=a.domain_id
            AND e.session_id=a.session_id AND e.generation=a.generation
            AND e.request_id=a.request_id AND e.process_operation_id=a.process_operation_id
          JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id
            AND c.domain_id=e.domain_id AND c.generation=e.generation
          WHERE a.domain_id=?1 AND a.session_id=?2 AND a.generation=?3
            AND a.request_id=?4 AND a.revision=?5 AND a.phase='CUSTODY_BOUND'
            AND a.child_operation_id=c.operation_id AND a.child_ticket=c.ticket
            AND a.child_nonce=c.custodian_nonce AND a.child_pid=c.pid
            AND a.child_creation_time_100ns=c.creation_time_100ns
            AND c.profile_id=a.instance_id
            AND c.binary_digest_sha256=a.binary_digest_sha256
            AND e.phase='STOPPED' AND e.stop_fact_id=?6
            AND c.state='STOPPED' AND c.stop_proof_hash=?6",
        &[&alias.domain_id,&alias.session_id,&alias.generation,&alias.request_id,
            &alias.revision.to_string(),stop_fact])?;
    if matches!=1 {return Err(AdmissionError::Denied);}
    Ok(())
}

fn read_alias_stop_fact(
    connection:&VerifiedDatabaseConnection<'_>,alias:&DosAliasIntent,
)->Result<Option<String>,AdmissionError>{
    let row=Statement::prepare(connection.as_ptr(),
        "SELECT e.stop_fact_id FROM main.gogoke_v37_h_process_episode e
          JOIN main.gogoke_coordination_process_custody c
            ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
            AND c.generation=e.generation
          WHERE e.domain_id=?1 AND e.session_id=?2 AND e.generation=?3
            AND e.request_id=?4 AND e.process_operation_id=?5
            AND e.phase='STOPPED' AND e.stop_fact_id IS NOT NULL
            AND c.state='STOPPED' AND c.stop_proof_hash=e.stop_fact_id")?;
    for(index,value)in [alias.domain_id.as_str(),alias.session_id.as_str(),
        alias.generation.as_str(),alias.request_id.as_str(),alias.process_operation_id.as_str()]
        .iter().enumerate(){row.bind_text(index as i32+1,value)?;}
    if !row.step_row()? {return Ok(None);}
    let stop=row.column_text(0)?;
    if stop.is_empty() || row.step_row()? {return Err(AdmissionError::Conflict);}
    Ok(Some(stop))
}

pub(super) fn record_opencode_dos_alias_cleanup(
    connection:&mut VerifiedDatabaseConnection<'_>,alias:&DosAliasIntent,
    stop_fact:&str,status:&str,result:&str,
)->Result<DosAliasIntent,AdmissionError>{
    require(stop_fact,"stop_fact")?;
    if !matches!(status,"REMOVED"|"ABSENT"|"UNKNOWN") || result.is_empty() {
        return Err(AdmissionError::Invalid("cleanup_result"));
    }
    verify_opencode_dos_alias_stop_source(connection,alias,stop_fact)?;
    in_transaction(connection,|connection|{
        let update=Statement::prepare(connection.as_ptr(),
            "UPDATE main.gogoke_v37_h_dos_alias SET cleanup_status=?1,cleanup_result=?2,
              revision=revision+1 WHERE domain_id=?3 AND session_id=?4 AND generation=?5
                AND request_id=?6 AND revision=?7 AND phase='CUSTODY_BOUND'
                AND cleanup_status IN ('NOT_REQUESTED','UNKNOWN')")?;
        let revision=alias.revision.to_string();
        for(index,value) in [status,result,alias.domain_id.as_str(),alias.session_id.as_str(),
            alias.generation.as_str(),alias.request_id.as_str(),revision.as_str()]
            .iter().enumerate(){update.bind_text(index as i32+1,value)?;}
        update.step_done()?;
        if changes(connection)?!=1{return Err(AdmissionError::Conflict);}
        let mut updated=alias.clone();
        updated.cleanup_status=status.into();updated.cleanup_result=Some(result.into());
        updated.revision+=1;
        Ok(updated)
    })
}

/// Reconcile one original alias after a prior native-host process disappeared.
/// This updates only H alias cleanup state; it never changes claim state,
/// admission release, StopFact or any user-visible H operation receipt.
pub(crate) fn recover_orphan_opencode_dos_alias(
    connection:&mut VerifiedDatabaseConnection<'_>,domain:&str,session:&str,generation:&str,
)->Result<String,AdmissionError>{
    let alias=read_opencode_dos_alias(connection,domain,session,generation)?
        .ok_or(AdmissionError::Unknown)?;
    if matches!(alias.cleanup_status.as_str(),"REMOVED"|"ABSENT") {
        return Ok(alias.cleanup_status);
    }
    let fail_closed=|connection:&mut VerifiedDatabaseConnection<'_>,record:&DosAliasIntent,reason:&str|
        ->Result<String,AdmissionError>{
        record_orphan_dos_alias_result(connection,record,"UNKNOWN",reason)?;
        Err(AdmissionError::Unknown)
    };
    let namespace=match super::dos_alias::current_namespace_id(){
        Ok(value)=>value,Err(error)=>return fail_closed(connection,&alias,&format!(
            "current namespace query failed: {error}")),
    };
    if namespace!=alias.namespace_id {
        return fail_closed(connection,&alias,"recorded logon namespace is not current");
    }
    let host_pid=alias.host_pid.parse::<u32>().map_err(|_|AdmissionError::Denied)?;
    let host_creation=alias.host_creation_time_100ns.parse::<u64>()
        .map_err(|_|AdmissionError::Denied)?;
    match super::dos_alias::current_host_identity(){
        Ok((pid,creation)) if (pid,creation)==(host_pid,host_creation)=>
            return Err(AdmissionError::Unknown),
        Ok(_)=>{},
        Err(error)=>return fail_closed(connection,&alias,&format!(
            "current host identity query failed: {error}")),
    }
    if let Err(error)=crate::process::NativeProcessHoldersGone::observe(&[(host_pid,host_creation)]){
        return fail_closed(connection,&alias,&format!("original host holder not proven gone: {error:?}"));
    }
    let stop_fact=match alias.phase.as_str(){
        // The request path is fenced before ProcessCustodian.prepare. These
        // phases can own an alias side effect, but cannot own a CLI child.
        "INTENT"|"MAPPED"=>None,
        // A crash after PREPARE_STARTED can leave a child whose identity was
        // never committed to H. Host exit alone is not a Job proof.
        "PREPARE_STARTED"=>return fail_closed(connection,&alias,
            "prepare-started alias lacks committed child identity and StopFact"),
        "CUSTODY_BOUND"=>match read_alias_stop_fact(connection,&alias){
            Ok(Some(value))=>Some(value),
            Ok(None)=>return fail_closed(connection,&alias,
                "C Job and H episode do not share a durable StopFact"),
            Err(error)=>return fail_closed(connection,&alias,&format!(
                "original C/H StopFact could not be verified: {error:?}")),
        },
        _=>return fail_closed(connection,&alias,"alias phase is not recoverable"),
    };
    if stop_fact.is_some(){
        if let Err(error)=verify_opencode_dos_alias_stop_source(
            connection,&alias,stop_fact.as_deref().unwrap()) {
            return fail_closed(connection,&alias,&format!("original C/H StopFact mismatch: {error:?}"));
        }
        let pid=alias.child_pid.as_deref().ok_or(AdmissionError::Denied)?
            .parse::<u32>().map_err(|_|AdmissionError::Denied)?;
        let creation=alias.child_creation_time_100ns.as_deref().ok_or(AdmissionError::Denied)?
            .parse::<u64>().map_err(|_|AdmissionError::Denied)?;
        if let Err(error)=crate::process::NativeProcessHoldersGone::observe(&[(pid,creation)]){
            return fail_closed(connection,&alias,&format!("original C process not proven gone: {error:?}"));
        }
    }
    let root_identity=identity_from_alias(&alias)?;
    let mappings=match super::dos_alias::query(&alias.alias_name){
        Ok(value)=>value,Err(error)=>return fail_closed(connection,&alias,&format!(
            "current mapping query failed: {error}")),
    };
    let status=if mappings.is_empty(){
        "ABSENT"
    } else if mappings.len()==1
        && alias.observed_target.as_deref().map_or(true,|target|
            mappings.first().is_some_and(|observed|observed==target)) {
        if let Err(error)=super::dos_alias::verify_mapping(&alias.alias_name,
            alias.observed_target.as_deref(),&root_identity) {
            return fail_closed(connection,&alias,&format!("physical alias verification failed: {error}"));
        }
        if let Some(stop_fact)=stop_fact.as_deref(){
            if let Err(error)=verify_opencode_dos_alias_stop_source(connection,&alias,stop_fact){
                return fail_closed(connection,&alias,&format!("StopFact changed before alias removal: {error:?}"));
            }
        }
        if let Err(error)=super::dos_alias::remove_exact(&alias.alias_name,&alias.requested_target){
            return fail_closed(connection,&alias,&format!("exact removal or absence readback failed: {error}"));
        }
        "REMOVED"
    } else {
        return fail_closed(connection,&alias,"current target or physical F FileID differs");
    };
    record_orphan_dos_alias_result(connection,&alias,status,
        if status=="REMOVED"{"exact target removed after original holders were proven gone"}
        else {"exact mapping was already absent after original holders were proven gone"})?;
    Ok(status.into())
}

fn identity_from_alias(alias:&DosAliasIntent)->Result<crate::root::RootIdentity,AdmissionError>{
    let volume_serial=u64::from_str_radix(&alias.volume_serial,16)
        .map_err(|_|AdmissionError::Denied)?;
    if alias.file_id.len()!=32 || !alias.file_id.bytes().all(|b|b.is_ascii_hexdigit()) {
        return Err(AdmissionError::Denied);
    }
    let mut file_id=[0u8;16];
    for(index,slot) in file_id.iter_mut().enumerate(){
        *slot=u8::from_str_radix(&alias.file_id[index*2..index*2+2],16)
            .map_err(|_|AdmissionError::Denied)?;
    }
    Ok(crate::root::RootIdentity{volume_serial,file_id})
}

fn read_opencode_dos_alias(connection:&VerifiedDatabaseConnection<'_>,domain:&str,
    session:&str,generation:&str)->Result<Option<DosAliasIntent>,AdmissionError>{
    let row=Statement::prepare(connection.as_ptr(),
        "SELECT request_id,request_sha256,origin_kind,process_operation_id,instance_id,
          binary_digest_sha256,binding_id,worktree_id,namespace_id,alias_name,requested_target,
          COALESCE(observed_target,''),volume_serial,file_id,host_pid,
          host_creation_time_100ns,phase,COALESCE(child_operation_id,''),
          COALESCE(child_ticket,''),COALESCE(child_nonce,''),COALESCE(child_pid,''),
          COALESCE(child_creation_time_100ns,''),cleanup_status,COALESCE(cleanup_result,''),revision
          FROM main.gogoke_v37_h_dos_alias WHERE domain_id=?1 AND session_id=?2 AND generation=?3")?;
    row.bind_text(1,domain)?;row.bind_text(2,session)?;row.bind_text(3,generation)?;
    if !row.step_row()? {return Ok(None);}
    let text=(0..25).map(|index|row.column_text(index)).collect::<Result<Vec<_>,_>>()?;
    if row.step_row()? {return Err(AdmissionError::Conflict);}
    let optional=|index:usize|if text[index].is_empty(){None}else{Some(text[index].clone())};
    let alias=DosAliasIntent{domain_id:domain.into(),session_id:session.into(),request_id:text[0].clone(),
        request_sha256:text[1].clone(),generation:generation.into(),origin_kind:text[2].clone(),
        process_operation_id:text[3].clone(),instance_id:text[4].clone(),
        binary_digest_sha256:text[5].clone(),binding_id:text[6].clone(),
        worktree_id:text[7].clone(),namespace_id:text[8].clone(),alias_name:text[9].clone(),
        protocol_path:format!(r"{}\",text[9]),requested_target:text[10].clone(),
        observed_target:optional(11),volume_serial:text[12].clone(),file_id:text[13].clone(),
        host_pid:text[14].clone(),host_creation_time_100ns:text[15].clone(),phase:text[16].clone(),
        child_operation_id:optional(17),child_ticket:optional(18),child_nonce:optional(19),
        child_pid:optional(20),child_creation_time_100ns:optional(21),cleanup_status:text[22].clone(),
        cleanup_result:optional(23),revision:text[24].parse().map_err(|_|AdmissionError::Denied)?};
    verify_opencode_dos_alias_record(connection,&alias)?;
    Ok(Some(alias))
}

struct AliasCustodyFacts<'a>{
    operation_id:&'a str,ticket:&'a str,nonce:&'a str,pid:&'a str,
    creation:&'a str,image_path:&'a str,binary_digest:&'a str,
    profile_id:&'a str,domain_id:&'a str,generation:&'a str,state:&'a str,
}

/// Compare every identity field with the durable C row. The H record never
/// treats copied PID/ticket strings as a substitute for ProcessCustodian.
fn verify_original_alias_custody_row(
    connection:&VerifiedDatabaseConnection<'_>,alias:&DosAliasIntent,
    facts:&AliasCustodyFacts<'_>,expected_state:&str,
)->Result<(),AdmissionError>{
    if facts.operation_id!=alias.process_operation_id
        || facts.ticket.is_empty() || facts.nonce.is_empty()
        || facts.pid=="0" || facts.creation=="0" || facts.image_path.is_empty()
        || facts.binary_digest!=alias.binary_digest_sha256
        || facts.profile_id!=alias.instance_id || facts.domain_id!=alias.domain_id
        || facts.generation!=alias.generation || facts.state!=expected_state {
        return Err(AdmissionError::Conflict);
    }
    let row=Statement::prepare(connection.as_ptr(),
        "SELECT ticket,custodian_nonce,pid,creation_time_100ns,image_path,
          binary_digest_sha256,profile_id,domain_id,generation,state,
          COALESCE(stop_proof_hash,'')
          FROM main.gogoke_coordination_process_custody WHERE operation_id=?1")?;
    row.bind_text(1,facts.operation_id)?;
    if !row.step_row()? {return Err(AdmissionError::Unknown);}
    let expected=[facts.ticket,facts.nonce,facts.pid,facts.creation,facts.image_path,
        facts.binary_digest,facts.profile_id,facts.domain_id,facts.generation,facts.state];
    for(index,value)in expected.iter().enumerate(){
        if row.column_text(index as i32)?!=*value {return Err(AdmissionError::Conflict);}
    }
    if expected_state=="PREPARED" && !row.column_text(10)?.is_empty(){
        return Err(AdmissionError::Conflict);
    }
    if row.step_row()? {return Err(AdmissionError::Conflict);}
    Ok(())
}

fn record_orphan_dos_alias_result(connection:&mut VerifiedDatabaseConnection<'_>,
    alias:&DosAliasIntent,status:&str,result:&str)->Result<(),AdmissionError>{
    in_transaction(connection,|connection|{
        let update=Statement::prepare(connection.as_ptr(),
            "UPDATE main.gogoke_v37_h_dos_alias SET cleanup_status=?1,cleanup_result=?2,
              revision=revision+1 WHERE domain_id=?3 AND session_id=?4 AND generation=?5
                AND request_id=?6 AND revision=?7 AND cleanup_status IN ('NOT_REQUESTED','UNKNOWN')")?;
        let revision=alias.revision.to_string();
        for(index,value) in [status,result,alias.domain_id.as_str(),alias.session_id.as_str(),
            alias.generation.as_str(),alias.request_id.as_str(),revision.as_str()]
            .iter().enumerate(){update.bind_text(index as i32+1,value)?;}
        update.step_done()?;
        if changes(connection)?!=1{return Err(AdmissionError::Conflict);}
        Ok(())
    })
}

fn valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}
fn require(value: &str, name: &'static str) -> Result<(), AdmissionError> {
    if valid(value) {
        Ok(())
    } else {
        Err(AdmissionError::Invalid(name))
    }
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 15) as usize] as char);
    }
    out
}
fn count(
    connection: &VerifiedDatabaseConnection<'_>,
    sql: &str,
    params: &[&str],
) -> Result<i64, AdmissionError> {
    let query = Statement::prepare(connection.as_ptr(), sql)?;
    for (index, value) in params.iter().enumerate() {
        query.bind_text((index + 1) as i32, value)?;
    }
    if !query.step_row()? {
        return Err(AdmissionError::Unknown);
    }
    query
        .column_text(0)?
        .parse()
        .map_err(|_| AdmissionError::Unknown)
}
fn changes(connection: &VerifiedDatabaseConnection<'_>) -> Result<i64, AdmissionError> {
    count(connection, "SELECT changes()", &[])
}
fn in_transaction<T>(
    connection: &mut VerifiedDatabaseConnection<'_>,
    action: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T, AdmissionError>,
) -> Result<T, AdmissionError> {
    connection.execute("BEGIN IMMEDIATE")?;
    match action(connection) {
        Ok(value) => {
            connection
                .execute("COMMIT")
                .map_err(AdmissionError::CommitUnknown)?;
            Ok(value)
        }
        Err(error) => {
            connection
                .execute("ROLLBACK")
                .map_err(AdmissionError::RollbackUnknown)?;
            Err(error)
        }
    }
}

/// Exact schema family; partial or changed state is refused, not repaired.
pub(crate) fn initialize_admission_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<(), AdmissionError> {
    reject_shadow_or_effects(connection)?;
    let rows = observed_schema(connection)?;
    let mut expected: Vec<(String, String)> = SCHEMA
        .iter()
        .map(|(name, sql)| ((*name).to_owned(), (*sql).to_owned()))
        .collect();
    expected.sort_by(|left, right| left.0.cmp(&right.0));
    if rows == expected { return Ok(()); }
    let mut previous: Vec<(String, String)> = SCHEMA[..6].iter()
        .map(|(name, sql)| ((*name).to_owned(), (*sql).to_owned())).collect();
    previous.sort_by(|left, right| left.0.cmp(&right.0));
    let mut older: Vec<(String, String)> = SCHEMA[..5].iter()
        .map(|(name, sql)| ((*name).to_owned(), (*sql).to_owned())).collect();
    older.sort_by(|left, right| left.0.cmp(&right.0));
    let mut prior_eight: Vec<(String, String)> = SCHEMA[..8].iter()
        .map(|(name, sql)| ((*name).to_owned(), (*sql).to_owned())).collect();
    prior_eight.sort_by(|left, right| left.0.cmp(&right.0));
    let mut prior_nine: Vec<(String, String)> = SCHEMA[..9].iter()
        .map(|(name, sql)| ((*name).to_owned(), (*sql).to_owned())).collect();
    prior_nine.sort_by(|left, right| left.0.cmp(&right.0));
    if !rows.is_empty() && rows != previous && rows != older && rows != prior_eight
        && rows != prior_nine {
        return Err(AdmissionError::Denied);
    }
    in_transaction(connection, |connection| {
        reject_shadow_or_effects(connection)?;
        if observed_schema(connection)? != rows {
            return Err(AdmissionError::Denied);
        }
        let to_create = if rows.is_empty() { &SCHEMA[..] }
            else if rows == older { &SCHEMA[5..] }
            else if rows == previous { &SCHEMA[6..] }
            else if rows == prior_eight { &SCHEMA[8..] } else { &SCHEMA[9..] };
        for (_, sql) in to_create {
            connection.execute(sql)?;
        }
        if rows == older || rows == previous {
            super::episodes::backfill(connection).map_err(AdmissionError::Store)?;
        }
        if observed_schema(connection)? != expected {
            return Err(AdmissionError::Denied);
        }
        Ok(())
    })
}

/// Durably reserve one unused logon-local DOS name for an already-persisted
/// OpenCode H open/resume intent. No OS mapping exists when this returns.
#[allow(clippy::too_many_arguments)]
pub(super) fn reserve_opencode_dos_alias(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain: &str,
    session: &str,
    request_id: &str,
    request_bytes: &[u8],
    generation: &str,
    process_operation_id: &str,
    instance_id: &str,
    binary_digest_sha256: &str,
    binding_id: &str,
    worktree_id: &str,
    physical_path: &str,
    identity: &crate::root::RootIdentity,
) -> Result<DosAliasIntent, AdmissionError> {
    for (value, name) in [
        (domain, "domain"), (session, "session"), (request_id, "request"),
        (generation, "generation"), (process_operation_id, "process_operation"),
        (instance_id, "instance"), (binary_digest_sha256,"binary_digest"),
        (binding_id, "binding"),
        (worktree_id, "worktree"),
    ] { require(value, name)?; }
    if request_bytes.is_empty() || physical_path.is_empty() || physical_path.contains('\0')
        || !std::path::Path::new(physical_path).is_absolute() {
        return Err(AdmissionError::Invalid("request_bytes_or_path"));
    }
    let namespace_id = super::dos_alias::current_namespace_id()
        .map_err(AdmissionError::Native)?;
    let (host_pid, host_creation) = super::dos_alias::current_host_identity()
        .map_err(AdmissionError::Native)?;
    let request_sha256 = crate::store::digest::sha256_hex(request_bytes);
    let request_hex = hex(request_bytes);
    let volume_serial = format!("{:016x}", identity.volume_serial);
    let file_id = identity.file_id.iter().map(|b| format!("{b:02x}")).collect::<String>();

    in_transaction(connection, |connection| {
        let intent = Statement::prepare(connection.as_ptr(),
            "SELECT
              (SELECT COUNT(*) FROM main.gogoke_v37_h_operation
                WHERE domain_id=?1 AND request_id=?2 AND session_id=?3
                  AND raw_hex=?4 AND operation='open' AND status='UNKNOWN'),
              (SELECT COUNT(*) FROM main.gogoke_v37_h_process_episode
                WHERE domain_id=?1 AND request_id=?2 AND session_id=?3
                  AND raw_hex=?4 AND generation=?5 AND old_generation IS NOT NULL
                  AND phase='INTENT')")?;
        intent.bind_text(1, domain)?; intent.bind_text(2, request_id)?;
        intent.bind_text(3, session)?; intent.bind_text(4, &request_hex)?;
        intent.bind_text(5, generation)?;
        if !intent.step_row()? {
            return Err(AdmissionError::Conflict);
        }
        let open=intent.column_text(0)?;
        let resume=intent.column_text(1)?;
        let origin_kind=if open=="1"&&resume=="0"{"OPEN"}
            else if open=="0"&&resume=="1"{"RESUME"}
            else{return Err(AdmissionError::Conflict);};
        drop(intent);
        let existing = count(connection,
            "SELECT COUNT(*) FROM main.gogoke_v37_h_dos_alias
              WHERE domain_id=?1 AND session_id=?2 AND generation=?3",
            &[domain, session, generation])?;
        if existing != 0 {
            return Err(AdmissionError::Conflict);
        }
        let mut selected = None;
        for name in super::dos_alias::free_candidates() {
            let reserved = count(connection,
                "SELECT COUNT(*) FROM main.gogoke_v37_h_dos_alias
                  WHERE namespace_id=?1 AND alias_name=?2
                    AND cleanup_status NOT IN ('REMOVED','ABSENT')",
                &[&namespace_id, name])?;
            if reserved != 0 || !super::dos_alias::query(name)
                .map_err(AdmissionError::Native)?.is_empty() {
                continue;
            }
            selected = Some(name.to_owned());
            break;
        }
        let alias_name = selected.ok_or(AdmissionError::UnsupportedCapacity)?;
        let insert = Statement::prepare(connection.as_ptr(), "INSERT INTO main.gogoke_v37_h_dos_alias(
          domain_id,session_id,request_id,request_sha256,generation,origin_kind,
          process_operation_id,instance_id,binary_digest_sha256,binding_id,worktree_id,namespace_id,
          alias_name,requested_target,observed_target,volume_serial,file_id,
          host_pid,host_creation_time_100ns,phase,cleanup_status,revision)
          VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,NULL,?15,?16,?17,?18,
            'INTENT','NOT_REQUESTED',1)")?;
        let host_pid_text=host_pid.to_string();
        let host_creation_text=host_creation.to_string();
        for (index, value) in [domain, session, request_id, request_sha256.as_str(), generation,
            origin_kind,process_operation_id, instance_id,binary_digest_sha256,
            binding_id, worktree_id, namespace_id.as_str(),
            alias_name.as_str(), physical_path, volume_serial.as_str(), file_id.as_str(),
            host_pid_text.as_str(), host_creation_text.as_str()]
            .iter().enumerate() {
            insert.bind_text(index as i32 + 1, value)?;
        }
        insert.step_done()?;
        Ok(DosAliasIntent {
            domain_id:domain.into(),session_id:session.into(),request_id:request_id.into(),
            request_sha256,generation:generation.into(),origin_kind:origin_kind.into(),
            process_operation_id:process_operation_id.into(),
            instance_id:instance_id.into(),binary_digest_sha256:binary_digest_sha256.into(),
            binding_id:binding_id.into(),worktree_id:worktree_id.into(),
            namespace_id,alias_name,protocol_path:String::new(),
            requested_target:physical_path.into(),observed_target:None,
            volume_serial,file_id,host_pid:host_pid.to_string(),
            host_creation_time_100ns:host_creation.to_string(),phase:"INTENT".into(),
            child_operation_id:None,child_ticket:None,child_nonce:None,child_pid:None,
            child_creation_time_100ns:None,cleanup_status:"NOT_REQUESTED".into(),
            cleanup_result:None,revision:1,
        })
    })
}

pub(super) fn mark_opencode_dos_alias_mapped(
    connection: &mut VerifiedDatabaseConnection<'_>, alias: &DosAliasIntent,
    observed_target: &str,
) -> Result<DosAliasIntent, AdmissionError> {
    if observed_target.is_empty() || observed_target.contains('\0') {
        return Err(AdmissionError::Invalid("observed_target"));
    }
    in_transaction(connection, |connection| {
        let update=Statement::prepare(connection.as_ptr(),
            "UPDATE main.gogoke_v37_h_dos_alias SET observed_target=?1,phase='MAPPED',revision=revision+1
              WHERE domain_id=?2 AND session_id=?3 AND generation=?4 AND request_id=?5
                AND process_operation_id=?6 AND alias_name=?7 AND requested_target=?8
                AND namespace_id=?9 AND volume_serial=?10 AND file_id=?11 AND phase='INTENT'
                AND revision=?12")?;
        for (index,value) in [observed_target,alias.domain_id.as_str(),alias.session_id.as_str(),
            alias.generation.as_str(),alias.request_id.as_str(),alias.process_operation_id.as_str(),
            alias.alias_name.as_str(),alias.requested_target.as_str(),alias.namespace_id.as_str(),
            alias.volume_serial.as_str(),alias.file_id.as_str()].iter().enumerate() {
            update.bind_text(index as i32+1,value)?;
        }
        update.bind_text(12,&alias.revision.to_string())?;
        update.step_done()?;
        if changes(connection)?!=1 {return Err(AdmissionError::Conflict);}
        let mut mapped=alias.clone();
        mapped.observed_target=Some(observed_target.into());
        mapped.phase="MAPPED".into();
        mapped.revision+=1;
        Ok(mapped)
    })
}

pub(super) fn mark_opencode_dos_alias_prepare_started(
    connection: &mut VerifiedDatabaseConnection<'_>, alias: &DosAliasIntent,
) -> Result<DosAliasIntent, AdmissionError> {
    set_dos_alias_phase(connection,alias,"MAPPED","PREPARE_STARTED")
}

pub(super) fn bind_opencode_dos_alias_custody(
    connection: &mut VerifiedDatabaseConnection<'_>, alias: &DosAliasIntent,
    child_operation_id: &str, expected_binary_digest:&str,
    custody:&crate::process::PreparedCustody,
) -> Result<DosAliasIntent, AdmissionError> {
    require(child_operation_id,"child_operation")?;
    let pid=custody.identity.pid.to_string();
    let creation=custody.identity.creation_time_100ns.to_string();
    let image=custody.identity.image_path.to_string_lossy().into_owned();
    let binding=&custody.binding;
    let facts=AliasCustodyFacts{operation_id:child_operation_id,
        ticket:custody.ticket.opaque(),nonce:&custody.custodian_nonce,pid:&pid,
        creation:&creation,image_path:&image,binary_digest:&binding.binary_digest_sha256,
        profile_id:&binding.profile_id,domain_id:&binding.domain_id,
        generation:&binding.generation,state:"PREPARED"};
    if expected_binary_digest.is_empty() || expected_binary_digest!=alias.binary_digest_sha256
        || binding.binary_digest_sha256!=expected_binary_digest {
        return Err(AdmissionError::Conflict);
    }
    in_transaction(connection,|connection|{
        verify_original_alias_custody_row(connection,alias,&facts,"PREPARED")?;
        let update=Statement::prepare(connection.as_ptr(),
            "UPDATE main.gogoke_v37_h_dos_alias SET phase='CUSTODY_BOUND',
              child_operation_id=?1,child_ticket=?2,child_nonce=?3,child_pid=?4,
              child_creation_time_100ns=?5,revision=revision+1
              WHERE domain_id=?6 AND session_id=?7 AND generation=?8 AND request_id=?9
                AND process_operation_id=?10 AND phase='PREPARE_STARTED' AND revision=?11")?;
        for (index,value) in [child_operation_id,facts.ticket,facts.nonce,pid.as_str(),
            creation.as_str(),alias.domain_id.as_str(),alias.session_id.as_str(),
            alias.generation.as_str(),alias.request_id.as_str(),alias.process_operation_id.as_str()]
            .iter().enumerate(){update.bind_text(index as i32+1,value)?;}
        update.bind_text(11,&alias.revision.to_string())?;
        update.step_done()?;
        if changes(connection)?!=1 {return Err(AdmissionError::Conflict);}
        let mut bound=alias.clone();
        bound.phase="CUSTODY_BOUND".into();
        bound.child_operation_id=Some(child_operation_id.into());
        bound.child_ticket=Some(facts.ticket.into());
        bound.child_nonce=Some(facts.nonce.into());
        bound.child_pid=Some(pid.clone());
        bound.child_creation_time_100ns=Some(creation.clone());
        bound.revision+=1;
        Ok(bound)
    })
}

fn set_dos_alias_phase(connection:&mut VerifiedDatabaseConnection<'_>,alias:&DosAliasIntent,
    before:&str,after:&str)->Result<DosAliasIntent,AdmissionError>{
    in_transaction(connection,|connection|{
        let update=Statement::prepare(connection.as_ptr(),
            "UPDATE main.gogoke_v37_h_dos_alias SET phase=?1,revision=revision+1
              WHERE domain_id=?2 AND session_id=?3 AND generation=?4 AND request_id=?5
                AND process_operation_id=?6 AND phase=?7 AND cleanup_status='NOT_REQUESTED'
                AND revision=?8")?;
        for (index,value) in [after,alias.domain_id.as_str(),alias.session_id.as_str(),
            alias.generation.as_str(),alias.request_id.as_str(),alias.process_operation_id.as_str(),before]
            .iter().enumerate(){update.bind_text(index as i32+1,value)?;}
        update.bind_text(8,&alias.revision.to_string())?;
        update.step_done()?;
        if changes(connection)?!=1 {return Err(AdmissionError::Conflict);}
        let mut updated=alias.clone();
        updated.phase=after.into();
        updated.revision+=1;
        Ok(updated)
    })
}
fn observed_schema(
    connection: &VerifiedDatabaseConnection<'_>,
) -> Result<Vec<(String, String)>, AdmissionError> {
    let observed = Statement::prepare(connection.as_ptr(),
        "SELECT name,sql FROM main.sqlite_schema WHERE substr(name,1,13)='gogoke_v37_h_' ORDER BY name")?;
    let mut rows = Vec::new();
    while observed.step_row()? {
        rows.push((observed.column_text(0)?, observed.column_text(1)?));
    }
    Ok(rows)
}
fn reject_shadow_or_effects(
    connection: &VerifiedDatabaseConnection<'_>,
) -> Result<(), AdmissionError> {
    for sql in [
        "SELECT 1 FROM temp.sqlite_schema WHERE substr(name,1,13)='gogoke_v37_h_' OR substr(tbl_name,1,13)='gogoke_v37_h_' LIMIT 1",
        "SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND substr(tbl_name,1,13)='gogoke_v37_h_' LIMIT 1",
    ] {
        if Statement::prepare(connection.as_ptr(),sql)?.step_row()? { return Err(AdmissionError::Denied); }
    }
    Ok(())
}

pub(crate) struct OwnerBinding<'a> {
    pub(crate) binding_id: &'a str,
    pub(crate) instance_id: &'a str,
    pub(crate) domain_id: &'a str,
    pub(crate) kind: &'a str,
    pub(crate) owner_id: &'a str,
    pub(crate) generation: &'a str,
}

/// Bind the actual E seat incarnation to this session in H's same admission
/// transaction. Shared instance and generation alone do not identify a seat.
pub(crate) fn bind_seat_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    seat: &crate::store::seat::Seat,
    session_id: &str,
) -> Result<(), AdmissionError> {
    require(session_id, "session_id")?;
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT seat_id,seat_incarnation,generation FROM main.gogoke_v37_h_seat_binding WHERE domain_id=?1 AND session_id=?2")?;
    row.bind_text(1, &seat.domain_id)?;
    row.bind_text(2, session_id)?;
    if row.step_row()? {
        if row.column_text(0)? != seat.seat_id || row.column_text(1)? != seat.incarnation
            || row.column_text(2)? != seat.generation.to_string() || row.step_row()? {
            return Err(AdmissionError::Conflict);
        }
        return Ok(());
    }
    let occupied = Statement::prepare(connection.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_h_seat_binding WHERE domain_id=?1 AND seat_incarnation=?2 AND generation=?3")?;
    occupied.bind_text(1, &seat.domain_id)?;
    occupied.bind_text(2, &seat.incarnation)?;
    occupied.bind_text(3, &seat.generation.to_string())?;
    if occupied.step_row()? { return Err(AdmissionError::Conflict); }
    let insert = Statement::prepare(connection.as_ptr(),
        "INSERT INTO main.gogoke_v37_h_seat_binding(domain_id,session_id,seat_id,seat_incarnation,generation) VALUES(?1,?2,?3,?4,?5)")?;
    let generation = seat.generation.to_string();
    for (index, value) in [seat.domain_id.as_str(), session_id, seat.seat_id.as_str(),
        seat.incarnation.as_str(), generation.as_str()].iter().enumerate() {
        insert.bind_text((index + 1) as i32, value)?;
    }
    insert.step_done()?;
    Ok(())
}

/// Called only after a native E/H seat binding is established. It does not
/// infer a caller from an app-server frame or a model-provided ticket.
pub(crate) fn bind_owner_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerBinding<'_>,
) -> Result<String, AdmissionError> {
    for (name, value) in [
        ("binding_id", owner.binding_id),
        ("instance_id", owner.instance_id),
        ("domain_id", owner.domain_id),
        ("owner_id", owner.owner_id),
        ("generation", owner.generation),
    ] {
        require(value, name)?;
    }
    if owner.kind != "SESSION" && owner.kind != "CALL" {
        return Err(AdmissionError::Invalid("kind"));
    }
    if count(
        connection,
        "SELECT COUNT(*) FROM gogoke_v37_instances WHERE instance_id=?1",
        &[owner.instance_id],
    )? != 1
    {
        return Err(AdmissionError::Denied);
    }
    let row = Statement::prepare(connection.as_ptr(),
        "INSERT INTO gogoke_v37_h_owner_binding(binding_id,instance_id,domain_id,kind,owner_id,generation,state) VALUES(?1,?2,?3,?4,?5,?6,'ACTIVE')")?;
    for (index, value) in [
        owner.binding_id,
        owner.instance_id,
        owner.domain_id,
        owner.kind,
        owner.owner_id,
        owner.generation,
    ]
    .iter()
    .enumerate()
    {
        row.bind_text((index + 1) as i32, value)?;
    }
    row.step_done()?;
    Ok(owner.binding_id.to_owned())
}

pub(crate) fn verify_home_owner_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    instance_id: &str,
    domain_id: &str,
    kind: &str,
    owner_id: &str,
    generation: &str,
) -> Result<String, AdmissionError> {
    binding_ref(
        connection,
        instance_id,
        domain_id,
        kind,
        owner_id,
        generation,
        true,
    )
}
fn binding_ref(
    connection: &VerifiedDatabaseConnection<'_>,
    instance_id: &str,
    domain_id: &str,
    kind: &str,
    owner_id: &str,
    generation: &str,
    active_only: bool,
) -> Result<String, AdmissionError> {
    let row = Statement::prepare(
        connection.as_ptr(),
        if active_only {
            "SELECT binding_id FROM gogoke_v37_h_owner_binding WHERE instance_id=?1 AND domain_id=?2 AND kind=?3 AND owner_id=?4 AND generation=?5 AND state='ACTIVE'"
        } else {
            "SELECT binding_id FROM gogoke_v37_h_owner_binding WHERE instance_id=?1 AND domain_id=?2 AND kind=?3 AND owner_id=?4 AND generation=?5"
        },
    )?;
    for (index, value) in [instance_id, domain_id, kind, owner_id, generation]
        .iter()
        .enumerate()
    {
        row.bind_text((index + 1) as i32, value)?;
    }
    if !row.step_row()? {
        return Err(AdmissionError::Denied);
    }
    Ok(row.column_text(0)?)
}

/// Revocation makes future home creation and admission fail, without erasing
/// already committed custody or stop facts.
pub(crate) fn revoke_owner_binding_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    binding_id: &str,
) -> Result<(), AdmissionError> {
    let row = Statement::prepare(connection.as_ptr(),
        "UPDATE gogoke_v37_h_owner_binding SET state='REVOKED' WHERE binding_id=?1 AND state='ACTIVE'")?;
    row.bind_text(1, binding_id)?;
    row.step_done()?;
    if changes(connection)? != 1 {
        return Err(AdmissionError::Conflict);
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct TrustedLimits {
    pub(crate) project_parallel: i64,
    pub(crate) instance_concurrency: i64,
}
pub(crate) struct AdmissionRequest<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) request_id: &'a str,
    pub(crate) raw_bytes: &'a [u8],
    pub(crate) instance_id: &'a str,
    pub(crate) home_id: &'a str,
    pub(crate) generation: &'a str,
    pub(crate) expected_revision: i64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdmissionResult {
    Applied(i64),
    Replayed(i64),
    Unknown,
    Conflict,
    Stale,
}

fn prior(
    connection: &VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    operation: &str,
) -> Result<Option<AdmissionResult>, AdmissionError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT raw_hex,operation,session_id,status,revision FROM gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2")?;
    row.bind_text(1, input.domain_id)?;
    row.bind_text(2, input.request_id)?;
    if !row.step_row()? {
        return Ok(None);
    }
    if row.column_text(0)? != hex(input.raw_bytes)
        || row.column_text(1)? != operation
        || row.column_text(2)? != input.session_id
    {
        return Ok(Some(AdmissionResult::Conflict));
    }
    if row.column_text(3)? == "UNKNOWN" {
        return Ok(Some(AdmissionResult::Unknown));
    }
    let revision = row
        .column_text(4)?
        .parse::<i64>()
        .map_err(|_| AdmissionError::Unknown)?;
    Ok(Some(AdmissionResult::Replayed(revision)))
}
fn journal(
    connection: &VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    operation: &str,
    before: i64,
    after: i64,
) -> Result<(), AdmissionError> {
    let row = Statement::prepare(connection.as_ptr(),
        "INSERT INTO gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES(?1,?2,?3,?4,?5,'APPLIED',?6,?7)")?;
    let raw = hex(input.raw_bytes);
    for (index, value) in [
        input.domain_id,
        input.request_id,
        &raw,
        operation,
        input.session_id,
    ]
    .iter()
    .enumerate()
    {
        row.bind_text((index + 1) as i32, value)?;
    }
    row.bind_i64(6, before)?;
    row.bind_i64(7, after)?;
    row.step_done()?;
    Ok(())
}
fn claim(
    connection: &VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
) -> Result<Option<(String, i64, String, String, String)>, AdmissionError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT state,revision,instance_id,home_id,generation FROM gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2")?;
    row.bind_text(1, input.domain_id)?;
    row.bind_text(2, input.session_id)?;
    if !row.step_row()? {
        return Ok(None);
    }
    Ok(Some((
        row.column_text(0)?,
        row.column_text(1)?
            .parse()
            .map_err(|_| AdmissionError::Unknown)?,
        row.column_text(2)?,
        row.column_text(3)?,
        row.column_text(4)?,
    )))
}

/// The authorization closure must read the current native issuer/E/F facts on
/// this same connection, inside BEGIN IMMEDIATE. No wire capacity or caller bool.
pub(crate) fn reserve_admission(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    authorize: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<TrustedLimits, AdmissionError>,
) -> Result<AdmissionResult, AdmissionError> {
    reserve_admission_inner(connection,input,false,|db,_|authorize(db))
}

/// Native ingress may seal a new selection only for a genuinely empty H slot.
/// Replays and conflicts still pass through the caller's current authorization.
pub(crate) fn reserve_native_admission(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    authorize: impl FnOnce(&mut VerifiedDatabaseConnection<'_>,bool) -> Result<TrustedLimits, AdmissionError>,
) -> Result<AdmissionResult, AdmissionError> {
    reserve_admission_inner(connection,input,true,authorize)
}

fn native_slot_empty(connection:&VerifiedDatabaseConnection<'_>,
    input:&AdmissionRequest<'_>)->Result<bool,AdmissionError> {
    if claim(connection,input)?.is_some()
        || count(connection,"SELECT COUNT(*) FROM main.gogoke_v37_h_seat_binding WHERE domain_id=?1 AND session_id=?2",
            &[input.domain_id,input.session_id])?!=0
        || count(connection,"SELECT COUNT(*) FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2",
            &[input.domain_id,input.session_id])?!=0
        || super::session_binding::read_pending(connection,input.domain_id,input.session_id)
            .map_err(AdmissionError::Relationship)?.is_some()
        || super::session_binding::read(connection,input.domain_id,input.session_id)
            .map_err(AdmissionError::Relationship)?.is_some() {
        return Ok(false);
    }
    Ok(true)
}

fn reserve_admission_inner(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    native:bool,
    authorize: impl FnOnce(&mut VerifiedDatabaseConnection<'_>,bool) -> Result<TrustedLimits, AdmissionError>,
) -> Result<AdmissionResult, AdmissionError> {
    if input.expected_revision != 0 || input.raw_bytes.is_empty() || input.raw_bytes.len() > 65_536
    {
        return Err(AdmissionError::Invalid("reserve"));
    }
    for (name, value) in [
        ("domain_id", input.domain_id),
        ("session_id", input.session_id),
        ("request_id", input.request_id),
        ("instance_id", input.instance_id),
        ("home_id", input.home_id),
        ("generation", input.generation),
    ] {
        require(value, name)?;
    }
    in_transaction(connection, |connection| {
        let original=prior(connection,input,"admission-reserve")?;
        let fresh=original.is_none() && (!native || native_slot_empty(connection,input)?);
        let limits = authorize(connection,fresh)?;
        if let Some(result) = original {
            return Ok(result);
        }
        if native && !fresh {return Ok(AdmissionResult::Conflict);}
        if limits.project_parallel <= 0 || limits.instance_concurrency <= 0 {
            return Err(AdmissionError::UnsupportedCapacity);
        }
        if claim(connection, input)?.is_some() {
            return Ok(AdmissionResult::Conflict);
        }
        let binding = verify_home_owner_in_transaction(
            connection,
            input.instance_id,
            input.domain_id,
            "SESSION",
            input.session_id,
            input.generation,
        )?;
        if count(connection,
            "SELECT COUNT(*) FROM gogoke_v37_instance_homes WHERE home_id=?1 AND instance_id=?2 AND domain_id=?3 AND kind='SESSION' AND owner_id=?4 AND generation=?5 AND state='ACTIVE'",
            &[input.home_id,input.instance_id,input.domain_id,input.session_id,input.generation])? != 1 {
            return Err(AdmissionError::Denied);
        }
        if count(
            connection,
            "SELECT COUNT(*) FROM gogoke_v37_h_home_fence WHERE home_id=?1",
            &[input.home_id],
        )? != 0
        {
            return Err(AdmissionError::Denied);
        }
        let project_active = count(connection,
            "SELECT COUNT(*) FROM gogoke_v37_h_claim WHERE domain_id=?1 AND state IN ('RESERVED','COMMITTED','STOPPED','UNKNOWN')",
            &[input.domain_id])?;
        let instance_active = count(connection,
            "SELECT COUNT(*) FROM gogoke_v37_h_claim WHERE instance_id=?1 AND state IN ('RESERVED','COMMITTED','STOPPED','UNKNOWN')",
            &[input.instance_id])?;
        if project_active >= limits.project_parallel
            || instance_active >= limits.instance_concurrency
        {
            return Err(AdmissionError::CapacityExceeded {
                project_active, project_cap: limits.project_parallel,
                instance_active, instance_cap: limits.instance_concurrency,
            });
        }
        let row = Statement::prepare(connection.as_ptr(),
            "INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES(?1,?2,?3,?4,?5,?6,'RESERVED',1)")?;
        for (index, value) in [
            input.domain_id,
            input.session_id,
            input.instance_id,
            input.home_id,
            &binding,
            input.generation,
        ]
        .iter()
        .enumerate()
        {
            row.bind_text((index + 1) as i32, value)?;
        }
        row.step_done()?;
        journal(connection, input, "admission-reserve", 0, 1)?;
        Ok(AdmissionResult::Applied(1))
    })
}

fn transition(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    operation: &str,
    from: &str,
    to: &str,
    authorize: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<(), AdmissionError>,
) -> Result<AdmissionResult, AdmissionError> {
    in_transaction(connection, |connection| {
        authorize(connection)?;
        if let Some(result) = prior(connection, input, operation)? {
            return Ok(result);
        }
        let Some((state, revision, instance_id, home_id, generation)) = claim(connection, input)?
        else {
            return Ok(AdmissionResult::Conflict);
        };
        if instance_id != input.instance_id
            || home_id != input.home_id
            || generation != input.generation
        {
            return Ok(AdmissionResult::Conflict);
        }
        if count(connection,
            "SELECT COUNT(*) FROM gogoke_v37_h_claim AS a JOIN gogoke_v37_h_owner_binding AS b ON b.binding_id=a.binding_id WHERE a.domain_id=?1 AND a.session_id=?2 AND b.state='ACTIVE'",
            &[input.domain_id,input.session_id])? != 1 { return Err(AdmissionError::Denied); }
        if count(connection,
            "SELECT COUNT(*) FROM gogoke_v37_instance_homes WHERE home_id=?1 AND instance_id=?2 AND domain_id=?3 AND kind='SESSION' AND owner_id=?4 AND generation=?5 AND state='ACTIVE'",
            &[input.home_id,input.instance_id,input.domain_id,input.session_id,input.generation])? != 1 ||
            count(connection,"SELECT COUNT(*) FROM gogoke_v37_h_home_fence WHERE home_id=?1",
                &[input.home_id])? != 0 { return Err(AdmissionError::Denied); }
        if input.expected_revision != revision {
            return Ok(AdmissionResult::Stale);
        }
        if state != from {
            return Ok(AdmissionResult::Conflict);
        }
        let row = Statement::prepare(connection.as_ptr(),
            "UPDATE gogoke_v37_h_claim SET state=?1,revision=revision+1 WHERE domain_id=?2 AND session_id=?3 AND state=?4 AND revision=?5")?;
        for (index, value) in [to, input.domain_id, input.session_id, from]
            .iter()
            .enumerate()
        {
            row.bind_text((index + 1) as i32, value)?;
        }
        row.bind_i64(5, revision)?;
        row.step_done()?;
        if changes(connection)? != 1 {
            return Err(AdmissionError::Conflict);
        }
        journal(connection, input, operation, revision, revision + 1)?;
        Ok(AdmissionResult::Applied(revision + 1))
    })
}
pub(crate) fn commit_admission(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    authorize: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<(), AdmissionError>,
) -> Result<AdmissionResult, AdmissionError> {
    transition(
        connection,
        input,
        "admission-commit",
        "RESERVED",
        "COMMITTED",
        authorize,
    )
}
pub(crate) fn release_admission(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    authorize: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<(), AdmissionError>,
) -> Result<AdmissionResult, AdmissionError> {
    release_admission_inner(connection,input,authorize,false)
}

/// The native Owner may release its exact COMMITTED admission when H proves
/// that process launch never crossed the durable open-intent boundary. This
/// releases admission only; credential or history preparation custody follows
/// its own lifecycle and is neither settled nor changed here. This is still the
/// ordinary Owner release operation: authorization, claim identity, generation,
/// revision, home, seat binding and active-generation checks are performed by
/// the same transaction below. Model-originated release keeps using
/// `release_admission` and cannot take this branch.
pub(crate) fn release_unstarted_owner_commit(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    authorize: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<(), AdmissionError>,
) -> Result<AdmissionResult, AdmissionError> {
    release_admission_inner(connection,input,authorize,true)
}

/// C's original failed FRESH preparation can return an unstarted COMMITTED
/// reservation only when H proves no launch intent or physical custody exists.
/// The private caller must bind this to its frozen Host recipient first.
pub(crate) fn release_unstarted_host_commit(
    connection:&mut VerifiedDatabaseConnection<'_>,input:&AdmissionRequest<'_>,
    authorize:impl FnOnce(&mut VerifiedDatabaseConnection<'_>)->Result<(),AdmissionError>,
) -> Result<AdmissionResult,AdmissionError> {
    release_admission_inner(connection,input,authorize,true)
}

fn release_admission_inner(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    authorize: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<(), AdmissionError>,
    allow_unstarted_commit: bool,
) -> Result<AdmissionResult, AdmissionError> {
    in_transaction(connection, |connection| {
        authorize(connection)?;
        if let Some(result) = prior(connection, input, "admission-release")? {
            return Ok(result);
        }
        let Some((state, revision, instance_id, home_id, generation)) = claim(connection, input)?
        else {
            return Ok(AdmissionResult::Conflict);
        };
        if instance_id != input.instance_id
            || home_id != input.home_id
            || generation != input.generation
        {
            return Ok(AdmissionResult::Conflict);
        }
        if input.expected_revision != revision {
            return Ok(AdmissionResult::Stale);
        }
        if state != "RESERVED" && state != "STOPPED" &&
            !(allow_unstarted_commit && state=="COMMITTED") {
            return Ok(AdmissionResult::Conflict);
        }
        if state=="COMMITTED" {
            // No process episode, custody or H open intent may exist. A
            // crashed or uncertain launch stays COMMITTED/UNKNOWN for readback.
            if count(connection,
                "SELECT COUNT(*) FROM gogoke_v37_h_claim a WHERE a.domain_id=?1
                   AND a.session_id=?2 AND a.state='COMMITTED' AND a.process_operation_id IS NULL",
                &[input.domain_id,input.session_id])?!=1 ||
                count(connection,
                "SELECT COUNT(*) FROM gogoke_v37_h_process_episode WHERE domain_id=?1 AND session_id=?2",
                &[input.domain_id,input.session_id])?!=0 ||
                count(connection,
                "SELECT COUNT(*) FROM gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2 AND operation='open'",
                &[input.domain_id,input.session_id])?!=0 {
                return Ok(AdmissionResult::Conflict);
            }
        }
        if super::generation_change::active_for_session(connection,input.domain_id,input.session_id)
            .map_err(AdmissionError::Store)?.is_some() {
            return Ok(AdmissionResult::Conflict);
        }
        let row = Statement::prepare(connection.as_ptr(),
            "UPDATE gogoke_v37_h_claim SET state='RELEASED',revision=revision+1 WHERE domain_id=?1 AND session_id=?2 AND state=?3 AND revision=?4")?;
        row.bind_text(1, input.domain_id)?;
        row.bind_text(2, input.session_id)?;
        row.bind_text(3, &state)?;
        row.bind_i64(4, revision)?;
        row.step_done()?;
        if changes(connection)? != 1 {
            return Err(AdmissionError::Conflict);
        }
        // A product seat becomes IDLE only in the transaction that releases
        // its unstarted reservation or its already proven STOPPED claim.
        let binding = Statement::prepare(connection.as_ptr(),
            "SELECT seat_id,seat_incarnation,seat_authorization_generation,selected_instance_id FROM main.gogoke_v37_effective_seat WHERE domain_id=?1 AND session_id=?2 AND generation=?3")?;
        binding.bind_text(1, input.domain_id)?;
        binding.bind_text(2, input.session_id)?;
        binding.bind_text(3, input.generation)?;
        if binding.step_row()? {
            let seat_id = binding.column_text(0)?;
            let incarnation = binding.column_text(1)?;
            let authorization_generation=binding.column_text(2)?.parse::<i64>()
                .map_err(|_|AdmissionError::Denied)?;
            let selected_instance=binding.column_text(3)?;
            if binding.step_row()? { return Err(AdmissionError::Conflict); }
            let seat = crate::store::seat::get(connection, input.domain_id, &seat_id)
                .map_err(AdmissionError::Seat)?.ok_or(AdmissionError::Denied)?;
            if seat.incarnation != incarnation || seat.generation!=authorization_generation
                || selected_instance != input.instance_id || seat.instance_id != selected_instance
                || seat.state != crate::store::seat::State::Busy {
                return Err(AdmissionError::Denied);
            }
            // E belongs to the seat, not this physical H episode. Releasing
            // one native session cannot revoke a sibling's still-held scope.
            let siblings=super::session_binding::has_unreleased_seat_claim(
                connection,input.domain_id,&seat_id,&incarnation)
                .map_err(AdmissionError::Relationship)?;
            if !siblings {
                crate::store::seat::set_dispatch_state_in_transaction(connection, &seat, false)
                    .map_err(AdmissionError::Seat)?;
            }
        } else {return Err(AdmissionError::Denied);}
        journal(
            connection,
            input,
            "admission-release",
            revision,
            revision + 1,
        )?;
        Ok(AdmissionResult::Applied(revision + 1))
    })
}

/// The native launcher binds its already-persisted custody operation before
/// the process is exposed. No wire field can create this relationship.
pub(crate) fn bind_process_operation_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    session_id: &str,
    process_operation_id: &str,
) -> Result<(), AdmissionError> {
    if count(connection,
        "SELECT COUNT(*) FROM gogoke_coordination_process_custody AS c JOIN gogoke_v37_h_claim AS a ON a.domain_id=c.domain_id AND a.generation=c.generation WHERE a.domain_id=?1 AND a.session_id=?2 AND c.operation_id=?3 AND c.state IN ('PREPARED','ACTIVE') AND a.state='COMMITTED' AND a.process_operation_id IS NULL",
        &[domain_id,session_id,process_operation_id])? != 1 { return Err(AdmissionError::Denied); }
    let row = Statement::prepare(connection.as_ptr(),
        "UPDATE gogoke_v37_h_claim SET process_operation_id=?1 WHERE domain_id=?2 AND session_id=?3 AND state='COMMITTED' AND process_operation_id IS NULL")?;
    row.bind_text(1, process_operation_id)?;
    row.bind_text(2, domain_id)?;
    row.bind_text(3, session_id)?;
    row.step_done()?;
    if changes(connection)? != 1 {
        return Err(AdmissionError::Conflict);
    }
    Ok(())
}

/// Native launch uncertainty is durable under the original request ID. A
/// second request ID cannot start this claim while its outcome is unresolved.
pub(crate) fn mark_start_unknown_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    process_operation_id: &str,
) -> Result<(), AdmissionError> {
    let next_revision = input
        .expected_revision
        .checked_add(1)
        .ok_or(AdmissionError::Invalid("revision overflow"))?;
    if count(connection,
        "SELECT COUNT(*) FROM gogoke_v37_h_claim AS a JOIN gogoke_coordination_process_custody AS c ON c.operation_id=a.process_operation_id AND c.domain_id=a.domain_id AND c.generation=a.generation WHERE a.domain_id=?1 AND a.session_id=?2 AND a.instance_id=?3 AND a.home_id=?4 AND a.generation=?5 AND a.process_operation_id=?6 AND a.state='COMMITTED' AND c.state='UNKNOWN'",
        &[input.domain_id,input.session_id,input.instance_id,input.home_id,input.generation,
          process_operation_id])? != 1 { return Err(AdmissionError::Denied); }
    let row = Statement::prepare(connection.as_ptr(),
        "UPDATE gogoke_v37_h_claim SET state='UNKNOWN',revision=revision+1 WHERE domain_id=?1 AND session_id=?2 AND state='COMMITTED' AND revision=?3")?;
    row.bind_text(1, input.domain_id)?;
    row.bind_text(2, input.session_id)?;
    row.bind_i64(3, input.expected_revision)?;
    row.step_done()?;
    if changes(connection)? != 1 {
        return Err(AdmissionError::Stale);
    }
    let operation = Statement::prepare(connection.as_ptr(),
        "INSERT INTO gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES(?1,?2,?3,'open',?4,'UNKNOWN',?5,?6)")?;
    let raw = hex(input.raw_bytes);
    operation.bind_text(1, input.domain_id)?;
    operation.bind_text(2, input.request_id)?;
    operation.bind_text(3, &raw)?;
    operation.bind_text(4, input.session_id)?;
    operation.bind_i64(5, input.expected_revision)?;
    operation.bind_i64(6, next_revision)?;
    operation.step_done()?;
    Ok(())
}

/// Accepts only an already persisted native custody STOPPED fact. This cannot
/// be made true by a wire `stopped` field or a Node-side callback.
pub(crate) fn record_session_stop_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    session_id: &str,
    process_operation_id: &str,
) -> Result<String, AdmissionError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT c.stop_proof_hash FROM gogoke_coordination_process_custody AS c JOIN gogoke_v37_h_claim AS a ON a.process_operation_id=c.operation_id AND a.domain_id=c.domain_id AND a.generation=c.generation WHERE a.domain_id=?1 AND a.session_id=?2 AND c.operation_id=?3 AND c.state='STOPPED' AND c.stop_proof_hash IS NOT NULL AND a.state IN ('COMMITTED','UNKNOWN')")?;
    row.bind_text(1, domain_id)?;
    row.bind_text(2, session_id)?;
    row.bind_text(3, process_operation_id)?;
    if !row.step_row()? {
        return Err(AdmissionError::Denied);
    }
    let proof = row.column_text(0)?;
    drop(row);
    let update = Statement::prepare(connection.as_ptr(),
        "UPDATE gogoke_v37_h_claim SET state='STOPPED',revision=revision+1,stop_fact_id=?1 WHERE domain_id=?2 AND session_id=?3 AND state IN ('COMMITTED','UNKNOWN') AND process_operation_id=?4")?;
    update.bind_text(1, &proof)?;
    update.bind_text(2, domain_id)?;
    update.bind_text(3, session_id)?;
    update.bind_text(4, process_operation_id)?;
    update.step_done()?;
    if changes(connection)? != 1 {
        return Err(AdmissionError::Conflict);
    }
    super::episodes::mark_stopped(connection,process_operation_id,&proof)
        .map_err(AdmissionError::Store)?;
    let resolve = Statement::prepare(connection.as_ptr(),
        "UPDATE gogoke_v37_h_operation SET status='APPLIED',revision=(SELECT revision FROM gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2) WHERE domain_id=?1 AND session_id=?2 AND operation='open' AND status='UNKNOWN'")?;
    resolve.bind_text(1, domain_id)?;
    resolve.bind_text(2, session_id)?;
    resolve.step_done()?;
    Ok(proof)
}

/// An original compact/renew request stops its old physical episode before
/// creating the next one. The compound request owns the sole public revision
/// change; this internal physical stop never impersonates a wire stop.
pub(crate) fn record_generation_change_stop_in_transaction(
    connection:&mut VerifiedDatabaseConnection<'_>,domain:&str,session:&str,
    process:&str,change_id:&str,
)->Result<String,AdmissionError> {
    let change=super::generation_change::read(connection,domain,change_id)
        .map_err(AdmissionError::Store)?.ok_or(AdmissionError::Conflict)?;
    if change.session_id!=session || change.old_operation!=process
        || change.owner_stop_request_id.is_some()
        || !matches!(change.stage.as_str(),"INTENT"|"ITEM_OBSERVED") {
        return Err(AdmissionError::Conflict);
    }
    let row=Statement::prepare(connection.as_ptr(),
        "SELECT c.stop_proof_hash FROM main.gogoke_coordination_process_custody c
           JOIN main.gogoke_v37_h_claim a ON a.process_operation_id=c.operation_id
             AND a.domain_id=c.domain_id AND a.generation=c.generation
          WHERE a.domain_id=?1 AND a.session_id=?2 AND a.generation=?3
            AND a.revision=?4 AND a.state='COMMITTED'
            AND c.operation_id=?5 AND c.state='STOPPED' AND c.stop_proof_hash IS NOT NULL")?;
    row.bind_text(1,domain)?;row.bind_text(2,session)?;
    row.bind_text(3,&change.old_generation)?;
    row.bind_i64(4,change.unknown_revision.unwrap_or(change.previous_revision))?;
    row.bind_text(5,process)?;
    if !row.step_row()? {return Err(AdmissionError::Denied);}
    let proof=row.column_text(0)?;
    if row.step_row()? {return Err(AdmissionError::Conflict);}
    drop(row);
    let claim=Statement::prepare(connection.as_ptr(),
        "UPDATE main.gogoke_v37_h_claim SET state='STOPPED',stop_fact_id=?4
          WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3
            AND state='COMMITTED' AND stop_fact_id IS NULL")?;
    claim.bind_text(1,domain)?;claim.bind_text(2,session)?;
    claim.bind_text(3,process)?;claim.bind_text(4,&proof)?;
    claim.step_done()?;
    if changes(connection)?!=1 {return Err(AdmissionError::Conflict);}
    super::episodes::mark_stopped(connection,process,&proof).map_err(AdmissionError::Store)?;
    super::generation_change::mark_old_stopped(connection,domain,change_id)
        .map_err(AdmissionError::Store)?;
    Ok(proof)
}

pub(crate) fn verify_home_stop_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    instance_id: &str,
    domain_id: &str,
    kind: &str,
    owner_id: &str,
    generation: &str,
) -> Result<String, AdmissionError> {
    binding_ref(
        connection,
        instance_id,
        domain_id,
        kind,
        owner_id,
        generation,
        false,
    )?;
    if kind == "CALL" {
        return Err(AdmissionError::Unknown);
    }
    if kind != "SESSION" {
        return Err(AdmissionError::Invalid("kind"));
    }
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT a.stop_fact_id FROM gogoke_v37_h_process_episode AS a
           JOIN gogoke_v37_h_generation AS g ON g.domain_id=a.domain_id
             AND g.session_id=a.session_id AND g.generation=a.generation
             AND g.process_operation_id=a.process_operation_id
           JOIN gogoke_coordination_process_custody AS c
             ON c.operation_id=a.process_operation_id AND c.domain_id=a.domain_id
             AND c.generation=a.generation
          WHERE a.instance_id=?1 AND a.domain_id=?2 AND a.session_id=?3
            AND a.generation=?4 AND a.phase='STOPPED' AND c.state='STOPPED'
            AND c.stop_proof_hash=a.stop_fact_id AND a.stop_fact_id IS NOT NULL")?;
    for (index, value) in [instance_id, domain_id, owner_id, generation]
        .iter()
        .enumerate()
    {
        row.bind_text((index + 1) as i32, value)?;
    }
    if !row.step_row()? {
        return Err(AdmissionError::Denied);
    }
    Ok(row.column_text(0)?)
}

/// Called by F inside its cleanup transaction. The inserted fence survives
/// restart and makes every later reserve for this exact home fail closed.
pub(crate) fn fence_home_admission_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    home_id: &str,
    instance_id: &str,
    domain_id: &str,
    kind: &str,
    owner_id: &str,
    generation: &str,
) -> Result<String, AdmissionError> {
    verify_home_stop_in_transaction(
        connection,
        instance_id,
        domain_id,
        kind,
        owner_id,
        generation,
    )?;
    let existing = Statement::prepare(connection.as_ptr(),
        "SELECT instance_id,domain_id,kind,owner_id,generation,fence_id FROM gogoke_v37_h_home_fence WHERE home_id=?1")?;
    existing.bind_text(1, home_id)?;
    if existing.step_row()? {
        let matches = [instance_id, domain_id, kind, owner_id, generation]
            .iter()
            .enumerate()
            .try_fold(
                true,
                |same, (index, value)| -> Result<bool, AdmissionError> {
                    Ok(same && existing.column_text(index as i32)? == *value)
                },
            )?;
        return if matches {
            Ok(existing.column_text(5)?)
        } else {
            Err(AdmissionError::Conflict)
        };
    }
    drop(existing);
    if count(connection,
        "SELECT COUNT(*) FROM gogoke_v37_instance_homes WHERE home_id=?1 AND instance_id=?2 AND domain_id=?3 AND kind=?4 AND owner_id=?5 AND generation=?6 AND state IN ('CLOSED','CLEANUP_UNKNOWN')",
        &[home_id,instance_id,domain_id,kind,owner_id,generation])? != 1 {
        return Err(AdmissionError::Denied);
    }
    if count(connection,
        "SELECT COUNT(*) FROM gogoke_v37_h_claim WHERE home_id=?1 AND state IN ('RESERVED','COMMITTED','STOPPED','UNKNOWN')",
        &[home_id])? != 0 { return Err(AdmissionError::Denied); }
    let fence_id = format!("h-fence-{home_id}");
    let row = Statement::prepare(connection.as_ptr(),
        "INSERT INTO gogoke_v37_h_home_fence(home_id,instance_id,domain_id,kind,owner_id,generation,fence_id) VALUES(?1,?2,?3,?4,?5,?6,?7)")?;
    for (index, value) in [
        home_id,
        instance_id,
        domain_id,
        kind,
        owner_id,
        generation,
        &fence_id,
    ]
    .iter()
    .enumerate()
    {
        row.bind_text((index + 1) as i32, value)?;
    }
    row.step_done()?;
    Ok(fence_id)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, open_existing, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn eighth_table_upgrade_adds_generation_change_and_alias_without_rewriting_old_rows() {
        let _guard=route_b_test_guard();
        let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder=std::env::temp_dir().join(format!("gogoke-h-change-schema-{}-{stamp}",std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let root=RootLock::acquire(&folder).unwrap();
        let path=folder.join("state.sqlite");
        let mut db=create_new(&root,&path).unwrap();
        for (_,sql) in &SCHEMA[..8] {db.execute(sql).unwrap();}
        db.execute("INSERT INTO gogoke_v37_h_owner_binding(binding_id,instance_id,domain_id,kind,owner_id,generation,state) VALUES('binding','instance','domain','SESSION','session','1','ACTIVE')").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('domain','session','instance','home','binding','1','COMMITTED',3,'original-process')").unwrap();
        let before=observed_schema(&db).unwrap();
        initialize_admission_schema(&mut db).unwrap();
        assert_eq!(observed_schema(&db).unwrap().len(),10);
        assert_eq!(observed_schema(&db).unwrap().into_iter().filter(|(name,_)|
            name!="gogoke_v37_h_generation_change"&&name!="gogoke_v37_h_dos_alias")
            .collect::<Vec<_>>(),before);
        assert_eq!(count(&db,"SELECT COUNT(*) FROM gogoke_v37_h_claim WHERE process_operation_id=?1",
            &["original-process"]).unwrap(),1);
        initialize_admission_schema(&mut db).unwrap();
        db.execute("CREATE TEMP TABLE gogoke_v37_h_generation_change_shadow(x TEXT)").unwrap();
        assert!(matches!(initialize_admission_schema(&mut db),Err(AdmissionError::Denied)));
        db.close_checked().unwrap();drop(root);std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn exact_nine_table_migration_adds_only_the_h_alias_record() {
        let _guard=route_b_test_guard();
        let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder=std::env::temp_dir().join(format!("gogoke-h-alias-schema-{}-{stamp}",std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let root=RootLock::acquire(&folder).unwrap();
        let path=folder.join("state.sqlite");
        let mut db=create_new(&root,&path).unwrap();
        for(_,sql)in &SCHEMA[..9]{db.execute(sql).unwrap();}
        db.execute("INSERT INTO gogoke_v37_h_owner_binding(binding_id,instance_id,domain_id,kind,owner_id,generation,state) VALUES('binding','instance','domain','SESSION','session','1','ACTIVE')").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('domain','session','instance','home','binding','1','UNKNOWN',4,'original-process')").unwrap();
        let before=observed_schema(&db).unwrap();
        assert_eq!(before.len(),9);
        assert_eq!(count(&db,"SELECT COUNT(*) FROM gogoke_v37_h_claim WHERE process_operation_id=?1",
            &["original-process"]).unwrap(),1);
        initialize_admission_schema(&mut db).unwrap();
        let after=observed_schema(&db).unwrap();
        assert_eq!(after.len(),10);
        assert_eq!(after.into_iter().filter(|(name,_)|name!="gogoke_v37_h_dos_alias")
            .collect::<Vec<_>>(),before);
        assert_eq!(count(&db,"SELECT COUNT(*) FROM gogoke_v37_h_claim WHERE process_operation_id=?1",
            &["original-process"]).unwrap(),1);
        initialize_admission_schema(&mut db).unwrap();
        db.close_checked().unwrap();drop(root);std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn opencode_alias_reservation_and_original_c_custody_are_exact() {
        let _guard=route_b_test_guard();
        let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder=std::env::temp_dir().join(format!(
            "gogoke-h-alias-reserve-{}-{stamp}",std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let root=RootLock::acquire(&folder).unwrap();
        let mut db=create_new(&root,&folder.join("state.sqlite")).unwrap();
        initialize_admission_schema(&mut db).unwrap();
        let f=folder.join("f");
        std::fs::create_dir(&f).unwrap();
        let f_path=f.to_str().unwrap();
        let identity=crate::root::inspect_root(&f).unwrap().identity;
        let request=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"open","requestId":"request","targetId":"session","domainId":"domain","expectedRevision":"0","payload":{}}"#;
        let process_operation=format!("h-open-{}",&crate::store::digest::sha256_hex(request)[..40]);
        let raw_hex=hex(request);
        db.execute(&format!("INSERT INTO gogoke_v37_h_operation(
            domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision)
            VALUES('domain','request','{raw_hex}','open','session','UNKNOWN',0,1)")).unwrap();
        db.execute(&format!("INSERT INTO gogoke_v37_h_process_episode(
            domain_id,request_id,session_id,generation,old_generation,raw_hex,
            previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,
            seat_id,seat_incarnation,phase,stop_fact_id)
            VALUES('domain','request','session','generation',NULL,'{raw_hex}',0,NULL,
            '{process_operation}','instance','home','binding',NULL,NULL,'INTENT',NULL)")).unwrap();
        assert_eq!(count(&db,"SELECT COUNT(*) FROM gogoke_v37_h_operation
            WHERE domain_id='domain' AND request_id='request' AND session_id='session'
              AND raw_hex=?1 AND operation='open' AND status='UNKNOWN'",&[&raw_hex]).unwrap(),1);
        assert_eq!(count(&db,"SELECT COUNT(*) FROM gogoke_v37_h_process_episode
            WHERE domain_id='domain' AND request_id='request' AND session_id='session'
              AND raw_hex=?1 AND generation='generation' AND phase='INTENT'",&[&raw_hex]).unwrap(),1);
        let source=DosAliasIntent{domain_id:"domain".into(),session_id:"session".into(),
            request_id:"request".into(),request_sha256:crate::store::digest::sha256_hex(request),
            generation:"generation".into(),origin_kind:"OPEN".into(),
            process_operation_id:process_operation.clone(),instance_id:"instance".into(),
            binary_digest_sha256:"sha256:abc".into(),binding_id:"binding".into(),
            worktree_id:"worktree".into(),namespace_id:"namespace".into(),alias_name:"Z:".into(),
            protocol_path:String::new(),requested_target:f_path.into(),observed_target:None,
            volume_serial:format!("{:016x}",identity.volume_serial),file_id:identity.file_id.iter()
                .map(|b|format!("{b:02x}")).collect(),host_pid:"1".into(),
            host_creation_time_100ns:"1".into(),phase:"INTENT".into(),
            child_operation_id:None,child_ticket:None,child_nonce:None,child_pid:None,
            child_creation_time_100ns:None,cleanup_status:"NOT_REQUESTED".into(),
            cleanup_result:None,revision:1};
        verify_opencode_alias_source(&db,&source).unwrap();

        assert!(matches!(reserve_opencode_dos_alias(&mut db,"domain","session","request",
            b"different raw bytes","generation",&process_operation,"instance","sha256:abc","binding",
            "worktree",f_path,&identity),Err(AdmissionError::Conflict)));
        let alias=reserve_opencode_dos_alias(&mut db,"domain","session","request",request,
            "generation",&process_operation,"instance","sha256:abc","binding","worktree",f_path,&identity)
            .unwrap_or_else(|error|panic!("matching durable H open intent should reserve alias: {error:?}"));
        assert_eq!(alias.origin_kind,"OPEN");
        assert_eq!(alias.phase,"INTENT");
        assert_eq!(alias.requested_target,f_path);
        assert_eq!(alias.file_id,identity.file_id.iter().map(|b|format!("{b:02x}"))
            .collect::<String>());

        let mapped=mark_opencode_dos_alias_mapped(&mut db,&alias,"fixture-observed-target").unwrap();
        assert_eq!(mapped.phase,"MAPPED");
        let started=mark_opencode_dos_alias_prepare_started(&mut db,&mapped).unwrap();
        assert_eq!(started.phase,"PREPARE_STARTED");
        db.execute("CREATE TABLE gogoke_coordination_process_custody(
            operation_id TEXT,ticket TEXT,custodian_nonce TEXT,pid TEXT,creation_time_100ns TEXT,
            image_path TEXT,binary_digest_sha256 TEXT,profile_id TEXT,domain_id TEXT,
            generation TEXT,state TEXT,stop_proof_hash TEXT)").unwrap();
        db.execute(&format!(r#"INSERT INTO gogoke_coordination_process_custody VALUES(
            '{process_operation}','child-ticket','custodian-nonce','42','123','C:\Program Files\fixture\cli.exe',
            'sha256:abc','instance','domain','generation','PREPARED',NULL)"#)).unwrap();
        let facts=AliasCustodyFacts{operation_id:&process_operation,ticket:"child-ticket",
            nonce:"custodian-nonce",pid:"42",creation:"123",image_path:r"C:\Program Files\fixture\cli.exe",
            binary_digest:"sha256:abc",profile_id:"instance",domain_id:"domain",
            generation:"generation",state:"PREPARED"};
        verify_original_alias_custody_row(&db,&started,&facts,"PREPARED").unwrap();
        let wrong=AliasCustodyFacts{ticket:"wrong-ticket",..facts};
        assert!(matches!(verify_original_alias_custody_row(&db,&started,&wrong,"PREPARED"),
            Err(AdmissionError::Conflict)),"a mismatched C receipt cannot bind into H");
        db.execute("UPDATE gogoke_coordination_process_custody SET state='STOPPED',stop_proof_hash='stop-1'").unwrap();
        db.execute("UPDATE gogoke_v37_h_process_episode SET phase='STOPPED',stop_fact_id='stop-1'").unwrap();
        db.execute(&format!("UPDATE gogoke_v37_h_dos_alias SET phase='CUSTODY_BOUND',
            child_operation_id='{process_operation}',child_ticket='child-ticket',
            child_nonce='custodian-nonce',child_pid='42',child_creation_time_100ns='123',
            revision=revision+1")).unwrap();
        let persisted=read_opencode_dos_alias(&db,"domain","session","generation")
            .unwrap().expect("persisted exact H alias record");
        assert!(matches!(verify_opencode_dos_alias_stop_source(&db,&persisted,"stop-1"),Ok(())),
            "exact C/H StopFact is enough without requiring the mutable current claim");
        assert!(matches!(verify_opencode_dos_alias_stop_source(&db,&persisted,"different-stop"),
            Err(AdmissionError::Denied)));
        db.close_checked().unwrap();drop(root);std::fs::remove_dir_all(folder).unwrap();
    }

    fn one_capacity(
        _: &mut VerifiedDatabaseConnection<'_>,
    ) -> Result<TrustedLimits, AdmissionError> {
        Ok(TrustedLimits {
            project_parallel: 1,
            instance_concurrency: 1,
        })
    }

    fn authorized(_: &mut VerifiedDatabaseConnection<'_>) -> Result<(), AdmissionError> {
        Ok(())
    }

    fn owner_unstarted_fixture(
        run: impl FnOnce(
            &RootLock,
            &mut VerifiedDatabaseConnection<'_>,
            &crate::store::authority::OwnerIssuer,
            &crate::store::seat::Seat,
        ),
    ) {
        use crate::store::same_open::create_new;
        use std::time::{SystemTime, UNIX_EPOCH};

        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder = std::env::temp_dir().join(format!(
            "gogoke-h-owner-unstarted-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let root = RootLock::acquire(&folder).unwrap();
        let database = folder.join("state.sqlite");
        let mut db = create_new(&root, &database).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        let owner = crate::store::authority::initialize_profile(&mut db, &root).unwrap();
        crate::store::authority::initialize_process_custody_schema(&mut db).unwrap();
        crate::store::instance::initialize_schema(&mut db).unwrap();
        crate::store::seat::initialize_schema(&mut db).unwrap();
        initialize_admission_schema(&mut db).unwrap();
        super::super::session_binding::initialize_schema(&mut db).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','homeRefA','homeIdentityA','sha256:test','0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,directory_ref,directory_identity,state,revision) VALUES('homeA','instanceA','projectA','SESSION','sessionA','2',NULL,NULL,'ACTIVE',1)").unwrap();
        crate::store::seat::store_template(&mut db,
            crate::store::seat::NativeOrigin::user(&owner),
            crate::store::seat::StoreTemplate {
                domain_id: "projectA", template_id: "templateA",
                settings_json: br#"{"instruction":"default"}"#,
            }).unwrap();
        let created = crate::store::seat::create(&mut db,
            crate::store::seat::NativeOrigin::user(&owner),
            crate::store::seat::CreateSeat {
                domain_id: "projectA", seat_id: "seatA", template_id: "templateA",
                instance_id: Some("instanceA"), kind: crate::store::seat::Kind::Long,
                request_id: "createA", request_bytes: b"create seat A",
            }).unwrap().seat;
        let busy = crate::store::seat::set_dispatch_state(&mut db, &created, true).unwrap();
        in_transaction(&mut db, |connection| {
            bind_owner_in_transaction(connection, &OwnerBinding {
                binding_id: "bindingA", instance_id: "instanceA", domain_id: "projectA",
                kind: "SESSION", owner_id: "sessionA", generation: "2",
            })?;
            bind_seat_in_transaction(connection, &busy, "sessionA")?;
            Ok(())
        }).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('projectA','sessionA','instanceA','homeA','bindingA','2','COMMITTED',3,NULL)").unwrap();

        run(&root, &mut db, &owner, &busy);
        db.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(&database).unwrap();
        std::fs::remove_dir(&folder).unwrap();
    }

    fn owner_release_request(expected_revision: i64) -> AdmissionRequest<'static> {
        AdmissionRequest {
            domain_id: "projectA", session_id: "sessionA", request_id: "ownerReleaseA",
            raw_bytes: b"owner admission release", instance_id: "instanceA", home_id: "homeA",
            generation: "2", expected_revision,
        }
    }

    #[test]
    fn legacy_reserve_replay_and_same_slot_new_id_never_select_native() {
        owner_unstarted_fixture(|_root,db,_owner,busy| {
            use super::super::session_binding::{self,Provenance,SessionBinding};
            let reserve=AdmissionRequest {domain_id:"projectA",session_id:"sessionA",
                request_id:"legacyReserveA",raw_bytes:b"original legacy reserve",
                instance_id:"instanceA",home_id:"homeA",generation:"2",expected_revision:0};
            let original=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA',?1,?2,'admission-reserve','sessionA','APPLIED',0,1)").unwrap();
            original.bind_text(1,reserve.request_id).unwrap();
            original.bind_text(2,&hex(reserve.raw_bytes)).unwrap();
            original.step_done().unwrap();drop(original);
            let attempt=|db:&mut VerifiedDatabaseConnection<'_>,new:bool| {
                if new {session_binding::select_native_in_transaction(db,&SessionBinding {
                    domain_id:"projectA".into(),session_id:"sessionA".into(),
                    seat_id:busy.seat_id.clone(),seat_incarnation:busy.incarnation.clone(),
                    seat_authorization_generation:busy.generation,
                    selected_instance_id:"instanceA".into(),provenance:Provenance::NativeV2,
                }).map_err(AdmissionError::Relationship)?;}
                one_capacity(db)
            };
            assert_eq!(reserve_native_admission(db,&reserve,attempt).unwrap(),
                AdmissionResult::Replayed(1));
            assert!(session_binding::read_pending(db,"projectA","sessionA").unwrap().is_none());
            let changed=AdmissionRequest {raw_bytes:b"changed original bytes",..reserve};
            assert_eq!(reserve_native_admission(db,&changed,attempt).unwrap(),
                AdmissionResult::Conflict);
            let new_id=AdmissionRequest {request_id:"differentReserveA",raw_bytes:b"other request",..reserve};
            assert_eq!(reserve_native_admission(db,&new_id,attempt).unwrap(),
                AdmissionResult::Conflict);
            assert!(session_binding::read_pending(db,"projectA","sessionA").unwrap().is_none());
        });
    }

    #[test]
    fn one_boundary_survives_reopen_unknown_replay_and_home_fence_pin_matrix() {
        use super::super::runtime::{require_new_reservation_supported, InstancePin};
        use crate::store::seat::PermissionTier;
        let fixed=InstancePin {driver_id:"codex".into(),version:"0.160.0".into(),
            digest:"sha256:fdda5fa3cf3fb3d000b876720742857676293e4315e4b045fae6f8bd7e866d1d".into()};
        for (tier, label) in [(PermissionTier::ReadOnly,"READ_ONLY"),
            (PermissionTier::NoNetwork,"NO_NETWORK")] {
            let refused=require_new_reservation_supported(&fixed,true,||Ok(tier));
            assert!(matches!(refused,Err(AdmissionError::Invalid(reason))
                if reason.contains(&format!("{label} LPAC unavailable"))
                    && reason.contains("GetComputerNameExW did not provide buffer size")
                    && reason.contains("GetLastError was not captured")));
        }
        assert!(require_new_reservation_supported(&fixed,false,||panic!("old replay read E tier")).is_ok());
        for changed in [InstancePin {driver_id:"claude".into(),..fixed.clone()},
            InstancePin {version:"0.160.1".into(),..fixed.clone()},
            InstancePin {digest:format!("sha256:{}","0".repeat(64)),..fixed.clone()}] {
            assert!(require_new_reservation_supported(&changed,true,
                ||panic!("unrelated pin read E tier")).is_ok());
            assert!(require_new_reservation_supported(&changed,true,
                ||Err(AdmissionError::Denied)).is_ok());
        }
        for tier in [PermissionTier::IsolatedWrite,PermissionTier::NetworkedWrite] {
            assert!(require_new_reservation_supported(&fixed,true,||Ok(tier)).is_ok());
        }
        assert!(matches!(require_new_reservation_supported(&fixed,true,
            ||Err(AdmissionError::Denied)),Err(AdmissionError::Denied)));
    }

    #[test]
    fn one_boundary_survives_reopen_unknown_replay_and_home_fence_unavailable_reserve_rolls_back() {
        for (tier,label) in [(crate::store::seat::PermissionTier::ReadOnly,"READ_ONLY"),
            (crate::store::seat::PermissionTier::NoNetwork,"NO_NETWORK")] {
        owner_unstarted_fixture(|_root,db,_owner,_busy| {
            db.execute("CREATE TABLE test_reserve_side_effect(value TEXT) STRICT").unwrap();
            let reserve=AdmissionRequest {domain_id:"projectA",session_id:"unavailableSession",
                request_id:"unavailableReserve",raw_bytes:b"fresh unavailable reserve",
                instance_id:"instanceA",home_id:"unavailableHome",generation:"3",
                expected_revision:0};
            let pin=super::super::runtime::InstancePin {driver_id:"codex".into(),
                version:"0.160.0".into(),digest:"sha256:fdda5fa3cf3fb3d000b876720742857676293e4315e4b045fae6f8bd7e866d1d".into()};
            let result=reserve_native_admission(db,&reserve,|db,new| {
                assert!(new);
                db.execute("INSERT INTO test_reserve_side_effect VALUES('would-be-selection')")?;
                super::super::runtime::require_new_reservation_supported(&pin,new,
                    || Ok(tier))?;
                one_capacity(db)
            });
            assert!(matches!(result,Err(AdmissionError::Invalid(reason))
                if reason.contains(&format!("{label} LPAC unavailable"))
                    && reason.contains("GetComputerNameExW did not provide buffer size")));
            for sql in ["SELECT COUNT(*) FROM test_reserve_side_effect",
                "SELECT COUNT(*) FROM gogoke_v37_h_claim WHERE session_id='unavailableSession'",
                "SELECT COUNT(*) FROM gogoke_v37_h_operation WHERE session_id='unavailableSession'",
                "SELECT COUNT(*) FROM gogoke_v37_h_seat_binding WHERE session_id='unavailableSession'",
                "SELECT COUNT(*) FROM gogoke_v37_native_selection WHERE session_id='unavailableSession'",
                "SELECT COUNT(*) FROM gogoke_v37_h_process_episode WHERE session_id='unavailableSession'"] {
                assert_eq!(count(db,sql,&[]).unwrap(),0,"{sql}");
            }
            let old=AdmissionRequest {domain_id:"projectA",session_id:"sessionA",
                request_id:"originalLegacyReserve",raw_bytes:b"original legacy reserve",
                instance_id:"instanceA",home_id:"homeA",generation:"2",expected_revision:0};
            let original=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA',?1,?2,'admission-reserve','sessionA','APPLIED',0,1)").unwrap();
            original.bind_text(1,old.request_id).unwrap();
            original.bind_text(2,&hex(old.raw_bytes)).unwrap();
            original.step_done().unwrap();drop(original);
            let replay=reserve_native_admission(db,&old,|db,new| {
                assert!(!new);
                let seat=crate::store::seat::get(db,"projectA","seatA")
                    .map_err(AdmissionError::Seat)?.ok_or(AdmissionError::Denied)?;
                assert!(crate::store::seat::permission_tier(&seat).is_err(),
                    "legacy fixture has no permission setting");
                super::super::runtime::require_new_reservation_supported(&pin,new,
                    || crate::store::seat::permission_tier(&seat).map_err(AdmissionError::Seat))?;
                one_capacity(db)
            });
            assert_eq!(replay.unwrap(),AdmissionResult::Replayed(1));
        });
        }
    }

    #[test]
    fn owner_release_settles_only_an_unstarted_committed_claim_without_a_stop_fact() {
        owner_unstarted_fixture(|_root, db, owner, busy| {
            let request = owner_release_request(3);
            assert_eq!(release_admission(db, &request, authorized).unwrap(),
                AdmissionResult::Conflict,
                "the ordinary admission release path remains closed for COMMITTED claims");
            let released_admission = super::super::runtime::release_native(
                db, &crate::store::seat::NativeOrigin::user(owner), &request).unwrap();
            assert_eq!(released_admission, AdmissionResult::Applied(4));
            assert_eq!(claim(db, &request).unwrap().unwrap().0, "RELEASED");
            assert_eq!(count(db,
                "SELECT COUNT(*) FROM main.gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2 AND stop_fact_id IS NOT NULL",
                &["projectA", "sessionA"]).unwrap(), 0);
            assert_eq!(count(db,
                "SELECT COUNT(*) FROM main.gogoke_v37_h_process_episode WHERE domain_id=?1 AND session_id=?2",
                &["projectA", "sessionA"]).unwrap(), 0);
            assert_eq!(count(db,
                "SELECT COUNT(*) FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2 AND operation='stop'",
                &["projectA", "sessionA"]).unwrap(), 0);
            let released = crate::store::seat::get(db, "projectA", "seatA").unwrap().unwrap();
            assert_eq!(released.state, crate::store::seat::State::Idle);
            assert_eq!(released.generation, busy.generation + 1);
            let replayed_admission = super::super::runtime::release_native(
                db, &crate::store::seat::NativeOrigin::user(owner), &request).unwrap();
            assert_eq!(replayed_admission, AdmissionResult::Replayed(4));
            assert_eq!(crate::store::seat::get(db, "projectA", "seatA").unwrap().unwrap().generation,
                released.generation);
        });
    }

    #[test]
    fn owner_release_refuses_started_unknown_stale_and_revoked_claims() {
        for scenario in ["open", "episode", "process", "unknown", "generation", "stale",
            "revoked", "wrong-generation", "wrong-home", "wrong-seat-binding"] {
            owner_unstarted_fixture(|_root, db, owner, _busy| {
                match scenario {
                    "open" => db.execute("INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA','openA','00','open','sessionA','UNKNOWN',3,3)").unwrap(),
                    "episode" => db.execute("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,phase) VALUES('projectA','openA','sessionA','2',NULL,'00',3,NULL,NULL,'instanceA','homeA','bindingA','INTENT')").unwrap(),
                    "process" => db.execute("UPDATE main.gogoke_v37_h_claim SET process_operation_id='processA' WHERE domain_id='projectA' AND session_id='sessionA'").unwrap(),
                    "unknown" => db.execute("UPDATE main.gogoke_v37_h_claim SET state='UNKNOWN' WHERE domain_id='projectA' AND session_id='sessionA'").unwrap(),
                    "generation" => db.execute("INSERT INTO main.gogoke_v37_h_generation_change(domain_id,request_id,raw_hex,operation,session_id,old_generation,old_process_operation_id,old_ticket,old_nonce,thread_id,seat_id,previous_revision,source_watermark,stage) VALUES('projectA','changeA','00','renew-session','sessionA','1','oldProcess','ticket','nonce','thread','seatA',3,0,'INTENT')").unwrap(),
                    "wrong-seat-binding" => db.execute("UPDATE main.gogoke_v37_h_seat_binding SET seat_incarnation='changed-incarnation' WHERE domain_id='projectA' AND session_id='sessionA'").unwrap(),
                    "stale" | "revoked" | "wrong-generation" | "wrong-home" => (),
                    _ => unreachable!(),
                }
                if scenario == "revoked" {
                    db.execute("UPDATE main.gogoke_authority_profile SET issuer_id='revoked-issuer' WHERE singleton=1").unwrap();
                }
                let mut request = owner_release_request(if scenario == "stale" { 2 } else { 3 });
                if scenario == "wrong-generation" { request.generation = "1"; }
                if scenario == "wrong-home" { request.home_id = "homeB"; }
                let outcome = super::super::runtime::release_native(
                    db, &crate::store::seat::NativeOrigin::user(owner), &request);
                match scenario {
                    "stale" => assert_eq!(outcome.unwrap(), AdmissionResult::Stale),
                    "revoked" | "wrong-seat-binding" => assert!(outcome.is_err()),
                    _ => assert_eq!(outcome.unwrap(), AdmissionResult::Conflict),
                }
                let current = claim(db, &owner_release_request(3)).unwrap().unwrap();
                assert_eq!(current.0, if scenario == "unknown" { "UNKNOWN" } else { "COMMITTED" });
                assert_eq!(current.1, 3);
                assert_eq!(count(db,
                    "SELECT COUNT(*) FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2 AND operation='admission-release'",
                    &["projectA", "sessionA"]).unwrap(), 0);
                assert_eq!(count(db,
                    "SELECT COUNT(*) FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2 AND operation='open'",
                    &["projectA", "sessionA"]).unwrap(), if scenario == "open" { 1 } else { 0 });
                assert_eq!(count(db,
                    "SELECT COUNT(*) FROM main.gogoke_v37_h_process_episode WHERE domain_id=?1 AND session_id=?2",
                    &["projectA", "sessionA"]).unwrap(), if scenario == "episode" { 1 } else { 0 });
                let process = Statement::prepare(db.as_ptr(),
                    "SELECT COALESCE(process_operation_id,'') FROM main.gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2").unwrap();
                process.bind_text(1, "projectA").unwrap();
                process.bind_text(2, "sessionA").unwrap();
                assert!(process.step_row().unwrap());
                assert_eq!(process.column_text(0).unwrap(),
                    if scenario == "process" { "processA" } else { "" });
                assert_eq!(count(db,
                    "SELECT COUNT(*) FROM main.gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2 AND stop_fact_id IS NOT NULL",
                    &["projectA", "sessionA"]).unwrap(), 0);
                let unchanged = crate::store::seat::get(db, "projectA", "seatA").unwrap().unwrap();
                assert_eq!(unchanged.state, crate::store::seat::State::Busy);
                assert_eq!(unchanged.generation, 2);
            });
        }
    }

    #[test]
    fn one_boundary_survives_reopen_unknown_replay_and_home_fence() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let folder =
            std::env::temp_dir().join(format!("gogoke-v37-h-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let root = RootLock::acquire(&folder).unwrap();
        let path = folder.join("state.sqlite");
        let mut db = create_new(&root, &path).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        db.execute("CREATE TABLE gogoke_v37_instances(instance_id TEXT PRIMARY KEY) STRICT")
            .unwrap();
        db.execute("CREATE TABLE gogoke_v37_instance_homes(home_id TEXT PRIMARY KEY,instance_id TEXT,domain_id TEXT,kind TEXT,owner_id TEXT,generation TEXT,state TEXT) STRICT").unwrap();
        db.execute("CREATE TABLE gogoke_coordination_process_custody(operation_id TEXT PRIMARY KEY,domain_id TEXT,generation TEXT,state TEXT,stop_proof_hash TEXT) STRICT").unwrap();
        db.execute("INSERT INTO gogoke_v37_instances VALUES('instanceA')")
            .unwrap();
        db.execute("INSERT INTO gogoke_v37_instance_homes VALUES('homeA','instanceA','projectA','SESSION','sessionA','1','ACTIVE'),('homeB','instanceA','projectA','SESSION','sessionB','1','ACTIVE')").unwrap();
        crate::store::seat::initialize_schema(&mut db).unwrap();
        initialize_admission_schema(&mut db).unwrap();
        super::super::session_binding::initialize_schema(&mut db).unwrap();
        db.execute("INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('projectA','seatA','incA','USER','LONG','instanceA','BUSY',1,1),('projectA','seatB','incB','USER','LONG','instanceA','BUSY',1,1)").unwrap();
        in_transaction(&mut db, |connection| {
            for (binding_id, session_id, seat_id) in [("bindingA", "sessionA", "seatA"), ("bindingB", "sessionB", "seatB")] {
                bind_owner_in_transaction(
                    connection,
                    &OwnerBinding {
                        binding_id,
                        instance_id: "instanceA",
                        domain_id: "projectA",
                        kind: "SESSION",
                        owner_id: session_id,
                        generation: "1",
                    },
                )?;
                let seat=crate::store::seat::get(connection,"projectA",seat_id)
                    .map_err(AdmissionError::Seat)?.ok_or(AdmissionError::Denied)?;
                bind_seat_in_transaction(connection,&seat,session_id)?;
            }
            Ok(())
        })
        .unwrap();
        let first = AdmissionRequest {
            domain_id: "projectA",
            session_id: "sessionA",
            request_id: "reserveA",
            raw_bytes: b"reserve bytes A",
            instance_id: "instanceA",
            home_id: "homeA",
            generation: "1",
            expected_revision: 0,
        };
        let second = AdmissionRequest {
            domain_id: "projectA",
            session_id: "sessionB",
            request_id: "reserveB",
            raw_bytes: b"reserve bytes B",
            instance_id: "instanceA",
            home_id: "homeB",
            generation: "1",
            expected_revision: 0,
        };
        assert_eq!(
            reserve_admission(&mut db, &first, one_capacity).unwrap(),
            AdmissionResult::Applied(1)
        );
        assert_eq!(
            reserve_admission(&mut db, &first, one_capacity).unwrap(),
            AdmissionResult::Replayed(1)
        );
        let changed = AdmissionRequest {
            raw_bytes: b"changed bytes",
            ..first
        };
        assert_eq!(
            reserve_admission(&mut db, &changed, one_capacity).unwrap(),
            AdmissionResult::Conflict
        );
        assert!(matches!(
            reserve_admission(&mut db, &second, one_capacity),
            Err(AdmissionError::CapacityExceeded {
                project_active: 1, project_cap: 1, instance_active: 1, instance_cap: 1,
            })
        ));
        assert_eq!(count(&db,
            "SELECT COUNT(*) FROM gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2",
            &["projectA", "sessionB"]).unwrap(), 0);
        assert_eq!(count(&db,
            "SELECT COUNT(*) FROM gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2",
            &["projectA", "sessionB"]).unwrap(), 0);
        let commit = AdmissionRequest {
            request_id: "commitA",
            raw_bytes: b"commit bytes",
            expected_revision: 1,
            ..first
        };
        assert_eq!(
            commit_admission(&mut db, &commit, authorized).unwrap(),
            AdmissionResult::Applied(2)
        );
        db.execute("INSERT INTO gogoke_coordination_process_custody VALUES('processA','projectA','1','PREPARED',NULL)").unwrap();
        in_transaction(&mut db, |connection| {
            bind_process_operation_in_transaction(connection, "projectA", "sessionA", "processA")
        })
        .unwrap();
        db.execute("UPDATE gogoke_coordination_process_custody SET state='UNKNOWN' WHERE operation_id='processA'").unwrap();
        let opened = AdmissionRequest {
            request_id: "openA",
            raw_bytes: b"open bytes",
            expected_revision: 2,
            ..first
        };
        in_transaction(&mut db, |connection| {
            mark_start_unknown_in_transaction(connection, &opened, "processA")
        })
        .unwrap();
        db.execute("INSERT INTO gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,phase) VALUES('projectA','openA','sessionA','1',NULL,'6f70656e206279746573',2,3,'processA','instanceA','homeA','bindingA','UNKNOWN')").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_generation VALUES('projectA','sessionA','1','openA','processA')").unwrap();
        db.close_checked().unwrap();
        let mut db = open_existing(&root, &path).unwrap();
        initialize_admission_schema(&mut db).unwrap();
        super::super::session_binding::initialize_schema(&mut db).unwrap();
        assert_eq!(
            prior(&db, &opened, "open").unwrap(),
            Some(AdmissionResult::Unknown)
        );
        assert!(matches!(
            reserve_admission(&mut db, &second, one_capacity),
            Err(AdmissionError::CapacityExceeded {
                project_active: 1, project_cap: 1, instance_active: 1, instance_cap: 1,
            })
        ));
        db.execute("UPDATE gogoke_coordination_process_custody SET state='STOPPED',stop_proof_hash='proofA' WHERE operation_id='processA'").unwrap();
        in_transaction(&mut db, |connection| {
            assert_eq!(
                record_session_stop_in_transaction(connection, "projectA", "sessionA", "processA")?,
                "proofA"
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(
            prior(&db, &opened, "open").unwrap(),
            Some(AdmissionResult::Replayed(4))
        );
        let release = AdmissionRequest {
            request_id: "releaseA",
            raw_bytes: b"release bytes",
            expected_revision: 4,
            ..first
        };
        assert_eq!(
            release_admission(&mut db, &release, authorized).unwrap(),
            AdmissionResult::Applied(5)
        );
        db.execute("UPDATE gogoke_v37_instance_homes SET state='CLOSED' WHERE home_id='homeA'")
            .unwrap();
        in_transaction(&mut db, |connection| {
            assert_eq!(
                fence_home_admission_in_transaction(
                    connection,
                    "homeA",
                    "instanceA",
                    "projectA",
                    "SESSION",
                    "sessionA",
                    "1"
                )?,
                "h-fence-homeA"
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(
            reserve_admission(&mut db, &second, one_capacity).unwrap(),
            AdmissionResult::Applied(1)
        );
        in_transaction(&mut db, |connection| {
            revoke_owner_binding_in_transaction(connection, "bindingB")
        })
        .unwrap();
        assert!(matches!(
            verify_home_owner_in_transaction(
                &mut db,
                "instanceA",
                "projectA",
                "SESSION",
                "sessionB",
                "1"
            ),
            Err(AdmissionError::Denied)
        ));
        db.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&folder).unwrap();
    }
}
