//! Claude's original H holder can disappear without a ProcessCustodian
//! StopFact. This retires only its three exact writable ACL trees and then
//! releases its original H claim. A failed-resume child retires only its own
//! resources; its parent's true STOPPED claim is unchanged. No model replay.
use super::*;
use crate::process::{AppContainerProfile, ClaudeAclRetirement, NativeProcessHoldersGone};
use crate::root::RootIdentity;
use crate::store::digest::sha256_hex;
use crate::store::session_transport::launch::original_session_profile_name;
use crate::store::{seat, worktree};
use std::path::PathBuf;

#[path = "v37_claude_holder_recovery/journal.rs"]
mod journal;

pub(super) fn initialize_claude_holder_recovery_schema(
    db: &mut VerifiedDatabaseConnection<'_>,
) -> Result<()> {
    journal::initialize(db)
}
fn denied(reason: &'static str) -> OrchestrationError {
    OrchestrationError::Invalid(reason)
}
fn evidence<T, E: std::fmt::Debug>(value: std::result::Result<T, E>) -> Result<T> {
    value.map_err(|error| {
        OrchestrationError::V37StoreFailure(format!("Claude disappeared holder: {error:?}"))
    })
}
fn rows(
    db: &VerifiedDatabaseConnection<'_>,
    sql: &str,
    params: &[&str],
    columns: usize,
) -> Result<Vec<Vec<String>>> {
    let q = Statement::prepare(db.as_ptr(), sql)?;
    for (n, value) in params.iter().enumerate() {
        q.bind_text(n as i32 + 1, value)?;
    }
    let mut found = Vec::new();
    while q.step_row()? {
        found.push(
            (0..columns)
                .map(|n| q.column_text(n as i32))
                .collect::<std::result::Result<Vec<_>, _>>()?,
        );
    }
    Ok(found)
}
fn unhex(value: &str) -> Result<Vec<u8>> {
    if value.is_empty()
        || value.len() > 131072
        || value.len() % 2 != 0
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(denied("Claude original H bytes invalid"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(
                std::str::from_utf8(pair).map_err(|_| denied("Claude original H hex UTF8"))?,
                16,
            )
            .map_err(|_| denied("Claude original H hex digit"))
        })
        .collect()
}
fn fact(facts: &BTreeMap<String, String>, key: &str) -> Result<String> {
    facts
        .get(key)
        .cloned()
        .ok_or_else(|| denied("Claude holder original fact absent"))
}
fn put(facts: &mut BTreeMap<String, String>, key: &str, value: &str) {
    facts.insert(key.into(), value.into());
}

#[derive(Clone, Debug)]
struct Original {
    facts: BTreeMap<String, String>,
    pair: (u32, u64),
    claim_revision: i64,
    claim_state: String,
    profile_name: String,
    sid: String,
    repository: String,
    worktree: String,
}

impl<'root> ProductDatabase<'root> {
    // A normally stopped H leaves its own ACE on the reusable instance HOME.
    // It is a preserved peer, not a holder to release again. Admit its SID only
    // from the exact original open and matching claim/episode/custody StopFact;
    // an arbitrary package SID present on disk remains a mismatch.
    fn claude_stopped_peers(
        &self,
        instance: &str,
        captured: Option<&[String]>,
    ) -> Result<(Vec<String>, String, Vec<(String,String,String,String,String)>)> {
        let stopped = rows(
            &self.connection,
            "SELECT a.domain_id,a.session_id,a.generation,e.seat_incarnation,
                    e.request_id,e.raw_hex,e.seat_id,o.raw_hex,o.status,
                    a.binding_id,a.home_id,a.process_operation_id,a.stop_fact_id,
                    c.ticket,c.custodian_nonce,c.pid,c.creation_time_100ns,
                    c.image_path,c.binary_digest_sha256,e.stop_request_id,s.raw_hex,s.status
             FROM main.gogoke_v37_h_claim a
             JOIN main.gogoke_v37_h_process_episode e
               ON e.process_operation_id=a.process_operation_id
              AND e.domain_id=a.domain_id AND e.session_id=a.session_id
              AND e.generation=a.generation AND e.instance_id=a.instance_id
              AND e.home_id=a.home_id AND e.binding_id=a.binding_id
             JOIN main.gogoke_coordination_process_custody c
               ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
              AND c.generation=e.generation AND c.profile_id=e.instance_id
             JOIN main.gogoke_v37_h_operation o
               ON o.domain_id=e.domain_id AND o.request_id=e.request_id
              AND o.session_id=e.session_id AND o.operation='open'
             JOIN main.gogoke_v37_h_operation s
               ON s.domain_id=e.domain_id AND s.request_id=e.stop_request_id
              AND s.session_id=e.session_id AND s.operation='stop'
             WHERE a.instance_id=?1 AND a.state='RELEASED' AND e.phase='STOPPED'
               AND c.state='STOPPED' AND e.old_generation IS NULL
               AND a.stop_fact_id IS NOT NULL AND a.stop_fact_id!=''
               AND e.stop_fact_id=a.stop_fact_id AND c.stop_proof_hash=a.stop_fact_id
             ORDER BY a.domain_id,a.session_id",
            &[instance],
            22,
        )?;
        let mut sids = Vec::new();
        let mut provenance = Vec::new();
        let mut scopes = Vec::new();
        for r in stopped {
            let name = original_session_profile_name(
                &self.root.canonical_root().identity.opaque(), &r[0], &r[1], &r[3], &r[2],
            );
            let profile = evidence(AppContainerProfile::derive_for_revocation(&name))?;
            let sid = evidence(profile.sid_identity())?;
            if captured.is_some_and(|members| !members.contains(&sid)) {
                // A later normally stopped session is not part of this older
                // receipt. New captures still qualify the complete current set.
                continue;
            }
            let raw = unhex(&r[5])?;
            let request = evidence(decode_request(&raw))?;
            if r[5] != r[7] || r[8] != "APPLIED"
                || request.raw_bytes != raw || request.family != "K-SESSION"
                || request.operation != "open" || request.request_id != r[4]
                || request.domain_id != r[0] || request.target_id != r[1]
                || request.payload.len() != 4
                || user_payload_string(&request, "seatId")? != r[6]
                || user_payload_string(&request, "generation")? != r[2]
            {
                return Err(denied("Claude stopped peer original open changed"));
            }
            // Validate the other original payload fields, without interpreting
            // either as a path or granting any new access.
            let repository=user_payload_string(&request, "repositoryId")?;
            let tree=user_payload_string(&request, "worktreeId")?;
            let stop_raw = unhex(&r[20])?;
            let stop = evidence(decode_request(&stop_raw))?;
            if r[21] != "APPLIED" || stop.raw_bytes != stop_raw
                || stop.family != "K-SESSION" || stop.operation != "stop"
                || stop.request_id != r[19] || stop.domain_id != r[0] || stop.target_id != r[1]
                || stop.payload.len() != 2
                || user_payload_string(&stop, "seatId")? != r[6]
                || user_payload_string(&stop, "generation")? != r[2]
            {
                return Err(denied("Claude stopped peer original stop changed"));
            }
            if sids.contains(&sid) {
                return Err(denied("Claude stopped peer SID reused"));
            }
            scopes.push((sid.clone(),tree,repository,r[0].clone(),r[6].clone()));
            sids.push(sid);
            for value in r {
                provenance.extend_from_slice(&(value.len() as u64).to_be_bytes());
                provenance.extend_from_slice(value.as_bytes());
            }
        }
        sids.sort();
        if captured.is_some_and(|members| members != sids.as_slice()) {
            return Err(denied("Claude captured stopped peer missing or changed"));
        }
        Ok((sids, sha256_hex(&provenance), scopes))
    }

    fn claude_originals(&self, instance: &str) -> Result<Vec<Original>> {
        self.claude_originals_with_current_pin(instance, true)
    }

