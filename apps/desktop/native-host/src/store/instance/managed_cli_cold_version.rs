//! A version action may re-evaluate historical executions only when their
//! original producers and every version blocker identify the complete family.
//! The journal stores a snapshot; it is not a native absence observation.

use super::{inspect_staged_official_cli_version, no_unsettled_managed_version_change,
    read_official_cli_version, read_prepared_managed_replacement, ManagedCliError, OFFICIAL};
use super::super::{managed_version_action::{self,ActionRecord}, program_source, registry};
use crate::process::{NativeLegacyHoldersGone,NativeStopProof};
use crate::root::{inspect_root, inspect_regular_file, RootLock};
use crate::store::atomic::Statement;
use crate::store::digest::{content_hash, sha256_hex};
use crate::store::same_open::VerifiedDatabaseConnection;
use crate::store::atomic::{Json,JsonString};
use std::os::windows::fs::MetadataExt;

/// Kept private to F. The sealed H observation cannot be reconstructed from
/// persisted action bytes or a caller-supplied `gone` value.
enum NativeVersionAbsence { Reboot(NativeLegacyHoldersGone) }

struct QualifiedUnit {
    source: Unit,
    absence: NativeVersionAbsence,
}

pub(crate) struct QualifiedVersionAction {
    source_hex: String,
    source_digest: String,
    pairs_hex: String,
    pairs_digest: String,
    units: Vec<QualifiedUnit>,
    stdin: Vec<StdinMember>,
    global_operations: Vec<String>,
    global_native: Vec<(Vec<(u32,u64)>,NativeLegacyHoldersGone)>,
}

impl QualifiedVersionAction {
    pub(crate) fn is_cold(&self) -> bool { !self.source_hex.is_empty() }
    pub(crate) fn proof_family(&self) -> &'static str {
        if self.is_cold() { "family-v1" } else { "" }
    }
    pub(crate) fn source_hex(&self) -> &str { &self.source_hex }
    pub(crate) fn source_digest(&self) -> &str { &self.source_digest }
    pub(crate) fn pairs_hex(&self) -> &str { &self.pairs_hex }
    pub(crate) fn pairs_digest(&self) -> &str { &self.pairs_digest }
    pub(crate) fn allows_claim(&self, instance: &str, domain: &str, session: &str,
        generation: &str, binding: &str, process: &str) -> bool {
        self.units.iter().any(|member| { let u=&member.source; u.instance == instance && u.domain == domain &&
            u.session == session && u.generation == generation && u.binding == binding &&
            u.process == process })
    }
    pub(crate) fn allows_episode(&self, domain: &str, request: &str, process: &str) -> bool {
        self.units.iter().any(|member| { let u=&member.source; u.domain == domain &&
            u.open_request == request && u.process == process })
    }
    pub(crate) fn allows_operation(&self, domain:&str, session:&str, request:&str)->bool {
        self.units.iter().any(|member| { let u=&member.source; u.domain==domain &&
            u.session==session && u.open_request==request })
    }
    pub(crate) fn allows_home(&self, home: &str) -> bool {
        self.units.iter().any(|unit| unit.source.home == home)
    }
    pub(crate) fn allows_custody(&self, process: &str) -> bool {
        self.units.iter().any(|unit| unit.source.process == process) ||
            self.global_operations.iter().any(|operation|operation==process)
    }
    pub(crate) fn allows_alias(&self, _domain: &str, _session: &str,
        _generation: &str) -> bool { false }
    pub(crate) fn allows_stdin(&self, instance:&str, domain:&str, session:&str,
        generation:&str, process:&str, request:&str, ticket:&str, nonce:&str)->bool {
        self.stdin.iter().any(|row| row.pending && row.instance==instance && row.domain==domain &&
            row.session==session && row.generation==generation && row.process==process &&
            row.request==request && row.ticket==ticket && row.nonce==nonce)
    }
    pub(crate) fn recheck_native(&self) -> Result<(), ManagedCliError> {
        for member in &self.units {
            match &member.absence {
                NativeVersionAbsence::Reboot(fact) => fact.validate(&member.source.pairs)
                    .map_err(|error|ManagedCliError::NativeGone(format!(
                        "cold version reboot recheck: {error:?}")))?,
            }
        }
        for (pairs,fact) in &self.global_native {
            fact.validate(pairs).map_err(|error|ManagedCliError::NativeGone(format!(
                "global original kernel recheck: {error:?}")))?;
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Unit {
    instance: String, domain: String, session: String, generation: String,
    binding: String, home: String, process: String, open_request: String,
    pairs: Vec<(u32,u64)>,
}

struct StdinMember {
    instance:String, domain:String, session:String, generation:String,
    process:String, request:String, ticket:String, nonce:String,pending:bool,
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len()*2);
    for byte in bytes { out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 15) as usize] as char); }
    out
}
fn frame(parts: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in parts { out.extend_from_slice(&(part.len() as u64).to_be_bytes());
        out.extend_from_slice(part.as_bytes()); }
    out
}
fn rows(db: &VerifiedDatabaseConnection<'_>, sql: &str, driver: &str,
    width: i32) -> Result<Vec<Vec<String>>, ManagedCliError> {
    let q = Statement::prepare(db.as_ptr(), sql)?;
    q.bind_text(1, driver)?;
    let mut all = Vec::new();
    while q.step_row()? {
        let mut row = Vec::with_capacity(width as usize);
        for index in 0..width { row.push(q.column_text(index)?); }
        all.push(row);
    }
    Ok(all)
}
fn no_rows(db: &VerifiedDatabaseConnection<'_>, sql: &str,
    driver: &str) -> Result<(), ManagedCliError> {
    if rows(db, sql, driver, 1)?.is_empty() { Ok(()) } else { Err(ManagedCliError::Busy) }
}
fn one_matching(db: &VerifiedDatabaseConnection<'_>, sql: &str, driver: &str,
    width: i32, expected: impl Fn(&[String]) -> bool) -> Result<(), ManagedCliError> {
    let all = rows(db,sql,driver,width)?;
    if all.iter().all(|row| expected(row)) { Ok(()) } else { Err(ManagedCliError::Busy) }
}
fn collect_blocker(blocked:&mut bool,result:Result<(),ManagedCliError>)
    ->Result<(),ManagedCliError>{
    match result {Ok(())=>Ok(()),Err(ManagedCliError::Busy)=>{*blocked=true;Ok(())},
        Err(error)=>Err(error)}
}

fn stage_name_from_original_image(staging:&std::path::Path,
    original_image:&std::path::Path,
    pin:super::VerifiedOfficialCli)->Result<String,ManagedCliError>{
    let staging=std::fs::canonicalize(staging)?;
    let original=std::fs::canonicalize(original_image)?;
    let relative=original.strip_prefix(&staging).map_err(|_|ManagedCliError::Busy)?;
    let name=relative.components().next().and_then(|part|part.as_os_str().to_str())
        .filter(|name|!name.is_empty()).ok_or(ManagedCliError::Busy)?;
    let expected=pin.image_relative.split('/').fold(
        std::path::PathBuf::from(name).join("content"),|path,part|path.join(part));
    if relative!=expected {return Err(ManagedCliError::Busy)}
    Ok(name.to_owned())
}

fn original_catalog_pin(driver:&str,digest:&str)
    ->Result<super::VerifiedOfficialCli,ManagedCliError>{
    let image=digest.strip_prefix("sha256:").ok_or(ManagedCliError::Busy)?;
    let mut matches=OFFICIAL.iter().copied().filter(|pin|
        pin.driver==driver && pin.image_sha256==image);
    let pin=matches.next().ok_or(ManagedCliError::Busy)?;
    if matches.next().is_some(){return Err(ManagedCliError::Busy)}
    Ok(pin)
}

/// The old image comes from the original H/C producer, never the current F
/// program columns. The current columns are validated separately through the
/// creation/switch witness chain, source row and copy state.
fn verified_current_managed_source(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str,
    instance:&str,home:&str,current_digest:&str,current_version:&str,
)->Result<(),ManagedCliError>{
    let creation=registry::verified_creation_request(db,root,instance)
        .map_err(|error|ManagedCliError::Observation(format!("current F creation/switch chain: {error:?}")))?;
    if read_prepared_managed_replacement(db,root,driver)?.is_none(){
        return Err(ManagedCliError::Busy)
    }
    let current_pin=read_official_cli_version(driver,current_version).ok_or(ManagedCliError::Busy)?;
    if current_digest!=format!("sha256:{}",current_pin.image_sha256){
        return Err(ManagedCliError::Busy)
    }
    let source=Statement::prepare(db.as_ptr(),
        "SELECT source,stage_name,program_digest,version,home_identity,registration_request_id FROM main.gogoke_v37_instance_program_sources WHERE instance_id=?1")?;
    source.bind_text(1,instance)?;
    if !source.step_row()?{return Err(ManagedCliError::Busy)}
    let values=(0..6).map(|n|source.column_text(n))
        .collect::<std::result::Result<Vec<_>,_>>()?;
    if source.step_row()? || values[0]!="MANAGED" || values[2]!=current_digest ||
        values[3]!=current_version || values[4]!=home || values[5]!=creation {
        return Err(ManagedCliError::Busy)
    }
    inspect_staged_official_cli_version(root,driver,current_version,&values[1])?;
    Ok(())
}

fn verified_source_images(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str,
    instance:&str,home:&str,current_digest:&str,current_version:&str,
    original_image:&str,original_digest:&str)->Result<(),ManagedCliError>{
    verified_current_managed_source(db,root,driver,instance,home,
        current_digest,current_version)?;

    let old=original_catalog_pin(driver,original_digest)?;
    let original=std::path::Path::new(original_image);
    let metadata=std::fs::symlink_metadata(original)?;
    if !metadata.is_file() || metadata.file_attributes()&0x400!=0 {
        return Err(ManagedCliError::IdentityChanged)
    }
    let original_identity=inspect_regular_file(original)
        .map_err(|error|ManagedCliError::Observation(format!("original C image: {error:?}")))?;
    let retained=Statement::prepare(db.as_ptr(),
        "SELECT COALESCE(version,''),COALESCE(image_sha256,''),COALESCE(stage_name,''),COALESCE(previous_version,''),COALESCE(previous_image_sha256,''),COALESCE(previous_stage_name,'') FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
    retained.bind_text(1,driver)?;
    if !retained.step_row()? {return Err(ManagedCliError::Busy)}
    let columns=(0..6).map(|n|retained.column_text(n))
        .collect::<std::result::Result<Vec<_>,_>>()?;
    if retained.step_row()? {return Err(ManagedCliError::Busy)}
    for (version,image,stage) in [(&columns[0],&columns[1],&columns[2]),
        (&columns[3],&columns[4],&columns[5])] {
        if version.as_str()==old.version && image.as_str()==old.image_sha256 &&
            !stage.is_empty() {
            let verified=inspect_staged_official_cli_version(root,driver,old.version,stage)?;
            let identity=inspect_regular_file(&verified).map_err(|error|
                ManagedCliError::Observation(format!("retained old stage: {error:?}")))?;
            if identity==original_identity {return Ok(())}
        }
    }
    // Once the old image has left both copy columns, its original C path is
    // the only retained stage selector. It must still resolve to the exact
    // compiled catalog path under F's managed staging root.
    let staging=root.canonical_root().canonical_path.join("v37-managed-cli").join("staging");
    let stage=stage_name_from_original_image(&staging,original,old)?;
    let verified=inspect_staged_official_cli_version(root,driver,old.version,&stage)?;
    let original_path=std::fs::canonicalize(original)?;
    let verified_path=std::fs::canonicalize(&verified)?;
    if original_path!=verified_path {return Err(ManagedCliError::IdentityChanged)}
    let verified_identity=inspect_regular_file(&verified)
        .map_err(|error|ManagedCliError::Observation(format!("original staged image: {error:?}")))?;
    if original_identity!=verified_identity {
        return Err(ManagedCliError::IdentityChanged)
    }
    Ok(())
}

/// Exact original source is the persisted H OPEN/C process chain. The current
/// F program/copy revisions are intentionally absent: a legitimate stage or
/// final switch changes those and the action journal tracks their CAS chain.
fn original_units(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    driver: &str) -> Result<Vec<(Unit,Vec<String>)>, ManagedCliError> {
    let root_seen = inspect_root(&root.canonical_root().canonical_path)
        .map_err(|error| ManagedCliError::Observation(format!("cold root: {error:?}")))?;
    if root_seen.identity != root.canonical_root().identity ||
        db.root_identity() != &root_seen.identity { return Err(ManagedCliError::IdentityChanged); }
    match no_rows(db,"SELECT 1 FROM main.gogoke_v37_h_claim c JOIN main.gogoke_v37_instances i ON i.instance_id=c.instance_id WHERE i.driver_id=?1 AND c.process_operation_id IS NULL AND c.state IN ('COMMITTED','UNKNOWN') LIMIT 1",driver) {
        Err(ManagedCliError::Busy)=>return Err(ManagedCliError::Observation(
            "processless historical claim lacks original D-NOLAUNCH producer binding".into())),
        Err(error)=>return Err(error),
        Ok(())=>(),
    }
    let sql = "SELECT i.instance_id,i.home_identity,c.domain_id,e.session_id,e.generation,
        e.request_id,e.process_operation_id,e.binding_id,e.home_id,e.phase,e.raw_hex,
        CAST(e.previous_revision AS TEXT),COALESCE(CAST(e.result_revision AS TEXT),''),
        c.ticket,c.custodian_nonce,c.pid,c.creation_time_100ns,c.image_path,
        c.binary_digest_sha256,c.state,COALESCE(c.stop_proof_hash,''),
        a.state,CAST(a.revision AS TEXT),b.state,b.kind,b.owner_id,
        h.state,h.directory_ref,h.directory_identity,CAST(h.revision AS TEXT),
        o.status,o.raw_hex,CAST(o.previous_revision AS TEXT),CAST(o.revision AS TEXT),
        i.program_digest,i.version,
        e.result_revision IS NULL,c.stop_proof_hash IS NULL,
        COALESCE(e.seat_id,''),COALESCE(e.seat_incarnation,'')
        FROM main.gogoke_v37_h_process_episode e
        JOIN main.gogoke_coordination_process_custody c
          ON c.operation_id=e.process_operation_id AND c.profile_id=e.instance_id
          AND c.domain_id=e.domain_id AND c.generation=e.generation
        JOIN main.gogoke_v37_h_claim a
          ON a.instance_id=e.instance_id AND a.domain_id=e.domain_id
          AND a.session_id=e.session_id AND a.generation=e.generation
          AND a.process_operation_id=e.process_operation_id AND a.binding_id=e.binding_id
          AND a.home_id=e.home_id
        JOIN main.gogoke_v37_h_owner_binding b
          ON b.binding_id=e.binding_id AND b.instance_id=e.instance_id
          AND b.domain_id=e.domain_id AND b.generation=e.generation
        JOIN main.gogoke_v37_instance_homes h
          ON h.home_id=e.home_id AND h.instance_id=e.instance_id
          AND h.domain_id=e.domain_id AND h.generation=e.generation
        JOIN main.gogoke_v37_h_operation o
          ON o.domain_id=e.domain_id AND o.request_id=e.request_id
          AND o.session_id=e.session_id AND o.raw_hex=e.raw_hex AND o.operation='open'
        JOIN main.gogoke_v37_h_generation g
          ON g.domain_id=e.domain_id AND g.session_id=e.session_id
          AND g.generation=e.generation AND g.request_id=e.request_id
          AND g.process_operation_id=e.process_operation_id
        JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
        WHERE i.driver_id=?1 AND e.old_generation IS NULL
          AND e.phase IN ('PREPARED','ACTIVE','UNKNOWN')
          AND c.state IN ('PREPARED','ACTIVE','UNKNOWN')
          AND a.state IN ('COMMITTED','UNKNOWN')
          AND b.state='ACTIVE' AND b.kind='SESSION' AND b.owner_id=e.session_id
          AND h.kind='SESSION' AND h.owner_id=e.session_id
          AND h.state IN ('ACTIVE','CLOSE_UNKNOWN','UNKNOWN')
          AND o.status IN ('APPLIED','UNKNOWN')";
    let candidates = rows(db,sql,driver,40)?;
    // The current namespace observer proves a mapping in this namespace. It
    // cannot prove an old OpenCode logon namespace disappeared, so that family
    // remains unqualified even when its host and child PIDs are gone.
    match no_rows(db,"SELECT 1 FROM main.gogoke_v37_h_dos_alias a JOIN main.gogoke_v37_instances i ON i.instance_id=a.instance_id WHERE i.driver_id=?1 LIMIT 1",driver) {
        Err(ManagedCliError::Busy)=>return Err(ManagedCliError::Observation(
            "original OpenCode alias lacks old namespace disappearance proof".into())),
        Err(error)=>return Err(error),
        Ok(())=>(),
    }
    let mut units=Vec::with_capacity(candidates.len());
    for raw in &candidates {
    if raw[18].is_empty() || raw[17].is_empty() || raw[13].is_empty() || raw[14].is_empty() ||
        !raw[20].is_empty() || raw[31] != raw[10] || raw[36]!="0" || raw[37]!="1" {
        return Err(ManagedCliError::Busy);
    }
    let open_bytes = raw[10].as_bytes().chunks_exact(2).map(|pair| {
        let value = std::str::from_utf8(pair).ok()?;
        u8::from_str_radix(value,16).ok()
    }).collect::<Option<Vec<_>>>().ok_or(ManagedCliError::Busy)?;
    if raw[10].len()%2!=0 || open_bytes.is_empty() ||
        !raw[10].bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        return Err(ManagedCliError::Busy);
    }
    let open_hash = sha256_hex(&open_bytes);
    if raw[6] != format!("h-open-{}", &open_hash[..40]) {
        return Err(ManagedCliError::Busy);
    }
    let request=crate::store::session_transport::decode_request(&open_bytes)
        .map_err(|error|ManagedCliError::Observation(format!(
            "original H OPEN raw: {error:?}")))?;
    if !original_open_equations(&request,raw) ||
        request.family!="K-SESSION" || request.operation!="open" ||
        request.request_id!=raw[5] || request.domain_id!=raw[2] ||
        request.target_id!=raw[3] || raw[38].is_empty() || raw[39].is_empty() {
        return Err(ManagedCliError::Busy)
    }
    let worktree_source=verified_original_worktree(db,&request,&raw[0],&raw[2],&raw[4],
        &raw[38],&raw[39])?;
    verified_source_images(db,root,driver,&raw[0],&raw[1],&raw[34],&raw[35],
        &raw[17],&raw[18])?;
    let pid = raw[15].parse::<u32>().map_err(|_|ManagedCliError::Busy)?;
    let creation = raw[16].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
    if pid==0 || creation==0 { return Err(ManagedCliError::Busy); }
    let home_identity = registry::observed_home(root,&raw[0])
        .map_err(|error|ManagedCliError::Observation(format!("original instance home: {error:?}")))?
        .ok_or(ManagedCliError::Busy)?;
    if home_identity.opaque()!=raw[1] || raw[27] != format!("temp-home-{}",sha256_hex(raw[8].as_bytes())) {
        return Err(ManagedCliError::IdentityChanged);
    }
    let temp = root.canonical_root().canonical_path.join("v37-temporary-homes")
        .join(&raw[0]).join(&raw[27]);
    let temp_meta=std::fs::symlink_metadata(&temp)?;
    if !temp_meta.is_dir() || temp_meta.file_attributes()&0x400!=0 {
        return Err(ManagedCliError::IdentityChanged);
    }
    let temp_identity = inspect_root(&temp)
        .map_err(|error|ManagedCliError::Observation(format!("original temporary home: {error:?}")))?;
    if temp_identity.identity.opaque()!=raw[28] { return Err(ManagedCliError::IdentityChanged); }
    let pairs=vec![(pid,creation)];
    let unit = Unit {instance:raw[0].clone(), domain:raw[2].clone(), session:raw[3].clone(),
        generation:raw[4].clone(), open_request:raw[5].clone(), process:raw[6].clone(),
        binding:raw[7].clone(), home:raw[8].clone(), pairs};
    // The only omitted column is the live F program digest at index 34.
    let mut source=raw[..34].to_vec();
    source.push("no-alias".into());
    source.extend(raw[36..40].iter().cloned());
    source.push(worktree_source);
    source.insert(0,"H_SESSION_OPEN-v1".into());
    units.push((unit,source));
    }
    units.sort_by(|left,right| {
        let a=&left.0; let b=&right.0;
        (&a.instance,&a.domain,&a.session,&a.generation,&a.process)
            .cmp(&(&b.instance,&b.domain,&b.session,&b.generation,&b.process))
    });
    if units.windows(2).any(|pair| {
        let a=&pair[0].0; let b=&pair[1].0;
        a.instance==b.instance && a.domain==b.domain && a.session==b.session &&
            a.generation==b.generation && a.process==b.process
    }) {return Err(ManagedCliError::Busy)}
    Ok(units)
}

