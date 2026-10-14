//! A version action may re-evaluate one historical OPEN execution only when
//! its producer rows and every version blocker still identify that execution.
//! The journal stores a snapshot; it is not a native absence observation.

use super::{inspect_staged_official_cli_version, no_unsettled_managed_version_change,
    read_official_cli_version, read_prepared_managed_replacement, ManagedCliError, OFFICIAL};
use super::super::{managed_version_action::{self,ActionRecord}, program_source, registry};
use crate::process::{NativeLegacyHoldersGone,NativeVersionAliasNamespace};
use crate::root::{inspect_root, RootLock};
use crate::store::atomic::Statement;
use crate::store::digest::{content_hash, sha256_hex};
use crate::store::same_open::VerifiedDatabaseConnection;
use std::os::windows::fs::MetadataExt;

/// Kept private to F. The sealed H observation cannot be reconstructed from
/// persisted action bytes or a caller-supplied `gone` value.
enum NativeVersionAbsence { Reboot(NativeLegacyHoldersGone) }

pub(crate) struct QualifiedVersionAction {
    source_hex: String,
    source_digest: String,
    pairs_hex: String,
    pairs_digest: String,
    unit: Option<Unit>,
    native: Option<NativeVersionAbsence>,
    alias_native: Option<NativeVersionAliasNamespace>,
}

impl QualifiedVersionAction {
    pub(crate) fn is_cold(&self) -> bool { self.unit.is_some() }
    pub(crate) fn proof_family(&self) -> &'static str {
        if self.is_cold() { "reboot" } else { "" }
    }
    pub(crate) fn source_hex(&self) -> &str { &self.source_hex }
    pub(crate) fn source_digest(&self) -> &str { &self.source_digest }
    pub(crate) fn pairs_hex(&self) -> &str { &self.pairs_hex }
    pub(crate) fn pairs_digest(&self) -> &str { &self.pairs_digest }
    pub(crate) fn allows_instance(&self, instance: &str) -> bool {
        self.unit.as_ref().is_some_and(|unit| unit.instance == instance)
    }
    pub(crate) fn allows_claim(&self, instance: &str, domain: &str, session: &str,
        generation: &str, binding: &str, process: &str) -> bool {
        self.unit.as_ref().is_some_and(|u| u.instance == instance && u.domain == domain &&
            u.session == session && u.generation == generation && u.binding == binding &&
            u.process == process)
    }
    pub(crate) fn allows_episode(&self, domain: &str, request: &str, process: &str) -> bool {
        self.unit.as_ref().is_some_and(|u| u.domain == domain && u.open_request == request &&
            u.process == process)
    }
    pub(crate) fn allows_operation(&self, domain:&str, session:&str, request:&str)->bool {
        self.unit.as_ref().is_some_and(|u| u.domain==domain && u.session==session &&
            u.open_request==request)
    }
    pub(crate) fn allows_home(&self, home: &str) -> bool {
        self.unit.as_ref().is_some_and(|u| u.home == home)
    }
    pub(crate) fn allows_custody(&self, process: &str) -> bool {
        self.unit.as_ref().is_some_and(|u| u.process == process)
    }
    pub(crate) fn allows_alias(&self, domain: &str, session: &str,
        generation: &str) -> bool {
        self.unit.as_ref().is_some_and(|u| u.alias.is_some() &&
            u.domain==domain && u.session==session && u.generation==generation)
    }
    pub(crate) fn recheck_native(&self) -> Result<(), ManagedCliError> {
        if let (Some(alias),Some(unit))=(&self.alias_native,&self.unit) {
            let row=unit.alias.as_ref().ok_or(ManagedCliError::Busy)?;
            alias.validate(&row.namespace,&row.name,&row.requested_target,
                &row.observed_target,&row.volume_serial,&row.file_id)
                .map_err(|error|ManagedCliError::Observation(format!("version alias: {error}")))?;
        } else if self.alias_native.is_some() {return Err(ManagedCliError::Busy)}
        match (&self.native, &self.unit) {
            (Some(NativeVersionAbsence::Reboot(fact)), Some(unit)) =>
                fact.validate(&unit.pairs).map_err(|error|
                    ManagedCliError::NativeGone(format!("cold version reboot recheck: {error:?}"))),
            (None, None) => Ok(()),
            _ => Err(ManagedCliError::Busy),
        }
    }
}