    // The version action verifies the historical C image and today's managed
    // SOURCE separately before calling its completed-release reader. Other
    // recovery callers continue to require the historical C to be current.
    fn claude_originals_with_current_pin(&self, instance: &str, require_current_pin: bool) -> Result<Vec<Original>> {
        let registered = self
            .read_registered_instance(instance)?
            .ok_or_else(|| denied("Claude original registered instance absent"))?;
        if registered.driver_id != "claude" {
            return Err(denied("Claude original driver changed"));
        }
        let sql="SELECT a.domain_id,a.session_id,a.generation,a.binding_id,a.home_id,
                   a.state,CAST(a.revision AS TEXT),a.process_operation_id,
                   COALESCE(a.stop_fact_id,''),e.request_id,e.raw_hex,e.phase,
                   e.seat_id,e.seat_incarnation,e.home_id,e.binding_id,
                   e.instance_id,e.generation,e.process_operation_id,COALESCE(e.stop_fact_id,''),
                   c.ticket,c.custodian_nonce,c.pid,c.creation_time_100ns,c.image_path,
                   c.binary_digest_sha256,c.state,COALESCE(c.stop_proof_hash,'')
              FROM main.gogoke_v37_h_claim a
              JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=a.process_operation_id
                AND e.domain_id=a.domain_id AND e.session_id=a.session_id
                AND e.generation=a.generation AND e.instance_id=a.instance_id
                AND e.home_id=a.home_id AND e.binding_id=a.binding_id
              JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id
                AND c.domain_id=e.domain_id AND c.generation=e.generation AND c.profile_id=e.instance_id
             WHERE a.instance_id=?1 AND a.state IN ('COMMITTED','UNKNOWN','RELEASED')
               AND a.process_operation_id IS NOT NULL AND e.old_generation IS NULL
               AND NOT (a.state='RELEASED' AND a.stop_fact_id IS NOT NULL
                 AND e.phase='STOPPED' AND e.stop_fact_id=a.stop_fact_id
                 AND c.state='STOPPED' AND c.stop_proof_hash=a.stop_fact_id)
             ORDER BY a.domain_id,a.session_id";
        let mut originals = Vec::new();
        for r in rows(&self.connection, sql, &[instance], 28)? {
            if r[3].is_empty()
                || r[4].is_empty()
                || r[7].is_empty()
                || !r[8].is_empty()
                || !r[19].is_empty()
                || !r[27].is_empty()
                || !matches!(r[11].as_str(), "PREPARED" | "ACTIVE" | "UNKNOWN")
                || !matches!(r[26].as_str(), "PREPARED" | "ACTIVE" | "UNKNOWN")
                || r[14] != r[4]
                || r[15] != r[3]
                || r[16] != instance
                || r[17] != r[2]
                || r[18] != r[7]
                || (require_current_pin && r[25] != registered.program_digest)
            {
                return Err(denied("Claude original H episode/custody changed"));
            }
            if r[16] != instance {
                return Err(denied("Claude original instance mismatch"));
            }
            let pid = r[22]
                .parse::<u32>()
                .map_err(|_| denied("Claude original PID"))?;
            let creation = r[23]
                .parse::<u64>()
                .map_err(|_| denied("Claude original creation"))?;
            if pid == 0
                || creation == 0
                || r[22] != pid.to_string()
                || r[23] != creation.to_string()
            {
                return Err(denied("Claude original process identity"));
            }
            let claim_revision = r[6]
                .parse::<i64>()
                .map_err(|_| denied("Claude claim revision"))?;
            if claim_revision < 1 {
                return Err(denied("Claude claim revision invalid"));
            }
            let raw = unhex(&r[10])?;
            let request = evidence(decode_request(&raw))?;
            if request.raw_bytes != raw
                || request.family != "K-SESSION"
                || request.operation != "open"
                || request.request_id != r[9]
                || request.domain_id != r[0]
                || request.target_id != r[1]
                || request.payload.len() != 4
                || user_payload_string(&request, "seatId")? != r[12]
                || user_payload_string(&request, "generation")? != r[2]
            {
                return Err(denied("Claude original open bytes/tuple"));
            }
            let repository = user_payload_string(&request, "repositoryId")?;
            let worktree = user_payload_string(&request, "worktreeId")?;
            let open_effect = rows(
                &self.connection,
                "SELECT raw_hex,operation,session_id,status FROM main.gogoke_v37_h_operation
                 WHERE domain_id=?1 AND request_id=?2",
                &[&r[0], &r[9]],
                4,
            )?;
            let [effect] = open_effect.as_slice() else {
                return Err(denied("Claude original open effect missing or duplicated"));
            };
            // UNKNOWN records the original admitted launch whose outcome was
            // not confirmed. It is not a successful open, nor a StopFact. Its
            // exact tuple still identifies the old holder being retired.
            if effect[0] != r[10]
                || effect[1] != "open"
                || effect[2] != r[1]
                || !matches!(effect[3].as_str(), "APPLIED" | "UNKNOWN")
            {
                return Err(denied("Claude original open effect tuple/status changed"));
            }
            let relationship = evidence(
                crate::store::session_transport::session_binding::current_relationship(
                    &self.connection,
                    &r[0],
                    &r[1],
                ),
            )?;
            if r[5] != "RELEASED"
                && relationship.as_ref().is_none_or(|s| {
                    s.seat_id != r[12]
                        || s.seat_incarnation != r[13]
                        || s.instance_id != instance
                        || s.session_generation != r[2]
                })
            {
                return Err(denied("Claude original E/H binding changed"));
            }
            let profile_name = original_session_profile_name(
                &self.root.canonical_root().identity.opaque(),
                &r[0],
                &r[1],
                &r[13],
                &r[2],
            );
            let profile = evidence(AppContainerProfile::derive_for_revocation(&profile_name))?;
            let sid = evidence(profile.sid_identity())?;
            let mut facts = BTreeMap::new();
            for (key, index) in [
                ("domain", 0),
                ("session", 1),
                ("generation", 2),
                ("binding", 3),
                ("home", 4),
                ("claimState", 5),
                ("claimRevision", 6),
                ("operation", 7),
                ("openRequest", 9),
                ("openRaw", 10),
                ("episodePhase", 11),
                ("seat", 12),
                ("incarnation", 13),
                ("ticket", 20),
                ("nonce", 21),
                ("pid", 22),
                ("creation", 23),
                ("image", 24),
                ("binary", 25),
                ("custodyState", 26),
            ] {
                put(&mut facts, key, &r[index]);
            }
            put(&mut facts, "instance", instance);
            put(&mut facts, "openStatus", &effect[3]);
            put(&mut facts, "repository", &repository);
            put(&mut facts, "worktree", &worktree);
            put(&mut facts, "profile", &profile_name);
            put(&mut facts, "sid", &sid);
            originals.push(Original {
                facts,
                pair: (pid, creation),
                claim_revision,
                claim_state: r[5].clone(),
                profile_name,
                sid,
                repository,
                worktree,
            });
        }
        Ok(originals)
    }

    fn claude_roots(&self, original: &Original) -> Result<Vec<(PathBuf, RootIdentity, bool)>> {
        let f = &original.facts;
        let profile = evidence(AppContainerProfile::derive_for_revocation(
            &original.profile_name,
        ))?;
        if evidence(profile.sid_identity())? != original.sid {
            return Err(denied("Claude original profile SID"));
        }
        let released=original.claim_state=="RELEASED";
        let homes=if released {
            evidence(instance::resolve_historical_session_homes(
                &self.connection,self.root,&profile,&fact(f,"instance")?,
                &fact(f,"home")?,&fact(f,"domain")?,&fact(f,"session")?,
                &fact(f,"generation")?,&fact(f,"binding")?,"claude"))?
        } else {
            evidence(instance::resolve_provider_session_launch_homes(
                &self.connection,self.root,&profile,&fact(f,"instance")?,
                &fact(f,"home")?,&fact(f,"domain")?,&fact(f,"session")?,
                &fact(f,"generation")?,"claude"))?
        };
        let seat = evidence(seat::get(
            &self.connection,
            &fact(f, "domain")?,
            &fact(f, "seat")?,
        ))?
        .ok_or_else(|| denied("Claude original seat absent"))?;
        let child = resource_only(original);
        if seat.state == seat::State::Reclaimed
            || seat.instance_id != fact(f, "instance")?
            || seat.incarnation != fact(f, "incarnation")?
            || (!released && !child && seat.state!=seat::State::Busy)
        {
            return Err(denied("Claude original E authority changed"));
        }
        if !released {
            let relationship=evidence(
                crate::store::session_transport::session_binding::current_relationship(
                    &self.connection,&fact(f,"domain")?,&fact(f,"session")?))?
                .ok_or_else(||denied("Claude original current relationship absent"))?;
            let relationship_generation=if child {fact(f,"parentGeneration")?}
                else {fact(f,"generation")?};
            if relationship.seat_id!=seat.seat_id ||
                relationship.seat_incarnation!=seat.incarnation ||
                relationship.instance_id!=seat.instance_id ||
                relationship.session_generation!=relationship_generation ||
                relationship.seat_authorization_generation!=seat.generation {
                return Err(denied("Claude original E authority changed"));
            }
        }
        let tier = evidence(seat::permission_tier(&seat))?;
        let writable = matches!(
            tier,
            seat::PermissionTier::IsolatedWrite | seat::PermissionTier::NetworkedWrite
        );
        let binding = evidence(worktree::resolve_for_launch(
            &self.connection,
            self.root,
            &original.worktree,
            &original.repository,
            &fact(f, "domain")?,
            &seat.seat_id,
            &seat.incarnation,
            seat.generation,
        ))?;
        let graph = evidence(worktree::graph_query(&self.connection, &original.worktree))?
            .ok_or_else(|| denied("Claude original F graph absent"))?;
        if graph.classification != "SINGLE"
            || graph.members.len() != 1
            || graph.members[0].worktree_id != original.worktree
            || binding.instance_id != seat.instance_id
            || binding.permission_tier
                != match tier {
                    // F persists the exact Rust variant spelling, not the
                    // uppercase permission tier used by the USER wire format.
                    seat::PermissionTier::ReadOnly => "ReadOnly",
                    seat::PermissionTier::NoNetwork => "NoNetwork",
                    seat::PermissionTier::IsolatedWrite => "IsolatedWrite",
                    seat::PermissionTier::NetworkedWrite => "NetworkedWrite",
                }
        {
            return Err(denied("Claude original SINGLE F/tier changed"));
        }
        let group = evidence(worktree::resolve_group_for_launch(
            &self.connection,
            self.root,
            &binding,
        ))?;
        if group.len() != 1
            || group[0].worktree_id != original.worktree
            || group[0].identity != binding.identity
        {
            return Err(denied("Claude original F group changed"));
        }
        Ok(vec![
            (homes.instance.path, homes.instance.identity, true),
            (homes.session.path, homes.session.identity, true),
            (binding.path, binding.identity, writable),
        ])
    }

