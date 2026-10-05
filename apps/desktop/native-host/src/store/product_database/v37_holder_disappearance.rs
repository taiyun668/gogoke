//! Native-only retirement of lost credential holders. This is resource
//! reconciliation, never a ProcessCustodian StopFact or continuation proof.
use super::*;
use crate::process::{AppContainerProfile, CredentialAliasScope, CredentialBinding,
    NativeCredentialAclRecoveryStep, NativeProcessHoldersGone,
    adopt_holder_gone_source_baseline};
use crate::store::instance::holder_disappearance::{self as gone,
    HolderDisappearanceInput, HolderDisappearanceRecord, HolderDisappearancePhase as Phase};
use crate::store::atomic::Parser;
use crate::store::digest::sha256_hex;
use crate::root::RootIdentity;

type Fields = BTreeMap<String, String>;
const CREDENTIAL_RIGHTS: u32 = 0x0012_019f;

#[cfg(test)]
std::thread_local! {
    static HOLDER_GONE_CUT: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}
#[cfg(test)]
pub(super) fn set_holder_gone_cut_for_test(stage: Option<&'static str>) {
    HOLDER_GONE_CUT.with(|cut| cut.set(stage));
}
#[cfg(test)]
fn holder_gone_cut_for_test(stage: &'static str) -> Result<()> {
    if HOLDER_GONE_CUT.with(|cut| {
        if cut.get()==Some(stage) {cut.set(None);true} else {false}
    }) {return Err(refused("controlled holder-gone cut"));}
    Ok(())
}

