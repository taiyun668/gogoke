//! Private F journal for one pinned Grok instance HOME and its original
//! per-generation SID. No credential contents, path supplied by wire, or CLI
//! command is stored here. An ACL effect is never made before its intent.
use crate::root::RootIdentity;
use crate::store::atomic::Statement;
use crate::store::same_open::VerifiedDatabaseConnection;

const SCHEMA: [(&str, &str); 3] = [
    ("gogoke_v37_grok_home_domains", "CREATE TABLE gogoke_v37_grok_home_domains(instance_id TEXT PRIMARY KEY REFERENCES gogoke_v37_instances(instance_id),root_identity TEXT NOT NULL,home_identity TEXT NOT NULL,program_digest TEXT NOT NULL,version TEXT NOT NULL,registration_revision INTEGER NOT NULL CHECK(registration_revision>=1),revision INTEGER NOT NULL CHECK(revision>=1)) STRICT"),
    ("gogoke_v37_grok_home_grants", "CREATE TABLE gogoke_v37_grok_home_grants(binding_id TEXT PRIMARY KEY,instance_id TEXT NOT NULL REFERENCES gogoke_v37_grok_home_domains(instance_id),domain_id TEXT NOT NULL,session_id TEXT NOT NULL,seat_id TEXT NOT NULL,seat_incarnation TEXT NOT NULL,generation TEXT NOT NULL,request_id TEXT NOT NULL UNIQUE,profile_name TEXT NOT NULL,profile_sid TEXT NOT NULL UNIQUE,program_digest TEXT NOT NULL,home_identity TEXT NOT NULL,auth_identity TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('GRANT_PENDING','GRANTED_UNCREATED','ACTIVE','REVOKE_PENDING','RETIRED_CLEANUP_PENDING','REVOKED','UNKNOWN')),process_operation_id TEXT,ticket TEXT,custodian_nonce TEXT,pid INTEGER,creation_time_100ns INTEGER,image_path TEXT,stop_fact_id TEXT,revision INTEGER NOT NULL CHECK(revision>=1)) STRICT"),
    ("gogoke_v37_grok_home_effects", "CREATE TABLE gogoke_v37_grok_home_effects(effect_id TEXT PRIMARY KEY,binding_id TEXT NOT NULL REFERENCES gogoke_v37_grok_home_grants(binding_id),action TEXT NOT NULL CHECK(action IN ('GRANT_ROOT','GRANT_AUTH','REVOKE_ROOT','REVOKE_AUTH','REVOKE_RESIDUE')),object_identity TEXT NOT NULL,relative_name TEXT NOT NULL,rights INTEGER NOT NULL CHECK(rights=1245631),flags INTEGER NOT NULL CHECK(flags IN (0,3)),before_aces TEXT NOT NULL,after_aces TEXT NOT NULL,before_control INTEGER NOT NULL,after_control INTEGER NOT NULL,other_aces_sha256 TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('INTENT','APPLIED','UNKNOWN')),revision INTEGER NOT NULL CHECK(revision>=1)) STRICT"),
];

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
fn tx<T>(db: &mut VerifiedDatabaseConnection<'_>, f: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<T,String>) -> Result<T,String> {
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
    let mut expected=SCHEMA.iter().map(|(n,s)|(n.to_string(),s.to_string())).collect::<Vec<_>>();
    expected.sort();
    if observed==expected {return Ok(());}
    if !observed.is_empty(){return Err("grok F journal: changed or partial schema".into());}
    tx(db,|db|{
        for (_,sql) in SCHEMA {db.execute(sql).map_err(db_error)?;}
        Ok(())
    })
}

