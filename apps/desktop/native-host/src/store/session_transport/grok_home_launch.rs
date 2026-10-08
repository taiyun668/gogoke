//! Grok-only F/H composition. F persists each ACL intent before H changes the
//! exact, single-link original HOME object. No credential data is opened.
use super::runtime::{ClaimObservation, InstancePin};
use crate::process::{AppContainerProfile, GrokAuthMetadata, GrokAclSnapshot, PreparedCustody,
    NativeProcessHoldersGone,
    grok_root_acl, grok_residue_acl,
    observe_grok_auth, observe_grok_auth_candidate, grant_grok_home_root,
    grant_grok_auth_successor,
    observe_grok_recorded_auth, verify_grok_home_tree, verify_grok_auth, revoke_grok_home_root,
    revoke_grok_auth, inspect_grok_home_residue, revoke_grok_home_residue};
use crate::root::{RootIdentity, RootLock};
use crate::store::atomic::Statement;
use crate::store::digest::sha256_hex;
use crate::store::instance::{self, GrokGrant, GrokEffect, ResolvedDirectory};
use crate::store::same_open::VerifiedDatabaseConnection;
use std::sync::Mutex;

const RIGHTS: u32 = 0x0013_01bf;
const RESUME_CANDIDATE_GUARD_SQL:&str="SELECT 1 FROM main.gogoke_v37_h_claim h JOIN main.gogoke_coordination_process_custody oldc ON oldc.operation_id=h.process_operation_id AND oldc.profile_id=h.instance_id AND oldc.domain_id=h.domain_id AND oldc.generation=h.generation JOIN main.gogoke_v37_h_process_episode olde ON olde.process_operation_id=h.process_operation_id AND olde.instance_id=h.instance_id AND olde.domain_id=h.domain_id AND olde.session_id=h.session_id AND olde.generation=h.generation AND olde.binding_id=h.binding_id JOIN main.gogoke_v37_effective_seat b ON b.domain_id=h.domain_id AND b.session_id=h.session_id AND b.generation=h.generation JOIN main.gogoke_v37_seats s ON s.domain_id=b.domain_id AND s.seat_id=b.seat_id JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=h.domain_id AND e.session_id=h.session_id AND e.old_generation=h.generation WHERE h.domain_id=?1 AND h.session_id=?2 AND h.instance_id=?3 AND h.binding_id=?4 AND h.generation=?5 AND h.process_operation_id=?6 AND h.state='STOPPED' AND h.stop_fact_id IS NOT NULL AND h.stop_fact_id<>'' AND oldc.state='STOPPED' AND oldc.stop_proof_hash=h.stop_fact_id AND olde.phase='STOPPED' AND olde.stop_fact_id=h.stop_fact_id AND olde.seat_id=b.seat_id AND olde.seat_incarnation=b.seat_incarnation AND b.seat_id=?7 AND b.seat_incarnation=?8 AND s.incarnation=b.seat_incarnation AND s.generation=b.seat_authorization_generation AND s.instance_id=b.selected_instance_id AND b.selected_instance_id=h.instance_id AND s.state='BUSY' AND e.request_id=?9 AND e.generation=?10 AND e.instance_id=?3 AND e.binding_id=?11 AND e.seat_id=?7 AND e.seat_incarnation=?8 AND COALESCE(e.process_operation_id,'')=?12 AND e.phase IN ('INTENT','PREPARED','ACTIVE')";

fn evidence<T,E:std::fmt::Debug>(stage:&str,value:Result<T,E>)->Result<T,String>{
    value.map_err(|error|format!("Grok private HOME [{stage}]: {error:?}"))
}

fn effect(grant:&GrokGrant, action:&str, identity:&RootIdentity, relative:&str,
    before:&GrokAclSnapshot)->GrokEffect{
    let digest=sha256_hex(format!("{}\n{}\n{}\n{}",grant.binding_id,action,
        identity.opaque(),relative).as_bytes());
    GrokEffect{effect_id:format!("grok-acl-{}",&digest[..40]),
        binding_id:grant.binding_id.clone(),action:action.into(),
        object_identity:identity.clone(),relative_name:relative.into(),rights:RIGHTS,
        flags:if action.ends_with("ROOT"){3}else{0},
        before_aces:before.target_aces.clone(),
        after_aces:if action.starts_with("GRANT") {
            format!("1:{RIGHTS}:{}",if action.ends_with("ROOT"){3}else{0})
        }else{String::new()},
        before_control:before.dacl_control,
        // The existing Win32 ROOT setter applies automatic inheritance. Its
        // exact-target no-op does not write a descriptor or add this bit.
        after_control:if action=="GRANT_ROOT" && before.target_aces.is_empty() {
            before.dacl_control|0x0400
        }else{before.dacl_control},
        other_aces_sha256:sha256_hex(&before.other_aces_bytes()),
        phase:"INTENT".into(),revision:1}
}

fn apply(db:&mut VerifiedDatabaseConnection<'_>,expected:GrokEffect,
    observe:impl Fn()->Result<GrokAclSnapshot,String>,
    mutate:impl FnOnce()->Result<(),String>)->Result<(),String>{
    let intent=instance::begin_grok_effect(db,&expected)?;
    let before=observe()?;
    if intent.phase=="APPLIED" {
        return applied_readback(&intent,&before);
    }
    if before.identity!=intent.object_identity ||
        sha256_hex(&before.other_aces_bytes())!=intent.other_aces_sha256 {
        return Err("Grok private HOME: effect physical before/other ACE drift".into());
    }
    if before.target_aces==intent.after_aces &&
        before.dacl_control==intent.after_control {
        return instance::finish_grok_effect(db,&intent);
    }
    if before.target_aces!=intent.before_aces ||before.dacl_control!=intent.before_control {
        return Err("Grok private HOME: original ACL effect diverged".into());
    }
    mutate()?;
    let after=observe()?;
    if after.identity!=intent.object_identity ||after.target_aces!=intent.after_aces ||
        after.dacl_control!=intent.after_control ||
        !before.preserves_other_aces(&after) {
        return Err(format!("Grok private HOME: ACL effect readback diverged; action={}; before_control=0x{:04x}; expected_control=0x{:04x}; actual_control=0x{:04x}; identity_match={}; expected_target={:?}; actual_target={:?}; peer_bytes_preserved={}",
            intent.action, before.dacl_control, intent.after_control, after.dacl_control,
            after.identity==intent.object_identity, intent.after_aces, after.target_aces,
            before.preserves_other_aces(&after)));
    }
    instance::finish_grok_effect(db,&intent)
}

/// An APPLIED effect is historical. Later authorized peer-SID grants and
/// revocations can change its non-target ACE set, so replay checks the exact
/// target SID's final state and protected bit without issuing a second write.
fn applied_readback(intent:&GrokEffect,current:&GrokAclSnapshot)->Result<(),String>{
    if current.identity!=intent.object_identity ||current.target_aces!=intent.after_aces ||
        current.dacl_protected!=(intent.after_control & 0x1000 !=0) {
        return Err("Grok private HOME: APPLIED target SID or protection changed".into());
    }
    Ok(())
}

fn ordered_root_acl(snapshot:&GrokAclSnapshot)->String {
    snapshot.ordered_aces_bytes().iter().map(|byte|format!("{byte:02x}")).collect()
}

/// The ordered root anchor and the original ACL effect advance in one F
/// transaction. A crash after the OS write leaves INTENT and fails closed;
/// it cannot synthesize an APPLIED receipt from a later observation.
fn apply_root(db:&mut VerifiedDatabaseConnection<'_>,grant:&GrokGrant,expected:GrokEffect,
    observe:impl Fn()->Result<GrokAclSnapshot,String>,
    mutate:impl FnOnce()->Result<(),String>)->Result<(),String>{
    if expected.binding_id!=grant.binding_id ||
        expected.object_identity!=grant.home_identity {
        return Err("Grok private HOME: root effect grant changed".into());
    }
    let anchor=instance::read_grok_root_anchor(db,&grant.instance_id)?;
    let anchor=anchor.ok_or("Grok private HOME: ordered root anchor absent")?;
    let before=observe()?;
    if before.identity!=anchor.home_identity ||
        ordered_root_acl(&before)!=anchor.acl_hex ||
        before.dacl_control!=anchor.acl_control ||!before.canonical_dacl() {
        return Err("Grok private HOME: ordered root ACL differs from F anchor".into());
    }
    let intent=instance::begin_grok_effect(db,&expected)?;
    if intent.phase=="APPLIED" {
        // This is an immutable historical effect. A later peer ROOT effect
        // can advance the complete ordered anchor without changing this SID.
        return applied_readback(&intent,&before);
    }
    if before.identity!=intent.object_identity ||
        sha256_hex(&before.other_aces_bytes())!=intent.other_aces_sha256 ||
        before.target_aces!=intent.before_aces ||
        before.dacl_control!=intent.before_control {
        return Err("Grok private HOME: original root ACL effect diverged".into());
    }
    mutate()?;
    let after=observe()?;
    if after.identity!=intent.object_identity ||
        after.target_aces!=intent.after_aces ||
        after.dacl_control!=intent.after_control ||
        !before.preserves_other_aces(&after) ||!after.canonical_dacl() {
        return Err("Grok private HOME: ordered root ACL effect readback diverged".into());
    }
    instance::finish_grok_root_effect_with_anchor(db,&intent,&anchor,
        &ordered_root_acl(&after),after.dacl_control)
}

fn one(db:&VerifiedDatabaseConnection<'_>,instance_id:&str,binding_id:&str)->Result<GrokGrant,String>{
    instance::read_grok_grants(db,instance_id)?.into_iter().find(|r|r.binding_id==binding_id)
        .ok_or_else(||"Grok private HOME: original F grant absent".into())
}

fn exact_exists(db:&VerifiedDatabaseConnection<'_>,stage:&str,sql:&str,
    values:&[&str])->Result<bool,String>{
    let row=Statement::prepare(db.as_ptr(),sql).map_err(|e|format!("Grok private HOME {stage}: {e:?}"))?;
    for (i,value) in values.iter().enumerate(){row.bind_text(i as i32+1,value)
        .map_err(|e|format!("Grok private HOME {stage} bind: {e:?}"))?;}
    let found=row.step_row().map_err(|e|format!("Grok private HOME {stage} read: {e:?}"))?;
    if found && row.step_row().map_err(|e|format!("Grok private HOME {stage} duplicate: {e:?}"))? {
        return Err(format!("Grok private HOME {stage}: duplicate original witness"));
    }
    Ok(found)
}

/// The first ACL transition belongs to the exact unused H reservation and
/// current E seat. A resume candidate still has the old STOPPED H claim, so
/// its new generation is witnessed by the existing process-episode intent.
fn original_grant_admitted(db:&VerifiedDatabaseConnection<'_>,claim:&ClaimObservation,
    seat_id:&str,seat_incarnation:&str,request_id:&str)->Result<bool,String>{
    if claim.phase!=super::runtime::SessionPhase::Committed ||
        claim.process_operation_id.is_some() {return Ok(false);}
    let seat=exact_exists(db,"initial E seat",
        "SELECT 1 FROM main.gogoke_v37_effective_seat b JOIN main.gogoke_v37_seats s ON s.domain_id=b.domain_id AND s.seat_id=b.seat_id WHERE b.domain_id=?1 AND b.session_id=?2 AND b.seat_id=?3 AND b.seat_incarnation=?4 AND b.selected_instance_id=?5 AND s.incarnation=b.seat_incarnation AND s.generation=b.seat_authorization_generation AND s.instance_id=b.selected_instance_id AND s.state='BUSY'",
        &[&claim.domain_id,&claim.session_id,seat_id,seat_incarnation,&claim.instance_id])?;
    if !seat {return Ok(false);}
    if exact_exists(db,"initial H claim",
        "SELECT 1 FROM main.gogoke_v37_h_claim h JOIN main.gogoke_v37_effective_seat b ON b.domain_id=h.domain_id AND b.session_id=h.session_id AND b.generation=h.generation WHERE h.domain_id=?1 AND h.session_id=?2 AND h.instance_id=?3 AND h.binding_id=?4 AND h.generation=?5 AND h.home_id=?6 AND h.state='COMMITTED' AND h.process_operation_id IS NULL",
        &[&claim.domain_id,&claim.session_id,&claim.instance_id,&claim.binding_id,&claim.generation,&claim.home_id])? {
        return Ok(true);
    }
    exact_exists(db,"resume H candidate",
        "SELECT 1 FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_h_claim h ON h.domain_id=e.domain_id AND h.session_id=e.session_id AND h.generation=e.old_generation AND h.instance_id=e.instance_id JOIN main.gogoke_coordination_process_custody c ON c.operation_id=h.process_operation_id AND c.profile_id=h.instance_id AND c.domain_id=h.domain_id AND c.generation=h.generation JOIN main.gogoke_v37_h_process_episode olde ON olde.process_operation_id=h.process_operation_id AND olde.instance_id=h.instance_id AND olde.domain_id=h.domain_id AND olde.session_id=h.session_id AND olde.generation=h.generation AND olde.binding_id=h.binding_id JOIN main.gogoke_v37_effective_seat b ON b.domain_id=h.domain_id AND b.session_id=h.session_id AND b.generation=h.generation WHERE e.domain_id=?1 AND e.session_id=?2 AND e.instance_id=?3 AND e.binding_id=?4 AND e.generation=?5 AND e.request_id=?6 AND e.seat_id=?7 AND e.seat_incarnation=?8 AND e.home_id=?9 AND e.old_generation IS NOT NULL AND e.phase='INTENT' AND e.process_operation_id IS NULL AND h.state='STOPPED' AND h.stop_fact_id IS NOT NULL AND h.stop_fact_id<>'' AND c.state='STOPPED' AND c.stop_proof_hash=h.stop_fact_id AND olde.phase='STOPPED' AND olde.stop_fact_id=h.stop_fact_id AND olde.seat_id=b.seat_id AND olde.seat_incarnation=b.seat_incarnation AND b.seat_id=e.seat_id AND b.seat_incarnation=e.seat_incarnation",
        &[&claim.domain_id,&claim.session_id,&claim.instance_id,&claim.binding_id,&claim.generation,
            request_id,seat_id,seat_incarnation,&claim.home_id])
}

struct OriginalH {
    operation:String,ticket:String,nonce:String,pid:u32,creation:u64,image:String,
    digest:String,custody_state:String,custody_stop:Option<String>,
    claim_state:String,claim_stop:Option<String>,episode_state:String,
    episode_stop:Option<String>,
    candidate_before_promote:bool,
}

fn parse_original_row(row:&Statement,candidate_before_promote:bool)->Result<OriginalH,String>{
    let col=|i|row.column_text(i).map_err(|e|format!("Grok private HOME original H column: {e:?}"));
    let optional=|i|->Result<Option<String>,String>{let value=col(i)?;
        Ok(if value.is_empty(){None}else{Some(value)})};
    Ok(OriginalH{operation:col(0)?,ticket:col(1)?,nonce:col(2)?,
        pid:col(3)?.parse().map_err(|e|format!("Grok private HOME original pid: {e:?}"))?,
        creation:col(4)?.parse().map_err(|e|format!("Grok private HOME original creation: {e:?}"))?,
        image:col(5)?,digest:col(6)?,custody_state:col(7)?,custody_stop:optional(8)?,
        claim_state:col(9)?,claim_stop:optional(10)?,episode_state:col(11)?,
        episode_stop:optional(12)?,candidate_before_promote})
}

fn original_resume_candidate_h(db:&VerifiedDatabaseConnection<'_>,
    grant:&GrokGrant)->Result<Option<OriginalH>,String>{
    let row=Statement::prepare(db.as_ptr(),"SELECT c.operation_id,c.ticket,c.custodian_nonce,c.pid,c.creation_time_100ns,c.image_path,c.binary_digest_sha256,c.state,COALESCE(c.stop_proof_hash,''),oldh.state,COALESCE(oldh.stop_fact_id,''),e.phase,COALESCE(e.stop_fact_id,'') FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation JOIN main.gogoke_v37_h_claim oldh ON oldh.domain_id=e.domain_id AND oldh.session_id=e.session_id AND oldh.generation=e.old_generation AND oldh.instance_id=e.instance_id JOIN main.gogoke_coordination_process_custody oldc ON oldc.operation_id=oldh.process_operation_id AND oldc.profile_id=oldh.instance_id AND oldc.domain_id=oldh.domain_id AND oldc.generation=oldh.generation JOIN main.gogoke_v37_h_process_episode olde ON olde.process_operation_id=oldh.process_operation_id AND olde.instance_id=oldh.instance_id AND olde.domain_id=oldh.domain_id AND olde.session_id=oldh.session_id AND olde.generation=oldh.generation AND olde.binding_id=oldh.binding_id JOIN main.gogoke_v37_effective_seat s ON s.domain_id=oldh.domain_id AND s.session_id=oldh.session_id AND s.generation=oldh.generation WHERE e.binding_id=?1 AND e.instance_id=?2 AND e.domain_id=?3 AND e.session_id=?4 AND e.generation=?5 AND e.request_id=?6 AND e.seat_id=?7 AND e.seat_incarnation=?8 AND e.old_generation IS NOT NULL AND oldh.state='STOPPED' AND oldh.stop_fact_id IS NOT NULL AND oldh.stop_fact_id<>'' AND oldc.state='STOPPED' AND oldc.stop_proof_hash=oldh.stop_fact_id AND olde.phase='STOPPED' AND olde.stop_fact_id=oldh.stop_fact_id AND olde.seat_id=s.seat_id AND olde.seat_incarnation=s.seat_incarnation AND s.seat_id=e.seat_id AND s.seat_incarnation=e.seat_incarnation")
        .map_err(|e|format!("Grok private HOME original resume candidate: {e:?}"))?;
    let values:[&str;8]=[&grant.binding_id,&grant.instance_id,&grant.domain_id,
        &grant.session_id,&grant.generation,&grant.request_id,&grant.seat_id,&grant.seat_incarnation];
    for (i,value) in values.iter().enumerate(){row.bind_text(i as i32+1,value)
        .map_err(|e|format!("Grok private HOME original candidate bind: {e:?}"))?;}
    if !row.step_row().map_err(|e|format!("Grok private HOME original candidate read: {e:?}"))? {
        return Ok(None);
    }
    super::session_binding::authorization_generation(db,&grant.domain_id,&grant.session_id)
        .map_err(|error|format!("Grok resume relationship: {error:?}"))?;
    let result=parse_original_row(&row,true)?;
    if row.step_row().map_err(|e|format!("Grok private HOME original candidate duplicate: {e:?}"))? {
        return Err("Grok private HOME: duplicate original resume candidate".into());
    }
    Ok(Some(result))
}

fn original_h(db:&VerifiedDatabaseConnection<'_>,grant:&GrokGrant)->Result<Option<OriginalH>,String>{
    let row=Statement::prepare(db.as_ptr(),"SELECT c.operation_id,c.ticket,c.custodian_nonce,c.pid,c.creation_time_100ns,c.image_path,c.binary_digest_sha256,c.state,COALESCE(c.stop_proof_hash,''),h.state,COALESCE(h.stop_fact_id,''),e.phase,COALESCE(e.stop_fact_id,'') FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation JOIN main.gogoke_v37_h_claim h ON h.process_operation_id=c.operation_id AND h.instance_id=c.profile_id AND h.domain_id=c.domain_id AND h.generation=c.generation AND h.session_id=e.session_id AND h.binding_id=e.binding_id JOIN main.gogoke_v37_effective_seat s ON s.domain_id=h.domain_id AND s.session_id=h.session_id AND s.generation=h.generation AND s.seat_id=e.seat_id AND s.seat_incarnation=e.seat_incarnation WHERE e.binding_id=?1 AND e.instance_id=?2 AND e.domain_id=?3 AND e.session_id=?4 AND e.generation=?5 AND e.request_id=?6 AND e.seat_id=?7 AND e.seat_incarnation=?8")
        .map_err(|e|format!("Grok private HOME original H query: {e:?}"))?;
    let values:[&str;8]=[&grant.binding_id,&grant.instance_id,&grant.domain_id,
        &grant.session_id,&grant.generation,&grant.request_id,&grant.seat_id,&grant.seat_incarnation];
    for (i,value) in values.iter().enumerate(){row.bind_text(i as i32+1,value)
        .map_err(|e|format!("Grok private HOME original H bind: {e:?}"))?;}
    if !row.step_row().map_err(|e|format!("Grok private HOME original H read: {e:?}"))? {
        return original_resume_candidate_h(db,grant);
    }
    super::session_binding::authorization_generation(db,&grant.domain_id,&grant.session_id)
        .map_err(|error|format!("Grok original relationship: {error:?}"))?;
    let result=parse_original_row(&row,false)?;
    if row.step_row().map_err(|e|format!("Grok private HOME original H duplicate: {e:?}"))? {
        return Err("Grok private HOME: duplicate original H tuple".into());
    }
    Ok(Some(result))
}