fn fail<T>(value: std::result::Result<T, impl std::fmt::Debug>) -> Result<T> {
    value.map_err(|error| OrchestrationError::V37StoreFailure(
        format!("disappeared credential holder: {error:?}")))
}
fn refused(detail: &'static str) -> OrchestrationError { OrchestrationError::Invalid(detail) }
fn text(value: &str) -> Json { Json::String(JsonString::from_str(value)) }
fn encoded(fields: &Fields) -> String {
    Json::Object(fields.iter().map(|(k,v)|(JsonString::from_str(k),text(v))).collect()).canonical()
}
fn parse_fields(source: &str) -> Result<Fields> {
    let parsed=fail(Parser::parse(source))?;
    if parsed.canonical()!=source {return Err(refused("holder capture is not canonical"));}
    let Json::Object(fields)=parsed else{return Err(refused("holder capture object"));};
    fields.into_iter().map(|(key,value)| {
        let Json::String(value)=value else{return Err(refused("holder capture string field"));};
        Ok((String::from_utf16(key.units()).map_err(|e|OrchestrationError::V37StoreFailure(format!("holder key: {e}")))?,
            String::from_utf16(value.units()).map_err(|e|OrchestrationError::V37StoreFailure(format!("holder value: {e}")))?))
    }).collect()
}
fn put(fields: &mut Fields, key: &str, value: &str) {
    fields.insert(key.into(), value.into());
}
fn get(fields: &Fields, key: &str) -> Result<String> {
    fields.get(key).cloned().ok_or_else(||refused("original holder capture field missing"))
}
fn number(fields: &Fields, key: &str) -> Result<i64> {
    get(fields,key)?.parse().map_err(|error|
        OrchestrationError::V37StoreFailure(format!("holder capture number: {error}")))
}
fn bytes_hex(bytes: &[u8]) -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() }
fn unhex(value: &str) -> Result<Vec<u8>> {
    if value.len()>131072 || value.len()%2!=0 || !value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(refused("holder capture hex"));
    }
    value.as_bytes().chunks_exact(2).map(|b| {
        let value=std::str::from_utf8(b).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("holder capture encoding: {error}")))?;
        u8::from_str_radix(value,16).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("holder capture digit: {error}")))
    }).collect()
}
fn decode(record: &HolderDisappearanceRecord) -> Result<Fields> {
    let bytes=unhex(&record.input.snapshot_hex)?;
    if sha256_hex(&bytes)!=record.input.snapshot_digest {return Err(refused("holder capture digest changed"));}
    let source=std::str::from_utf8(&bytes).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("holder capture UTF8: {error}")))?;
    parse_fields(source)
}
fn rows(db: &VerifiedDatabaseConnection<'_>, sql: &str, params: &[&str], columns: usize)
    -> Result<Vec<Vec<String>>> {
    let q=Statement::prepare(db.as_ptr(),sql)?;
    for (n,value) in params.iter().enumerate() {q.bind_text((n+1) as i32,value)?;}
    let mut result=Vec::new();
    while q.step_row()? {result.push((0..columns).map(|n|q.column_text(n as i32)).collect::<std::result::Result<_,_>>()?);}
    Ok(result)
}

impl<'root> ProductDatabase<'root> {
    fn gone_original(&self, profile: &instance::CredentialProfileRecord) -> Result<Fields> {
        let history=fail(instance::read_private_history_generation(&self.connection,
            &profile.binding_id,&profile.generation))?.ok_or_else(||refused("holder original F generation absent"))?;
        let source=history.source.as_ref().ok_or_else(||refused("holder original F custody absent"))?;
        if history.instance_id!=profile.instance_id || history.history_id!=profile.history_id {
            return Err(refused("holder original profile/history changed"));
        }
        let found=rows(&self.connection,
            "SELECT c.pid,c.creation_time_100ns,c.image_path,c.binary_digest_sha256,c.state,
                    COALESCE(c.stop_proof_hash,''),a.state,CAST(a.revision AS TEXT),a.binding_id,a.home_id,
                    e.phase,e.raw_hex,CAST(e.result_revision AS TEXT)
               FROM main.gogoke_coordination_process_custody c
               JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=c.operation_id
                 AND e.instance_id=c.profile_id AND e.domain_id=c.domain_id AND e.generation=c.generation
               JOIN main.gogoke_v37_h_claim a ON a.process_operation_id=c.operation_id
                 AND a.instance_id=c.profile_id AND a.domain_id=c.domain_id AND a.generation=c.generation
               JOIN main.gogoke_v37_h_seat_binding s ON s.domain_id=a.domain_id
                 AND s.session_id=a.session_id AND s.generation=a.generation
              WHERE c.operation_id=?1 AND c.ticket=?2 AND c.custodian_nonce=?3 AND c.profile_id=?4
                AND c.domain_id=?5 AND c.generation=?6 AND a.session_id=?7
                AND s.seat_id=?8 AND s.seat_incarnation=?9 AND e.binding_id=?10
                AND e.session_id=a.session_id AND e.seat_id=s.seat_id AND e.seat_incarnation=s.seat_incarnation",
            &[&source.process_operation_id,&source.ticket,&source.custodian_nonce,&profile.instance_id,
                &history.domain_id,&profile.generation,&history.session_id,&history.seat_id,
                &history.seat_incarnation,&profile.binding_id],13)?;
        if found.len()!=1 {return Err(refused("holder original H/F/custody association changed"));}
        let r=&found[0];
        if !matches!(r[4].as_str(),"ACTIVE"|"PREPARED"|"UNKNOWN") || !r[5].is_empty()
            || !matches!(r[10].as_str(),"ACTIVE"|"PREPARED"|"UNKNOWN") {
            return Err(refused("holder disappearance does not replace a stop proof"));
        }
        let suffix=sha256_hex(format!("{}\n{}\n{}\n{}\n{}",
            self.root.canonical_root().identity.opaque(),history.domain_id,history.session_id,
            history.seat_incarnation,profile.generation).as_bytes());
        let sid=fail(AppContainerProfile::derive_for_revocation(&format!("Gogoke37.Session.{}",&suffix[..40])))?;
        if fail(sid.sid_identity())?!=profile.profile_sid {return Err(refused("holder original SID derivation changed"));}
        let mut f=Fields::new();
        for (key,value) in [("binding",profile.binding_id.as_str()),("instance",profile.instance_id.as_str()),
            ("history",profile.history_id.as_str()),("domain",history.domain_id.as_str()),
            ("session",history.session_id.as_str()),("seat",history.seat_id.as_str()),
            ("incarnation",history.seat_incarnation.as_str()),("generation",profile.generation.as_str()),
            ("operation",source.process_operation_id.as_str()),("ticket",source.ticket.as_str()),
            ("nonce",source.custodian_nonce.as_str()),("sid",profile.profile_sid.as_str())] {put(&mut f,key,value);}
        for (key,value) in ["pid","creation","image","binary","custodyState","stopHash","claimState",
            "claimRevision","claimBinding","claimHome","episodePhase","episodeHex","episodeRevision"].iter().zip(r) {
            put(&mut f,key,value);
        }
        Ok(f)
    }

    // An existing verified same-physical-root exclusive holder and this empty
    // native custody scope form the retired-host part of the qualification.
    // H's original unique ACTIVE writer used noninheritable kill-on-close Jobs;
    // close_checked drops those Jobs before DB/RootLock release. This is not
    // a caller-provided hostGone flag or an invented historical Job receipt.
    fn gone_scope(&mut self, instance: &str, allowed: &[String], incoming: Option<&V37Request>, retained: bool) -> Result<()> {
        fail(self.connection.execute("BEGIN IMMEDIATE"))?;
        let checked=self.gone_scope_in_current_transaction(instance,allowed,incoming,retained);
        self.finish_native_transaction(checked)
    }

    fn gone_scope_in_current_transaction(&self, instance: &str, allowed: &[String], incoming: Option<&V37Request>, retained: bool) -> Result<()> {
        fail(authority::check_owner_in_current_transaction(&self.connection,&self.owner))?;
        if self.owner_login.is_some() || !self.pending_native_launches.is_empty()
            || !self.pending_credential_preparations.is_empty()
            || self.native_sessions.values().any(|r|r.custody.binding.profile_id==instance) {
            return Err(refused("holder recovery retains a current native custodian or intent"));
        }
        for r in rows(&self.connection,
            "SELECT operation_id FROM main.gogoke_coordination_process_custody WHERE profile_id=?1
               AND (state<>'STOPPED' OR stop_proof_hash IS NULL OR stop_proof_hash='')",
            &[instance],1)? {
            if !allowed.contains(&r[0]) {return Err(refused("holder recovery retains an unlisted custody"));}
        }
        for r in rows(&self.connection,
            "SELECT domain_id,session_id,COALESCE(process_operation_id,'') FROM main.gogoke_v37_h_claim
              WHERE instance_id=?1 AND state NOT IN ('STOPPED','RELEASED')",&[instance],3)? {
            if allowed.contains(&r[2]) {continue;}
            let admitted=incoming.is_some_and(|q|q.domain_id==r[0] && q.target_id==r[1]) && r[2].is_empty();
            let no_effect=rows(&self.connection,
                "SELECT 1 FROM main.gogoke_v37_h_process_episode WHERE domain_id=?1 AND session_id=?2
                   UNION ALL SELECT 1 FROM main.gogoke_v37_h_operation
                   WHERE domain_id=?1 AND session_id=?2 AND operation='open'",&[&r[0],&r[1]],1)?.is_empty();
            if !admitted || !no_effect {return Err(refused("holder recovery retains another claim or open intent"));}
        }
        for r in rows(&self.connection,
            "SELECT g.domain_id,g.request_id,g.session_id,g.raw_hex,g.operation,g.stage,
                    CAST(g.owner_stop_request_id IS NULL AS TEXT),e.phase,COALESCE(e.stop_fact_id,''),
                    COALESCE(c.state,''),COALESCE(c.stop_proof_hash,''),
                    COALESCE(CAST(c.profile_id=e.instance_id AND c.domain_id=e.domain_id
                      AND c.generation=e.generation AND e.domain_id=g.domain_id
                      AND e.session_id=g.session_id AND e.generation=g.old_generation
                      AND c.ticket=g.old_ticket AND c.custodian_nonce=g.old_nonce AS TEXT),'0')
               FROM main.gogoke_v37_h_generation_change g JOIN main.gogoke_v37_h_process_episode e
               ON e.process_operation_id=g.old_process_operation_id
               LEFT JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id
              WHERE e.instance_id=?1
               AND g.stage NOT IN ('APPLIED','CANCELLED','UNSUPPORTED')",&[instance],12)? {
            let original=retained && incoming.is_some_and(|q|q.domain_id==r[0] && q.request_id==r[1]
                && q.target_id==r[2] && bytes_hex(&q.raw_bytes)==r[3] && q.operation==r[4]
                && matches!(q.operation.as_str(),"compact"|"renew-session"));
            if !original || r[5]!="OLD_STOPPED" || r[6]!="1" || r[7]!="STOPPED"
                || r[8].is_empty() || r[9]!="STOPPED" || r[8]!=r[10] || r[11]!="1" {
                return Err(refused("holder recovery retains a generation change"));
            }
        }
        Ok(())
    }

    fn gone_release_in_transaction(&mut self, capture: &Fields, record: &HolderDisappearanceRecord,
        proof: &NativeProcessHoldersGone) -> Result<()> {
        fail(authority::check_owner_in_current_transaction(&self.connection,&self.owner))?;
        if record.phase!=Phase::Revoked {return Err(refused("holder release lacks original actual credential revoke"));}
        let domain=get(capture,"domain")?;let session=get(capture,"session")?;
        let generation=get(capture,"generation")?;let instance=get(capture,"instance")?;
        if fail(crate::store::session_transport::generation_change::active_for_session(
            &self.connection,&domain,&session))?.is_some() {return Err(refused("holder release generation change"));}
        let revision=number(capture,"claimRevision")?;
        let current=self.gone_original(&fail(instance::read_credential_profiles(&self.connection,&instance))?
            .into_iter().find(|p|p.binding_id==record.input.binding_id)
            .ok_or_else(||refused("holder release original profile absent"))?)?;
        if get(&current,"claimState")?!=get(capture,"claimState")?
            || number(&current,"claimRevision")?!=revision || !get(&current,"stopHash")?.is_empty() {
            return Err(refused("holder release original claim changed"));
        }
        let seat_id=get(capture,"seat")?;
        let seat=fail(crate::store::seat::get(&self.connection,&domain,&seat_id))?
            .ok_or_else(||refused("holder release seat absent"))?;
        if seat.state!=crate::store::seat::State::Busy || seat.instance_id!=instance
            || seat.incarnation!=get(capture,"incarnation")? || seat.generation.to_string()!=generation {
            return Err(refused("holder release original seat changed"));
        }
        let q=Statement::prepare(self.connection.as_ptr(),
            "UPDATE main.gogoke_v37_h_claim SET state='RELEASED',revision=revision+1
               WHERE domain_id=?1 AND session_id=?2 AND state=?3 AND revision=?4
                 AND process_operation_id=?5 AND binding_id=?6 AND home_id=?7
                 AND instance_id=?8 AND generation=?9 AND stop_fact_id IS NULL")?;
        for (n,v) in [&domain,&session,&get(capture,"claimState")?].iter().enumerate(){q.bind_text((n+1) as i32,v)?;}
        q.bind_i64(4,revision)?;
        for (n,v) in [&get(capture,"operation")?,&get(capture,"claimBinding")?,&get(capture,"claimHome")?,
            &instance,&generation].iter().enumerate(){q.bind_text((n+5) as i32,v)?;}
        q.step_done()?;drop(q);
        if rows(&self.connection,"SELECT changes()",&[],1)?!=vec![vec![String::from("1")]] {
            return Err(refused("holder release claim CAS"));
        }
        fail(crate::store::seat::set_dispatch_state_in_transaction(&mut self.connection,&seat,false))?;
        let raw=encoded(capture);
        let op=Statement::prepare(self.connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,
                previous_revision,revision) VALUES(?1,?2,?3,'holder-gone-release',?4,'APPLIED',?5,?6)")?;
        for (n,v) in [&domain,&record.input.request_id,&bytes_hex(raw.as_bytes()),&session].iter().enumerate(){op.bind_text((n+1) as i32,v)?;}
        op.bind_i64(5,revision)?;op.bind_i64(6,revision.checked_add(1).ok_or_else(||refused("holder release revision overflow"))?)?;
        op.step_done()?;drop(op);
        fail(gone::advance_in_transaction(&self.connection,record,Phase::Revoked,Phase::Applied,proof))?;
        Ok(())
    }

    pub(super) fn recover_disappeared_credential_resources(&mut self, instance_id: &str,
        incoming: Option<&V37Request>) -> Result<()> {
        let Some(registered)=self.read_registered_instance(instance_id)? else{return Ok(());};
        if registered.driver_id!="codex" || registered.login_state!="LOGGED_IN" {return Ok(());}
        // An ordinary active holder uses the original grant path, not cold
        // retirement. Never revoke a profile owned by this live native host.
        if self.native_sessions.values().any(|r|r.custody.binding.profile_id==instance_id) {return Ok(());}
        let profiles=fail(instance::read_credential_profiles(&self.connection,instance_id))?;
        if profiles.is_empty() {return Ok(());}
        let Some(object)=fail(instance::read_credential_object(&self.connection,instance_id))? else {
            return Err(refused("holder recovery registered source absent"));
        };
        let home=fail(instance::resolve_codex_instance_home(&self.connection,self.root,instance_id))?;
        if object.root_identity!=*self.connection.root_identity() || object.home_identity!=home.identity
            || object.source_parent_identity!=home.identity {return Err(refused("holder recovery root/home changed"));}
        let aliases=fail(instance::read_credential_aliases(&self.connection,instance_id))?;
        let mut scopes=Vec::new();let mut alias_physical=Fields::new();
        for alias in aliases.iter().filter(|a|a.state!="REMOVED") {
            if !matches!(alias.state.as_str(),"ACTIVE"|"DORMANT") || alias.source_file_identity!=object.file_identity {
                return Err(refused("holder recovery unknown alias/source"));
            }
            let dir=fail(instance::resolve_private_history_directory(&self.connection,self.root,&alias.history_id))?;
            if dir.identity!=alias.directory_identity {return Err(refused("holder recovery alias directory changed"));}
            put(&mut alias_physical,&alias.history_id,&dir.identity.opaque());
            scopes.push(CredentialAliasScope{root:dir.path,root_identity:dir.identity});
        }
        let binding=fail(CredentialBinding::open_registered(self.root,&home.path.join("auth.json"),
            &home.identity,&object.file_identity,&scopes))?;
        fail(gone::initialize_holder_disappearance_schema(&mut self.connection))?;
        let mut records=BTreeMap::new();
        for profile in &profiles {
            if let Some(record)=fail(gone::read_holder_disappearance(&self.connection,&profile.binding_id))? {
                if record.phase==Phase::Unknown {return Err(refused("holder recovery original UNKNOWN fence"));}
                records.insert(profile.binding_id.clone(),record);
            } else if profile.state!="ACTIVE" && profile.state!="REVOKED" {
                return Err(refused("holder recovery unowned profile intent"));
            }
        }
        let members:Vec<String>=if let Some(record)=records.values().find(|r|r.phase!=Phase::Applied) {
            get(&decode(record)?,"members")?.split(',').map(str::to_owned).collect()
        } else {rows(&self.connection,
            "SELECT p.binding_id FROM main.gogoke_v37_credential_profiles p
               JOIN main.gogoke_v37_instance_history_generations g ON g.binding_id=p.binding_id
               JOIN main.gogoke_coordination_process_custody c ON c.operation_id=g.process_operation_id
              WHERE p.instance_id=?1 AND p.state='ACTIVE'
                AND (c.state<>'STOPPED' OR c.stop_proof_hash IS NULL OR c.stop_proof_hash='') ORDER BY p.binding_id",
            &[instance_id],1)?.into_iter().map(|r|r[0].clone()).collect()};
        if members.is_empty() && records.is_empty() {return Ok(());}
        if profiles.iter().any(|p|p.state!="REVOKED" && !members.contains(&p.binding_id)) {
            return Err(refused("holder recovery capture would expand"));
        }
        // Reuse only this host's already adopted physical metadata holder.
        // A genuine internally stopped continuation is not another cold
        // retirement. Pending recovery members never receive this exception.
        let retained=members.is_empty() && !records.is_empty()
            && records.values().all(|r|r.phase==Phase::Applied)
            && self.disappeared_credential_holders.get(&(instance_id.into(),object.file_identity.opaque()))
                .is_some_and(|prior|std::sync::Arc::ptr_eq(prior,&binding));
        let retained=retained && fail(binding.acl_prepared_in_this_holder())?;
        let mut originals=BTreeMap::new();
        for profile in &profiles {
            if members.contains(&profile.binding_id) || records.contains_key(&profile.binding_id) {
                originals.insert(profile.binding_id.clone(),self.gone_original(profile)?);
            }
        }
        // A phase label alone is not a completed fact. Validate its immutable
        // original and the actual F/H receipts before excluding it from OS
        // observation or performing any remaining resource effect.
        for (id,record) in records.iter().filter(|(_,r)|r.phase==Phase::Applied) {
            self.gone_validate_capture(originals.get(id).ok_or_else(||refused("holder completed original absent"))?,
                &decode(record)?,&object,&home.identity,&alias_physical,record)?;
        }
        let allowed:Vec<String>=originals.values().map(|f|get(f,"operation")).collect::<Result<_>>()?;
        self.gone_scope(instance_id,&allowed,incoming,retained)?;
        // APPLIED is the durable conclusion of the original disappearance,
        // F revoke and H release. PID reuse cannot invalidate that completed
        // fact. Observe the OS only for resources which still need an effect;
        // completed captures and receipts remain verified below.
        let all_pairs=originals.iter().filter(|(id,_)|!records.get(*id)
            .is_some_and(|r|r.phase==Phase::Applied)).map(|(_,f)|Ok((get(f,"pid")?.parse::<u32>().map_err(|e|
            OrchestrationError::V37StoreFailure(format!("holder pid: {e}")))?,get(f,"creation")?.parse::<u64>().map_err(|e|
            OrchestrationError::V37StoreFailure(format!("holder creation: {e}")))?))).collect::<Result<Vec<_>>>()?;
        let all_gone=if all_pairs.is_empty() {None}
            else {Some(fail(NativeProcessHoldersGone::observe(&all_pairs))?)};
        let known:Vec<(String,u32)>=members.iter().map(|id| {
            let profile=profiles.iter().find(|p|&p.binding_id==id).ok_or_else(||refused("holder capture member absent"))?;
            Ok((profile.profile_sid.clone(),CREDENTIAL_RIGHTS))
        }).collect::<Result<_>>()?;
        let mut final_digest=None;
        for member in &members {
            let profile=profiles.iter().find(|p|&p.binding_id==member).ok_or_else(||refused("holder member absent"))?;
            let original=originals.get(member).ok_or_else(||refused("holder original absent"))?;
            let pair=(get(original,"pid")?.parse::<u32>().map_err(|e|OrchestrationError::V37StoreFailure(format!("holder pid: {e}")))?,
                get(original,"creation")?.parse::<u64>().map_err(|e|OrchestrationError::V37StoreFailure(format!("holder creation: {e}")))?);
            let proof=if records.get(member).is_some_and(|r|r.phase==Phase::Applied) {None}
                else {Some(fail(NativeProcessHoldersGone::observe(&[pair]))?)};
            let (mut record,capture,step)=if let Some(record)=records.get(member) {
                let capture=decode(record)?;
                self.gone_validate_capture(original,&capture,&object,&home.identity,&alias_physical,record)?;
                let step=fail(NativeCredentialAclRecoveryStep::restore(&get(&capture,"aclSnapshot")?,
                    &get(&capture,"aclBefore")?,&profile.profile_sid,&known))?;
                (record.clone(),capture,step)
            } else {
                if profile.state!="ACTIVE" || !matches!(get(original,"claimState")?.as_str(),"COMMITTED"|"UNKNOWN") {
                    return Err(refused("holder initial capture requires exact original active resources"));
                }
                let step=fail(NativeCredentialAclRecoveryStep::capture(&binding,&profile.profile_sid,&known))?;
                let mut capture=original.clone();
                for (k,v) in [("root",self.connection.root_identity().opaque()),("database",self.connection.identity().opaque()),
                    ("home",home.identity.opaque()),("source",object.file_identity.opaque()),
                    ("profileRevision",profile.revision.to_string()),("profileIntent",profile.intent_request.clone()),
                    ("members",members.join(",")),("aclSnapshot",step.encode_snapshot()),
                    ("aclBefore",step.before_digest()),("aclTarget",step.target_digest())] {put(&mut capture,k,&v);}
                put(&mut capture,"aliases",&encoded(&alias_physical));
                let bytes=encoded(&capture).into_bytes();
                let input=HolderDisappearanceInput{binding_id:member.clone(),instance_id:instance_id.into(),
                    process_operation_id:get(original,"operation")?,
                    request_id:format!("holder-gone-{}",sha256_hex(&bytes)),pid:pair.0.to_string(),
                    creation_time_100ns:pair.1.to_string(),snapshot_hex:bytes_hex(&bytes),snapshot_digest:sha256_hex(&bytes)};
                let record=fail(gone::begin_holder_disappearance(&mut self.connection,&input,
                    proof.as_ref().ok_or_else(||refused("holder initial capture lacks disappearance proof"))?))?;
                (record,capture,step)
            };
            #[cfg(test)]
            holder_gone_cut_for_test("AFTER_CAPTURE")?;
            if record.phase==Phase::Preparing {
                let proof=proof.as_ref().ok_or_else(||refused("holder revoke lacks disappearance proof"))?;
                self.gone_scope(instance_id,&allowed,incoming,retained)?;
                if let Some(all_gone)=&all_gone {fail(all_gone.validate(&all_pairs))?;}
                let intent=fail(instance::begin_credential_profile(&mut self.connection,&instance::CredentialProfileIntent{
                    request_id:format!("{}-revoke",record.input.request_id),instance_id:instance_id.into(),
                    history_id:profile.history_id.clone(),binding_id:member.clone(),generation:profile.generation.clone(),
                    profile_sid:profile.profile_sid.clone(),source_file_identity:object.file_identity.clone(),
                    expected_revision:number(&capture,"profileRevision")?,action:instance::CredentialProfileAction::Revoke}))?;
                if !matches!(intent.disposition,instance::CredentialIntentDisposition::New|instance::CredentialIntentDisposition::Pending|instance::CredentialIntentDisposition::Applied) {
                    return Err(refused("holder recovery retains ambiguous original revoke"));
                }
                self.gone_scope(instance_id,&allowed,incoming,retained)?;
                if let Some(all_gone)=&all_gone {fail(all_gone.validate(&all_pairs))?;}
                let observed=if intent.disposition==instance::CredentialIntentDisposition::Applied {
                    if intent.profile.state!="REVOKED" {return Err(refused("holder applied revoke profile changed"));}
                    fail(step.readback_target(&binding,&proof,&[pair]))?
                } else {fail(step.apply_or_readback(&binding,&proof,&[pair]))?};
                if observed!=get(&capture,"aclTarget")? {return Err(refused("holder actual revoke target changed"));}
                #[cfg(test)]
                holder_gone_cut_for_test("AFTER_ACL")?;
                fail(instance::complete_credential_profile(&mut self.connection,&intent,instance::CredentialProfileResult::Revoked))?;
                #[cfg(test)]
                holder_gone_cut_for_test("AFTER_F_REVOKE")?;
                fail(self.connection.execute("BEGIN IMMEDIATE"))?;
                let advanced=(||->Result<HolderDisappearanceRecord>{
                    fail(authority::check_owner_in_current_transaction(&self.connection,&self.owner))?;
                    self.gone_scope_in_current_transaction(instance_id,&allowed,incoming,retained)?;
                    fail(gone::advance_in_transaction(&self.connection,&record,Phase::Preparing,Phase::Revoked,&proof))
                })();
                record=match advanced {Ok(r)=>{self.finish_native_transaction(Ok(()))?;r},
                    Err(e)=>{self.finish_native_transaction(Err(e))?;unreachable!()}};
                #[cfg(test)]
                holder_gone_cut_for_test("AFTER_REVOKED")?;
            }
            if record.phase==Phase::Revoked {
                let proof=proof.as_ref().ok_or_else(||refused("holder release lacks disappearance proof"))?;
                self.gone_scope(instance_id,&allowed,incoming,retained)?;
                if let Some(all_gone)=&all_gone {fail(all_gone.validate(&all_pairs))?;}
                fail(step.readback_target(&binding,&proof,&[pair]))?;
                fail(self.connection.execute("BEGIN IMMEDIATE"))?;
                let released=self.gone_release_in_transaction(&capture,&record,&proof);
                self.finish_native_transaction(released)?;
                record=fail(gone::read_holder_disappearance(&self.connection,member))?
                    .ok_or_else(||refused("holder release journal absent"))?;
                #[cfg(test)]
                holder_gone_cut_for_test("AFTER_RELEASE")?;
            }
            if record.phase!=Phase::Applied {return Err(refused("holder resource recovery incomplete"));}
            self.gone_validate_capture(&self.gone_original(&fail(instance::read_credential_profiles(&self.connection,instance_id))?
                .into_iter().find(|p|&p.binding_id==member).ok_or_else(||refused("holder final profile absent"))?)?,
                &capture,&object,&home.identity,&alias_physical,&record)?;
            final_digest=Some(step.target_digest());records.insert(member.clone(),record);
        }
        // Each complete group ends with a protected Owner/System baseline.
        // On later cold starts its immutable last step still supplies the same
        // expected baseline; subsequent legitimate stopped histories add no ACE.
        for record in records.values() {
                if record.phase!=Phase::Applied {return Err(refused("holder original resource recovery incomplete"));}
                let capture=decode(record)?;let group=get(&capture,"members")?;
                if final_digest.is_none() && group.split(',').last()==Some(record.input.binding_id.as_str()) {
                    let ids:Vec<_>=group.split(',').collect();
                    if ids.iter().all(|id|records.get(*id).is_some_and(|r|r.phase==Phase::Applied)) {
                        final_digest=Some(get(&capture,"aclTarget")?);
                    }
                }
                let fresh_profile=fail(instance::read_credential_profiles(&self.connection,instance_id))?
                    .into_iter().find(|p|p.binding_id==record.input.binding_id)
                    .ok_or_else(||refused("holder completed original absent"))?;
                self.gone_validate_capture(&self.gone_original(&fresh_profile)?,
                    &capture,&object,&home.identity,&alias_physical,record)?;
        }
        self.gone_scope(instance_id,&allowed,incoming,retained)?;
        if let Some(all_gone)=&all_gone {fail(all_gone.validate(&all_pairs))?;}
        if fail(instance::read_credential_profiles(&self.connection,instance_id))?.iter().any(|p|p.state!="REVOKED") {
            return Err(refused("holder cold adoption retains a profile"));
        }
        fail(binding.verify_registered_aliases(&scopes))?;
        fail(adopt_holder_gone_source_baseline(&binding,&final_digest.ok_or_else(||refused("holder baseline receipt absent"))?,&[]))?;
        self.disappeared_credential_holders.insert((instance_id.into(),object.file_identity.opaque()),binding);
        Ok(())
    }

    fn gone_validate_capture(&self, current:&Fields,capture:&Fields,object:&instance::CredentialObjectRecord,
        home:&RootIdentity,aliases:&Fields,record:&HolderDisappearanceRecord)->Result<()> {
        for key in ["binding","instance","history","domain","session","seat","incarnation","generation",
            "operation","ticket","nonce","sid","pid","creation","image","binary","custodyState",
            "stopHash","claimBinding","claimHome","episodePhase","episodeHex","episodeRevision"] {
            if get(current,key)?!=get(capture,key)? {return Err(refused("holder frozen original changed"));}
        }
        if get(capture,"root")?!=self.connection.root_identity().opaque()
            || get(capture,"database")?!=self.connection.identity().opaque()
            || get(capture,"home")?!=home.opaque() || get(capture,"source")?!=object.file_identity.opaque() {
            return Err(refused("holder frozen physical object changed"));
        }
        let original_aliases=parse_fields(&get(capture,"aliases")?)?;
        if original_aliases.iter().any(|(key,value)|aliases.get(key)!=Some(value))
            || (record.phase!=Phase::Applied && &original_aliases!=aliases) {
            return Err(refused("holder frozen alias set changed"));
        }
        let (state,revision)=if record.phase==Phase::Applied {("RELEASED".into(),number(capture,"claimRevision")?.checked_add(1)
            .ok_or_else(||refused("holder released revision overflow"))?)}
            else {(get(capture,"claimState")?,number(capture,"claimRevision")?)};
        if get(current,"claimState")?!=state || number(current,"claimRevision")?!=revision {
            return Err(refused("holder frozen claim receipt changed"));
        }
        if matches!(record.phase,Phase::Revoked|Phase::Applied) {
            fail(instance::read_completed_profile_revoke(&self.connection,&instance::CredentialProfileIntent {
                request_id:format!("{}-revoke",record.input.request_id),instance_id:get(capture,"instance")?,
                history_id:get(capture,"history")?,binding_id:get(capture,"binding")?,generation:get(capture,"generation")?,
                profile_sid:get(capture,"sid")?,source_file_identity:object.file_identity.clone(),
                expected_revision:number(capture,"profileRevision")?,action:instance::CredentialProfileAction::Revoke
            }))?;
        }
        if record.phase==Phase::Applied {
            let h=rows(&self.connection,
                "SELECT raw_hex,operation,status,CAST(previous_revision AS TEXT),CAST(revision AS TEXT),session_id
                   FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2",
                &[&get(capture,"domain")?,&record.input.request_id],6)?;
            if h!=vec![vec![bytes_hex(encoded(capture).as_bytes()),"holder-gone-release".into(),"APPLIED".into(),
                number(capture,"claimRevision")?.to_string(),revision.to_string(),get(capture,"session")?]] {
                return Err(refused("holder frozen H release receipt changed"));
            }
        }
        Ok(())
    }
}
