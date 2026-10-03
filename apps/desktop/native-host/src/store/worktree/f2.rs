//! F.2 reads the native worktree graph and fences lifecycle effects. The
//! original linked-worktree row remains the physical source of truth.

use super::*;
use crate::store::atomic::{Json, JsonString};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphMember {
    pub(crate) worktree_id: String,
    pub(crate) repository_id: String,
    pub(crate) domain_id: String,
    pub(crate) seat_id: String,
    pub(crate) instance_id: String,
    pub(crate) baseline_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeGraph {
    pub(crate) space_id: String,
    pub(crate) classification: String,
    pub(crate) state: String,
    pub(crate) revision: i64,
    pub(crate) merge_reason: Option<String>,
    pub(crate) merge_target_commit: Option<String>,
    pub(crate) members: Vec<GraphMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegisterReceipt {
    pub(crate) worktree_id: String,
    pub(crate) revision: i64,
    pub(crate) replayed: bool,
}

/// F.2 registers an already created physical tree; it never repeats `git
/// worktree add`. The old M1 create entry is intentionally still immediate.
pub(crate) fn register_created_worktree(db: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock, owner: &OwnerIssuer, raw_request: &[u8]) -> Result<RegisterReceipt> {
    let request = crate::store::session_transport::decode_request(raw_request)
        .map_err(|_| WorktreeError::Invalid("register request"))?;
    if request.family!="K-WORKTREE" || request.operation!="register" ||
        request.expected_revision!=1 || !request.payload.is_empty() ||
        !atom(&request.target_id) { return Err(WorktreeError::Invalid("register wire")); }
    let fingerprint=sha256_hex(raw_request);
    let old=Statement::prepare(db.as_ptr(),
        "SELECT request_hash,worktree_id,operation,phase FROM main.gogoke_v37_worktree_lifecycle_ops WHERE request_id=?1")?;
    old.bind_text(1,&request.request_id)?;
    if old.step_row()? {
        if old.column_text(0)?!=fingerprint || old.column_text(1)?!=request.target_id ||
            old.column_text(2)?!="REGISTER" { return Err(WorktreeError::Conflict); }
        if old.column_text(3)?!="APPLIED" { return Err(WorktreeError::Unknown); }
        return Ok(RegisterReceipt {worktree_id:request.target_id,revision:2,replayed:true});
    }
    let physical=resolve_id(db,root,&request.target_id)?;
    drop(physical);
    transaction(db, |db| {
        check_owner_in_current_transaction(db,owner)?;
        let row=Statement::prepare(db.as_ptr(),
            "SELECT domain_id FROM main.gogoke_v37_worktrees WHERE worktree_id=?1 AND state='REGISTERED'")?;
        row.bind_text(1,&request.target_id)?;
        if !row.step_row()? || row.column_text(0)?!=request.domain_id || row.step_row()? {
            return Err(WorktreeError::Denied);
        }
        let state=lifecycle(db,&request.target_id)?.ok_or(WorktreeError::Denied)?;
        if state.0!="CREATED" || state.1!=1 { return Err(WorktreeError::Denied); }
        let pending=Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_worktree_lifecycle_ops WHERE worktree_id=?1 AND phase IN ('INTENT','UNKNOWN') LIMIT 1")?;
        pending.bind_text(1,&request.target_id)?;
        if pending.step_row()? { return Err(WorktreeError::Unknown); }
        let update=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_worktree_lifecycle SET state='REGISTERED',revision=2 WHERE worktree_id=?1 AND state='CREATED' AND revision=1")?;
        update.bind_text(1,&request.target_id)?; update.step_done()?;
        let changed=Statement::prepare(db.as_ptr(),"SELECT changes()")?;
        if !changed.step_row()? || changed.column_text(0)?!="1" {
            return Err(WorktreeError::Unknown);
        }
        let insert=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_worktree_lifecycle_ops(request_id,request_hash,worktree_id,operation,phase) VALUES(?1,?2,?3,'REGISTER','APPLIED')")?;
        insert.bind_text(1,&request.request_id)?; insert.bind_text(2,&fingerprint)?;
        insert.bind_text(3,&request.target_id)?; insert.step_done()?;
        Ok(RegisterReceipt {worktree_id:request.target_id.clone(),revision:2,replayed:false})
    })
}

fn lifecycle(db: &VerifiedDatabaseConnection<'_>, worktree_id: &str)
    -> Result<Option<(String, i64, Option<String>, Option<String>)>> {
    let q = Statement::prepare(db.as_ptr(),
        "SELECT state,revision,COALESCE(merge_reason,''),COALESCE(merge_target_commit,'') FROM main.gogoke_v37_worktree_lifecycle WHERE worktree_id=?1")?;
    q.bind_text(1, worktree_id)?;
    if !q.step_row()? { return Ok(None); }
    let reason = q.column_text(2)?;
    let target = q.column_text(3)?;
    let result = (q.column_text(0)?, parse_i64(&q.column_text(1)?, "worktree lifecycle revision")?,
        if reason.is_empty() { None } else { Some(reason) },
        if target.is_empty() { None } else { Some(target) });
    if q.step_row()? { return Err(WorktreeError::SchemaDrift); }
    Ok(Some(result))
}

pub(super) fn launch_allowed(db: &VerifiedDatabaseConnection<'_>, worktree_id: &str) -> Result<()> {
    if matches!(lifecycle(db, worktree_id)?, Some((state, ..)) if state != "REGISTERED") {
        return Err(WorktreeError::Denied);
    }
    Ok(())
}

/// A graph read is a projection of the same native rows used for launch. It
/// never accepts caller supplied paths, branch names or repository edges.
pub(crate) fn graph_query(db: &VerifiedDatabaseConnection<'_>, worktree_id: &str)
    -> Result<Option<WorktreeGraph>> {
    if !atom(worktree_id) { return Err(WorktreeError::Invalid("worktree id")); }
    let row = Statement::prepare(db.as_ptr(),
        "SELECT repository_id,domain_id,seat_id,instance_id,baseline_commit FROM main.gogoke_v37_worktrees WHERE worktree_id=?1 AND state='REGISTERED'")?;
    row.bind_text(1, worktree_id)?;
    if !row.step_row()? { return Ok(None); }
    let original = GraphMember { worktree_id: worktree_id.to_owned(), repository_id: row.column_text(0)?,
        domain_id: row.column_text(1)?, seat_id: row.column_text(2)?, instance_id: row.column_text(3)?,
        baseline_commit: row.column_text(4)? };
    if row.step_row()? { return Err(WorktreeError::SchemaDrift); }
    let state = lifecycle(db, worktree_id)?.unwrap_or(("REGISTERED".into(), 1, None, None));
    if state.0 == "CLEANED" { return Ok(None); }
    let member = Statement::prepare(db.as_ptr(),
        "SELECT space_id FROM main.gogoke_v37_worktree_members WHERE worktree_id=?1")?;
    member.bind_text(1, worktree_id)?;
    if !member.step_row()? {
        return Ok(Some(WorktreeGraph { space_id: worktree_id.to_owned(),
            classification: "SINGLE".into(), state: state.0, revision: state.1,
            merge_reason: state.2, merge_target_commit: state.3, members: vec![original] }));
    }
    let space_id = member.column_text(0)?;
    if member.step_row()? { return Err(WorktreeError::SchemaDrift); }
    let space = Statement::prepare(db.as_ptr(),
        "SELECT classification,state FROM main.gogoke_v37_worktree_spaces WHERE space_id=?1")?;
    space.bind_text(1, &space_id)?;
    if !space.step_row()? { return Err(WorktreeError::SchemaDrift); }
    let classification = space.column_text(0)?;
    if space.column_text(1)? != "ACTIVE" || space.step_row()? { return Err(WorktreeError::Denied); }
    let members = Statement::prepare(db.as_ptr(),
        "SELECT w.worktree_id,w.repository_id,w.domain_id,w.seat_id,w.instance_id,w.baseline_commit FROM main.gogoke_v37_worktree_members m JOIN main.gogoke_v37_worktrees w ON w.worktree_id=m.worktree_id WHERE m.space_id=?1 ORDER BY w.worktree_id")?;
    members.bind_text(1, &space_id)?;
    let mut graph_members = Vec::new();
    while members.step_row()? {
        graph_members.push(GraphMember { worktree_id: members.column_text(0)?, repository_id: members.column_text(1)?,
            domain_id: members.column_text(2)?, seat_id: members.column_text(3)?, instance_id: members.column_text(4)?,
            baseline_commit: members.column_text(5)? });
    }
    if graph_members.is_empty() || !graph_members.iter().any(|item| item.worktree_id == worktree_id) {
        return Err(WorktreeError::SchemaDrift);
    }
    Ok(Some(WorktreeGraph { space_id, classification, state: state.0,
        revision: state.1, merge_reason: state.2, merge_target_commit: state.3,
        members: graph_members }))
}

/// Look up the repository pin through the native row, never through an IPC
/// path or a caller-selected program. This is shared by public merge/cleanup.
pub(crate) fn repository_for_worktree(db: &VerifiedDatabaseConnection<'_>,
    domain_id: &str, worktree_id: &str) -> Result<String> {
    if !atom(domain_id) || !atom(worktree_id) { return Err(WorktreeError::Denied); }
    let row = Statement::prepare(db.as_ptr(),
        "SELECT repository_id FROM main.gogoke_v37_worktrees WHERE worktree_id=?1 AND domain_id=?2 AND state='REGISTERED'")?;
    row.bind_text(1, worktree_id)?;
    row.bind_text(2, domain_id)?;
    if !row.step_row()? { return Err(WorktreeError::Denied); }
    let repository = row.column_text(0)?;
    if row.step_row()? { return Err(WorktreeError::SchemaDrift); }
    Ok(repository)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExactStopFact {
    pub(crate) process_operation_id: String,
    pub(crate) stop_fact_id: String,
}

fn original_session_worktree(db: &VerifiedDatabaseConnection<'_>, domain: &str,
    session: &str) -> Result<Option<String>> {
    let rows=Statement::prepare(db.as_ptr(),
        "SELECT raw_hex FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2 AND operation='open'")?;
    rows.bind_text(1,domain)?; rows.bind_text(2,session)?;
    let mut found: Option<String>=None;
    while rows.step_row()? {
        let raw=rows.column_text(0)?;
        if raw.len()%2!=0 || raw.len()>131_072 { return Err(WorktreeError::Denied); }
        let mut bytes=Vec::with_capacity(raw.len()/2);
        for chunk in raw.as_bytes().chunks_exact(2) {
            let hex=std::str::from_utf8(chunk).map_err(|_|WorktreeError::Denied)?;
            bytes.push(u8::from_str_radix(hex,16).map_err(|_|WorktreeError::Denied)?);
        }
        let request=crate::store::session_transport::decode_request(&bytes)
            .map_err(|_|WorktreeError::Denied)?;
        if request.family!="K-SESSION" || request.operation!="open" ||
            request.domain_id!=domain || request.target_id!=session {
            return Err(WorktreeError::Denied);
        }
        let target=match request.payload.get(&JsonString::from_str("worktreeId")) {
            Some(Json::String(value))=>value.to_well_formed_string()
                .ok_or(WorktreeError::Denied)?,
            _=>return Err(WorktreeError::Denied),
        };
        if !atom(&target) || found.as_ref().is_some_and(|old| old!=&target) {
            return Err(WorktreeError::Denied);
        }
        found=Some(target);
    }
    Ok(found)
}

/// An unstarted reservation is still an active reservation. A STOPPED claim
/// also occupies capacity until release. Every native model process whose
/// sealed write root intersects this directory needs its own StopFact.
fn physical_roots_overlap(target_path: &Path, target_identity: &RootIdentity,
    target_pointer: &RootIdentity, other_path: &Path, other_identity: &RootIdentity,
    other_pointer: &RootIdentity) -> bool {
    target_identity == other_identity || target_pointer == other_pointer ||
        target_path.starts_with(other_path) || other_path.starts_with(target_path)
}

fn sealed_roots_overlap(target: &ResolvedBinding, other: &ResolvedBinding) -> bool {
    physical_roots_overlap(&target.path, &target.identity, &target.pointer_identity,
        &other.path, &other.identity, &other.pointer_identity)
}

fn proven_cleaned_root(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    worktree_id: &str) -> Result<bool> {
    let row = Statement::prepare(db.as_ptr(),
        "SELECT w.worktree_path FROM main.gogoke_v37_worktrees w JOIN main.gogoke_v37_worktree_lifecycle l ON l.worktree_id=w.worktree_id WHERE w.worktree_id=?1 AND l.state='CLEANED' AND l.stop_fact_id IS NOT NULL AND EXISTS(SELECT 1 FROM main.gogoke_v37_worktree_lifecycle_ops o WHERE o.worktree_id=w.worktree_id AND o.operation='CLEANUP' AND o.phase='APPLIED')")?;
    row.bind_text(1,worktree_id)?;
    if !row.step_row()? { return Ok(false); }
    let path=PathBuf::from(row.column_text(0)?);
    if row.step_row()? || !path.starts_with(root.canonical_root().canonical_path.join("v37-worktrees")) {
        return Err(WorktreeError::Denied);
    }
    require_absent(&path)?;
    Ok(true)
}

pub(crate) fn cleanup_stop_gate(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    worktree_id: &str) -> Result<Vec<ExactStopFact>> {
    // resolve_id verifies the graph member's host-generated canonical path,
    // directory identity, Git pointer and common directory before comparing
    // H's original sealed write root with the directory to be removed.
    let target = resolve_id(db, root, worktree_id).map_err(|_| WorktreeError::Denied)?;
    cleanup_stop_gate_with(db, worktree_id, |other_id| {
        if other_id == worktree_id { return Ok(true); }
        // A previously completed native cleanup cannot still provide a
        // workspace write root. Its historical H rows need not pin every
        // unrelated future cleanup once physical absence is rechecked.
        if proven_cleaned_root(db, root, other_id)? { return Ok(false); }
        let other = resolve_id(db, root, other_id).map_err(|_| WorktreeError::Denied)?;
        Ok(sealed_roots_overlap(&target, &other))
    })
}

fn cleanup_stop_gate_with<F>(db: &VerifiedDatabaseConnection<'_>, worktree_id: &str,
    mut overlaps: F) -> Result<Vec<ExactStopFact>>
where F: FnMut(&str) -> Result<bool> {
    if !atom(worktree_id) { return Err(WorktreeError::Invalid("worktree id")); }
    let binding = Statement::prepare(db.as_ptr(),
        "SELECT domain_id,seat_id,seat_incarnation FROM main.gogoke_v37_worktrees WHERE worktree_id=?1 AND state='REGISTERED'")?;
    binding.bind_text(1, worktree_id)?;
    if !binding.step_row()? { return Err(WorktreeError::Denied); }
    let domain = binding.column_text(0)?;
    let seat = binding.column_text(1)?;
    let incarnation = binding.column_text(2)?;
    if binding.step_row()? { return Err(WorktreeError::SchemaDrift); }
    let reservations = Statement::prepare(db.as_ptr(),
        "SELECT a.domain_id,a.session_id,COALESCE(s.seat_id,''),COALESCE(s.seat_incarnation,'') FROM main.gogoke_v37_h_claim a LEFT JOIN main.gogoke_v37_h_seat_binding s ON s.domain_id=a.domain_id AND s.session_id=a.session_id WHERE a.state!='RELEASED'")?;
    while reservations.step_row()? {
        let claim_domain=reservations.column_text(0)?;
        let session=reservations.column_text(1)?;
        let claim_seat=reservations.column_text(2)?;
        let claim_incarnation=reservations.column_text(3)?;
        // No open request means an unresolved reservation could still target
        // this tree only when bound to its seat. A model session on another
        // seat still needs a physical comparison once it has an open target.
        match original_session_worktree(db,&claim_domain,&session)? {
            Some(other) if !overlaps(&other)?=>continue,
            Some(_)=>return Err(WorktreeError::Denied),
            None if claim_domain==domain && claim_seat==seat &&
                claim_incarnation==incarnation=>return Err(WorktreeError::Denied),
            None=>continue,
        }
    }
    let episodes = Statement::prepare(db.as_ptr(),
        "SELECT e.domain_id,e.session_id,COALESCE(e.process_operation_id,''),e.phase,COALESCE(e.stop_fact_id,''),COALESCE(c.state,''),COALESCE(c.stop_proof_hash,'') FROM main.gogoke_v37_h_process_episode e LEFT JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id AND c.generation=e.generation ORDER BY e.process_operation_id")?;
    let mut proofs = Vec::new();
    while episodes.step_row()? {
        let episode_domain=episodes.column_text(0)?;
        let session=episodes.column_text(1)?;
        let process=episodes.column_text(2)?;
        let phase=episodes.column_text(3)?;
        // H marks FAILED only for a cancelled resume INTENT that never
        // acquired a process operation. It cannot hold a workspace handle.
        if phase=="FAILED" && process.is_empty() { continue; }
        match original_session_worktree(db,&episode_domain,&session)? {
            Some(target) if !overlaps(&target)?=>continue,
            Some(_)=>{},
            // A historical H model process without its sealed root cannot
            // prove non-overlap. Fixed account/read/login records are outside
            // this H process-episode table.
            None=>return Err(WorktreeError::Denied),
        }
        let fact = episodes.column_text(4)?;
        let custody = episodes.column_text(5)?;
        let proof = episodes.column_text(6)?;
        if process.is_empty() || phase != "STOPPED" || fact.is_empty()
            || custody != "STOPPED" || proof != fact { return Err(WorktreeError::Denied); }
        proofs.push(ExactStopFact { process_operation_id: process, stop_fact_id: fact });
    }
    if proofs.is_empty() { return Err(WorktreeError::Denied); }
    Ok(proofs)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanupReceipt {
    pub(crate) worktree_id: String,
    pub(crate) revision: i64,
    pub(crate) stop_fact_id: String,
    pub(crate) replayed: bool,
}

/// K-WORKTREE cleanup accepts only the exact User request. The native claim,
/// episode and custody rows are rechecked in the same transaction that writes
/// INTENT; no caller-provided stop assertion can authorize directory removal.
pub(crate) fn cleanup_worktree(db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
    owner: &OwnerIssuer, pin: &GitProgramPin, custodian: &mut ProcessCustodian,
    raw_request: &[u8]) -> Result<CleanupReceipt> {
    let request = crate::store::session_transport::decode_request(raw_request)
        .map_err(|_| WorktreeError::Invalid("cleanup request"))?;
    if request.family != "K-WORKTREE" || request.operation != "cleanup" ||
        !request.payload.is_empty() || !atom(&request.target_id) ||
        request.expected_revision == 0 || request.expected_revision >= i64::MAX as u64 {
        return Err(WorktreeError::Invalid("cleanup wire"));
    }
    let fingerprint = sha256_hex(raw_request);
    let existing = Statement::prepare(db.as_ptr(),
        "SELECT request_hash,worktree_id,operation,phase FROM main.gogoke_v37_worktree_lifecycle_ops WHERE request_id=?1")?;
    existing.bind_text(1, &request.request_id)?;
    if existing.step_row()? {
        if existing.column_text(0)? != fingerprint || existing.column_text(1)? != request.target_id ||
            existing.column_text(2)? != "CLEANUP" { return Err(WorktreeError::Conflict); }
        if existing.column_text(3)? != "APPLIED" { return Err(WorktreeError::Unknown); }
        let done = lifecycle(db, &request.target_id)?.ok_or(WorktreeError::Unknown)?;
        if done.0 != "CLEANED" { return Err(WorktreeError::Unknown); }
        let fact = Statement::prepare(db.as_ptr(),
            "SELECT stop_fact_id FROM main.gogoke_v37_worktree_lifecycle WHERE worktree_id=?1")?;
        fact.bind_text(1, &request.target_id)?;
        if !fact.step_row()? { return Err(WorktreeError::Unknown); }
        return Ok(CleanupReceipt { worktree_id: request.target_id, revision: done.1,
            stop_fact_id: fact.column_text(0)?, replayed: true });
    }
    let (source, path, fact) = transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        let row = Statement::prepare(db.as_ptr(),
            "SELECT w.repository_id,w.domain_id,w.worktree_path,s.source_path,s.git_digest,s.git_version FROM main.gogoke_v37_worktrees w JOIN main.gogoke_v37_worktree_sources s ON s.repository_id=w.repository_id WHERE w.worktree_id=?1 AND w.state='REGISTERED'")?;
        row.bind_text(1, &request.target_id)?;
        if !row.step_row()? { return Err(WorktreeError::Denied); }
        let _repository = row.column_text(0)?;
        if row.column_text(1)? != request.domain_id || row.column_text(4)? != pin.digest ||
            row.column_text(5)? != pin.version { return Err(WorktreeError::Denied); }
        let path = PathBuf::from(row.column_text(2)?);
        let source = PathBuf::from(row.column_text(3)?);
        if row.step_row()? { return Err(WorktreeError::SchemaDrift); }
        let prior = lifecycle(db, &request.target_id)?.unwrap_or(("REGISTERED".into(), 1, None, None));
        if !matches!(prior.0.as_str(), "REGISTERED" | "MERGED") ||
            prior.1 != request.expected_revision as i64 { return Err(WorktreeError::Denied); }
        let facts = cleanup_stop_gate(db, root, &request.target_id)?;
        let fact = facts.last().ok_or(WorktreeError::Denied)?.stop_fact_id.clone();
        let check = Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_worktree_lifecycle_ops WHERE worktree_id=?1 AND phase IN ('INTENT','UNKNOWN') LIMIT 1")?;
        check.bind_text(1, &request.target_id)?;
        if check.step_row()? { return Err(WorktreeError::Unknown); }
        let insert = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_worktree_lifecycle_ops(request_id,request_hash,worktree_id,operation,phase) VALUES(?1,?2,?3,'CLEANUP','INTENT')")?;
        insert.bind_text(1, &request.request_id)?;
        insert.bind_text(2, &fingerprint)?;
        insert.bind_text(3, &request.target_id)?;
        insert.step_done()?;
        let update = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_worktree_lifecycle(worktree_id,state,revision,stop_fact_id) VALUES(?1,'CLEANUP_INTENT',?2,?3) ON CONFLICT(worktree_id) DO UPDATE SET state='CLEANUP_INTENT',stop_fact_id=excluded.stop_fact_id WHERE state IN ('REGISTERED','MERGED') AND revision=?4")?;
        update.bind_text(1, &request.target_id)?;
        update.bind_i64(2, prior.1)?;
        update.bind_text(3, &fact)?;
        update.bind_i64(4, prior.1)?;
        update.step_done()?;
        let changed = Statement::prepare(db.as_ptr(), "SELECT changes()")?;
        if !changed.step_row()? || changed.column_text(0)? != "1" { return Err(WorktreeError::Unknown); }
        Ok((source, path, fact))
    })?;
    let effect = (|| -> Result<()> {
        // The pointer guard is dropped before Git removes the exact tree; no
        // seat process is permitted after the checked stop/release boundary.
        let binding = resolve_id(db, root, &request.target_id)?;
        if binding.path != path { return Err(WorktreeError::Denied); }
        drop(binding);
        git(db, root, custodian, pin, "worktree_remove", Some(&source), &[
            "worktree".into(), "remove".into(), "--quiet".into(),
            git_launch_path(&path)?.to_string_lossy().into_owned(),
        ], false)?;
        require_absent(&path)?;
        Ok(())
    })();
    match effect {
        Ok(()) => transaction(db, |db| {
            check_owner_in_current_transaction(db, owner)?;
            let current = lifecycle(db, &request.target_id)?.ok_or(WorktreeError::Unknown)?;
            if current.0 != "CLEANUP_INTENT" || current.1 != request.expected_revision as i64 {
                return Err(WorktreeError::Unknown);
            }
            let next = current.1 + 1;
            let update = Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_worktree_lifecycle SET state='CLEANED',revision=?1 WHERE worktree_id=?2 AND state='CLEANUP_INTENT' AND revision=?3")?;
            update.bind_i64(1, next)?;
            update.bind_text(2, &request.target_id)?;
            update.bind_i64(3, current.1)?;
            update.step_done()?;
            let update = Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_worktree_lifecycle_ops SET phase='APPLIED' WHERE request_id=?1 AND phase='INTENT'")?;
            update.bind_text(1, &request.request_id)?;
            update.step_done()?;
            Ok(CleanupReceipt { worktree_id: request.target_id.clone(), revision: next,
                stop_fact_id: fact.clone(), replayed: false })
        }).map_err(|error| { RootLock::poison_identity(&root.canonical_root().identity); error }),
        Err(error) => {
            let cause = format!("{error:?}");
            let recorded = transaction(db, |db| {
                check_owner_in_current_transaction(db, owner)?;
                let update = Statement::prepare(db.as_ptr(),
                    "UPDATE main.gogoke_v37_worktree_lifecycle SET state='CLEANUP_UNKNOWN' WHERE worktree_id=?1 AND state='CLEANUP_INTENT'")?;
                update.bind_text(1, &request.target_id)?; update.step_done()?;
                let update = Statement::prepare(db.as_ptr(),
                    "UPDATE main.gogoke_v37_worktree_lifecycle_ops SET phase='UNKNOWN',cause=?1 WHERE request_id=?2 AND phase='INTENT'")?;
                update.bind_text(1, &cause)?; update.bind_text(2, &request.request_id)?; update.step_done()?;
                Ok(())
            });
            match recorded {
                Ok(()) => Err(error),
                Err(persist) => { RootLock::poison_identity(&root.canonical_root().identity);
                    Err(joined("cleanup UNKNOWN persistence", error, persist)) }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MergeReceipt {
    pub(crate) worktree_id: String,
    pub(crate) revision: i64,
    pub(crate) target_commit: String,
    pub(crate) replayed: bool,
}

// APPLIED has no failure cause. Reuse its existing cause field for the exact
// immutable operation receipt; UNKNOWN continues to retain the original error.
// This follows instance repin's canonical receipt comparison without a schema
// migration that could discard an older operation's evidence.
fn merge_receipt_record(request: &crate::store::session_transport::V37Request,
    revision: i64, commit: &str) -> String {
    let raw_hex = request.raw_bytes.iter().map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Json::Object([
        ("schema", "gogoke.37.worktree-merge-result.v1".to_owned()),
        ("requestId", request.request_id.clone()), ("rawHex", raw_hex),
        ("requestHash", sha256_hex(&request.raw_bytes)),
        ("family", request.family.clone()), ("operation", request.operation.clone()),
        ("domainId", request.domain_id.clone()), ("worktreeId", request.target_id.clone()),
        ("revision", revision.to_string()), ("targetCommit", commit.to_owned()),
    ].into_iter().map(|(key, value)| (JsonString::from_str(key),
        Json::String(JsonString::from_str(&value)))).collect()).canonical()
}

/// `authorize` must read the current E.2 permission source and authenticate the
/// current caller from trusted ingress, returning its observed turn ID. The
/// stored source seat argument is provenance, not a caller equality condition.
/// Historical receipt reads require the same current domain/MAIN/Merge grant.
/// The native F module never accepts a Boolean permission from wire payload.
pub(crate) fn merge_worktree(
    db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
    pin: &GitProgramPin, custodian: &mut ProcessCustodian, raw_request: &[u8],
    authorize: impl FnOnce(&VerifiedDatabaseConnection<'_>, &str, &str, &str)
        -> Result<Option<String>>,
) -> Result<MergeReceipt> {
    let request = crate::store::session_transport::decode_request(raw_request)
        .map_err(|_| WorktreeError::Invalid("merge request"))?;
    if request.family != "K-WORKTREE" || request.operation != "merge" ||
        request.expected_revision == 0 || request.expected_revision >= i64::MAX as u64 ||
        !atom(&request.target_id) || request.payload.len() != 2 {
        return Err(WorktreeError::Invalid("merge wire"));
    }
    let decision = request.payload.get(&JsonString::from_str("decision"));
    if !matches!(decision, Some(Json::String(value)) if value.to_well_formed_string().as_deref()==Some("MERGE")) {
        return Err(WorktreeError::Invalid("merge decision"));
    }
    let reason = match request.payload.get(&JsonString::from_str("reason")) {
        Some(Json::String(value)) => value.to_well_formed_string()
            .ok_or(WorktreeError::Invalid("merge reason"))?,
        _ => return Err(WorktreeError::Invalid("merge reason")),
    };
    if reason.trim().is_empty() || reason.len() > 4096 || reason.chars().any(char::is_control) {
        return Err(WorktreeError::Invalid("merge reason"));
    }
    let fingerprint = sha256_hex(raw_request);
    let existing = Statement::prepare(db.as_ptr(),
        "SELECT request_hash,worktree_id,operation,phase,COALESCE(result_commit,''),cause FROM main.gogoke_v37_worktree_lifecycle_ops WHERE request_id=?1")?;
    existing.bind_text(1, &request.request_id)?;
    if existing.step_row()? {
        return transaction(db, |db| {
            // Read only native ownership metadata. A CLEANED physical tree no
            // longer exists, so replay must not resolve it or invoke any Git.
            let source = Statement::prepare(db.as_ptr(),
                "SELECT domain_id,seat_id FROM main.gogoke_v37_worktrees WHERE worktree_id=?1")?;
            source.bind_text(1, &request.target_id)?;
            if !source.step_row()? || source.column_text(0)? != request.domain_id {
                return Err(WorktreeError::Denied);
            }
            let seat_id = source.column_text(1)?;
            if source.step_row()? { return Err(WorktreeError::SchemaDrift); }
            let turn = authorize(db, &request.domain_id, &seat_id, &request.target_id)?
                .ok_or(WorktreeError::Denied)?;
            if !atom(&turn) { return Err(WorktreeError::Denied); }
            if existing.column_text(0)? != fingerprint || existing.column_text(1)? != request.target_id ||
                existing.column_text(2)? != "MERGE" { return Err(WorktreeError::Conflict); }
            if existing.column_text(3)? != "APPLIED" { return Err(WorktreeError::Unknown); }
            let result = existing.column_text(4)?;
            if !hex_commit(&result) { return Err(WorktreeError::Unknown); }
            // Older APPLIED records hold only result_commit and the exact
            // request hash. That request's expected revision uniquely fixes
            // the historical result revision under the existing +1 transition.
            let revision = request.expected_revision as i64 + 1;
            let record = existing.column_text(5)?;
            if !record.is_empty() && record != merge_receipt_record(&request, revision, &result) {
                return Err(WorktreeError::Unknown);
            }
            Ok(MergeReceipt { worktree_id: request.target_id.clone(), revision,
                target_commit: result, replayed: true })
        });
    }
    let binding = resolve_id(db, root, &request.target_id)?;
    let source = Statement::prepare(db.as_ptr(),
        "SELECT s.source_path,s.git_digest,s.git_version,w.domain_id,w.seat_id,w.instance_id,s.common_path FROM main.gogoke_v37_worktrees w JOIN main.gogoke_v37_worktree_sources s ON s.repository_id=w.repository_id WHERE w.worktree_id=?1")?;
    source.bind_text(1, &request.target_id)?;
    if !source.step_row()? { return Err(WorktreeError::Denied); }
    let source_path = PathBuf::from(source.column_text(0)?);
    if source.column_text(1)? != pin.digest || source.column_text(2)? != pin.version ||
        source.column_text(3)? != request.domain_id { return Err(WorktreeError::Denied); }
    let seat_id = source.column_text(4)?;
    let instance_id = source.column_text(5)?;
    let common = PathBuf::from(source.column_text(6)?);
    if source.step_row()? { return Err(WorktreeError::SchemaDrift); }
    // Both trees must be clean before an effect is even reserved. Git's
    // porcelain output may have multiple lines; nonempty means no merge.
    for cwd in [&source_path, &binding.path] {
        let dirty = git(db, root, custodian, pin, "f2_status", Some(cwd), &[
            "status".into(), "--porcelain=v1".into(), "--untracked-files=all".into(),
        ], true)?;
        if !dirty.is_empty() { return Err(WorktreeError::Denied); }
    }
    let before = git(db, root, custodian, pin, "merge_before", Some(&source_path), &[
        "rev-parse".into(), "--verify".into(), "HEAD^{commit}".into(),
    ], true)?;
    let incoming = git(db, root, custodian, pin, "merge_incoming", Some(&binding.path), &[
        "rev-parse".into(), "--verify".into(), "HEAD^{commit}".into(),
    ], true)?;
    if !hex_commit(&before) || !hex_commit(&incoming) || before == incoming {
        return Err(WorktreeError::Denied);
    }
    // The seat's new commit may contain attributes not present at source
    // registration. Re-run the existing no-external-driver gate on the exact
    // incoming tree before Git merge can evaluate its contents.
    source_checkout_has_no_external_drivers(db, root, custodian, pin,
        &binding.path, &common, &incoming)?;
    git(db, root, custodian, pin, "merge_baseline", Some(&binding.path), &[
        "merge-base".into(), "--is-ancestor".into(), binding.baseline_commit.clone(), incoming.clone(),
    ], false)?;
    let turn_id = transaction(db, |db| {
        let state = lifecycle(db, &request.target_id)?.unwrap_or(("REGISTERED".into(), 1, None, None));
        if state.0 != "REGISTERED" || state.1 != request.expected_revision as i64 {
            return Err(WorktreeError::Denied);
        }
        cleanup_stop_gate(db, root, &request.target_id)?;
        let turn = authorize(db, &request.domain_id, &seat_id, &request.target_id)?
            .ok_or(WorktreeError::Denied)?;
        if !atom(&turn) { return Err(WorktreeError::Denied); }
        let pending = Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_worktree_lifecycle_ops WHERE worktree_id=?1 AND phase IN ('INTENT','UNKNOWN') LIMIT 1")?;
        pending.bind_text(1, &request.target_id)?;
        if pending.step_row()? { return Err(WorktreeError::Unknown); }
        let insert = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_worktree_lifecycle_ops(request_id,request_hash,worktree_id,operation,phase) VALUES(?1,?2,?3,'MERGE','INTENT')")?;
        insert.bind_text(1, &request.request_id)?; insert.bind_text(2, &fingerprint)?;
        insert.bind_text(3, &request.target_id)?; insert.step_done()?;
        let update = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_worktree_lifecycle(worktree_id,state,revision,merge_reason) VALUES(?1,'MERGE_INTENT',?2,?3) ON CONFLICT(worktree_id) DO UPDATE SET state='MERGE_INTENT',merge_reason=excluded.merge_reason WHERE state='REGISTERED' AND revision=?4")?;
        update.bind_text(1, &request.target_id)?; update.bind_i64(2, state.1)?;
        update.bind_text(3, &reason)?; update.bind_i64(4, state.1)?; update.step_done()?;
        let changed = Statement::prepare(db.as_ptr(), "SELECT changes()")?;
        if !changed.step_row()? || changed.column_text(0)? != "1" { return Err(WorktreeError::Unknown); }
        Ok(turn)
    })?;
    // No remote, push, fetch, credential helper or caller supplied path is
    // invoked. A conflict or uncertain commit remains MERGE_UNKNOWN.
    let effect = (|| -> Result<String> {
        if git(db, root, custodian, pin, "merge_recheck", Some(&source_path), &[
            "rev-parse".into(), "--verify".into(), "HEAD^{commit}".into(),
        ], true)? != before { return Err(WorktreeError::Denied); }
        git(db, root, custodian, pin, "merge_apply", Some(&source_path), &[
            "merge".into(), "--no-ff".into(), "--no-commit".into(), "--quiet".into(), incoming.clone(),
        ], false)?;
        let message = format!("Merge worktree {}\n\n{}\n\nGogoke-Project: {}\nGogoke-Seat: {}\nGogoke-Instance: {}\nGogoke-Turn: {}",
            request.target_id, reason, request.domain_id, seat_id, instance_id, turn_id);
        git(db, root, custodian, pin, "merge_commit", Some(&source_path), &[
            "-c".into(), "commit.gpgsign=false".into(), "commit".into(), "--quiet".into(), "-m".into(), message,
        ], false)?;
        let result = git(db, root, custodian, pin, "merge_result", Some(&source_path), &[
            "rev-list".into(), "--parents".into(), "-n".into(), "1".into(), "HEAD".into(),
        ], true)?;
        let parts: Vec<_> = result.split_whitespace().collect();
        if parts.len() != 3 || parts[1] != before || parts[2] != incoming || !hex_commit(parts[0]) {
            return Err(WorktreeError::Unknown);
        }
        Ok(parts[0].to_owned())
    })();
    match effect {
        Ok(commit) => transaction(db, |db| {
            let state = lifecycle(db, &request.target_id)?.ok_or(WorktreeError::Unknown)?;
            if state.0 != "MERGE_INTENT" || state.1 != request.expected_revision as i64 {
                return Err(WorktreeError::Unknown);
            }
            let next = state.1 + 1;
            let update = Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_worktree_lifecycle SET state='MERGED',revision=?1,merge_target_commit=?2 WHERE worktree_id=?3 AND state='MERGE_INTENT' AND revision=?4")?;
            update.bind_i64(1, next)?; update.bind_text(2, &commit)?;
            update.bind_text(3, &request.target_id)?; update.bind_i64(4, state.1)?; update.step_done()?;
            let update = Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_worktree_lifecycle_ops SET phase='APPLIED',result_commit=?1,cause=?3 WHERE request_id=?2 AND phase='INTENT'")?;
            update.bind_text(1, &commit)?; update.bind_text(2, &request.request_id)?;
            update.bind_text(3, &merge_receipt_record(&request, next, &commit))?;
            update.step_done()?;
            Ok(MergeReceipt { worktree_id: request.target_id.clone(), revision: next,
                target_commit: commit.clone(), replayed: false })
        }).map_err(|error| { RootLock::poison_identity(&root.canonical_root().identity); error }),
        Err(error) => {
            let cause = format!("{error:?}");
            let recorded = transaction(db, |db| {
                let update = Statement::prepare(db.as_ptr(),
                    "UPDATE main.gogoke_v37_worktree_lifecycle SET state='MERGE_UNKNOWN' WHERE worktree_id=?1 AND state='MERGE_INTENT'")?;
                update.bind_text(1, &request.target_id)?; update.step_done()?;
                let update = Statement::prepare(db.as_ptr(),
                    "UPDATE main.gogoke_v37_worktree_lifecycle_ops SET phase='UNKNOWN',cause=?1 WHERE request_id=?2 AND phase='INTENT'")?;
                update.bind_text(1, &cause)?; update.bind_text(2, &request.request_id)?; update.step_done()?;
                Ok(())
            });
            match recorded {
                Ok(()) => Err(error),
                Err(persist) => { RootLock::poison_identity(&root.canonical_root().identity);
                    Err(joined("merge UNKNOWN persistence", error, persist)) }
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::store::{authority, instance, seat, session_transport};
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn merge_replay_after_cleanup_requires_current_authority_and_no_git() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-f2-merge-replay-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut db = create_new(&root, &path.join("state.sqlite")).unwrap();
        authority::initialize_profile(&mut db, &root).unwrap();
        authority::initialize_process_custody_schema(&mut db).unwrap();
        initialize_schema(&mut db).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_worktree_sources VALUES('repoA','absent-source','sourceIdentity','absent-common','commonIdentity','HTTPS','baseline','gitDigest','gitVersion',1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_worktrees(worktree_id,path_id,repository_id,domain_id,seat_id,seat_incarnation,seat_generation,seat_revision,permission_tier,instance_id,source_revision,worktree_path,worktree_identity,git_pointer_hash,git_pointer_len,git_pointer_identity,common_identity,baseline_commit,state,revision) VALUES('treeA','wtA','repoA','projectA','writerSeat','incarnationA',1,1,'NetworkedWrite','instanceA',1,'absent-cleaned-tree','pathIdentity','pointerHash',10,'pointerIdentity','commonIdentity','baseline','REGISTERED',1)").unwrap();
        // Cleanup has legitimately advanced the lifecycle past the merge's
        // original revision. Neither source nor linked physical tree exists.
        db.execute("INSERT INTO main.gogoke_v37_worktree_lifecycle VALUES('treeA','CLEANED',4,'accepted merge',NULL,'stopA')").unwrap();
        let raw = br#"{"schema":"gogoke.37.operations.v1","family":"K-WORKTREE","operation":"merge","requestId":"mergeA","targetId":"treeA","domainId":"projectA","expectedRevision":"2","payload":{"decision":"MERGE","reason":"accepted merge"}}"#;
        let request = session_transport::decode_request(raw).unwrap();
        let commit = "a".repeat(40);
        let record = merge_receipt_record(&request, 3, &commit);
        let insert = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_worktree_lifecycle_ops(request_id,request_hash,worktree_id,operation,phase,cause,result_commit) VALUES('mergeA',?1,'treeA','MERGE','APPLIED',?2,?3)").unwrap();
        insert.bind_text(1, &sha256_hex(raw)).unwrap();
        insert.bind_text(2, &record).unwrap(); insert.bind_text(3, &commit).unwrap();
        insert.step_done().unwrap(); drop(insert);
        // An inert file pin cannot launch Git; a replay accidentally entering
        // physical/Git code fails this test before it can manufacture evidence.
        let pin = GitProgramPin { path: path.join("missing-git.exe"), digest: "gitDigest".into(),
            version: "gitVersion".into(), _file: File::open(path.join("state.sqlite")).unwrap(),
            profile_id: "unused".into(), owner_seat_id: "unused".into(), policy_revision: "unused".into() };
        let mut custodian = ProcessCustodian::new().unwrap();
        let authorize = |_: &VerifiedDatabaseConnection<'_>, domain: &str, source_seat: &str, target: &str| {
            assert_eq!((domain, source_seat, target), ("projectA", "writerSeat", "treeA"));
            // Shared ingress authenticates its current merger independently
            // of the historical writer seat; this is the observed current turn.
            Ok(Some("currentMergerTurn".into()))
        };
        let expected = MergeReceipt { worktree_id: "treeA".into(), revision: 3,
            target_commit: commit.clone(), replayed: true };
        assert_eq!(merge_worktree(&mut db, &root, &pin, &mut custodian, raw, authorize).unwrap(), expected);
        assert!(matches!(merge_worktree(&mut db, &root, &pin, &mut custodian, raw,
            |_, _, _, _| Ok(None)), Err(WorktreeError::Denied)), "revoked current grant cannot read old success");
        assert!(matches!(merge_worktree(&mut db, &root, &pin, &mut custodian, raw,
            |_, _, _, _| Ok(Some(String::new()))), Err(WorktreeError::Denied)));
        let other_domain = std::str::from_utf8(raw).unwrap().replace("projectA", "projectB");
        assert!(matches!(merge_worktree(&mut db, &root, &pin, &mut custodian, other_domain.as_bytes(),
            |_, _, _, _| panic!("different domain must be denied before reading receipt")), Err(WorktreeError::Denied)));
        let different_bytes = [raw.as_slice(), b" "].concat();
        assert!(matches!(merge_worktree(&mut db, &root, &pin, &mut custodian, &different_bytes,
            authorize), Err(WorktreeError::Conflict)), "same parsed request with changed raw bytes is not replay");
        db.execute("UPDATE main.gogoke_v37_worktree_lifecycle_ops SET cause='{}'").unwrap();
        assert!(matches!(merge_worktree(&mut db, &root, &pin, &mut custodian, raw,
            authorize), Err(WorktreeError::Unknown)), "incomplete receipt is never inferred from lifecycle");
        // An exact older-schema APPLIED row keeps its original commit and uses
        // its hash-bound request's +1 revision, without rewriting evidence.
        db.execute("UPDATE main.gogoke_v37_worktree_lifecycle_ops SET cause=''").unwrap();
        assert_eq!(merge_worktree(&mut db, &root, &pin, &mut custodian, raw, authorize).unwrap(), expected);
        db.execute("UPDATE main.gogoke_v37_worktree_lifecycle_ops SET phase='UNKNOWN',cause='original uncertain Git error'").unwrap();
        assert!(matches!(merge_worktree(&mut db, &root, &pin, &mut custodian, raw,
            authorize), Err(WorktreeError::Unknown)), "UNKNOWN must never resend");
        let unchanged = Statement::prepare(db.as_ptr(),
            "SELECT phase,cause FROM main.gogoke_v37_worktree_lifecycle_ops WHERE request_id='mergeA'").unwrap();
        assert!(unchanged.step_row().unwrap());
        assert_eq!((unchanged.column_text(0).unwrap(), unchanged.column_text(1).unwrap()),
            ("UNKNOWN".into(), "original uncertain Git error".into()));
        drop(unchanged);
        let count = Statement::prepare(db.as_ptr(),
            "SELECT count(*) FROM main.gogoke_coordination_process_custody").unwrap();
        assert!(count.step_row().unwrap()); assert_eq!(count.column_text(0).unwrap(), "0");
        drop(count); drop(pin); drop(custodian);
        db.close_checked().unwrap(); drop(root); fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn physical_overlap_includes_mixed_parent_and_child_but_not_sibling() {
        let identity = |id| RootIdentity { volume_serial: 1, file_id: [id; 16] };
        let base = std::env::temp_dir().join("gogoke-f2-overlap-fixture");
        let mixed = base.join("mixed/space");
        let single = mixed.join("member");
        let sibling = base.join("mixed/other/member");
        assert!(physical_roots_overlap(&mixed, &identity(1), &identity(2),
            &single, &identity(3), &identity(4)));
        assert!(physical_roots_overlap(&single, &identity(3), &identity(4),
            &mixed, &identity(1), &identity(2)));
        assert!(!physical_roots_overlap(&mixed, &identity(1), &identity(2),
            &sibling, &identity(5), &identity(6)));
        assert!(physical_roots_overlap(&mixed, &identity(1), &identity(2),
            &sibling, &identity(1), &identity(6)), "path aliases share physical identity");
    }

    fn fixture_gate(db: &VerifiedDatabaseConnection<'_>, other_overlaps: bool)
        -> Result<Vec<ExactStopFact>> {
        cleanup_stop_gate_with(db,"treeA",|id| match id {
            "treeA" => Ok(true),
            "treeB" => Ok(other_overlaps),
            _ => Err(WorktreeError::Denied),
        })
    }

    #[test]
    fn cleanup_needs_released_reservation_and_the_same_native_stop_fact() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-f2-stop-gate-{}-{nonce}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut db = create_new(&root,&path.join("state.sqlite")).unwrap();
        authority::initialize_profile(&mut db,&root).unwrap();
        authority::initialize_process_custody_schema(&mut db).unwrap();
        instance::initialize_schema(&mut db).unwrap();
        seat::initialize_schema(&mut db).unwrap();
        session_transport::initialize_admission_schema(&mut db).unwrap();
        initialize_schema(&mut db).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_instances VALUES('instanceA','codex','homeA','identityA','sha256:test','0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('projectA','seatA','incarnationA','USER','LONG','instanceA','IDLE',1,1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('projectA','seatB','incarnationB','USER','LONG','instanceA','IDLE',1,1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_worktree_sources VALUES('repoA','sourceA','sourceIdentity','commonA','commonIdentity','HTTPS','baseline','gitDigest','gitVersion',1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_worktrees(worktree_id,path_id,repository_id,domain_id,seat_id,seat_incarnation,seat_generation,seat_revision,permission_tier,instance_id,source_revision,worktree_path,worktree_identity,git_pointer_hash,git_pointer_len,git_pointer_identity,common_identity,baseline_commit,state,revision) VALUES('treeA','wtA','repoA','projectA','seatA','incarnationA',1,1,'NetworkedWrite','instanceA',1,'pathA','pathIdentity','pointerHash',10,'pointerIdentity','commonIdentity','baseline','REGISTERED',1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingA','instanceA','projectA','SESSION','sessionA','1','ACTIVE')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES('projectA','sessionA','instanceA','homeA','bindingA','1','STOPPED',3)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_seat_binding VALUES('projectA','sessionA','seatA','incarnationA','1')").unwrap();
        assert!(matches!(fixture_gate(&db,false),Err(WorktreeError::Denied)));
        db.execute("UPDATE main.gogoke_v37_h_claim SET state='RELEASED'").unwrap();
        assert!(matches!(fixture_gate(&db,false),Err(WorktreeError::Denied)),
            "released without exact StopFact is insufficient");
        let original=r#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"open","requestId":"openA","targetId":"sessionA","domainId":"projectA","expectedRevision":"1","payload":{"repositoryId":"repoA","worktreeId":"treeA"}}"#;
        let raw_hex=original.bytes().map(|byte|format!("{byte:02x}")).collect::<String>();
        let insert=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA','openA',?1,'open','sessionA','APPLIED',1,2)").unwrap();
        insert.bind_text(1,&raw_hex).unwrap(); insert.step_done().unwrap(); drop(insert);
        db.execute("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase,stop_fact_id) VALUES('projectA','openA','sessionA','1','00',1,'processA','instanceA','homeA','bindingA','seatA','incarnationA','STOPPED','proofA')").unwrap();
        assert!(matches!(fixture_gate(&db,false),Err(WorktreeError::Denied)));
        db.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,stop_proof_hash) VALUES('processA','ticketA','nonceA','42','99','fixture-image','fixture-digest','profileA','projectA','1','STOPPED','proofA')").unwrap();
        assert_eq!(fixture_gate(&db,false).unwrap(),vec![ExactStopFact {
            process_operation_id:"processA".into(),stop_fact_id:"proofA".into() }]);
        // Another seat on the same instance is decided by the sealed physical
        // root, not by seat identity or a different worktree ID.
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingB','instanceA','projectA','SESSION','sessionB','2','ACTIVE')").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES('projectA','sessionB','instanceA','homeB','bindingB','2','RESERVED',1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_seat_binding VALUES('projectA','sessionB','seatB','incarnationB','2')").unwrap();
        let other=original.replace("openA","openB").replace("sessionA","sessionB").replace("treeA","treeB");
        let other_hex=other.bytes().map(|byte|format!("{byte:02x}")).collect::<String>();
        let insert=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA','openB',?1,'open','sessionB','UNKNOWN',1,1)").unwrap();
        insert.bind_text(1,&other_hex).unwrap(); insert.step_done().unwrap(); drop(insert);
        assert_eq!(fixture_gate(&db,false).unwrap().len(),1);
        assert!(matches!(fixture_gate(&db,true),Err(WorktreeError::Denied)),
            "a different ID whose sealed root intersects this tree still blocks");
        db.execute("UPDATE main.gogoke_coordination_process_custody SET stop_proof_hash='otherProof'").unwrap();
        assert!(matches!(fixture_gate(&db,false),Err(WorktreeError::Denied)));
        db.close_checked().unwrap(); drop(root); fs::remove_dir_all(path).unwrap();
    }
}
