//! Claude's original H holder can disappear without a ProcessCustodian
//! StopFact. This retires only its three exact writable ACL trees and then
//! releases its original H claim. It never resumes or replays model work.
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
    fn claude_stopped_peers(&self, instance: &str) -> Result<(Vec<String>, String)> {
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
        for r in stopped {
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
            user_payload_string(&request, "repositoryId")?;
            user_payload_string(&request, "worktreeId")?;
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
            let name = original_session_profile_name(
                &self.root.canonical_root().identity.opaque(), &r[0], &r[1], &r[3], &r[2],
            );
            let profile = evidence(AppContainerProfile::derive_for_revocation(&name))?;
            sids.push(evidence(profile.sid_identity())?);
            for value in r {
                provenance.extend_from_slice(&(value.len() as u64).to_be_bytes());
                provenance.extend_from_slice(value.as_bytes());
            }
        }
        Ok((sids, sha256_hex(&provenance)))
    }

    fn claude_originals(&self, instance: &str) -> Result<Vec<Original>> {
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
                || r[25] != registered.program_digest
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
        let homes = evidence(instance::resolve_provider_session_launch_homes(
            &self.connection,
            self.root,
            &profile,
            &fact(f, "instance")?,
            &fact(f, "home")?,
            &fact(f, "domain")?,
            &fact(f, "session")?,
            &fact(f, "generation")?,
            "claude",
        ))?;
        let seat = evidence(seat::get(
            &self.connection,
            &fact(f, "domain")?,
            &fact(f, "seat")?,
        ))?
        .ok_or_else(|| denied("Claude original seat absent"))?;
        let relationship = evidence(
            crate::store::session_transport::session_binding::current_relationship(
                &self.connection,
                &fact(f, "domain")?,
                &fact(f, "session")?,
            ),
        )?
        .ok_or_else(|| denied("Claude original current relationship absent"))?;
        if seat.state != seat::State::Busy
            || seat.instance_id != fact(f, "instance")?
            || seat.incarnation != fact(f, "incarnation")?
            || relationship.seat_id != seat.seat_id
            || relationship.seat_incarnation != seat.incarnation
            || relationship.instance_id != seat.instance_id
            || relationship.session_generation != fact(f, "generation")?
            || relationship.seat_authorization_generation != seat.generation
        {
            return Err(denied("Claude original E authority changed"));
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
        let (_, stopped_provenance) = self.claude_stopped_peers(&fact(&facts, "instance")?)?;
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
        let (_, stopped_provenance) = self.claude_stopped_peers(&fact(saved, "instance")?)?;
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
            return Ok(Vec::new());
        }
        if original.claim_state != fact(saved, "claimState")?
            || original.claim_revision
                != fact(saved, "claimRevision")?
                    .parse::<i64>()
                    .map_err(|_| denied("Claude captured revision"))?
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

    pub(super) fn completed_claude_holder_release(
        &self,
        instance: &str,
        domain: &str,
        session: &str,
        operation: &str,
    ) -> Result<bool> {
        let Some(record) = journal::read(&self.connection, operation)? else {
            return Ok(false);
        };
        if record.phase != "APPLIED"
            || record.instance != instance
            || record.domain != domain
            || record.session != session
            || record.revision != 2
            || record.capture.objects.is_empty()
        {
            return Ok(false);
        }
        let original = self.claude_originals(instance)?.into_iter().find(|item| {
            item.facts
                .get("operation")
                .is_some_and(|value| value == operation)
        });
        let Some(original) = original else {
            return Ok(false);
        };
        let known = fact(&record.capture.facts, "knownSids")?;
        let known: Vec<String> = known.split(',').map(str::to_owned).collect();
        if self
            .claude_validate_capture(&original, &record, &known, true)
            .is_err()
        {
            return Ok(false);
        }
        let receipt=rows(&self.connection,
            "SELECT raw_hex,status,previous_revision,revision,session_id FROM main.gogoke_v37_h_operation
              WHERE domain_id=?1 AND request_id=?2 AND operation='claude-holder-gone-release'",
            &[domain,&record.request_id],5)?;
        Ok(receipt
            == vec![vec![
                record.snapshot_hex,
                "APPLIED".into(),
                fact(&record.capture.facts, "claimRevision")?,
                original.claim_revision.to_string(),
                session.into(),
            ]])
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
        let originals = self.claude_originals(instance)?;
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
        let active: Vec<_> = originals
            .iter()
            .filter(|o| o.claim_state != "RELEASED")
            .collect();
        if active.is_empty() {
            return Ok(());
        }
        let allowed: Vec<String> = originals
            .iter()
            .map(|o| fact(&o.facts, "operation"))
            .collect::<Result<_>>()?;
        self.gone_scope(instance, &allowed, incoming, false)?;
        let pairs: Vec<_> = active.iter().map(|o| o.pair).collect();
        let all_gone = evidence(NativeProcessHoldersGone::observe(&pairs))?;
        let mut known_sids: Vec<String> = originals.iter().map(|o| o.sid.clone()).collect();
        known_sids.sort();
        known_sids.dedup();
        if known_sids.len() != originals.len() {
            return Err(denied("Claude original SID reused"));
        }
        for peer in self.claude_stopped_peers(instance)?.0 {
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
                            .claude_originals(instance)?
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
                    if self.completed_claude_holder_release(
                        instance,
                        &fact(&original.facts, "domain")?,
                        &fact(&original.facts, "session")?,
                        &operation,
                    )? {
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
                        .claude_originals(instance)?
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