    fn claude_capture(
        &self,
        original: &Original,
        known_sids: &[String],
    ) -> Result<journal::Capture> {
        let roots = self.claude_roots(original)?;
        let profile = evidence(AppContainerProfile::derive_for_revocation(
            &original.profile_name,
        ))?;
        let objects = evidence(ClaudeAclRetirement::capture(&profile, &roots, known_sids))?;
        let mut facts = original.facts.clone();
        let (stopped_sids, stopped_provenance) = if resource_only(original) {
            let (sids, provenance, _) = self.claude_child_peers(&fact(&facts, "instance")?)?;
            (sids, provenance)
        } else {
            let (sids,provenance,_)=self.claude_stopped_peers(&fact(&facts, "instance")?, None)?;
            (sids,provenance)
        };
        put(&mut facts, "stoppedPeerSids", &stopped_sids.join(","));
        put(&mut facts, "stoppedPeerProvenance", &stopped_provenance);
        for (key, value) in [
            ("root", self.connection.root_identity().opaque()),
            ("database", self.connection.identity().opaque()),
            ("instanceHome", roots[0].1.opaque()),
            ("sessionHome", roots[1].1.opaque()),
            ("worktreeRoot", roots[2].1.opaque()),
            (
                "worktreeWritable",
                if roots[2].2 { "1".into() } else { "0".into() },
            ),
            ("knownSids", known_sids.join(",")),
        ] {
            put(&mut facts, key, &value);
        }
        let digest = sha256_hex(fact(&facts, "operation")?.as_bytes());
        put(
            &mut facts,
            "request",
            &format!("claude-gone-{}", &digest[..40]),
        );
        Ok(journal::Capture { facts, objects })
    }

    fn claude_validate_capture(
        &self,
        original: &Original,
        record: &journal::Record,
        known_sids: &[String],
        released: bool,
    ) -> Result<Vec<(PathBuf, RootIdentity, bool)>> {
        let saved = &record.capture.facts;
        let current = &original.facts;
        if saved.get("kind") != current.get("kind") {
            return Err(denied("Claude recovery kind changed"));
        }
        if resource_only(original) && current.iter().any(|(key,value)|saved.get(key)!=Some(value)) {
            return Err(denied("Claude child captured original tuple changed"));
        }
        for key in [
            "instance",
            "domain",
            "session",
            "generation",
            "binding",
            "home",
            "operation",
            "openRequest",
            "openRaw",
            "openStatus",
            "episodePhase",
            "seat",
            "incarnation",
            "ticket",
            "nonce",
            "pid",
            "creation",
            "image",
            "binary",
            "custodyState",
            "repository",
            "worktree",
            "profile",
            "sid",
        ] {
            if fact(saved, key)? != fact(current, key)? {
                return Err(denied("Claude original H tuple changed"));
            }
        }
        if fact(saved, "root")? != self.connection.root_identity().opaque()
            || fact(saved, "database")? != self.connection.identity().opaque()
            || fact(saved, "knownSids")? != known_sids.join(",")
        {
            return Err(denied("Claude original root/database/SID set changed"));
        }
        // Requalify historical peers both while pending and after completion,
        // including inside the H release transaction. A matching SID alone is
        // insufficient if its original stop/open provenance has since changed.
        let captured = fact(saved, "stoppedPeerSids")?;
        let captured: Vec<String> = if captured.is_empty() { Vec::new() }
            else { captured.split(',').map(str::to_owned).collect() };
        let stopped_provenance = if resource_only(original) {
            let (sids, provenance, _) = self.claude_child_peers(&fact(saved, "instance")?)?;
            if sids != captured { return Err(denied("Claude child peer set changed")); }
            let mut expected=sids;
            for child in self.claude_failed_resume_originals(&fact(saved,"instance")?)? {
                if expected.contains(&child.sid) { return Err(denied("Claude child SID reused")); }
                expected.push(child.sid);
            }
            expected.sort();
            if expected != known_sids {return Err(denied("Claude child known SID provenance changed"));}
            provenance
        } else {
            self.claude_stopped_peers(&fact(saved, "instance")?, Some(&captured))?.1
        };
        if fact(saved, "stoppedPeerProvenance")? != stopped_provenance {
            return Err(denied("Claude stopped peer provenance changed"));
        }
        if released {
            let expected_revision = fact(saved, "claimRevision")?
                .parse::<i64>()
                .map_err(|_| denied("Claude captured revision"))?
                .checked_add(1)
                .ok_or_else(|| denied("Claude captured revision overflow"))?;
            if original.claim_state != "RELEASED" || original.claim_revision != expected_revision {
                return Err(denied("Claude H release receipt changed"));
            }
        }
        if !released && (original.claim_state != fact(saved, "claimState")?
            || original.claim_revision
                != fact(saved, "claimRevision")?
                    .parse::<i64>()
                    .map_err(|_| denied("Claude captured revision"))?)
        {
            return Err(denied("Claude original H claim changed"));
        }
        let roots = self.claude_roots(original)?;
        for (index, key) in ["instanceHome", "sessionHome", "worktreeRoot"]
            .iter()
            .enumerate()
        {
            if fact(saved, key)? != roots[index].1.opaque() {
                return Err(denied("Claude physical root changed"));
            }
        }
        if fact(saved, "worktreeWritable")? != if roots[2].2 { "1" } else { "0" } {
            return Err(denied("Claude F permission changed"));
        }
        evidence(ClaudeAclRetirement::verify_inventory(
            &roots,
            &record.capture.objects,
        ))?;
        Ok(roots)
    }