/// A stopped predecessor remains a member of the historical family even
/// after the current claim advances to its child. Only a genuine original
/// USER STOP is handled here; compound/internal STOP without its distinct
/// producer chain is retained as a refusal.
fn original_stopped_sources(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str)
    ->Result<Vec<(Vec<String>,Vec<String>)>,ManagedCliError>{
    let sql="SELECT e.instance_id,e.domain_id,e.session_id,e.generation,e.request_id,
        e.process_operation_id,e.home_id,e.binding_id,e.raw_hex,
        CAST(e.previous_revision AS TEXT),COALESCE(CAST(e.result_revision AS TEXT),''),
        COALESCE(e.old_generation,''),COALESCE(e.stop_request_id,''),
        COALESCE(e.stop_fact_id,''),e.phase,c.profile_id,c.domain_id,c.generation,
        c.state,COALESCE(c.stop_proof_hash,''),c.ticket,c.custodian_nonce,
        c.pid,c.creation_time_100ns,c.image_path,c.binary_digest_sha256,
        i.home_identity,i.program_digest,i.version,
        COALESCE(e.seat_id,''),COALESCE(e.seat_incarnation,''),
        e.old_generation IS NULL,e.stop_request_id IS NULL,e.stop_fact_id IS NULL,
        c.stop_proof_hash IS NULL
        FROM main.gogoke_v37_h_process_episode e
        JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
        LEFT JOIN main.gogoke_coordination_process_custody c
          ON c.operation_id=e.process_operation_id
        WHERE i.driver_id=?1 AND e.phase='STOPPED'";
    let rows=rows(db,sql,driver,35)?;
    let mut result=Vec::with_capacity(rows.len());
    for row in rows {
        if row[11]!="" && row[11].parse::<u64>().ok().and_then(|n|n.checked_add(1))
            !=row[3].parse::<u64>().ok() {
            return Err(ManagedCliError::Observation(
                "unsupported original stopped compound or malformed resume predecessor source".into()));
        }
        if row[0].is_empty() || row[1].is_empty() || row[2].is_empty() ||
            row[3].is_empty() || row[4].is_empty() || row[5].is_empty() ||
            row[6].is_empty() || row[7].is_empty() ||
            row[12].is_empty() || row[13].is_empty() || row[14]!="STOPPED" ||
            row[15]!=row[0] || row[16]!=row[1] || row[17]!=row[3] ||
            row[18]!="STOPPED" || row[19]!=row[13] ||
            row[20].is_empty() || row[21].is_empty() || row[24].is_empty() ||
            row[25].is_empty() || row[29].is_empty() || row[30].is_empty() ||
            row[31]!=if row[11].is_empty(){"1"}else{"0"} ||
            row[32]!="0" || row[33]!="0" || row[34]!="0" {
            return Err(ManagedCliError::Busy);
        }
        let pid=row[22].parse::<u32>().map_err(|_|ManagedCliError::Busy)?;
        let created=row[23].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
        if pid==0 || created==0 || row[22]!=pid.to_string() ||
            row[23]!=created.to_string() {return Err(ManagedCliError::Busy)}
        let raw=decode_original_hex(&row[8])?;
        let request=crate::store::session_transport::decode_request(&raw)
            .map_err(|error|ManagedCliError::Observation(format!(
                "original stopped H request: {error:?}")))?;
        if matches!(request.operation.as_str(),"compact"|"renew-session") {
            return Err(ManagedCliError::Observation(
                "unsupported stopped compound original generation-change source".into()))
        }
        let before=row[9].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
        let after=row[10].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
        let resume=!row[11].is_empty();
        if request.family!="K-SESSION" ||
            request.operation!=if resume{"resume"}else{"open"} ||
            request.request_id!=row[4] || request.domain_id!=row[1] ||
            request.target_id!=row[2] ||
            !(request.expected_revision==before || resume &&
                request.expected_revision.checked_add(1)==Some(before)) ||
            before.checked_add(1)!=Some(after) ||
            row[5]!=format!("h-{}-{}",if resume{"resume"}else{"open"},
                &sha256_hex(&raw)[..40]) {
            return Err(ManagedCliError::Busy);
        }
        let field=|name:&str| match request.payload.get(&JsonString::from_str(name)) {
            Some(Json::String(value))=>value.to_well_formed_string(),_=>None,
        };
        if resume {
            if request.payload.len()!=1 ||
                field("generation").as_deref()!=Some(row[11].as_str()) {
                return Err(ManagedCliError::Busy)
            }
            let unexpected=Statement::prepare(db.as_ptr(),
                "SELECT 1 FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2 UNION ALL SELECT 1 FROM main.gogoke_v37_h_generation_change WHERE domain_id=?1 AND request_id=?2")?;
            unexpected.bind_text(1,&row[1])?;unexpected.bind_text(2,&row[4])?;
            if unexpected.step_row()? {return Err(ManagedCliError::Busy)}
        } else if row[1]=="global" {
            if !request.payload.is_empty() {return Err(ManagedCliError::Busy)}
            let selected=Statement::prepare(db.as_ptr(),
                "SELECT 1 FROM main.gogoke_v37_native_selection s JOIN main.gogoke_v37_seat_secretary d ON d.domain_id=s.domain_id AND d.seat_id=s.seat_id AND d.incarnation=s.seat_incarnation WHERE s.domain_id='global' AND s.session_id=?1 AND s.seat_id=?2 AND s.seat_incarnation=?3 AND s.selected_instance_id=?4 UNION ALL SELECT 1 FROM main.gogoke_v37_h_seat_binding s JOIN main.gogoke_v37_seat_secretary d ON d.domain_id=s.domain_id AND d.seat_id=s.seat_id AND d.incarnation=s.seat_incarnation JOIN main.gogoke_v37_h_claim c ON c.domain_id=s.domain_id AND c.session_id=s.session_id WHERE s.domain_id='global' AND s.session_id=?1 AND s.seat_id=?2 AND s.seat_incarnation=?3 AND c.instance_id=?4 AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_native_selection n WHERE n.domain_id=s.domain_id AND n.session_id=s.session_id)")?;
            for (n,value) in [&row[2],&row[29],&row[30],&row[0]].iter().enumerate(){
                selected.bind_text(n as i32+1,value)?;
            }
            if !selected.step_row()? || selected.step_row()? {return Err(ManagedCliError::Busy)}
        } else {
            let formal=field("purpose").as_deref()==Some("FORMAL_REVIEW");
            if request.payload.len()!=if formal{5}else{4} ||
                field("generation").as_deref()!=Some(row[3].as_str()) ||
                field("seatId").as_deref()!=Some(row[29].as_str()) ||
                field("repositoryId").is_none_or(|value|value.is_empty()) ||
                field("worktreeId").is_none_or(|value|value.is_empty()) {
                return Err(ManagedCliError::Busy);
            }
        }
        let opened=Statement::prepare(db.as_ptr(),
            "SELECT raw_hex,status,CAST(previous_revision AS TEXT),CAST(revision AS TEXT)
             FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2
             AND session_id=?3 AND operation='open'")?;
        opened.bind_text(1,&row[1])?;opened.bind_text(2,&row[4])?;
        opened.bind_text(3,&row[2])?;
        if resume {
            if opened.step_row()? {return Err(ManagedCliError::Busy)}
        } else {
            if !opened.step_row()? || opened.column_text(0)?!=row[8] ||
                opened.column_text(1)?!="APPLIED" || opened.column_text(2)?!=row[9] ||
                opened.column_text(3)?!=row[10] || opened.step_row()? {
                return Err(ManagedCliError::Busy);
            }
        }
        let generation=Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_h_generation WHERE domain_id=?1 AND
             session_id=?2 AND generation=?3 AND request_id=?4 AND process_operation_id=?5")?;
        for (n,value) in [&row[1],&row[2],&row[3],&row[4],&row[5]].iter().enumerate(){
            generation.bind_text(n as i32+1,value)?;
        }
        if !generation.step_row()? || generation.step_row()? {return Err(ManagedCliError::Busy)}
        let owner=Statement::prepare(db.as_ptr(),
            "SELECT state FROM main.gogoke_v37_h_owner_binding WHERE binding_id=?1
             AND instance_id=?2 AND domain_id=?3 AND kind='SESSION' AND owner_id=?4
             AND generation=?5")?;
        for (n,value) in [&row[7],&row[0],&row[1],&row[2],&row[3]].iter().enumerate(){
            owner.bind_text(n as i32+1,value)?;
        }
        if !owner.step_row()? || !matches!(owner.column_text(0)?.as_str(),
            "ACTIVE"|"REVOKED") || owner.step_row()? {return Err(ManagedCliError::Busy)}
        let home_source=super::super::temporary::validate_original_execution_home_source(
            db,&row[6],&row[0],&row[1],&row[2],&row[3],&row[4],
            if resume{"resume"}else{"open"})
            .map_err(|error|ManagedCliError::Observation(format!(
                "original stopped F HOME: {error:?}")))?;
        let stop_source=super::super::temporary::validate_original_execution_stop_source(
            db,&row[6],&row[0],&row[1],&row[2],&row[3],&row[13])
            .map_err(|error|ManagedCliError::Observation(format!(
                "original USER STOP: {error:?}")))?;
        let current=Statement::prepare(db.as_ptr(),
            "SELECT generation,home_id,binding_id,COALESCE(process_operation_id,''),
             COALESCE(stop_fact_id,'') FROM main.gogoke_v37_h_claim WHERE domain_id=?1
             AND session_id=?2 AND instance_id=?3")?;
        current.bind_text(1,&row[1])?;current.bind_text(2,&row[2])?;
        current.bind_text(3,&row[0])?;
        if current.step_row()? {
            let values=(0..5).map(|n|current.column_text(n))
                .collect::<std::result::Result<Vec<_>,_>>()?;
            if current.step_row()? || (values[0]==row[3] &&
                (values[1]!=row[6] || values[2]!=row[7] ||
                 values[3]!=row[5] || values[4]!=row[13])) {
                return Err(ManagedCliError::Busy)
            }
        }
        verified_source_images(db,root,driver,&row[0],&row[26],&row[27],&row[28],
            &row[24],&row[25])?;
        let worktree_source=verified_original_worktree(db,&request,&row[0],&row[1],&row[3],
            &row[29],&row[30])?;
        let home=Statement::prepare(db.as_ptr(),
            "SELECT state,CAST(revision AS TEXT),directory_ref,directory_identity FROM main.gogoke_v37_instance_homes WHERE home_id=?1 AND instance_id=?2 AND domain_id=?3 AND generation=?4 AND kind='SESSION' AND owner_id=?5")?;
        for (n,value) in [&row[6],&row[0],&row[1],&row[3],&row[2]].iter().enumerate(){
            home.bind_text(n as i32+1,value)?;
        }
        if !home.step_row()? {return Err(ManagedCliError::Busy)}
        let home_values=(0..4).map(|n|home.column_text(n))
            .collect::<std::result::Result<Vec<_>,_>>()?;
        if home.step_row()? || home_values[2]!=format!("temp-home-{}",sha256_hex(row[6].as_bytes())) ||
            home_values[3].is_empty() || home_values[1].parse::<u64>().is_err() ||
            !matches!(home_values[0].as_str(),"ACTIVE"|"CLOSED"|"CLEANED") {
            return Err(ManagedCliError::Busy)
        }
        let suffix=row[6].strip_prefix("home-").ok_or(ManagedCliError::Busy)?;
        if suffix.len()!=40 || !suffix.bytes().all(|b|b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
            return Err(ManagedCliError::Busy)
        }
        let profile=crate::process::AppContainerProfile::derive_for_revocation(
            &format!("Gogoke37.Session.{suffix}"))
            .map_err(|error|ManagedCliError::Observation(format!(
                "original stopped HOME profile: {error:?}")))?;
        let (_,physical)=super::super::temporary::resolve_historical_session_home(
            db,root,&profile,&row[0],&row[6],&row[1],&row[2],&row[3],&row[7])
            .map_err(|error|ManagedCliError::Observation(format!(
                "original stopped physical HOME: {error:?}")))?;
        if physical.opaque()!=home_values[3] {return Err(ManagedCliError::IdentityChanged)}
        let mut source=vec![if resume{"H_SESSION_RESUME_STOP-v1"}else{"H_SESSION_STOP-v1"}.to_owned()];
        source.extend(row[..27].iter().cloned());
        source.extend(row[29..].iter().cloned());
        source.push(home_source);source.push(stop_source);source.push(worktree_source);
        source.extend(home_values);
        source.push(sha256_hex(&frame(&[db.root_identity().opaque(),
            db.identity().opaque(),row[0].clone(),row[1].clone(),row[2].clone(),
            row[3].clone(),row[6].clone(),row[7].clone(),physical.opaque()])));
        let proof=vec!["ORIGINAL_STOP".to_owned(),source[1].clone(),source[2].clone(),
            source[3].clone(),source[4].clone(),source[6].clone(),
            pid.to_string(),created.to_string()];
        result.push((source,proof));
    }
    result.sort_by(|a,b|(&a.0[1],&a.0[2],&a.0[3],&a.0[4],&a.0[6])
        .cmp(&(&b.0[1],&b.0[2],&b.0[3],&b.0[4],&b.0[6])));
    if result.windows(2).any(|p| p[0].1==p[1].1) {return Err(ManagedCliError::Busy)}
    for (source,_) in &result {
        if source[0]=="H_SESSION_RESUME_STOP-v1" &&
            !result.iter().any(|(parent,_)|parent[1]==source[1] &&
                parent[2]==source[2] && parent[3]==source[3] &&
                parent[4]==source[12]) {return Err(ManagedCliError::Busy)}
    }
    Ok(result)
}

fn decode_original_hex(value:&str)->Result<Vec<u8>,ManagedCliError>{
    if value.is_empty() || value.len()%2!=0 ||
        !value.bytes().all(|b|b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        return Err(ManagedCliError::Busy);
    }
    value.as_bytes().chunks_exact(2).map(|pair| {
        let text=std::str::from_utf8(pair).map_err(|_|ManagedCliError::Busy)?;
        u8::from_str_radix(text,16).map_err(|_|ManagedCliError::Busy)
    }).collect()
}

fn decode_original_frames(value:&str)->Result<Vec<Vec<u8>>,ManagedCliError>{
    let raw=decode_original_hex(value)?;
    let mut offset=0usize;
    let mut result=Vec::new();
    while offset<raw.len() {
        let end=offset.checked_add(8).ok_or(ManagedCliError::Busy)?;
        let size=u64::from_be_bytes(raw.get(offset..end).ok_or(ManagedCliError::Busy)?
            .try_into().map_err(|_|ManagedCliError::Busy)?) as usize;
        offset=end;
        let end=offset.checked_add(size).ok_or(ManagedCliError::Busy)?;
        result.push(raw.get(offset..end).ok_or(ManagedCliError::Busy)?.to_vec());
        offset=end;
        if result.len()>16 {return Err(ManagedCliError::Busy)}
    }
    Ok(result)
}

fn original_global_observation(db:&VerifiedDatabaseConnection<'_>,row:&[String])
    ->Result<String,ManagedCliError>{
    let producer=if row[0].starts_with("owner-login-status-") {"owner-status"}
        else if row[0].starts_with("owner-login-") {"owner-login"}
        else if row[0].starts_with("login-observe-") {"login-observe"}
        else {return Err(ManagedCliError::Observation(
            "global C lacks a recognized original login producer".into()))};
    let operations=Statement::prepare(db.as_ptr(),
        "SELECT request_id,request_hex,phase,COALESCE(receipt_json,''),
         receipt_json IS NULL,COALESCE(native_receipt_id,''),native_receipt_id IS NULL
         FROM main.gogoke_v37_instance_operations WHERE target_id=?1")?;
    operations.bind_text(1,&row[7])?;
    let mut matched=None;
    while operations.step_row()? {
        let fields=(0..7).map(|n|operations.column_text(n))
            .collect::<std::result::Result<Vec<_>,_>>()?;
        let frames=match decode_original_frames(&fields[1]) {
            Ok(value)=>value,Err(_)=>continue,
        };
        let kind=frames.first().map(Vec::as_slice);
        if (kind!=Some(&b"observe"[..]) && kind!=Some(&b"read-login"[..])) ||
            !matches!(frames.len(),6|7) {continue}
        let raw=&frames[1];
        let revision=std::str::from_utf8(&frames[3]).ok()
            .and_then(|value|value.parse::<u64>().ok());
        let Some(revision)=revision.filter(|n|*n>0) else {continue};
        let command=fields[0].strip_suffix("-account-read");
        let synthetic=command.and_then(|id|{
            let raw=std::str::from_utf8(raw).ok()?;
            let instance=&row[7];
            if raw==format!("owner-login-account-read:{instance}:{id}") {
                Some("codex-account")
            } else if raw==format!("owner-login-provider-status:{instance}:{id}") {
                Some("provider-status")
            } else if raw==format!("owner-login-opencode-xai-cli-completion:{instance}:{id}") {
                Some("opencode-completion")
            } else {None}
        });
        let json=if synthetic.is_none() {
            crate::store::session_transport::decode_request(raw).ok()
        } else {None};
        let original_json=json.as_ref().is_some_and(|request|
            request.family=="K-INSTANCE" && request.operation=="login-state" &&
            request.domain_id=="global" && request.target_id==row[7] &&
            request.request_id==fields[0] && request.expected_revision==revision &&
            request.payload.is_empty());
        let operation=match producer {
            "login-observe" if original_json || synthetic==Some("codex-account")=>
                format!("login-observe-{}",&sha256_hex(raw)[..40]),
            "owner-login" if synthetic.is_some() => {
                let id=command.ok_or(ManagedCliError::Busy)?;
                let source=format!("{}\n{}\n{}",row[7],id,revision);
                format!("owner-login-{}",&sha256_hex(source.as_bytes())[..40])
            },
            "owner-status" if synthetic==Some("provider-status") => {
                let id=command.ok_or(ManagedCliError::Busy)?;
                let source=format!("{}\n{}\n{}",row[7],id,revision);
                let login=format!("owner-login-{}",&sha256_hex(source.as_bytes())[..40]);
                format!("owner-login-status-{}",&sha256_hex(
                    format!("{login}\nstatus").as_bytes())[..40])
            },
            _=>continue,
        };
        if row[0]!=operation {continue}
        if frames[2].as_slice()!=row[7].as_bytes() ||
            frames[3].as_slice()!=revision.to_string().as_bytes() ||
            frames[4].as_slice()!=b"login" ||
            ![&b"UNKNOWN"[..],&b"LOGGED_IN"[..],&b"LOGGED_OUT"[..]]
                .contains(&frames[5].as_slice()) ||
            (frames.len()==7 && (frames[0].as_slice()!=b"read-login" ||
                frames[6].as_slice()!=b"NO_REVISION_CHANGE")) ||
            (frames.len()==6 && frames[0].as_slice()!=b"observe") ||
            row[9]!=revision.to_string() ||
            fields[2]!="APPLIED" || fields[4]!="1" || fields[6]!="1" ||
            !fields[3].is_empty() || !fields[5].is_empty() {
            return Err(ManagedCliError::Busy)
        }
        if matched.replace(fields).is_some(){return Err(ManagedCliError::Busy)}
    }
    let fields=matched.ok_or(ManagedCliError::Observation(format!(
        "original {producer} F operation/raw source missing")))?;
    Ok(sha256_hex(&frame(&fields)))
}

fn original_open_equations(request:&crate::store::session_transport::V37Request,
    r:&[String])->bool {
    let (Ok(before),Ok(after))=(r[11].parse::<u64>(),r[12].parse::<u64>()) else {
        return false
    };
    if request.expected_revision!=before || r[11]!=before.to_string() ||
        r[12]!=after.to_string() || r[32]!=r[11] || r[33]!=r[12] ||
        !(match r[30].as_str() {
            "APPLIED"=>before.checked_add(1)==Some(after),
            // The original pre-launch UNKNOWN intention has not advanced.
            "UNKNOWN"=>after==before || before.checked_add(1)==Some(after),
            _=>false,
        }) {return false}
    if r[2]=="global" {return request.payload.is_empty()}
    let field=|name:&str| match request.payload.get(&JsonString::from_str(name)) {
        Some(Json::String(value))=>value.to_well_formed_string(),_=>None,
    };
    let formal=field("purpose").as_deref()==Some("FORMAL_REVIEW");
    request.payload.len()==if formal{5}else{4} &&
        field("generation").as_deref()==Some(r[4].as_str()) &&
        field("seatId").as_deref()==Some(r[38].as_str()) &&
        field("repositoryId").is_some_and(|value|!value.is_empty()) &&
        field("worktreeId").is_some_and(|value|!value.is_empty())
}

fn verified_original_worktree(db:&VerifiedDatabaseConnection<'_>,
    request:&crate::store::session_transport::V37Request,
    instance:&str,domain:&str,generation:&str,seat:&str,incarnation:&str)
    ->Result<String,ManagedCliError>{
    if domain=="global" {
        // Native V2 selections and bindings are immutable per session. The
        // physical H generation is independent of E authorization generation.
        let q=Statement::prepare(db.as_ptr(),
            "SELECT p.domain_id,p.session_id,p.seat_id,p.seat_incarnation,
             CAST(p.seat_authorization_generation AS TEXT),p.selected_instance_id,'NATIVE_V2'
             FROM main.gogoke_v37_native_selection p
             JOIN main.gogoke_v37_session_binding_v2 v ON v.domain_id=p.domain_id
               AND v.session_id=p.session_id AND v.seat_id=p.seat_id
               AND v.seat_incarnation=p.seat_incarnation
               AND v.seat_authorization_generation=p.seat_authorization_generation
               AND v.selected_instance_id=p.selected_instance_id AND v.provenance='NATIVE_V2'
             JOIN main.gogoke_v37_seat_secretary d ON d.domain_id=p.domain_id
               AND d.seat_id=p.seat_id AND d.incarnation=p.seat_incarnation
             WHERE p.domain_id='global' AND p.session_id=?1 AND p.seat_id=?2
               AND p.seat_incarnation=?3 AND p.selected_instance_id=?4
             UNION ALL
             SELECT b.domain_id,b.session_id,b.seat_id,b.seat_incarnation,
               b.generation,c.instance_id,'LEGACY_V1'
             FROM main.gogoke_v37_h_seat_binding b
             JOIN main.gogoke_v37_h_claim c ON c.domain_id=b.domain_id
               AND c.session_id=b.session_id AND c.generation=b.generation
             JOIN main.gogoke_v37_seat_secretary d ON d.domain_id=b.domain_id
               AND d.seat_id=b.seat_id AND d.incarnation=b.seat_incarnation
             WHERE b.domain_id='global' AND b.session_id=?1 AND b.seat_id=?2
               AND b.seat_incarnation=?3 AND c.instance_id=?4
               AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_native_selection n
                   WHERE n.domain_id=b.domain_id AND n.session_id=b.session_id)")?;
        for (n,value) in [request.target_id.as_str(),seat,incarnation,instance].iter().enumerate(){
            q.bind_text(n as i32+1,value)?;
        }
        if !q.step_row()? {return Err(ManagedCliError::Busy)}
        let values=(0..7).map(|n|q.column_text(n))
            .collect::<std::result::Result<Vec<_>,_>>()?;
        if q.step_row()? || request.operation=="open" && values[4]!=generation {
            return Err(ManagedCliError::Busy)
        }
        return Ok(sha256_hex(&frame(&values)))
    }
    let field=|name:&str| match request.payload.get(&JsonString::from_str(name)) {
        Some(Json::String(value))=>value.to_well_formed_string(),_=>None,
    };
    let repository=field("repositoryId").ok_or(ManagedCliError::Busy)?;
    let worktree=field("worktreeId").ok_or(ManagedCliError::Busy)?;
    let q=Statement::prepare(db.as_ptr(),
        "SELECT w.instance_id,w.domain_id,w.repository_id,w.seat_id,
         w.seat_incarnation,w.worktree_path,w.worktree_identity,w.state,
         l.state,s.source_path,s.source_identity
         FROM main.gogoke_v37_worktrees w
         JOIN main.gogoke_v37_worktree_lifecycle l ON l.worktree_id=w.worktree_id
         JOIN main.gogoke_v37_worktree_sources s ON s.repository_id=w.repository_id
         WHERE w.worktree_id=?1")?;
    q.bind_text(1,&worktree)?;
    if !q.step_row()? {return Err(ManagedCliError::Busy)}
    let values=(0..11).map(|n|q.column_text(n))
        .collect::<std::result::Result<Vec<_>,_>>()?;
    if q.step_row()? || values[0]!=instance || values[1]!=domain ||
        values[2]!=repository || values[3]!=seat || values[4]!=incarnation ||
        values[7]!="REGISTERED" || values[8]!="REGISTERED" {
        return Err(ManagedCliError::Busy)
    }
    for (path,identity) in [(&values[5],&values[6]),(&values[9],&values[10])] {
        let observed=inspect_root(std::path::Path::new(path))
            .map_err(|error|ManagedCliError::Observation(format!(
                "original F worktree/source root: {error:?}")))?;
        if observed.identity.opaque()!=*identity {return Err(ManagedCliError::IdentityChanged)}
    }
    Ok(sha256_hex(&frame(&values)))
}

/// `mark_resume_unknown` intentionally leaves the old STOPPED claim in place
/// and does not create h_generation. Authenticate the F create producer and
/// the physical child HOME without interpreting those absent rows as damage.
fn prepromotion_resume_home(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,
    r:&[String],raw:&[u8])->Result<String,ManagedCliError>{
    let suffix=r[7].strip_prefix("home-").ok_or(ManagedCliError::Busy)?;
    if suffix.len()!=40 || !suffix.bytes().all(|b|b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        return Err(ManagedCliError::Busy)
    }
    let profile=crate::process::AppContainerProfile::derive_for_revocation(
        &format!("Gogoke37.Session.{suffix}"))
        .map_err(|error|ManagedCliError::Observation(format!("resume child profile: {error:?}")))?;
    let sid=profile.sid_identity().map_err(|error|
        ManagedCliError::Observation(format!("resume child SID: {error:?}")))?;
    let (_,identity)=super::super::temporary::resolve_active_session_home(db,root,&profile,
        &r[0],&r[7],&r[1],&r[2],&r[3])
        .map_err(|error|ManagedCliError::Observation(format!("resume child physical HOME: {error:?}")))?;
    if identity.opaque()!=r[37] || r[35]!="ACTIVE" ||
        r[36]!=format!("temp-home-{}",sha256_hex(r[7].as_bytes())) {
        return Err(ManagedCliError::IdentityChanged)
    }
    let id=format!("homeprep-{}",&sha256_hex(format!("{}\n{}",r[1],r[5]).as_bytes())[..40]);
    let expected=hex(&frame(&["temporary-create-detached".into(),
        String::from_utf8(raw.to_vec()).map_err(|_|ManagedCliError::Busy)?,
        r[7].clone(),r[0].clone(),r[1].clone(),"SESSION".into(),
        r[2].clone(),r[3].clone(),sid]));
    let operation=Statement::prepare(db.as_ptr(),
        "SELECT request_hex,phase,COALESCE(native_receipt_id,''),native_receipt_id IS NULL FROM main.gogoke_v37_instance_operations WHERE request_id=?1 AND target_id=?2")?;
    operation.bind_text(1,&id)?;operation.bind_text(2,&r[7])?;
    if !operation.step_row()? {return Err(ManagedCliError::Busy)}
    let values=(0..4).map(|n|operation.column_text(n))
        .collect::<std::result::Result<Vec<_>,_>>()?;
    if operation.step_row()? || values[0]!=expected || values[1]!="APPLIED" ||
        values[2]!=r[37] || values[3]!="0" {return Err(ManagedCliError::Busy)}
    let duplicates=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND request_id!=?2 AND request_hex GLOB ?3 LIMIT 1")?;
    duplicates.bind_text(1,&r[7])?;duplicates.bind_text(2,&id)?;
    duplicates.bind_text(3,&format!("{:016x}{}*",b"temporary-create-detached".len(),
        hex(b"temporary-create-detached")))?;
    if duplicates.step_row()? {return Err(ManagedCliError::Busy)}
    Ok(sha256_hex(&frame(&[id,expected,values[1].clone(),values[2].clone(),
        r[35].clone(),r[36].clone(),r[37].clone(),r[38].clone()])))
}

/// Plain resume has two original producer shapes. Before promotion the child
/// is UNKNOWN and the stopped parent remains the current claim; after it,
/// h_generation and the child claim are present. The parent is independently
/// authenticated as an original STOP member before either shape is admitted.
fn original_resume_units(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str,
    stopped:&[(Vec<String>,Vec<String>)])
    ->Result<(Vec<(Unit,Vec<String>)>,std::collections::BTreeMap<String,String>),ManagedCliError>{
    let sql="SELECT e.instance_id,e.domain_id,e.session_id,e.generation,
        COALESCE(e.old_generation,''),e.request_id,e.process_operation_id,e.home_id,
        e.binding_id,e.raw_hex,CAST(e.previous_revision AS TEXT),
        COALESCE(CAST(e.result_revision AS TEXT),''),e.phase,
        COALESCE(e.seat_id,''),COALESCE(e.seat_incarnation,''),
        e.stop_request_id IS NULL,e.stop_fact_id IS NULL,c.ticket,c.custodian_nonce,
        c.pid,c.creation_time_100ns,c.image_path,c.binary_digest_sha256,c.state,
        c.stop_proof_hash IS NULL,i.home_identity,i.program_digest,i.version,
        a.generation,a.state,CAST(a.revision AS TEXT),a.home_id,a.binding_id,
        COALESCE(a.process_operation_id,''),COALESCE(a.stop_fact_id,''),
        h.state,h.directory_ref,h.directory_identity,CAST(h.revision AS TEXT),b.state,
        (SELECT COUNT(*) FROM main.gogoke_v37_h_generation g WHERE g.domain_id=e.domain_id
          AND g.session_id=e.session_id AND g.generation=e.generation
          AND g.request_id=e.request_id AND g.process_operation_id=e.process_operation_id),
        (SELECT COUNT(*) FROM main.gogoke_v37_h_generation_change g WHERE
          g.domain_id=e.domain_id AND g.request_id=e.request_id),
        (SELECT COUNT(*) FROM main.gogoke_v37_h_operation o WHERE
          o.domain_id=e.domain_id AND o.request_id=e.request_id)
        FROM main.gogoke_v37_h_process_episode e
        JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
        JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id
          AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id
          AND c.generation=e.generation
        JOIN main.gogoke_v37_h_claim a ON a.instance_id=e.instance_id
          AND a.domain_id=e.domain_id AND a.session_id=e.session_id
        JOIN main.gogoke_v37_instance_homes h ON h.home_id=e.home_id
          AND h.instance_id=e.instance_id AND h.domain_id=e.domain_id
          AND h.generation=e.generation AND h.kind='SESSION' AND h.owner_id=e.session_id
        JOIN main.gogoke_v37_h_owner_binding b ON b.binding_id=e.binding_id
          AND b.instance_id=e.instance_id AND b.domain_id=e.domain_id
          AND b.generation=e.generation AND b.kind='SESSION' AND b.owner_id=e.session_id
        WHERE i.driver_id=?1 AND e.old_generation IS NOT NULL
          AND e.phase IN ('PREPARED','ACTIVE','UNKNOWN')";
    let mut result=Vec::new();
    let mut before_promotion=std::collections::BTreeMap::new();
    for r in rows(db,sql,driver,43)? {
        let old=r[4].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
        let generation=r[3].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
        if old.checked_add(1)!=Some(generation) || r[3]!=generation.to_string() ||
            r[4]!=old.to_string() {return Err(ManagedCliError::Busy)}
        let parent=stopped.iter().find(|(source,_)|source[1]==r[0] &&
            source[2]==r[1] && source[3]==r[2] && source[4]==r[4])
            .ok_or(ManagedCliError::Busy)?;
        if parent.0[28]!=r[13] || parent.0[29]!=r[14] ||
            r[13].is_empty() || r[14].is_empty() ||
            r[15]!="1" || r[16]!="1" || r[24]!="1" ||
            r[17].is_empty() || r[18].is_empty() ||
            r[21].is_empty() || r[22].is_empty() || r[23].is_empty() ||
            r[39]!="ACTIVE" || r[41]!="0" || r[42]!="0" ||
            r[36]!=format!("temp-home-{}",sha256_hex(r[7].as_bytes())) ||
            r[37].is_empty() {return Err(ManagedCliError::Busy)}
        let raw=decode_original_hex(&r[9])?;
        let request=crate::store::session_transport::decode_request(&raw)
            .map_err(|error|ManagedCliError::Observation(format!(
                "original plain H resume raw: {error:?}")))?;
        if matches!(request.operation.as_str(),"compact"|"renew-session") {
            return Err(ManagedCliError::Observation(
                "unsupported original compound generation-change source".into()))
        }
        let previous=r[10].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
        let after=r[11].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
        let payload=match request.payload.get(&JsonString::from_str("generation")) {
            Some(Json::String(value))=>value.to_well_formed_string(),_=>None,
        };
        if request.family!="K-SESSION" || request.operation!="resume" ||
            request.request_id!=r[5] || request.domain_id!=r[1] ||
            request.target_id!=r[2] || request.payload.len()!=1 ||
            payload.as_deref()!=Some(r[4].as_str()) ||
            r[6]!=format!("h-resume-{}",&sha256_hex(&raw)[..40]) ||
            !(request.expected_revision==previous ||
              request.expected_revision.checked_add(1)==Some(previous)) ||
            previous.checked_add(1)!=Some(after) ||
            r[19].parse::<u32>().ok().filter(|n|*n>0).is_none() ||
            r[20].parse::<u64>().ok().filter(|n|*n>0).is_none() ||
            r[22]!=parent.0[26] ||
            r[38].parse::<u64>().is_err() {return Err(ManagedCliError::Busy)}
        let pending=r[12]=="UNKNOWN" && r[23]=="UNKNOWN" && r[40]=="0";
        let promoted=r[40]=="1" &&
            matches!(r[12].as_str(),"ACTIVE"|"UNKNOWN") &&
            matches!(r[23].as_str(),"ACTIVE"|"UNKNOWN") &&
            r[28]==r[3] && matches!(r[29].as_str(),"COMMITTED"|"UNKNOWN") &&
            r[30]==r[11] && r[31]==r[7] && r[32]==r[8] &&
            r[33]==r[6] && r[34].is_empty();
        if pending {
            // The original mark_resume_unknown writer moves only the parent's
            // revision. There is deliberately no child generation/claim.
            if r[28]!=r[4] || r[29]!="STOPPED" || r[30]!=r[11] ||
                r[31]!=parent.0[7] || r[32]!=parent.0[8] ||
                r[33]!=parent.0[6] || r[34]!=parent.0[14] ||
                request.expected_revision!=previous || r[35]!="ACTIVE" {
                return Err(ManagedCliError::Busy)
            }
        } else if !promoted {return Err(ManagedCliError::Busy)}
        let home_source=if pending {
            prepromotion_resume_home(db,root,&r,&raw)?
        } else {
            super::super::temporary::validate_original_execution_home_source(
                db,&r[7],&r[0],&r[1],&r[2],&r[3],&r[5],"resume")
                .map_err(|error|ManagedCliError::Observation(format!(
                    "original promoted resume HOME: {error:?}")))?
        };
        let home_identity=registry::observed_home(root,&r[0])
            .map_err(|error|ManagedCliError::Observation(format!(
                "original resume instance home: {error:?}")))?
            .ok_or(ManagedCliError::Busy)?;
        if home_identity.opaque()!=r[25] {return Err(ManagedCliError::IdentityChanged)}
        verified_source_images(db,root,driver,&r[0],&r[25],&r[26],&r[27],
            &r[21],&r[22])?;
        let pairs=vec![(r[19].parse::<u32>().map_err(|_|ManagedCliError::Busy)?,
            r[20].parse::<u64>().map_err(|_|ManagedCliError::Busy)?)];
        if r[19]!=pairs[0].0.to_string() || r[20]!=pairs[0].1.to_string() {
            return Err(ManagedCliError::Busy)
        }
        let unit=Unit{instance:r[0].clone(),domain:r[1].clone(),session:r[2].clone(),
            generation:r[3].clone(),open_request:r[5].clone(),process:r[6].clone(),
            home:r[7].clone(),binding:r[8].clone(),pairs};
        let mut source=vec![if pending{"H_SESSION_RESUME_PENDING-v1"}
            else{"H_SESSION_RESUME-v1"}.to_owned()];
        source.extend(r[..26].iter().cloned());
        source.extend(r[28..].iter().cloned());
        source.push(home_source);
        if pending {
            before_promotion.insert(r[6].clone(),sha256_hex(&frame(&source)));
        }
        result.push((unit,source));
    }
    result.sort_by(|a,b|(&a.0.instance,&a.0.domain,&a.0.session,&a.0.generation,&a.0.process)
        .cmp(&(&b.0.instance,&b.0.domain,&b.0.session,&b.0.generation,&b.0.process)));
    if result.windows(2).any(|p|p[0].0.process==p[1].0.process) {
        return Err(ManagedCliError::Busy)
    }
    Ok((result,before_promotion))
}

/// The existing H single-HOME source rejects a second unsettled family
/// episode and pending stdin. Its per-HOME cardinality is too narrow for a
/// complete family, so F rechecks each original C generation and the existing
/// read-only F physical HOME resolver before recording the member source.
fn family_execution_source(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,
    unit:&Unit,operation:&str,expected_identity:&str)
    ->Result<(String,String),ManagedCliError>{
    let exact=Statement::prepare(db.as_ptr(),
        "SELECT profile_id,domain_id,generation,pid,creation_time_100ns FROM main.gogoke_coordination_process_custody WHERE operation_id=?1")?;
    exact.bind_text(1,&unit.process)?;
    if !exact.step_row()? {return Err(ManagedCliError::Busy)}
    let c=(0..5).map(|n|exact.column_text(n))
        .collect::<std::result::Result<Vec<_>,_>>()?;
    let pair=c[3].parse::<u32>().ok().zip(c[4].parse::<u64>().ok());
    if exact.step_row()? || c[0]!=unit.instance || c[1]!=unit.domain ||
        c[2]!=unit.generation || pair.is_none_or(|pair|pair.0==0 || pair.1==0 ||
            c[3]!=pair.0.to_string() || c[4]!=pair.1.to_string() ||
            vec![pair]!=unit.pairs) {
        return Err(ManagedCliError::Busy)
    }
    let suffix=unit.home.strip_prefix("home-").ok_or(ManagedCliError::Busy)?;
    if suffix.len()!=40 || !suffix.bytes().all(|b|b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        return Err(ManagedCliError::Busy)
    }
    let profile=crate::process::AppContainerProfile::derive_for_revocation(
        &format!("Gogoke37.Session.{suffix}"))
        .map_err(|error|ManagedCliError::Observation(format!("family HOME profile: {error:?}")))?;
    let (_,physical)=super::super::temporary::resolve_historical_session_home(
        db,root,&profile,&unit.instance,&unit.home,&unit.domain,&unit.session,
        &unit.generation,&unit.binding)
        .map_err(|error|ManagedCliError::Observation(format!(
            "family original physical HOME: {error:?}")))?;
    if physical.opaque()!=expected_identity {return Err(ManagedCliError::IdentityChanged)}
    let home_source=super::super::temporary::validate_original_execution_home_source(
        db,&unit.home,&unit.instance,&unit.domain,&unit.session,&unit.generation,
        &unit.open_request,operation)
        .map_err(|error|ManagedCliError::Observation(format!(
            "family original F HOME: {error:?}")))?;
    let physical_source=sha256_hex(&frame(&[db.root_identity().opaque(),
        db.identity().opaque(),unit.instance.clone(),unit.domain.clone(),
        unit.session.clone(),unit.generation.clone(),unit.home.clone(),
        unit.binding.clone(),unit.process.clone(),physical.opaque()]));
    Ok((home_source,physical_source))
}

/// Global login/observer custody has no H episode by original design. It is
/// enumerated independently; the existing applied ACL fence reader validates
/// the complete old UNKNOWN C set and fresh kernel cutoff on every call.
fn original_global_sources(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str)
    ->Result<(Vec<(Vec<String>,Vec<String>)>,Vec<String>,
        Vec<(Vec<(u32,u64)>,NativeLegacyHoldersGone)>),ManagedCliError>{
    let query="SELECT c.operation_id,c.ticket,c.custodian_nonce,c.pid,
        c.creation_time_100ns,c.image_path,c.binary_digest_sha256,c.profile_id,
        c.domain_id,c.generation,c.state,COALESCE(c.stop_proof_hash,''),
        c.stop_proof_hash IS NULL,i.home_identity,i.program_digest,i.version
        FROM main.gogoke_coordination_process_custody c
        JOIN main.gogoke_v37_instances i ON i.instance_id=c.profile_id
        WHERE i.driver_id=?1 AND c.domain_id='global' AND NOT EXISTS(
            SELECT 1 FROM main.gogoke_v37_h_process_episode e
            WHERE e.process_operation_id=c.operation_id)
        ORDER BY c.operation_id";
    let original=rows(db,query,driver,16)?;
    if original.is_empty(){return Ok((Vec::new(),Vec::new(),Vec::new()))}
    super::no_unsettled_global_cli_custody(db,driver)?;
    let mut members=Vec::new();
    let mut operations=Vec::new();
    let mut native=Vec::new();
    for row in original {
        let observation=original_global_observation(db,&row)?;
        if row[1].is_empty() || row[2].is_empty() || row[5].is_empty() ||
            row[7].is_empty() || row[8]!="global" ||
            !matches!(row[10].as_str(),"UNKNOWN"|"STOPPED") {
            return Err(ManagedCliError::Busy)
        }
        let pid=row[3].parse::<u32>().map_err(|_|ManagedCliError::Busy)?;
        let created=row[4].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
        if pid==0 || created==0 || row[3]!=pid.to_string() ||
            row[4]!=created.to_string() {return Err(ManagedCliError::Busy)}
        verified_current_managed_source(db,root,driver,&row[7],&row[13],
            &row[14],&row[15])?;
        let original_pin=original_catalog_pin(driver,&row[6])?;
        super::sha256_file(std::path::Path::new(&row[5]),original_pin.image_sha256)?;
        let kind=if row[10]=="UNKNOWN" {
            if !row[11].is_empty() || row[12]!="1" || driver!="codex" {
                return Err(ManagedCliError::Busy)
            }
            let pairs=vec![(pid,created)];
            let fact=NativeLegacyHoldersGone::observe(&pairs).map_err(|error|
                ManagedCliError::NativeGone(format!("global original kernel cutoff: {error:?}")))?;
            fact.validate(&pairs).map_err(|error|
                ManagedCliError::NativeGone(format!("global original kernel recheck: {error:?}")))?;
            native.push((pairs,fact));
            "KERNEL_REBOOT"
        } else {
            if row[12]!="0" || !row[11].starts_with("sha256:") ||
                row[11].len()!=71 || !row[11][7..].bytes().all(|b|
                    b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
                return Err(ManagedCliError::Busy)
            }
            "ORIGINAL_STOP"
        };
        let mut source=std::iter::once("GLOBAL_LOGIN_C-v1".to_owned())
            .chain(row[..14].iter().cloned()).collect::<Vec<_>>();
        source.push(observation);
        let proof=vec![kind.to_owned(),row[7].clone(),"global".to_owned(),
            row[0].clone(),row[9].clone(),row[0].clone(),
            pid.to_string(),created.to_string()];
        operations.push(row[0].clone());
        members.push((source,proof));
    }
    Ok((members,operations,native))
}

/// The managed --version probe is an action-owned C effect. Retained earlier
/// probes are part of the original family. The one new row is omitted from
/// that immutable source only after its exact action producer is verified.
fn original_probe_sources(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str,
    current:Option<&ActionRecord>,live:Option<&NativeStopProof>)
    ->Result<Vec<(Vec<String>,Vec<String>)>,ManagedCliError>{
    let profile=format!("managed-cli-{driver}");
    let misplaced=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_coordination_process_custody WHERE
         operation_id GLOB ?1 AND profile_id!=?2 LIMIT 1")?;
    misplaced.bind_text(1,&format!("managed-probe-{driver}-*"))?;
    misplaced.bind_text(2,&profile)?;
    if misplaced.step_row()? {return Err(ManagedCliError::Busy)}
    let custody_rows=rows(db,"SELECT operation_id,ticket,custodian_nonce,pid,
        creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,
        generation,state,COALESCE(stop_proof_hash,''),stop_proof_hash IS NULL
        FROM main.gogoke_coordination_process_custody WHERE profile_id=?1",
        &profile,13)?;
    let requests=rows(db,"SELECT request_id FROM main.gogoke_v37_instance_version_actions WHERE driver_id=?1 ORDER BY request_id",driver,1)?;
    let mut actions=Vec::new();
    for request in requests {
        let action=managed_version_action::read(db,root,&request[0])
            .map_err(|error|ManagedCliError::Observation(format!(
                "original probe action: {error:?}")))?.ok_or(ManagedCliError::Busy)?;
        actions.push(action);
    }
    let mut original=Vec::new();
    let mut attempts=std::collections::BTreeSet::new();
    let mut own_count=0;
    for row in &custody_rows {
        let pid=row[3].parse::<u32>().map_err(|_|ManagedCliError::Busy)?;
        let created=row[4].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
        let generation=row[9].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
        if pid==0 || created==0 || generation==0 || row[3]!=pid.to_string() ||
            row[4]!=created.to_string() || row[9]!=generation.to_string() ||
            row[1].is_empty() || row[2].is_empty() || row[5].is_empty() ||
            row[7]!=profile || row[8]!="global" || row[10]!="STOPPED" ||
            row[12]!="0" || !row[11].starts_with("sha256:") || row[11].len()!=71 ||
            !row[11][7..].bytes().all(|b|b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
            return Err(ManagedCliError::Busy)
        }
        let mut matched=None;
        for action in &actions {
            let prefix=format!("managed-probe-{driver}-{}-",
                &sha256_hex(action.request_id.as_bytes())[..40]);
            if let Some(suffix)=row[0].strip_prefix(&prefix) {
                let attempt=suffix.parse::<usize>().map_err(|_|ManagedCliError::Busy)?;
                if attempt==0 || suffix!=attempt.to_string() || matched.is_some() {
                    return Err(ManagedCliError::Busy)
                }
                matched=Some((action,attempt));
            }
        }
        let (owner,attempt)=matched.ok_or(ManagedCliError::Busy)?;
        if !attempts.insert(attempt) || attempt>custody_rows.len() {return Err(ManagedCliError::Busy)}
        let pin=read_official_cli_version(driver,&owner.target_version)
            .ok_or(ManagedCliError::Busy)?;
        if row[6]!=format!("sha256:{}",pin.image_sha256) ||
            owner.target_image_sha256!=pin.image_sha256 ||
            owner.stage_name.is_empty() || owner.probe_hex.is_empty() {
            return Err(ManagedCliError::Busy)
        }
        let staged=inspect_staged_official_cli_version(root,driver,pin.version,
            &owner.stage_name)?;
        if std::fs::canonicalize(&staged)?!=
            std::fs::canonicalize(std::path::Path::new(&row[5]))? {
            return Err(ManagedCliError::IdentityChanged)
        }
        if current.is_some_and(|action|action.request_id==owner.request_id) {
            own_count+=1;
            if attempt!=custody_rows.len() || owner.phase==managed_version_action::ActionPhase::Failed {
                return Err(ManagedCliError::Busy)
            }
            if owner.phase==managed_version_action::ActionPhase::Probing {
                let proof=live.ok_or(ManagedCliError::Busy)?;
                if !proof.parent_exited || !proof.writer_fence_verified ||
                    proof.active_job_processes!=Some(0) || !proof.errors.is_empty() ||
                    proof.exit_code!=Some(0) || proof.deadline_exceeded ||
                    proof.ticket.opaque()!=row[1] || proof.custodian_nonce!=row[2] ||
                    proof.identity.pid!=pid || proof.identity.creation_time_100ns!=created ||
                    std::fs::canonicalize(&proof.identity.image_path)?!=
                        std::fs::canonicalize(std::path::Path::new(&row[5]))? ||
                    proof.binding.profile_id!=profile || proof.binding.domain_id!="global" ||
                    proof.binding.generation!=row[9] ||
                    proof.binding.binary_digest_sha256!=row[6] ||
                    proof.proof_hash()!=row[11] {return Err(ManagedCliError::Busy)}
            } else if !matches!(owner.phase,managed_version_action::ActionPhase::Probed |
                managed_version_action::ActionPhase::Consumed) {
                return Err(ManagedCliError::Busy)
            } else {
                let witness=managed_version_action::bound_probe_witness(owner)
                    .map_err(|error|ManagedCliError::Observation(format!(
                        "same-action original probe witness: {error:?}")))?;
                if witness.operation_id!=row[0] || witness.ticket!=row[1] ||
                    witness.nonce!=row[2] || witness.pid!=row[3] ||
                    witness.creation!=row[4] || witness.image!=row[5] ||
                    witness.digest!=row[6] || witness.profile!=row[7] ||
                    witness.domain!=row[8] || witness.generation!=row[9] ||
                    witness.proof_hash!=row[11] {return Err(ManagedCliError::Busy)}
            }
            continue;
        }
        if owner.phase!=managed_version_action::ActionPhase::Consumed {
            return Err(ManagedCliError::Busy)
        }
        let witness=managed_version_action::bound_probe_witness(owner)
            .map_err(|error|ManagedCliError::Observation(format!(
                "historical original probe witness: {error:?}")))?;
        if witness.operation_id!=row[0] || witness.ticket!=row[1] ||
            witness.nonce!=row[2] || witness.pid!=row[3] ||
            witness.creation!=row[4] || witness.image!=row[5] ||
            witness.digest!=row[6] || witness.profile!=row[7] ||
            witness.domain!=row[8] || witness.generation!=row[9] ||
            witness.proof_hash!=row[11] {return Err(ManagedCliError::Busy)}
        let mut source=vec!["MANAGED_PROBE_C-v1".to_owned()];
        source.extend(row.iter().cloned());
        source.extend([owner.request_id.clone(),owner.probe_hex.clone(),
            owner.probe_receipt_hex.clone(),owner.stage_name.clone(),
            owner.target_version.clone()]);
        let proof=vec!["ORIGINAL_STOP".to_owned(),profile.clone(),
            "managed-probe".to_owned(),row[0].clone(),row[9].clone(),
            row[0].clone(),row[3].clone(),row[4].clone()];
        original.push((source,proof));
    }
    if attempts.len()!=custody_rows.len() ||
        (1..=custody_rows.len()).any(|attempt|!attempts.contains(&attempt)) || own_count>1 {
        return Err(ManagedCliError::Busy)
    }
    if live.is_some() && (current.is_none_or(|action|
        action.phase!=managed_version_action::ActionPhase::Probing) || own_count!=1) {
        return Err(ManagedCliError::Busy)
    }
    if current.is_some_and(|action|matches!(action.phase,
        managed_version_action::ActionPhase::Probed |
        managed_version_action::ActionPhase::Consumed)) && own_count!=1 {
        return Err(ManagedCliError::Busy)
    }
    Ok(original)
}

fn independent_f_targets_clear(db:&VerifiedDatabaseConnection<'_>,driver:&str)
    ->Result<(),ManagedCliError>{
    let owners=Statement::prepare(db.as_ptr(),
        "SELECT i.instance_id,i.driver_id FROM main.gogoke_v37_instances i
         UNION ALL SELECT h.home_id,i.driver_id FROM main.gogoke_v37_instance_homes h
             JOIN main.gogoke_v37_instances i ON i.instance_id=h.instance_id
         UNION ALL SELECT h.history_id,i.driver_id FROM main.gogoke_v37_instance_histories h
             JOIN main.gogoke_v37_instances i ON i.instance_id=h.instance_id")?;
    let mut targets=std::collections::BTreeMap::<String,String>::new();
    while owners.step_row()? {
        let id=owners.column_text(0)?;
        let owner=owners.column_text(1)?;
        if id.is_empty() || owner.is_empty() || targets.insert(id.clone(),owner.clone()).is_some() {
            return Err(ManagedCliError::Busy)
        }
    }
    let all=Statement::prepare(db.as_ptr(),
        "SELECT instance_id,driver_id FROM main.gogoke_v37_instances")?;
    while all.step_row()? {
        let id=all.column_text(0)?;
        let owner=all.column_text(1)?;
        let credential=format!("credential-instance-{}",sha256_hex(id.as_bytes()));
        if targets.insert(credential,owner).is_some() {return Err(ManagedCliError::Busy)}
    }
    let pending=Statement::prepare(db.as_ptr(),
        "SELECT target_id,phase FROM main.gogoke_v37_instance_operations WHERE phase!='APPLIED'")?;
    while pending.step_row()? {
        let target=pending.column_text(0)?;
        let owner=targets.get(&target).ok_or(ManagedCliError::Busy)?;
        if owner==driver {return Err(ManagedCliError::Busy)}
    }
    Ok(())
}

fn no_unowned_original_h_rows(db:&VerifiedDatabaseConnection<'_>)
    ->Result<(),ManagedCliError>{
    for sql in [
        "SELECT 1 FROM main.gogoke_v37_h_generation g WHERE NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e WHERE e.domain_id=g.domain_id AND e.session_id=g.session_id AND e.generation=g.generation AND e.request_id=g.request_id AND e.process_operation_id=g.process_operation_id) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_process_episode e WHERE NOT EXISTS(SELECT 1 FROM main.gogoke_v37_instances i WHERE i.instance_id=e.instance_id) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_owner_binding b WHERE NOT EXISTS(SELECT 1 FROM main.gogoke_v37_instances i WHERE i.instance_id=b.instance_id) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_owner_binding b WHERE NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_claim c WHERE c.binding_id=b.binding_id) AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e WHERE e.binding_id=b.binding_id) LIMIT 1",
        "SELECT 1 FROM main.gogoke_coordination_process_custody c JOIN main.gogoke_v37_instances i ON i.instance_id=c.profile_id WHERE c.domain_id!='global' AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e WHERE e.process_operation_id=c.operation_id AND e.instance_id=c.profile_id AND e.domain_id=c.domain_id AND e.generation=c.generation) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_instance_homes h WHERE h.state NOT IN ('CLOSED','CLEANED') AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_claim c WHERE c.home_id=h.home_id) AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e WHERE e.home_id=h.home_id) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_operation o WHERE NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_claim c WHERE c.domain_id=o.domain_id AND c.session_id=o.session_id) AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e WHERE e.domain_id=o.domain_id AND e.session_id=o.session_id) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_stdin_journal j WHERE NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e WHERE e.domain_id=j.domain_id AND e.session_id=j.session_id AND e.generation=j.generation AND e.process_operation_id=j.process_operation_id) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_rpc_steps s WHERE NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e WHERE e.domain_id=s.domain_id AND e.session_id=s.session_id AND e.generation=s.generation AND e.process_operation_id=s.process_operation_id AND e.request_id=s.open_request_id) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_generation_change g WHERE NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e WHERE e.domain_id=g.domain_id AND e.session_id=g.session_id AND e.process_operation_id=g.old_process_operation_id) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_dos_alias a WHERE NOT EXISTS(SELECT 1 FROM main.gogoke_v37_instances i WHERE i.instance_id=a.instance_id) LIMIT 1",
    ] {
        let row=Statement::prepare(db.as_ptr(),sql)?;
        if row.step_row()? {return Err(ManagedCliError::Busy)}
    }
    Ok(())
}

/// Every original input, including RECEIPTED history, is an immutable member.
/// Only PREPARED/UNKNOWN may consume the future-occupancy exception.
fn original_stdin_sources(db:&VerifiedDatabaseConnection<'_>,driver:&str,
    units:&[(Unit,Vec<String>)],stopped:&[Unit])
    ->Result<Vec<(StdinMember,Vec<String>,Vec<String>)>,ManagedCliError>{
    let sql="SELECT e.instance_id,j.domain_id,j.session_id,j.generation,
        j.process_operation_id,j.request_id,j.operation,j.ticket,j.custodian_nonce,
        j.request_hex,j.phase,COALESCE(j.receipt_hex,''),COALESCE(j.receipt_status,''),
        j.expected_revision,COALESCE(j.receipt_previous_revision,''),
        COALESCE(j.receipt_revision,''),j.receipt_hex IS NULL,
        j.receipt_status IS NULL,j.receipt_previous_revision IS NULL,
        j.receipt_revision IS NULL,c.ticket,c.custodian_nonce,c.pid,
        c.creation_time_100ns,c.state,e.phase
        FROM main.gogoke_v37_h_stdin_journal j
        JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=j.domain_id
          AND e.session_id=j.session_id AND e.generation=j.generation
          AND e.process_operation_id=j.process_operation_id
        JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
        JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id
          AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id
          AND c.generation=e.generation
        WHERE i.driver_id=?1";
    let mut found=Vec::new();
    for r in rows(db,sql,driver,26)? {
        let unit=units.iter().map(|(u,_)|u).chain(stopped.iter())
            .find(|u|u.instance==r[0] && u.domain==r[1] &&
                u.session==r[2] && u.generation==r[3] && u.process==r[4])
            .ok_or(ManagedCliError::Busy)?;
        let pending=r[10]!="RECEIPTED";
        let raw=decode_original_hex(&r[9])?;
        let request=crate::store::session_transport::decode_request(&raw)
            .map_err(|error|ManagedCliError::Observation(format!(
                "original pending H stdin raw: {error:?}")))?;
        let revision=r[13].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
        if request.family!="K-SESSION" ||
            !matches!(request.operation.as_str(),"send"|"append-without-turn") ||
            (request.operation=="append-without-turn" && driver!="codex") ||
            request.operation!=r[6] || request.request_id!=r[5] ||
            request.domain_id!=r[1] || request.target_id!=r[2] ||
            request.expected_revision!=revision || r[13]!=revision.to_string() ||
            r[7].is_empty() || r[8].is_empty() || r[7]!=r[20] || r[8]!=r[21] ||
            !matches!(r[24].as_str(),"PREPARED"|"ACTIVE"|"UNKNOWN"|"STOPPED") ||
            !matches!(r[25].as_str(),"PREPARED"|"ACTIVE"|"UNKNOWN"|"STOPPED") ||
            (pending && ((r[24]=="STOPPED") != (r[25]=="STOPPED"))) ||
            (r[10]=="PREPARED" && !(r[16]=="1" && r[17]=="1" &&
                r[18]=="1" && r[19]=="1")) ||
            (r[10]=="UNKNOWN" && !(
                r[16]=="1" && r[17]=="1" && r[18]=="1" && r[19]=="1" ||
                r[16]=="0" && r[17]=="0" && r[18]=="0" && r[19]=="0" &&
                    r[12]=="UNKNOWN")) ||
            (r[10]=="RECEIPTED" && !(r[16]=="0" && r[17]=="0" &&
                r[18]=="0" && r[19]=="0" && r[12]!="UNKNOWN")) {
            return Err(ManagedCliError::Busy)
        }
        if r[16]=="0" {
            let receipt=crate::store::session_transport::decode_receipt(
                &decode_original_hex(&r[11])?).map_err(|error|
                ManagedCliError::Observation(format!("original pending H stdin receipt: {error:?}")))?;
            if receipt.family!="K-SESSION" || receipt.operation!=r[6] ||
                receipt.request_id!=r[5] || receipt.target_id!=r[2] ||
                receipt.status.wire()!=r[12] ||
                (r[10]=="UNKNOWN" &&
                    receipt.status!=crate::store::session_transport::V37Status::Unknown) ||
                (r[10]=="RECEIPTED" &&
                    receipt.status==crate::store::session_transport::V37Status::Unknown) ||
                receipt.previous_revision.to_string()!=r[14] ||
                receipt.revision.to_string()!=r[15] ||
                receipt.revision<receipt.previous_revision {return Err(ManagedCliError::Busy)}
        }
        let pid=r[22].parse::<u32>().map_err(|_|ManagedCliError::Busy)?;
        let created=r[23].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
        if pid==0 || created==0 || r[22]!=pid.to_string() ||
            r[23]!=created.to_string() || unit.pairs!=vec![(pid,created)] {
            return Err(ManagedCliError::Busy)
        }
        let member=StdinMember{instance:r[0].clone(),domain:r[1].clone(),session:r[2].clone(),
            generation:r[3].clone(),process:r[4].clone(),request:r[5].clone(),
            ticket:r[7].clone(),nonce:r[8].clone(),pending};
        let mut source=vec![if pending{"H_STDIN_PENDING-v1"}
            else{"H_STDIN_RECEIPTED-v1"}.to_owned()];
        source.extend(r);
        let proof=vec![if pending{"ORIGINAL_STDIN_OCCUPANCY"}
            else{"ORIGINAL_STDIN_HISTORY"}.to_owned(),member.instance.clone(),
            member.domain.clone(),member.session.clone(),member.generation.clone(),
            member.process.clone(),pid.to_string(),created.to_string(),
            member.request.clone()];
        found.push((member,source,proof));
    }
    found.sort_by(|a,b|(&a.0.instance,&a.0.domain,&a.0.session,&a.0.generation,
        &a.0.process,&a.0.request).cmp(&(&b.0.instance,&b.0.domain,&b.0.session,
        &b.0.generation,&b.0.process,&b.0.request)));
    if found.windows(2).any(|p|p[0].0.instance==p[1].0.instance &&
        p[0].0.domain==p[1].0.domain && p[0].0.request==p[1].0.request){
        return Err(ManagedCliError::Busy)
    }
    Ok(found)
}

/// RPC rows are bound as original side effects, not proof that an unfinished
/// command is live or that it received a response. OBSERVED alone requires
/// its original ledger source key and bytes.
fn original_rpc_sources(db:&VerifiedDatabaseConnection<'_>,driver:&str,
    units:&[(Unit,Vec<String>)],stopped:&[Unit])
    ->Result<Vec<(Vec<String>,Vec<String>)>,ManagedCliError>{
    let sql="SELECT e.instance_id,s.domain_id,s.session_id,s.open_request_id,
        s.step_id,s.process_operation_id,s.ticket,s.custodian_nonce,s.pid,
        s.creation_time,s.image_path,s.binary_digest,s.profile_id,s.generation,
        s.command_hex,CAST(s.requires_response AS TEXT),s.phase,
        COALESCE(s.source_epoch,''),COALESCE(s.source_cursor,''),
        COALESCE(s.original_error,''),COALESCE(s.permission_evidence,''),
        s.source_epoch IS NULL,s.source_cursor IS NULL,s.original_error IS NULL,
        s.permission_evidence IS NULL
        FROM main.gogoke_v37_rpc_steps s
        JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=s.domain_id
          AND e.session_id=s.session_id AND e.generation=s.generation
          AND e.process_operation_id=s.process_operation_id
          AND e.request_id=s.open_request_id
        JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
        WHERE i.driver_id=?1";
    let mut result=Vec::new();
    for r in rows(db,sql,driver,25)? {
        let u=units.iter().map(|(u,_)|u).chain(stopped.iter())
            .find(|u|u.instance==r[0] && u.domain==r[1] && u.session==r[2] &&
                u.open_request==r[3] && u.process==r[5] && u.generation==r[13])
            .ok_or(ManagedCliError::Busy)?;
        let command=decode_original_hex(&r[14])?;
        let pid=r[8].parse::<u32>().map_err(|_|ManagedCliError::Busy)?;
        let created=r[9].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
        if command.is_empty() || r[4].is_empty() || r[6].is_empty() ||
            r[7].is_empty() || r[10].is_empty() || r[11].is_empty() ||
            r[12]!=r[0] || pid==0 || created==0 ||
            r[8]!=pid.to_string() || r[9]!=created.to_string() ||
            u.pairs!=vec![(pid,created)] ||
            !matches!(r[15].as_str(),"0"|"1") ||
            !matches!(r[16].as_str(),"INTENT"|"WRITTEN"|"UNKNOWN"|"OBSERVED") ||
            (r[16]=="OBSERVED" && (r[15]!="1" || r[21]!="0" ||
                r[22]!="0" || r[23]!="1" || r[17].is_empty() ||
                r[18].parse::<u64>().ok().filter(|n|*n>0)
                    .is_none_or(|n|r[18]!=n.to_string()))) ||
            (r[16]=="UNKNOWN" && (r[21]!="1" || r[22]!="1" ||
                r[23]!="0" || r[19].is_empty())) ||
            (matches!(r[16].as_str(),"INTENT"|"WRITTEN") &&
                !(r[21]=="1" && r[22]=="1" && r[23]=="1")) {
            return Err(ManagedCliError::Busy)
        }
        let c=Statement::prepare(db.as_ptr(),
            "SELECT ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation FROM main.gogoke_coordination_process_custody WHERE operation_id=?1")?;
        c.bind_text(1,&r[5])?;
        if !c.step_row()? {return Err(ManagedCliError::Busy)}
        let values=(0..9).map(|n|c.column_text(n))
            .collect::<std::result::Result<Vec<_>,_>>()?;
        if c.step_row()? || values!=vec![r[6].clone(),r[7].clone(),r[8].clone(),
            r[9].clone(),r[10].clone(),r[11].clone(),r[12].clone(),
            r[1].clone(),r[13].clone()] {return Err(ManagedCliError::Busy)}
        let mut source=vec!["H_RPC_STEP-v1".to_owned()];
        source.extend(r.iter().cloned());
        if r[16]=="OBSERVED" {
            let ledger=Statement::prepare(db.as_ptr(),
                "SELECT lower(hex(raw_bytes)),state,COALESCE(no_event_reason,''),
                 no_event_reason IS NULL FROM main.v37_ledger_raw_source WHERE
                 operation_id=?1 AND process_ticket=?2 AND custodian_nonce=?3 AND
                 domain_id=?4 AND session_id=?5 AND generation=?6 AND
                 source_epoch=?7 AND source_cursor=?8")?;
            for (n,value) in [&r[5],&r[6],&r[7],&r[1],&r[2],&r[13],&r[17],&r[18]]
                .iter().enumerate(){ledger.bind_text(n as i32+1,value)?;}
            if !ledger.step_row()? {return Err(ManagedCliError::Busy)}
            let ledger_row=(0..4).map(|n|ledger.column_text(n))
                .collect::<std::result::Result<Vec<_>,_>>()?;
            if ledger.step_row()? || decode_original_hex(&ledger_row[0])?.is_empty() {
                return Err(ManagedCliError::Busy)
            }
            source.extend(ledger_row);
        } else {source.extend([String::new(),String::new(),String::new(),"1".into()]);}
        let proof=vec!["ORIGINAL_RPC_HISTORY".to_owned(),r[0].clone(),
            r[1].clone(),r[2].clone(),r[13].clone(),r[5].clone(),
            r[8].clone(),r[9].clone(),r[4].clone()];
        result.push((source,proof));
    }
    result.sort_by(|a,b|(&a.1[1],&a.1[2],&a.1[3],&a.1[4],&a.1[5],&a.1[8])
        .cmp(&(&b.1[1],&b.1[2],&b.1[3],&b.1[4],&b.1[5],&b.1[8])));
    if result.windows(2).any(|p|p[0].1==p[1].1){return Err(ManagedCliError::Busy)}
    Ok(result)
}

/// Start at C's complete original set, before any instance/profile JOIN. A
/// row executing this driver's official managed-stage image cannot disappear
/// merely because its profile no longer resolves to an F instance.
fn no_unattributed_managed_image_c(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,
    driver:&str,units:&[(Unit,Vec<String>)],stopped:&[Unit],
    globals:&[(Vec<String>,Vec<String>)],strict:bool)->Result<(),ManagedCliError>{
    let all=Statement::prepare(db.as_ptr(),
        "SELECT operation_id,profile_id,domain_id,generation,image_path,binary_digest_sha256 FROM main.gogoke_coordination_process_custody ORDER BY operation_id")?;
    let staging=root.canonical_root().canonical_path.join("v37-managed-cli").join("staging");
    while all.step_row()? {
        let r=(0..6).map(|n|all.column_text(n))
            .collect::<std::result::Result<Vec<_>,_>>()?;
        let pin=match original_catalog_pin(driver,&r[5]) {
            Ok(pin)=>pin,Err(ManagedCliError::Busy)=>continue,Err(error)=>return Err(error),
        };
        let image=std::fs::canonicalize(std::path::Path::new(&r[4]))?;
        let staging=match std::fs::canonicalize(&staging) {
            Ok(value)=>value,
            Err(error)=>{
                if image.starts_with(&root.canonical_root().canonical_path.join("v37-managed-cli")) {
                    return Err(ManagedCliError::Observation(format!(
                        "managed staging root for original C: {error:?}")))
                }
                continue
            },
        };
        if !image.starts_with(&staging) {continue}
        let stage=stage_name_from_original_image(&staging,&image,pin)?;
        let verified=inspect_staged_official_cli_version(root,driver,pin.version,&stage)?;
        if std::fs::canonicalize(&verified)?!=image {
            return Err(ManagedCliError::IdentityChanged)
        }
        let h=units.iter().any(|(u,_)|u.process==r[0] && u.instance==r[1] &&
            u.domain==r[2] && u.generation==r[3]) ||
            stopped.iter().any(|u|u.process==r[0] && u.instance==r[1] &&
                u.domain==r[2] && u.generation==r[3]);
        let global=globals.iter().any(|(source,_)|source[1]==r[0] &&
            source[8]==r[1] && source[9]==r[2] && source[10]==r[3]);
        let managed=r[1]==format!("managed-cli-{driver}") && r[2]=="global";
        let known_strict=if strict {
            let q=Statement::prepare(db.as_ptr(),
                "SELECT driver_id FROM main.gogoke_v37_instances WHERE instance_id=?1")?;
            q.bind_text(1,&r[1])?;
            if q.step_row()? {
                let same=q.column_text(0)?==driver;
                if q.step_row()? {return Err(ManagedCliError::Busy)}
                same
            } else {false}
        } else {false};
        if !h && !global && !managed && !known_strict {return Err(ManagedCliError::Observation(
            "original C managed image has no authenticated family producer".into()))}
    }
    Ok(())
}

fn all_blockers_are_family(db:&VerifiedDatabaseConnection<'_>,driver:&str,
    units:&[(Unit,Vec<String>)],stopped:&[Unit],global_operations:&[String])
    -> Result<(),ManagedCliError> {
    let member=|expected:&dyn Fn(&Unit)->bool| {
        units.iter().any(|(unit,_)|expected(unit)) || stopped.iter().any(expected)
    };
    let mut blocked=false;
    collect_blocker(&mut blocked,no_unowned_original_h_rows(db))?;
    collect_blocker(&mut blocked,independent_f_targets_clear(db,driver))?;
    // These reverse checks run before predicates that join through a claim.
    // A missing claim, generation or producer must not make an H row vanish.
    for sql in [
        "SELECT 1 FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE i.driver_id=?1 AND e.phase='FAILED' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_generation g WHERE EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE i.driver_id=?1 AND e.domain_id=g.domain_id AND e.session_id=g.session_id) AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e WHERE e.domain_id=g.domain_id AND e.session_id=g.session_id AND e.generation=g.generation AND e.request_id=g.request_id AND e.process_operation_id=g.process_operation_id) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_operation o WHERE o.status!='APPLIED' AND EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE i.driver_id=?1 AND e.domain_id=o.domain_id AND e.session_id=o.session_id) AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e WHERE e.domain_id=o.domain_id AND e.session_id=o.session_id AND ((o.operation='open' AND e.request_id=o.request_id AND e.raw_hex=o.raw_hex) OR (o.operation='stop' AND e.stop_request_id=o.request_id))) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_stdin_journal j WHERE EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE i.driver_id=?1 AND e.domain_id=j.domain_id AND e.session_id=j.session_id) AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e WHERE e.domain_id=j.domain_id AND e.session_id=j.session_id AND e.generation=j.generation AND e.process_operation_id=j.process_operation_id) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_generation_change g WHERE EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE i.driver_id=?1 AND e.domain_id=g.domain_id AND e.session_id=g.session_id) AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e WHERE e.domain_id=g.domain_id AND e.session_id=g.session_id AND e.process_operation_id=g.old_process_operation_id) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_instance_operations o JOIN main.gogoke_v37_instance_homes h ON h.home_id=o.target_id JOIN main.gogoke_v37_instances i ON i.instance_id=h.instance_id WHERE i.driver_id=?1 AND o.phase!='APPLIED' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_instance_operations o JOIN main.gogoke_v37_instance_histories h ON h.history_id=o.target_id JOIN main.gogoke_v37_instances i ON i.instance_id=h.instance_id WHERE i.driver_id=?1 AND o.phase!='APPLIED' LIMIT 1",
    ] {collect_blocker(&mut blocked,no_rows(db,sql,driver))?;}
    let instances=rows(db,"SELECT instance_id FROM main.gogoke_v37_instances WHERE driver_id=?1 ORDER BY instance_id",driver,1)?;
    for instance in instances {
        let target=format!("credential-instance-{}",sha256_hex(instance[0].as_bytes()));
        collect_blocker(&mut blocked,no_rows(db,
            "SELECT 1 FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase!='APPLIED' LIMIT 1",
            &target))?;
    }
    collect_blocker(&mut blocked,no_rows(db,"SELECT 1 FROM main.gogoke_coordination_process_custody WHERE profile_id=?1 AND (state!='STOPPED' OR stop_proof_hash IS NULL OR stop_proof_hash='') LIMIT 1",
        &format!("managed-cli-{driver}")))?;
    collect_blocker(&mut blocked,one_matching(db,"SELECT c.instance_id,c.domain_id,c.session_id,c.generation,c.binding_id,c.home_id,COALESCE(c.process_operation_id,'') FROM main.gogoke_v37_h_claim c JOIN main.gogoke_v37_instances i ON i.instance_id=c.instance_id LEFT JOIN main.gogoke_coordination_process_custody p ON p.operation_id=c.process_operation_id AND p.domain_id=c.domain_id AND p.generation=c.generation WHERE i.driver_id=?1 AND (c.state NOT IN ('RELEASED','STOPPED') OR (c.state='STOPPED' AND (c.stop_fact_id IS NULL OR p.state IS NULL OR p.state!='STOPPED' OR p.stop_proof_hash IS NULL OR c.stop_fact_id!=p.stop_proof_hash)))",driver,7,|r|
        member(&|u|r[0]==u.instance&&r[1]==u.domain&&r[2]==u.session&&r[3]==u.generation&&r[4]==u.binding&&r[5]==u.home&&r[6]==u.process)))?;
    collect_blocker(&mut blocked,one_matching(db,"SELECT b.binding_id,b.instance_id,b.domain_id,b.generation,b.owner_id FROM main.gogoke_v37_h_owner_binding b JOIN main.gogoke_v37_instances i ON i.instance_id=b.instance_id LEFT JOIN main.gogoke_v37_h_claim c ON c.binding_id=b.binding_id LEFT JOIN main.gogoke_coordination_process_custody p ON p.operation_id=c.process_operation_id AND p.domain_id=c.domain_id AND p.generation=c.generation WHERE i.driver_id=?1 AND b.state='ACTIVE' AND (c.binding_id IS NULL OR c.state NOT IN ('STOPPED','RELEASED') OR (c.state='STOPPED' AND (c.stop_fact_id IS NULL OR p.state IS NULL OR p.state!='STOPPED' OR p.stop_proof_hash IS NULL OR c.stop_fact_id!=p.stop_proof_hash)))",driver,5,|r|
        member(&|u|r[0]==u.binding&&r[1]==u.instance&&r[2]==u.domain&&r[3]==u.generation&&r[4]==u.session)))?;
    collect_blocker(&mut blocked,one_matching(db,"SELECT e.instance_id,e.domain_id,e.session_id,e.generation,e.request_id,COALESCE(e.process_operation_id,''),e.home_id,e.binding_id FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id LEFT JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id WHERE i.driver_id=?1 AND (e.phase NOT IN ('STOPPED','FAILED') OR (e.process_operation_id IS NOT NULL AND (e.stop_fact_id IS NULL OR c.state IS NULL OR c.state!='STOPPED' OR c.stop_proof_hash IS NULL OR e.stop_fact_id!=c.stop_proof_hash)))",driver,8,|r|
        member(&|u|r[0]==u.instance&&r[1]==u.domain&&r[2]==u.session&&r[3]==u.generation&&r[4]==u.open_request&&r[5]==u.process&&r[6]==u.home&&r[7]==u.binding)))?;
    collect_blocker(&mut blocked,one_matching(db,"SELECT h.home_id,h.instance_id,h.domain_id,h.generation,h.owner_id,h.state FROM main.gogoke_v37_instance_homes h JOIN main.gogoke_v37_instances i ON i.instance_id=h.instance_id WHERE i.driver_id=?1 AND h.state NOT IN ('CLEANED','CLOSED')",driver,6,|r|
        r[5]=="ACTIVE" && member(&|u|r[0]==u.home&&r[1]==u.instance&&r[2]==u.domain&&r[3]==u.generation&&r[4]==u.session)))?;
    collect_blocker(&mut blocked,one_matching(db,"SELECT c.operation_id,c.profile_id,c.domain_id,c.generation FROM main.gogoke_coordination_process_custody c JOIN main.gogoke_v37_instances i ON i.instance_id=c.profile_id WHERE i.driver_id=?1 AND (c.state!='STOPPED' OR c.stop_proof_hash IS NULL OR c.stop_proof_hash='')",driver,4,|r|
        member(&|u|r[0]==u.process&&r[1]==u.instance&&r[2]==u.domain&&r[3]==u.generation)
            || global_operations.iter().any(|operation|operation==&r[0])))?;
    collect_blocker(&mut blocked,one_matching(db,"SELECT o.domain_id,o.session_id,o.request_id FROM main.gogoke_v37_h_operation o WHERE o.status!='APPLIED' AND (EXISTS(SELECT 1 FROM main.gogoke_v37_h_claim c JOIN main.gogoke_v37_instances i ON i.instance_id=c.instance_id WHERE i.driver_id=?1 AND c.domain_id=o.domain_id AND c.session_id=o.session_id) OR EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE i.driver_id=?1 AND e.domain_id=o.domain_id AND e.session_id=o.session_id))",driver,3,|r|
        member(&|u|r[0]==u.domain&&r[1]==u.session&&r[2]==u.open_request)))?;
    collect_blocker(&mut blocked,one_matching(db,"SELECT a.domain_id,a.session_id,a.generation,a.request_id,a.process_operation_id FROM main.gogoke_v37_h_dos_alias a JOIN main.gogoke_v37_instances i ON i.instance_id=a.instance_id WHERE i.driver_id=?1 AND (a.phase IN ('INTENT','PREPARE_STARTED','UNKNOWN') OR a.cleanup_status IN ('NOT_REQUESTED','UNKNOWN'))",driver,5,|r|
        member(&|u|r[0]==u.domain && r[1]==u.session &&
        r[2]==u.generation && r[3]==u.open_request && r[4]==u.process)))?;
    for sql in [
        "SELECT 1 FROM main.gogoke_v37_h_generation_change g JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=g.old_process_operation_id JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE i.driver_id=?1 AND g.stage NOT IN ('APPLIED','CANCELLED') LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_stdin_journal j JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=j.domain_id AND e.session_id=j.session_id AND e.generation=j.generation AND e.process_operation_id=j.process_operation_id JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE i.driver_id=?1 AND j.phase NOT IN ('RECEIPTED','PREPARED','UNKNOWN') LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_instance_operations o JOIN main.gogoke_v37_instances i ON i.instance_id=o.target_id WHERE i.driver_id=?1 AND o.phase!='APPLIED' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_credential_objects o JOIN main.gogoke_v37_instances i ON i.instance_id=o.instance_id WHERE i.driver_id=?1 AND o.phase!='ACTIVE' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_credential_aliases a JOIN main.gogoke_v37_instances i ON i.instance_id=a.instance_id WHERE i.driver_id=?1 AND a.state IN ('PREPARING','REMOVE_PENDING','UNKNOWN') LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_credential_profiles p JOIN main.gogoke_v37_instances i ON i.instance_id=p.instance_id WHERE i.driver_id=?1 AND p.state IN ('GRANT_PENDING','REVOKE_PENDING','UNKNOWN') LIMIT 1",
    ] { collect_blocker(&mut blocked,no_rows(db,sql,driver))?; }
    if blocked {Err(ManagedCliError::Busy)} else {Ok(())}
}

fn cold(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str,
    action:Option<&ActionRecord>,live_probe:Option<&NativeStopProof>)
    ->Result<QualifiedVersionAction,ManagedCliError>{
    let stopped=original_stopped_sources(db,root,driver)?;
    let mut original=original_units(db,root,driver)?;
    let (resumes,before_promotion)=original_resume_units(db,root,driver,&stopped)?;
    original.extend(resumes);
    original.sort_by(|a,b|(&a.0.instance,&a.0.domain,&a.0.session,&a.0.generation,&a.0.process)
        .cmp(&(&b.0.instance,&b.0.domain,&b.0.session,&b.0.generation,&b.0.process)));
    if original.windows(2).any(|p|p[0].0.instance==p[1].0.instance &&
        p[0].0.domain==p[1].0.domain && p[0].0.session==p[1].0.session &&
        p[0].0.generation==p[1].0.generation && p[0].0.process==p[1].0.process){
        return Err(ManagedCliError::Busy)
    }
    let stopped_units=stopped.iter().map(|(source,_)|Ok(Unit{
        instance:source[1].clone(),domain:source[2].clone(),session:source[3].clone(),
        generation:source[4].clone(),open_request:source[5].clone(),
        process:source[6].clone(),home:source[7].clone(),binding:source[8].clone(),
        pairs:vec![(source[23].parse::<u32>().map_err(|_|ManagedCliError::Busy)?,
            source[24].parse::<u64>().map_err(|_|ManagedCliError::Busy)?)],
    })).collect::<Result<Vec<_>,ManagedCliError>>()?;
    let all_stdin=original_stdin_sources(db,driver,&original,&stopped_units)?;
    let rpc=original_rpc_sources(db,driver,&original,&stopped_units)?;
    let (global,global_operations,global_native)=original_global_sources(db,root,driver)?;
    let probes=original_probe_sources(db,root,driver,action,live_probe)?;
    no_unattributed_managed_image_c(db,root,driver,&original,&stopped_units,&global,false)?;
    all_blockers_are_family(db,driver,&original,&stopped_units,&global_operations)?;
    let mut source_parts=vec!["managed-cli-family-source-v1".to_owned(),driver.to_owned()];
    let mut pair_parts=vec!["managed-cli-family-proof-v1".to_owned(),driver.to_owned()];
    let mut units=Vec::with_capacity(original.len());
    let mut members=Vec::new();
    for (unit,mut source) in original {
        let (home_source,h_source)=if let Some(source)=before_promotion.get(&unit.process) {
            (String::new(),source.clone())
        } else {
            let operation=if source[0]=="H_SESSION_OPEN-v1"{"open"}else{"resume"};
            let expected_identity=if operation=="open"{&source[29]}else{&source[36]};
            family_execution_source(db,root,&unit,operation,expected_identity)?
        };
        let fact=NativeLegacyHoldersGone::observe(&unit.pairs).map_err(|error|
            ManagedCliError::NativeGone(format!("original family kernel cutoff: {error:?}")))?;
        fact.validate(&unit.pairs).map_err(|error|
            ManagedCliError::NativeGone(format!("original family kernel recheck: {error:?}")))?;
        if source[0]=="H_SESSION_OPEN-v1" {source.push(home_source)}
        else if !home_source.is_empty() && source[42]!=home_source {
            return Err(ManagedCliError::Busy)
        }
        source.push(h_source);
        let mut proof=vec!["KERNEL_REBOOT".to_owned(),unit.instance.clone(),
            unit.domain.clone(),unit.session.clone(),unit.generation.clone(),
            unit.process.clone()];
        proof.extend(unit.pairs.iter().flat_map(|(pid,time)|
            [pid.to_string(),time.to_string()]));
        let key=(unit.instance.clone(),unit.domain.clone(),unit.session.clone(),
            unit.generation.clone(),unit.process.clone(),String::new());
        members.push((key,source,proof));
        units.push(QualifiedUnit{source:unit,absence:NativeVersionAbsence::Reboot(fact)});
    }
    for (source,proof) in stopped {
        let key=(proof[1].clone(),proof[2].clone(),proof[3].clone(),
            proof[4].clone(),proof[5].clone(),String::new());
        members.push((key,source,proof));
    }
    for (source,proof) in global {
        let key=(proof[1].clone(),proof[2].clone(),proof[3].clone(),
            proof[4].clone(),proof[5].clone(),String::new());
        members.push((key,source,proof));
    }
    for (source,proof) in probes {
        let key=(proof[1].clone(),proof[2].clone(),proof[3].clone(),
            proof[4].clone(),proof[5].clone(),String::new());
        members.push((key,source,proof));
    }
    let mut stdin=Vec::new();
    for (member,source,proof) in all_stdin {
        let key=(member.instance.clone(),member.domain.clone(),member.session.clone(),
            member.generation.clone(),member.process.clone(),format!("stdin:{}",member.request));
        members.push((key,source,proof));
        stdin.push(member);
    }
    for (source,proof) in rpc {
        let key=(proof[1].clone(),proof[2].clone(),proof[3].clone(),proof[4].clone(),
            proof[5].clone(),format!("rpc:{}",proof[8]));
        members.push((key,source,proof));
    }
    if members.is_empty() {
        no_unsettled_managed_version_change(db,driver)?;
        return Ok(QualifiedVersionAction{source_hex:String::new(),source_digest:String::new(),
            pairs_hex:String::new(),pairs_digest:String::new(),units:Vec::new(),stdin:Vec::new(),
            global_operations:Vec::new(),global_native:Vec::new()});
    }
    members.sort_by(|a,b|a.0.cmp(&b.0));
    if members.windows(2).any(|pair|pair[0].0==pair[1].0) {
        return Err(ManagedCliError::Busy)
    }
    for (_,source,proof) in members {
        source_parts.push(hex(&frame(&source)));
        pair_parts.push(hex(&frame(&proof)));
    }
    let source_bytes=frame(&source_parts);
    let pair_bytes=frame(&pair_parts);
    Ok(QualifiedVersionAction{source_hex:hex(&source_bytes),source_digest:content_hash(&source_bytes),
        pairs_hex:hex(&pair_bytes),pairs_digest:content_hash(&pair_bytes),
        units,stdin,global_operations,global_native})
}

fn strict_own_probe(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str,
    action:&ActionRecord,live:Option<&NativeStopProof>)->Result<(),ManagedCliError>{
    let prefix=format!("managed-probe-{driver}-{}-",
        &sha256_hex(action.request_id.as_bytes())[..40]);
    let q=Statement::prepare(db.as_ptr(),
        "SELECT operation_id,ticket,custodian_nonce,pid,creation_time_100ns,
         image_path,binary_digest_sha256,profile_id,domain_id,generation,state,
         COALESCE(stop_proof_hash,''),stop_proof_hash IS NULL
         FROM main.gogoke_coordination_process_custody WHERE operation_id GLOB ?1")?;
    q.bind_text(1,&format!("{prefix}*"))?;
    let mut rows=Vec::new();
    while q.step_row()? {
        rows.push((0..13).map(|n|q.column_text(n))
            .collect::<std::result::Result<Vec<_>,_>>()?);
    }
    let expects=matches!(action.phase,managed_version_action::ActionPhase::Probed |
        managed_version_action::ActionPhase::Consumed) ||
        action.phase==managed_version_action::ActionPhase::Probing && live.is_some();
    if rows.len()!=if expects {1}else{0} {return Err(ManagedCliError::Busy)}
    if !expects {return Ok(())}
    let row=&rows[0];
    let attempt=row[0].strip_prefix(&prefix).ok_or(ManagedCliError::Busy)?;
    let number=attempt.parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
    let pid=row[3].parse::<u32>().map_err(|_|ManagedCliError::Busy)?;
    let created=row[4].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
    let generation=row[9].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
    if number==0 || attempt!=number.to_string() || pid==0 || created==0 ||
        generation==0 || row[3]!=pid.to_string() || row[4]!=created.to_string() ||
        row[9]!=generation.to_string() || row[1].is_empty() || row[2].is_empty() ||
        row[7]!=format!("managed-cli-{driver}") || row[8]!="global" ||
        row[10]!="STOPPED" || row[12]!="0" ||
        !row[11].starts_with("sha256:") || row[11].len()!=71 ||
        row[6]!=format!("sha256:{}",action.target_image_sha256) {
        return Err(ManagedCliError::Busy)
    }
    let pin=read_official_cli_version(driver,&action.target_version)
        .ok_or(ManagedCliError::Busy)?;
    if pin.image_sha256!=action.target_image_sha256 {return Err(ManagedCliError::Busy)}
    let staged=inspect_staged_official_cli_version(root,driver,pin.version,
        &action.stage_name)?;
    if std::fs::canonicalize(&staged)?!=
        std::fs::canonicalize(std::path::Path::new(&row[5]))? {
        return Err(ManagedCliError::IdentityChanged)
    }
    if let Some(proof)=live {
        if action.phase!=managed_version_action::ActionPhase::Probing ||
            !proof.parent_exited || !proof.writer_fence_verified ||
            proof.active_job_processes!=Some(0) || !proof.errors.is_empty() ||
            proof.exit_code!=Some(0) || proof.deadline_exceeded ||
            proof.ticket.opaque()!=row[1] || proof.custodian_nonce!=row[2] ||
            proof.identity.pid!=pid || proof.identity.creation_time_100ns!=created ||
            proof.identity.image_path.to_string_lossy().as_ref()!=row[5].as_str() ||
            proof.binding.profile_id!=row[7] || proof.binding.domain_id!=row[8] ||
            proof.binding.generation!=row[9] ||
            proof.binding.binary_digest_sha256!=row[6] ||
            proof.proof_hash()!=row[11] {return Err(ManagedCliError::Busy)}
    } else {
        let witness=managed_version_action::bound_probe_witness(action)
            .map_err(|error|ManagedCliError::Observation(format!(
                "strict original own probe witness: {error:?}")))?;
        if witness.operation_id!=row[0] || witness.ticket!=row[1] ||
            witness.nonce!=row[2] || witness.pid!=row[3] ||
            witness.creation!=row[4] || witness.image!=row[5] ||
            witness.digest!=row[6] || witness.profile!=row[7] ||
            witness.domain!=row[8] || witness.generation!=row[9] ||
            witness.proof_hash!=row[11] {return Err(ManagedCliError::Busy)}
    }
    Ok(())
}

fn strict_scope(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str,
    action:Option<&ActionRecord>,live:Option<&NativeStopProof>)
    ->Result<QualifiedVersionAction,ManagedCliError>{
    no_unsettled_managed_version_change(db,driver)?;
    no_unattributed_managed_image_c(db,root,driver,&[],&[],&[],true)?;
    if let Some(action)=action {strict_own_probe(db,root,driver,action,live)?}
    Ok(QualifiedVersionAction{source_hex:String::new(),source_digest:String::new(),
        pairs_hex:String::new(),pairs_digest:String::new(),units:Vec::new(),stdin:Vec::new(),
        global_operations:Vec::new(),global_native:Vec::new()})
}

pub(super) fn require_saved_action(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,
    action:&ActionRecord)->Result<(),ManagedCliError>{
    let saved=managed_version_action::read(db,root,&action.request_id)
        .map_err(|error|ManagedCliError::Observation(format!("version action readback: {error:?}")))?
        .ok_or(ManagedCliError::Busy)?;
    if saved==*action {Ok(())} else {Err(ManagedCliError::Busy)}
}

/// Call at first USER begin inside its verified Owner transaction, before
/// storing ActionBegin. A strictly clear driver remains strict.
pub(crate) fn observe_version_scope(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,
    driver:&str)->Result<QualifiedVersionAction,ManagedCliError>{
    match strict_scope(db,root,driver,None,None) {
        Ok(view)=>Ok(view),Err(ManagedCliError::Busy)=>cold(db,root,driver,None,None),
        Err(error)=>Err(error),
    }
}

/// Exercise the existing cold collector against original writer fixtures
/// even when the production entry correctly chooses its STRICT fast path.
#[cfg(test)]
pub(crate) fn test_collect_cold_version_scope(db:&VerifiedDatabaseConnection<'_>,
    root:&RootLock,driver:&str)->Result<QualifiedVersionAction,ManagedCliError>{
    cold(db,root,driver,None,None)
}

/// Rebuild from current producer rows. The caller separately checks the live
/// copy/source fingerprints against the action's legitimate revision chain.
pub(crate) fn revalidate_version_action(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,
    driver:&str,action:&ActionRecord)->Result<QualifiedVersionAction,ManagedCliError>{
    revalidate_version_action_at_fingerprints(db,root,driver,action,
        &action.current_copy_fingerprint,&action.current_source_fingerprint)
}

/// The expected pair is the journal's next CAS pair after the caller has
/// applied its own copy/source write in the same immediate transaction. This
/// does not authorize an arbitrary external drift: the caller must derive the
/// pair from that exact write and advance the action row before commit.
pub(crate) fn revalidate_version_action_at_fingerprints(
    db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str,
    action:&ActionRecord,expected_copy:&str,expected_source:&str,
)->Result<QualifiedVersionAction,ManagedCliError>{
    revalidate_version_action_at_fingerprints_inner(db,root,driver,action,
        expected_copy,expected_source,None)
}

pub(crate) fn revalidate_version_action_with_probe(
    db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str,
    action:&ActionRecord,proof:&NativeStopProof,
)->Result<QualifiedVersionAction,ManagedCliError>{
    revalidate_version_action_at_fingerprints_inner(db,root,driver,action,
        &action.current_copy_fingerprint,&action.current_source_fingerprint,Some(proof))
}

pub(crate) fn revalidate_version_action_at_fingerprints_with_probe(
    db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str,
    action:&ActionRecord,expected_copy:&str,expected_source:&str,proof:&NativeStopProof,
)->Result<QualifiedVersionAction,ManagedCliError>{
    revalidate_version_action_at_fingerprints_inner(db,root,driver,action,
        expected_copy,expected_source,Some(proof))
}

fn revalidate_version_action_at_fingerprints_inner(
    db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str,
    action:&ActionRecord,expected_copy:&str,expected_source:&str,
    live_probe:Option<&NativeStopProof>,
)->Result<QualifiedVersionAction,ManagedCliError>{
    if action.driver!=driver || action.root_identity!=root.canonical_root().identity.opaque()
        || action.database_identity!=db.identity().opaque() {
        return Err(ManagedCliError::IdentityChanged);
    }
    require_saved_action(db,root,action)?;
    let observed=program_source::managed_version_fingerprints(db,root,driver)
        .map_err(|error|ManagedCliError::Observation(format!("version copy/source fingerprint: {error:?}")))?;
    if observed.0!=expected_copy || observed.1!=expected_source {
        return Err(ManagedCliError::Busy);
    }
    let view=match action.mode.as_str(){
        "STRICT"=>strict_scope(db,root,driver,Some(action),live_probe)?,
        "COLD" if action.proof_family=="family-v1"=>
            cold(db,root,driver,Some(action),live_probe)?,
        _=>return Err(ManagedCliError::Busy),
    };
    if view.proof_family()!=action.proof_family || view.source_hex!=action.source_hex ||
        view.source_digest!=action.source_digest || view.pairs_hex!=action.pairs_hex ||
        view.pairs_digest!=action.pairs_digest {return Err(ManagedCliError::Busy);}
    view.recheck_native()?;
    Ok(view)
}

/// Action-aware alternative; never changes the unbound/first-copy strict API.
pub(crate) fn no_unsettled_managed_version_change_for_action(
    db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str,
    action:&ActionRecord)->Result<QualifiedVersionAction,ManagedCliError>{
    revalidate_version_action(db,root,driver,action)
}

pub(crate) fn no_unsettled_managed_version_change_for_action_with_probe(
    db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str,
    action:&ActionRecord,proof:&NativeStopProof,
)->Result<QualifiedVersionAction,ManagedCliError>{
    revalidate_version_action_with_probe(db,root,driver,action,proof)
}

#[cfg(test)]
mod source_image_selection_tests {
    use super::*;
    use std::time::{SystemTime,UNIX_EPOCH};

    #[test]
    fn original_open_equations_keep_pending_revision_and_bind_project_payload() {
        let global=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"open","requestId":"openA","targetId":"sessionA","domainId":"global","expectedRevision":"2","payload":{}}"#;
        let request=crate::store::session_transport::decode_request(global).unwrap();
        let mut source=vec![String::new();40];
        for (index,value) in [(2,"global"),(4,"1"),(11,"2"),(12,"2"),
            (30,"UNKNOWN"),(32,"2"),(33,"2"),(38,"seatA")] {
            source[index]=value.into();
        }
        assert!(original_open_equations(&request,&source),
            "the original pending intention has not advanced its revision");
        for (index,value) in [(11,"1"),(12,"3"),(32,"1"),(33,"3")] {
            let mut changed=source.clone();changed[index]=value.into();
            assert!(!original_open_equations(&request,&changed),
                "original OPEN producer revision column {index} must bind");
        }
        source[30]="APPLIED".into();
        assert!(!original_open_equations(&request,&source));
        source[12]="3".into();source[33]="3".into();
        assert!(original_open_equations(&request,&source));
        let project=r#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"open","requestId":"openA","targetId":"sessionA","domainId":"projectA","expectedRevision":"2","payload":{"seatId":"seatA","generation":"1","repositoryId":"repoA","worktreeId":"treeA"}}"#;
        source[2]="projectA".into();
        let decode=|raw:&str|crate::store::session_transport::decode_request(raw.as_bytes()).unwrap();
        assert!(original_open_equations(&decode(project),&source));
        for raw in [project.replace("\"generation\":\"1\"","\"generation\":\"2\""),
            project.replace("\"seatId\":\"seatA\"","\"seatId\":\"seatB\""),
            project.replace("\"expectedRevision\":\"2\"","\"expectedRevision\":\"3\""),
            project.replace("\"worktreeId\":\"treeA\"","\"worktreeId\":\"treeA\",\"extra\":\"x\"")] {
            assert!(!original_open_equations(&decode(&raw),&source));
        }
    }

    #[test]
    fn old_image_stage_requires_canonical_catalog_path_under_owned_staging() {
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base=std::env::temp_dir().join(format!("gogoke-cold-source-{}-{nonce}",
            std::process::id()));
        let staging=base.join("v37-managed-cli").join("staging");
        let pin=read_official_cli_version("opencode","1.18.32").unwrap();
        let stage="opencode-1.18.32-original";
        let expected=pin.image_relative.split('/').fold(
            staging.join(stage).join("content"),|path,part|path.join(part));
        let wrong=staging.join(stage).join("content").join("other.exe");
        let outside=base.join("outside.exe");
        std::fs::create_dir_all(expected.parent().unwrap()).unwrap();
        std::fs::create_dir_all(wrong.parent().unwrap()).unwrap();
        std::fs::write(&expected,b"path-selection-only").unwrap();
        std::fs::write(&wrong,b"wrong-child").unwrap();
        std::fs::write(&outside,b"outside").unwrap();
        assert_eq!(stage_name_from_original_image(&staging,&expected,pin).unwrap(),stage);
        assert!(matches!(stage_name_from_original_image(&staging,&wrong,pin),
            Err(ManagedCliError::Busy)));
        assert!(matches!(stage_name_from_original_image(&staging,&outside,pin),
            Err(ManagedCliError::Busy)));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rollback_catalog_selection_uses_original_c_digest() {
        let historical=read_official_cli_version("opencode","1.18.32").unwrap();
        let current=read_official_cli_version("opencode","1.18.33").unwrap();
        assert_ne!(historical.image_sha256,current.image_sha256);
        let selected=original_catalog_pin("opencode",
            &format!("sha256:{}",historical.image_sha256)).unwrap();
        assert_eq!(selected.version,historical.version);
        assert!(matches!(original_catalog_pin("opencode","sha256:untrusted"),
            Err(ManagedCliError::Busy)));
    }
}
