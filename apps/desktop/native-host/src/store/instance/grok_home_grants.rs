//! Private F journal for one pinned Grok instance HOME and its original
//! per-generation SID. No credential contents, path supplied by wire, or CLI
//! command is stored here. An ACL effect is never made before its intent.
use crate::root::RootIdentity;
use crate::process::AppContainerProfile;
use crate::store::atomic::Statement;
use crate::store::digest::sha256_hex;
use crate::store::same_open::VerifiedDatabaseConnection;

const SCHEMA: [(&str, &str); 3] = [
    ("gogoke_v37_grok_home_domains", "CREATE TABLE gogoke_v37_grok_home_domains(instance_id TEXT PRIMARY KEY REFERENCES gogoke_v37_instances(instance_id),root_identity TEXT NOT NULL,home_identity TEXT NOT NULL,program_digest TEXT NOT NULL,version TEXT NOT NULL,registration_revision INTEGER NOT NULL CHECK(registration_revision>=1),revision INTEGER NOT NULL CHECK(revision>=1)) STRICT"),
    ("gogoke_v37_grok_home_grants", "CREATE TABLE gogoke_v37_grok_home_grants(binding_id TEXT PRIMARY KEY,instance_id TEXT NOT NULL REFERENCES gogoke_v37_grok_home_domains(instance_id),domain_id TEXT NOT NULL,session_id TEXT NOT NULL,seat_id TEXT NOT NULL,seat_incarnation TEXT NOT NULL,generation TEXT NOT NULL,request_id TEXT NOT NULL UNIQUE,profile_name TEXT NOT NULL,profile_sid TEXT NOT NULL UNIQUE,program_digest TEXT NOT NULL,home_identity TEXT NOT NULL,auth_identity TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('GRANT_PENDING','GRANTED_UNCREATED','ACTIVE','REVOKE_PENDING','RETIRED_CLEANUP_PENDING','REVOKED','UNKNOWN')),process_operation_id TEXT,ticket TEXT,custodian_nonce TEXT,pid INTEGER,creation_time_100ns INTEGER,image_path TEXT,stop_fact_id TEXT,revision INTEGER NOT NULL CHECK(revision>=1)) STRICT"),
    ("gogoke_v37_grok_home_effects", "CREATE TABLE gogoke_v37_grok_home_effects(effect_id TEXT PRIMARY KEY,binding_id TEXT NOT NULL REFERENCES gogoke_v37_grok_home_grants(binding_id),action TEXT NOT NULL CHECK(action IN ('GRANT_ROOT','GRANT_AUTH','REVOKE_ROOT','REVOKE_AUTH','REVOKE_RESIDUE')),object_identity TEXT NOT NULL,relative_name TEXT NOT NULL,rights INTEGER NOT NULL CHECK(rights=1245631),flags INTEGER NOT NULL CHECK(flags IN (0,3)),before_aces TEXT NOT NULL,after_aces TEXT NOT NULL,before_control INTEGER NOT NULL,after_control INTEGER NOT NULL,other_aces_sha256 TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('INTENT','APPLIED','UNKNOWN')),revision INTEGER NOT NULL CHECK(revision>=1)) STRICT"),
];
const ROOT_ANCHOR_SQL: &str = "CREATE TABLE gogoke_v37_grok_home_root_anchor(instance_id TEXT PRIMARY KEY REFERENCES gogoke_v37_grok_home_domains(instance_id),root_identity TEXT NOT NULL,home_identity TEXT NOT NULL,program_digest TEXT NOT NULL,version TEXT NOT NULL,registration_revision INTEGER NOT NULL,baseline_acl_hex TEXT NOT NULL,baseline_control INTEGER NOT NULL,baseline_effect_id TEXT NOT NULL,acl_hex TEXT NOT NULL,acl_control INTEGER NOT NULL,source_effect_id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>=1)) STRICT";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GrokRootAnchor {
    pub(crate) instance_id:String, pub(crate) root_identity:RootIdentity,
    pub(crate) home_identity:RootIdentity, pub(crate) program_digest:String,
    pub(crate) version:String, pub(crate) registration_revision:i64,
    pub(crate) baseline_acl_hex:String, pub(crate) baseline_control:u16,
    pub(crate) baseline_effect_id:String,
    pub(crate) acl_hex:String, pub(crate) acl_control:u16,
    pub(crate) source_effect_id:String, pub(crate) revision:i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GrokDomain {
    pub(crate) instance_id: String, pub(crate) root_identity: RootIdentity,
    pub(crate) home_identity: RootIdentity, pub(crate) program_digest: String,
    pub(crate) version: String, pub(crate) registration_revision: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GrokGrant {
    pub(crate) binding_id: String, pub(crate) instance_id: String,
    pub(crate) domain_id: String, pub(crate) session_id: String,pub(crate) seat_id:String,
    pub(crate) seat_incarnation:String,pub(crate) generation: String, pub(crate) request_id: String,
    pub(crate) profile_name: String, pub(crate) profile_sid: String, pub(crate) home_identity: RootIdentity,
    pub(crate) program_digest:String,
    pub(crate) auth_identity: RootIdentity, pub(crate) phase: String,
    pub(crate) process_operation_id: Option<String>, pub(crate) ticket: Option<String>,
    pub(crate) custodian_nonce: Option<String>, pub(crate) pid: Option<u32>,
    pub(crate) creation_time_100ns: Option<u64>, pub(crate) image_path: Option<String>,
    pub(crate) stop_fact_id: Option<String>,
    pub(crate) revision: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GrokEffect {
    pub(crate) effect_id: String, pub(crate) binding_id: String,
    pub(crate) action: String, pub(crate) object_identity: RootIdentity,
    pub(crate) relative_name: String, pub(crate) rights: u32,
    pub(crate) flags: u32, pub(crate) before_aces:String,
    pub(crate) after_aces:String,pub(crate) before_control:u16,
    pub(crate) after_control:u16,pub(crate) other_aces_sha256:String,
    pub(crate) phase: String,
    pub(crate) revision: i64,
}

fn db_error(error: impl std::fmt::Debug) -> String { format!("grok F journal: {error:?}") }
fn stmt(db: &VerifiedDatabaseConnection<'_>, sql: &str) -> Result<Statement, String> {
    Statement::prepare(db.as_ptr(), sql).map_err(db_error)
}
fn bind(row: &Statement, values: &[&str]) -> Result<(), String> {
    for (index, value) in values.iter().enumerate() {
        row.bind_text(index as i32 + 1, value).map_err(db_error)?;
    }
    Ok(())
}
fn next(row: &Statement) -> Result<bool, String> { row.step_row().map_err(db_error) }
fn text(row: &Statement, index: i32) -> Result<String, String> {
    row.column_text(index).map_err(db_error)
}
fn changed(db: &VerifiedDatabaseConnection<'_>) -> Result<(), String> {
    let row = stmt(db, "SELECT changes()")?;
    if !next(&row)? || text(&row,0)? != "1" { return Err("grok F journal: CAS conflict".into()); }
    Ok(())
}
fn tx<T>(db: &mut VerifiedDatabaseConnection<'_>, f: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T,String>) -> Result<T,String> {
    db.execute("BEGIN IMMEDIATE").map_err(db_error)?;
    match f(db) {
        Ok(value) => { db.execute("COMMIT").map_err(|e|format!("grok F journal commit UNKNOWN: {e:?}"))?; Ok(value) },
        Err(error) => {
            db.execute("ROLLBACK").map_err(|e|format!("grok F journal rollback UNKNOWN: original={error}; rollback={e:?}"))?;
            Err(error)
        }
    }
}
fn parse_identity(value: String) -> Result<RootIdentity,String> {
    let (volume,file)=value.strip_prefix("volume:").and_then(|v|v.split_once("/file:"))
        .ok_or("grok F journal: malformed identity")?;
    if volume.len()!=16 || file.len()!=32 || !volume.bytes().chain(file.bytes()).all(|b|b.is_ascii_hexdigit()) {
        return Err("grok F journal: malformed identity".into());
    }
    let volume_serial=u64::from_str_radix(volume,16).map_err(db_error)?;
    let mut file_id=[0u8;16];
    for (i, part) in file.as_bytes().chunks_exact(2).enumerate() {
        file_id[i]=u8::from_str_radix(std::str::from_utf8(part).map_err(db_error)?,16).map_err(db_error)?;
    }
    let identity=RootIdentity{volume_serial,file_id};
    if identity.opaque()!=value {return Err("grok F journal: noncanonical identity".into());}
    Ok(identity)
}
fn optional(row:&Statement,index:i32)->Result<Option<String>,String> {
    let value=text(row,index)?;
    if value.is_empty() {Ok(None)} else {Ok(Some(value))}
}

pub(crate) fn initialize_grok_home_grant_schema(db:&mut VerifiedDatabaseConnection<'_>)->Result<(),String> {
    let shadow=stmt(db,"SELECT 1 FROM temp.sqlite_schema WHERE name LIKE 'gogoke_v37_grok_home_%' OR tbl_name LIKE 'gogoke_v37_grok_home_%' LIMIT 1")?;
    if next(&shadow)? {return Err("grok F journal: shadow schema".into());}
    let side_effect=stmt(db,"SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND tbl_name LIKE 'gogoke_v37_grok_home_%' LIMIT 1")?;
    if next(&side_effect)? {return Err("grok F journal: unexpected schema side effect".into());}
    let query=stmt(db,"SELECT name,sql FROM main.sqlite_schema WHERE name LIKE 'gogoke_v37_grok_home_%' ORDER BY name")?;
    let mut observed=Vec::new();
    while next(&query)? {observed.push((text(&query,0)?,text(&query,1)?));}
    let mut old=SCHEMA.iter().map(|(n,s)|(n.to_string(),s.to_string())).collect::<Vec<_>>();
    old.sort();
    let mut expected=old.clone();
    expected.push(("gogoke_v37_grok_home_root_anchor".into(),ROOT_ANCHOR_SQL.into()));
    expected.sort();
    if observed==expected {return Ok(());}
    // A pre-anchor database is allowed to acquire the empty table. Its old
    // effects do not become an ordered baseline by this schema transition.
    if observed==old {
        return tx(db,|db|db.execute(ROOT_ANCHOR_SQL).map_err(db_error));
    }
    if !observed.is_empty(){return Err("grok F journal: changed or partial schema".into());}
    tx(db,|db|{
        for (_,sql) in SCHEMA {db.execute(sql).map_err(db_error)?;}
        db.execute(ROOT_ANCHOR_SQL).map_err(db_error)?;
        Ok(())
    })
}

pub(crate) fn read_grok_root_anchor(db:&VerifiedDatabaseConnection<'_>,
    instance:&str)->Result<Option<GrokRootAnchor>,String>{
    let row=stmt(db,"SELECT root_identity,home_identity,program_digest,version,registration_revision,baseline_acl_hex,baseline_control,baseline_effect_id,acl_hex,acl_control,source_effect_id,revision FROM main.gogoke_v37_grok_home_root_anchor WHERE instance_id=?1")?;
    bind(&row,&[instance])?;
    if !next(&row)? {return Ok(None);}
    let anchor=GrokRootAnchor{instance_id:instance.into(),
        root_identity:parse_identity(text(&row,0)?)?,
        home_identity:parse_identity(text(&row,1)?)?,
        program_digest:text(&row,2)?,version:text(&row,3)?,
        registration_revision:text(&row,4)?.parse().map_err(db_error)?,
        baseline_acl_hex:text(&row,5)?,
        baseline_control:text(&row,6)?.parse().map_err(db_error)?,
        baseline_effect_id:text(&row,7)?,
        acl_hex:text(&row,8)?,
        acl_control:text(&row,9)?.parse().map_err(db_error)?,
        source_effect_id:text(&row,10)?,
        revision:text(&row,11)?.parse().map_err(db_error)?};
    if next(&row)? {return Err("grok F journal: duplicate root anchor".into());}
    Ok(Some(anchor))
}

pub(crate) fn current_domain(db:&VerifiedDatabaseConnection<'_>, instance_id:&str)->Result<GrokDomain,String> {
    current_domain_with_catalog(db,instance_id,&|driver,digest,version|{
        super::program_source::locate_bound_instance_program(db,instance_id,driver,digest,version)
            .map(|_|()).map_err(|error|format!("grok F journal: managed Grok program unavailable: {error:?}"))
    })
}

fn current_domain_with_catalog(db:&VerifiedDatabaseConnection<'_>,instance_id:&str,
    catalog:&impl Fn(&str,&str,&str)->Result<(),String>)->Result<GrokDomain,String> {
    let row=stmt(db,"SELECT home_identity,program_digest,version,revision FROM main.gogoke_v37_instances WHERE instance_id=?1 AND driver_id='grok' AND version='1.0.41' AND login_state='LOGGED_IN'")?;
    bind(&row,&[instance_id])?;
    if !next(&row)? {return Err("grok F journal: fixed Grok instance absent".into());}
    let home_identity=parse_identity(text(&row,0)?)?;
    let program_digest=text(&row,1)?;
    let version=text(&row,2)?;
    let registration_revision=text(&row,3)?.parse::<i64>().map_err(db_error)?;
    if next(&row)? {return Err("grok F journal: duplicate instance".into());}
    drop(row);
    // F's install read is observational and does not maintain install_state.
    // Require the live fixed catalog to match the registered bytes and version.
    catalog("grok",&program_digest,&version)?;
    Ok(GrokDomain {instance_id:instance_id.into(),root_identity:db.root_identity().clone(),
        home_identity,program_digest,version,registration_revision})
}

fn existing_domain(db:&VerifiedDatabaseConnection<'_>,instance:&str)->Result<Option<(GrokDomain,i64)>,String>{
    let row=stmt(db,"SELECT root_identity,home_identity,program_digest,version,registration_revision,revision FROM main.gogoke_v37_grok_home_domains WHERE instance_id=?1")?;
    bind(&row,&[instance])?;
    if !next(&row)?{return Ok(None);}
    let result=GrokDomain{instance_id:instance.into(),root_identity:parse_identity(text(&row,0)?)?,
        home_identity:parse_identity(text(&row,1)?)?,program_digest:text(&row,2)?,version:text(&row,3)?,
        registration_revision:text(&row,4)?.parse().map_err(db_error)?};
    let revision=text(&row,5)?.parse().map_err(db_error)?;
    if next(&row)?{return Err("grok F journal: duplicate domain".into());}
    Ok(Some((result,revision)))
}

pub(crate) fn begin_grok_grant(db:&mut VerifiedDatabaseConnection<'_>,
    domain:&GrokDomain,grant:&GrokGrant)->Result<GrokGrant,String>{
    begin_grok_grant_with_catalog_and_anchor(db,domain,grant,&current_domain,None)
}

pub(crate) fn begin_grok_grant_with_root_anchor(db:&mut VerifiedDatabaseConnection<'_>,
    domain:&GrokDomain,grant:&GrokGrant,acl_hex:&str,acl_control:u16,
    other_aces_sha256:&str)
    ->Result<GrokGrant,String>{
    begin_grok_grant_with_catalog_and_anchor(db,domain,grant,&current_domain,
        Some((acl_hex,acl_control,other_aces_sha256)))
}

fn settled_grok_holder_gone_release(db:&VerifiedDatabaseConnection<'_>,
    grant:&GrokGrant)->Result<bool,String>{
    if grant.stop_fact_id.is_some() {return Ok(false);}
    let suffix=sha256_hex(format!("{}\n{}\n{}\n{}\n{}",
        db.root_identity().opaque(),grant.domain_id,grant.session_id,
        grant.seat_incarnation,grant.generation).as_bytes());
    if grant.profile_name!=format!("Gogoke37.Session.{}",&suffix[..40]) {
        return Ok(false);
    }
    let profile=AppContainerProfile::derive_for_revocation(&grant.profile_name)
        .map_err(db_error)?;
    if profile.sid_identity().map_err(db_error)?!=grant.profile_sid {
        return Ok(false);
    }
    let operation=grant.process_operation_id.as_deref().ok_or("grok F journal: old operation absent")?;
    let pid=grant.pid.ok_or("grok F journal: old PID absent")?.to_string();
    let creation=grant.creation_time_100ns
        .ok_or("grok F journal: old creation absent")?.to_string();
    let row=stmt(db,"SELECT h.revision FROM main.gogoke_v37_h_claim h JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=h.process_operation_id AND e.instance_id=h.instance_id AND e.domain_id=h.domain_id AND e.session_id=h.session_id AND e.generation=h.generation AND e.binding_id=h.binding_id JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.profile_id=h.instance_id AND c.domain_id=h.domain_id AND c.generation=h.generation JOIN main.gogoke_v37_effective_seat s ON s.domain_id=h.domain_id AND s.session_id=h.session_id AND s.generation=h.generation AND s.seat_id=e.seat_id AND s.seat_incarnation=e.seat_incarnation AND s.selected_instance_id=h.instance_id WHERE h.instance_id=?1 AND h.domain_id=?2 AND h.session_id=?3 AND h.generation=?4 AND h.binding_id=?5 AND e.request_id=?6 AND s.seat_id=?7 AND s.seat_incarnation=?8 AND c.operation_id=?9 AND c.ticket=?10 AND c.custodian_nonce=?11 AND c.pid=?12 AND c.creation_time_100ns=?13 AND c.image_path=?14 AND c.binary_digest_sha256=?15 AND h.state='RELEASED' AND h.stop_fact_id IS NULL AND e.stop_fact_id IS NULL AND c.stop_proof_hash IS NULL AND c.state IN ('PREPARED','ACTIVE','UNKNOWN') AND e.phase IN ('PREPARED','ACTIVE','UNKNOWN')")?;
    bind(&row,&[&grant.instance_id,&grant.domain_id,&grant.session_id,
        &grant.generation,&grant.binding_id,&grant.request_id,&grant.seat_id,
        &grant.seat_incarnation,operation,
        grant.ticket.as_deref().ok_or("grok F journal: old ticket absent")?,
        grant.custodian_nonce.as_deref().ok_or("grok F journal: old nonce absent")?,
        &pid,&creation,
        grant.image_path.as_deref().ok_or("grok F journal: old image absent")?,
        &grant.program_digest])?;
    if !next(&row)? {return Ok(false);}
    let revision:i64=text(&row,0)?.parse().map_err(db_error)?;
    if next(&row)? {return Err("grok F journal: duplicate holder-gone H tuple".into());}
    let previous=revision.checked_sub(1)
        .ok_or("grok F journal: invalid holder-gone H revision")?;
    let raw=format!("{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        grant.binding_id,grant.instance_id,grant.domain_id,grant.session_id,
        grant.seat_id,grant.seat_incarnation,grant.generation,grant.request_id,
        pid,creation,previous);
    let raw_hex:String=raw.as_bytes().iter().map(|byte|format!("{byte:02x}")).collect();
    let request_id=format!("grok-gone-{}",&sha256_hex(raw.as_bytes())[..40]);
    let journal=stmt(db,"SELECT 1 FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2 AND operation='holder-gone-release' AND status='APPLIED' AND request_id=?3 AND raw_hex=?4 AND previous_revision=?5 AND revision=?6")?;
    bind(&journal,&[&grant.domain_id,&grant.session_id,&request_id,&raw_hex,
        &previous.to_string(),&revision.to_string()])?;
    let found=next(&journal)?;
    if found &&next(&journal)? {
        return Err("grok F journal: duplicate holder-gone release".into());
    }
    Ok(found)
}

/// A completed old journal has no ordered ACE bytes. It may authorize a NEW
/// baseline only after every old writer is durably quiescent and the current
/// canonical root matches a settled original ROOT effect's complete
/// non-package multiset/control. This does not recover the old ACE order.
fn settled_legacy_root_baseline(db:&VerifiedDatabaseConnection<'_>,
    domain:&GrokDomain,candidate:&GrokGrant,old_grants:&[GrokGrant],control:u16,
    other_hash:&str)->Result<String,String>{
    let blocked=|sql:&str,value:&str|->Result<(),String>{
        let row=stmt(db,sql)?;
        bind(&row,&[value])?;
        if next(&row)? {return Err("grok F journal: old H/credential writer not quiescent".into());}
        Ok(())
    };
    for sql in [
        "SELECT 1 FROM main.gogoke_v37_h_generation_change g JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=g.old_process_operation_id WHERE e.instance_id=?1 AND g.stage NOT IN ('APPLIED','CANCELLED','UNSUPPORTED') LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_credential_aliases WHERE instance_id=?1 AND state<>'REMOVED' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_credential_profiles WHERE instance_id=?1 AND state<>'REVOKED' LIMIT 1",
    ] {blocked(sql,&domain.instance_id)?;}
    let credential_target=format!("credential-instance-{}",sha256_hex(domain.instance_id.as_bytes()));
    blocked("SELECT 1 FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase IN ('PREPARING','UNKNOWN') LIMIT 1",
        &credential_target)?;
    let mut source=None;
    let mut released_gone=Vec::new();
    for old in old_grants {
        if old.phase!="REVOKED" ||old.home_identity!=domain.home_identity ||
            old.program_digest!=domain.program_digest {
            return Err("grok F journal: old grant has unresolved authority".into());
        }
        let effects=read_grok_effects(db,&old.binding_id)?;
        if effects.iter().any(|effect|effect.phase!="APPLIED") {
            return Err("grok F journal: old physical effect unresolved".into());
        }
        let root_grants=effects.iter().filter(|e|e.action=="GRANT_ROOT" &&
            e.object_identity==domain.home_identity &&
            e.after_aces=="1:1245631:3").count();
        let root_revokes=effects.iter().filter(|e|e.action=="REVOKE_ROOT" &&
            e.object_identity==domain.home_identity &&e.after_aces.is_empty()).count();
        if root_grants!=1 ||root_revokes!=1 ||
            !effects.iter().any(|e|e.action=="GRANT_AUTH" &&
                e.object_identity==old.auth_identity) {
            return Err("grok F journal: old root/auth provenance incomplete".into());
        }
        for auth in effects.iter().filter(|e|e.action=="GRANT_AUTH") {
            if !effects.iter().any(|e|e.action=="REVOKE_AUTH" &&
                e.object_identity==auth.object_identity) {
                return Err("grok F journal: old auth FileID not revoked".into());
            }
        }
        if let Some(operation)=old.process_operation_id.as_deref() {
            let row=stmt(db,"SELECT 1 FROM main.gogoke_coordination_process_custody c JOIN main.gogoke_v37_h_claim h ON h.process_operation_id=c.operation_id AND h.instance_id=c.profile_id AND h.domain_id=c.domain_id AND h.generation=c.generation JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=c.operation_id AND e.instance_id=c.profile_id AND e.domain_id=c.domain_id AND e.generation=c.generation AND e.session_id=h.session_id AND e.binding_id=h.binding_id WHERE c.operation_id=?1 AND c.profile_id=?2 AND c.domain_id=?3 AND c.generation=?4 AND c.ticket=?5 AND c.custodian_nonce=?6 AND c.pid=?7 AND c.creation_time_100ns=?8 AND c.image_path=?9 AND c.binary_digest_sha256=?10 AND c.state='STOPPED' AND c.stop_proof_hash=?11 AND h.binding_id=?12 AND h.session_id=?13 AND h.state IN ('STOPPED','RELEASED') AND h.stop_fact_id=?11 AND e.request_id=?14 AND e.phase='STOPPED' AND e.stop_fact_id=?11")?;
            let pid=old.pid.ok_or("grok F journal: old original pid absent")?.to_string();
            let creation=old.creation_time_100ns
                .ok_or("grok F journal: old original creation absent")?.to_string();
            let stop=old.stop_fact_id.as_deref().unwrap_or("");
            bind(&row,&[operation,&old.instance_id,&old.domain_id,&old.generation,
                old.ticket.as_deref().ok_or("grok F journal: old original ticket absent")?,
                old.custodian_nonce.as_deref().ok_or("grok F journal: old original nonce absent")?,
                &pid,&creation,
                old.image_path.as_deref().ok_or("grok F journal: old original image absent")?,
                &old.program_digest,stop,&old.binding_id,&old.session_id,&old.request_id])?;
            let stopped=!stop.is_empty() &&next(&row)?;
            if stopped &&next(&row)? {
                return Err("grok F journal: duplicate old stopped F/H tuple".into());
            }
            if !stopped {
                if old.stop_fact_id.is_some() ||
                    !settled_grok_holder_gone_release(db,old)? {
                    return Err("grok F journal: old writer lacks exact StopFact or holder-gone release".into());
                }
                released_gone.push(operation.to_owned());
            }
        } else if old.stop_fact_id.is_some() {
            return Err("grok F journal: old no-attempt StopFact fabricated".into());
        }
        if source.is_none() {
            source=effects.iter().find(|e|e.action=="REVOKE_ROOT" &&
                e.after_control==control &&e.other_aces_sha256==other_hash)
                .map(|e|e.effect_id.clone());
        }
    }
    let custody=stmt(db,"SELECT operation_id,state,COALESCE(stop_proof_hash,'') FROM main.gogoke_coordination_process_custody WHERE profile_id=?1")?;
    bind(&custody,&[&domain.instance_id])?;
    while next(&custody)? {
        let operation=text(&custody,0)?;
        let state=text(&custody,1)?;
        let stop=text(&custody,2)?;
        if !(state=="STOPPED" && !stop.is_empty()) &&
            !released_gone.contains(&operation) {
            return Err("grok F journal: unqualified old process writer".into());
        }
    }
    let episode=stmt(db,"SELECT COALESCE(process_operation_id,''),phase,COALESCE(stop_fact_id,'') FROM main.gogoke_v37_h_process_episode WHERE instance_id=?1 AND binding_id<>?2")?;
    bind(&episode,&[&domain.instance_id,&candidate.binding_id])?;
    while next(&episode)? {
        let operation=text(&episode,0)?;
        let phase=text(&episode,1)?;
        let stop=text(&episode,2)?;
        if !(phase=="STOPPED" && !stop.is_empty()) &&
            !(phase=="FAILED" && operation.is_empty()) &&
            !(released_gone.contains(&operation) &&stop.is_empty() &&
                matches!(phase.as_str(),"PREPARED"|"ACTIVE"|"UNKNOWN")) {
            return Err("grok F journal: unqualified old H episode".into());
        }
    }
    let claim=stmt(db,"SELECT COALESCE(process_operation_id,''),state,COALESCE(stop_fact_id,'') FROM main.gogoke_v37_h_claim WHERE instance_id=?1 AND binding_id<>?2")?;
    bind(&claim,&[&domain.instance_id,&candidate.binding_id])?;
    while next(&claim)? {
        let operation=text(&claim,0)?;
        let state=text(&claim,1)?;
        let stop=text(&claim,2)?;
        if !(matches!(state.as_str(),"STOPPED"|"RELEASED") &&
            ((operation.is_empty() &&stop.is_empty()) ||
             (!stop.is_empty()) ||
             (state=="RELEASED" &&released_gone.contains(&operation)))) {
            return Err("grok F journal: unqualified old H claim".into());
        }
    }
    source.ok_or_else(||"grok F journal: old settled ROOT effect does not conserve current ACL".into())
}

fn begin_grok_grant_with_catalog(db:&mut VerifiedDatabaseConnection<'_>,
    domain:&GrokDomain,grant:&GrokGrant,
    observe:&impl Fn(&VerifiedDatabaseConnection<'_>,&str)->Result<GrokDomain,String>)->Result<GrokGrant,String>{
    begin_grok_grant_with_catalog_and_anchor(db,domain,grant,observe,None)
}

fn begin_grok_grant_with_catalog_and_anchor(db:&mut VerifiedDatabaseConnection<'_>,
    domain:&GrokDomain,grant:&GrokGrant,
    observe:&impl Fn(&VerifiedDatabaseConnection<'_>,&str)->Result<GrokDomain,String>,
    root_acl:Option<(&str,u16,&str)>)->Result<GrokGrant,String>{
    tx(db,|db|{
        if observe(db,&domain.instance_id)?!=*domain {return Err("grok F journal: current F pin/revision changed".into());}
        if let Some((old,revision))=existing_domain(db,&domain.instance_id)? {
            if old.instance_id!=domain.instance_id ||old.root_identity!=domain.root_identity ||
                old.home_identity!=domain.home_identity ||old.program_digest!=domain.program_digest ||
                old.version!=domain.version ||old.registration_revision>domain.registration_revision {
                return Err("grok F journal: established physical domain or pin changed".into());
            }
            if old.registration_revision<domain.registration_revision {
                let update=stmt(db,"UPDATE main.gogoke_v37_grok_home_domains SET registration_revision=?1,revision=revision+1 WHERE instance_id=?2 AND root_identity=?3 AND home_identity=?4 AND program_digest=?5 AND version=?6 AND registration_revision=?7 AND revision=?8")?;
                bind(&update,&[&domain.registration_revision.to_string(),&domain.instance_id,
                    &domain.root_identity.opaque(),&domain.home_identity.opaque(),
                    &domain.program_digest,&domain.version,&old.registration_revision.to_string(),
                    &revision.to_string()])?;
                next(&update)?;changed(db)?;
            }
        } else {
            let row=stmt(db,"INSERT INTO main.gogoke_v37_grok_home_domains VALUES(?1,?2,?3,?4,?5,?6,1)")?;
            bind(&row,&[&domain.instance_id,&domain.root_identity.opaque(),&domain.home_identity.opaque(),
                &domain.program_digest,&domain.version,&domain.registration_revision.to_string()])?;
            next(&row)?; changed(db)?;
        }
        let prior_grants=read_grok_grants(db,&domain.instance_id)?;
        for old in &prior_grants {
            if old.binding_id==grant.binding_id {
                if old.request_id==grant.request_id && old.instance_id==grant.instance_id &&
                    old.domain_id==grant.domain_id &&old.session_id==grant.session_id &&
                    old.generation==grant.generation && old.seat_id==grant.seat_id &&
                    old.seat_incarnation==grant.seat_incarnation &&
                    old.program_digest==grant.program_digest &&old.profile_name==grant.profile_name && old.profile_sid==grant.profile_sid &&
                    old.home_identity==grant.home_identity && old.auth_identity==grant.auth_identity {
                    return Ok(old.clone());
                }
                return Err("grok F journal: binding replay mismatch".into());
            }
            if old.profile_sid==grant.profile_sid || matches!(old.phase.as_str(),
                "GRANT_PENDING"|"REVOKE_PENDING"|"UNKNOWN") {
                return Err("grok F journal: profile reuse or unresolved ACL intent".into());
            }
            if read_grok_effects(db,&old.binding_id)?.iter().any(|effect|effect.phase!="APPLIED") {
                return Err("grok F journal: previous physical effect unresolved".into());
            }
        }
        if grant.phase!="GRANT_PENDING" || grant.revision!=1 || grant.instance_id!=domain.instance_id ||
            grant.home_identity!=domain.home_identity || grant.process_operation_id.is_some() ||
            grant.ticket.is_some() || grant.custodian_nonce.is_some() || grant.pid.is_some() ||
            grant.creation_time_100ns.is_some() || grant.image_path.is_some() ||
            grant.stop_fact_id.is_some() {
            return Err("grok F journal: invalid grant intent".into());
        }
        if let Some((acl_hex,acl_control,other_aces_sha256))=root_acl {
            if acl_hex.is_empty() ||acl_hex.len()%2!=0 ||
                !acl_hex.bytes().all(|b|b.is_ascii_hexdigit()) ||
                other_aces_sha256.len()!=64 ||
                !other_aces_sha256.bytes().all(|b|b.is_ascii_hexdigit()) {
                return Err("grok F journal: invalid ordered root ACL".into());
            }
            if let Some(anchor)=read_grok_root_anchor(db,&domain.instance_id)? {
                if anchor.root_identity!=domain.root_identity ||
                    anchor.home_identity!=domain.home_identity ||
                    anchor.program_digest!=domain.program_digest ||
                    anchor.version!=domain.version ||
                    anchor.registration_revision!=domain.registration_revision ||
                    anchor.acl_hex!=acl_hex ||anchor.acl_control!=acl_control {
                    return Err("grok F journal: ordered root ACL anchor changed".into());
                }
            } else {
                let baseline_effect_id=if prior_grants.is_empty() {String::new()} else {
                    settled_legacy_root_baseline(db,domain,grant,&prior_grants,
                        acl_control,other_aces_sha256)?
                };
                let row=stmt(db,"INSERT INTO main.gogoke_v37_grok_home_root_anchor VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'',1)")?;
                bind(&row,&[&domain.instance_id,&domain.root_identity.opaque(),
                    &domain.home_identity.opaque(),&domain.program_digest,&domain.version,
                    &domain.registration_revision.to_string(),acl_hex,&acl_control.to_string(),
                    &baseline_effect_id,acl_hex,&acl_control.to_string()])?;
                next(&row)?;changed(db)?;
            }
        }
        let row=stmt(db,"INSERT INTO main.gogoke_v37_grok_home_grants VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,NULL,NULL,NULL,NULL,NULL,NULL,NULL,1)")?;
        bind(&row,&[&grant.binding_id,&grant.instance_id,&grant.domain_id,&grant.session_id,
            &grant.seat_id,&grant.seat_incarnation,&grant.generation,&grant.request_id,&grant.profile_name,&grant.profile_sid,
            &grant.program_digest,&grant.home_identity.opaque(),
            &grant.auth_identity.opaque(),&grant.phase])?;
        next(&row)?;changed(db)?;
        Ok(grant.clone())
    })
}

pub(crate) fn read_grok_grants(db:&VerifiedDatabaseConnection<'_>,instance:&str)->Result<Vec<GrokGrant>,String>{
    let row=stmt(db,"SELECT binding_id,domain_id,session_id,seat_id,seat_incarnation,generation,request_id,profile_name,profile_sid,program_digest,home_identity,auth_identity,phase,COALESCE(process_operation_id,''),COALESCE(ticket,''),COALESCE(custodian_nonce,''),COALESCE(pid,''),COALESCE(creation_time_100ns,''),COALESCE(image_path,''),COALESCE(stop_fact_id,''),revision FROM main.gogoke_v37_grok_home_grants WHERE instance_id=?1 ORDER BY binding_id")?;
    bind(&row,&[instance])?;
    let mut result=Vec::new();
    while next(&row)? {
        result.push(GrokGrant{binding_id:text(&row,0)?,instance_id:instance.into(),domain_id:text(&row,1)?,
            session_id:text(&row,2)?,seat_id:text(&row,3)?,seat_incarnation:text(&row,4)?,generation:text(&row,5)?,request_id:text(&row,6)?,
            profile_name:text(&row,7)?,profile_sid:text(&row,8)?,program_digest:text(&row,9)?,
            home_identity:parse_identity(text(&row,10)?)?,auth_identity:parse_identity(text(&row,11)?)?,phase:text(&row,12)?,
            process_operation_id:optional(&row,13)?,ticket:optional(&row,14)?,
            custodian_nonce:optional(&row,15)?,pid:optional(&row,16)?.map(|v|v.parse()).transpose().map_err(db_error)?,
            creation_time_100ns:optional(&row,17)?.map(|v|v.parse()).transpose().map_err(db_error)?,
            image_path:optional(&row,18)?,stop_fact_id:optional(&row,19)?,
            revision:text(&row,20)?.parse().map_err(db_error)?});
    }
    Ok(result)
}

pub(crate) fn begin_grok_effect(db:&mut VerifiedDatabaseConnection<'_>,effect:&GrokEffect)->Result<GrokEffect,String>{
    tx(db,|db|{
        let row=stmt(db,"SELECT binding_id,action,object_identity,relative_name,rights,flags,before_aces,after_aces,before_control,after_control,other_aces_sha256,phase,revision FROM main.gogoke_v37_grok_home_effects WHERE effect_id=?1")?;
        bind(&row,&[&effect.effect_id])?;
        if next(&row)? {
            let old=GrokEffect{effect_id:effect.effect_id.clone(),binding_id:text(&row,0)?,action:text(&row,1)?,
                object_identity:parse_identity(text(&row,2)?)?,relative_name:text(&row,3)?,
                rights:text(&row,4)?.parse().map_err(db_error)?,flags:text(&row,5)?.parse().map_err(db_error)?,
                before_aces:text(&row,6)?,after_aces:text(&row,7)?,
                before_control:text(&row,8)?.parse().map_err(db_error)?,
                after_control:text(&row,9)?.parse().map_err(db_error)?,
                other_aces_sha256:text(&row,10)?,phase:text(&row,11)?,
                revision:text(&row,12)?.parse().map_err(db_error)?};
            if old.binding_id!=effect.binding_id ||old.action!=effect.action ||
                old.object_identity!=effect.object_identity ||old.relative_name!=effect.relative_name ||
                old.rights!=effect.rights ||old.flags!=effect.flags ||
                old.after_aces!=effect.after_aces ||old.after_control!=effect.after_control ||
                old.phase=="UNKNOWN" {
                return Err("grok F journal: effect replay mismatch/UNKNOWN".into());
            }
            return Ok(old);
        }
        let row=stmt(db,"INSERT INTO main.gogoke_v37_grok_home_effects VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'INTENT',1)")?;
        bind(&row,&[&effect.effect_id,&effect.binding_id,&effect.action,
            &effect.object_identity.opaque(),&effect.relative_name,&effect.rights.to_string(),
            &effect.flags.to_string(),&effect.before_aces,&effect.after_aces,
            &effect.before_control.to_string(),&effect.after_control.to_string(),
            &effect.other_aces_sha256])?;
        next(&row)?;changed(db)?;
        Ok(effect.clone())
    })
}

pub(crate) fn finish_grok_effect(db:&mut VerifiedDatabaseConnection<'_>,effect:&GrokEffect)->Result<(),String>{
    tx(db,|db|finish_grok_effect_in_tx(db,effect))
}

fn finish_grok_effect_in_tx(db:&mut VerifiedDatabaseConnection<'_>,
    effect:&GrokEffect)->Result<(),String>{
        let row=stmt(db,"UPDATE main.gogoke_v37_grok_home_effects SET phase='APPLIED',revision=revision+1 WHERE effect_id=?1 AND binding_id=?2 AND action=?3 AND object_identity=?4 AND relative_name=?5 AND rights=?6 AND flags=?7 AND before_aces=?8 AND after_aces=?9 AND before_control=?10 AND after_control=?11 AND other_aces_sha256=?12 AND phase='INTENT' AND revision=?13")?;
        bind(&row,&[&effect.effect_id,&effect.binding_id,&effect.action,
            &effect.object_identity.opaque(),&effect.relative_name,&effect.rights.to_string(),
            &effect.flags.to_string(),&effect.before_aces,&effect.after_aces,
            &effect.before_control.to_string(),&effect.after_control.to_string(),
            &effect.other_aces_sha256,
            &effect.revision.to_string()])?;
        next(&row)?;changed(db)
}

pub(crate) fn finish_grok_root_effect_with_anchor(db:&mut VerifiedDatabaseConnection<'_>,
    effect:&GrokEffect,previous:&GrokRootAnchor,after_acl_hex:&str,
    after_control:u16)->Result<(),String>{
    if !matches!(effect.action.as_str(),"GRANT_ROOT"|"REVOKE_ROOT") ||
        effect.object_identity!=previous.home_identity ||effect.phase!="INTENT" ||
        after_control!=effect.after_control ||after_acl_hex.is_empty() ||
        after_acl_hex.len()%2!=0 ||
        !after_acl_hex.bytes().all(|b|b.is_ascii_hexdigit()) {
        return Err("grok F journal: invalid root effect anchor".into());
    }
    tx(db,|db|{
        if read_grok_root_anchor(db,&previous.instance_id)?.as_ref()!=Some(previous) {
            return Err("grok F journal: root anchor CAS changed".into());
        }
        finish_grok_effect_in_tx(db,effect)?;
        let row=stmt(db,"UPDATE main.gogoke_v37_grok_home_root_anchor SET acl_hex=?1,acl_control=?2,source_effect_id=?3,revision=revision+1 WHERE instance_id=?4 AND root_identity=?5 AND home_identity=?6 AND program_digest=?7 AND version=?8 AND registration_revision=?9 AND acl_hex=?10 AND acl_control=?11 AND source_effect_id=?12 AND revision=?13")?;
        bind(&row,&[after_acl_hex,&after_control.to_string(),&effect.effect_id,
            &previous.instance_id,&previous.root_identity.opaque(),
            &previous.home_identity.opaque(),&previous.program_digest,
            &previous.version,&previous.registration_revision.to_string(),
            &previous.acl_hex,&previous.acl_control.to_string(),
            &previous.source_effect_id,&previous.revision.to_string()])?;
        next(&row)?;changed(db)
    })
}

pub(crate) fn read_grok_effects(db:&VerifiedDatabaseConnection<'_>,binding:&str)->Result<Vec<GrokEffect>,String>{
    let row=stmt(db,"SELECT effect_id,action,object_identity,relative_name,rights,flags,before_aces,after_aces,before_control,after_control,other_aces_sha256,phase,revision FROM main.gogoke_v37_grok_home_effects WHERE binding_id=?1 ORDER BY effect_id")?;
    bind(&row,&[binding])?;
    let mut result=Vec::new();
    while next(&row)? {result.push(GrokEffect{effect_id:text(&row,0)?,binding_id:binding.into(),
        action:text(&row,1)?,object_identity:parse_identity(text(&row,2)?)?,relative_name:text(&row,3)?,
        rights:text(&row,4)?.parse().map_err(db_error)?,flags:text(&row,5)?.parse().map_err(db_error)?,
        before_aces:text(&row,6)?,after_aces:text(&row,7)?,
        before_control:text(&row,8)?.parse().map_err(db_error)?,
        after_control:text(&row,9)?.parse().map_err(db_error)?,
        other_aces_sha256:text(&row,10)?,phase:text(&row,11)?,
        revision:text(&row,12)?.parse().map_err(db_error)?});}
    Ok(result)
}

pub(crate) fn set_grok_grant_phase(db:&mut VerifiedDatabaseConnection<'_>,grant:&GrokGrant,
    next_phase:&str,stop:Option<&str>)->Result<(),String>{
    let valid=matches!((grant.phase.as_str(),next_phase),
        ("GRANT_PENDING","GRANTED_UNCREATED")|
        ("GRANT_PENDING","REVOKE_PENDING")|
        ("GRANTED_UNCREATED","REVOKE_PENDING")|
        ("ACTIVE","REVOKE_PENDING")|
        ("REVOKE_PENDING","RETIRED_CLEANUP_PENDING")|
        ("RETIRED_CLEANUP_PENDING","REVOKED")|
        ("REVOKE_PENDING","REVOKED")) ||
        (next_phase=="UNKNOWN" && grant.phase!="REVOKED");
    if !valid || (stop.is_some() && next_phase!="REVOKE_PENDING") {
        return Err("grok F journal: invalid phase".into());
    }
    tx(db,|db|{
        let effects=read_grok_effects(db,&grant.binding_id)?;
        if matches!(next_phase,"GRANTED_UNCREATED"|"RETIRED_CLEANUP_PENDING"|"REVOKED") {
            let (root_action,auth_action)=if next_phase=="GRANTED_UNCREATED" {
                ("GRANT_ROOT","GRANT_AUTH")
            } else {("REVOKE_ROOT","REVOKE_AUTH")};
            let root_done=effects.iter().any(|e|e.action==root_action &&e.phase=="APPLIED" &&
                e.object_identity==grant.home_identity);
            let mut auth_ids=vec![grant.auth_identity.clone()];
            if next_phase!="GRANTED_UNCREATED" {
                for id in effects.iter().filter(|e|e.action=="GRANT_AUTH" &&e.phase=="APPLIED")
                    .map(|e|e.object_identity.clone()) {
                    if !auth_ids.contains(&id){auth_ids.push(id);}
                }
            }
            if !root_done ||
                auth_ids.iter().any(|id|!effects.iter().any(|e|e.action==auth_action &&
                    e.phase=="APPLIED" && &e.object_identity==id)) ||
                effects.iter().any(|e|e.phase!="APPLIED") {
                return Err("grok F journal: required physical effects unresolved".into());
            }
        }
        let row=stmt(db,"UPDATE main.gogoke_v37_grok_home_grants SET phase=?1,stop_fact_id=COALESCE(NULLIF(?2,''),stop_fact_id),revision=revision+1 WHERE binding_id=?3 AND instance_id=?4 AND phase=?5 AND revision=?6")?;
        bind(&row,&[next_phase,stop.unwrap_or(""),&grant.binding_id,&grant.instance_id,
            &grant.phase,&grant.revision.to_string()])?;
        next(&row)?;changed(db)
    })
}

pub(crate) fn bind_grok_original_process(db:&mut VerifiedDatabaseConnection<'_>,grant:&GrokGrant,
    operation:&str,ticket:&str,nonce:&str,pid:u32,creation:u64,image:&str)->Result<(),String>{
    if grant.phase!="GRANTED_UNCREATED" || operation.is_empty() || ticket.is_empty() ||
        nonce.is_empty() || image.is_empty() || pid==0 || creation==0 ||
        grant.process_operation_id.is_some() {
        return Err("grok F journal: original process binding invalid".into());
    }
    tx(db,|db|{
        let row=stmt(db,"UPDATE main.gogoke_v37_grok_home_grants SET phase='ACTIVE',process_operation_id=?1,ticket=?2,custodian_nonce=?3,pid=?4,creation_time_100ns=?5,image_path=?6,revision=revision+1 WHERE binding_id=?7 AND instance_id=?8 AND phase='GRANTED_UNCREATED' AND revision=?9 AND process_operation_id IS NULL")?;
        bind(&row,&[operation,ticket,nonce,&pid.to_string(),&creation.to_string(),image,
            &grant.binding_id,&grant.instance_id,&grant.revision.to_string()])?;
        next(&row)?;changed(db)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new,route_b_test_guard};
    use crate::store::instance::ProgramObservation;
    use std::time::{SystemTime,UNIX_EPOCH};

    fn identity(byte:u8)->RootIdentity{RootIdentity{volume_serial:1,file_id:[byte;16]}}
    fn grant(binding:&str,request:&str,sid:&str,digest:&str)->GrokGrant{
        GrokGrant{binding_id:binding.into(),instance_id:"grokA".into(),domain_id:"domainA".into(),
            session_id:format!("session{binding}"),seat_id:"seatA".into(),
            seat_incarnation:format!("seat-{binding}"),generation:"1".into(),
            request_id:request.into(),profile_name:format!("Gogoke37.{binding}"),
            profile_sid:sid.into(),program_digest:digest.into(),
            home_identity:identity(1),auth_identity:identity(2),phase:"GRANT_PENDING".into(),
            process_operation_id:None,ticket:None,custodian_nonce:None,pid:None,
            creation_time_100ns:None,image_path:None,stop_fact_id:None,revision:1}
    }
    fn acl_effect(binding:&str,action:&str,object:RootIdentity)->GrokEffect{
        GrokEffect{effect_id:format!("effect-{binding}-{action}"),binding_id:binding.into(),
            action:action.into(),object_identity:object,relative_name:if action.ends_with("ROOT"){
                ".".into()}else{"auth.json".into()},rights:1245631,
            flags:if action.ends_with("ROOT"){3}else{0},before_aces:String::new(),
            after_aces:format!("1:1245631:{}",if action.ends_with("ROOT"){3}else{0}),
            before_control:0,after_control:0,other_aces_sha256:"fixture-only".into(),
            phase:"INTENT".into(),revision:1}
    }

    #[test]
    fn original_acl_intent_replays_exactly_and_login_revision_advances_same_domain(){
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("grok-f-acl-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&path).unwrap();
        // This is a synthetic file for journal authority tests, not an
        // installed Grok program. The production path uses fixed_catalog.
        let program=path.join("synthetic-program.exe");
        let bytes=b"synthetic Grok catalog observation";
        std::fs::write(&program,bytes).unwrap();
        let digest=crate::store::digest::content_hash(bytes);
        let catalog=|driver:&str,pin:&str,version:&str|->Result<(),String>{
            if driver!="grok" || version!="1.0.41" {return Err("synthetic catalog: wrong driver or version".into());}
            let observed=ProgramObservation::observe(&program,version).map_err(db_error)?;
            if !observed.matches_pin(pin,version) {return Err("synthetic catalog: pin changed".into());}
            Ok(())
        };
        let root=RootLock::acquire(&path).unwrap();
        let mut db=create_new(&root,&path.join("state.sqlite")).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        super::super::initialize_schema(&mut db).unwrap();
        let insert=Statement::prepare(db.as_ptr(),"INSERT INTO main.gogoke_v37_instances VALUES('grokA','grok','homeA',?1,?2,'1.0.41','UNKNOWN','LOGGED_IN',1)").unwrap();
        insert.bind_text(1,&identity(1).opaque()).unwrap();insert.bind_text(2,&digest).unwrap();insert.step_done().unwrap();drop(insert);
        initialize_grok_home_grant_schema(&mut db).unwrap();
        let domain=current_domain_with_catalog(&db,"grokA",&catalog).unwrap();
        assert!(current_domain(&db,"grokA").is_err(),"production catalog must reject synthetic bytes");
        assert!(begin_grok_grant(&mut db,&domain,&grant("bindingA","openA","sid-A",&digest)).is_err(),
            "production grant must reject synthetic bytes before an intent");
        let first=begin_grok_grant_with_catalog(&mut db,&domain,&grant("bindingA","openA","sid-A",&digest),&|db,id|current_domain_with_catalog(db,id,&catalog)).unwrap();
        let root_effect=acl_effect("bindingA","GRANT_ROOT",identity(1));
        let pending=begin_grok_effect(&mut db,&root_effect).unwrap();
        assert_eq!(begin_grok_effect(&mut db,&root_effect).unwrap().phase,"INTENT");
        finish_grok_effect(&mut db,&pending).unwrap();
        assert_eq!(begin_grok_effect(&mut db,&root_effect).unwrap().phase,"APPLIED");
        let auth_effect=acl_effect("bindingA","GRANT_AUTH",identity(2));
        let pending=begin_grok_effect(&mut db,&auth_effect).unwrap();
        finish_grok_effect(&mut db,&pending).unwrap();
        set_grok_grant_phase(&mut db,&first,"GRANTED_UNCREATED",None).unwrap();
        // A failure before the first auth grant has a positive NoAttempt
        // settlement: exact root/auth revoke readbacks, without inventing a
        // GRANT_AUTH effect or process tuple.
        let failed=begin_grok_grant_with_catalog(&mut db,&domain,&grant("failedA","failedOpen","sid-failed",&digest),&|db,id|current_domain_with_catalog(db,id,&catalog)).unwrap();
        set_grok_grant_phase(&mut db,&failed,"REVOKE_PENDING",None).unwrap();
        for (action,id) in [("REVOKE_ROOT",identity(1)),("REVOKE_AUTH",identity(2))] {
            let mut effect=acl_effect("failedA",action,id);
            effect.after_aces.clear();
            let pending=begin_grok_effect(&mut db,&effect).unwrap();
            finish_grok_effect(&mut db,&pending).unwrap();
        }
        let pending=read_grok_grants(&db,"grokA").unwrap().into_iter()
            .find(|row|row.binding_id=="failedA").unwrap();
        set_grok_grant_phase(&mut db,&pending,"REVOKED",None).unwrap();
        db.execute("UPDATE main.gogoke_v37_instances SET revision=2 WHERE instance_id='grokA'").unwrap();
        let domain2=current_domain_with_catalog(&db,"grokA",&catalog).unwrap();
        begin_grok_grant_with_catalog(&mut db,&domain2,&grant("bindingB","openB","sid-B",&digest),&|db,id|current_domain_with_catalog(db,id,&catalog)).unwrap();
        let row=Statement::prepare(db.as_ptr(),"SELECT registration_revision,revision FROM main.gogoke_v37_grok_home_domains WHERE instance_id='grokA'").unwrap();
        assert!(row.step_row().unwrap());
        assert_eq!(row.column_text(0).unwrap(),"2");
        assert_eq!(row.column_text(1).unwrap(),"2");drop(row);
        db.execute("UPDATE main.gogoke_v37_instances SET program_digest='sha256:changed',revision=3 WHERE instance_id='grokA'").unwrap();
        assert!(current_domain_with_catalog(&db,"grokA",&catalog).is_err());
        db.execute(&format!("UPDATE main.gogoke_v37_instances SET program_digest='{digest}',revision=4 WHERE instance_id='grokA'")).unwrap();
        std::fs::remove_file(&program).unwrap();
        assert!(current_domain_with_catalog(&db,"grokA",&catalog).is_err());
        std::fs::write(&program,b"tampered synthetic program").unwrap();
        assert!(current_domain_with_catalog(&db,"grokA",&catalog).is_err());
        std::fs::write(&program,bytes).unwrap();
        db.execute(&format!("UPDATE main.gogoke_v37_instances SET home_identity='{}',revision=5 WHERE instance_id='grokA'",identity(3).opaque())).unwrap();
        let changed=current_domain_with_catalog(&db,"grokA",&catalog).unwrap();
        assert!(begin_grok_grant_with_catalog(&mut db,&changed,&grant("bindingC","openC","sid-C",&digest),&|db,id|current_domain_with_catalog(db,id,&catalog)).is_err());
        db.close_checked().unwrap();drop(root);std::fs::remove_dir_all(path).unwrap();
    }
}