    fn claude_release(
        &mut self,
        original: &Original,
        record: &journal::Record,
        known_sids: &[String],
        proof: &NativeProcessHoldersGone,
        original_pairs: &[(u32, u64)],
        allowed: &[String],
        incoming: Option<&V37Request>,
    ) -> Result<()> {
        if resource_only(original) {
            if incoming.is_some() {return Err(denied("Claude child retirement is not continuation"));}
            return self.claude_resource_only_finish(original,record,known_sids,proof,original_pairs,allowed);
        }
        let instance = fact(&original.facts, "instance")?;
        self.connection
            .execute("BEGIN IMMEDIATE")
            .map_err(OrchestrationError::CommitUnknownWithCause)?;
        let result = (|| -> Result<()> {
            self.gone_scope_in_current_transaction(&instance, allowed, incoming, false)?;
            evidence(proof.validate(original_pairs))?;
            let current = self
                .claude_originals(&instance)?
                .into_iter()
                .find(|item| {
                    item.pair == original.pair
                        && item.facts.get("operation") == original.facts.get("operation")
                })
                .ok_or_else(|| denied("Claude release original absent"))?;
            let roots = self.claude_validate_capture(&current, record, known_sids, false)?;
            let profile = evidence(AppContainerProfile::derive_for_revocation(
                &current.profile_name,
            ))?;
            evidence(ClaudeAclRetirement::verify_inventory(
                &roots,
                &record.capture.objects,
            ))?;
            for object in &record.capture.objects {
                // H release requires exact already-completed ACL effects;
                // this transaction never performs a second ACL write.
                evidence(ClaudeAclRetirement::readback_target(
                    &profile,
                    &roots,
                    object,
                    known_sids,
                    proof,
                    original_pairs,
                ))?;
            }
            let q = Statement::prepare(
                self.connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_claim SET state='RELEASED',revision=revision+1
                 WHERE instance_id=?1 AND domain_id=?2 AND session_id=?3 AND generation=?4
                   AND binding_id=?5 AND home_id=?6 AND process_operation_id=?7
                   AND state=?8 AND revision=?9 AND stop_fact_id IS NULL",
            )?;
            let values = [
                instance.clone(),
                fact(&current.facts, "domain")?,
                fact(&current.facts, "session")?,
                fact(&current.facts, "generation")?,
                fact(&current.facts, "binding")?,
                fact(&current.facts, "home")?,
                fact(&current.facts, "operation")?,
                current.claim_state.clone(),
            ];
            for (i, value) in values.iter().enumerate() {
                q.bind_text(i as i32 + 1, value)?;
            }
            q.bind_i64(9, current.claim_revision)?;
            q.step_done()?;
            drop(q);
            if rows(&self.connection, "SELECT changes()", &[], 1)? != vec![vec![String::from("1")]]
            {
                return Err(denied("Claude H release CAS"));
            }
            let domain = fact(&current.facts, "domain")?;
            let seat_id = fact(&current.facts, "seat")?;
            let seat = evidence(seat::get(&self.connection, &domain, &seat_id))?
                .ok_or_else(|| denied("Claude release E seat absent"))?;
            if seat.state != seat::State::Busy
                || seat.incarnation != fact(&current.facts, "incarnation")?
            {
                return Err(denied("Claude release E seat changed"));
            }
            if !evidence(
                crate::store::session_transport::session_binding::has_unreleased_seat_claim(
                    &self.connection,
                    &domain,
                    &seat_id,
                    &seat.incarnation,
                ),
            )? {
                evidence(seat::set_dispatch_state_in_transaction(
                    &mut self.connection,
                    &seat,
                    false,
                ))?;
            }
            let op = Statement::prepare(
                self.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,
                 session_id,status,previous_revision,revision)
                 VALUES(?1,?2,?3,'claude-holder-gone-release',?4,'APPLIED',?5,?6)",
            )?;
            let session = fact(&current.facts, "session")?;
            for (i, value) in [
                domain.as_str(),
                record.request_id.as_str(),
                record.snapshot_hex.as_str(),
                session.as_str(),
            ]
            .iter()
            .enumerate()
            {
                op.bind_text(i as i32 + 1, value)?;
            }
            op.bind_i64(5, current.claim_revision)?;
            op.bind_i64(
                6,
                current
                    .claim_revision
                    .checked_add(1)
                    .ok_or_else(|| denied("Claude claim revision overflow"))?,
            )?;
            op.step_done()?;
            journal::applied_in_transaction(&self.connection, record)?;
            Ok(())
        })();
        self.finish_native_transaction(result)
    }

    /// A retained UNKNOWN command is not a live resource owner after its exact
    /// original open has independently completed retirement. Never changes its
    /// status or grants replay, continuation, or a StopFact.
    pub(super) fn completed_claude_managed_pending_operation(
        &self, instance:&str, domain:&str, session:&str, request_id:&str,
    )->Result<bool> {
        let found=rows(&self.connection,
            "SELECT o.raw_hex,o.operation,o.status,a.process_operation_id,e.generation,
                    e.seat_id,e.request_id,e.raw_hex,COALESCE(e.stop_request_id,'')
               FROM main.gogoke_v37_h_operation o
               JOIN main.gogoke_v37_h_claim a ON a.domain_id=o.domain_id AND a.session_id=o.session_id
               JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=a.process_operation_id
                AND e.domain_id=a.domain_id AND e.session_id=a.session_id
                AND e.generation=a.generation AND e.instance_id=a.instance_id
              WHERE a.instance_id=?1 AND o.domain_id=?2 AND o.session_id=?3 AND o.request_id=?4
                AND e.old_generation IS NULL", &[instance,domain,session,request_id],9)?;
        let [r]=found.as_slice() else{return Ok(false)};
        if r[2]!="UNKNOWN" || !matches!(r[1].as_str(),"open"|"stop") || r[3].is_empty() {
            return Ok(false);
        }
        let raw=unhex(&r[0])?;
        let request=evidence(decode_request(&raw))?;
        if request.raw_bytes!=raw || request.family!="K-SESSION" || request.operation!=r[1]
            || request.domain_id!=domain || request.target_id!=session || request.request_id!=request_id
            || user_payload_string(&request,"generation")?!=r[4]
            || user_payload_string(&request,"seatId")?!=r[5] {return Ok(false)}
        if r[1]=="open" {
            if request.payload.len()!=4 || r[6]!=request_id || r[7]!=r[0] {return Ok(false)}
            user_payload_string(&request,"repositoryId")?;
            user_payload_string(&request,"worktreeId")?;
        } else if request.payload.len()!=2 || r[8]!=request_id {return Ok(false)}
        self.completed_claude_holder_release(instance,domain,session,&r[3])
    }

    pub(super) fn completed_claude_managed_pending_stdin(
        &self, instance:&str, domain:&str, request_id:&str,
    )->Result<bool> {
        let found=rows(&self.connection,
            "SELECT j.session_id,j.generation,j.process_operation_id,j.request_hex,j.operation,j.expected_revision
               FROM main.gogoke_v37_h_stdin_journal j
               JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=j.process_operation_id
                AND e.instance_id=?1 AND e.domain_id=j.domain_id AND e.session_id=j.session_id
                AND e.generation=j.generation AND e.old_generation IS NULL
               JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id
                AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation
                AND c.ticket=j.ticket AND c.custodian_nonce=j.custodian_nonce
              WHERE j.domain_id=?2 AND j.request_id=?3 AND j.phase IN ('PREPARED','UNKNOWN')",
            &[instance,domain,request_id],6)?;
        let [r]=found.as_slice() else{return Ok(false)};
        if !matches!(r[4].as_str(),"send"|"steer"|"interrupt") || r[2].is_empty(){return Ok(false)}
        let raw=unhex(&r[3])?;
        let request=evidence(decode_request(&raw))?;
        if request.raw_bytes!=raw || request.family!="K-SESSION" || request.operation!=r[4]
            || request.request_id!=request_id || request.domain_id!=domain || request.target_id!=r[0]
            || user_payload_string(&request,"generation")?!=r[1]
            || request.expected_revision.to_string()!=r[5] {return Ok(false)}
        self.completed_claude_holder_release(instance,domain,&r[0],&r[2])
    }

    pub(super) fn completed_claude_holder_release(
        &self,
        instance: &str,
        domain: &str,
        session: &str,
        operation: &str,
    ) -> Result<bool> {
        self.completed_claude_holder_release_reader(instance, domain, session, operation, true)
    }

    pub(super) fn completed_claude_holder_release_for_version(
        &self, instance: &str, domain: &str, session: &str, operation: &str,
    ) -> Result<bool> {
        self.completed_claude_holder_release_reader(instance, domain, session, operation, false)
    }

    fn completed_claude_holder_release_reader(
        &self, instance: &str, domain: &str, session: &str, operation: &str,
        require_current_pin: bool,
    ) -> Result<bool> {
        let mut verified=Vec::new();
        for original in self.claude_originals_with_current_pin(instance,require_current_pin)? {
            if original.claim_state!="RELEASED" {continue;}
            let old_operation=fact(&original.facts,"operation")?;
            let Some(record)=journal::read(&self.connection,&old_operation)? else {return Ok(false)};
            if record.phase!="APPLIED" || record.instance!=instance ||
                record.domain!=fact(&original.facts,"domain")? ||
                record.session!=fact(&original.facts,"session")? ||
                record.operation!=old_operation || record.revision!=2 ||
                record.capture.objects.is_empty() {return Ok(false)}
            let known=fact(&record.capture.facts,"knownSids")?
                .split(',').map(str::to_owned).collect::<Vec<_>>();
            let roots=match self.claude_validate_capture(&original,&record,&known,true) {
                Ok(roots)=>roots,Err(_)=>return Ok(false),
            };
            let receipt=rows(&self.connection,
                "SELECT raw_hex,status,previous_revision,revision,session_id FROM main.gogoke_v37_h_operation
                  WHERE domain_id=?1 AND request_id=?2 AND operation='claude-holder-gone-release'",
                &[&record.domain,&record.request_id],5)?;
            if receipt!=vec![vec![record.snapshot_hex.clone(),"APPLIED".into(),
                fact(&record.capture.facts,"claimRevision")?,
                original.claim_revision.to_string(),record.session.clone()]] {return Ok(false)}
            verified.push((original,record,roots,known));
        }
        let Some(index)=verified.iter().position(|(old,record,_,_)|
            record.operation==operation && record.domain==domain && record.session==session &&
            old.facts.get("operation").is_some_and(|value|value==operation)) else {return Ok(false)};
        let stopped=self.claude_stopped_peers(instance,None)?.2;
        let (original,record,roots,known)=&verified[index];
        let captured=fact(&record.capture.facts,"stoppedPeerSids")?;
        let captured=if captured.is_empty(){Vec::new()}else{
            captured.split(',').map(str::to_owned).collect::<Vec<_>>()};
        let mut later=Vec::new();
        for (sid,tree,repository,peer_domain,peer_seat) in stopped.iter()
            .filter(|(sid,_,_,_,_)|!captured.contains(sid)) {
            let same_worktree=tree==&original.worktree;
            if same_worktree {
                let graph=evidence(worktree::graph_query(&self.connection,tree))?
                    .ok_or_else(||denied("Claude later stopped peer F graph absent"))?;
                if graph.classification!="SINGLE" || graph.members.len()!=1 ||
                    graph.members[0].worktree_id.as_str()!=tree.as_str() ||
                    graph.members[0].repository_id.as_str()!=repository.as_str() ||
                    graph.members[0].instance_id!=instance ||
                    graph.members[0].domain_id.as_str()!=peer_domain.as_str() ||
                    graph.members[0].seat_id.as_str()!=peer_seat.as_str() {
                    return Err(denied("Claude later stopped peer F scope changed"));
                }
            }
            later.push((sid.as_str(),same_worktree));
        }
        let completed=verified.iter().map(|(old,record,roots,_)|
            (old.sid.as_str(),roots.as_slice(),record.capture.objects.as_slice()))
            .collect::<Vec<_>>();
        let profile=evidence(AppContainerProfile::derive_for_revocation(&original.profile_name))?;
        let proof=evidence(NativeProcessHoldersGone::observe(&[original.pair]))?;
        evidence(ClaudeAclRetirement::readback_completed_target(
            &profile,roots,&record.capture.objects,known,&completed,&later,
            &proof,&[original.pair],
        ))?;
        Ok(true)
    }

    fn claude_fence_unknown(
        &mut self,
        record: &journal::Record,
        original_error: &OrchestrationError,
    ) -> Result<()> {
        self.connection
            .execute("BEGIN IMMEDIATE")
            .map_err(OrchestrationError::CommitUnknownWithCause)?;
        let result = (|| -> Result<()> {
            authority::check_owner_in_current_transaction(&self.connection, &self.owner)?;
            let current = journal::read(&self.connection, &record.operation)?
                .ok_or_else(|| denied("Claude holder captured intent vanished"))?;
            if current != *record {
                return Err(denied("Claude holder captured intent changed"));
            }
            journal::unknown_in_transaction(
                &self.connection,
                record,
                &format!("{original_error:?}"),
            )
        })();
        self.finish_native_transaction(result)
    }

    pub(super) fn recover_disappeared_claude_resources(
        &mut self,
        instance: &str,
        incoming: Option<&V37Request>,
    ) -> Result<()> {
        self.recover_claude_resources(instance,incoming,false)
    }

    fn recover_claude_resources(
        &mut self, instance:&str, incoming:Option<&V37Request>, child_only:bool,
    )->Result<()> {
        let Some(registered) = self.read_registered_instance(instance)? else {
            return Ok(());
        };
        if registered.driver_id != "claude" {
            return Ok(());
        }
        if self
            .native_sessions
            .values()
            .any(|run| run.custody.binding.profile_id == instance)
        {
            return Ok(());
        }
        let originals = self.claude_recovery_originals(instance,child_only)?;
        if originals.is_empty() {
            return Ok(());
        }
        for old in originals.iter().filter(|old| old.claim_state == "RELEASED") {
            if !self.completed_claude_holder_release(
                instance,
                &fact(&old.facts, "domain")?,
                &fact(&old.facts, "session")?,
                &fact(&old.facts, "operation")?,
            )? {
                return Err(denied("Claude prior disappeared holder release unverified"));
            }
        }
        let mut active=Vec::new();
        for original in &originals {
            if original.claim_state=="RELEASED" {continue;}
            if child_only && self.claude_recovery_completed(instance,original)? {continue;}
            active.push(original);
        }
        if active.is_empty() {
            return Ok(());
        }
        let mut allowed: Vec<String> = originals
            .iter()
            .map(|o| fact(&o.facts, "operation"))
            .collect::<Result<_>>()?;
        if child_only {allowed.extend(self.claude_child_peers(instance)?.2);}
        self.gone_scope(instance, &allowed, incoming, false)?;
        let pairs: Vec<_> = active.iter().map(|o| o.pair).collect();
        let all_gone = evidence(NativeProcessHoldersGone::observe(&pairs))?;
        let mut known_sids: Vec<String> = originals.iter().map(|o| o.sid.clone()).collect();
        known_sids.sort();
        known_sids.dedup();
        if known_sids.len() != originals.len() {
            return Err(denied("Claude original SID reused"));
        }
        let peers=if child_only {self.claude_child_peers(instance)?.0}
            else {self.claude_stopped_peers(instance,None)?.0};
        for peer in peers {
            if known_sids.contains(&peer) {
                return Err(denied("Claude stopped peer SID reused"));
            }
            known_sids.push(peer);
        }
        known_sids.sort();
        known_sids.dedup();
        for original in active {
            let operation = fact(&original.facts, "operation")?;
            evidence(all_gone.validate(&pairs))?;
            let record = match journal::read(&self.connection, &operation)? {
                Some(record) => record,
                None => {
                    let capture = self.claude_capture(original, &known_sids)?;
                    self.connection
                        .execute("BEGIN IMMEDIATE")
                        .map_err(OrchestrationError::CommitUnknownWithCause)?;
                    let result = (|| -> Result<journal::Record> {
                        self.gone_scope_in_current_transaction(
                            instance, &allowed, incoming, false,
                        )?;
                        evidence(all_gone.validate(&pairs))?;
                        let current = self
                            .claude_recovery_originals(instance,child_only)?
                            .into_iter()
                            .find(|o| {
                                o.pair == original.pair
                                    && o.facts.get("operation") == original.facts.get("operation")
                            })
                            .ok_or_else(|| denied("Claude capture original vanished"))?;
                        if current.facts != original.facts {
                            return Err(denied("Claude capture original changed"));
                        }
                        journal::insert_in_transaction(&self.connection, capture)
                    })();
                    match result {
                        Ok(record) => {
                            self.finish_native_transaction(Ok(()))?;
                            record
                        }
                        Err(error) => {
                            self.finish_native_transaction(Err(error))?;
                            unreachable!()
                        }
                    }
                }
            };
            if record.phase != "PREPARED" {
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "Claude holder recovery journal {}: {}",
                    record.phase, record.original_error
                )));
            }
            let settled = (|| -> Result<()> {
                let roots = self.claude_validate_capture(original, &record, &known_sids, false)?;
                let profile = evidence(AppContainerProfile::derive_for_revocation(
                    &original.profile_name,
                ))?;
                for object in &record.capture.objects {
                    self.gone_scope(instance, &allowed, incoming, false)?;
                    evidence(all_gone.validate(&pairs))?;
                    if child_only {
                        let current=self.claude_failed_resume_originals(instance)?.into_iter()
                            .find(|o|o.facts.get("operation")==Some(&operation))
                            .ok_or_else(||denied("Claude child missing before ACL write"))?;
                        if current.facts!=original.facts
                            || journal::read(&self.connection,&operation)?.as_ref()!=Some(&record) {
                            return Err(denied("Claude child tuple/intent changed before ACL write"));
                        }
                        self.claude_validate_capture(&current,&record,&known_sids,false)?;
                    }
                    evidence(ClaudeAclRetirement::verify_inventory(
                        &roots,
                        &record.capture.objects,
                    ))?;
                    evidence(ClaudeAclRetirement::apply(
                        &profile,
                        &roots,
                        object,
                        &known_sids,
                        &all_gone,
                        &pairs,
                    ))?;
                }
                self.gone_scope(instance, &allowed, incoming, false)?;
                evidence(all_gone.validate(&pairs))?;
                self.claude_release(
                    original,
                    &record,
                    &known_sids,
                    &all_gone,
                    &pairs,
                    &allowed,
                    incoming,
                )
            })();
            if let Err(original_error) = settled {
                let current_record = journal::read(&self.connection, &operation)?
                    .ok_or_else(|| denied("Claude captured intent vanished after failure"))?;
                if current_record.phase == "APPLIED" {
                    if self.claude_recovery_completed(instance,original)? {
                        continue;
                    }
                    return Err(denied("Claude applied release lacks exact receipt"));
                }
                if current_record != record {
                    return Err(denied("Claude captured intent changed after failure"));
                }
                let progress = (|| -> Result<()> {
                    self.gone_scope(instance, &allowed, incoming, false)?;
                    evidence(all_gone.validate(&pairs))?;
                    let current = self
                        .claude_recovery_originals(instance,child_only)?
                        .into_iter()
                        .find(|o| {
                            o.pair == original.pair
                                && o.facts.get("operation") == original.facts.get("operation")
                        })
                        .ok_or_else(|| denied("Claude original holder vanished during failure"))?;
                    if current.facts != original.facts {
                        return Err(denied("Claude original holder changed during failure"));
                    }
                    let roots =
                        self.claude_validate_capture(&current, &record, &known_sids, false)?;
                    let profile = evidence(AppContainerProfile::derive_for_revocation(
                        &current.profile_name,
                    ))?;
                    evidence(ClaudeAclRetirement::verify_progress(
                        &profile,
                        &roots,
                        &record.capture.objects,
                        &known_sids,
                    ))
                })();
                if let Err(progress_error) = progress {
                    if let Err(fence_error) = self.claude_fence_unknown(&record, &original_error) {
                        return Err(OrchestrationError::V37StoreFailure(format!(
                            "Claude holder original failure: {original_error:?}; drift: {progress_error:?}; UNKNOWN journal CAS failed: {fence_error:?}")));
                    }
                }
                return Err(original_error);
            }
        }
        Ok(())
    }
}


