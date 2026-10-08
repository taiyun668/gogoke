//! Grok F resource retirement under the existing native cold-holder scope.
//! UNKNOWN episodes and NULL StopFacts remain historical process facts.
use super::*;
use crate::process::{AppContainerProfile, NativeProcessHoldersGone};
use crate::store::instance::GrokGrant;
use crate::store::session_transport::grok_home_launch;
use crate::store::digest::sha256_hex;

fn evidence<T,E:std::fmt::Debug>(value:std::result::Result<T,E>)->Result<T>{
    value.map_err(|error|OrchestrationError::V37StoreFailure(
        format!("Grok original holder recovery: {error:?}")))
}
fn denied(detail:&'static str)->OrchestrationError{OrchestrationError::Invalid(detail)}

#[derive(Clone,Debug,Eq,PartialEq)]
struct OriginalClaim {state:String,revision:i64,home_id:String,stop_fact:String}

impl<'root> ProductDatabase<'root> {
    fn grok_fully_retired(&self,grant:&GrokGrant)->Result<bool>{
        if grant.phase!="REVOKED"{return Ok(false);}
        if grant.process_operation_id.is_none(){
            // A terminal original F withdrawal is stronger than NULL process
            // fields. This only skips F retirement; H UNKNOWN/open intent and
            // capacity still require their existing original settlement.
            evidence(grok_home_launch::verify_completed_no_attempt(
                &self.connection,self.root,grant))?;
            return Ok(true);
        }
        if evidence(grok_home_launch::original_candidate_for_recovery(
            &self.connection,grant))?.is_some(){
            // A completed candidate F withdrawal has no new current H claim
            // or H release journal. Recheck its original holder and ACL below.
            return Ok(false);
        }
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT state,generation,binding_id,COALESCE(stop_fact_id,'') FROM main.gogoke_v37_h_claim
             WHERE instance_id=?1 AND domain_id=?2 AND session_id=?3")?;
        for (index,value) in [grant.instance_id.as_str(),&grant.domain_id,&grant.session_id].iter().enumerate(){
            q.bind_text(index as i32+1,value)?;
        }
        if !q.step_row()?{return Err(denied("Grok retired original claim absent"));}
        let same=q.column_text(1)?==grant.generation &&q.column_text(2)?==grant.binding_id;
        if same{
            let state=q.column_text(0)?;
            if state=="RELEASED"{return Ok(true);}
            let original_stop=q.column_text(3)?;
            return Ok(state=="STOPPED" &&grant.stop_fact_id.as_deref()
                .is_some_and(|stop|!stop.is_empty() &&original_stop==stop));
        }
        drop(q);
        // H may have legitimately advanced this logical session. Its old
        // real stop, or our original resource-release journal, remains bound
        // to the immutable F grant rather than to the new generation.
        if let Some(stop)=grant.stop_fact_id.as_deref(){
            let q=Statement::prepare(self.connection.as_ptr(),
                "SELECT 1 FROM main.gogoke_v37_h_process_episode e
                 JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id
                 WHERE e.instance_id=?1 AND e.domain_id=?2 AND e.session_id=?3 AND e.generation=?4
                   AND e.binding_id=?5 AND e.seat_id=?6 AND e.seat_incarnation=?7
                   AND e.process_operation_id=?8 AND e.phase='STOPPED' AND e.stop_fact_id=?9
                   AND c.state='STOPPED' AND c.stop_proof_hash=e.stop_fact_id")?;
            let values=[grant.instance_id.as_str(),&grant.domain_id,&grant.session_id,&grant.generation,
                &grant.binding_id,&grant.seat_id,&grant.seat_incarnation,
                grant.process_operation_id.as_deref().ok_or_else(||denied("Grok retired operation absent"))?,stop];
            for (index,value) in values.iter().enumerate(){q.bind_text(index as i32+1,value)?;}
            return Ok(q.step_row()?);
        }
        self.grok_release_journal_matches(grant,None)
    }

    fn grok_release_journal_matches(&self,grant:&GrokGrant,expected_revision:Option<i64>)->Result<bool>{
        let prefix=format!("{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
            grant.binding_id,grant.instance_id,grant.domain_id,grant.session_id,
            grant.seat_id,grant.seat_incarnation,grant.generation,grant.request_id,
            grant.pid.ok_or_else(||denied("Grok retired PID absent"))?,
            grant.creation_time_100ns.ok_or_else(||denied("Grok retired creation absent"))?);
        let prefix_hex:String=prefix.as_bytes().iter().map(|b|format!("{b:02x}")).collect();
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT raw_hex,request_id,previous_revision,revision FROM main.gogoke_v37_h_operation
             WHERE domain_id=?1 AND session_id=?2 AND operation='holder-gone-release' AND status='APPLIED'")?;
        q.bind_text(1,&grant.domain_id)?;q.bind_text(2,&grant.session_id)?;
        while q.step_row()?{
            let revision:i64=q.column_text(2)?.parse().map_err(|error|
                OrchestrationError::V37StoreFailure(format!("Grok retired revision: {error}")))?;
            if expected_revision.is_some_and(|expected|revision.checked_add(1)!=Some(expected)){continue;}
            let raw=format!("{prefix}{revision}");
            let expected=format!("{}{}",prefix_hex,revision.to_string().as_bytes().iter()
                .map(|b|format!("{b:02x}")).collect::<String>());
            if q.column_text(0)?==expected &&q.column_text(1)?==format!("grok-gone-{}",&sha256_hex(raw.as_bytes())[..40])
                &&revision.checked_add(1).map(|r|r.to_string()).as_deref()==Some(q.column_text(3)?.as_str()){
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn completed_grok_holder_release(&self,instance_id:&str,
        domain:&str,session:&str,operation:&str)->Result<bool>{
        for grant in evidence(instance::read_grok_grants(&self.connection,instance_id))? {
            if grant.domain_id!=domain || grant.session_id!=session
                || grant.process_operation_id.as_deref()!=Some(operation){continue;}
            if !matches!(grant.phase.as_str(),"REVOKED"|"RETIRED_CLEANUP_PENDING")
                || grant.stop_fact_id.is_some(){return Ok(false);}
            let original=self.grok_original_claim(&grant)?;
            if original.state!="RELEASED" || !original.stop_fact.is_empty(){return Ok(false);}
            return self.grok_release_journal_matches(&grant,Some(original.revision));
        }
        Ok(false)
    }

    fn grok_original_claim(&self,grant:&GrokGrant)->Result<OriginalClaim>{
        let suffix=sha256_hex(format!("{}\n{}\n{}\n{}\n{}",
            self.root.canonical_root().identity.opaque(),grant.domain_id,grant.session_id,
            grant.seat_incarnation,grant.generation).as_bytes());
        if grant.profile_name!=format!("Gogoke37.Session.{}",&suffix[..40]){
            return Err(denied("Grok original profile derivation changed"));
        }
        let profile=evidence(AppContainerProfile::derive_for_revocation(&grant.profile_name))?;
        if evidence(profile.sid_identity())?!=grant.profile_sid{
            return Err(denied("Grok original profile SID changed"));
        }
        let row=Statement::prepare(self.connection.as_ptr(),
            "SELECT h.state,h.revision,h.home_id,COALESCE(h.stop_fact_id,'')
             FROM main.gogoke_v37_h_claim h
             JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=h.process_operation_id
               AND e.instance_id=h.instance_id AND e.domain_id=h.domain_id
               AND e.session_id=h.session_id AND e.generation=h.generation AND e.binding_id=h.binding_id
             JOIN main.gogoke_v37_effective_seat s ON s.domain_id=h.domain_id
               AND s.session_id=h.session_id AND s.generation=h.generation
               AND e.seat_id=s.seat_id AND e.seat_incarnation=s.seat_incarnation
               AND s.selected_instance_id=h.instance_id
             JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id
               AND c.profile_id=h.instance_id AND c.domain_id=h.domain_id AND c.generation=h.generation
             WHERE h.instance_id=?1 AND h.domain_id=?2 AND h.session_id=?3 AND h.generation=?4
               AND h.binding_id=?5 AND s.seat_id=?6 AND s.seat_incarnation=?7 AND e.request_id=?8
               AND c.operation_id=?9 AND c.ticket=?10 AND c.custodian_nonce=?11
               AND c.pid=?12 AND c.creation_time_100ns=?13 AND c.image_path=?14
               AND c.binary_digest_sha256=?15
               AND ((h.stop_fact_id IS NULL AND e.stop_fact_id IS NULL
                     AND c.stop_proof_hash IS NULL AND c.state IN ('PREPARED','ACTIVE','UNKNOWN')
                     AND e.phase IN ('PREPARED','ACTIVE','UNKNOWN') AND h.state IN ('COMMITTED','UNKNOWN','RELEASED'))
                 OR (h.stop_fact_id IS NOT NULL AND h.stop_fact_id=e.stop_fact_id
                     AND h.stop_fact_id=c.stop_proof_hash AND c.state='STOPPED'
                     AND e.phase='STOPPED' AND h.state IN ('STOPPED','RELEASED')))")?;
        let pid=grant.pid.ok_or_else(||denied("Grok original PID absent"))?.to_string();
        let creation=grant.creation_time_100ns.ok_or_else(||denied("Grok original creation absent"))?.to_string();
        let values=[grant.instance_id.as_str(),&grant.domain_id,&grant.session_id,&grant.generation,
            &grant.binding_id,&grant.seat_id,&grant.seat_incarnation,&grant.request_id,
            grant.process_operation_id.as_deref().ok_or_else(||denied("Grok original operation absent"))?,
            grant.ticket.as_deref().ok_or_else(||denied("Grok original ticket absent"))?,
            grant.custodian_nonce.as_deref().ok_or_else(||denied("Grok original nonce absent"))?,
            &pid,&creation,grant.image_path.as_deref().ok_or_else(||denied("Grok original image absent"))?,
            &grant.program_digest];
        for (index,value) in values.iter().enumerate(){row.bind_text(index as i32+1,value)?;}
        if !row.step_row()?{return Err(denied("Grok original H/F/custody association absent"));}
        let result=OriginalClaim{state:row.column_text(0)?,revision:row.column_text(1)?.parse()
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("Grok original claim revision: {error}")))?,
            home_id:row.column_text(2)?,stop_fact:row.column_text(3)?};
        if row.step_row()?{return Err(denied("Grok original H/F/custody association duplicated"));}
        Ok(result)
    }

    fn release_grok_disappeared_claim(&mut self,grant:&GrokGrant,original:&OriginalClaim,
        proof:&NativeProcessHoldersGone,allowed:&[String],incoming:Option<&V37Request>)->Result<()>{
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let result=(||->Result<()>{
            self.gone_scope_in_current_transaction(&grant.instance_id,allowed,incoming,false)?;
            evidence(proof.validate(&[(grant.pid.ok_or_else(||denied("Grok release PID absent"))?,
                grant.creation_time_100ns.ok_or_else(||denied("Grok release creation absent"))?)]))?;
            if &self.grok_original_claim(grant)?!=original || !original.stop_fact.is_empty()
                || !matches!(original.state.as_str(),"COMMITTED"|"UNKNOWN"){
                return Err(denied("Grok disappeared claim changed before release"));
            }
            let current=evidence(instance::read_grok_grants(&self.connection,&grant.instance_id))?
                .into_iter().find(|g|g.binding_id==grant.binding_id)
                .ok_or_else(||denied("Grok release F grant absent"))?;
            if !matches!(current.phase.as_str(),"REVOKED"|"RETIRED_CLEANUP_PENDING"){
                return Err(denied("Grok release precedes original ACL retirement"));
            }
            let seat=evidence(crate::store::seat::get(&self.connection,&grant.domain_id,&grant.seat_id))?
                .ok_or_else(||denied("Grok release original seat absent"))?;
            let relationship=evidence(crate::store::session_transport::session_binding::current_relationship(
                &self.connection,&grant.domain_id,&grant.session_id))?
                .ok_or_else(||denied("Grok release original H/E relationship absent"))?;
            if seat.state!=crate::store::seat::State::Busy || seat.instance_id!=grant.instance_id
                || seat.incarnation!=grant.seat_incarnation || relationship.seat_id!=grant.seat_id
                || relationship.seat_incarnation!=grant.seat_incarnation
                || relationship.instance_id!=grant.instance_id || relationship.session_generation!=grant.generation
                || seat.generation!=relationship.seat_authorization_generation {
                return Err(denied("Grok release original seat changed"));
            }
            let q=Statement::prepare(self.connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_claim SET state='RELEASED',revision=revision+1
                 WHERE instance_id=?1 AND domain_id=?2 AND session_id=?3 AND generation=?4
                   AND binding_id=?5 AND process_operation_id=?6 AND home_id=?7
                   AND state=?8 AND revision=?9 AND stop_fact_id IS NULL")?;
            let values=[grant.instance_id.as_str(),&grant.domain_id,&grant.session_id,&grant.generation,
                &grant.binding_id,grant.process_operation_id.as_deref().ok_or_else(||denied("Grok release operation absent"))?,
                &original.home_id,&original.state];
            for (index,value) in values.iter().enumerate(){q.bind_text(index as i32+1,value)?;}
            q.bind_i64(9,original.revision)?;q.step_done()?;drop(q);
            let count=Statement::prepare(self.connection.as_ptr(),"SELECT changes()")?;
            if !count.step_row()? || count.column_text(0)?!="1"{return Err(denied("Grok release claim CAS"));}
            drop(count);
            let occupied=evidence(crate::store::session_transport::session_binding::has_unreleased_seat_claim(
                &self.connection,&grant.domain_id,&grant.seat_id,&grant.seat_incarnation))?;
            if !occupied {
                evidence(crate::store::seat::set_dispatch_state_in_transaction(&mut self.connection,&seat,false))?;
            }
            // The original F binding and immutable custody tuple are retained;
            // this journal reports resource release, never a process StopFact.
            let raw=format!("{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
                grant.binding_id,grant.instance_id,grant.domain_id,grant.session_id,
                grant.seat_id,grant.seat_incarnation,grant.generation,grant.request_id,
                grant.pid.ok_or_else(||denied("Grok release PID absent"))?,
                grant.creation_time_100ns.ok_or_else(||denied("Grok release creation absent"))?,original.revision);
            let request_id=format!("grok-gone-{}",&sha256_hex(raw.as_bytes())[..40]);
            let raw_hex:String=raw.as_bytes().iter().map(|b|format!("{b:02x}")).collect();
            let op=Statement::prepare(self.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,
                 session_id,status,previous_revision,revision) VALUES(?1,?2,?3,'holder-gone-release',?4,'APPLIED',?5,?6)")?;
            for (index,value) in [grant.domain_id.as_str(),&request_id,&raw_hex,&grant.session_id].iter().enumerate(){
                op.bind_text(index as i32+1,value)?;
            }
            op.bind_i64(5,original.revision)?;
            op.bind_i64(6,original.revision.checked_add(1).ok_or_else(||denied("Grok release revision overflow"))?)?;
            op.step_done()?;
            Ok(())
        })();
        self.finish_native_transaction(result)
    }

    pub(crate) fn recover_grok_home_resources(&mut self,instance_id:&str,
        incoming:Option<&V37Request>)->Result<()>{
        let Some(registered)=self.read_registered_instance(instance_id)? else{return Ok(());};
        if registered.driver_id!="grok"{return Ok(());}
        // Live peers continue through their original H custodian. Cold
        // recovery must never take their shared domain away from them.
        if self.native_sessions.values().any(|run|run.custody.binding.profile_id==instance_id){return Ok(());}
        let inventory=evidence(grok_home_launch::cold_inventory(&self.connection,instance_id))?;
        let mut allowed:Vec<String>=inventory.iter().filter_map(|g|g.process_operation_id.clone()).collect();
        let mut grants=Vec::new();
        for grant in inventory{
            if !self.grok_fully_retired(&grant)?{grants.push(grant);}
        }
        if grants.is_empty(){return Ok(());}
        let mut prepared=BTreeMap::new();
        for grant in grants.iter().filter(|g|g.phase=="GRANTED_UNCREATED"){
            let holder=evidence(grok_home_launch::prepared_holder_for_recovery(&self.connection,grant))?
                .ok_or_else(||denied("Grok cold grant has no original H process; no-attempt not inferred"))?;
            if !allowed.contains(&holder.0){allowed.push(holder.0.clone());}
            prepared.insert(grant.binding_id.clone(),holder);
        }
        self.gone_scope(instance_id,&allowed,incoming,false)?;
        for mut grant in grants {
            if grant.phase=="RETIRED_CLEANUP_PENDING" &&grant.stop_fact_id.is_some(){
                // A normal peer can outlive this stopped generation, which
                // may already have resumed and moved H's current pointer.
                // Validate the immutable historical stop/effects here; the
                // final quiescent domain scan still decides F completion.
                evidence(grok_home_launch::verify_retired_stopped(
                    &self.connection,self.root,&grant))?;
                continue;
            }
            if grant.phase=="GRANT_PENDING"{
                return Err(denied("Grok cold grant lacks confirmed process/no-attempt boundary; original fence retained"));
            }
            if grant.phase=="GRANTED_UNCREATED"{
                let (_,pid,creation)=prepared.get(&grant.binding_id)
                    .ok_or_else(||denied("Grok original prepared holder absent"))?;
                let proof=evidence(NativeProcessHoldersGone::observe(&[(*pid,*creation)]))?;
                self.gone_scope(instance_id,&allowed,incoming,false)?;
                evidence(grok_home_launch::adopt_original_holder_gone(
                    &mut self.connection,self.root,&grant,&proof))?;
                grant=evidence(instance::read_grok_grants(&self.connection,instance_id))?
                    .into_iter().find(|g|g.binding_id==grant.binding_id)
                    .ok_or_else(||denied("Grok original adopted row absent"))?;
            }
            if let Some((operation,pid,creation))=evidence(
                grok_home_launch::original_candidate_for_recovery(&self.connection,&grant))? {
                if grant.process_operation_id.as_deref()!=Some(operation.as_str()) {
                    return Err(denied("Grok candidate original operation changed"));
                }
                let proof=evidence(NativeProcessHoldersGone::observe(&[(pid,creation)]))?;
                self.gone_scope(instance_id,&allowed,incoming,false)?;
                if matches!(grant.phase.as_str(),"ACTIVE"|"REVOKE_PENDING") {
                    evidence(grok_home_launch::retire_holder_gone(
                        &mut self.connection,self.root,&grant,&proof))?;
                }else if !matches!(grant.phase.as_str(),"REVOKED"|"RETIRED_CLEANUP_PENDING") {
                    return Err(denied("Grok candidate original F effect unresolved"));
                }
                let retired=evidence(instance::read_grok_grants(&self.connection,instance_id))?
                    .into_iter().find(|g|g.binding_id==grant.binding_id)
                    .ok_or_else(||denied("Grok original candidate retired row absent"))?;
                evidence(grok_home_launch::verify_completed_holder_gone(
                    &self.connection,self.root,&retired,&proof))?;
                // Candidate UNKNOWN/NULL StopFact is not an H release or stop.
                continue;
            }
            let original=self.grok_original_claim(&grant)?;
            if !original.stop_fact.is_empty(){
                if !matches!(grant.phase.as_str(),"REVOKED"|"RETIRED_CLEANUP_PENDING"){
                    evidence(grok_home_launch::resume_stopped_revoke(&mut self.connection,self.root,&grant))?;
                }
                continue;
            }
            if original.state=="RELEASED"{continue;}
            let pair=(grant.pid.ok_or_else(||denied("Grok cold PID absent"))?,
                grant.creation_time_100ns.ok_or_else(||denied("Grok cold creation absent"))?);
            let proof=evidence(NativeProcessHoldersGone::observe(&[pair]))?;
            self.gone_scope(instance_id,&allowed,incoming,false)?;
            if matches!(grant.phase.as_str(),"ACTIVE"|"REVOKE_PENDING"){
                evidence(grok_home_launch::retire_holder_gone(&mut self.connection,self.root,&grant,&proof))?;
            }else if !matches!(grant.phase.as_str(),"REVOKED"|"RETIRED_CLEANUP_PENDING"){
                return Err(denied("Grok cold original effect unresolved"));
            }
            let retired=evidence(instance::read_grok_grants(&self.connection,instance_id))?
                .into_iter().find(|g|g.binding_id==grant.binding_id)
                .ok_or_else(||denied("Grok original retired row absent"))?;
            evidence(grok_home_launch::verify_completed_holder_gone(&self.connection,self.root,&retired,&proof))?;
            self.release_grok_disappeared_claim(&retired,&original,&proof,&allowed,incoming)?;
        }
        self.gone_scope(instance_id,&allowed,incoming,false)?;
        evidence(grok_home_launch::finalize_quiescent(&mut self.connection,self.root,instance_id))
    }
}