fn matches_bound_original(grant:&GrokGrant,h:&OriginalH)->bool{
    grant.process_operation_id.as_deref()==Some(h.operation.as_str()) &&
    grant.ticket.as_deref()==Some(h.ticket.as_str()) &&grant.custodian_nonce.as_deref()==Some(h.nonce.as_str()) &&
    grant.pid==Some(h.pid) &&grant.creation_time_100ns==Some(h.creation) &&
    grant.image_path.as_deref()==Some(h.image.as_str()) && grant.program_digest==h.digest
}

fn holder_gone_original_eligible(h:&OriginalH)->bool{
    h.custody_stop.is_none() &&h.episode_stop.is_none() &&
    matches!(h.custody_state.as_str(),"PREPARED"|"ACTIVE"|"UNKNOWN") &&
    matches!(h.episode_state.as_str(),"PREPARED"|"ACTIVE"|"UNKNOWN") &&
    (if h.candidate_before_promote {
        h.claim_state=="STOPPED" &&h.claim_stop.as_deref().is_some_and(|v|!v.is_empty())
    } else {
        matches!(h.claim_state.as_str(),"COMMITTED"|"UNKNOWN") &&h.claim_stop.is_none()
    })
}

fn original_no_attempt_has_process(db:&VerifiedDatabaseConnection<'_>,
    grant:&GrokGrant)->Result<bool,String>{
    let episode=Statement::prepare(db.as_ptr(),"SELECT 1 FROM main.gogoke_v37_h_process_episode WHERE binding_id=?1 AND instance_id=?2 AND domain_id=?3 AND session_id=?4 AND generation=?5 AND request_id=?6 AND seat_id=?7 AND seat_incarnation=?8 LIMIT 1")
        .map_err(|e|format!("Grok private HOME NoAttempt H episode: {e:?}"))?;
    let values:[&str;8]=[&grant.binding_id,&grant.instance_id,&grant.domain_id,
        &grant.session_id,&grant.generation,&grant.request_id,&grant.seat_id,
        &grant.seat_incarnation];
    for (i,value) in values.iter().enumerate(){episode.bind_text(i as i32+1,value)
        .map_err(|e|format!("Grok private HOME NoAttempt H bind: {e:?}"))?;}
    if episode.step_row().map_err(|e|format!("Grok private HOME NoAttempt H read: {e:?}"))? {
        return Ok(true);
    }
    let custody=Statement::prepare(db.as_ptr(),"SELECT 1 FROM main.gogoke_coordination_process_custody c JOIN main.gogoke_v37_h_claim h ON h.process_operation_id=c.operation_id AND h.instance_id=c.profile_id AND h.domain_id=c.domain_id AND h.generation=c.generation JOIN main.gogoke_v37_effective_seat s ON s.domain_id=h.domain_id AND s.session_id=h.session_id AND s.generation=h.generation WHERE h.binding_id=?1 AND h.instance_id=?2 AND h.domain_id=?3 AND h.session_id=?4 AND h.generation=?5 AND s.seat_id=?6 AND s.seat_incarnation=?7 LIMIT 1")
        .map_err(|e|format!("Grok private HOME NoAttempt original custody: {e:?}"))?;
    for (i,value) in [&grant.binding_id,&grant.instance_id,&grant.domain_id,
        &grant.session_id,&grant.generation,&grant.seat_id,&grant.seat_incarnation].iter().enumerate(){
        custody.bind_text(i as i32+1,value).map_err(|e|format!("Grok private HOME NoAttempt custody bind: {e:?}"))?;
    }
    custody.step_row().map_err(|e|format!("Grok private HOME NoAttempt custody read: {e:?}"))
}

fn recorded_auth(db:&VerifiedDatabaseConnection<'_>,grant:&GrokGrant)->Result<Vec<RootIdentity>,String>{
    let mut ids=vec![grant.auth_identity.clone()];
    for effect in instance::read_grok_effects(db,&grant.binding_id)? {
        if effect.action=="GRANT_AUTH" && effect.phase=="APPLIED" &&
            !ids.contains(&effect.object_identity) {ids.push(effect.object_identity);}
    }
    Ok(ids)
}

fn has_original_root_source(effects:&[GrokEffect],home:&RootIdentity)->bool{
    effects.iter().any(|effect|effect.action=="GRANT_ROOT" &&effect.phase=="APPLIED" &&
        effect.object_identity==*home &&effect.rights==RIGHTS &&effect.flags==3 &&
        effect.after_aces==format!("1:{RIGHTS}:3"))
}

/// A peer's durable auth grant can preserve this original SID's natural HOME
/// inheritance on the same physical successor. Qualify that exact FileID and
/// final peer ACL effect before permitting its separate residue revocation.
fn proven_inherited_successors(db:&VerifiedDatabaseConnection<'_>,grant:&GrokGrant,
    profile:&AppContainerProfile,home:&ResolvedDirectory)->Result<Vec<RootIdentity>,String>{
    let own_effects=instance::read_grok_effects(db,&grant.binding_id)?;
    if !has_original_root_source(&own_effects,&home.identity) {
        return Err("Grok private HOME: original ROOT source absent for inherited successor".into());
    }
    let own_ids=recorded_auth(db,grant)?;
    let mut ids=Vec::new();
    for peer in instance::read_grok_grants(db,&grant.instance_id)? {
        if peer.binding_id==grant.binding_id {continue;}
        if peer.home_identity!=home.identity ||peer.program_digest!=grant.program_digest {
            return Err("Grok private HOME: inherited successor peer domain changed".into());
        }
        let peer_profile=evidence("inherited-successor-peer-SID",
            AppContainerProfile::derive_for_revocation(&peer.profile_name))?;
        if evidence("inherited-successor-peer-SID-readback",peer_profile.sid_identity())?!=peer.profile_sid {
            return Err("Grok private HOME: inherited successor peer SID changed".into());
        }
        let effects=instance::read_grok_effects(db,&peer.binding_id)?;
        for source in effects.iter().filter(|effect|effect.action=="GRANT_AUTH" &&
            effect.phase=="APPLIED" &&effect.relative_name=="auth.json" &&
            effect.rights==RIGHTS &&effect.flags==0 &&
            effect.after_aces==format!("1:{RIGHTS}:0") &&
            !own_ids.contains(&effect.object_identity) &&!ids.contains(&effect.object_identity)) {
            let Some(auth)=evidence("inherited-successor-FileID",observe_grok_recorded_auth(
                &home.path,&home.identity,&source.object_identity))? else {continue;};
            let acl=evidence("inherited-successor-ACL",auth.acl(profile))?;
            if !acl.canonical_dacl() ||acl.target_aces!=format!("1:{RIGHTS}:16") {
                continue;
            }
            let peer_acl=evidence("inherited-successor-peer-ACL",auth.acl(&peer_profile))?;
            if peer_acl.identity!=acl.identity ||
                peer_acl.ordered_aces_bytes()!=acl.ordered_aces_bytes() ||
                peer_acl.dacl_control!=acl.dacl_control {
                return Err("Grok private HOME: inherited successor ACL changed during proof".into());
            }
            let final_effect=if matches!(peer.phase.as_str(),"GRANTED_UNCREATED"|"ACTIVE") {
                source
            } else if matches!(peer.phase.as_str(),"RETIRED_CLEANUP_PENDING"|"REVOKED") {
                effects.iter().find(|effect|effect.action=="REVOKE_AUTH" &&
                    effect.phase=="APPLIED" &&effect.object_identity==acl.identity)
                    .ok_or("Grok private HOME: inherited successor peer revoke absent")?
            } else {
                return Err("Grok private HOME: inherited successor peer lifecycle unresolved".into());
            };
            if peer_acl.target_aces!=final_effect.after_aces ||
                peer_acl.dacl_control!=final_effect.after_control ||
                sha256_hex(&peer_acl.other_aces_bytes())!=final_effect.other_aces_sha256 {
                // A later authorized peer may have changed the other-ACE set.
                // Only a final effect matching the current complete ACL proves
                // this FileID; the domain scan rejects it if none matches.
                continue;
            }
            ids.push(acl.identity);
        }
    }
    Ok(ids)
}

fn verify_active_root_authority(db:&VerifiedDatabaseConnection<'_>,grant:&GrokGrant,
    profile:&AppContainerProfile,home:&ResolvedDirectory)->Result<(),String>{
    let domain=instance::current_grok_home_domain(db,&grant.instance_id)?;
    let anchor=instance::read_grok_root_anchor(db,&grant.instance_id)?
        .ok_or("Grok private HOME: active ordered root anchor absent")?;
    if domain.root_identity!=*db.root_identity() ||domain.home_identity!=home.identity ||
        domain.program_digest!=grant.program_digest ||domain.version!="1.0.41" ||
        anchor.root_identity!=domain.root_identity ||anchor.home_identity!=domain.home_identity ||
        anchor.program_digest!=domain.program_digest ||anchor.version!=domain.version ||
        // Registration observation can move forward for the same exact
        // physical HOME/program. The authorized out-of-transaction refresh
        // advances both F domain and anchor by CAS; a pure verify can read the
        // newer catalog revision without mutating the record.
        anchor.registration_revision>domain.registration_revision ||
        anchor.source_effect_id.is_empty() {
        return Err("Grok private HOME: active F root anchor provenance changed".into());
    }
    let current=evidence("active-root-ACL",grok_root_acl(profile,&home.path,&home.identity))?;
    if ordered_root_acl(&current)!=anchor.acl_hex ||
        current.dacl_control!=anchor.acl_control ||!current.canonical_dacl() {
        return Err("Grok private HOME: complete ordered root ACL changed".into());
    }
    let grants=instance::read_grok_grants(db,&grant.instance_id)?;
    let mut expected_sids=Vec::new();
    let mut source_count=0usize;
    for peer in grants {
        if peer.home_identity!=home.identity ||
            peer.program_digest!=grant.program_digest ||
            peer.profile_name.is_empty() {
            return Err("Grok private HOME: F peer registration changed".into());
        }
        let sid=evidence("peer-SID",AppContainerProfile::derive_for_revocation(
            &peer.profile_name))?;
        let sid_text=evidence("peer-SID-readback",sid.sid_identity())?;
        if sid_text!=peer.profile_sid {
            return Err("Grok private HOME: F peer SID changed".into());
        }
        let effects=instance::read_grok_effects(db,&peer.binding_id)?;
        source_count+=effects.iter().filter(|effect|
            effect.effect_id==anchor.source_effect_id &&
            effect.phase=="APPLIED" &&
            matches!(effect.action.as_str(),"GRANT_ROOT"|"REVOKE_ROOT") &&
            effect.object_identity==home.identity &&
            effect.after_control==anchor.acl_control).count();
        let peer_root=evidence("peer-root-ACL",grok_root_acl(&sid,&home.path,&home.identity))?;
        if ordered_root_acl(&peer_root)!=anchor.acl_hex ||
            peer_root.dacl_control!=anchor.acl_control {
            return Err("Grok private HOME: F peer sees different root ACL".into());
        }
        match peer.phase.as_str() {
            "GRANTED_UNCREATED" => {
                if !exact_exists(db,"uncreated peer H/E guard",
                    "SELECT 1 FROM main.gogoke_v37_h_claim h JOIN main.gogoke_v37_effective_seat b ON b.domain_id=h.domain_id AND b.session_id=h.session_id AND b.generation=h.generation JOIN main.gogoke_v37_seats s ON s.domain_id=b.domain_id AND s.seat_id=b.seat_id JOIN main.gogoke_v37_instance_homes f ON f.home_id=h.home_id AND f.instance_id=h.instance_id AND f.domain_id=h.domain_id AND f.kind='SESSION' AND f.owner_id=h.session_id AND f.generation=h.generation AND f.state='ACTIVE' WHERE h.domain_id=?1 AND h.session_id=?2 AND h.instance_id=?3 AND h.binding_id=?4 AND h.generation=?5 AND h.state='COMMITTED' AND h.process_operation_id IS NULL AND b.seat_id=?6 AND b.seat_incarnation=?7 AND b.selected_instance_id=h.instance_id AND s.incarnation=b.seat_incarnation AND s.generation=b.seat_authorization_generation AND s.instance_id=h.instance_id AND s.state='BUSY'",
                    &[&peer.domain_id,&peer.session_id,&peer.instance_id,&peer.binding_id,
                        &peer.generation,&peer.seat_id,&peer.seat_incarnation])? {
                    return Err("Grok private HOME: uncreated peer H/E authority absent".into());
                }
                expected_sids.push(peer.profile_sid.clone());
            },
            "ACTIVE" => {
                let h=original_h(db,&peer)?.ok_or("Grok private HOME: active peer H absent")?;
                let claim_matches=if h.candidate_before_promote {
                    h.claim_state=="STOPPED" &&
                    h.claim_stop.as_deref().is_some_and(|stop|!stop.is_empty())
                } else {
                    h.claim_state=="COMMITTED" &&h.claim_stop.is_none()
                };
                if !matches_bound_original(&peer,&h) ||
                    !matches!(h.custody_state.as_str(),"PREPARED"|"ACTIVE") ||
                    !matches!(h.episode_state.as_str(),"INTENT"|"PREPARED"|"ACTIVE") ||
                    !claim_matches ||h.custody_stop.is_some() ||h.episode_stop.is_some() {
                    return Err("Grok private HOME: active peer H custody changed".into());
                }
                expected_sids.push(peer.profile_sid.clone());
            },
            "RETIRED_CLEANUP_PENDING"|"REVOKED" => {},
            _ => return Err("Grok private HOME: F peer ACL transition unresolved".into()),
        }
        let expected=if matches!(peer.phase.as_str(),"GRANTED_UNCREATED"|"ACTIVE") {
            format!("1:{RIGHTS}:3")
        } else {String::new()};
        if peer_root.target_aces!=expected {
            return Err("Grok private HOME: F peer root ACE differs from authority".into());
        }
    }
    if source_count!=1 {
        return Err("Grok private HOME: root anchor lacks one APPLIED source effect".into());
    }
    expected_sids.sort();
    let mut actual=current.package_sid_aces().to_vec();
    actual.sort();
    if actual!=expected_sids {
        return Err("Grok private HOME: root ACL has unknown or missing package SID".into());
    }
    Ok(())
}

fn verify_held_auth_authority(db:&VerifiedDatabaseConnection<'_>,grant:&GrokGrant,
    profile:&AppContainerProfile,held:&[GrokAuthMetadata])->Result<(),String>{
    let peers=instance::read_grok_grants(db,&grant.instance_id)?;
    for auth in held {
        let original=evidence("held-auth-ACL",auth.acl(profile))?;
        if !original.canonical_dacl() {
            return Err("Grok private HOME: held auth DACL order changed".into());
        }
        let original_bytes=original.ordered_aces_bytes();
        let mut expected_sids=Vec::new();
        let mut whole_effect=false;
        for peer in &peers {
            let peer_profile=evidence("held-auth-peer-SID",
                AppContainerProfile::derive_for_revocation(&peer.profile_name))?;
            if evidence("held-auth-peer-SID-readback",peer_profile.sid_identity())?!=peer.profile_sid {
                return Err("Grok private HOME: held auth peer SID changed".into());
            }
            let acl=evidence("held-auth-peer-ACL",auth.acl(&peer_profile))?;
            if acl.identity!=auth.identity ||acl.ordered_aces_bytes()!=original_bytes ||
                acl.dacl_control!=original.dacl_control {
                return Err("Grok private HOME: held auth peer ACL changed during proof".into());
            }
            let effects=instance::read_grok_effects(db,&peer.binding_id)?;
            let grant_effect=effects.iter().find(|effect|effect.action=="GRANT_AUTH" &&
                effect.phase=="APPLIED" &&effect.object_identity==auth.identity);
            let revoke_effect=effects.iter().find(|effect|effect.action=="REVOKE_AUTH" &&
                effect.phase=="APPLIED" &&effect.object_identity==auth.identity);
            let final_effect=match (grant_effect,revoke_effect,peer.phase.as_str()) {
                (None,None,_) if acl.target_aces.is_empty()=>None,
                (None,None,_) if acl.target_aces==format!("1:{RIGHTS}:16") &&
                    has_original_root_source(&effects,&grant.home_identity) => {
                    expected_sids.push(peer.profile_sid.clone());None
                },
                (Some(effect),None,"GRANTED_UNCREATED"|"ACTIVE")
                    if acl.target_aces==effect.after_aces => {
                    expected_sids.push(peer.profile_sid.clone());Some(effect)
                },
                (Some(_),Some(effect),"RETIRED_CLEANUP_PENDING"|"REVOKED")
                    if acl.target_aces==effect.after_aces=>Some(effect),
                _=>return Err("Grok private HOME: held auth peer effect lifecycle changed".into()),
            };
            if let Some(effect)=final_effect {
                if effect.after_control==acl.dacl_control &&
                    effect.other_aces_sha256==sha256_hex(&acl.other_aces_bytes()) {
                    whole_effect=true;
                }
            }
        }
        expected_sids.sort();
        let mut actual=original.package_sid_aces().to_vec();
        actual.sort();
        if actual!=expected_sids ||!whole_effect {
            return Err("Grok private HOME: held auth has unknown ACL evolution".into());
        }
    }
    Ok(())
}

pub(crate) struct GrokHomeLaunch {
    instance_id:String, binding_id:String, home:ResolvedDirectory,
    /// Includes deleted/replaced old objects. Every handle shares DELETE.
    auth:Mutex<Vec<GrokAuthMetadata>>,
}

impl GrokHomeLaunch {
    pub(crate) fn prepare(db:&mut VerifiedDatabaseConnection<'_>,root:&RootLock,
        profile:&AppContainerProfile,profile_name:&str,claim:&ClaimObservation,
        pin:&InstancePin,home:&ResolvedDirectory,seat_id:&str,seat_incarnation:&str,
        request_id:&str)->Result<Self,String>{
        if pin.driver_id!="grok" || pin.version!="1.0.41" {
            return Err("Grok private HOME: fixed pin absent".into());
        }
        instance::initialize_grok_home_grant_schema(db)?;
        let domain=instance::current_grok_home_domain(db,&claim.instance_id)?;
        if domain.root_identity!=*db.root_identity() ||domain.home_identity!=home.identity ||
            domain.program_digest!=pin.digest ||domain.version!=pin.version {
            return Err("Grok private HOME: F domain/pin differs from H".into());
        }
        if !original_grant_admitted(db,claim,seat_id,seat_incarnation,request_id)? {
            return Err("Grok private HOME: original H/E grant authority changed".into());
        }
        let auth=evidence("observe-original-auth-metadata",
            observe_grok_auth_candidate(&home.path,&home.identity))?;
        let original_root=evidence("root-ACL-original",
            grok_root_acl(profile,&home.path,&home.identity))?;
        if !original_root.canonical_dacl() {
            return Err("Grok private HOME: original root DACL is not canonical".into());
        }
        if instance::read_grok_root_anchor(db,&claim.instance_id)?.is_none() &&
            !original_root.package_sid_aces().is_empty() {
            return Err("Grok private HOME: first ordered root baseline has unknown package SID".into());
        }
        let grant=GrokGrant{binding_id:claim.binding_id.clone(),instance_id:claim.instance_id.clone(),
            domain_id:claim.domain_id.clone(),session_id:claim.session_id.clone(),seat_id:seat_id.into(),
            seat_incarnation:seat_incarnation.into(),generation:claim.generation.clone(),request_id:request_id.into(),
            profile_name:profile_name.into(),profile_sid:evidence("profile-SID",profile.sid_identity())?,
            program_digest:pin.digest.clone(),
            home_identity:home.identity.clone(),auth_identity:auth.identity.clone(),
            phase:"GRANT_PENDING".into(),process_operation_id:None,ticket:None,
            custodian_nonce:None,pid:None,creation_time_100ns:None,image_path:None,
            stop_fact_id:None,revision:1};
        let grant=instance::begin_grok_grant_with_root_anchor(db,&domain,&grant,
            &ordered_root_acl(&original_root),original_root.dacl_control,
            &sha256_hex(&original_root.other_aces_bytes()))?;
        if grant.phase!="GRANT_PENDING" {
            return Err("Grok private HOME: original grant already advanced".into());
        }
        let result=Self{instance_id:claim.instance_id.clone(),binding_id:claim.binding_id.clone(),
            home:home.clone(),auth:Mutex::new(vec![auth])};
        let prepared=(||{
            let root_before=evidence("root-ACL-before",grok_root_acl(profile,&home.path,&home.identity))?;
            apply_root(db,&grant,effect(&grant,"GRANT_ROOT",&home.identity,".",&root_before),
                ||evidence("root-ACL-readback",grok_root_acl(profile,&home.path,&home.identity)),||
                evidence("grant-root",grant_grok_home_root(profile,&home.path,&home.identity)))?;
            let held=result.auth.lock().map_err(|_|"Grok private HOME: auth custody poisoned")?;
            let auth_before=evidence("auth-ACL-before",held[0].candidate_acl(profile))?;
            let mut intent=effect(&grant,"GRANT_AUTH",&held[0].identity,"auth.json",&auth_before);
            intent.after_control=auth_before.dacl_control|0x1000;
            apply(db,intent,
                ||evidence("auth-ACL-readback",held[0].candidate_acl(profile)),||
                evidence("grant-auth",grant_grok_auth_successor(profile,&held[0])))?;
            evidence("verify-preactivation",verify_grok_home_tree(profile,&home.path,
                &home.identity,&held[0],&[held[0].identity.clone()]))?;
            drop(held);
            instance::set_grok_grant_phase(db,&grant,"GRANTED_UNCREATED",None)
        })();
        if let Err(original)=prepared {
            let cleanup=result.revoke_uncreated(db,root,profile);
            return Err(format!("{original}; no-attempt settlement: {cleanup:?}"));
        }
        Ok(result)
    }

