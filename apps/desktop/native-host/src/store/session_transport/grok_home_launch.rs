//! Grok-only F/H composition. F persists each ACL intent before H changes the
//! exact, single-link original HOME object. No credential data is opened.
use super::runtime::{ClaimObservation, InstancePin};
use crate::process::{AppContainerProfile, GrokAuthMetadata, GrokAclSnapshot, PreparedCustody,
    NativeProcessHoldersGone,
    grok_root_acl, grok_residue_acl,
    observe_grok_auth, observe_grok_auth_candidate, grant_grok_home_root, grant_grok_auth,
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
const RESUME_CANDIDATE_GUARD_SQL:&str="SELECT 1 FROM main.gogoke_v37_h_claim h JOIN main.gogoke_coordination_process_custody oldc ON oldc.operation_id=h.process_operation_id AND oldc.profile_id=h.instance_id AND oldc.domain_id=h.domain_id AND oldc.generation=h.generation JOIN main.gogoke_v37_h_process_episode olde ON olde.process_operation_id=h.process_operation_id AND olde.instance_id=h.instance_id AND olde.domain_id=h.domain_id AND olde.session_id=h.session_id AND olde.generation=h.generation AND olde.binding_id=h.binding_id JOIN main.gogoke_v37_h_seat_binding b ON b.domain_id=h.domain_id AND b.session_id=h.session_id AND b.generation=h.generation JOIN main.gogoke_v37_seats s ON s.domain_id=b.domain_id AND s.seat_id=b.seat_id JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=h.domain_id AND e.session_id=h.session_id AND e.old_generation=h.generation WHERE h.domain_id=?1 AND h.session_id=?2 AND h.instance_id=?3 AND h.binding_id=?4 AND h.generation=?5 AND h.process_operation_id=?6 AND h.state='STOPPED' AND h.stop_fact_id IS NOT NULL AND h.stop_fact_id<>'' AND oldc.state='STOPPED' AND oldc.stop_proof_hash=h.stop_fact_id AND olde.phase='STOPPED' AND olde.stop_fact_id=h.stop_fact_id AND olde.seat_id=b.seat_id AND olde.seat_incarnation=b.seat_incarnation AND b.seat_id=?7 AND b.seat_incarnation=?8 AND s.incarnation=b.seat_incarnation AND s.instance_id=h.instance_id AND s.state='BUSY' AND e.request_id=?9 AND e.generation=?10 AND e.instance_id=?3 AND e.binding_id=?11 AND e.seat_id=?7 AND e.seat_incarnation=?8 AND COALESCE(e.process_operation_id,'')=?12 AND e.phase IN ('INTENT','PREPARED','ACTIVE')";

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
        before_control:before.dacl_control,after_control:before.dacl_control,
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
        return Err("Grok private HOME: ACL effect readback diverged".into());
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
    let row=Statement::prepare(db.as_ptr(),"SELECT c.operation_id,c.ticket,c.custodian_nonce,c.pid,c.creation_time_100ns,c.image_path,c.binary_digest_sha256,c.state,COALESCE(c.stop_proof_hash,''),oldh.state,COALESCE(oldh.stop_fact_id,''),e.phase,COALESCE(e.stop_fact_id,'') FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation JOIN main.gogoke_v37_h_claim oldh ON oldh.domain_id=e.domain_id AND oldh.session_id=e.session_id AND oldh.generation=e.old_generation AND oldh.instance_id=e.instance_id JOIN main.gogoke_coordination_process_custody oldc ON oldc.operation_id=oldh.process_operation_id AND oldc.profile_id=oldh.instance_id AND oldc.domain_id=oldh.domain_id AND oldc.generation=oldh.generation JOIN main.gogoke_v37_h_process_episode olde ON olde.process_operation_id=oldh.process_operation_id AND olde.instance_id=oldh.instance_id AND olde.domain_id=oldh.domain_id AND olde.session_id=oldh.session_id AND olde.generation=oldh.generation AND olde.binding_id=oldh.binding_id JOIN main.gogoke_v37_h_seat_binding s ON s.domain_id=oldh.domain_id AND s.session_id=oldh.session_id AND s.generation=oldh.generation WHERE e.binding_id=?1 AND e.instance_id=?2 AND e.domain_id=?3 AND e.session_id=?4 AND e.generation=?5 AND e.request_id=?6 AND e.seat_id=?7 AND e.seat_incarnation=?8 AND e.old_generation IS NOT NULL AND oldh.state='STOPPED' AND oldh.stop_fact_id IS NOT NULL AND oldh.stop_fact_id<>'' AND oldc.state='STOPPED' AND oldc.stop_proof_hash=oldh.stop_fact_id AND olde.phase='STOPPED' AND olde.stop_fact_id=oldh.stop_fact_id AND olde.seat_id=s.seat_id AND olde.seat_incarnation=s.seat_incarnation AND s.seat_id=e.seat_id AND s.seat_incarnation=e.seat_incarnation")
        .map_err(|e|format!("Grok private HOME original resume candidate: {e:?}"))?;
    let values:[&str;8]=[&grant.binding_id,&grant.instance_id,&grant.domain_id,
        &grant.session_id,&grant.generation,&grant.request_id,&grant.seat_id,&grant.seat_incarnation];
    for (i,value) in values.iter().enumerate(){row.bind_text(i as i32+1,value)
        .map_err(|e|format!("Grok private HOME original candidate bind: {e:?}"))?;}
    if !row.step_row().map_err(|e|format!("Grok private HOME original candidate read: {e:?}"))? {
        return Ok(None);
    }
    let result=parse_original_row(&row,true)?;
    if row.step_row().map_err(|e|format!("Grok private HOME original candidate duplicate: {e:?}"))? {
        return Err("Grok private HOME: duplicate original resume candidate".into());
    }
    Ok(Some(result))
}