pub(crate) fn current_domain(db:&VerifiedDatabaseConnection<'_>, instance_id:&str)->Result<GrokDomain,String> {
    let row=stmt(db,"SELECT home_identity,program_digest,version,revision FROM main.gogoke_v37_instances WHERE instance_id=?1 AND driver_id='grok' AND version='1.0.41'")?;
    bind(&row,&[instance_id])?;
    if !next(&row)? {return Err("grok F journal: fixed Grok instance absent".into());}
    let home_identity=parse_identity(text(&row,0)?)?;
    let program_digest=text(&row,1)?;
    let version=text(&row,2)?;
    let registration_revision=text(&row,3)?.parse::<i64>().map_err(db_error)?;
    if next(&row)? {return Err("grok F journal: duplicate instance".into());}
    Ok(GrokDomain {instance_id:instance_id.into(),root_identity:db.root_identity().clone(),
        home_identity,program_digest,version,registration_revision})
}

fn existing_domain(db:&VerifiedDatabaseConnection<'_>,instance:&str)->Result<Option<GrokDomain>,String>{
    let row=stmt(db,"SELECT root_identity,home_identity,program_digest,version,registration_revision FROM main.gogoke_v37_grok_home_domains WHERE instance_id=?1")?;
    bind(&row,&[instance])?;
    if !next(&row)?{return Ok(None);}
    let result=GrokDomain{instance_id:instance.into(),root_identity:parse_identity(text(&row,0)?)?,
        home_identity:parse_identity(text(&row,1)?)?,program_digest:text(&row,2)?,version:text(&row,3)?,
        registration_revision:text(&row,4)?.parse().map_err(db_error)?};
    if next(&row)?{return Err("grok F journal: duplicate domain".into());}
    Ok(Some(result))
}

pub(crate) fn begin_grok_grant(db:&mut VerifiedDatabaseConnection<'_>,
    domain:&GrokDomain,grant:&GrokGrant)->Result<GrokGrant,String>{
    tx(db,|db|{
        if current_domain(db,&domain.instance_id)?!=*domain {return Err("grok F journal: current F pin/revision changed".into());}
        if let Some(old)=existing_domain(db,&domain.instance_id)? {
            if old!=*domain {return Err("grok F journal: established domain changed".into());}
        } else {
            let row=stmt(db,"INSERT INTO main.gogoke_v37_grok_home_domains VALUES(?1,?2,?3,?4,?5,?6,1)")?;
            bind(&row,&[&domain.instance_id,&domain.root_identity.opaque(),&domain.home_identity.opaque(),
                &domain.program_digest,&domain.version,&domain.registration_revision.to_string()])?;
            next(&row)?; changed(db)?;
        }
        for old in read_grok_grants(db,&domain.instance_id)? {
            if old.binding_id==grant.binding_id {
                if old.request_id==grant.request_id && old.instance_id==grant.instance_id &&
                    old.domain_id==grant.domain_id &&old.session_id==grant.session_id &&
                    old.generation==grant.generation && old.seat_id==grant.seat_id &&
                    old.seat_incarnation==grant.seat_incarnation &&
                    old.program_digest==grant.program_digest &&old.profile_name==grant.profile_name && old.profile_sid==grant.profile_sid &&
                    old.home_identity==grant.home_identity && old.auth_identity==grant.auth_identity {
                    return Ok(old);
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
    tx(db,|db|{
        let row=stmt(db,"UPDATE main.gogoke_v37_grok_home_effects SET phase='APPLIED',revision=revision+1 WHERE effect_id=?1 AND binding_id=?2 AND action=?3 AND object_identity=?4 AND relative_name=?5 AND rights=?6 AND flags=?7 AND before_aces=?8 AND after_aces=?9 AND before_control=?10 AND after_control=?11 AND other_aces_sha256=?12 AND phase='INTENT' AND revision=?13")?;
        bind(&row,&[&effect.effect_id,&effect.binding_id,&effect.action,
            &effect.object_identity.opaque(),&effect.relative_name,&effect.rights.to_string(),
            &effect.flags.to_string(),&effect.before_aces,&effect.after_aces,
            &effect.before_control.to_string(),&effect.after_control.to_string(),
            &effect.other_aces_sha256,
            &effect.revision.to_string()])?;
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