    fn grant(&self,db:&VerifiedDatabaseConnection<'_>)->Result<GrokGrant,String>{
        one(db,&self.instance_id,&self.binding_id)
    }

    /// Pure read. H may call this inside its current transaction; a changed
    /// auth FileID requires an explicit out-of-transaction refresh first.
    pub(crate) fn verify(&self,db:&VerifiedDatabaseConnection<'_>,profile:&AppContainerProfile,
        full_tree:bool)->Result<(),String>{
        let grant=self.grant(db)?;
        if !matches!(grant.phase.as_str(),"GRANTED_UNCREATED"|"ACTIVE") ||
            grant.profile_sid!=evidence("profile-SID",profile.sid_identity())? ||
            grant.home_identity!=self.home.identity {
            return Err("Grok private HOME: current F grant unavailable".into());
        }
        let held=self.auth.lock().map_err(|_|"Grok private HOME: auth custody poisoned")?;
        if held.is_empty() {return Err("Grok private HOME: metadata custody absent".into());}
        if grant.phase=="ACTIVE" {
            verify_active_root_authority(db,&grant,profile,&self.home)?;
            verify_held_auth_authority(db,&grant,profile,&held)?;
            // The fixed Grok CLI atomically replaces auth.json without H/F
            // coordination. The stable original HOME and complete ordered
            // root ACL remain the active authority. Old held auth FileIDs
            // retain exact retirement custody; no replacement is granted.
            return Ok(());
        }
        let current=evidence("observe-current-auth",observe_grok_auth(&self.home.path,&self.home.identity))?;
        let ids=recorded_auth(db,&grant)?;
        if !ids.contains(&current.identity) {
            return Err("Grok private HOME: auth successor needs durable refresh".into());
        }
        evidence("verify-current-auth",verify_grok_auth(profile,&current))?;
        if full_tree {
            evidence("verify-tree",verify_grok_home_tree(profile,&self.home.path,
                &self.home.identity,&current,&ids))?;
        } else {
            evidence("verify-root",profile.verify_bound_directory_grant(&self.home.path,
                &self.home.identity,true))?;
        }
        // Old held objects keep exact custody; they may have been deleted.
        Ok(())
    }

    /// Explicit H safe boundary only, never called from verify/verify_live.
    /// An atomic CLI replacement can rotate FileID inside the same F HOME.
    pub(crate) fn refresh_readiness(&self,db:&mut VerifiedDatabaseConnection<'_>,
        profile:&AppContainerProfile,custody:Option<&PreparedCustody>,
        resume:Option<(&ClaimObservation,&str)>)->Result<(),String>{
        let grant=self.grant(db)?;
        if !matches!(grant.phase.as_str(),"GRANTED_UNCREATED"|"ACTIVE") {
            return Err("Grok private HOME: no live original grant".into());
        }
        match (grant.phase.as_str(),custody) {
            ("GRANTED_UNCREATED",None)=>{},
            ("ACTIVE",Some(c)) if grant.ticket.as_deref()==Some(c.ticket.opaque()) &&
                grant.custodian_nonce.as_deref()==Some(&c.custodian_nonce) &&
                grant.pid==Some(c.identity.pid) &&
                grant.creation_time_100ns==Some(c.identity.creation_time_100ns) &&
                grant.image_path.as_deref()==Some(c.identity.image_path.to_string_lossy().as_ref()) &&
                grant.program_digest==c.binding.binary_digest_sha256 &&
                grant.instance_id==c.binding.profile_id &&
                grant.domain_id==c.binding.domain_id &&
                grant.generation==c.binding.generation =>{},
            _=>return Err("Grok private HOME: refresh original custody missing or changed".into()),
        }
        let current=instance::current_grok_home_domain(db,&grant.instance_id)?;
        if current.root_identity!=*db.root_identity() ||current.home_identity!=self.home.identity ||
            current.program_digest!=grant.program_digest ||current.version!="1.0.41" {
            return Err("Grok private HOME: current F registration changed".into());
        }
        let operation=grant.process_operation_id.as_deref().unwrap_or("");
        let h_current=if let Some((old,resume_request))=resume {
            if old.domain_id!=grant.domain_id ||old.session_id!=grant.session_id ||
                old.instance_id!=grant.instance_id ||old.generation==grant.generation ||
                old.process_operation_id.is_none() ||old.phase!=super::runtime::SessionPhase::Stopped ||
                resume_request!=grant.request_id {
                return Err("Grok private HOME: original stopped resume admission changed".into());
            }
            exact_exists(db,"resume candidate guard",
                RESUME_CANDIDATE_GUARD_SQL,
                &[&grant.domain_id,&grant.session_id,&grant.instance_id,&old.binding_id,
                    &old.generation,old.process_operation_id.as_deref().unwrap_or(""),
                    &grant.seat_id,&grant.seat_incarnation,resume_request,&grant.generation,
                    &grant.binding_id,operation])?
        } else {
            exact_exists(db,"current H guard",
                "SELECT 1 FROM main.gogoke_v37_h_claim h JOIN main.gogoke_v37_effective_seat b ON b.domain_id=h.domain_id AND b.session_id=h.session_id AND b.generation=h.generation JOIN main.gogoke_v37_seats s ON s.domain_id=b.domain_id AND s.seat_id=b.seat_id WHERE h.domain_id=?1 AND h.session_id=?2 AND h.binding_id=?3 AND h.instance_id=?4 AND h.generation=?5 AND h.state='COMMITTED' AND COALESCE(h.process_operation_id,'')=?6 AND b.seat_id=?7 AND b.seat_incarnation=?8 AND s.incarnation=b.seat_incarnation AND s.generation=b.seat_authorization_generation AND s.instance_id=b.selected_instance_id AND b.selected_instance_id=h.instance_id AND s.state='BUSY'",
                &[&grant.domain_id,&grant.session_id,&grant.binding_id,&grant.instance_id,
                    &grant.generation,operation,&grant.seat_id,&grant.seat_incarnation])?
        };
        if !h_current {return Err("Grok private HOME: original H/E authority changed".into());}
        if grant.phase=="ACTIVE" {
            let c=custody.ok_or("Grok private HOME: current original custody absent")?;
            let pid=c.identity.pid.to_string();
            let creation=c.identity.creation_time_100ns.to_string();
            let image=c.identity.image_path.to_string_lossy().into_owned();
            if !exact_exists(db,"active custody guard",
                "SELECT 1 FROM main.gogoke_coordination_process_custody WHERE operation_id=?1 AND ticket=?2 AND custodian_nonce=?3 AND profile_id=?4 AND domain_id=?5 AND generation=?6 AND pid=?7 AND creation_time_100ns=?8 AND image_path=?9 AND binary_digest_sha256=?10 AND state IN ('PREPARED','ACTIVE') AND stop_proof_hash IS NULL",
                &[operation,c.ticket.opaque(),&c.custodian_nonce,&grant.instance_id,&grant.domain_id,
                    &grant.generation,&pid,&creation,&image,&grant.program_digest])? {
                return Err("Grok private HOME: original active custody changed".into());
            }
            instance::advance_grok_root_anchor_registration(db,&grant.instance_id)?;
            return verify_active_root_authority(db,&grant,profile,&self.home);
        }
        let auth=evidence("observe-successor-metadata",observe_grok_auth_candidate(&self.home.path,&self.home.identity))?;
        let ids=recorded_auth(db,&grant)?;
        if !ids.contains(&auth.identity) {
            let before=evidence("successor-ACL-before",auth.candidate_acl(profile))?;
            let mut intent=effect(&grant,"GRANT_AUTH",&auth.identity,"auth.json",&before);
            // The physical successor is first qualified by the original H/F
            // guard above. The protected-bit transition is part of the same
            // durable effect, including an ACL-applied/finish-not-yet-written crash.
            intent.after_control=before.dacl_control|0x1000;
            apply(db,intent,||evidence("successor-ACL-readback",auth.candidate_acl(profile)),||
                evidence("grant-successor",grant_grok_auth_successor(profile,&auth)))?;
        } else {evidence("verify-successor",verify_grok_auth(profile,&auth))?;}
        let mut held=self.auth.lock().map_err(|_|"Grok private HOME: auth custody poisoned")?;
        if !held.iter().any(|old|old.identity==auth.identity) {held.push(auth);}
        Ok(())
    }

    pub(crate) fn bind_process(&self,db:&mut VerifiedDatabaseConnection<'_>,
        operation:&str,custody:&PreparedCustody)->Result<(),String>{
        let grant=self.grant(db)?;
        if custody.binding.profile_id!=self.instance_id ||
            custody.binding.domain_id!=grant.domain_id ||
            custody.binding.generation!=grant.generation ||
            custody.binding.binary_digest_sha256!=grant.program_digest {
            return Err("Grok private HOME: original H custody mismatch".into());
        }
        instance::bind_grok_original_process(db,&grant,operation,custody.ticket.opaque(),
            &custody.custodian_nonce,custody.identity.pid,
            custody.identity.creation_time_100ns,&custody.identity.image_path.to_string_lossy())
    }

    pub(crate) fn revoke_uncreated(&self,db:&mut VerifiedDatabaseConnection<'_>,
        _root:&RootLock,profile:&AppContainerProfile)->Result<(),String>{
        let grant=self.grant(db)?;
        if !matches!(grant.phase.as_str(),"GRANT_PENDING"|"GRANTED_UNCREATED"|"REVOKE_PENDING") ||
            grant.process_operation_id.is_some() {
            return Err("Grok private HOME: no-attempt proof does not match F".into());
        }
        self.revoke_effects(db,profile,&grant,None,None)
    }

    pub(crate) fn revoke_stopped(&self,db:&mut VerifiedDatabaseConnection<'_>,
        _root:&RootLock,profile:&AppContainerProfile,operation:&str,
        custody:&PreparedCustody)->Result<(),String>{
        let grant=self.grant(db)?;
        if grant.process_operation_id.as_deref()!=Some(operation) ||
            grant.ticket.as_deref()!=Some(custody.ticket.opaque()) ||
            grant.custodian_nonce.as_deref()!=Some(&custody.custodian_nonce) ||
            grant.pid!=Some(custody.identity.pid) ||
            grant.creation_time_100ns!=Some(custody.identity.creation_time_100ns) ||
            grant.program_digest!=custody.binding.binary_digest_sha256 ||
            grant.image_path.as_deref()!=Some(custody.identity.image_path.to_string_lossy().as_ref()) {
            return Err("Grok private HOME: original H process tuple changed".into());
        }
        let row=Statement::prepare(db.as_ptr(),"SELECT c.stop_proof_hash FROM main.gogoke_coordination_process_custody c JOIN main.gogoke_v37_h_claim h ON h.process_operation_id=c.operation_id AND h.domain_id=c.domain_id AND h.generation=c.generation JOIN main.gogoke_v37_effective_seat s ON s.domain_id=h.domain_id AND s.session_id=h.session_id AND s.generation=h.generation JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=c.operation_id AND e.instance_id=c.profile_id AND e.domain_id=c.domain_id AND e.generation=c.generation AND e.session_id=h.session_id AND e.binding_id=h.binding_id AND e.seat_id=s.seat_id AND e.seat_incarnation=s.seat_incarnation WHERE c.operation_id=?1 AND c.ticket=?2 AND c.custodian_nonce=?3 AND c.domain_id=?4 AND c.generation=?5 AND c.profile_id=?6 AND c.binary_digest_sha256=?7 AND c.pid=?8 AND c.creation_time_100ns=?9 AND c.image_path=?10 AND h.binding_id=?11 AND h.instance_id=?6 AND h.session_id=?12 AND s.seat_incarnation=?13 AND s.seat_id=?14 AND e.request_id=?15 AND c.state='STOPPED' AND c.stop_proof_hash IS NOT NULL AND c.stop_proof_hash=h.stop_fact_id AND h.state='STOPPED'")
            .map_err(|e|format!("Grok private HOME stop custody: {e:?}"))?;
        let pid=custody.identity.pid.to_string();
        let creation=custody.identity.creation_time_100ns.to_string();
        let image=custody.identity.image_path.to_string_lossy().into_owned();
        let values:[&str;15]=[operation,custody.ticket.opaque(),&custody.custodian_nonce,
            &grant.domain_id,&grant.generation,&grant.instance_id,&grant.program_digest,
            &pid,&creation,&image,&grant.binding_id,&grant.session_id,&grant.seat_incarnation,
            &grant.seat_id,&grant.request_id];
        for (i,value) in values.iter().enumerate(){
            row.bind_text(i as i32+1,value).map_err(|e|format!("Grok private HOME stop bind: {e:?}"))?;
        }
        if !row.step_row().map_err(|e|format!("Grok private HOME stop read: {e:?}"))? {
            return Err("Grok private HOME: original StopFact absent".into());
        }
        let stop=row.column_text(0).map_err(|e|format!("Grok private HOME stop proof: {e:?}"))?;
        if stop.is_empty() ||row.step_row().map_err(|e|format!("Grok private HOME duplicate stop: {e:?}"))? {
            return Err("Grok private HOME: ambiguous StopFact".into());
        }
        self.revoke_effects(db,profile,&grant,Some(&stop),None)
    }

    fn revoke_effects(&self,db:&mut VerifiedDatabaseConnection<'_>,profile:&AppContainerProfile,
        grant:&GrokGrant,stop:Option<&str>,gone:Option<&NativeProcessHoldersGone>)->Result<(),String>{
        if grant.phase=="REVOKE_PENDING" && grant.stop_fact_id.as_deref()!=stop {
            return Err("Grok private HOME: original retirement proof changed".into());
        }
        let pair=grant.pid.zip(grant.creation_time_100ns)
            .map(|(pid,creation)|vec![(pid,creation)]);
        let validate_gone=||->Result<(),String>{
            if let Some(proof)=gone {
                let pairs=pair.as_deref().ok_or("Grok private HOME: original holder pair absent")?;
                evidence("holder-gone",proof.validate(pairs))?;
            }
            Ok(())
        };
        let pending=if grant.phase=="REVOKE_PENDING"{grant.clone()} else {
            instance::set_grok_grant_phase(db,grant,"REVOKE_PENDING",stop)?;
            self.grant(db)?
        };
        // The old holder is already stopped/gone. Finish only a previously
        // authorized grant whose exact ACL AFTER state can be read back; never
        // issue a grant during retirement or manufacture a stopped receipt.
        for intent in instance::read_grok_effects(db,&pending.binding_id)?.into_iter()
            .filter(|intent|intent.action=="GRANT_AUTH" &&intent.phase=="INTENT") {
            validate_gone()?;
            let mut held=self.auth.lock().map_err(|_|"Grok private HOME: auth custody poisoned")?;
            if !held.iter().any(|auth|auth.identity==intent.object_identity) {
                let auth=evidence("find-unfinished-grant-FileID",observe_grok_recorded_auth(
                    &self.home.path,&self.home.identity,&intent.object_identity))?
                    .ok_or("Grok private HOME: unfinished grant FileID not physically recoverable")?;
                held.push(auth);
            }
            let auth=held.iter().find(|auth|auth.identity==intent.object_identity)
                .ok_or("Grok private HOME: unfinished grant custody absent")?;
            let after=evidence("unfinished-grant-AFTER",auth.acl(profile))?;
            if after.identity!=intent.object_identity ||after.target_aces!=intent.after_aces ||
                after.dacl_control!=intent.after_control ||
                sha256_hex(&after.other_aces_bytes())!=intent.other_aces_sha256 {
                return Err("Grok private HOME: unfinished grant lacks exact ACL AFTER proof".into());
            }
            instance::finish_grok_effect(db,&intent)?;
        }
        let ids=recorded_auth(db,&pending)?;
        let prior_effects=instance::read_grok_effects(db,&pending.binding_id)?;
        {
            let mut held=self.auth.lock().map_err(|_|"Grok private HOME: auth custody poisoned")?;
            for id in &ids {
                if held.iter().any(|auth|&auth.identity==id) ||prior_effects.iter().any(|effect|
                    effect.action=="REVOKE_AUTH" &&effect.phase=="APPLIED" &&
                    &effect.object_identity==id) {continue;}
                let found=evidence("find-recorded-old-FileID",observe_grok_recorded_auth(
                    &self.home.path,&self.home.identity,id))?;
                let found=found.ok_or("Grok private HOME: old granted FileID not provably revoked or in F HOME")?;
                held.push(found);
            }
        }
        let root_before=evidence("root-ACL-before-revoke",grok_root_acl(profile,&self.home.path,&self.home.identity))?;
        validate_gone()?;
        let root_effect=effect(&pending,"REVOKE_ROOT",&self.home.identity,".",&root_before);
        if instance::read_grok_root_anchor(db,&pending.instance_id)?.is_some() {
            apply_root(db,&pending,root_effect,
                ||evidence("root-ACL-after-revoke",grok_root_acl(profile,&self.home.path,&self.home.identity)),||
                evidence("revoke-root",revoke_grok_home_root(profile,&self.home.path,&self.home.identity)))?;
        } else {
            // Old journals lack an ordered baseline. Keep their original exact
            // stopped/no-attempt retirement path; never use it for active use.
            apply(db,root_effect,
                ||evidence("legacy-root-ACL-after-revoke",grok_root_acl(profile,&self.home.path,&self.home.identity)),||
                evidence("legacy-revoke-root",revoke_grok_home_root(profile,&self.home.path,&self.home.identity)))?;
        }
        let held=self.auth.lock().map_err(|_|"Grok private HOME: auth custody poisoned")?;
        for auth in held.iter() {
            let before=evidence("auth-ACL-before-revoke",auth.acl(profile))?;
            let relative=prior_effects.iter().find(|effect|effect.action=="REVOKE_AUTH" &&
                effect.object_identity==auth.identity).map(|effect|effect.relative_name.clone())
                .unwrap_or_else(||auth.relative_name().to_string_lossy().into_owned());
            validate_gone()?;
            apply(db,effect(&pending,"REVOKE_AUTH",&auth.identity,&relative,&before),
                ||evidence("auth-ACL-after-revoke",auth.acl(profile)),||
                evidence("revoke-auth",revoke_grok_auth(profile,auth)))?;
        }
        drop(held);
        let effects=instance::read_grok_effects(db,&pending.binding_id)?;
        if ids.iter().any(|id|!effects.iter().any(|effect|
            effect.action=="REVOKE_AUTH" && effect.phase=="APPLIED" && &effect.object_identity==id)) {
            return Err("Grok private HOME: recorded old auth FileID lacks revoked ACL readback".into());
        }
        let peers=instance::read_grok_grants(db,&self.instance_id)?.into_iter().any(|g|
            g.binding_id!=self.binding_id && matches!(g.phase.as_str(),
                "GRANT_PENDING"|"GRANTED_UNCREATED"|"ACTIVE"|"REVOKE_PENDING"|"UNKNOWN"));
        if peers {
            instance::set_grok_grant_phase(db,&pending,"RETIRED_CLEANUP_PENDING",None)?;
            return Ok(());
        }
        let inherited=proven_inherited_successors(db,&pending,profile,&self.home)?;
        for object in evidence("scan-residue",inspect_grok_home_residue(profile,&self.home.path,
            &self.home.identity,&ids,&inherited))? {
            let relative=object.relative_name.to_string_lossy().into_owned();
            let before=evidence("residue-ACL-before",grok_residue_acl(profile,&self.home.path,
                &self.home.identity,&object))?;
            validate_gone()?;
            apply(db,effect(&pending,"REVOKE_RESIDUE",&object.identity,&relative,&before),
                ||evidence("residue-ACL-after",grok_residue_acl(profile,&self.home.path,
                    &self.home.identity,&object)),||
                evidence("revoke-residue",revoke_grok_home_residue(profile,&self.home.path,
                    &self.home.identity,&object)))?;
        }
        if !evidence("readback-residue",inspect_grok_home_residue(profile,&self.home.path,
            &self.home.identity,&ids,&inherited))?.is_empty() {
            return Err("Grok private HOME: residual original SID".into());
        }
        instance::set_grok_grant_phase(db,&pending,"REVOKED",None)
    }
}

