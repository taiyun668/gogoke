//! H's read-only proof for one original pre-boot SESSION HOME execution.
//! A parent PID alone does not prove descendants gone; a kernel cutoff covers
//! the old Job only after its original H/C source has been resolved.

use super::decode_request;
use crate::process::{NativeLegacyHoldersGone, NativeLegacyHoldersGoneError};
use crate::store::atomic::{AtomicError, Json, JsonString, Statement};
use crate::store::digest::sha256_hex;
use crate::store::same_open::VerifiedDatabaseConnection;

#[derive(Debug)]
pub(crate) enum ExactHomeGoneError {
    Missing(&'static str),
    Source(String),
    Store(AtomicError),
    Native(NativeLegacyHoldersGoneError),
}
impl From<AtomicError> for ExactHomeGoneError {
    fn from(error: AtomicError) -> Self { Self::Store(error) }
}

struct OriginalExecution { pairs: Vec<(u32, u64)>, source_digest: String }

fn original_payload_shape(request: &super::V37Request, domain: &str,
    generation: &str, old_generation: &str, seat: &str) -> bool {
    let field = |name: &str| match request.payload.get(&JsonString::from_str(name)) {
        Some(Json::String(value)) => value.to_well_formed_string(), _ => None,
    };
    if matches!(request.operation.as_str(), "resume" | "compact" | "renew-session") {
        return request.payload.len() == 1 &&
            field("generation").as_deref() == Some(old_generation);
    }
    if domain == "global" {
        // The resident Secretary producer supplies the seat and new generation
        // from committed native E/H state, never from the empty USER payload.
        return request.payload.is_empty();
    }
    let formal = field("purpose").as_deref() == Some("FORMAL_REVIEW");
    request.payload.len() == (if formal { 5 } else { 4 }) &&
        field("generation").as_deref() == Some(generation) &&
        field("seatId").as_deref() == Some(seat) &&
        field("repositoryId").is_some_and(|value| !value.is_empty()) &&
        field("worktreeId").is_some_and(|value| !value.is_empty()) &&
        (!request.payload.contains_key(&JsonString::from_str("purpose")) || formal)
}

fn count(db: &VerifiedDatabaseConnection<'_>, sql: &str, values: &[&str])
    -> Result<String, ExactHomeGoneError> {
    let query = Statement::prepare(db.as_ptr(), sql)?;
    for (index, value) in values.iter().enumerate() {
        query.bind_text((index + 1) as i32, value)?;
    }
    if !query.step_row()? { return Err(ExactHomeGoneError::Missing("count query")); }
    let result = query.column_text(0)?;
    if query.step_row()? { return Err(ExactHomeGoneError::Missing("ambiguous count")); }
    Ok(result)
}

fn decode_lower_hex(value: &str) -> Result<Vec<u8>, ExactHomeGoneError> {
    if value.is_empty() || value.len() > 131_072 || value.len() % 2 != 0 ||
        !value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return Err(ExactHomeGoneError::Missing("original H raw hex"));
    }
    value.as_bytes().chunks_exact(2).map(|pair| {
        let text = std::str::from_utf8(pair).map_err(|error|
            ExactHomeGoneError::Source(format!("original H raw UTF-8: {error}")))?;
        u8::from_str_radix(text, 16).map_err(|error|
            ExactHomeGoneError::Source(format!("original H raw hex: {error}")))
    }).collect()
}

fn canonical_pair(pid: &str, creation: &str) -> Result<(u32, u64), ExactHomeGoneError> {
    let parsed_pid = pid.parse::<u32>().map_err(|error|
        ExactHomeGoneError::Source(format!("original C PID: {error}")))?;
    let parsed_creation = creation.parse::<u64>().map_err(|error|
        ExactHomeGoneError::Source(format!("original C FILETIME: {error}")))?;
    if parsed_pid == 0 || parsed_creation == 0 || pid != parsed_pid.to_string() ||
        creation != parsed_creation.to_string() {
        return Err(ExactHomeGoneError::Missing("canonical original C process identity"));
    }
    Ok((parsed_pid, parsed_creation))
}

fn source(db: &VerifiedDatabaseConnection<'_>, home: &str, instance: &str,
    domain: &str, session: &str, generation: &str, binding: &str)
    -> Result<OriginalExecution, ExactHomeGoneError> {
    let episode = Statement::prepare(db.as_ptr(),
        "SELECT instance_id,domain_id,session_id,generation,binding_id,request_id,
                raw_hex,phase,COALESCE(CAST(result_revision AS TEXT),''),
                COALESCE(old_generation,''),COALESCE(process_operation_id,''),
                COALESCE(stop_fact_id,''),COALESCE(seat_id,''),COALESCE(seat_incarnation,''),
                CAST(previous_revision AS TEXT),COALESCE(stop_request_id,'')
           FROM main.gogoke_v37_h_process_episode WHERE home_id=?1")?;
    episode.bind_text(1, home)?;
    if !episode.step_row()? {
        return Err(ExactHomeGoneError::Missing("original H execution for HOME"));
    }
    let e = (0..16).map(|index| episode.column_text(index))
        .collect::<Result<Vec<_>, _>>()?;
    if episode.step_row()? || e[0] != instance || e[1] != domain || e[2] != session ||
        e[3] != generation || e[4] != binding || e[8].is_empty() ||
        e[10].is_empty() || e[12].is_empty() || e[13].is_empty() ||
        !matches!(e[7].as_str(), "ACTIVE" | "UNKNOWN") ||
        !e[11].is_empty() || !e[15].is_empty() {
        return Err(ExactHomeGoneError::Missing("unique completed original H episode"));
    }
    drop(episode);
    let raw = decode_lower_hex(&e[6])?;
    let request = decode_request(&raw).map_err(|error|
        ExactHomeGoneError::Source(format!("original H request: {error:?}")))?;
    let digest = sha256_hex(&raw);
    let before = e[14].parse::<u64>().map_err(|error|
        ExactHomeGoneError::Source(format!("original H previous revision: {error}")))?;
    let after = e[8].parse::<u64>().map_err(|error|
        ExactHomeGoneError::Source(format!("original H result revision: {error}")))?;
    let compound = matches!(request.operation.as_str(), "compact" | "renew-session");
    let resume_execution = request.operation == "resume" || compound;
    let execution = if resume_execution { "resume" } else { "open" };
    if request.family != "K-SESSION" || !matches!(request.operation.as_str(), "open" | "resume" | "compact" | "renew-session") ||
        request.request_id != e[5] || request.domain_id != domain || request.target_id != session ||
        before.checked_add(1) != Some(after) ||
        e[10] != format!("h-{execution}-{}", &digest[..40]) ||
        (request.operation == "open" && !e[9].is_empty()) ||
        (resume_execution && e[9].is_empty()) {
        return Err(ExactHomeGoneError::Missing("original H request/episode identity"));
    }
    if !original_payload_shape(&request, domain, generation, &e[9], &e[12]) {
        return Err(ExactHomeGoneError::Missing("original H producer payload"));
    }
    let mut ancestry_fields = Vec::new();
    if request.operation == "open" && domain == "global" && count(db,
        "SELECT COUNT(*) FROM main.gogoke_v37_effective_seat s JOIN main.gogoke_v37_seat_secretary d ON d.domain_id=s.domain_id AND d.seat_id=s.seat_id AND d.incarnation=s.seat_incarnation WHERE s.domain_id='global' AND s.session_id=?1 AND s.generation=?2 AND s.seat_id=?3 AND s.seat_incarnation=?4 AND s.selected_instance_id=?5",
        &[session, generation, &e[12], &e[13], instance])? != "1" {
        return Err(ExactHomeGoneError::Missing("original Secretary native E/H selection"));
    }
    if compound {
        let q = Statement::prepare(db.as_ptr(),
            "SELECT raw_hex,operation,session_id,old_generation,old_process_operation_id,old_ticket,old_nonce,thread_id,seat_id,CAST(previous_revision AS TEXT),COALESCE(CAST(unknown_revision AS TEXT),''),stage,COALESCE(owner_stop_request_id,''),COALESCE(CAST(result_revision AS TEXT),''),COALESCE(ack_step_id,''),COALESCE(item_source_epoch,''),COALESCE(item_source_cursor,''),COALESCE(item_id,'') FROM main.gogoke_v37_h_generation_change WHERE domain_id=?1 AND request_id=?2")?;
        q.bind_text(1, domain)?; q.bind_text(2, &e[5])?;
        if !q.step_row()? { return Err(ExactHomeGoneError::Missing("original compound journal")); }
        let g = (0..18).map(|index| q.column_text(index)).collect::<Result<Vec<_>,_>>()?;
        if q.step_row()? || g[0] != e[6] || g[1] != request.operation ||
            g[2] != session || g[3] != e[9] || g[7].is_empty() ||
            g[8] != e[12] || g[9] != request.expected_revision.to_string() ||
            g[11] != "APPLIED" || !g[12].is_empty() || g[13] != e[8] ||
            (g[10].is_empty() && before != request.expected_revision) ||
            (!g[10].is_empty() && (g[10] != e[14] ||
                request.expected_revision.checked_add(1).map(|value| value.to_string())
                    != Some(g[10].clone()))) ||
            (request.operation == "compact" && (g[14] != format!("compact-{}", &digest[..40]) ||
                g[15].is_empty() || g[16].is_empty() || g[17].is_empty())) ||
            (request.operation == "renew-session" && (!g[14].is_empty() ||
                !g[15].is_empty() || !g[16].is_empty() || !g[17].is_empty())) {
            return Err(ExactHomeGoneError::Missing("original compound journal identity"));
        }
        ancestry_fields.extend(g.iter().cloned());
        if count(db,
            "SELECT COUNT(*) FROM main.gogoke_v37_h_process_episode p JOIN main.gogoke_v37_h_generation h ON h.domain_id=p.domain_id AND h.session_id=p.session_id AND h.generation=p.generation AND h.request_id=p.request_id AND h.process_operation_id=p.process_operation_id JOIN main.gogoke_coordination_process_custody c ON c.operation_id=p.process_operation_id AND c.profile_id=p.instance_id AND c.domain_id=p.domain_id AND c.generation=p.generation WHERE p.domain_id=?1 AND p.session_id=?2 AND p.instance_id=?3 AND p.generation=?4 AND p.process_operation_id=?5 AND p.seat_id=?6 AND p.phase='STOPPED' AND p.stop_request_id IS NULL AND p.stop_fact_id IS NOT NULL AND c.ticket=?7 AND c.custodian_nonce=?8 AND c.state='STOPPED' AND c.stop_proof_hash=p.stop_fact_id",
            &[domain, session, instance, &e[9], &g[4], &g[8], &g[5], &g[6]])? != "1" {
            return Err(ExactHomeGoneError::Missing("original compound internal STOP source"));
        }
    }
    if request.operation == "open" {
        if request.expected_revision != before {
            return Err(ExactHomeGoneError::Missing("original H OPEN revision"));
        }
    } else if request.expected_revision != before {
        // UNKNOWN promotion overwrites episode.previous_revision with the
        // UNKNOWN result. Only the original compound journal can witness the
        // intervening revision; a bare +2 difference is not evidence.
        if request.expected_revision.checked_add(1) != Some(before) || count(db,
            "SELECT COUNT(*) FROM main.gogoke_v37_h_generation_change WHERE domain_id=?1 AND session_id=?2 AND request_id=?3 AND old_generation=?4 AND unknown_revision=?5 AND stage='APPLIED'",
            &[domain, session, &e[5], &e[9], &e[14]])? != "1" {
            return Err(ExactHomeGoneError::Missing("original H UNKNOWN revision witness"));
        }
    }
    if resume_execution {
        let old = e[9].parse::<u64>().map_err(|error|
            ExactHomeGoneError::Source(format!("original H old generation: {error}")))?;
        let current = generation.parse::<u64>().map_err(|error|
            ExactHomeGoneError::Source(format!("original H generation: {error}")))?;
        if old.checked_add(1) != Some(current) || count(db,
            "SELECT COUNT(*) FROM main.gogoke_v37_h_process_episode p JOIN main.gogoke_coordination_process_custody c ON c.operation_id=p.process_operation_id AND c.profile_id=p.instance_id AND c.domain_id=p.domain_id AND c.generation=p.generation WHERE p.instance_id=?1 AND p.domain_id=?2 AND p.session_id=?3 AND p.generation=?4 AND p.phase='STOPPED' AND p.stop_fact_id IS NOT NULL AND c.state='STOPPED' AND c.stop_proof_hash=p.stop_fact_id",
            &[instance, domain, session, &e[9]])? != "1" {
            return Err(ExactHomeGoneError::Missing("original H resume ancestry"));
        }
        if !compound {
        let predecessor = Statement::prepare(db.as_ptr(),
            "SELECT p.stop_request_id,o.raw_hex,o.status,o.previous_revision,o.revision \
               FROM main.gogoke_v37_h_process_episode p \
               JOIN main.gogoke_v37_h_operation o ON o.domain_id=p.domain_id \
                 AND o.request_id=p.stop_request_id AND o.operation='stop' \
                 AND o.session_id=p.session_id \
              WHERE p.instance_id=?1 AND p.domain_id=?2 AND p.session_id=?3 \
                AND p.generation=?4 AND p.phase='STOPPED' \
                AND p.stop_fact_id IS NOT NULL")?;
        for (index,value) in [instance,domain,session,e[9].as_str()].iter().enumerate() {
            predecessor.bind_text((index+1) as i32,value)?;
        }
        if !predecessor.step_row()? {
            return Err(ExactHomeGoneError::Missing("original H predecessor STOP"));
        }
        let stop_id = predecessor.column_text(0)?;
        let stop_hex = predecessor.column_text(1)?;
        let stop_status = predecessor.column_text(2)?;
        let stop_before = predecessor.column_text(3)?;
        let stop_after = predecessor.column_text(4)?;
        if predecessor.step_row()? || stop_status != "APPLIED" {
            return Err(ExactHomeGoneError::Missing("original H predecessor STOP"));
        }
        let stop_raw = decode_lower_hex(&stop_hex)?;
        let stop = decode_request(&stop_raw).map_err(|error|
            ExactHomeGoneError::Source(format!("original H predecessor STOP raw: {error:?}")))?;
        let stop_field = |name: &str| match stop.payload.get(&JsonString::from_str(name)) {
            Some(Json::String(value)) => value.to_well_formed_string(), _ => None,
        };
        if stop.family != "K-SESSION" || stop.operation != "stop" ||
            stop.request_id != stop_id || stop.domain_id != domain ||
            stop.target_id != session || stop.payload.len() != 2 ||
            stop_field("generation").as_deref() != Some(e[9].as_str()) ||
            stop_field("seatId").as_deref() != Some(e[12].as_str()) ||
            stop.expected_revision.to_string() != stop_before ||
            stop_before.parse::<u64>().ok().and_then(|value| value.checked_add(1))
                .map(|value| value.to_string()) != Some(stop_after.clone()) {
            return Err(ExactHomeGoneError::Missing("original H predecessor STOP raw/revision"));
        }
        ancestry_fields.extend([stop_id,stop_hex,stop_status,stop_before,stop_after]);
        }
        let old = Statement::prepare(db.as_ptr(),
            "SELECT p.request_id,p.home_id,p.generation,p.process_operation_id,p.stop_fact_id,COALESCE(p.stop_request_id,''),c.ticket,c.custodian_nonce,c.pid,c.creation_time_100ns,c.stop_proof_hash FROM main.gogoke_v37_h_process_episode p JOIN main.gogoke_coordination_process_custody c ON c.operation_id=p.process_operation_id AND c.profile_id=p.instance_id AND c.domain_id=p.domain_id AND c.generation=p.generation WHERE p.instance_id=?1 AND p.domain_id=?2 AND p.session_id=?3 AND p.generation=?4 AND p.phase='STOPPED' AND p.stop_fact_id IS NOT NULL AND c.state='STOPPED' AND c.stop_proof_hash=p.stop_fact_id")?;
        for (index,value) in [instance,domain,session,e[9].as_str()].iter().enumerate() {
            old.bind_text((index+1) as i32,value)?;
        }
        if !old.step_row()? {return Err(ExactHomeGoneError::Missing("original H predecessor source"));}
        let old_fields = (0..11).map(|index| old.column_text(index)).collect::<Result<Vec<_>,_>>()?;
        if old.step_row()? {return Err(ExactHomeGoneError::Missing("ambiguous H predecessor source"));}
        ancestry_fields.extend(old_fields);
    }
    if count(db,
        "SELECT COUNT(*) FROM main.gogoke_v37_h_generation WHERE domain_id=?1 AND session_id=?2 AND generation=?3 AND request_id=?4 AND process_operation_id=?5",
        &[domain, session, generation, &e[5], &e[10]])? != "1" {
        return Err(ExactHomeGoneError::Missing("original H generation mapping"));
    }
    if request.operation == "open" {
        let operation = Statement::prepare(db.as_ptr(),
            "SELECT raw_hex,status,previous_revision,revision FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2 AND session_id=?3 AND operation='open'")?;
        operation.bind_text(1, domain)?;
        operation.bind_text(2, &e[5])?;
        operation.bind_text(3, session)?;
        if !operation.step_row()? || operation.column_text(0)? != e[6] ||
            operation.column_text(1)? != "APPLIED" ||
            operation.column_text(2)? != e[14] || operation.column_text(3)? != e[8] ||
            operation.step_row()? {
            return Err(ExactHomeGoneError::Missing("original H OPEN operation"));
        }
    } else if count(db,
        "SELECT COUNT(*) FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2",
        &[domain, &e[5]])? != "0" {
        return Err(ExactHomeGoneError::Missing("RESUME has unexpected H operation"));
    }
    let owner = Statement::prepare(db.as_ptr(),
        "SELECT state FROM main.gogoke_v37_h_owner_binding WHERE binding_id=?1 AND instance_id=?2 AND domain_id=?3 AND kind='SESSION' AND owner_id=?4 AND generation=?5")?;
    for (index, value) in [binding, instance, domain, session, generation].iter().enumerate() {
        owner.bind_text((index + 1) as i32, value)?;
    }
    if !owner.step_row()? {
        return Err(ExactHomeGoneError::Missing("original H owner binding"));
    }
    let owner_state = owner.column_text(0)?;
    if owner.step_row()? || !matches!(owner_state.as_str(), "ACTIVE" | "REVOKED") {
        return Err(ExactHomeGoneError::Missing("original H owner binding state"));
    }
    drop(owner);
    let custody = Statement::prepare(db.as_ptr(),
        "SELECT ticket,custodian_nonce,pid,creation_time_100ns,image_path,
                binary_digest_sha256,state,COALESCE(stop_proof_hash,''),profile_id,domain_id,generation
           FROM main.gogoke_coordination_process_custody WHERE operation_id=?1")?;
    custody.bind_text(1, &e[10])?;
    if !custody.step_row()? { return Err(ExactHomeGoneError::Missing("original C custody")); }
    let c = (0..11).map(|index| custody.column_text(index))
        .collect::<Result<Vec<_>, _>>()?;
    if custody.step_row()? || c[0].is_empty() || c[1].is_empty() || c[4].is_empty() ||
        !c[5].starts_with("sha256:") || c[5].len() != 71 ||
        !c[5][7..].bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) ||
        c[8] != instance || c[9] != domain || c[10] != generation ||
        !matches!(c[6].as_str(), "ACTIVE" | "UNKNOWN") || !c[7].is_empty() {
        return Err(ExactHomeGoneError::Missing("original C/Job custody identity"));
    }
    drop(custody);
    let pairs = vec![canonical_pair(&c[2], &c[3])?];
    if count(db,
        "SELECT COUNT(*) FROM main.gogoke_coordination_process_custody WHERE profile_id=?1 AND domain_id=?2 AND generation=?3",
        &[instance, domain, generation])? != "1" {
        return Err(ExactHomeGoneError::Missing("complete original C generation"));
    }
    let claim = Statement::prepare(db.as_ptr(),
        "SELECT instance_id,home_id,binding_id,generation,COALESCE(process_operation_id,''),state,COALESCE(stop_fact_id,'')
           FROM main.gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2")?;
    claim.bind_text(1, domain)?;
    claim.bind_text(2, session)?;
    if !claim.step_row()? { return Err(ExactHomeGoneError::Missing("current H claim")); }
    let a = (0..7).map(|index| claim.column_text(index))
        .collect::<Result<Vec<_>, _>>()?;
    if claim.step_row()? || a[0] != instance || a[1] != home || a[2] != binding ||
        a[3] != generation || a[4] != e[10] || !a[6].is_empty() ||
        !matches!(a[5].as_str(), "COMMITTED" | "UNKNOWN" | "RELEASED") {
        return Err(ExactHomeGoneError::Missing("same-generation current H claim"));
    }
    drop(claim);
    for (label, sql, values) in [
        ("unsettled H operation", "SELECT COUNT(*) FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2 AND status!='APPLIED'", vec![domain, session]),
        ("unattributed H stop operation", "SELECT COUNT(*) FROM main.gogoke_v37_h_operation o WHERE o.domain_id=?1 AND o.session_id=?2 AND o.operation='stop' AND (SELECT COUNT(*) FROM main.gogoke_v37_h_process_episode p WHERE p.domain_id=o.domain_id AND p.session_id=o.session_id AND p.stop_request_id=o.request_id AND p.phase='STOPPED' AND p.stop_fact_id IS NOT NULL)!=1", vec![domain, session]),
        ("target H stop operation", "SELECT COUNT(*) FROM main.gogoke_v37_h_process_episode p JOIN main.gogoke_v37_h_operation o ON o.domain_id=p.domain_id AND o.request_id=p.stop_request_id AND o.operation='stop' AND o.session_id=p.session_id WHERE p.home_id=?1 AND p.generation=?2 AND p.request_id=?3", vec![home, generation, e[5].as_str()]),
        ("unsettled H stdin", "SELECT COUNT(*) FROM main.gogoke_v37_h_stdin_journal WHERE domain_id=?1 AND session_id=?2 AND generation=?3 AND phase!='RECEIPTED'", vec![domain, session, generation]),
        ("unsettled H generation change", "SELECT COUNT(*) FROM main.gogoke_v37_h_generation_change WHERE domain_id=?1 AND session_id=?2 AND old_process_operation_id=?3 AND stage NOT IN ('APPLIED','CANCELLED','UNSUPPORTED')", vec![domain, session, e[10].as_str()]),
        ("other unsettled H episode", "SELECT COUNT(*) FROM main.gogoke_v37_h_process_episode WHERE domain_id=?1 AND session_id=?2 AND request_id!=?3 AND phase IN ('INTENT','PREPARED','ACTIVE','UNKNOWN')", vec![domain, session, e[5].as_str()]),
        ("pending F HOME operation", "SELECT COUNT(*) FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase!='APPLIED'", vec![home]),
    ] {
        if count(db, sql, &values)? != "0" {
            return Err(ExactHomeGoneError::Missing(label));
        }
    }
    let driver = Statement::prepare(db.as_ptr(),
        "SELECT driver_id FROM main.gogoke_v37_instances WHERE instance_id=?1")?;
    driver.bind_text(1, instance)?;
    if !driver.step_row()? { return Err(ExactHomeGoneError::Missing("original F instance")); }
    let driver_id = driver.column_text(0)?;
    if driver.step_row()? || !matches!(driver_id.as_str(), "codex" | "claude" | "opencode" | "grok" | "antigravity") {
        return Err(ExactHomeGoneError::Missing("ambiguous F instance/driver"));
    }
    if !matches!(driver_id.as_str(), "codex" | "claude") {
        return Err(ExactHomeGoneError::Missing("provider-specific execution source not qualified"));
    }
    drop(driver);
    let alias = Statement::prepare(db.as_ptr(),
        "SELECT request_id,process_operation_id,instance_id,phase,host_pid,
                host_creation_time_100ns,COALESCE(child_operation_id,''),
                COALESCE(child_ticket,''),COALESCE(child_nonce,''),
                COALESCE(child_pid,''),COALESCE(child_creation_time_100ns,''),
                namespace_id,alias_name,requested_target,COALESCE(observed_target,''),
                volume_serial,file_id,cleanup_status
           FROM main.gogoke_v37_h_dos_alias WHERE domain_id=?1 AND session_id=?2 AND generation=?3")?;
    alias.bind_text(1, domain)?;
    alias.bind_text(2, session)?;
    alias.bind_text(3, generation)?;
    let alias_values = if alias.step_row()? {
        let row = (0..18).map(|index| alias.column_text(index))
            .collect::<Result<Vec<_>, _>>()?;
        if alias.step_row()? { return Err(ExactHomeGoneError::Missing("ambiguous H alias")); }
        Some(row)
    } else { None };
    // OpenCode's old logon DOS namespace and source-bound host/child require
    // independent native attribution after a restart. The retained alias row
    // alone cannot establish that old namespace, so keep that family closed.
    if driver_id == "opencode" || alias_values.is_some() {
        return Err(ExactHomeGoneError::Missing("original alias namespace/host-child not qualified"));
    }
    let mut fields = vec![db.root_identity().opaque(), db.identity().opaque(),
        home.into(), instance.into(), domain.into(), session.into(), generation.into(),
        binding.into(), driver_id, owner_state];
    fields.extend(e);
    fields.extend(c);
    fields.extend(a);
    fields.extend(ancestry_fields);
    if let Some(alias) = alias_values { fields.extend(alias); }
    let mut bytes = Vec::new();
    for field in fields {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field.as_bytes());
    }
    Ok(OriginalExecution { pairs, source_digest: sha256_hex(&bytes) })
}

/// F may use this sealed native fact only for its independent HOME close.
pub(crate) fn observe_exact_home_gone(
    db: &VerifiedDatabaseConnection<'_>, home: &str, instance: &str,
    domain: &str, session: &str, generation: &str, binding: &str,
) -> Result<(NativeLegacyHoldersGone, String, Vec<(u32, u64)>), ExactHomeGoneError> {
    let original = source(db, home, instance, domain, session, generation, binding)?;
    let fact = NativeLegacyHoldersGone::observe(&original.pairs)
        .map_err(ExactHomeGoneError::Native)?;
    fact.validate(&original.pairs).map_err(ExactHomeGoneError::Native)?;
    Ok((fact, original.source_digest, original.pairs))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn insert(db: &VerifiedDatabaseConnection<'_>, sql: &str, values: &[&str]) {
        let row = Statement::prepare(db.as_ptr(), sql).unwrap();
        for (index, value) in values.iter().enumerate() {
            row.bind_text((index + 1) as i32, value).unwrap();
        }
        row.step_done().unwrap();
    }

    #[test]
    fn original_home_source_is_qualified_before_any_os_absence_query() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-home-kernel-source-{}-{nonce}",
            std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut db = create_new(&root, &path.join("state.sqlite")).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        crate::store::instance::initialize_schema(&mut db).unwrap();
        super::super::initialize_admission_schema(&mut db).unwrap();
        crate::store::authority::initialize_process_custody_schema(&mut db).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','instance-home-instanceA','identityA','sha256:0000000000000000000000000000000000000000000000000000000000000000','0.1','UNKNOWN','UNKNOWN',1)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_h_owner_binding(binding_id,instance_id,domain_id,kind,owner_id,generation,state) VALUES('bindingA','instanceA','projectA','SESSION','sessionA','1','ACTIVE')").unwrap();
        let raw = br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"open","requestId":"openA","targetId":"sessionA","domainId":"projectA","expectedRevision":"1","payload":{"seatId":"seatA","generation":"1","repositoryId":"repoA","worktreeId":"worktreeA"}}"#;
        let hex = raw.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        let process = format!("h-open-{}", &sha256_hex(raw)[..40]);
        let operation = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA','openA',?1,'open','sessionA','APPLIED',1,2)").unwrap();
        operation.bind_text(1, &hex).unwrap(); operation.step_done().unwrap(); drop(operation);
        let claim = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('projectA','sessionA','instanceA','homeA','bindingA','1','COMMITTED',2,?1)").unwrap();
        claim.bind_text(1, &process).unwrap(); claim.step_done().unwrap(); drop(claim);
        let episode = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('projectA','openA','sessionA','1',?1,1,2,?2,'instanceA','homeA','bindingA','seatA','incA','ACTIVE')").unwrap();
        episode.bind_text(1, &hex).unwrap(); episode.bind_text(2, &process).unwrap();
        episode.step_done().unwrap(); drop(episode);
        let generation = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id) VALUES('projectA','sessionA','1','openA',?1)").unwrap();
        generation.bind_text(1, &process).unwrap(); generation.step_done().unwrap(); drop(generation);
        let custody = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES(?1,'ticketA','nonceA','42','1','fixture.exe','sha256:0000000000000000000000000000000000000000000000000000000000000000','instanceA','projectA','1','UNKNOWN')").unwrap();
        custody.bind_text(1, &process).unwrap(); custody.step_done().unwrap(); drop(custody);
        let original = source(&db, "homeA", "instanceA", "projectA", "sessionA", "1", "bindingA").unwrap();
        assert_eq!(original.pairs, vec![(42, 1)]);
        db.execute("INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA','olderStop','00','stop','sessionA','APPLIED',0,1)").unwrap();
        assert!(matches!(source(&db, "homeA", "instanceA", "projectA", "sessionA", "1", "bindingA"),
            Err(ExactHomeGoneError::Missing("unattributed H stop operation"))),
            "an unowned malformed STOP cannot be called unrelated history");
        db.execute("DELETE FROM main.gogoke_v37_h_operation WHERE request_id='olderStop'").unwrap();
        db.execute("UPDATE main.gogoke_v37_h_operation SET status='UNKNOWN' WHERE request_id='openA'").unwrap();
        assert!(source(&db, "homeA", "instanceA", "projectA", "sessionA", "1", "bindingA").is_err(),
            "unsettled original H raw cannot reach the native fact");
        db.execute("UPDATE main.gogoke_v37_h_operation SET status='APPLIED' WHERE request_id='openA'").unwrap();
        db.execute("UPDATE main.gogoke_v37_h_claim SET generation='2' WHERE session_id='sessionA'").unwrap();
        assert!(source(&db, "homeA", "instanceA", "projectA", "sessionA", "1", "bindingA").is_err(),
            "another generation cannot donate the old execution");
        db.execute("UPDATE main.gogoke_v37_h_claim SET generation='1' WHERE session_id='sessionA'").unwrap();
        db.execute("UPDATE main.gogoke_coordination_process_custody SET creation_time_100ns='0' WHERE ticket='ticketA'").unwrap();
        assert!(source(&db, "homeA", "instanceA", "projectA", "sessionA", "1", "bindingA").is_err(),
            "missing original FILETIME cannot be inferred");
        db.execute("UPDATE main.gogoke_coordination_process_custody SET creation_time_100ns='1' WHERE ticket='ticketA'").unwrap();
        let stdin = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_stdin_journal(domain_id,request_id,operation,ticket,process_operation_id,custodian_nonce,session_id,generation,request_hex,phase,expected_revision) VALUES('projectA','pendingSend','send','ticketA',?1,'nonceA','sessionA','1','00','UNKNOWN','2')").unwrap();
        stdin.bind_text(1, &process).unwrap(); stdin.step_done().unwrap(); drop(stdin);
        assert!(source(&db, "homeA", "instanceA", "projectA", "sessionA", "1", "bindingA").is_err(),
            "an UNKNOWN H stdin request cannot be hidden by kernel absence");
        db.execute("DELETE FROM main.gogoke_v37_h_stdin_journal WHERE request_id='pendingSend'").unwrap();
        db.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES('otherProcess','ticketB','nonceB','43','2','fixture.exe','sha256:0000000000000000000000000000000000000000000000000000000000000000','instanceA','projectA','1','UNKNOWN')").unwrap();
        assert!(source(&db, "homeA", "instanceA", "projectA", "sessionA", "1", "bindingA").is_err(),
            "an unlisted same-generation C process must refuse");
        drop(db); drop(root); std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn original_producer_shapes_keep_native_seat_and_generation_authority() {
        let open = decode_request(br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"open","requestId":"openA","targetId":"sessionA","domainId":"projectA","expectedRevision":"1","payload":{"seatId":"seatA","generation":"1","repositoryId":"repoA","worktreeId":"worktreeA"}}"#).unwrap();
        assert!(original_payload_shape(&open,"projectA","1","","seatA"));
        assert!(!original_payload_shape(&open,"projectA","2","","seatA"));
        let resident = decode_request(br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"open","requestId":"resident-open-a","targetId":"resident-secretary-a","domainId":"global","expectedRevision":"2","payload":{}}"#).unwrap();
        assert!(original_payload_shape(&resident,"global","1","","seatA"));
        assert!(!original_payload_shape(&resident,"projectA","1","","seatA"));
        let resume = decode_request(br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"resume","requestId":"resumeA","targetId":"sessionA","domainId":"projectA","expectedRevision":"3","payload":{"generation":"1"}}"#).unwrap();
        assert!(original_payload_shape(&resume,"projectA","2","1","seatA"));
        assert!(!original_payload_shape(&resume,"projectA","2","2","seatA"));
    }

    #[test]
    fn original_resume_and_compound_rows_qualify_only_their_own_stop_source() {
        let _guard = route_b_test_guard();
        for case in ["resume", "compact", "renew-session", "compact_unknown"] {
            let operation = if case == "compact_unknown" { "compact" } else { case };
            let unknown = case == "compact_unknown";
            let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir().join(format!("gogoke-home-continuation-{case}-{nonce}"));
            std::fs::create_dir(&path).unwrap();
            let root = RootLock::acquire(&path).unwrap();
            let mut db = create_new(&root, &path.join("state.sqlite")).unwrap();
            db.execute("PRAGMA foreign_keys=ON").unwrap();
            crate::store::instance::initialize_schema(&mut db).unwrap();
            super::super::initialize_admission_schema(&mut db).unwrap();
            crate::store::authority::initialize_process_custody_schema(&mut db).unwrap();
            db.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','instance-home-instanceA','identityA','sha256:0000000000000000000000000000000000000000000000000000000000000000','0.160.0','UNKNOWN','UNKNOWN',1)").unwrap();
            let open = br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"open","requestId":"openA","targetId":"sessionA","domainId":"projectA","expectedRevision":"1","payload":{"seatId":"seatA","generation":"1","repositoryId":"repoA","worktreeId":"worktreeA"}}"#;
            let open_hex = open.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
            let old_process = format!("h-open-{}", &sha256_hex(open)[..40]);
            insert(&db,"INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA','openA',?1,'open','sessionA','APPLIED',1,2)", &[&open_hex]);
            let stop_id = if operation == "resume" { "stopA" } else { "" };
            if operation == "resume" {
                let stop = br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"stop","requestId":"stopA","targetId":"sessionA","domainId":"projectA","expectedRevision":"2","payload":{"seatId":"seatA","generation":"1"}}"#;
                let stop_hex = stop.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
                insert(&db,"INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA','stopA',?1,'stop','sessionA','APPLIED',2,3)", &[&stop_hex]);
            }
            insert(&db,"INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,stop_request_id,phase,stop_fact_id) VALUES('projectA','openA','sessionA','1',?1,1,2,?2,'instanceA','homeOld','bindingOld','seatA','incA',NULLIF(?3,''),'STOPPED','proofOld')", &[&open_hex,&old_process,stop_id]);
            insert(&db,"INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id) VALUES('projectA','sessionA','1','openA',?1)", &[&old_process]);
            insert(&db,"INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,stop_proof_hash) VALUES(?1,'ticketOld','nonceOld','41','1','fixture.exe','sha256:0000000000000000000000000000000000000000000000000000000000000000','instanceA','projectA','1','STOPPED','proofOld')", &[&old_process]);
            let before = if operation == "resume" { 3 } else { 2 };
            let raw = format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"{operation}\",\"requestId\":\"nextA\",\"targetId\":\"sessionA\",\"domainId\":\"projectA\",\"expectedRevision\":\"{before}\",\"payload\":{{\"generation\":\"1\"}}}}");
            let raw_hex = raw.as_bytes().iter().map(|byte| format!("{byte:02x}")).collect::<String>();
            let digest = sha256_hex(raw.as_bytes());
            let process = format!("h-resume-{}", &digest[..40]);
            let episode_before = before + (if unknown { 1 } else { 0 });
            let before_text = episode_before.to_string();
            let after_text = (episode_before + 1).to_string();
            insert(&db,"INSERT INTO main.gogoke_v37_h_owner_binding(binding_id,instance_id,domain_id,kind,owner_id,generation,state) VALUES('bindingNew','instanceA','projectA','SESSION','sessionA','2','ACTIVE')", &[]);
            insert(&db,"INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('projectA','sessionA','instanceA','homeNew','bindingNew','2','COMMITTED',?2,?1)", &[&process,&after_text]);
            insert(&db,"INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('projectA','nextA','sessionA','2','1',?1,?2,?3,?4,'instanceA','homeNew','bindingNew','seatA','incA','ACTIVE')", &[&raw_hex,&before_text,&after_text,&process]);
            insert(&db,"INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id) VALUES('projectA','sessionA','2','nextA',?1)", &[&process]);
            insert(&db,"INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES(?1,'ticketNew','nonceNew','42','1','fixture.exe','sha256:0000000000000000000000000000000000000000000000000000000000000000','instanceA','projectA','2','UNKNOWN')", &[&process]);
            if operation != "resume" {
                let ack = if operation == "compact" { format!("compact-{}", &digest[..40]) } else { String::new() };
                if operation == "compact" {
                    insert(&db,"INSERT INTO main.gogoke_v37_h_generation_change(domain_id,request_id,raw_hex,operation,session_id,old_generation,old_process_operation_id,old_ticket,old_nonce,thread_id,seat_id,previous_revision,source_watermark,stage,ack_step_id,item_source_epoch,item_source_cursor,item_id) VALUES('projectA','nextA',?1,?2,'sessionA','1',?3,'ticketOld','nonceOld','threadOld','seatA',2,0,'OLD_STOPPED',?4,'epochA','1','itemA')", &[&raw_hex,operation,&old_process,&ack]);
                } else {
                    insert(&db,"INSERT INTO main.gogoke_v37_h_generation_change(domain_id,request_id,raw_hex,operation,session_id,old_generation,old_process_operation_id,old_ticket,old_nonce,thread_id,seat_id,previous_revision,source_watermark,stage) VALUES('projectA','nextA',?1,?2,'sessionA','1',?3,'ticketOld','nonceOld','threadOld','seatA',2,0,'OLD_STOPPED')", &[&raw_hex,operation,&old_process]);
                }
                if unknown {
                    super::super::generation_change::note_candidate_unknown(
                        &db,"projectA","nextA",episode_before).unwrap();
                }
                super::super::generation_change::mark_applied(&db,"projectA","nextA",episode_before + 1).unwrap();
            }
            assert!(source(&db,"homeNew","instanceA","projectA","sessionA","2","bindingNew").is_ok(),
                "real wire operation must retain its internal h-resume source");
            if operation != "resume" {
                db.execute("UPDATE main.gogoke_v37_h_generation_change SET old_ticket='wrong' WHERE request_id='nextA'").unwrap();
                assert!(matches!(source(&db,"homeNew","instanceA","projectA","sessionA","2","bindingNew"),
                    Err(ExactHomeGoneError::Missing("original compound internal STOP source"))));
            } else {
                db.execute("UPDATE main.gogoke_v37_h_process_episode SET stop_request_id=NULL WHERE request_id='openA'").unwrap();
                assert!(matches!(source(&db,"homeNew","instanceA","projectA","sessionA","2","bindingNew"),
                    Err(ExactHomeGoneError::Missing("original H predecessor STOP"))));
            }
            drop(db); drop(root); std::fs::remove_dir_all(path).unwrap();
        }
    }
}