fn original_h(db:&VerifiedDatabaseConnection<'_>,grant:&GrokGrant)->Result<Option<OriginalH>,String>{
    let row=Statement::prepare(db.as_ptr(),"SELECT c.operation_id,c.ticket,c.custodian_nonce,c.pid,c.creation_time_100ns,c.image_path,c.binary_digest_sha256,c.state,COALESCE(c.stop_proof_hash,''),h.state,COALESCE(h.stop_fact_id,''),e.phase,COALESCE(e.stop_fact_id,'') FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation JOIN main.gogoke_v37_h_claim h ON h.process_operation_id=c.operation_id AND h.instance_id=c.profile_id AND h.domain_id=c.domain_id AND h.generation=c.generation AND h.session_id=e.session_id AND h.binding_id=e.binding_id JOIN main.gogoke_v37_h_seat_binding s ON s.domain_id=h.domain_id AND s.session_id=h.session_id AND s.generation=h.generation AND s.seat_id=e.seat_id AND s.seat_incarnation=e.seat_incarnation WHERE e.binding_id=?1 AND e.instance_id=?2 AND e.domain_id=?3 AND e.session_id=?4 AND e.generation=?5 AND e.request_id=?6 AND e.seat_id=?7 AND e.seat_incarnation=?8")
        .map_err(|e|format!("Grok private HOME original H query: {e:?}"))?;
    let values:[&str;8]=[&grant.binding_id,&grant.instance_id,&grant.domain_id,
        &grant.session_id,&grant.generation,&grant.request_id,&grant.seat_id,&grant.seat_incarnation];
    for (i,value) in values.iter().enumerate(){row.bind_text(i as i32+1,value)
        .map_err(|e|format!("Grok private HOME original H bind: {e:?}"))?;}
    if !row.step_row().map_err(|e|format!("Grok private HOME original H read: {e:?}"))? {
        return original_resume_candidate_h(db,grant);
    }
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
    let custody=Statement::prepare(db.as_ptr(),"SELECT 1 FROM main.gogoke_coordination_process_custody c JOIN main.gogoke_v37_h_claim h ON h.process_operation_id=c.operation_id AND h.instance_id=c.profile_id AND h.domain_id=c.domain_id AND h.generation=c.generation JOIN main.gogoke_v37_h_seat_binding s ON s.domain_id=h.domain_id AND s.session_id=h.session_id AND s.generation=h.generation WHERE h.binding_id=?1 AND h.instance_id=?2 AND h.domain_id=?3 AND h.session_id=?4 AND h.generation=?5 AND s.seat_id=?6 AND s.seat_incarnation=?7 LIMIT 1")
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
        let auth=evidence("observe-original-auth",observe_grok_auth(&home.path,&home.identity))?;
        let grant=GrokGrant{binding_id:claim.binding_id.clone(),instance_id:claim.instance_id.clone(),
            domain_id:claim.domain_id.clone(),session_id:claim.session_id.clone(),seat_id:seat_id.into(),
            seat_incarnation:seat_incarnation.into(),generation:claim.generation.clone(),request_id:request_id.into(),
            profile_name:profile_name.into(),profile_sid:evidence("profile-SID",profile.sid_identity())?,
            program_digest:pin.digest.clone(),
            home_identity:home.identity.clone(),auth_identity:auth.identity.clone(),
            phase:"GRANT_PENDING".into(),process_operation_id:None,ticket:None,
            custodian_nonce:None,pid:None,creation_time_100ns:None,image_path:None,
            stop_fact_id:None,revision:1};
        let grant=instance::begin_grok_grant(db,&domain,&grant)?;
        if grant.phase!="GRANT_PENDING" {
            return Err("Grok private HOME: original grant already advanced".into());
        }
        let result=Self{instance_id:claim.instance_id.clone(),binding_id:claim.binding_id.clone(),
            home:home.clone(),auth:Mutex::new(vec![auth])};
        let prepared=(||{
            let root_before=evidence("root-ACL-before",grok_root_acl(profile,&home.path,&home.identity))?;
            apply(db,effect(&grant,"GRANT_ROOT",&home.identity,".",&root_before),
                ||evidence("root-ACL-readback",grok_root_acl(profile,&home.path,&home.identity)),||
                evidence("grant-root",grant_grok_home_root(profile,&home.path,&home.identity)))?;
            let held=result.auth.lock().map_err(|_|"Grok private HOME: auth custody poisoned")?;
            let auth_before=evidence("auth-ACL-before",held[0].acl(profile))?;
            apply(db,effect(&grant,"GRANT_AUTH",&held[0].identity,"auth.json",&auth_before),
                ||evidence("auth-ACL-readback",held[0].acl(profile)),||
                evidence("grant-auth",grant_grok_auth(profile,&held[0])))?;
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
        if held.is_empty() {return Err("Grok private HOME: metadata custody absent".into());}
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
                "SELECT 1 FROM main.gogoke_v37_h_claim h JOIN main.gogoke_v37_h_seat_binding b ON b.domain_id=h.domain_id AND b.session_id=h.session_id AND b.generation=h.generation JOIN main.gogoke_v37_seats s ON s.domain_id=b.domain_id AND s.seat_id=b.seat_id WHERE h.domain_id=?1 AND h.session_id=?2 AND h.binding_id=?3 AND h.instance_id=?4 AND h.generation=?5 AND h.state='COMMITTED' AND COALESCE(h.process_operation_id,'')=?6 AND b.seat_id=?7 AND b.seat_incarnation=?8 AND s.incarnation=b.seat_incarnation AND s.instance_id=h.instance_id AND s.state='BUSY'",
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
        let row=Statement::prepare(db.as_ptr(),"SELECT c.stop_proof_hash FROM main.gogoke_coordination_process_custody c JOIN main.gogoke_v37_h_claim h ON h.process_operation_id=c.operation_id AND h.domain_id=c.domain_id AND h.generation=c.generation JOIN main.gogoke_v37_h_seat_binding s ON s.domain_id=h.domain_id AND s.session_id=h.session_id AND s.generation=h.generation JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=c.operation_id AND e.instance_id=c.profile_id AND e.domain_id=c.domain_id AND e.generation=c.generation AND e.session_id=h.session_id AND e.binding_id=h.binding_id AND e.seat_id=s.seat_id AND e.seat_incarnation=s.seat_incarnation WHERE c.operation_id=?1 AND c.ticket=?2 AND c.custodian_nonce=?3 AND c.domain_id=?4 AND c.generation=?5 AND c.profile_id=?6 AND c.binary_digest_sha256=?7 AND c.pid=?8 AND c.creation_time_100ns=?9 AND c.image_path=?10 AND h.binding_id=?11 AND h.instance_id=?6 AND h.session_id=?12 AND s.seat_incarnation=?13 AND s.seat_id=?14 AND e.request_id=?15 AND c.state='STOPPED' AND c.stop_proof_hash IS NOT NULL AND c.stop_proof_hash=h.stop_fact_id AND h.state='STOPPED'")
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
        apply(db,effect(&pending,"REVOKE_ROOT",&self.home.identity,".",&root_before),
            ||evidence("root-ACL-after-revoke",grok_root_acl(profile,&self.home.path,&self.home.identity)),||
            evidence("revoke-root",revoke_grok_home_root(profile,&self.home.path,&self.home.identity)))?;
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
        for object in evidence("scan-residue",inspect_grok_home_residue(profile,&self.home.path,
            &self.home.identity,&ids))? {
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
            &self.home.identity,&ids))?.is_empty() {
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
    if !root_acl.target_aces.is_empty() ||
        !evidence("domain-readback",inspect_grok_home_residue(&profile,&home.path,
            &home.identity,&ids))?.is_empty() {
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
            &home.identity,&ids))?.is_empty() {
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
        if !root_acl.target_aces.is_empty() ||
            !evidence("retired-domain-readback",inspect_grok_home_residue(&profile,
                &home.path,&home.identity,&ids))?.is_empty() {
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
    use crate::store::same_open::{create_new,route_b_test_guard};
    use std::time::{SystemTime,UNIX_EPOCH};

    #[test]
    fn inherited_successor_intent_recovers_after_acl_write_before_finish(){
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("grok-successor-intent-{}-{nonce}",std::process::id()));
        let home=path.join("home");std::fs::create_dir_all(&home).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let mut db=create_new(&root,&path.join("state.sqlite")).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        instance::initialize_schema(&mut db).unwrap();
        instance::initialize_grok_home_grant_schema(&mut db).unwrap();
        let profile=AppContainerProfile::derived_for_test("Gogoke37.GrokSuccessorIntent").unwrap();
        let home_id=crate::root::inspect_root(&home).unwrap().identity;
        grant_grok_home_root(&profile,&home,&home_id).unwrap();
        std::fs::write(home.join("auth.json"),b"synthetic non-secret fixture").unwrap();
        let auth=observe_grok_auth_candidate(&home,&home_id).unwrap();
        let before=auth.candidate_acl(&profile).unwrap();assert!(!before.dacl_protected);
        db.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('grokA','grok','homeA','fixture','sha256:fixture','1.0.41','INSTALLED','LOGGED_IN',1)").unwrap();
        let root_id=root.canonical_root().identity.opaque();
        db.execute(&format!("INSERT INTO main.gogoke_v37_grok_home_domains VALUES('grokA','{root_id}','{}','sha256:fixture','1.0.41',1,1)",home_id.opaque())).unwrap();
        db.execute(&format!("INSERT INTO main.gogoke_v37_grok_home_grants(binding_id,instance_id,domain_id,session_id,seat_id,seat_incarnation,generation,request_id,profile_name,profile_sid,program_digest,home_identity,auth_identity,phase,revision) VALUES('bindingA','grokA','domainA','sessionA','seatA','incA','1','openA','Gogoke37.GrokSuccessorIntent','{}','sha256:fixture','{}','{}','ACTIVE',1)",profile.sid_identity().unwrap(),home_id.opaque(),auth.identity.opaque())).unwrap();
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
        assert_eq!(recovered.len(),1);assert_eq!(recovered[0].phase,"APPLIED");
        assert_eq!(recovered[0].revision,2);
        assert!(before.preserves_other_aces(&auth.acl(&profile).unwrap()));
        verify_grok_auth(&profile,&auth).unwrap();
        revoke_grok_home_root(&profile,&home,&home_id).unwrap();
        revoke_grok_auth(&profile,&auth).unwrap();
        db.close_checked().unwrap();drop(auth);drop(profile);drop(root);
        std::fs::remove_dir_all(path).unwrap();
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
        db.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('grokA','grok','homeA','volume:0000000000000001/file:01010101010101010101010101010101','sha256:fixture','1.0.41','INSTALLED','LOGGED_IN',1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('domainA','seatA','incA','USER','LONG','grokA','BUSY',1,1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('oldBinding','grokA','domainA','SESSION','sessionA','1','ACTIVE')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id,stop_fact_id) VALUES('domainA','sessionA','grokA','oldHome','oldBinding','1','STOPPED',3,'oldOp','realStop')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_seat_binding VALUES('domainA','sessionA','seatA','incA','1')").unwrap();
        db.execute("INSERT INTO main.gogoke_coordination_process_custody VALUES('oldOp','oldTicket','oldNonce','100','1000','oldImage','sha256:fixture','grokA','domainA','1','STOPPED','realStop')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase,stop_fact_id) VALUES('domainA','openOld','sessionA','1','00',1,'oldOp','grokA','oldHome','oldBinding','seatA','incA','STOPPED','realStop')").unwrap();
        db.execute("INSERT INTO main.gogoke_coordination_process_custody VALUES('newOp','newTicket','newNonce','101','1001','newImage','sha256:fixture','grokA','domainA','2','PREPARED',NULL)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('domainA','resumeA','sessionA','2','1','00',3,'newOp','grokA','newHome','newBinding','seatA','incA','PREPARED')").unwrap();
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