/// Cold open is only an inventory. It never treats empty in-memory H holders
/// as evidence that a process stopped or disappeared.
pub(crate) fn cold_inventory(db:&VerifiedDatabaseConnection<'_>,instance_id:&str)
    ->Result<Vec<GrokGrant>,String>{
    // Include REVOKED: F may have finished ACL retirement immediately before
    // the original H holder-gone RELEASED transaction crashed.
    instance::read_grok_grants(db,instance_id)
}

/// Read only: expose the original H operation and exact process pair so Root
/// can qualify its real holder-gone scope. None is not a NoAttempt receipt.
pub(crate) fn prepared_holder_for_recovery(db:&VerifiedDatabaseConnection<'_>,
    original:&GrokGrant)->Result<Option<(String,u32,u64)>,String>{
    let grant=one(db,&original.instance_id,&original.binding_id)?;
    if grant!=*original ||grant.phase!="GRANTED_UNCREATED" ||
        grant.process_operation_id.is_some() {
        return Err("Grok private HOME: uncreated recovery row changed".into());
    }
    let Some(h)=original_h(db,&grant)? else{return Ok(None);};
    if h.digest!=grant.program_digest ||!holder_gone_original_eligible(&h) {
        return Err("Grok private HOME: original prepared H tuple ineligible".into());
    }
    Ok(Some((h.operation,h.pid,h.creation)))
}

/// A resume candidate has an original process episode but no current H claim
/// for its new generation. Its old claim and StopFact remain untouched while
/// F retires only the candidate's original physical grant.
pub(crate) fn original_candidate_for_recovery(db:&VerifiedDatabaseConnection<'_>,
    original:&GrokGrant)->Result<Option<(String,u32,u64)>,String>{
    let grant=one(db,&original.instance_id,&original.binding_id)?;
    if grant!=*original ||grant.process_operation_id.is_none() {
        return Err("Grok private HOME: candidate original F row changed".into());
    }
    let Some(h)=original_h(db,&grant)? else{return Ok(None);};
    if !h.candidate_before_promote{return Ok(None);}
    if exact_exists(db,"candidate current claim",
        "SELECT 1 FROM main.gogoke_v37_h_claim WHERE instance_id=?1 AND domain_id=?2
         AND session_id=?3 AND generation=?4 AND binding_id=?5",
        &[&grant.instance_id,&grant.domain_id,&grant.session_id,&grant.generation,
            &grant.binding_id])? || !matches_bound_original(&grant,&h) ||
        h.digest!=grant.program_digest || !holder_gone_original_eligible(&h) {
        return Err("Grok private HOME: original candidate H/F/custody association changed".into());
    }
    Ok(Some((h.operation,h.pid,h.creation)))
}

/// Crash after H persisted its exact process but before F copied that tuple.
/// A genuine kernel holder-gone proof permits adoption of the original H row
/// only; no NULL process row is interpreted as NoAttempt.
pub(crate) fn adopt_original_holder_gone(db:&mut VerifiedDatabaseConnection<'_>,root:&RootLock,
    original:&GrokGrant,proof:&NativeProcessHoldersGone)->Result<(),String>{
    let grant=one(db,&original.instance_id,&original.binding_id)?;
    if grant!=*original ||grant.phase!="GRANTED_UNCREATED" ||
        grant.process_operation_id.is_some() {
        return Err("Grok private HOME: uncreated original F row changed".into());
    }
    let h=original_h(db,&grant)?.ok_or("Grok private HOME: original H process absent; no NoAttempt inference")?;
    if h.digest!=grant.program_digest ||!holder_gone_original_eligible(&h) {
        return Err("Grok private HOME: original H process cannot be adopted".into());
    }
    evidence("holder-gone",proof.validate(&[(h.pid,h.creation)]))?;
    instance::bind_grok_original_process(db,&grant,&h.operation,&h.ticket,&h.nonce,
        h.pid,h.creation,&h.image)?;
    let adopted=one(db,&grant.instance_id,&grant.binding_id)?;
    retire_holder_gone(db,root,&adopted,proof)
}

/// Continue the same persisted revoke after a real original StopFact. The F
/// phase and H/custody stop hashes must all be the same before any ACL write.
pub(crate) fn resume_stopped_revoke(db:&mut VerifiedDatabaseConnection<'_>,root:&RootLock,
    original:&GrokGrant)->Result<(),String>{
    let grant=one(db,&original.instance_id,&original.binding_id)?;
    if grant!=*original ||!matches!(grant.phase.as_str(),"ACTIVE"|"REVOKE_PENDING") ||
        (grant.phase=="ACTIVE" && grant.stop_fact_id.is_some()) {
        return Err("Grok private HOME: stopped revoke original F row changed".into());
    }
    let h=original_h(db,&grant)?.ok_or("Grok private HOME: original stopped H tuple absent")?;
    if !matches_bound_original(&grant,&h) ||h.custody_state!="STOPPED" ||
        !matches!(h.claim_state.as_str(),"STOPPED"|"RELEASED") ||h.episode_state!="STOPPED" ||
        h.custody_stop.as_deref()!=h.claim_stop.as_deref() ||
        h.custody_stop.as_deref()!=h.episode_stop.as_deref() ||
        h.custody_stop.as_deref().map_or(true,str::is_empty) ||
        (grant.phase=="REVOKE_PENDING" &&grant.stop_fact_id!=h.custody_stop) {
        return Err("Grok private HOME: original StopFact/F tuple disagrees".into());
    }
    let home=evidence("resolve-F-HOME",instance::resolve_grok_original_home(db,root,&grant.instance_id))?;
    if home.identity!=grant.home_identity {return Err("Grok private HOME: F HOME identity changed".into());}
    let profile=evidence("derive-original-SID",AppContainerProfile::derive_for_revocation(&grant.profile_name))?;
    if evidence("SID-readback",profile.sid_identity())?!=grant.profile_sid {
        return Err("Grok private HOME: original SID changed".into());
    }
    let current=evidence("observe-current-auth-metadata",observe_grok_auth_candidate(&home.path,&home.identity))?;
    let ids=recorded_auth(db,&grant)?;
    let held=if ids.contains(&current.identity){vec![current]}else{Vec::new()};
    let recovered=GrokHomeLaunch{instance_id:grant.instance_id.clone(),binding_id:grant.binding_id.clone(),
        home,auth:Mutex::new(held)};
    recovered.revoke_effects(db,&profile,&grant,h.custody_stop.as_deref(),None)
}

/// F revoked the original root and recorded auth FileIDs first, then H release
/// crashed. A RETIRED_CLEANUP_PENDING row still awaits natural peer quiescence
/// for its domain scan. This path does no ACL or H write.
pub(crate) fn verify_completed_holder_gone(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,
    original:&GrokGrant,proof:&NativeProcessHoldersGone)->Result<(),String>{
    let grant=one(db,&original.instance_id,&original.binding_id)?;
    if grant!=*original ||!matches!(grant.phase.as_str(),"REVOKED"|"RETIRED_CLEANUP_PENDING") ||
        grant.stop_fact_id.is_some() {
        return Err("Grok private HOME: completed holder-gone F row changed".into());
    }
    let h=original_h(db,&grant)?.ok_or("Grok private HOME: original H tuple absent")?;
    if !matches_bound_original(&grant,&h) ||!holder_gone_original_eligible(&h) {
        return Err("Grok private HOME: original H release window changed".into());
    }
    evidence("holder-gone",proof.validate(&[(h.pid,h.creation)]))?;
    let home=evidence("resolve-F-HOME",instance::resolve_grok_original_home(db,root,&grant.instance_id))?;
    if home.identity!=grant.home_identity {return Err("Grok private HOME: F HOME identity changed".into());}
    let profile=evidence("derive-original-SID",AppContainerProfile::derive_for_revocation(&grant.profile_name))?;
    if evidence("SID-readback",profile.sid_identity())?!=grant.profile_sid {
        return Err("Grok private HOME: original SID changed".into());
    }
    let effects=instance::read_grok_effects(db,&grant.binding_id)?;
    let ids=recorded_auth(db,&grant)?;
    if !effects.iter().any(|e|e.action=="REVOKE_ROOT" &&e.phase=="APPLIED" &&
        e.object_identity==home.identity) ||ids.iter().any(|id|!effects.iter().any(|e|
        e.action=="REVOKE_AUTH" &&e.phase=="APPLIED" && &e.object_identity==id)) ||
        effects.iter().any(|e|e.phase!="APPLIED") {
        return Err("Grok private HOME: completed F ACL effects unproven".into());
    }
    let root_acl=evidence("root-readback",grok_root_acl(&profile,&home.path,&home.identity))?;
    let inherited=if grant.phase=="RETIRED_CLEANUP_PENDING" {
        proven_inherited_successors(db,&grant,&profile,&home)?
    }else{Vec::new()};
    let residue=evidence("domain-readback",inspect_grok_home_residue(&profile,&home.path,
        &home.identity,&ids,&inherited))?;
    if !root_acl.target_aces.is_empty() ||residue.iter().any(|object|
        !object.protected_inherited ||!inherited.contains(&object.identity)) {
        return Err("Grok private HOME: completed F SID residue".into());
    }
    Ok(())
}

/// A previously completed pre-factory NoAttempt retirement. This is a
/// historical readback only; a new NULL process row is never NoAttempt proof.
pub(crate) fn verify_completed_no_attempt(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,
    original:&GrokGrant)->Result<(),String>{
    let grant=one(db,&original.instance_id,&original.binding_id)?;
    if grant!=*original ||grant.phase!="REVOKED" ||grant.process_operation_id.is_some() ||
        grant.ticket.is_some() ||grant.custodian_nonce.is_some() ||grant.pid.is_some() ||
        grant.creation_time_100ns.is_some() ||grant.image_path.is_some() ||
        grant.stop_fact_id.is_some() {
        return Err("Grok private HOME: completed NoAttempt F row changed".into());
    }
    if original_no_attempt_has_process(db,&grant)? {
        return Err("Grok private HOME: original binding has H episode or custody".into());
    }
    let home=evidence("resolve-F-HOME",instance::resolve_grok_original_home(db,root,&grant.instance_id))?;
    if home.identity!=grant.home_identity {return Err("Grok private HOME: NoAttempt F HOME changed".into());}
    let profile=evidence("derive-original-SID",AppContainerProfile::derive_for_revocation(&grant.profile_name))?;
    if evidence("SID-readback",profile.sid_identity())?!=grant.profile_sid {
        return Err("Grok private HOME: NoAttempt original SID changed".into());
    }
    let effects=instance::read_grok_effects(db,&grant.binding_id)?;
    let ids=recorded_auth(db,&grant)?;
    if effects.iter().any(|e|e.phase!="APPLIED") ||
        !effects.iter().any(|e|e.action=="REVOKE_ROOT" &&e.phase=="APPLIED" &&
            e.object_identity==home.identity) ||
        ids.iter().any(|id|!effects.iter().any(|e|e.action=="REVOKE_AUTH" &&
            e.phase=="APPLIED" && &e.object_identity==id)) {
        return Err("Grok private HOME: NoAttempt ACL effects incomplete".into());
    }
    let root_acl=evidence("NoAttempt root readback",grok_root_acl(&profile,&home.path,&home.identity))?;
    if !root_acl.target_aces.is_empty() ||
        !evidence("NoAttempt domain readback",inspect_grok_home_residue(&profile,&home.path,
            &home.identity,&ids,&[]))?.is_empty() {
        return Err("Grok private HOME: NoAttempt SID residue".into());
    }
    Ok(())
}

/// The stopped generation may no longer be the current H claim after a
/// legitimate resume. Verify its immutable episode/custody and F ACL effects
/// before Root defers the final HOME scan to natural quiescence.
pub(crate) fn verify_retired_stopped(db:&VerifiedDatabaseConnection<'_>,root:&RootLock,
    original:&GrokGrant)->Result<(),String>{
    let grant=one(db,&original.instance_id,&original.binding_id)?;
    if grant!=*original ||grant.phase!="RETIRED_CLEANUP_PENDING" ||
        grant.stop_fact_id.as_deref().map_or(true,str::is_empty) {
        return Err("Grok private HOME: original retired StopFact F row changed".into());
    }
    let operation=grant.process_operation_id.as_deref()
        .ok_or("Grok private HOME: retired original operation absent")?;
    let ticket=grant.ticket.as_deref().ok_or("Grok private HOME: retired ticket absent")?;
    let nonce=grant.custodian_nonce.as_deref().ok_or("Grok private HOME: retired nonce absent")?;
    let pid=grant.pid.ok_or("Grok private HOME: retired PID absent")?.to_string();
    let creation=grant.creation_time_100ns.ok_or("Grok private HOME: retired creation absent")?.to_string();
    let image=grant.image_path.as_deref().ok_or("Grok private HOME: retired image absent")?;
    let stop=grant.stop_fact_id.as_deref().unwrap_or("");
    if !exact_exists(db,"retired stopped original",
        "SELECT 1 FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation WHERE e.binding_id=?1 AND e.instance_id=?2 AND e.domain_id=?3 AND e.session_id=?4 AND e.generation=?5 AND e.request_id=?6 AND e.seat_id=?7 AND e.seat_incarnation=?8 AND e.process_operation_id=?9 AND e.phase='STOPPED' AND e.stop_fact_id=?10 AND c.ticket=?11 AND c.custodian_nonce=?12 AND c.pid=?13 AND c.creation_time_100ns=?14 AND c.image_path=?15 AND c.binary_digest_sha256=?16 AND c.state='STOPPED' AND c.stop_proof_hash=?10",
        &[&grant.binding_id,&grant.instance_id,&grant.domain_id,&grant.session_id,
            &grant.generation,&grant.request_id,&grant.seat_id,&grant.seat_incarnation,
            operation,stop,ticket,nonce,&pid,&creation,image,&grant.program_digest])? {
        return Err("Grok private HOME: historical StopFact H/custody tuple changed".into());
    }
    let home=evidence("resolve-F-HOME",instance::resolve_grok_original_home(db,root,&grant.instance_id))?;
    if home.identity!=grant.home_identity {return Err("Grok private HOME: retired F HOME changed".into());}
    let profile=evidence("derive-original-SID",AppContainerProfile::derive_for_revocation(&grant.profile_name))?;
    if evidence("SID-readback",profile.sid_identity())?!=grant.profile_sid {
        return Err("Grok private HOME: retired original SID changed".into());
    }
    let effects=instance::read_grok_effects(db,&grant.binding_id)?;
    let ids=recorded_auth(db,&grant)?;
    if effects.iter().any(|e|e.phase!="APPLIED") ||
        !effects.iter().any(|e|e.action=="REVOKE_ROOT" &&e.phase=="APPLIED" &&
            e.object_identity==home.identity) ||
        ids.iter().any(|id|!effects.iter().any(|e|e.action=="REVOKE_AUTH" &&
            e.phase=="APPLIED" && &e.object_identity==id)) {
        return Err("Grok private HOME: retired stopped ACL effects incomplete".into());
    }
    let root_acl=evidence("retired root readback",grok_root_acl(&profile,&home.path,&home.identity))?;
    if !root_acl.target_aces.is_empty() {
        return Err("Grok private HOME: retired stopped root SID persists".into());
    }
    Ok(())
}

/// Root's holder-disappearance ingress supplies the original F row and sealed
/// kernel observation. We independently join the original H episode, claim,
/// seat and custody; a NULL StopFact remains NULL. H release is Root-owned.
pub(crate) fn retire_holder_gone(db:&mut VerifiedDatabaseConnection<'_>,root:&RootLock,
    original:&GrokGrant,proof:&NativeProcessHoldersGone)->Result<(),String>{
    let grant=one(db,&original.instance_id,&original.binding_id)?;
    if grant!=*original || !matches!(grant.phase.as_str(),"ACTIVE"|"REVOKE_PENDING") ||
        grant.stop_fact_id.is_some() {
        return Err("Grok private HOME: original F holder changed".into());
    }
    let (pid,creation)=grant.pid.zip(grant.creation_time_100ns)
        .ok_or("Grok private HOME: original holder identity absent")?;
    evidence("holder-gone",proof.validate(&[(pid,creation)]))?;
    let h=original_h(db,&grant)?.ok_or("Grok private HOME: original H/custody association absent")?;
    if !matches_bound_original(&grant,&h) ||!holder_gone_original_eligible(&h) {
        return Err("Grok private HOME: original holder state not eligible".into());
    }
    let home=evidence("resolve-F-HOME",instance::resolve_grok_original_home(db,root,&grant.instance_id))?;
    if home.identity!=grant.home_identity {return Err("Grok private HOME: F HOME identity changed".into());}
    let profile=evidence("derive-original-SID",AppContainerProfile::derive_for_revocation(&grant.profile_name))?;
    if evidence("SID-readback",profile.sid_identity())?!=grant.profile_sid {
        return Err("Grok private HOME: original SID changed".into());
    }
    let auth=evidence("observe-original-HOME-auth-metadata",observe_grok_auth_candidate(&home.path,&home.identity))?;
    let ids=recorded_auth(db,&grant)?;
    let held=if ids.contains(&auth.identity){vec![auth]}else{Vec::new()};
    let recovered=GrokHomeLaunch{instance_id:grant.instance_id.clone(),binding_id:grant.binding_id.clone(),
        home,auth:Mutex::new(held)};
    recovered.revoke_effects(db,&profile,&grant,None,Some(proof))
}

/// Natural quiescence readback for grants whose root and all recorded auth
/// FileIDs were already revoked under durable effects while peers remained.
/// This never repairs an unproven effect or treats an empty host map as proof.
pub(crate) fn finalize_quiescent(db:&mut VerifiedDatabaseConnection<'_>,root:&RootLock,
    instance_id:&str)->Result<(),String>{
    let grants=instance::read_grok_grants(db,instance_id)?;
    if grants.iter().any(|g|matches!(g.phase.as_str(),
        "GRANT_PENDING"|"GRANTED_UNCREATED"|"ACTIVE"|"REVOKE_PENDING"|"UNKNOWN")) {
        return Err("Grok private HOME: other original grant is not quiescent".into());
    }
    let h=Statement::prepare(db.as_ptr(),"SELECT 1 FROM main.gogoke_v37_h_claim WHERE instance_id=?1 AND state NOT IN ('STOPPED','RELEASED') LIMIT 1")
        .map_err(|e|format!("Grok private HOME H quiescence: {e:?}"))?;
    h.bind_text(1,instance_id).map_err(|e|format!("Grok private HOME H quiescence bind: {e:?}"))?;
    if h.step_row().map_err(|e|format!("Grok private HOME H quiescence read: {e:?}"))? {
        return Err("Grok private HOME: original H claim still owns capacity".into());
    }
    drop(h);
    let home=evidence("resolve-F-HOME",instance::resolve_grok_original_home(db,root,instance_id))?;
    for grant in grants.into_iter().filter(|g|g.phase=="RETIRED_CLEANUP_PENDING") {
        if grant.home_identity!=home.identity ||grant.profile_name.is_empty() {
            return Err("Grok private HOME: retired F domain changed".into());
        }
        let profile=evidence("derive-original-SID",AppContainerProfile::derive_for_revocation(&grant.profile_name))?;
        if evidence("SID-readback",profile.sid_identity())?!=grant.profile_sid {
            return Err("Grok private HOME: original SID changed".into());
        }
        let effects=instance::read_grok_effects(db,&grant.binding_id)?;
        let root_done=effects.iter().any(|e|e.action=="REVOKE_ROOT" && e.phase=="APPLIED" &&
            e.object_identity==home.identity);
        let ids=recorded_auth(db,&grant)?;
        if !root_done || ids.iter().any(|id|!effects.iter().any(|e|
            e.action=="REVOKE_AUTH" &&e.phase=="APPLIED" && &e.object_identity==id)) ||
            effects.iter().any(|e|e.phase!="APPLIED") {
            return Err("Grok private HOME: retired original ACL effect unresolved".into());
        }
        let root_acl=evidence("retired-root-readback",grok_root_acl(&profile,&home.path,&home.identity))?;
        let inherited=proven_inherited_successors(db,&grant,&profile,&home)?;
        if !root_acl.target_aces.is_empty() ||
            !evidence("retired-domain-readback",inspect_grok_home_residue(&profile,
                &home.path,&home.identity,&ids,&inherited))?.is_empty() {
            return Err("Grok private HOME: retired SID residue".into());
        }
        instance::set_grok_grant_phase(db,&grant,"REVOKED",None)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new,open_existing,route_b_test_guard};
    use crate::process::{NativeBinding,ProcessCustodian,ProcessLaunch,PrepareRequest,StopBudgets};
    use crate::store::session_transport::admission::{self,AdmissionRequest,AdmissionResult};
    use std::path::Path;
    use std::time::{SystemTime,UNIX_EPOCH};

    #[test]
    fn original_inherited_auth_is_journaled_protected_and_revoked_without_process(){
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("grok-original-auth-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        crate::store::product_database::prepare_managed_grok_acl_fixture(&root,&database,"grokA");
        let mut db=open_existing(&root,&database).unwrap();
        let home=instance::resolve_grok_original_home(&db,&root,"grokA").unwrap();
        let profile=AppContainerProfile::derived_for_test("Gogoke37.OriginalInheritedAuth").unwrap();
        std::fs::write(home.path.join("auth.json"),b"synthetic non-secret fixture").unwrap();
        let candidate=observe_grok_auth_candidate(&home.path,&home.identity).unwrap();
        let before=candidate.candidate_acl(&profile).unwrap();
        assert!(!before.dacl_protected);
        assert!(observe_grok_auth(&home.path,&home.identity).is_err());
        let pin=super::super::runtime::current_instance_pin(&db,"grokA").unwrap();
        let claim=ClaimObservation{domain_id:"domainA".into(),session_id:"sessionA".into(),
            instance_id:"grokA".into(),home_id:"sessionHomeA".into(),
            binding_id:"bindingA".into(),generation:"1".into(),revision:2,
            phase:super::super::runtime::SessionPhase::Committed,process_operation_id:None};
        db.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('domainA','seatA','incA','USER','LONG','grokA','BUSY',1,1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingA','grokA','domainA','SESSION','sessionA','1','ACTIVE')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES('domainA','sessionA','grokA','sessionHomeA','bindingA','1','COMMITTED',2)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_seat_binding VALUES('domainA','sessionA','seatA','incA','1')").unwrap();
        assert!(!original_grant_admitted(&db,&claim,"seatA","wrong-incarnation","openA").unwrap());
        let launch=GrokHomeLaunch::prepare(&mut db,&root,&profile,
            "Gogoke37.OriginalInheritedAuth",&claim,&pin,&home,"seatA","incA","openA").unwrap();
        let granted=observe_grok_auth(&home.path,&home.identity).unwrap();
        assert_eq!(granted.identity,candidate.identity);
        assert!(before.preserves_other_aces(&granted.acl(&profile).unwrap()));
        let effects=instance::read_grok_effects(&db,"bindingA").unwrap();
        let auth_effect=effects.iter().find(|e|e.action=="GRANT_AUTH").unwrap();
        // prepare grants the parent before protecting auth. Windows may set
        // AUTO_INHERITED during that parent transition; the earlier snapshot
        // is not the control value at the actual auth effect's write boundary.
        assert_eq!(auth_effect.before_control & 0x1000,0);
        assert_eq!(auth_effect.after_control,auth_effect.before_control|0x1000);
        assert_eq!(auth_effect.after_control,granted.acl(&profile).unwrap().dacl_control);
        assert_eq!(auth_effect.phase,"APPLIED");
        launch.revoke_uncreated(&mut db,&root,&profile).unwrap();
        assert_eq!(one(&db,"grokA","bindingA").unwrap().phase,"REVOKED");
        assert!(grok_root_acl(&profile,&home.path,&home.identity).unwrap().target_aces.is_empty());
        assert!(granted.candidate_acl(&profile).unwrap().target_aces.is_empty());
        drop(granted);drop(candidate);drop(launch);drop(profile);
        // Model an old, fully settled F/H journal opened by the new schema:
        // its original ordered anchor did not exist. A new authorized F
        // preparation may establish its own baseline without another login.
        db.execute("DELETE FROM main.gogoke_v37_grok_home_root_anchor WHERE instance_id='grokA'").unwrap();
        let release=AdmissionRequest{domain_id:"domainA",session_id:"sessionA",request_id:"releaseA",
            raw_bytes:b"fixture original unstarted release",instance_id:"grokA",
            home_id:"sessionHomeA",generation:"1",expected_revision:2};
        assert_eq!(admission::release_unstarted_owner_commit(&mut db,&release,|_|Ok(())).unwrap(),
            AdmissionResult::Applied(3));
        db.execute("UPDATE main.gogoke_v37_seats SET generation=2,state='BUSY' WHERE domain_id='domainA' AND seat_id='seatA'").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingB','grokA','domainA','SESSION','sessionB','2','ACTIVE')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES('domainA','sessionB','grokA','sessionHomeB','bindingB','2','COMMITTED',2)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_seat_binding VALUES('domainA','sessionB','seatA','incA','2')").unwrap();
        let next=ClaimObservation{domain_id:"domainA".into(),session_id:"sessionB".into(),
            instance_id:"grokA".into(),home_id:"sessionHomeB".into(),
            binding_id:"bindingB".into(),generation:"2".into(),revision:2,
            phase:super::super::runtime::SessionPhase::Committed,process_operation_id:None};
        let rebased_profile=AppContainerProfile::derived_for_test("Gogoke37.OriginalRebase").unwrap();
        let rebased=GrokHomeLaunch::prepare(&mut db,&root,&rebased_profile,
            "Gogoke37.OriginalRebase",&next,&pin,&home,"seatA","incA","openB").unwrap();
        let anchor=instance::read_grok_root_anchor(&db,"grokA").unwrap().unwrap();
        assert!(!anchor.baseline_effect_id.is_empty());
        rebased.revoke_uncreated(&mut db,&root,&rebased_profile).unwrap();
        drop(rebased);
        db.close_checked().unwrap();drop(root);std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn active_inherited_successor_peer_stop_and_original_final_cleanup(){
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("grok-inherited-peer-stop-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        crate::store::product_database::prepare_managed_grok_acl_fixture(&root,&database,"grokA");
        let mut db=open_existing(&root,&database).unwrap();
        let home=instance::resolve_grok_original_home(&db,&root,"grokA").unwrap();
        std::fs::write(home.path.join("auth.json"),b"synthetic non-secret original fixture").unwrap();
        let pin=super::super::runtime::current_instance_pin(&db,"grokA").unwrap();
        let program=instance::locate_bound_instance_program(&db,"grokA","grok",
            &pin.digest,&pin.version).unwrap();
        let peer_home=path.join("peer-session-home");
        std::fs::create_dir(&peer_home).unwrap();
        let peer_lock=RootLock::acquire(&peer_home).unwrap();
        let peer_home_id=peer_lock.canonical_root().identity.opaque();
        drop(peer_lock);
        db.execute(&format!("INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,directory_identity,state,revision) VALUES('sessionHomeB','grokA','domainA','SESSION','sessionB','1','{peer_home_id}','ACTIVE',1)")).unwrap();
        for (seat,inc,session,binding,home_id) in [
            ("seatA","incA","sessionA","bindingA","sessionHomeA"),
            ("seatB","incB","sessionB","bindingB","sessionHomeB")] {
            db.execute(&format!("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('domainA','{seat}','{inc}','USER','LONG','grokA','BUSY',1,1)")).unwrap();
            db.execute(&format!("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('{binding}','grokA','domainA','SESSION','{session}','1','ACTIVE')")).unwrap();
            db.execute(&format!("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES('domainA','{session}','grokA','{home_id}','{binding}','1','COMMITTED',2)")).unwrap();
            db.execute(&format!("INSERT INTO main.gogoke_v37_h_seat_binding VALUES('domainA','{session}','{seat}','{inc}','1')")).unwrap();
        }
        let claim_a=ClaimObservation{domain_id:"domainA".into(),session_id:"sessionA".into(),
            instance_id:"grokA".into(),home_id:"sessionHomeA".into(),binding_id:"bindingA".into(),
            generation:"1".into(),revision:2,phase:super::super::runtime::SessionPhase::Committed,
            process_operation_id:None};
        let profile_a=AppContainerProfile::derived_for_test("Gogoke37.InheritedPeerStopA").unwrap();
        let launch_a=GrokHomeLaunch::prepare(&mut db,&root,&profile_a,
            "Gogoke37.InheritedPeerStopA",&claim_a,&pin,&home,"seatA","incA","openA").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_operation VALUES('domainA','openA','6f70656e','open','sessionA','APPLIED',1,2)").unwrap();
        let mut custodian=ProcessCustodian::new().unwrap();
        let mut process_a=ProcessLaunch::new(program.clone());
        process_a.arguments=vec!["--version".into()];
        let custody_a=custodian.prepare(&PrepareRequest{launch:process_a,binding:NativeBinding{
            binary_digest_sha256:pin.digest.clone(),profile_id:"grokA".into(),
            domain_id:"domainA".into(),generation:"1".into()}}).unwrap();
        crate::store::authority::record_prepared_process(&mut db,"opA",&custody_a).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        admission::bind_process_operation_in_transaction(&mut db,"domainA","sessionA","opA").unwrap();
        db.execute("COMMIT").unwrap();
        super::super::episodes::record_initial(&db,"domainA","sessionA","openA","opA").unwrap();
        launch_a.bind_process(&mut db,"opA",&custody_a).unwrap();
        custodian.activate(&custody_a).unwrap();
        crate::store::authority::mark_process_active(&mut db,"opA",&custody_a).unwrap();
        super::super::episodes::mark_active(&db,"opA").unwrap();
        let successor_path=home.path.join("auth-next.json");
        std::fs::write(&successor_path,b"synthetic non-secret successor fixture").unwrap();
        std::fs::remove_file(home.path.join("auth.json")).unwrap();
        std::fs::rename(&successor_path,home.path.join("auth.json")).unwrap();
        let successor=observe_grok_auth_candidate(&home.path,&home.identity).unwrap();
        assert_ne!(successor.identity,one(&db,"grokA","bindingA").unwrap().auth_identity);
        let claim_b=ClaimObservation{domain_id:"domainA".into(),session_id:"sessionB".into(),
            instance_id:"grokA".into(),home_id:"sessionHomeB".into(),binding_id:"bindingB".into(),
            generation:"1".into(),revision:2,phase:super::super::runtime::SessionPhase::Committed,
            process_operation_id:None};
        let profile_b=AppContainerProfile::derived_for_test("Gogoke37.InheritedPeerStopB").unwrap();
        let launch_b=GrokHomeLaunch::prepare(&mut db,&root,&profile_b,
            "Gogoke37.InheritedPeerStopB",&claim_b,&pin,&home,"seatB","incB","openB").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_operation VALUES('domainA','openB','6f70656e','open','sessionB','APPLIED',1,2)").unwrap();
        let mut process_b=ProcessLaunch::new(program);
        process_b.arguments=vec!["--version".into()];
        let custody_b=custodian.prepare(&PrepareRequest{launch:process_b,binding:NativeBinding{
            binary_digest_sha256:pin.digest.clone(),profile_id:"grokA".into(),
            domain_id:"domainA".into(),generation:"1".into()}}).unwrap();
        crate::store::authority::record_prepared_process(&mut db,"opB",&custody_b).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        admission::bind_process_operation_in_transaction(&mut db,"domainA","sessionB","opB").unwrap();
        db.execute("COMMIT").unwrap();
        super::super::episodes::record_initial(&db,"domainA","sessionB","openB","opB").unwrap();
        launch_b.bind_process(&mut db,"opB",&custody_b).unwrap();
        custodian.activate(&custody_b).unwrap();
        crate::store::authority::mark_process_active(&mut db,"opB",&custody_b).unwrap();
        super::super::episodes::mark_active(&db,"opB").unwrap();
        launch_a.verify(&db,&profile_a,false).unwrap();
        launch_b.verify(&db,&profile_b,false).unwrap();
        let unrecorded=AppContainerProfile::derived_for_test("Gogoke37.InheritedPeerUnrecorded").unwrap();
        grant_grok_auth_successor(&unrecorded,&successor).unwrap();
        assert!(launch_b.verify(&db,&profile_b,false).is_err());
        revoke_grok_auth(&unrecorded,&successor).unwrap();
        launch_b.verify(&db,&profile_b,false).unwrap();
        assert!(instance::read_grok_effects(&db,"bindingA").unwrap().iter().all(|effect|
            effect.action!="GRANT_AUTH" ||effect.object_identity!=successor.identity));
        let stop_a=custodian.stop(&custody_a.ticket,StopBudgets::production(),||Ok(())).unwrap();
        assert!(stop_a.errors.is_empty() &&stop_a.parent_exited &&stop_a.active_job_processes==Some(0));
        crate::store::authority::mark_process_stopped(&mut db,"opA",&stop_a).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let fact_a=admission::record_session_stop_in_transaction(&mut db,"domainA","sessionA","opA").unwrap();
        db.execute("COMMIT").unwrap();
        assert_eq!(fact_a,stop_a.proof_hash());
        launch_a.revoke_stopped(&mut db,&root,&profile_a,"opA",&custody_a).unwrap();
        let retired_a=one(&db,"grokA","bindingA").unwrap();
        assert_eq!(retired_a.phase,"RETIRED_CLEANUP_PENDING");
        verify_retired_stopped(&db,&root,&retired_a).unwrap();
        launch_b.verify(&db,&profile_b,false).unwrap();
        let stop_b=custodian.stop(&custody_b.ticket,StopBudgets::production(),||Ok(())).unwrap();
        assert!(stop_b.errors.is_empty() &&stop_b.parent_exited &&stop_b.active_job_processes==Some(0));
        crate::store::authority::mark_process_stopped(&mut db,"opB",&stop_b).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let fact_b=admission::record_session_stop_in_transaction(&mut db,"domainA","sessionB","opB").unwrap();
        db.execute("COMMIT").unwrap();
        assert_eq!(fact_b,stop_b.proof_hash());
        launch_b.revoke_stopped(&mut db,&root,&profile_b,"opB",&custody_b).unwrap();
        assert!(inspect_grok_home_residue(&profile_a,&home.path,&home.identity,
            &[one(&db,"grokA","bindingA").unwrap().auth_identity],&[]).is_err());
        let peer_before=successor.acl(&profile_b).unwrap();
        finalize_quiescent(&mut db,&root,"grokA").unwrap();
        assert_eq!(one(&db,"grokA","bindingA").unwrap().phase,"REVOKED");
        assert!(successor.acl(&profile_a).unwrap().target_aces.is_empty());
        let peer_after=successor.acl(&profile_b).unwrap();
        assert_eq!(peer_before.target_aces,peer_after.target_aces);
        assert_eq!(peer_before.dacl_control,peer_after.dacl_control);
        assert!(inspect_grok_home_residue(&profile_a,&home.path,&home.identity,&[],&[])
            .unwrap().is_empty());
        drop(successor);drop(launch_a);drop(launch_b);drop(custodian);
        db.close_checked().unwrap();drop(root);std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn active_original_root_authority_survives_two_auth_file_replacements(){
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("grok-active-root-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        crate::store::product_database::prepare_managed_grok_acl_fixture(&root,&database,"grokA");
        let mut db=open_existing(&root,&database).unwrap();
        let home=instance::resolve_grok_original_home(&db,&root,"grokA").unwrap();
        let profile=AppContainerProfile::derived_for_test("Gogoke37.ActiveRootRotation").unwrap();
        std::fs::write(home.path.join("auth.json"),b"synthetic non-secret initial fixture").unwrap();
        let pin=super::super::runtime::current_instance_pin(&db,"grokA").unwrap();
        let claim=ClaimObservation{domain_id:"domainA".into(),session_id:"sessionA".into(),
            instance_id:"grokA".into(),home_id:"sessionHomeA".into(),
            binding_id:"bindingA".into(),generation:"1".into(),revision:2,
            phase:super::super::runtime::SessionPhase::Committed,process_operation_id:None};
        db.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('domainA','seatA','incA','USER','LONG','grokA','BUSY',1,1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingA','grokA','domainA','SESSION','sessionA','1','ACTIVE')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES('domainA','sessionA','grokA','sessionHomeA','bindingA','1','COMMITTED',2)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_seat_binding VALUES('domainA','sessionA','seatA','incA','1')").unwrap();
        let launch=GrokHomeLaunch::prepare(&mut db,&root,&profile,
            "Gogoke37.ActiveRootRotation",&claim,&pin,&home,"seatA","incA","openA").unwrap();
        let anchor=instance::read_grok_root_anchor(&db,"grokA").unwrap().unwrap();
        assert!(!anchor.source_effect_id.is_empty());
        let grant=one(&db,"grokA","bindingA").unwrap();
        db.execute(&format!("INSERT INTO main.gogoke_coordination_process_custody VALUES('oldOp','ticket','nonce','100','1000','image','{}','grokA','domainA','1','ACTIVE',NULL)",pin.digest)).unwrap();
        db.execute("UPDATE main.gogoke_v37_h_claim SET process_operation_id='oldOp' WHERE binding_id='bindingA'").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('domainA','openA','sessionA','1','00',1,'oldOp','grokA','sessionHomeA','bindingA','seatA','incA','ACTIVE')").unwrap();
        instance::bind_grok_original_process(&mut db,&grant,"oldOp","ticket","nonce",100,1000,"image").unwrap();
        launch.verify(&db,&profile,false).unwrap();
        db.execute("UPDATE main.gogoke_v37_instances SET revision=revision+1 WHERE instance_id='grokA'").unwrap();
        launch.verify(&db,&profile,false).unwrap();
        instance::advance_grok_root_anchor_registration(&mut db,"grokA").unwrap();
        assert_eq!(instance::read_grok_root_anchor(&db,"grokA").unwrap().unwrap().registration_revision,
            instance::current_grok_home_domain(&db,"grokA").unwrap().registration_revision);
        let peer_home_path=path.join("peer-session-home");
        std::fs::create_dir(&peer_home_path).unwrap();
        let peer_home_lock=RootLock::acquire(&peer_home_path).unwrap();
        let peer_home_id=peer_home_lock.canonical_root().identity.opaque();
        drop(peer_home_lock);
        db.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('domainA','seatB','incB','USER','LONG','grokA','BUSY',1,1)").unwrap();
        db.execute(&format!("INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,directory_identity,state,revision) VALUES('sessionHomeB','grokA','domainA','SESSION','sessionB','1','{peer_home_id}','ACTIVE',1)")).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingB','grokA','domainA','SESSION','sessionB','1','ACTIVE')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES('domainA','sessionB','grokA','sessionHomeB','bindingB','1','COMMITTED',2)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_seat_binding VALUES('domainA','sessionB','seatB','incB','1')").unwrap();
        let peer_claim=ClaimObservation{domain_id:"domainA".into(),session_id:"sessionB".into(),
            instance_id:"grokA".into(),home_id:"sessionHomeB".into(),
            binding_id:"bindingB".into(),generation:"1".into(),revision:2,
            phase:super::super::runtime::SessionPhase::Committed,process_operation_id:None};
        let successor_path=home.path.join("auth-next.json");
        std::fs::write(&successor_path,b"synthetic non-secret successor fixture").unwrap();
        std::fs::remove_file(home.path.join("auth.json")).unwrap();
        std::fs::rename(&successor_path,home.path.join("auth.json")).unwrap();
        let successor=observe_grok_auth_candidate(&home.path,&home.identity).unwrap();
        assert_ne!(successor.identity,grant.auth_identity);
        launch.verify(&db,&profile,false).unwrap();
        let peer_profile=AppContainerProfile::derived_for_test("Gogoke37.ActiveRootAuthorizedPeer").unwrap();
        let peer_launch=GrokHomeLaunch::prepare(&mut db,&root,&peer_profile,
            "Gogoke37.ActiveRootAuthorizedPeer",&peer_claim,&pin,&home,
            "seatB","incB","openB").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_operation VALUES('domainA','openB','6f70656e','open','sessionB','APPLIED',1,2)").unwrap();
        let program=instance::locate_bound_instance_program(&db,"grokA","grok",
            &pin.digest,&pin.version).unwrap();
        let mut custodian=ProcessCustodian::new().unwrap();
        let mut peer_process=ProcessLaunch::new(program);
        peer_process.arguments=vec!["--version".into()];
        let peer_custody=custodian.prepare(&PrepareRequest{launch:peer_process,binding:NativeBinding{
            binary_digest_sha256:pin.digest.clone(),profile_id:"grokA".into(),
            domain_id:"domainA".into(),generation:"1".into()}}).unwrap();
        crate::store::authority::record_prepared_process(&mut db,"peerOp",&peer_custody).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        admission::bind_process_operation_in_transaction(&mut db,"domainA","sessionB","peerOp").unwrap();
        db.execute("COMMIT").unwrap();
        super::super::episodes::record_initial(&db,"domainA","sessionB","openB","peerOp").unwrap();
        peer_launch.bind_process(&mut db,"peerOp",&peer_custody).unwrap();
        custodian.activate(&peer_custody).unwrap();
        crate::store::authority::mark_process_active(&mut db,"peerOp",&peer_custody).unwrap();
        super::super::episodes::mark_active(&db,"peerOp").unwrap();
        peer_launch.verify(&db,&peer_profile,false).unwrap();
        launch.verify(&db,&profile,false).unwrap();
        let current_root=grok_root_acl(&profile,&home.path,&home.identity).unwrap();
        apply_root(&mut db,&grant,effect(&grant,"GRANT_ROOT",&home.identity,".",&current_root),
            ||evidence("historical root replay",grok_root_acl(&profile,&home.path,&home.identity)),
            ||panic!("historical APPLIED root effect must not write")).unwrap();
        let peer_stop=custodian.stop(&peer_custody.ticket,StopBudgets::production(),||Ok(())).unwrap();
        assert!(peer_stop.errors.is_empty() &&peer_stop.parent_exited &&
            peer_stop.active_job_processes==Some(0));
        crate::store::authority::mark_process_stopped(&mut db,"peerOp",&peer_stop).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let peer_stop_fact=admission::record_session_stop_in_transaction(
            &mut db,"domainA","sessionB","peerOp").unwrap();
        db.execute("COMMIT").unwrap();
        assert_eq!(peer_stop_fact,peer_stop.proof_hash());
        peer_launch.revoke_stopped(&mut db,&root,&peer_profile,"peerOp",&peer_custody).unwrap();
        launch.verify(&db,&profile,false).unwrap();
        drop(peer_launch);
        db.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('domainA','seatC','incC','USER','LONG','grokA','BUSY',1,1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('oldBindingC','grokA','domainA','SESSION','sessionC','1','ACTIVE')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id,stop_fact_id) VALUES('domainA','sessionC','grokA','oldHomeC','oldBindingC','1','STOPPED',3,'oldOpC','realStopC')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_seat_binding VALUES('domainA','sessionC','seatC','incC','1')").unwrap();
        db.execute(&format!("INSERT INTO main.gogoke_coordination_process_custody VALUES('oldOpC','oldTicketC','oldNonceC','200','2000','oldImageC','{}','grokA','domainA','1','STOPPED','realStopC')",pin.digest)).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase,stop_fact_id) VALUES('domainA','oldOpenC','sessionC','1','00',1,'oldOpC','grokA','oldHomeC','oldBindingC','seatC','incC','STOPPED','realStopC')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('domainA','resumeC','sessionC','2','1','00',3,'grokA','sessionHomeC','bindingC','seatC','incC','INTENT')").unwrap();
        let resume_claim=ClaimObservation{domain_id:"domainA".into(),session_id:"sessionC".into(),
            instance_id:"grokA".into(),home_id:"sessionHomeC".into(),
            binding_id:"bindingC".into(),generation:"2".into(),revision:3,
            phase:super::super::runtime::SessionPhase::Committed,process_operation_id:None};
        let resume_profile=AppContainerProfile::derived_for_test("Gogoke37.ActiveRootResumePeer").unwrap();
        let resume_launch=GrokHomeLaunch::prepare(&mut db,&root,&resume_profile,
            "Gogoke37.ActiveRootResumePeer",&resume_claim,&pin,&home,
            "seatC","incC","resumeC").unwrap();
        db.execute(&format!("INSERT INTO main.gogoke_coordination_process_custody VALUES('newOpC','newTicketC','newNonceC','201','2001','newImageC','{}','grokA','domainA','2','PREPARED',NULL)",pin.digest)).unwrap();
        db.execute("UPDATE main.gogoke_v37_h_process_episode SET process_operation_id='newOpC',phase='PREPARED' WHERE request_id='resumeC'").unwrap();
        let resume_grant=one(&db,"grokA","bindingC").unwrap();
        instance::bind_grok_original_process(&mut db,&resume_grant,
            "newOpC","newTicketC","newNonceC",201,2001,"newImageC").unwrap();
        launch.verify(&db,&profile,false).unwrap();
        resume_launch.verify(&db,&resume_profile,false).unwrap();
        db.execute("UPDATE main.gogoke_coordination_process_custody SET stop_proof_hash='wrong' WHERE operation_id='oldOpC'").unwrap();
        assert!(launch.verify(&db,&profile,false).is_err());
        db.execute("UPDATE main.gogoke_coordination_process_custody SET stop_proof_hash='realStopC' WHERE operation_id='oldOpC'").unwrap();
        launch.verify(&db,&profile,false).unwrap();
        drop(resume_launch);
        for index in 0..2 {
            std::fs::rename(home.path.join("auth.json"),
                home.path.join(format!("auth-old-{index}.json"))).unwrap();
            std::fs::write(home.path.join("auth.json"),
                b"replacement synthetic non-secret fixture").unwrap();
            launch.verify(&db,&profile,false).unwrap();
        }
        assert_eq!(instance::read_grok_effects(&db,"bindingA").unwrap().iter()
            .filter(|effect|effect.action=="GRANT_AUTH").count(),1);
        let rogue=AppContainerProfile::derived_for_test("Gogoke37.ActiveRootUnknownPeer").unwrap();
        grant_grok_home_root(&rogue,&home.path,&home.identity).unwrap();
        assert!(launch.verify(&db,&profile,false).is_err());
        revoke_grok_home_root(&rogue,&home.path,&home.identity).unwrap();
        launch.verify(&db,&profile,false).unwrap();
        db.execute("UPDATE main.gogoke_coordination_process_custody SET custodian_nonce='wrong' WHERE operation_id='oldOp'").unwrap();
        assert!(launch.verify(&db,&profile,false).is_err());
        db.execute("UPDATE main.gogoke_coordination_process_custody SET custodian_nonce='nonce' WHERE operation_id='oldOp'").unwrap();
        let mut wrong_home=home.clone();
        wrong_home.identity=root.canonical_root().identity.clone();
        assert!(verify_active_root_authority(&db,&one(&db,"grokA","bindingA").unwrap(),
            &profile,&wrong_home).is_err());
        launch.verify(&db,&profile,false).unwrap();
        drop(launch);db.close_checked().unwrap();drop(root);std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn old_stopped_grok_grant_rebases_after_h_claim_resume_promotion(){
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("grok-promoted-rebase-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        crate::store::product_database::prepare_managed_grok_acl_fixture(&root,&database,"grokA");
        let mut db=open_existing(&root,&database).unwrap();
        let home=instance::resolve_grok_original_home(&db,&root,"grokA").unwrap();
        std::fs::write(home.path.join("auth.json"),b"synthetic non-secret original fixture").unwrap();
        let pin=super::super::runtime::current_instance_pin(&db,"grokA").unwrap();
        let program=instance::locate_bound_instance_program(&db,"grokA","grok",
            &pin.digest,&pin.version).unwrap();
        let session_home=path.join("old-session-home");
        std::fs::create_dir(&session_home).unwrap();
        let session_lock=RootLock::acquire(&session_home).unwrap();
        let session_identity=session_lock.canonical_root().identity.opaque();
        drop(session_lock);
        db.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('domainA','seatA','incA','USER','LONG','grokA','BUSY',1,1)").unwrap();
        db.execute(&format!("INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,directory_identity,state,revision) VALUES('oldHome','grokA','domainA','SESSION','sessionA','1','{session_identity}','ACTIVE',1)")).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('oldBinding','grokA','domainA','SESSION','sessionA','1','ACTIVE')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES('domainA','sessionA','grokA','oldHome','oldBinding','1','COMMITTED',2)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_seat_binding VALUES('domainA','sessionA','seatA','incA','1')").unwrap();
        let old_claim=ClaimObservation{domain_id:"domainA".into(),session_id:"sessionA".into(),
            instance_id:"grokA".into(),home_id:"oldHome".into(),binding_id:"oldBinding".into(),
            generation:"1".into(),revision:2,
            phase:super::super::runtime::SessionPhase::Committed,process_operation_id:None};
        let old_profile=AppContainerProfile::derived_for_test("Gogoke37.PromotedRebaseOld").unwrap();
        let old=GrokHomeLaunch::prepare(&mut db,&root,&old_profile,
            "Gogoke37.PromotedRebaseOld",&old_claim,&pin,&home,"seatA","incA","openOld").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_operation VALUES('domainA','openOld','6f70656e','open','sessionA','APPLIED',1,2)").unwrap();
        let mut custodian=ProcessCustodian::new().unwrap();
        let mut old_process=ProcessLaunch::new(program.clone());
        old_process.arguments=vec!["--version".into()];
        let old_binding=NativeBinding{binary_digest_sha256:pin.digest.clone(),
            profile_id:"grokA".into(),domain_id:"domainA".into(),generation:"1".into()};
        let old_custody=custodian.prepare(&PrepareRequest{
            launch:old_process,binding:old_binding}).unwrap();
        crate::store::authority::record_prepared_process(&mut db,"oldOp",&old_custody).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        admission::bind_process_operation_in_transaction(&mut db,"domainA","sessionA","oldOp").unwrap();
        db.execute("COMMIT").unwrap();
        super::super::episodes::record_initial(&db,"domainA","sessionA","openOld","oldOp").unwrap();
        old.bind_process(&mut db,"oldOp",&old_custody).unwrap();
        custodian.activate(&old_custody).unwrap();
        crate::store::authority::mark_process_active(&mut db,"oldOp",&old_custody).unwrap();
        super::super::episodes::mark_active(&db,"oldOp").unwrap();
        let old_stop=custodian.stop(&old_custody.ticket,StopBudgets::production(),||Ok(())).unwrap();
        assert!(old_stop.errors.is_empty() &&old_stop.parent_exited &&
            old_stop.active_job_processes==Some(0));
        crate::store::authority::mark_process_stopped(&mut db,"oldOp",&old_stop).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let old_stop_fact=admission::record_session_stop_in_transaction(
            &mut db,"domainA","sessionA","oldOp").unwrap();
        db.execute("COMMIT").unwrap();
        assert_eq!(old_stop_fact,old_stop.proof_hash());
        old.revoke_stopped(&mut db,&root,&old_profile,"oldOp",&old_custody).unwrap();
        assert_eq!(one(&db,"grokA","oldBinding").unwrap().phase,"REVOKED");
        let old_effects=instance::read_grok_effects(&db,"oldBinding").unwrap();
        let old_episode=Statement::prepare(db.as_ptr(),"SELECT phase,stop_fact_id FROM main.gogoke_v37_h_process_episode WHERE process_operation_id='oldOp'").unwrap();
        assert!(old_episode.step_row().unwrap());
        assert_eq!(old_episode.column_text(0).unwrap(),"STOPPED");
        assert_eq!(old_episode.column_text(1).unwrap(),old_stop_fact);
        drop(old_episode);
        drop(old);
        let resume_home=path.join("resume-session-home");
        std::fs::create_dir(&resume_home).unwrap();
        let resume_lock=RootLock::acquire(&resume_home).unwrap();
        let resume_identity=resume_lock.canonical_root().identity.opaque();
        drop(resume_lock);
        db.execute(&format!("INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,directory_identity,state,revision) VALUES('resumeHome','grokA','domainA','SESSION','sessionA','2','{resume_identity}','ACTIVE',1)")).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('resumeBinding','grokA','domainA','SESSION','sessionA','2','ACTIVE')").unwrap();
        super::super::episodes::begin_resume(&db,"domainA","sessionA","resumeA",
            b"fixture original resume","1","2",3,"resumeHome","resumeBinding").unwrap();
        let resume_claim=ClaimObservation{domain_id:"domainA".into(),session_id:"sessionA".into(),
            instance_id:"grokA".into(),home_id:"resumeHome".into(),binding_id:"resumeBinding".into(),
            generation:"2".into(),revision:3,
            phase:super::super::runtime::SessionPhase::Committed,process_operation_id:None};
        let resume_profile=AppContainerProfile::derived_for_test("Gogoke37.PromotedRebaseResume").unwrap();
        let resume=GrokHomeLaunch::prepare(&mut db,&root,&resume_profile,
            "Gogoke37.PromotedRebaseResume",&resume_claim,&pin,&home,
            "seatA","incA","resumeA").unwrap();
        let mut resume_process=ProcessLaunch::new(program);
        resume_process.arguments=vec!["--version".into()];
        let resume_binding=NativeBinding{binary_digest_sha256:pin.digest.clone(),
            profile_id:"grokA".into(),domain_id:"domainA".into(),generation:"2".into()};
        let resume_custody=custodian.prepare(&PrepareRequest{
            launch:resume_process,binding:resume_binding}).unwrap();
        crate::store::authority::record_prepared_process(&mut db,"resumeOp",&resume_custody).unwrap();
        super::super::episodes::attach_resume_process(&db,"domainA","resumeA","resumeOp").unwrap();
        resume.bind_process(&mut db,"resumeOp",&resume_custody).unwrap();
        custodian.activate(&resume_custody).unwrap();
        crate::store::authority::mark_process_active(&mut db,"resumeOp",&resume_custody).unwrap();
        // The production promotion requires the vendor's original ACP resume
        // ACK; this fixture retains the same already-proven H row/episode
        // transition after two real kernel custodians and a real old StopFact.
        db.execute("UPDATE main.gogoke_v37_seats SET generation=2,revision=revision+1 WHERE domain_id='domainA' AND seat_id='seatA' AND incarnation='incA' AND generation=1 AND state='BUSY'").unwrap();
        db.execute("UPDATE main.gogoke_v37_h_seat_binding SET generation='2' WHERE domain_id='domainA' AND session_id='sessionA' AND generation='1'").unwrap();
        db.execute(&format!("UPDATE main.gogoke_v37_h_claim SET generation='2',home_id='resumeHome',binding_id='resumeBinding',process_operation_id='resumeOp',stop_fact_id=NULL,state='COMMITTED',revision=revision+1 WHERE domain_id='domainA' AND session_id='sessionA' AND generation='1' AND binding_id='oldBinding' AND process_operation_id='oldOp' AND state='STOPPED' AND revision=3 AND stop_fact_id='{old_stop_fact}'")).unwrap();
        let changed=Statement::prepare(db.as_ptr(),"SELECT changes()").unwrap();
        assert!(changed.step_row().unwrap());
        assert_eq!(changed.column_text(0).unwrap(),"1");
        drop(changed);
        db.execute("UPDATE main.gogoke_v37_h_process_episode SET phase='ACTIVE',result_revision=4 WHERE process_operation_id='resumeOp' AND phase='PREPARED' AND old_generation='1'").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_generation VALUES('domainA','sessionA','2','resumeA','resumeOp')").unwrap();
        let promoted=Statement::prepare(db.as_ptr(),"SELECT binding_id,generation,CASE WHEN stop_fact_id IS NULL THEN '1' ELSE '0' END FROM main.gogoke_v37_h_claim WHERE domain_id='domainA' AND session_id='sessionA'").unwrap();
        assert!(promoted.step_row().unwrap());
        assert_eq!(promoted.column_text(0).unwrap(),"resumeBinding");
        assert_eq!(promoted.column_text(1).unwrap(),"2");
        assert_eq!(promoted.column_text(2).unwrap(),"1");
        drop(promoted);
        let stopped_old=Statement::prepare(db.as_ptr(),"SELECT e.stop_fact_id,c.stop_proof_hash FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id WHERE e.process_operation_id='oldOp'").unwrap();
        assert!(stopped_old.step_row().unwrap());
        assert_eq!(stopped_old.column_text(0).unwrap(),old_stop_fact);
        assert_eq!(stopped_old.column_text(1).unwrap(),old_stop_fact);
        drop(stopped_old);
        let resume_stop=custodian.stop(&resume_custody.ticket,StopBudgets::production(),||Ok(())).unwrap();
        assert!(resume_stop.errors.is_empty() &&resume_stop.parent_exited &&
            resume_stop.active_job_processes==Some(0));
        crate::store::authority::mark_process_stopped(&mut db,"resumeOp",&resume_stop).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let resume_stop_fact=admission::record_session_stop_in_transaction(
            &mut db,"domainA","sessionA","resumeOp").unwrap();
        db.execute("COMMIT").unwrap();
        assert_eq!(resume_stop_fact,resume_stop.proof_hash());
        resume.revoke_stopped(&mut db,&root,&resume_profile,"resumeOp",&resume_custody).unwrap();
        assert_eq!(one(&db,"grokA","resumeBinding").unwrap().phase,"REVOKED");
        drop(resume);
        let final_home=path.join("final-session-home");
        std::fs::create_dir(&final_home).unwrap();
        let final_lock=RootLock::acquire(&final_home).unwrap();
        let final_identity=final_lock.canonical_root().identity.opaque();
        drop(final_lock);
        db.execute(&format!("INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,directory_identity,state,revision) VALUES('finalHome','grokA','domainA','SESSION','sessionA','3','{final_identity}','ACTIVE',1)")).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('finalBinding','grokA','domainA','SESSION','sessionA','3','ACTIVE')").unwrap();
        super::super::episodes::begin_resume(&db,"domainA","sessionA","resumeFinal",
            b"fixture final resume","2","3",5,"finalHome","finalBinding").unwrap();
        let final_claim=ClaimObservation{domain_id:"domainA".into(),session_id:"sessionA".into(),
            instance_id:"grokA".into(),home_id:"finalHome".into(),binding_id:"finalBinding".into(),
            generation:"3".into(),revision:5,
            phase:super::super::runtime::SessionPhase::Committed,process_operation_id:None};
        let final_profile=AppContainerProfile::derived_for_test("Gogoke37.PromotedRebaseFinal").unwrap();
        db.execute("DELETE FROM main.gogoke_v37_grok_home_root_anchor WHERE instance_id='grokA'").unwrap();
        let acl_before=grok_root_acl(&final_profile,&home.path,&home.identity).unwrap();
        db.execute("UPDATE main.gogoke_coordination_process_custody SET stop_proof_hash='wrong' WHERE operation_id='oldOp'").unwrap();
        assert!(GrokHomeLaunch::prepare(&mut db,&root,&final_profile,
            "Gogoke37.PromotedRebaseFinal",&final_claim,&pin,&home,
            "seatA","incA","resumeFinal").is_err());
        assert!(instance::read_grok_root_anchor(&db,"grokA").unwrap().is_none());
        assert!(instance::read_grok_grants(&db,"grokA").unwrap().iter()
            .all(|grant|grant.binding_id!="finalBinding"));
        assert_eq!(grok_root_acl(&final_profile,&home.path,&home.identity).unwrap(),acl_before);
        db.execute(&format!("UPDATE main.gogoke_coordination_process_custody SET stop_proof_hash='{}' WHERE operation_id='oldOp'",old_stop_fact)).unwrap();
        let final_launch=GrokHomeLaunch::prepare(&mut db,&root,&final_profile,
            "Gogoke37.PromotedRebaseFinal",&final_claim,&pin,&home,
            "seatA","incA","resumeFinal").unwrap();
        assert!(!instance::read_grok_root_anchor(&db,"grokA").unwrap().unwrap()
            .baseline_effect_id.is_empty());
        assert_eq!(instance::read_grok_effects(&db,"oldBinding").unwrap(),old_effects);
        final_launch.revoke_uncreated(&mut db,&root,&final_profile).unwrap();
        drop(final_launch);
        db.close_checked().unwrap();drop(root);std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn inherited_successor_intent_recovers_after_acl_write_before_finish(){
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("grok-successor-intent-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let mut db=create_new(&root,&path.join("state.sqlite")).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        crate::store::authority::initialize_profile(&mut db,&root).unwrap();
        crate::store::authority::initialize_process_custody_schema(&mut db).unwrap();
        instance::initialize_schema(&mut db).unwrap();
        instance::initialize_grok_home_grant_schema(&mut db).unwrap();
        crate::store::seat::initialize_schema(&mut db).unwrap();
        super::super::admission::initialize_admission_schema(&mut db).unwrap();
        super::super::session_binding::initialize_schema(&mut db).unwrap();
        let powershell=Path::new(&std::env::var("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let program=instance::ProgramObservation::observe(&powershell,"1.0.41").unwrap();
        instance::register_instance(&mut db,&root,&instance::Registration{
            request_id:"registerGrok",request_bytes:b"registered physical Grok HOME",
            instance_id:"grokA",driver_id:"grok",program:&program}).unwrap();
        let home=instance::resolve_grok_original_home(&db,&root,"grokA").unwrap();
        let profile=AppContainerProfile::derived_for_test("Gogoke37.GrokSuccessorIntent").unwrap();
        let home_id=home.identity.clone();
        let program_digest=crate::store::digest::content_hash(&std::fs::read(&powershell).unwrap());
        let root_id=root.canonical_root().identity.opaque();
        db.execute(&format!("INSERT INTO main.gogoke_v37_grok_home_domains VALUES(
            'grokA','{root_id}','{}','{program_digest}','1.0.41',1,1)",home_id.opaque())).unwrap();
        db.execute(&format!("INSERT INTO main.gogoke_v37_grok_home_grants(
            binding_id,instance_id,domain_id,session_id,seat_id,seat_incarnation,
            generation,request_id,profile_name,profile_sid,program_digest,
            home_identity,auth_identity,phase,revision)
            VALUES('bindingA','grokA','domainA','sessionA','seatA','incA','1','openA',
            'Gogoke37.GrokSuccessorIntent','{}','{program_digest}','{}','{}','GRANTED_UNCREATED',1)",
            profile.sid_identity().unwrap(),home_id.opaque(),home_id.opaque())).unwrap();
        let grant=one(&db,"grokA","bindingA").unwrap();
        let root_before=grok_root_acl(&profile,&home.path,&home_id).unwrap();
        let root_intent=effect(&grant,"GRANT_ROOT",&home_id,".",&root_before);
        apply(&mut db,root_intent,||evidence("test root ACL",grok_root_acl(&profile,&home.path,&home_id)),||
            evidence("test root grant",grant_grok_home_root(&profile,&home.path,&home_id))).unwrap();
        std::fs::write(home.path.join("auth.json"),b"synthetic non-secret fixture").unwrap();
        let auth=observe_grok_auth_candidate(&home.path,&home_id).unwrap();
        let before=auth.candidate_acl(&profile).unwrap();
        assert!(!before.dacl_protected);assert!(!before.target_aces.is_empty());
        db.execute(&format!("UPDATE main.gogoke_v37_grok_home_grants SET auth_identity='{}' WHERE binding_id='bindingA'",
            auth.identity.opaque())).unwrap();
        let grant=one(&db,"grokA","bindingA").unwrap();
        let mut expected=effect(&grant,"GRANT_AUTH",&auth.identity,"auth.json",&before);
        expected.after_control=before.dacl_control|0x1000;
        instance::begin_grok_effect(&mut db,&expected).unwrap();
        grant_grok_auth_successor(&profile,&auth).unwrap();
        // Model a crash after the exact ACL write, with the original INTENT
        // still durable. Replay must read AFTER and finish without another write.
        db.close_checked().unwrap();
        let mut db=crate::store::same_open::open_existing(&root,&path.join("state.sqlite")).unwrap();
        apply(&mut db,expected,||evidence("test current",auth.candidate_acl(&profile)),
            ||panic!("AFTER replay must not write the ACL again")).unwrap();
        let recovered=instance::read_grok_effects(&db,"bindingA").unwrap();
        assert_eq!(recovered.len(),2);
        assert!(recovered.iter().all(|effect|effect.phase=="APPLIED"));
        assert!(recovered.iter().any(|effect|effect.action=="GRANT_AUTH" &&effect.revision==2));
        assert!(before.preserves_other_aces(&auth.acl(&profile).unwrap()));
        verify_grok_auth(&profile,&auth).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('domainA','seatA','incA','USER','LONG','grokA','BUSY',1,1)").unwrap();
        db.execute(&format!("INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,directory_identity,state,revision) VALUES('sessionHomeA','grokA','domainA','SESSION','sessionA','1','{}','ACTIVE',1)",home_id.opaque())).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingA','grokA','domainA','SESSION','sessionA','1','ACTIVE')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES('domainA','sessionA','grokA','sessionHomeA','bindingA','1','COMMITTED',2)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_seat_binding VALUES('domainA','sessionA','seatA','incA','1')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_operation VALUES('domainA','openA','6f70656e','open','sessionA','APPLIED',1,2)").unwrap();
        let mut launch=ProcessLaunch::new(powershell.clone());
        launch.arguments=vec!["-NoProfile".into(),"-NonInteractive".into(),"-Command".into(),
            "while ($true) { Start-Sleep -Milliseconds 100 }".into()];
        let binding=NativeBinding{binary_digest_sha256:program_digest.clone(),profile_id:"grokA".into(),
            domain_id:"domainA".into(),generation:"1".into()};
        let mut custodian=ProcessCustodian::new().unwrap();
        let prepared=custodian.prepare(&PrepareRequest{launch,binding}).unwrap();
        crate::store::authority::record_prepared_process(&mut db,"oldOp",&prepared).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        admission::bind_process_operation_in_transaction(&mut db,"domainA","sessionA","oldOp").unwrap();
        db.execute("COMMIT").unwrap();
        super::super::episodes::record_initial(&db,"domainA","sessionA","openA","oldOp").unwrap();
        let grant=one(&db,"grokA","bindingA").unwrap();
        instance::bind_grok_original_process(&mut db,&grant,"oldOp",prepared.ticket.opaque(),
            &prepared.custodian_nonce,prepared.identity.pid,prepared.identity.creation_time_100ns,
            &prepared.identity.image_path.to_string_lossy()).unwrap();
        custodian.activate(&prepared).unwrap();
        crate::store::authority::mark_process_active(&mut db,"oldOp",&prepared).unwrap();
        super::super::episodes::mark_active(&db,"oldOp").unwrap();

        // A second physical successor stops before finish. The real native
        // process/custody stop writers bind one StopFact through H and F.
        std::fs::rename(home.path.join("auth.json"),home.path.join("auth-first.json")).unwrap();
        std::fs::write(home.path.join("auth.json"),b"second synthetic non-secret fixture").unwrap();
        let second=observe_grok_auth_candidate(&home.path,&home_id).unwrap();
        let second_before=second.candidate_acl(&profile).unwrap();
        assert!(!second_before.dacl_protected);assert!(!second_before.target_aces.is_empty());
        let second_id=second.identity.clone();
        let mut second_intent=effect(&grant,"GRANT_AUTH",&second.identity,"auth.json",&second_before);
        second_intent.after_control=second_before.dacl_control|0x1000;
        instance::begin_grok_effect(&mut db,&second_intent).unwrap();
        grant_grok_auth_successor(&profile,&second).unwrap();

        let stop=custodian.stop(&prepared.ticket,StopBudgets::production(),||Ok(())).unwrap();
        assert!(stop.errors.is_empty() && stop.parent_exited && stop.active_job_processes==Some(0));
        let durable_revision=crate::store::authority::mark_process_stopped(&mut db,"oldOp",&stop).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let stop_fact=admission::record_session_stop_in_transaction(&mut db,"domainA","sessionA","oldOp").unwrap();
        db.execute("COMMIT").unwrap();
        assert_eq!(stop_fact,stop.proof_hash());
        let release=AdmissionRequest{domain_id:"domainA",session_id:"sessionA",request_id:"releaseA",
            raw_bytes:b"fixture release after persisted stop",instance_id:"grokA",home_id:"sessionHomeA",
            generation:"1",expected_revision:3};
        assert_eq!(admission::release_admission(&mut db,&release,|connection|{
            if exact_exists(connection,"test active H owner binding",
                "SELECT 1 FROM main.gogoke_v37_h_owner_binding WHERE binding_id=?1 AND instance_id=?2 AND domain_id=?3 AND owner_id=?4 AND generation=?5 AND state='ACTIVE'",
                &["bindingA","grokA","domainA","sessionA","1"]).unwrap() {
                Ok(())
            } else { Err(admission::AdmissionError::Denied) }
        }).unwrap(),AdmissionResult::Applied(4));
        assert_eq!(custodian.confirm_stop_durable(&crate::process::DurableStopConfirmation{
            ticket:prepared.ticket.clone(),custodian_nonce:prepared.custodian_nonce.clone(),
            identity:prepared.identity.clone(),proof_hash:stop.proof_hash(),durable_revision
        }).unwrap().proof_hash(),stop_fact);
        let active_grant=one(&db,"grokA","bindingA").unwrap();
        instance::set_grok_grant_phase(&mut db,&active_grant,"REVOKE_PENDING",Some(&stop_fact)).unwrap();
        let stopped_claim=Statement::prepare(db.as_ptr(),"SELECT state,stop_fact_id FROM main.gogoke_v37_h_claim WHERE domain_id='domainA' AND session_id='sessionA'").unwrap();
        assert!(stopped_claim.step_row().unwrap());
        assert_eq!((stopped_claim.column_text(0).unwrap(),stopped_claim.column_text(1).unwrap()),("RELEASED".into(),stop_fact.clone()));
        drop(stopped_claim);drop(custodian);drop(auth);drop(second);
        db.close_checked().unwrap();
        let mut db=crate::store::same_open::open_existing(&root,&path.join("state.sqlite")).unwrap();
        let grant=one(&db,"grokA","bindingA").unwrap();
        assert_eq!(grant.phase,"REVOKE_PENDING");
        assert_eq!(grant.stop_fact_id.as_deref(),Some(stop_fact.as_str()));
        let effects_before=instance::read_grok_effects(&db,"bindingA").unwrap();
        let root_before=grok_root_acl(&profile,&home.path,&home_id).unwrap();
        let current_before=observe_grok_auth_candidate(&home.path,&home_id).unwrap().candidate_acl(&profile).unwrap();

        db.execute("UPDATE main.gogoke_v37_h_process_episode SET stop_fact_id='wrong-stop-fact' WHERE request_id='openA'").unwrap();
        assert!(resume_stopped_revoke(&mut db,&root,&grant).is_err());
        assert_eq!(one(&db,"grokA","bindingA").unwrap(),grant);
        assert_eq!(instance::read_grok_effects(&db,"bindingA").unwrap(),effects_before);
        assert_eq!(grok_root_acl(&profile,&home.path,&home_id).unwrap(),root_before);
        assert_eq!(observe_grok_auth_candidate(&home.path,&home_id).unwrap().candidate_acl(&profile).unwrap(),current_before);
        db.execute("UPDATE main.gogoke_v37_h_process_episode SET stop_fact_id=(SELECT stop_fact_id FROM main.gogoke_v37_h_claim WHERE domain_id='domainA' AND session_id='sessionA') WHERE request_id='openA'").unwrap();

        db.execute("UPDATE main.gogoke_coordination_process_custody SET ticket='wrong-ticket' WHERE operation_id='oldOp'").unwrap();
        assert!(resume_stopped_revoke(&mut db,&root,&grant).is_err());
        assert_eq!(one(&db,"grokA","bindingA").unwrap(),grant);
        assert_eq!(instance::read_grok_effects(&db,"bindingA").unwrap(),effects_before);
        assert_eq!(grok_root_acl(&profile,&home.path,&home_id).unwrap(),root_before);
        assert_eq!(observe_grok_auth_candidate(&home.path,&home_id).unwrap().candidate_acl(&profile).unwrap(),current_before);
        db.execute(&format!("UPDATE main.gogoke_coordination_process_custody SET ticket='{}' WHERE operation_id='oldOp'",prepared.ticket.opaque())).unwrap();

        db.execute("UPDATE main.gogoke_coordination_process_custody SET state='ACTIVE' WHERE operation_id='oldOp'").unwrap();
        assert!(resume_stopped_revoke(&mut db,&root,&grant).is_err());
        assert_eq!(one(&db,"grokA","bindingA").unwrap(),grant);
        assert_eq!(instance::read_grok_effects(&db,"bindingA").unwrap(),effects_before);
        assert_eq!(grok_root_acl(&profile,&home.path,&home_id).unwrap(),root_before);
        assert_eq!(observe_grok_auth_candidate(&home.path,&home_id).unwrap().candidate_acl(&profile).unwrap(),current_before);
        db.execute("UPDATE main.gogoke_coordination_process_custody SET state='STOPPED' WHERE operation_id='oldOp'").unwrap();

        db.close_checked().unwrap();
        let mut db=crate::store::same_open::open_existing(&root,&path.join("state.sqlite")).unwrap();
        let original=one(&db,"grokA","bindingA").unwrap();
        resume_stopped_revoke(&mut db,&root,&original).unwrap();
        let revoked=one(&db,"grokA","bindingA").unwrap();
        assert_eq!(revoked.phase,"REVOKED");assert_eq!(revoked.stop_fact_id.as_deref(),Some(stop_fact.as_str()));
        let hclaim=Statement::prepare(db.as_ptr(),"SELECT state,stop_fact_id FROM main.gogoke_v37_h_claim WHERE domain_id='domainA' AND session_id='sessionA'").unwrap();
        assert!(hclaim.step_row().unwrap());
        assert_eq!((hclaim.column_text(0).unwrap(),hclaim.column_text(1).unwrap()),("RELEASED".into(),stop_fact.clone()));
        let custody=Statement::prepare(db.as_ptr(),"SELECT state,stop_proof_hash FROM main.gogoke_coordination_process_custody WHERE operation_id='oldOp'").unwrap();
        assert!(custody.step_row().unwrap());
        assert_eq!((custody.column_text(0).unwrap(),custody.column_text(1).unwrap()),("STOPPED".into(),stop_fact.clone()));
        let episode=Statement::prepare(db.as_ptr(),"SELECT phase,stop_fact_id FROM main.gogoke_v37_h_process_episode WHERE request_id='openA'").unwrap();
        assert!(episode.step_row().unwrap());
        assert_eq!((episode.column_text(0).unwrap(),episode.column_text(1).unwrap()),("STOPPED".into(),stop_fact.clone()));
        let effects=instance::read_grok_effects(&db,"bindingA").unwrap();
        assert!(effects.iter().all(|effect|effect.phase=="APPLIED"));
        assert_eq!(effects.iter().filter(|effect|effect.action=="REVOKE_AUTH").count(),2);
        assert!(inspect_grok_home_residue(&profile,&home.path,&home_id,&[],&[]).unwrap().is_empty());
        drop(hclaim);drop(custody);drop(episode);
        db.close_checked().unwrap();drop(profile);drop(root);
        std::fs::remove_dir_all(path).unwrap();
    }

    fn deleted_old_auth_cold_recovery_case(revoke_old_auth:bool){
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("grok-deleted-auth-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let mut db=create_new(&root,&path.join("state.sqlite")).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        crate::store::authority::initialize_profile(&mut db,&root).unwrap();
        crate::store::authority::initialize_process_custody_schema(&mut db).unwrap();
        instance::initialize_schema(&mut db).unwrap();
        instance::initialize_grok_home_grant_schema(&mut db).unwrap();
        crate::store::seat::initialize_schema(&mut db).unwrap();
        super::super::admission::initialize_admission_schema(&mut db).unwrap();
        super::super::session_binding::initialize_schema(&mut db).unwrap();
        let powershell=Path::new(&std::env::var("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let program=instance::ProgramObservation::observe(&powershell,"1.0.41").unwrap();
        instance::register_instance(&mut db,&root,&instance::Registration{
            request_id:"registerGrok",request_bytes:b"registered physical Grok HOME",
            instance_id:"grokA",driver_id:"grok",program:&program}).unwrap();
        let home=instance::resolve_grok_original_home(&db,&root,"grokA").unwrap();
        let profile=AppContainerProfile::derived_for_test("Gogoke37.GrokDeletedOldAuth").unwrap();
        let home_id=home.identity.clone();
        let program_digest=crate::store::digest::content_hash(&std::fs::read(&powershell).unwrap());
        let root_id=root.canonical_root().identity.opaque();
        db.execute(&format!("INSERT INTO main.gogoke_v37_grok_home_domains VALUES(
            'grokA','{root_id}','{}','{program_digest}','1.0.41',1,1)",home_id.opaque())).unwrap();
        db.execute(&format!("INSERT INTO main.gogoke_v37_grok_home_grants(
            binding_id,instance_id,domain_id,session_id,seat_id,seat_incarnation,
            generation,request_id,profile_name,profile_sid,program_digest,
            home_identity,auth_identity,phase,revision)
            VALUES('bindingA','grokA','domainA','sessionA','seatA','incA','1','openA',
            'Gogoke37.GrokDeletedOldAuth','{}','{program_digest}','{}','{}','GRANTED_UNCREATED',1)",
            profile.sid_identity().unwrap(),home_id.opaque(),home_id.opaque())).unwrap();
        let grant=one(&db,"grokA","bindingA").unwrap();
        let root_before=grok_root_acl(&profile,&home.path,&home_id).unwrap();
        apply(&mut db,effect(&grant,"GRANT_ROOT",&home_id,".",&root_before),
            ||evidence("test root ACL",grok_root_acl(&profile,&home.path,&home_id)),||
            evidence("test root grant",grant_grok_home_root(&profile,&home.path,&home_id))).unwrap();
        std::fs::write(home.path.join("auth.json"),b"synthetic non-secret fixture").unwrap();
        let old=observe_grok_auth_candidate(&home.path,&home_id).unwrap();
        let old_id=old.identity.clone();
        let old_before=old.candidate_acl(&profile).unwrap();
        assert!(!old_before.dacl_protected && !old_before.target_aces.is_empty());
        db.execute(&format!("UPDATE main.gogoke_v37_grok_home_grants SET auth_identity='{}' WHERE binding_id='bindingA'",
            old_id.opaque())).unwrap();
        let grant=one(&db,"grokA","bindingA").unwrap();
        let mut old_grant=effect(&grant,"GRANT_AUTH",&old_id,"auth.json",&old_before);
        old_grant.after_control=old_before.dacl_control|0x1000;
        apply(&mut db,old_grant,||evidence("old auth grant readback",old.candidate_acl(&profile)),||
            evidence("grant old auth",grant_grok_auth_successor(&profile,&old))).unwrap();
        verify_grok_auth(&profile,&old).unwrap();

        db.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('domainA','seatA','incA','USER','LONG','grokA','BUSY',1,1)").unwrap();
        db.execute(&format!("INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,directory_identity,state,revision) VALUES('sessionHomeA','grokA','domainA','SESSION','sessionA','1','{}','ACTIVE',1)",home_id.opaque())).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingA','grokA','domainA','SESSION','sessionA','1','ACTIVE')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES('domainA','sessionA','grokA','sessionHomeA','bindingA','1','COMMITTED',2)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_seat_binding VALUES('domainA','sessionA','seatA','incA','1')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_operation VALUES('domainA','openA','6f70656e','open','sessionA','APPLIED',1,2)").unwrap();
        let mut launch=ProcessLaunch::new(powershell.clone());
        launch.arguments=vec!["-NoProfile".into(),"-NonInteractive".into(),"-Command".into(),
            "while ($true) { Start-Sleep -Milliseconds 100 }".into()];
        let binding=NativeBinding{binary_digest_sha256:program_digest,profile_id:"grokA".into(),
            domain_id:"domainA".into(),generation:"1".into()};
        let mut custodian=ProcessCustodian::new().unwrap();
        let prepared=custodian.prepare(&PrepareRequest{launch,binding}).unwrap();
        crate::store::authority::record_prepared_process(&mut db,"oldOp",&prepared).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        admission::bind_process_operation_in_transaction(&mut db,"domainA","sessionA","oldOp").unwrap();
        db.execute("COMMIT").unwrap();
        super::super::episodes::record_initial(&db,"domainA","sessionA","openA","oldOp").unwrap();
        let grant=one(&db,"grokA","bindingA").unwrap();
        instance::bind_grok_original_process(&mut db,&grant,"oldOp",prepared.ticket.opaque(),
            &prepared.custodian_nonce,prepared.identity.pid,prepared.identity.creation_time_100ns,
            &prepared.identity.image_path.to_string_lossy()).unwrap();
        custodian.activate(&prepared).unwrap();
        crate::store::authority::mark_process_active(&mut db,"oldOp",&prepared).unwrap();
        super::super::episodes::mark_active(&db,"oldOp").unwrap();
        let stop=custodian.stop(&prepared.ticket,StopBudgets::production(),||Ok(())).unwrap();
        assert!(stop.errors.is_empty() && stop.parent_exited && stop.active_job_processes==Some(0));
        let durable_revision=crate::store::authority::mark_process_stopped(&mut db,"oldOp",&stop).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let stop_fact=admission::record_session_stop_in_transaction(&mut db,"domainA","sessionA","oldOp").unwrap();
        db.execute("COMMIT").unwrap();
        assert_eq!(stop_fact,stop.proof_hash());
        let release=AdmissionRequest{domain_id:"domainA",session_id:"sessionA",request_id:"releaseA",
            raw_bytes:b"fixture release after persisted stop",instance_id:"grokA",home_id:"sessionHomeA",
            generation:"1",expected_revision:3};
        assert_eq!(admission::release_admission(&mut db,&release,|connection|{
            if exact_exists(connection,"test active H owner binding",
                "SELECT 1 FROM main.gogoke_v37_h_owner_binding WHERE binding_id=?1 AND instance_id=?2 AND domain_id=?3 AND owner_id=?4 AND generation=?5 AND state='ACTIVE'",
                &["bindingA","grokA","domainA","sessionA","1"]).unwrap() {
                Ok(())
            } else { Err(admission::AdmissionError::Denied) }
        }).unwrap(),AdmissionResult::Applied(4));
        assert_eq!(custodian.confirm_stop_durable(&crate::process::DurableStopConfirmation{
            ticket:prepared.ticket.clone(),custodian_nonce:prepared.custodian_nonce.clone(),
            identity:prepared.identity.clone(),proof_hash:stop.proof_hash(),durable_revision
        }).unwrap().proof_hash(),stop_fact);
        let active=one(&db,"grokA","bindingA").unwrap();
        instance::set_grok_grant_phase(&mut db,&active,"REVOKE_PENDING",Some(&stop_fact)).unwrap();
        let pending=one(&db,"grokA","bindingA").unwrap();

        if revoke_old_auth {
            let old_before=old.acl(&profile).unwrap();
            apply(&mut db,effect(&pending,"REVOKE_AUTH",&old_id,"auth.json",&old_before),
                ||evidence("old revoke readback",old.acl(&profile)),||
                evidence("revoke old auth",revoke_grok_auth(&profile,&old))).unwrap();
            assert!(old.acl(&profile).unwrap().target_aces.is_empty());
        }
        // The successor inherits the old HOME grant while the deleted old
        // FileID is still held. It is intentionally not a GRANT_AUTH successor.
        std::fs::remove_file(home.path.join("auth.json")).unwrap();
        std::fs::write(home.path.join("auth.json"),b"replacement synthetic non-secret fixture").unwrap();
        let successor=observe_grok_auth_candidate(&home.path,&home_id).unwrap();
        assert_ne!(successor.identity,old_id);
        let successor_before=successor.candidate_acl(&profile).unwrap();
        assert!(!successor_before.dacl_protected && !successor_before.target_aces.is_empty());
        let root_before=grok_root_acl(&profile,&home.path,&home_id).unwrap();
        apply(&mut db,effect(&pending,"REVOKE_ROOT",&home_id,".",&root_before),
            ||evidence("test root revoke readback",grok_root_acl(&profile,&home.path,&home_id)),||
            evidence("test root revoke",revoke_grok_home_root(&profile,&home.path,&home_id))).unwrap();
        assert!(grok_root_acl(&profile,&home.path,&home_id).unwrap().target_aces.is_empty());
        // Win32 ROOT revoke can propagate removal to this unprotected child.
        // Cold recovery must use the actual child ACL, not assume a residue.
        drop(old);drop(successor);drop(custodian);
        assert!(observe_grok_recorded_auth(&home.path,&home_id,&old_id).unwrap().is_none());
        let effects_before=instance::read_grok_effects(&db,"bindingA").unwrap();
        assert!(effects_before.iter().any(|effect|effect.action=="GRANT_AUTH" &&
            effect.object_identity==old_id &&effect.phase=="APPLIED"));
        assert!(effects_before.iter().any(|effect|effect.action=="REVOKE_ROOT" &&
            effect.object_identity==home_id &&effect.phase=="APPLIED"));
        assert_eq!(effects_before.iter().filter(|effect|effect.action=="REVOKE_AUTH" &&
            effect.object_identity==old_id && effect.phase=="APPLIED").count(),
            if revoke_old_auth {1}else{0});
        db.close_checked().unwrap();
        let mut db=crate::store::same_open::open_existing(&root,&path.join("state.sqlite")).unwrap();
        let original=one(&db,"grokA","bindingA").unwrap();
        assert_eq!(original.phase,"REVOKE_PENDING");
        assert_eq!(original.stop_fact_id.as_deref(),Some(stop_fact.as_str()));
        let root_acl_before=grok_root_acl(&profile,&home.path,&home_id).unwrap();
        let successor_acl_before=observe_grok_auth_candidate(&home.path,&home_id).unwrap().candidate_acl(&profile).unwrap();
        if revoke_old_auth {
            resume_stopped_revoke(&mut db,&root,&original).unwrap();
            assert_eq!(one(&db,"grokA","bindingA").unwrap().phase,"REVOKED");
            let effects=instance::read_grok_effects(&db,"bindingA").unwrap();
            assert!(effects.iter().all(|effect|effect.phase=="APPLIED"));
            for original_effect in &effects_before {
                assert_eq!(effects.iter().find(|effect|effect.effect_id==original_effect.effect_id),
                    Some(original_effect),"cold ingress must preserve each original completed effect and revision");
            }
            assert_eq!(effects.iter().filter(|effect|effect.action=="REVOKE_AUTH" &&
                effect.object_identity==old_id).count(),1);
            let has_residue_effect=effects.iter().any(|effect|effect.action=="REVOKE_RESIDUE" &&
                effect.object_identity==successor_acl_before.identity &&effect.phase=="APPLIED");
            assert_eq!(has_residue_effect,!successor_acl_before.target_aces.is_empty());
            assert!(inspect_grok_home_residue(&profile,&home.path,&home_id,&[],&[]).unwrap().is_empty());
            let successor_after=observe_grok_auth_candidate(&home.path,&home_id).unwrap().candidate_acl(&profile).unwrap();
            assert!(successor_after.target_aces.is_empty());
            assert_eq!(successor_after.identity,successor_acl_before.identity);
            assert_eq!(successor_after.dacl_control,successor_acl_before.dacl_control);
            assert!(successor_acl_before.preserves_other_aces(&successor_after));
            assert_eq!(grok_root_acl(&profile,&home.path,&home_id).unwrap(),root_acl_before);
        } else {
            let error=resume_stopped_revoke(&mut db,&root,&original).unwrap_err();
            assert!(error.contains("old granted FileID not provably revoked or in F HOME"),"{error}");
            assert_eq!(one(&db,"grokA","bindingA").unwrap(),original);
            assert_eq!(instance::read_grok_effects(&db,"bindingA").unwrap(),effects_before);
            assert_eq!(grok_root_acl(&profile,&home.path,&home_id).unwrap(),root_acl_before);
            assert_eq!(observe_grok_auth_candidate(&home.path,&home_id).unwrap().candidate_acl(&profile).unwrap(),successor_acl_before);
        }
        let claim=Statement::prepare(db.as_ptr(),"SELECT state,stop_fact_id FROM main.gogoke_v37_h_claim WHERE domain_id='domainA' AND session_id='sessionA'").unwrap();
        assert!(claim.step_row().unwrap());
        assert_eq!((claim.column_text(0).unwrap(),claim.column_text(1).unwrap()),("RELEASED".into(),stop_fact.clone()));
        let custody=Statement::prepare(db.as_ptr(),"SELECT state,stop_proof_hash FROM main.gogoke_coordination_process_custody WHERE operation_id='oldOp'").unwrap();
        assert!(custody.step_row().unwrap());
        assert_eq!((custody.column_text(0).unwrap(),custody.column_text(1).unwrap()),("STOPPED".into(),stop_fact.clone()));
        let episode=Statement::prepare(db.as_ptr(),"SELECT phase,stop_fact_id FROM main.gogoke_v37_h_process_episode WHERE request_id='openA'").unwrap();
        assert!(episode.step_row().unwrap());
        assert_eq!((episode.column_text(0).unwrap(),episode.column_text(1).unwrap()),("STOPPED".into(),stop_fact));
        drop(claim);drop(custody);drop(episode);
        db.close_checked().unwrap();drop(profile);drop(root);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn deleted_revoked_old_auth_cold_recovery_preserves_original_receipts(){
        deleted_old_auth_cold_recovery_case(true);
    }

    #[test]
    fn deleted_unrevoked_old_auth_cold_recovery_rejects_without_acl_or_journal_write(){
        deleted_old_auth_cold_recovery_case(false);
    }

    #[test]
    fn applied_root_effect_survives_later_authorized_peer_sid_revocation(){
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let home=std::env::temp_dir().join(format!("grok-applied-peer-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&home).unwrap();
        let id=crate::root::inspect_root(&home).unwrap().identity;
        let a=AppContainerProfile::derived_for_test("Gogoke37.GrokAppliedA").unwrap();
        let b=AppContainerProfile::derived_for_test("Gogoke37.GrokAppliedB").unwrap();
        let before=evidence("test root",grok_root_acl(&a,&home,&id)).unwrap();
        let grant=GrokGrant{binding_id:"bindingA".into(),instance_id:"instanceA".into(),
            domain_id:"domainA".into(),session_id:"sessionA".into(),seat_id:"seatA".into(),
            seat_incarnation:"seat-incarnation-A".into(),generation:"1".into(),
            request_id:"openA".into(),profile_name:"Gogoke37.GrokAppliedA".into(),
            profile_sid:a.sid_identity().unwrap(),program_digest:"sha256:fixture".into(),
            home_identity:id.clone(),auth_identity:id.clone(),phase:"ACTIVE".into(),
            process_operation_id:None,ticket:None,custodian_nonce:None,pid:None,
            creation_time_100ns:None,image_path:None,stop_fact_id:None,revision:1};
        grant_grok_home_root(&a,&home,&id).unwrap();
        grant_grok_home_root(&b,&home,&id).unwrap();
        let before_revoke=grok_root_acl(&a,&home,&id).unwrap();
        let historical=effect(&grant,"REVOKE_ROOT",&id,".",&before_revoke);
        revoke_grok_home_root(&a,&home,&id).unwrap();
        assert!(applied_readback(&historical,&grok_root_acl(&a,&home,&id).unwrap()).is_ok());
        revoke_grok_home_root(&b,&home,&id).unwrap();
        let later=grok_root_acl(&a,&home,&id).unwrap();
        assert_ne!(sha256_hex(&later.other_aces_bytes()),historical.other_aces_sha256);
        assert!(applied_readback(&historical,&later).is_ok());
        assert!(before.target_aces.is_empty());
        std::fs::remove_dir(home).unwrap();
    }

    #[test]
    fn before_promote_resume_uses_old_stopped_claim_and_exact_new_episode(){
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("grok-resume-guard-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let mut db=create_new(&root,&path.join("state.sqlite")).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        crate::store::authority::initialize_profile(&mut db,&root).unwrap();
        crate::store::authority::initialize_process_custody_schema(&mut db).unwrap();
        crate::store::instance::initialize_schema(&mut db).unwrap();
        crate::store::instance::initialize_grok_home_grant_schema(&mut db).unwrap();
        crate::store::seat::initialize_schema(&mut db).unwrap();
        super::super::admission::initialize_admission_schema(&mut db).unwrap();
        super::super::session_binding::initialize_schema(&mut db).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('grokA','grok','homeA','volume:0000000000000001/file:01010101010101010101010101010101','sha256:fixture','1.0.41','INSTALLED','LOGGED_IN',1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('domainA','seatA','incA','USER','LONG','grokA','BUSY',1,1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('oldBinding','grokA','domainA','SESSION','sessionA','1','ACTIVE')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id,stop_fact_id) VALUES('domainA','sessionA','grokA','oldHome','oldBinding','1','STOPPED',3,'oldOp','realStop')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_seat_binding VALUES('domainA','sessionA','seatA','incA','1')").unwrap();
        db.execute("INSERT INTO main.gogoke_coordination_process_custody VALUES('oldOp','oldTicket','oldNonce','100','1000','oldImage','sha256:fixture','grokA','domainA','1','STOPPED','realStop')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase,stop_fact_id) VALUES('domainA','openOld','sessionA','1','00',1,'oldOp','grokA','oldHome','oldBinding','seatA','incA','STOPPED','realStop')").unwrap();
        db.execute("INSERT INTO main.gogoke_coordination_process_custody VALUES('newOp','newTicket','newNonce','101','1001','newImage','sha256:fixture','grokA','domainA','2','PREPARED',NULL)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('domainA','resumeA','sessionA','2','1','00',3,'newOp','grokA','newHome','newBinding','seatA','incA','PREPARED')").unwrap();
        let candidate_claim=ClaimObservation{domain_id:"domainA".into(),session_id:"sessionA".into(),
            instance_id:"grokA".into(),home_id:"newHome".into(),binding_id:"newBinding".into(),
            generation:"2".into(),revision:3,phase:super::super::runtime::SessionPhase::Committed,
            process_operation_id:None};
        db.execute("UPDATE main.gogoke_v37_h_process_episode SET process_operation_id=NULL,phase='INTENT' WHERE request_id='resumeA'").unwrap();
        assert!(original_grant_admitted(&db,&candidate_claim,"seatA","incA","resumeA").unwrap());
        db.execute("UPDATE main.gogoke_coordination_process_custody SET stop_proof_hash='wrong' WHERE operation_id='oldOp'").unwrap();
        assert!(!original_grant_admitted(&db,&candidate_claim,"seatA","incA","resumeA").unwrap());
        db.execute("UPDATE main.gogoke_coordination_process_custody SET stop_proof_hash='realStop' WHERE operation_id='oldOp'").unwrap();
        db.execute("UPDATE main.gogoke_v37_h_process_episode SET process_operation_id='newOp',phase='PREPARED' WHERE request_id='resumeA'").unwrap();
        let values:[&str;12]=["domainA","sessionA","grokA","oldBinding","1","oldOp",
            "seatA","incA","resumeA","2","newBinding","newOp"];
        assert!(exact_exists(&db,"test-resume",RESUME_CANDIDATE_GUARD_SQL,&values).unwrap());
        db.execute("UPDATE main.gogoke_coordination_process_custody SET stop_proof_hash='wrong' WHERE operation_id='oldOp'").unwrap();
        assert!(!exact_exists(&db,"test-resume",RESUME_CANDIDATE_GUARD_SQL,&values).unwrap());
        db.execute("UPDATE main.gogoke_coordination_process_custody SET stop_proof_hash='realStop' WHERE operation_id='oldOp'").unwrap();
        db.execute("UPDATE main.gogoke_v37_h_process_episode SET stop_fact_id='wrong' WHERE request_id='openOld'").unwrap();
        assert!(!exact_exists(&db,"test-resume",RESUME_CANDIDATE_GUARD_SQL,&values).unwrap());
        db.execute("UPDATE main.gogoke_v37_h_process_episode SET stop_fact_id='realStop' WHERE request_id='openOld'").unwrap();
        let no_attempt=GrokGrant{binding_id:"noAttempt".into(),instance_id:"grokA".into(),
            domain_id:"domainA".into(),session_id:"otherSession".into(),seat_id:"otherSeat".into(),
            seat_incarnation:"otherInc".into(),generation:"2".into(),request_id:"otherOpen".into(),
            profile_name:"Gogoke37.NoAttempt".into(),profile_sid:"sid-no-attempt".into(),
            program_digest:"sha256:fixture".into(),home_identity:root.canonical_root().identity.clone(),
            auth_identity:root.canonical_root().identity.clone(),phase:"REVOKED".into(),
            process_operation_id:None,ticket:None,custodian_nonce:None,pid:None,
            creation_time_100ns:None,image_path:None,stop_fact_id:None,revision:2};
        let candidate=GrokGrant{binding_id:"newBinding".into(),session_id:"sessionA".into(),
            seat_id:"seatA".into(),seat_incarnation:"incA".into(),
            request_id:"resumeA".into(),..no_attempt.clone()};
        let original=original_h(&db,&candidate).unwrap().unwrap();
        assert!(original.candidate_before_promote);
        assert_eq!(original.operation,"newOp");
        assert_eq!(original.claim_state,"STOPPED");
        assert_eq!(original.claim_stop.as_deref(),Some("realStop"));
        let identity=root.canonical_root().identity.opaque();
        db.execute(&format!("INSERT INTO main.gogoke_v37_grok_home_domains VALUES(
            'grokA','{identity}','{identity}','sha256:fixture','1.0.41',1,1)")).unwrap();
        db.execute(&format!("INSERT INTO main.gogoke_v37_grok_home_grants(
            binding_id,instance_id,domain_id,session_id,seat_id,seat_incarnation,
            generation,request_id,profile_name,profile_sid,program_digest,
            home_identity,auth_identity,phase,process_operation_id,ticket,
            custodian_nonce,pid,creation_time_100ns,image_path,revision)
            VALUES('newBinding','grokA','domainA','sessionA','seatA','incA',
            '2','resumeA','Gogoke37.TestCandidate','candidateSid','sha256:fixture',
            '{identity}','{identity}','REVOKED','newOp','newTicket','newNonce',
            101,1001,'newImage',2)")).unwrap();
        let recorded=one(&db,"grokA","newBinding").unwrap();
        assert_eq!(original_candidate_for_recovery(&db,&recorded).unwrap(),
            Some(("newOp".into(),101,1001)));
        db.execute("UPDATE main.gogoke_v37_grok_home_grants SET ticket='wrong'
            WHERE binding_id='newBinding'").unwrap();
        let drifted=one(&db,"grokA","newBinding").unwrap();
        assert!(original_candidate_for_recovery(&db,&drifted).is_err());
        // Same-instance, same-generation peer custody is not this original binding.
        assert!(!original_no_attempt_has_process(&db,&no_attempt).unwrap());
        db.execute("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('domainA','otherOpen','otherSession','2','00',1,'grokA','otherHome','noAttempt','otherSeat','otherInc','INTENT')").unwrap();
        assert!(original_no_attempt_has_process(&db,&no_attempt).unwrap());
        db.close_checked().unwrap();drop(root);std::fs::remove_dir_all(path).unwrap();
    }
}