const FAILED_RESUME_RESOURCE_ONLY: &str = "FAILED_RESUME_RESOURCE_ONLY";
fn resource_only(original: &Original) -> bool {
    original.facts.get("kind").map(String::as_str) == Some(FAILED_RESUME_RESOURCE_ONLY)
}

impl<'root> ProductDatabase<'root> {
    // Migration-only candidate. The old H claim remains a real STOPPED claim.
    // The new generation is identified by its own original episode and custody.
    fn claude_failed_resume_originals(&self, instance: &str) -> Result<Vec<Original>> {
        let registered = self.read_registered_instance(instance)?
            .ok_or_else(|| denied("Claude child instance missing"))?;
        if registered.driver_id != "claude" { return Err(denied("Claude child driver")); }
        let fields = [
            ("e.domain_id","domain"),("e.session_id","session"),("e.generation","generation"),
            ("e.old_generation","parentGeneration"),("e.binding_id","binding"),("e.home_id","home"),
            ("e.phase","episodePhase"),("e.raw_hex","openRaw"),("e.request_id","openRequest"),
            ("e.seat_id","seat"),("e.seat_incarnation","incarnation"),
            ("CAST(e.previous_revision AS TEXT)","previousRevision"),
            ("COALESCE(CAST(e.result_revision AS TEXT),'')","resultRevision"),
            ("e.process_operation_id","operation"),("c.ticket","ticket"),("c.custodian_nonce","nonce"),
            ("c.pid","pid"),("c.creation_time_100ns","creation"),("c.image_path","image"),
            ("c.binary_digest_sha256","binary"),("c.state","custodyState"),
            ("COALESCE(c.stop_proof_hash,'')","stopHash"),("COALESCE(e.stop_fact_id,'')","episodeStop"),
            ("a.state","claimState"),("CAST(a.revision AS TEXT)","claimRevision"),
            ("a.process_operation_id","parentOperation"),("a.stop_fact_id","parentStop"),
            ("a.home_id","parentHome"),("a.binding_id","parentBinding"),
            ("p.raw_hex","parentOpenRaw"),("p.request_id","parentOpenRequest"),
            ("p.stop_request_id","parentStopRequest"),("pc.ticket","parentTicket"),
            ("pc.custodian_nonce","parentNonce"),("pc.pid","parentPid"),
            ("pc.creation_time_100ns","parentCreation"),("pc.image_path","parentImage"),
            ("pc.binary_digest_sha256","parentBinary"),("o.raw_hex","parentEffectRaw"),
            ("o.status","parentOpenStatus"),("stop.raw_hex","parentStopRaw"),
            ("stop.status","parentStopStatus"),("CAST(stop.previous_revision AS TEXT)","parentStopPrevious"),
            ("CAST(stop.revision AS TEXT)","parentStopRevision"),
        ];
        let sql = format!("SELECT {} FROM main.gogoke_v37_h_process_episode e
            JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id
              AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation
            JOIN main.gogoke_v37_h_claim a ON a.instance_id=e.instance_id
              AND a.domain_id=e.domain_id AND a.session_id=e.session_id AND a.generation=e.old_generation
            JOIN main.gogoke_v37_h_process_episode p ON p.process_operation_id=a.process_operation_id
              AND p.instance_id=a.instance_id AND p.domain_id=a.domain_id AND p.session_id=a.session_id
              AND p.generation=a.generation AND p.home_id=a.home_id AND p.binding_id=a.binding_id
              AND p.seat_id=e.seat_id AND p.seat_incarnation=e.seat_incarnation
              AND p.old_generation IS NULL AND p.phase='STOPPED' AND p.stop_fact_id=a.stop_fact_id
            JOIN main.gogoke_coordination_process_custody pc ON pc.operation_id=p.process_operation_id
              AND pc.profile_id=p.instance_id AND pc.domain_id=p.domain_id AND pc.generation=p.generation
              AND pc.state='STOPPED' AND pc.stop_proof_hash=a.stop_fact_id
            JOIN main.gogoke_v37_h_generation pg ON pg.domain_id=p.domain_id
              AND pg.session_id=p.session_id AND pg.generation=p.generation
              AND pg.request_id=p.request_id AND pg.process_operation_id=p.process_operation_id
            JOIN main.gogoke_v37_h_operation o ON o.domain_id=p.domain_id AND o.request_id=p.request_id
              AND o.session_id=p.session_id AND o.operation='open'
            JOIN main.gogoke_v37_h_operation stop ON stop.domain_id=p.domain_id
              AND stop.request_id=p.stop_request_id AND stop.session_id=p.session_id AND stop.operation='stop'
            WHERE e.instance_id=?1 AND e.old_generation IS NOT NULL
              AND e.phase='UNKNOWN' AND e.stop_fact_id IS NULL
              AND c.state='UNKNOWN' AND c.stop_proof_hash IS NULL AND a.state='STOPPED'
              AND a.stop_fact_id IS NOT NULL AND a.stop_fact_id<>''
              AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_generation g
                WHERE g.process_operation_id=e.process_operation_id OR
                (g.domain_id=e.domain_id AND g.session_id=e.session_id AND g.generation=e.generation))
              AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_generation_change gc
                WHERE gc.domain_id=e.domain_id AND gc.request_id=e.request_id)
            ORDER BY e.domain_id,e.session_id,e.generation",
            fields.iter().map(|(sql,_)|*sql).collect::<Vec<_>>().join(","));
        let mut found = Vec::new();
        for row in rows(&self.connection,&sql,&[instance],fields.len())? {
            let mut f: BTreeMap<String,String> = fields.iter().zip(row)
                .map(|((_,key),value)|((*key).into(),value)).collect();
            let number = |key:&str| -> Result<u64> {
                let raw=fact(&f,key)?;
                let n=raw.parse::<u64>().map_err(|_|denied("Claude child integer"))?;
                if n==0 || n.to_string()!=raw {return Err(denied("Claude child canonical integer"));}
                Ok(n)
            };
            for key in ["domain","session","binding","home","seat","incarnation","operation",
                "ticket","nonce","image","parentOperation","parentStop","parentHome","parentBinding",
                "parentTicket","parentNonce","parentImage"] {
                if fact(&f,key)?.is_empty() {return Err(denied("Claude child original field missing"));}
            }
            number("parentPid")?;
            number("parentCreation")?;
            let old=number("parentGeneration")?;
            let new=number("generation")?;
            let previous=number("previousRevision")?;
            let revision=number("claimRevision")?;
            if old.checked_add(1)!=Some(new) || previous.checked_add(1)!=Some(revision)
                || number("resultRevision")?!=revision
                || fact(&f,"binary")?!=registered.program_digest
                || fact(&f,"parentBinary")?!=registered.program_digest
                || fact(&f,"image")?!=fact(&f,"parentImage")?
                || fact(&f,"parentOpenRaw")?!=fact(&f,"parentEffectRaw")?
                || fact(&f,"parentOpenStatus")?!="APPLIED"
                || fact(&f,"parentStopStatus")?!="APPLIED"
                || number("parentStopRevision")?!=previous {
                return Err(denied("Claude child parent/candidate lineage"));
            }
            let raw=unhex(&fact(&f,"openRaw")?)?;
            let request=evidence(decode_request(&raw))?;
            if request.raw_bytes!=raw || request.family!="K-SESSION" || request.operation!="resume"
                || request.request_id!=fact(&f,"openRequest")?
                || request.domain_id!=fact(&f,"domain")? || request.target_id!=fact(&f,"session")?
                || request.payload.len()!=1 || request.expected_revision!=previous
                || user_payload_string(&request,"generation")?!=old.to_string()
                || fact(&f,"operation")?!=format!("h-resume-{}",&sha256_hex(&raw)[..40]) {
                return Err(denied("Claude child original resume bytes"));
            }
            let parent_raw=unhex(&fact(&f,"parentOpenRaw")?)?;
            let parent=evidence(decode_request(&parent_raw))?;
            if parent.raw_bytes!=parent_raw || parent.family!="K-SESSION" || parent.operation!="open"
                || parent.request_id!=fact(&f,"parentOpenRequest")?
                || parent.domain_id!=request.domain_id || parent.target_id!=request.target_id
                || parent.payload.len()!=4 || user_payload_string(&parent,"seatId")?!=fact(&f,"seat")?
                || user_payload_string(&parent,"generation")?!=old.to_string()
                || fact(&f,"parentOperation")?!=format!("h-open-{}",&sha256_hex(&parent_raw)[..40]) {
                return Err(denied("Claude child original parent open"));
            }
            let stop_raw=unhex(&fact(&f,"parentStopRaw")?)?;
            let stop=evidence(decode_request(&stop_raw))?;
            if stop.raw_bytes!=stop_raw || stop.family!="K-SESSION" || stop.operation!="stop"
                || stop.request_id!=fact(&f,"parentStopRequest")?
                || stop.domain_id!=request.domain_id || stop.target_id!=request.target_id
                || stop.payload.len()!=2 || user_payload_string(&stop,"seatId")?!=fact(&f,"seat")?
                || user_payload_string(&stop,"generation")?!=old.to_string()
                || stop.expected_revision!=number("parentStopPrevious")? {
                return Err(denied("Claude child original parent stop"));
            }
            let pid=u32::try_from(number("pid")?).map_err(|_|denied("Claude child PID"))?;
            let pair=(pid,number("creation")?);
            let claim_revision=i64::try_from(revision).map_err(|_|denied("Claude parent revision"))?;
            let repository=user_payload_string(&parent,"repositoryId")?;
            let worktree=user_payload_string(&parent,"worktreeId")?;
            let profile_name=original_session_profile_name(
                &self.root.canonical_root().identity.opaque(),&request.domain_id,&request.target_id,
                &fact(&f,"incarnation")?,&new.to_string());
            let profile=evidence(AppContainerProfile::derive_for_revocation(&profile_name))?;
            let sid=evidence(profile.sid_identity())?;
            let current_seat=evidence(seat::get(&self.connection,&request.domain_id,&fact(&f,"seat")?))?
                .ok_or_else(||denied("Claude child original seat absent"))?;
            let relation=evidence(crate::store::session_transport::session_binding::current_relationship(
                &self.connection,&request.domain_id,&request.target_id))?
                .ok_or_else(||denied("Claude child original relationship absent"))?;
            if current_seat.state==seat::State::Reclaimed || current_seat.instance_id!=instance
                || current_seat.incarnation!=fact(&f,"incarnation")?
                || relation.seat_id!=current_seat.seat_id || relation.seat_incarnation!=current_seat.incarnation
                || relation.instance_id!=instance || relation.session_generation!=old.to_string()
                || relation.seat_authorization_generation!=current_seat.generation {
                return Err(denied("Claude child parent E relationship changed"));
            }
            // These are resource-binding observations, not a BUSY requirement.
            put(&mut f,"seatGeneration",&current_seat.generation.to_string());
            put(&mut f,"seatRevision",&current_seat.revision.to_string());
            put(&mut f,"seatState",&format!("{:?}",current_seat.state));
            put(&mut f,"seatTier",&format!("{:?}",evidence(seat::permission_tier(&current_seat))?));
            for (key,value) in [("kind",FAILED_RESUME_RESOURCE_ONLY),("instance",instance),
                ("openStatus","UNKNOWN"),("repository",&repository),("worktree",&worktree),
                ("profile",&profile_name),("sid",&sid)] {put(&mut f,key,value);}
            found.push(Original {facts:f,pair,claim_revision,claim_state:"STOPPED".into(),
                profile_name,sid,repository,worktree});
        }
        Ok(found)
    }


    // A stopped resume peer inherits its identity from a complete chain of
    // original stopped generations. The live claim may already point elsewhere.
    // Follow strictly decreasing canonical generations; no depth/time fallback.
    pub(super) fn claude_stopped_peer_parent_chain(
        &self,instance:&str,binary:&str,child:&[String],
    )->Result<Vec<Vec<String>>> {
        let mut next=child[7].clone();
        let mut child_generation=child[2].clone();
        // UNKNOWN promotion may change episode.previous_revision; the original
        // resume request bytes still name the exact parent stop revision.
        let mut child_previous=evidence(decode_request(&unhex(&child[6])?))?.expected_revision;
        let mut chain=Vec::new();
        while !next.is_empty() {
            let parent_number=next.parse::<u64>()
                .map_err(|_|denied("Claude peer parent generation"))?;
            let child_number=child_generation.parse::<u64>()
                .map_err(|_|denied("Claude peer child generation"))?;
            if parent_number==0 || parent_number.to_string()!=next
                || child_number.to_string()!=child_generation
                || parent_number.checked_add(1)!=Some(child_number) {
                return Err(denied("Claude peer parent generation edge"));
            }
            let found=rows(&self.connection,
                "SELECT e.domain_id,e.session_id,e.generation,e.seat_incarnation,e.seat_id,
                        e.request_id,e.raw_hex,COALESCE(e.old_generation,''),e.process_operation_id,
                        e.stop_request_id,e.stop_fact_id,c.ticket,c.custodian_nonce,c.pid,
                        c.creation_time_100ns,c.image_path,c.binary_digest_sha256,
                        s.raw_hex,s.status,COALESCE(o.raw_hex,''),COALESCE(o.status,''),
                        e.home_id,e.binding_id,CAST(e.previous_revision AS TEXT),
                        COALESCE(CAST(e.result_revision AS TEXT),''),
                        CAST(s.previous_revision AS TEXT),CAST(s.revision AS TEXT)
                 FROM main.gogoke_v37_h_process_episode e
                 JOIN main.gogoke_coordination_process_custody c
                   ON c.operation_id=e.process_operation_id AND c.profile_id=e.instance_id
                  AND c.domain_id=e.domain_id AND c.generation=e.generation
                  AND c.state='STOPPED' AND c.stop_proof_hash=e.stop_fact_id
                 JOIN main.gogoke_v37_h_generation g
                   ON g.domain_id=e.domain_id AND g.session_id=e.session_id
                  AND g.generation=e.generation AND g.request_id=e.request_id
                  AND g.process_operation_id=e.process_operation_id
                 JOIN main.gogoke_v37_h_operation s
                   ON s.domain_id=e.domain_id AND s.session_id=e.session_id
                  AND s.request_id=e.stop_request_id AND s.operation='stop'
                 LEFT JOIN main.gogoke_v37_h_operation o
                   ON o.domain_id=e.domain_id AND o.session_id=e.session_id
                  AND o.request_id=e.request_id AND o.operation='open'
                 WHERE e.instance_id=?1 AND e.domain_id=?2 AND e.session_id=?3
                   AND e.generation=?4 AND e.seat_id=?5 AND e.seat_incarnation=?6
                   AND e.phase='STOPPED' AND e.stop_fact_id IS NOT NULL
                   AND e.stop_fact_id<>''",
                &[instance,&child[0],&child[1],&next,&child[4],&child[3]],27)?;
            let [r]=found.as_slice() else {
                return Err(denied("Claude stopped peer parent tuple missing or duplicated"));
            };
            if r[16]!=binary || r[18]!="APPLIED"
                || [8usize,9,10,11,12,13,14,15,21,22,24].iter().any(|&i|r[i].is_empty()) {
                return Err(denied("Claude stopped peer parent custody/receipt"));
            }
            let raw=unhex(&r[6])?;
            let start=evidence(decode_request(&raw))?;
            if start.raw_bytes!=raw || start.family!="K-SESSION"
                || start.request_id!=r[5] || start.domain_id!=r[0] || start.target_id!=r[1] {
                return Err(denied("Claude stopped peer parent original request"));
            }
            if r[7].is_empty() {
                if start.operation!="open" || start.payload.len()!=4
                    || r[19]!=r[6] || r[20]!="APPLIED"
                    || user_payload_string(&start,"seatId")?!=r[4]
                    || user_payload_string(&start,"generation")?!=r[2] {
                    return Err(denied("Claude stopped peer ancestor open"));
                }
                user_payload_string(&start,"repositoryId")?;
                user_payload_string(&start,"worktreeId")?;
            } else {
                // A native resume has no h_operation start receipt. Its own
                // exact episode + generation mapping is the durable promotion.
                if start.operation!="resume" || start.payload.len()!=1
                    || user_payload_string(&start,"generation")?!=r[7]
                    || r[8]!=format!("h-resume-{}",&sha256_hex(&raw)[..40]) {
                    return Err(denied("Claude stopped peer ancestor resume"));
                }
            }
            let raw=unhex(&r[17])?;
            let stop=evidence(decode_request(&raw))?;
            let stop_previous=r[25].parse::<u64>()
                .map_err(|_|denied("Claude peer parent stop previous revision"))?;
            let stop_revision=r[26].parse::<u64>()
                .map_err(|_|denied("Claude peer parent stop revision"))?;
            if stop.raw_bytes!=raw || stop.family!="K-SESSION" || stop.operation!="stop"
                || stop.request_id!=r[9] || stop.domain_id!=r[0] || stop.target_id!=r[1]
                || stop.payload.len()!=2 || user_payload_string(&stop,"seatId")?!=r[4]
                || user_payload_string(&stop,"generation")?!=r[2]
                || stop.expected_revision!=stop_previous
                || stop_previous.checked_add(1)!=Some(stop_revision)
                || child_previous!=stop_revision {
                return Err(denied("Claude stopped peer ancestor stop"));
            }
            // Freeze the complete validated parent chain into peer provenance,
            // not just the final set of derived package SID strings.
            chain.push(r.clone());
            child_generation=r[2].clone();
            child_previous=start.expected_revision;
            next=r[7].clone();
        }
        Ok(chain)
    }

    // All normal stopped generations, including a STOPPED parent and stopped
    // resume episodes. Only exact original H/custody/stop tuples supply SIDs.
    fn claude_child_peers(&self, instance:&str) -> Result<(Vec<String>,String,Vec<String>)> {
        let registered=self.read_registered_instance(instance)?
            .ok_or_else(||denied("Claude peer instance missing"))?;
        let stopped=rows(&self.connection,
            "SELECT e.domain_id,e.session_id,e.generation,e.seat_incarnation,e.seat_id,
                    e.request_id,e.raw_hex,COALESCE(e.old_generation,''),e.process_operation_id,
                    e.stop_request_id,e.stop_fact_id,c.ticket,c.custodian_nonce,c.pid,
                    c.creation_time_100ns,c.image_path,c.binary_digest_sha256,
                    s.raw_hex,s.status,COALESCE(o.raw_hex,''),COALESCE(o.status,''),
                    CAST(s.previous_revision AS TEXT),CAST(s.revision AS TEXT)
             FROM main.gogoke_v37_h_process_episode e
             JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id
               AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation
               AND c.state='STOPPED' AND c.stop_proof_hash=e.stop_fact_id
             JOIN main.gogoke_v37_h_generation g ON g.domain_id=e.domain_id AND g.session_id=e.session_id
               AND g.generation=e.generation AND g.request_id=e.request_id
               AND g.process_operation_id=e.process_operation_id
             JOIN main.gogoke_v37_h_operation s ON s.domain_id=e.domain_id
               AND s.session_id=e.session_id AND s.request_id=e.stop_request_id AND s.operation='stop'
             LEFT JOIN main.gogoke_v37_h_operation o ON o.domain_id=e.domain_id
               AND o.session_id=e.session_id AND o.request_id=e.request_id AND o.operation='open'
             WHERE e.instance_id=?1 AND e.phase='STOPPED' AND e.stop_fact_id IS NOT NULL
               AND e.stop_fact_id<>'' ORDER BY e.domain_id,e.session_id,e.generation",
            &[instance],23)?;
        let mut sids=Vec::new();
        let mut bytes=Vec::new();
        let mut append=|value:&str| {
            bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
            bytes.extend_from_slice(value.as_bytes());
        };
        for r in stopped {
            let raw=unhex(&r[6])?;
            let start=evidence(decode_request(&raw))?;
            if start.raw_bytes!=raw || start.family!="K-SESSION" || start.request_id!=r[5]
                || start.domain_id!=r[0] || start.target_id!=r[1]
                || r[16]!=registered.program_digest || r[18]!="APPLIED" {
                return Err(denied("Claude stopped child peer original tuple"));
            }
            if r[7].is_empty() {
                if start.operation!="open" || start.payload.len()!=4 || r[19]!=r[6] || r[20]!="APPLIED"
                    || user_payload_string(&start,"seatId")?!=r[4]
                    || user_payload_string(&start,"generation")?!=r[2] {
                    return Err(denied("Claude stopped peer open"));
                }
                user_payload_string(&start,"repositoryId")?;
                user_payload_string(&start,"worktreeId")?;
            } else {
                let old=r[7].parse::<u64>().map_err(|_|denied("Claude peer old generation"))?;
                let new=r[2].parse::<u64>().map_err(|_|denied("Claude peer new generation"))?;
                if old==0 || old.to_string()!=r[7] || new.to_string()!=r[2]
                    || old.checked_add(1)!=Some(new) || start.operation!="resume"
                    || start.payload.len()!=1 || user_payload_string(&start,"generation")?!=r[7]
                    || r[8]!=format!("h-resume-{}",&sha256_hex(&raw)[..40]) {
                    return Err(denied("Claude stopped peer resume"));
                }
            }
            let raw=unhex(&r[17])?;
            let stop=evidence(decode_request(&raw))?;
            let stop_previous=r[21].parse::<u64>()
                .map_err(|_|denied("Claude stopped peer previous revision"))?;
            let stop_revision=r[22].parse::<u64>()
                .map_err(|_|denied("Claude stopped peer revision"))?;
            if stop.raw_bytes!=raw || stop.family!="K-SESSION" || stop.operation!="stop"
                || stop.request_id!=r[9] || stop.domain_id!=r[0] || stop.target_id!=r[1]
                || stop.payload.len()!=2 || user_payload_string(&stop,"seatId")?!=r[4]
                || user_payload_string(&stop,"generation")?!=r[2]
                || stop.expected_revision!=stop_previous
                || stop_previous.checked_add(1)!=Some(stop_revision) {
                return Err(denied("Claude stopped peer stop"));
            }
            if !r[7].is_empty() {
                for ancestor in self.claude_stopped_peer_parent_chain(
                    instance,&registered.program_digest,&r)? {
                    for value in ancestor {append(&value);}
                }
            }
            let name=original_session_profile_name(&self.root.canonical_root().identity.opaque(),
                &r[0],&r[1],&r[3],&r[2]);
            let sid=evidence(evidence(AppContainerProfile::derive_for_revocation(&name))?.sid_identity())?;
            if sids.contains(&sid) {return Err(denied("Claude stopped peer SID duplicate"));}
            sids.push(sid);
            for value in r {append(&value);}
        }
        let mut allowed=Vec::new();
        for original in self.claude_originals(instance)? {
            let operation=fact(&original.facts,"operation")?;
            if original.claim_state!="RELEASED" || !self.completed_claude_holder_release(
                instance,&fact(&original.facts,"domain")?,&fact(&original.facts,"session")?,&operation)? {
                return Err(denied("Claude child has unfinished independent open holder"));
            }
            if sids.contains(&original.sid) {return Err(denied("Claude completed peer SID duplicate"));}
            sids.push(original.sid);
            allowed.push(operation.clone());
            let record=journal::read(&self.connection,&operation)?
                .ok_or_else(||denied("Claude completed peer receipt missing"))?;
            append(&record.snapshot_digest);
            append(&record.snapshot_hex);
            for (key,value) in original.facts {append(&key);append(&value);}
        }
        drop(append);
        sids.sort();
        Ok((sids,sha256_hex(&bytes),allowed))
    }

    // Select an existing original, never manufacture a claim for the child.
    fn claude_recovery_originals(&self,instance:&str,child_only:bool)->Result<Vec<Original>> {
        if child_only {self.claude_failed_resume_originals(instance)} else {self.claude_originals(instance)}
    }

    fn claude_resource_only_finish(
        &mut self, original:&Original, record:&journal::Record, known_sids:&[String],
        proof:&NativeProcessHoldersGone,pairs:&[(u32,u64)],allowed:&[String],
    )->Result<()> {
        let instance=fact(&original.facts,"instance")?;
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let result=(||->Result<()> {
            self.gone_scope_in_current_transaction(&instance,allowed,None,false)?;
            evidence(proof.validate(pairs))?;
            let current=self.claude_failed_resume_originals(&instance)?.into_iter()
                .find(|o|o.facts.get("operation")==Some(&record.operation))
                .ok_or_else(||denied("Claude child original missing at finish"))?;
            if !resource_only(&current) || current.facts!=original.facts
                || journal::read(&self.connection,&record.operation)?.as_ref()!=Some(record) {
                return Err(denied("Claude child original intent changed at finish"));
            }
            let roots=self.claude_validate_capture(&current,record,known_sids,false)?;
            let profile=evidence(AppContainerProfile::derive_for_revocation(&current.profile_name))?;
            for object in &record.capture.objects {
                evidence(ClaudeAclRetirement::readback_target(
                    &profile,&roots,object,known_sids,proof,pairs))?;
            }
            // This receipt has no H claim transition. Its equal revisions
            // identify the unchanged real STOPPED parent, not a child StopFact.
            let op=Statement::prepare(self.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,
                 session_id,status,previous_revision,revision)
                 VALUES(?1,?2,?3,'claude-failed-resume-resource-release',?4,'APPLIED',?5,?5)")?;
            for (i,value) in [fact(&current.facts,"domain")?,record.request_id.clone(),
                record.snapshot_hex.clone(),fact(&current.facts,"session")?].iter().enumerate() {
                op.bind_text(i as i32+1,value)?;
            }
            op.bind_i64(5,current.claim_revision)?;
            op.step_done()?;
            journal::applied_in_transaction(&self.connection,record)?;
            Ok(())
        })();
        self.finish_native_transaction(result)
    }

    pub(super) fn completed_claude_failed_resume_resource_release(
        &self,instance:&str,domain:&str,session:&str,operation:&str,
    )->Result<bool> {
        let Some(record)=journal::read(&self.connection,operation)? else {return Ok(false)};
        if record.phase!="APPLIED" || record.revision!=2 || record.instance!=instance
            || record.domain!=domain || record.session!=session
            || record.capture.facts.get("kind").map(String::as_str)!=Some(FAILED_RESUME_RESOURCE_ONLY) {
            return Ok(false);
        }
        let Some(current)=self.claude_failed_resume_originals(instance)?.into_iter()
            .find(|o|o.facts.get("operation").is_some_and(|v|v==operation)) else {return Ok(false)};
        let known=fact(&record.capture.facts,"knownSids")?;
        let known:Vec<String>=known.split(',').map(str::to_owned).collect();
        let roots=self.claude_validate_capture(&current,&record,&known,false)?;
        // A durable completion is not a current ACL observation. Migration
        // consumes the exact original pair and target-only readback again.
        let pairs=[current.pair];
        let gone=evidence(NativeProcessHoldersGone::observe(&pairs))?;
        let profile=evidence(AppContainerProfile::derive_for_revocation(&current.profile_name))?;
        for object in &record.capture.objects {
            evidence(ClaudeAclRetirement::readback_target(
                &profile,&roots,object,&known,&gone,&pairs))?;
        }
        let receipt=rows(&self.connection,
            "SELECT raw_hex,status,previous_revision,revision,session_id
             FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2
               AND operation='claude-failed-resume-resource-release'",
            &[domain,&record.request_id],5)?;
        Ok(receipt==vec![vec![record.snapshot_hex,"APPLIED".into(),
            current.claim_revision.to_string(),current.claim_revision.to_string(),session.into()]])
    }

    fn claude_recovery_completed(&self,instance:&str,original:&Original)->Result<bool> {
        let domain=fact(&original.facts,"domain")?;
        let session=fact(&original.facts,"session")?;
        let operation=fact(&original.facts,"operation")?;
        if resource_only(original) {
            self.completed_claude_failed_resume_resource_release(instance,&domain,&session,&operation)
        } else {self.completed_claude_holder_release(instance,&domain,&session,&operation)}
    }

    pub(super) fn recover_claude_failed_resume_for_first_copy(&mut self,instance:&str)->Result<()> {
        if !evidence(instance::is_first_same_pin_copy(&self.connection,"claude"))? {
            return Err(denied("Claude child retirement is first same-pin migration only"));
        }
        self.recover_claude_resources(instance,None,true)
    }
}