#[derive(Clone)]
struct Unit {
    instance: String, domain: String, session: String, generation: String,
    binding: String, home: String, process: String, open_request: String,
    pairs: Vec<(u32,u64)>,
    alias: Option<AliasSource>,
}

#[derive(Clone)]
struct AliasSource {
    namespace:String,name:String,requested_target:String,observed_target:String,
    volume_serial:String,file_id:String,
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
fn verified_source_images(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str,
    instance:&str,home:&str,current_digest:&str,current_version:&str,
    original_image:&str,original_digest:&str)->Result<(),ManagedCliError>{
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

    let old=original_catalog_pin(driver,original_digest)?;
    let original=std::path::Path::new(original_image);
    let metadata=std::fs::symlink_metadata(original)?;
    if !metadata.is_file() || metadata.file_attributes()&0x400!=0 {
        return Err(ManagedCliError::IdentityChanged)
    }
    let original_identity=inspect_root(original)
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
            let identity=inspect_root(&verified).map_err(|error|
                ManagedCliError::Observation(format!("retained old stage: {error:?}")))?;
            if identity.identity==original_identity.identity {return Ok(())}
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
    let verified_identity=inspect_root(&verified)
        .map_err(|error|ManagedCliError::Observation(format!("original staged image: {error:?}")))?;
    if original_identity.identity!=verified_identity.identity {
        return Err(ManagedCliError::IdentityChanged)
    }
    Ok(())
}

/// Exact original source is the persisted H OPEN/C process chain. The current
/// F program/copy revisions are intentionally absent: a legitimate stage or
/// final switch changes those and the action journal tracks their CAS chain.
fn original_unit(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    driver: &str) -> Result<(Unit,Vec<String>), ManagedCliError> {
    let root_seen = inspect_root(&root.canonical_root().canonical_path)
        .map_err(|error| ManagedCliError::Observation(format!("cold root: {error:?}")))?;
    if root_seen.identity != root.canonical_root().identity ||
        db.root_identity() != &root_seen.identity { return Err(ManagedCliError::IdentityChanged); }
    let sql = "SELECT i.instance_id,i.home_identity,c.domain_id,e.session_id,e.generation,
        e.request_id,e.process_operation_id,e.binding_id,e.home_id,e.phase,e.raw_hex,
        CAST(e.previous_revision AS TEXT),COALESCE(CAST(e.result_revision AS TEXT),''),
        c.ticket,c.custodian_nonce,c.pid,c.creation_time_100ns,c.image_path,
        c.binary_digest_sha256,c.state,COALESCE(c.stop_proof_hash,''),
        a.state,CAST(a.revision AS TEXT),b.state,b.kind,b.owner_id,
        h.state,h.directory_ref,h.directory_identity,CAST(h.revision AS TEXT),
        o.status,o.raw_hex,CAST(o.previous_revision AS TEXT),CAST(o.revision AS TEXT),
        i.program_digest,i.version
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
    let candidates = rows(db,sql,driver,36)?;
    if candidates.len()!=1 { return Err(ManagedCliError::Busy); }
    let raw = &candidates[0];
    if raw[18].is_empty() || raw[17].is_empty() || raw[13].is_empty() || raw[14].is_empty() ||
        !raw[20].is_empty() || raw[31] != raw[10] { return Err(ManagedCliError::Busy); }
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
    let aliases=rows(db,"SELECT a.domain_id,a.session_id,a.generation,a.request_id,
        a.request_sha256,a.origin_kind,a.process_operation_id,a.instance_id,
        a.binary_digest_sha256,a.binding_id,a.worktree_id,a.namespace_id,
        a.alias_name,a.requested_target,COALESCE(a.observed_target,''),
        a.volume_serial,a.file_id,a.host_pid,a.host_creation_time_100ns,
        a.phase,COALESCE(a.child_operation_id,''),COALESCE(a.child_ticket,''),
        COALESCE(a.child_nonce,''),COALESCE(a.child_pid,''),
        COALESCE(a.child_creation_time_100ns,''),a.cleanup_status,
        COALESCE(a.cleanup_result,''),CAST(a.revision AS TEXT)
        FROM main.gogoke_v37_h_dos_alias a JOIN main.gogoke_v37_instances i
        ON i.instance_id=a.instance_id WHERE i.driver_id=?1",driver,28)?;
    if aliases.len()>1 || (driver=="opencode" && aliases.is_empty()) {
        return Err(ManagedCliError::Busy)
    }
    let mut pairs=vec![(pid,creation)];
    let alias=if let Some(row)=aliases.first(){
        if driver!="opencode" || row[0]!=raw[2] || row[1]!=raw[3] ||
            row[2]!=raw[4] || row[3]!=raw[5] || row[4]!=open_hash ||
            row[5]!="OPEN" || row[6]!=raw[6] || row[7]!=raw[0] ||
            row[8]!=raw[18] || row[9]!=raw[7] || row[19]!="CUSTODY_BOUND" ||
            row[20]!=raw[6] || row[21]!=raw[13] || row[22]!=raw[14] ||
            row[23]!=raw[15] || row[24]!=raw[16] || row[14].is_empty() ||
            row[11].is_empty() || row[10].is_empty() {
            return Err(ManagedCliError::Busy)
        }
        let host_pid=row[17].parse::<u32>().map_err(|_|ManagedCliError::Busy)?;
        let host_creation=row[18].parse::<u64>().map_err(|_|ManagedCliError::Busy)?;
        if host_pid==0 || host_creation==0 || (host_pid,host_creation)==(pid,creation) {
            return Err(ManagedCliError::Busy)
        }
        let worktree=Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_worktrees w JOIN main.gogoke_v37_worktree_lifecycle l ON l.worktree_id=w.worktree_id WHERE w.worktree_id=?1 AND w.instance_id=?2 AND w.domain_id=?3 AND w.worktree_path=?4 AND w.worktree_identity=?5 AND w.state='REGISTERED' AND l.state='REGISTERED' AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_worktree_lifecycle_ops x WHERE x.worktree_id=w.worktree_id AND x.phase!='APPLIED')")?;
        worktree.bind_text(1,&row[10])?;worktree.bind_text(2,&raw[0])?;
        worktree.bind_text(3,&raw[2])?;worktree.bind_text(4,&row[13])?;
        let f=inspect_root(std::path::Path::new(&row[13]))
            .map_err(|error|ManagedCliError::Observation(format!("original F: {error:?}")))?;
        worktree.bind_text(5,&f.identity.opaque())?;
        if !worktree.step_row()? || worktree.step_row()? {return Err(ManagedCliError::Busy)}
        pairs.push((host_pid,host_creation));
        Some(AliasSource {namespace:row[11].clone(),name:row[12].clone(),
            requested_target:row[13].clone(),observed_target:row[14].clone(),
            volume_serial:row[15].clone(),file_id:row[16].clone()})
    } else {None};
    pairs.sort_unstable();
    let unit = Unit {instance:raw[0].clone(), domain:raw[2].clone(), session:raw[3].clone(),
        generation:raw[4].clone(), open_request:raw[5].clone(), process:raw[6].clone(),
        binding:raw[7].clone(), home:raw[8].clone(), pairs,alias};
    // The only omitted column is the live F program digest at index 34.
    let mut source=raw[..34].to_vec();
    source.push(if aliases.is_empty(){"no-alias"}else{"alias"}.into());
    if let Some(row)=aliases.first(){source.extend(row.iter().cloned());}
    Ok((unit,source))
}

fn all_blockers_are_one(db:&VerifiedDatabaseConnection<'_>,driver:&str,u:&Unit)
    -> Result<(),ManagedCliError> {
    let mut blocked=false;
    collect_blocker(&mut blocked,no_rows(db,"SELECT 1 FROM main.gogoke_coordination_process_custody WHERE profile_id=?1 AND (state!='STOPPED' OR stop_proof_hash IS NULL OR stop_proof_hash='') LIMIT 1",
        &format!("managed-cli-{driver}")))?;
    collect_blocker(&mut blocked,one_matching(db,"SELECT c.instance_id,c.domain_id,c.session_id,c.generation,c.binding_id,c.home_id,COALESCE(c.process_operation_id,'') FROM main.gogoke_v37_h_claim c JOIN main.gogoke_v37_instances i ON i.instance_id=c.instance_id LEFT JOIN main.gogoke_coordination_process_custody p ON p.operation_id=c.process_operation_id AND p.domain_id=c.domain_id AND p.generation=c.generation WHERE i.driver_id=?1 AND (c.state NOT IN ('RELEASED','STOPPED') OR (c.state='STOPPED' AND (c.stop_fact_id IS NULL OR p.state IS NULL OR p.state!='STOPPED' OR p.stop_proof_hash IS NULL OR c.stop_fact_id!=p.stop_proof_hash)))",driver,7,|r|
        r[0]==u.instance&&r[1]==u.domain&&r[2]==u.session&&r[3]==u.generation&&r[4]==u.binding&&r[5]==u.home&&r[6]==u.process))?;
    collect_blocker(&mut blocked,one_matching(db,"SELECT b.binding_id,b.instance_id,b.domain_id,b.generation,b.owner_id FROM main.gogoke_v37_h_owner_binding b JOIN main.gogoke_v37_instances i ON i.instance_id=b.instance_id LEFT JOIN main.gogoke_v37_h_claim c ON c.binding_id=b.binding_id LEFT JOIN main.gogoke_coordination_process_custody p ON p.operation_id=c.process_operation_id AND p.domain_id=c.domain_id AND p.generation=c.generation WHERE i.driver_id=?1 AND b.state='ACTIVE' AND (c.binding_id IS NULL OR c.state NOT IN ('STOPPED','RELEASED') OR (c.state='STOPPED' AND (c.stop_fact_id IS NULL OR p.state IS NULL OR p.state!='STOPPED' OR p.stop_proof_hash IS NULL OR c.stop_fact_id!=p.stop_proof_hash)))",driver,5,|r|
        r[0]==u.binding&&r[1]==u.instance&&r[2]==u.domain&&r[3]==u.generation&&r[4]==u.session))?;
    collect_blocker(&mut blocked,one_matching(db,"SELECT e.instance_id,e.domain_id,e.session_id,e.generation,e.request_id,COALESCE(e.process_operation_id,''),e.home_id,e.binding_id FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id LEFT JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id WHERE i.driver_id=?1 AND (e.phase NOT IN ('STOPPED','FAILED') OR (e.process_operation_id IS NOT NULL AND (e.stop_fact_id IS NULL OR c.state IS NULL OR c.state!='STOPPED' OR c.stop_proof_hash IS NULL OR e.stop_fact_id!=c.stop_proof_hash)))",driver,8,|r|
        r[0]==u.instance&&r[1]==u.domain&&r[2]==u.session&&r[3]==u.generation&&r[4]==u.open_request&&r[5]==u.process&&r[6]==u.home&&r[7]==u.binding))?;
    collect_blocker(&mut blocked,one_matching(db,"SELECT h.home_id,h.instance_id,h.domain_id,h.generation,h.owner_id FROM main.gogoke_v37_instance_homes h JOIN main.gogoke_v37_instances i ON i.instance_id=h.instance_id WHERE i.driver_id=?1 AND h.state NOT IN ('CLEANED','CLOSED')",driver,5,|r|
        r[0]==u.home&&r[1]==u.instance&&r[2]==u.domain&&r[3]==u.generation&&r[4]==u.session))?;
    collect_blocker(&mut blocked,one_matching(db,"SELECT c.operation_id,c.profile_id,c.domain_id,c.generation FROM main.gogoke_coordination_process_custody c JOIN main.gogoke_v37_instances i ON i.instance_id=c.profile_id WHERE i.driver_id=?1 AND (c.state!='STOPPED' OR c.stop_proof_hash IS NULL OR c.stop_proof_hash='')",driver,4,|r|
        r[0]==u.process&&r[1]==u.instance&&r[2]==u.domain&&r[3]==u.generation))?;
    collect_blocker(&mut blocked,one_matching(db,"SELECT o.domain_id,o.session_id,o.request_id FROM main.gogoke_v37_h_operation o JOIN main.gogoke_v37_h_claim c ON c.domain_id=o.domain_id AND c.session_id=o.session_id JOIN main.gogoke_v37_instances i ON i.instance_id=c.instance_id WHERE i.driver_id=?1 AND o.status!='APPLIED'",driver,3,|r|
        r[0]==u.domain&&r[1]==u.session&&r[2]==u.open_request))?;
    collect_blocker(&mut blocked,one_matching(db,"SELECT a.domain_id,a.session_id,a.generation,a.request_id,a.process_operation_id FROM main.gogoke_v37_h_dos_alias a JOIN main.gogoke_v37_instances i ON i.instance_id=a.instance_id WHERE i.driver_id=?1 AND (a.phase IN ('INTENT','PREPARE_STARTED','UNKNOWN') OR a.cleanup_status IN ('NOT_REQUESTED','UNKNOWN'))",driver,5,|r|
        u.alias.is_some() && r[0]==u.domain && r[1]==u.session &&
        r[2]==u.generation && r[3]==u.open_request && r[4]==u.process))?;
    for sql in [
        "SELECT 1 FROM main.gogoke_v37_h_generation_change g JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=g.old_process_operation_id JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE i.driver_id=?1 AND g.stage NOT IN ('APPLIED','CANCELLED','UNSUPPORTED') LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_stdin_journal j JOIN main.gogoke_v37_h_claim c ON c.domain_id=j.domain_id AND c.session_id=j.session_id JOIN main.gogoke_v37_instances i ON i.instance_id=c.instance_id WHERE i.driver_id=?1 AND j.phase!='RECEIPTED' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_instance_operations o JOIN main.gogoke_v37_instances i ON i.instance_id=o.target_id WHERE i.driver_id=?1 AND o.phase!='APPLIED' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_credential_objects o JOIN main.gogoke_v37_instances i ON i.instance_id=o.instance_id WHERE i.driver_id=?1 AND o.phase!='ACTIVE' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_credential_aliases a JOIN main.gogoke_v37_instances i ON i.instance_id=a.instance_id WHERE i.driver_id=?1 AND a.state IN ('PREPARING','REMOVE_PENDING','UNKNOWN') LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_credential_profiles p JOIN main.gogoke_v37_instances i ON i.instance_id=p.instance_id WHERE i.driver_id=?1 AND p.state IN ('GRANT_PENDING','REVOKE_PENDING','UNKNOWN') LIMIT 1",
    ] { collect_blocker(&mut blocked,no_rows(db,sql,driver))?; }
    if blocked {Err(ManagedCliError::Busy)} else {Ok(())}
}

fn cold(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,driver:&str)
    ->Result<QualifiedVersionAction,ManagedCliError>{
    let (unit,source)=original_unit(db,root,driver)?;
    all_blockers_are_one(db,driver,&unit)?;
    let source_bytes=frame(&source);
    let pair_parts=unit.pairs.iter().flat_map(|(pid,time)|
        [pid.to_string(),time.to_string()]).collect::<Vec<_>>();
    let pair_bytes=frame(&pair_parts);
    let fact=NativeLegacyHoldersGone::observe(&unit.pairs).map_err(|error|
        ManagedCliError::NativeGone(format!("cold version kernel BootTime: {error:?}")))?;
    fact.validate(&unit.pairs).map_err(|error|
        ManagedCliError::NativeGone(format!("cold version reboot validation: {error:?}")))?;
    let alias_native=if let Some(alias)=unit.alias.as_ref(){
        Some(NativeVersionAliasNamespace::observe(&alias.namespace,&alias.name,
            &alias.requested_target,&alias.observed_target,
            &alias.volume_serial,&alias.file_id)
            .map_err(|error|ManagedCliError::Observation(format!("version alias: {error}")))?)
    } else {None};
    Ok(QualifiedVersionAction{source_hex:hex(&source_bytes),source_digest:content_hash(&source_bytes),
        pairs_hex:hex(&pair_bytes),pairs_digest:content_hash(&pair_bytes),
        unit:Some(unit),native:Some(NativeVersionAbsence::Reboot(fact)),alias_native})
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
    match no_unsettled_managed_version_change(db,driver) {
        Ok(())=>Ok(QualifiedVersionAction{source_hex:String::new(),source_digest:String::new(),
            pairs_hex:String::new(),pairs_digest:String::new(),unit:None,native:None,
            alias_native:None}),
        Err(ManagedCliError::Busy)=>cold(db,root,driver),
        Err(error)=>Err(error),
    }
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
        "STRICT"=>{
            no_unsettled_managed_version_change(db,driver)?;
            QualifiedVersionAction{source_hex:String::new(),source_digest:String::new(),
                pairs_hex:String::new(),pairs_digest:String::new(),unit:None,native:None,
                alias_native:None}
        },
        "COLD" if action.proof_family=="reboot"=>cold(db,root,driver)?,
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

#[cfg(test)]
mod source_image_selection_tests {
    use super::*;
    use std::time::{SystemTime,UNIX_EPOCH};

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
