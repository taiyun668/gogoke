//! C's internal HOST_RULE notification. The E proof is the only source of the
//! destination and body; neither a User wire request nor model text enters here.
use std::collections::BTreeMap;

use super::{raw_hex, read_message, read_operation, replay, require_one_change, save_operation,
    unhex_native,
    transact, valid_id, InboxError, Message, StoredOperation};
use crate::store::atomic::{Json, JsonString, Parser, Statement};
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
use crate::store::same_open::VerifiedDatabaseConnection;
use crate::store::seat::{revalidate_host_escalation_in_transaction, HostEscalationProof};
use crate::store::session_transport::{codex_rpc, decode_receipt, decode_request, read_stdin_journal,
    JournalState, StdinJournalKey, V37Status};

/// This value is persisted in C's sender column only for the internal path.
/// The actual source seat remains in the original request and E's sealed cause.
pub(crate) const HOST_RULE_ACTOR: &str = "HOST_RULE";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HostMessageIds {
    pub(crate) message_id: String,
    pub(crate) enqueue_request_id: String,
    pub(crate) delivery_request_id: String,
    /// H's sole ordinary K-SESSION/send request ID for this cause.
    pub(crate) send_request_id: String,
}

pub(crate) struct HostDeliveryTarget<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) ticket: &'a str,
    pub(crate) generation: &'a str,
}

struct HostFields<'a> {
    domain: &'a str,
    escalation_request: &'a str,
    trigger: &'a str,
    cause: &'a str,
    source: &'a str,
    destination: &'a str,
    policy_revision: i64,
    route_revision: i64,
    body: &'a str,
}

fn proof_fields(proof: &HostEscalationProof) -> HostFields<'_> {
    HostFields { domain: proof.domain_id(), escalation_request: proof.request_id(),
        trigger: proof.trigger_id(), cause: proof.cause_event_id(),
        source: proof.source_seat_id(), destination: proof.destination_seat_id(),
        policy_revision: proof.policy_revision(), route_revision: proof.route_revision(),
        body: proof.notice_body() }
}

fn identity_fields(fields: &HostFields<'_>) -> HostMessageIds {
    let basis = format!("{}\n{}\n{}\n{}\n{}", fields.domain, fields.escalation_request,
        fields.trigger, fields.cause, fields.policy_revision);
    let digest = crate::store::digest::sha256_hex(basis.as_bytes());
    HostMessageIds {
        message_id: format!("hostmsg-{digest}"),
        enqueue_request_id: format!("hostenqueue-{digest}"),
        delivery_request_id: format!("hostdeliver-{digest}"),
        send_request_id: format!("hostsend-{digest}"),
    }
}

fn identity(proof: &HostEscalationProof) -> HostMessageIds { identity_fields(&proof_fields(proof)) }

/// Stable selectors are derived solely from the E proof, never accepted from
/// the Node, User, or Model request surface.
pub(crate) fn host_message_ids(proof: &HostEscalationProof) -> HostMessageIds { identity(proof) }

fn string(value: &str) -> Json { Json::String(JsonString::from_str(value)) }

fn original_request_fields(fields: &HostFields<'_>, ids: &HostMessageIds,
    operation: &str, target: Option<&HostDeliveryTarget<'_>>) -> Vec<u8> {
    let mut fields = BTreeMap::from([
        (JsonString::from_str("schema"), string("gogoke.37.host-rule-inbox.v1")),
        (JsonString::from_str("actor"), string(HOST_RULE_ACTOR)),
        (JsonString::from_str("operation"), string(operation)),
        (JsonString::from_str("domainId"), string(fields.domain)),
        (JsonString::from_str("messageId"), string(&ids.message_id)),
        (JsonString::from_str("escalationRequestId"), string(fields.escalation_request)),
        (JsonString::from_str("triggerId"), string(fields.trigger)),
        (JsonString::from_str("causeEventId"), string(fields.cause)),
        (JsonString::from_str("sourceSeatId"), string(fields.source)),
        (JsonString::from_str("destinationSeatId"), string(fields.destination)),
        (JsonString::from_str("policyRevision"), string(&fields.policy_revision.to_string())),
        (JsonString::from_str("routeRevision"), string(&fields.route_revision.to_string())),
        (JsonString::from_str("body"), string(fields.body)),
    ]);
    if let Some(target) = target {
        fields.insert(JsonString::from_str("sessionId"), string(target.session_id));
        fields.insert(JsonString::from_str("ticket"), string(target.ticket));
        fields.insert(JsonString::from_str("generation"), string(target.generation));
        fields.insert(JsonString::from_str("hSendRequestId"), string(&ids.send_request_id));
    }
    let mut bytes = Json::Object(fields).canonical().into_bytes();
    bytes.push(b'\n');
    bytes
}

fn original_request(proof: &HostEscalationProof, ids: &HostMessageIds,
    operation: &str, target: Option<&HostDeliveryTarget<'_>>) -> Vec<u8> {
    original_request_fields(&proof_fields(proof), ids, operation, target)
}

fn revalidate(db: &VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    proof: &HostEscalationProof) -> Result<(), InboxError> {
    check_owner_in_current_transaction(db, owner).map_err(InboxError::Authority)?;
    revalidate_host_escalation_in_transaction(db, owner, proof)
        .map_err(|error| InboxError::InvalidEvidence(format!("E host cause: {error:?}")))?;
    let row = Statement::prepare(db.as_ptr(),
        "SELECT request_id,from_seat_id,to_seat_id,state FROM main.gogoke_v37_seat_policy_escalations
          WHERE domain_id=?1 AND trigger_id=?2")?;
    row.bind_text(1, proof.domain_id())?;
    row.bind_text(2, proof.trigger_id())?;
    if !row.step_row()? || row.column_text(0)? != proof.request_id() ||
        row.column_text(1)? != proof.source_seat_id() ||
        row.column_text(2)? != proof.destination_seat_id() ||
        !matches!(row.column_text(3)?.as_str(), "INTENT" | "UNKNOWN" | "DELIVERED") ||
        row.step_row()? { return Err(InboxError::Denied); }
    let event = Statement::prepare(db.as_ptr(),
        "SELECT operation,target_id,policy_revision,state,detail FROM
           main.gogoke_v37_seat_policy_events WHERE domain_id=?1 AND event_id=?2")?;
    event.bind_text(1, proof.domain_id())?;
    event.bind_text(2, proof.request_id())?;
    if !event.step_row()? || event.column_text(0)? != "escalate" ||
        event.column_text(1)? != proof.trigger_id() ||
        event.column_text(2)? != proof.policy_revision().to_string() ||
        event.column_text(3)? != "INTENT" ||
        event.column_text(4)? != proof.cause_event_id() || event.step_row()? {
        return Err(InboxError::Denied);
    }
    Ok(())
}

fn original_host_message(db: &VerifiedDatabaseConnection<'_>, proof: &HostEscalationProof,
    ids: &HostMessageIds) -> Result<Message, InboxError> {
    let message = read_message(db, proof.domain_id(), &ids.message_id)?
        .ok_or(InboxError::Conflict)?;
    let original = read_operation(db, proof.domain_id(), &ids.enqueue_request_id)?
        .ok_or(InboxError::Conflict)?;
    if message.sender_seat_id != HOST_RULE_ACTOR ||
        message.seat_id != proof.destination_seat_id() ||
        message.body != proof.notice_body() ||
        original.request_hex != raw_hex(&original_request(proof, ids, "enqueue", None)) ||
        original.message_id != ids.message_id || original.phase != "APPLIED" ||
        original.result_state != "PENDING" { return Err(InboxError::Denied); }
    Ok(message)
}

/// One durable HOST_RULE notification from the existing E INTENT. Its pending
/// turn and generation are explicitly empty until H proves an ordinary send.
pub(crate) fn enqueue_host_escalation(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, proof: &HostEscalationProof)
    -> Result<(Message, StoredOperation), InboxError> {
    transact(db, |db| enqueue_host_escalation_in_transaction(db, owner, proof))
}

/// Compose directly after E's begin_host_escalation_in_transaction in one
/// Owner transaction. A route change cannot strand an E-only INTENT.
pub(crate) fn enqueue_host_escalation_in_transaction(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, proof: &HostEscalationProof)
    -> Result<(Message, StoredOperation), InboxError> {
    let ids = identity(proof);
    let bytes = original_request(proof, &ids, "enqueue", None);
    revalidate(db, owner, proof)?;
    if let Some(prior) = replay(db, proof.domain_id(), &ids.enqueue_request_id,
        &ids.message_id, &bytes)? {
        return Ok((original_host_message(db, proof, &ids)?, prior));
    }
    if read_message(db, proof.domain_id(), &ids.message_id)?.is_some() {
        return Err(InboxError::Conflict);
    }
    let row = Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_inbox_messages
             (domain_id,message_id,revision,state,sender_seat_id,seat_id,turn_id,generation,body,requeued_as)
             VALUES(?1,?2,'1','PENDING',?3,?4,'','',?5,NULL)")?;
    for (index, value) in [proof.domain_id(), ids.message_id.as_str(), HOST_RULE_ACTOR,
        proof.destination_seat_id(), proof.notice_body()].iter().enumerate() {
        row.bind_text((index + 1) as i32, value)?;
    }
    row.step_done()?;
    save_operation(db, proof.domain_id(), &ids.enqueue_request_id, &raw_hex(&bytes),
        &ids.message_id, "APPLIED", 0, 1, "PENDING", "", "")?;
    Ok((original_host_message(db, proof, &ids)?,
        read_operation(db, proof.domain_id(), &ids.enqueue_request_id)?
            .ok_or(InboxError::Unknown)?))
}

fn target_valid(target: &HostDeliveryTarget<'_>) -> bool {
    valid_id(target.session_id) && valid_id(target.ticket) &&
        !target.generation.is_empty() && target.generation.bytes().all(|b| b.is_ascii_digit())
}

fn original_target(db: &VerifiedDatabaseConnection<'_>, domain: &str, destination: &str,
    target: &HostDeliveryTarget<'_>, observed_readback: bool) -> Result<(), InboxError> {
    let reserve_sql = "SELECT 1 FROM main.gogoke_v37_h_seat_binding sb
          JOIN main.gogoke_v37_h_claim h ON h.domain_id=sb.domain_id
            AND h.session_id=sb.session_id AND h.generation=sb.generation
          JOIN main.gogoke_v37_h_process_episode ep ON ep.domain_id=h.domain_id
            AND ep.session_id=h.session_id AND ep.generation=h.generation
            AND ep.process_operation_id=h.process_operation_id
          JOIN main.gogoke_v37_seats seat ON seat.domain_id=sb.domain_id
            AND seat.seat_id=sb.seat_id AND seat.incarnation=sb.seat_incarnation
            AND CAST(seat.generation AS TEXT)=sb.generation
            AND seat.instance_id=h.instance_id AND seat.state='BUSY'
          JOIN main.gogoke_coordination_process_custody c ON c.operation_id=ep.process_operation_id
            AND c.domain_id=ep.domain_id AND c.generation=ep.generation
          WHERE sb.domain_id=?1 AND sb.session_id=?2 AND sb.seat_id=?3
            AND sb.generation=?4 AND c.ticket=?5 AND h.state='COMMITTED'
            AND ep.phase='ACTIVE' AND c.state='ACTIVE'";
    // The seat binding is updated by a later generation. Historical readback
    // must use this exact episode's original seat, not the new live binding.
    let readback_sql = "SELECT 1 FROM main.gogoke_v37_h_process_episode ep
          JOIN main.gogoke_v37_h_generation g ON g.domain_id=ep.domain_id
            AND g.session_id=ep.session_id AND g.generation=ep.generation
            AND g.process_operation_id=ep.process_operation_id
          JOIN main.gogoke_coordination_process_custody c ON c.operation_id=ep.process_operation_id
            AND c.domain_id=ep.domain_id AND c.generation=ep.generation
          WHERE ep.domain_id=?1 AND ep.session_id=?2 AND ep.seat_id=?3
            AND ep.generation=?4 AND c.ticket=?5
            AND ((ep.phase='ACTIVE' AND c.state IN ('ACTIVE','UNKNOWN'))
              OR (ep.phase='UNKNOWN' AND c.state IN ('UNKNOWN','STOPPED'))
              OR (ep.phase='STOPPED' AND c.state='STOPPED'
                AND length(c.stop_proof_hash)>0 AND ep.stop_fact_id=c.stop_proof_hash))";
    let row = Statement::prepare(db.as_ptr(), if observed_readback {readback_sql} else {reserve_sql})?;
    for (index, value) in [domain, target.session_id,
        destination, target.generation, target.ticket].iter().enumerate() {
        row.bind_text((index + 1) as i32, value)?;
    }
    if !row.step_row()? || row.step_row()? { return Err(InboxError::Denied); }
    Ok(())
}

/// Reserve exactly one ordinary H send. Replaying PREPARED or UNKNOWN returns
/// the original operation and no message to dispatch again.
pub(crate) fn reserve_host_delivery(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, proof: &HostEscalationProof, target: &HostDeliveryTarget<'_>)
    -> Result<(StoredOperation, Option<Message>), InboxError> {
    if !target_valid(target) { return Err(InboxError::Invalid("host delivery target")); }
    let ids = identity(proof);
    let bytes = original_request(proof, &ids, "deliver", Some(target));
    transact(db, |db| {
        revalidate(db, owner, proof)?;
        let message = original_host_message(db, proof, &ids)?;
        if let Some(prior) = replay(db, proof.domain_id(), &ids.delivery_request_id,
            &ids.message_id, &bytes)? { return Ok((prior, None)); }
        if message.state != "PENDING" || !message.turn_id.is_empty() ||
            !message.generation.is_empty() { return Err(InboxError::Conflict); }
        original_target(db, proof.domain_id(), proof.destination_seat_id(), target, false)?;
        let row = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_inbox_messages SET state='PREPARED'
              WHERE domain_id=?1 AND message_id=?2 AND state='PENDING' AND revision='1'")?;
        row.bind_text(1, proof.domain_id())?;
        row.bind_text(2, &ids.message_id)?;
        row.step_done()?;
        require_one_change(db)?;
        save_operation(db, proof.domain_id(), &ids.delivery_request_id, &raw_hex(&bytes),
            &ids.message_id, "PREPARED", 1, 1, "PREPARED", "", "")?;
        Ok((read_operation(db, proof.domain_id(), &ids.delivery_request_id)?
            .ok_or(InboxError::Unknown)?, Some(message)))
    })
}

/// Fence the physical H send before dispatch. A missing H result remains this
/// same UNKNOWN operation; no replay creates another provider write.
pub(crate) fn mark_host_delivery_unknown(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, proof: &HostEscalationProof, target: &HostDeliveryTarget<'_>)
    -> Result<StoredOperation, InboxError> {
    if !target_valid(target) { return Err(InboxError::Invalid("host delivery target")); }
    let ids = identity(proof);
    let bytes = original_request(proof, &ids, "deliver", Some(target));
    transact(db, |db| {
        revalidate(db, owner, proof)?;
        let message = original_host_message(db, proof, &ids)?;
        let prior = replay(db, proof.domain_id(), &ids.delivery_request_id,
            &ids.message_id, &bytes)?.ok_or(InboxError::Conflict)?;
        if prior.phase != "PREPARED" { return Ok(prior); }
        if message.state != "PREPARED" || message.revision != 1 ||
            !message.turn_id.is_empty() || !message.generation.is_empty() {
            return Err(InboxError::Conflict);
        }
        let row = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_inbox_messages SET state='UNKNOWN',revision='2'
              WHERE domain_id=?1 AND message_id=?2 AND state='PREPARED' AND revision='1'")?;
        row.bind_text(1, proof.domain_id())?;
        row.bind_text(2, &ids.message_id)?;
        row.step_done()?;
        require_one_change(db)?;
        let row = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_inbox_operations SET phase='UNKNOWN',revision='2',
              result_state='UNKNOWN' WHERE domain_id=?1 AND request_id=?2 AND phase='PREPARED'")?;
        row.bind_text(1, proof.domain_id())?;
        row.bind_text(2, &ids.delivery_request_id)?;
        row.step_done()?;
        require_one_change(db)?;
        read_operation(db, proof.domain_id(), &ids.delivery_request_id)?.ok_or(InboxError::Unknown)
    })
}

fn payload_text(request: &crate::store::session_transport::V37Request,
    name: &str) -> Result<String, InboxError> {
    match request.payload.get(&JsonString::from_str(name)) {
        Some(Json::String(value)) => value.to_well_formed_string().ok_or(InboxError::Denied),
        _ => Err(InboxError::Denied),
    }
}

struct HistoricalHost {
    domain: String,
    escalation_request: String,
    trigger: String,
    cause: String,
    source: String,
    destination: String,
    policy_revision: i64,
    route_revision: i64,
    body: String,
    ids: HostMessageIds,
    message: Message,
}

impl HistoricalHost {
    fn fields(&self) -> HostFields<'_> {
        HostFields { domain: &self.domain, escalation_request: &self.escalation_request,
            trigger: &self.trigger, cause: &self.cause, source: &self.source,
            destination: &self.destination, policy_revision: self.policy_revision,
            route_revision: self.route_revision, body: &self.body }
    }
}

fn internal_text(fields: &BTreeMap<JsonString, Json>, name: &str) -> Result<String, InboxError> {
    match fields.get(&JsonString::from_str(name)) {
        Some(Json::String(value)) => value.to_well_formed_string().ok_or(InboxError::Denied),
        _ => Err(InboxError::Denied),
    }
}

/// Recover the sealed historical identity from C's original internal request,
/// E's exact INTENT event, and the retained message. Current route/head are
/// intentionally not substituted for the original send's authorization.
fn historical_host(db: &VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    domain: &str, message_id: &str) -> Result<HistoricalHost, InboxError> {
    check_owner_in_current_transaction(db, owner).map_err(InboxError::Authority)?;
    if !valid_id(domain) || !valid_id(message_id) ||
        !message_id.starts_with("hostmsg-") { return Err(InboxError::Invalid("host message")); }
    let digest = &message_id["hostmsg-".len()..];
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit() &&
        !byte.is_ascii_uppercase()) { return Err(InboxError::Denied); }
    let ids = HostMessageIds { message_id: message_id.into(),
        enqueue_request_id: format!("hostenqueue-{digest}"),
        delivery_request_id: format!("hostdeliver-{digest}"),
        send_request_id: format!("hostsend-{digest}") };
    let message = read_message(db, domain, message_id)?.ok_or(InboxError::Conflict)?;
    let original = read_operation(db, domain, &ids.enqueue_request_id)?
        .ok_or(InboxError::Conflict)?;
    if message.sender_seat_id != HOST_RULE_ACTOR || original.message_id != message_id ||
        original.phase != "APPLIED" || original.previous_revision != 0 ||
        original.revision != 1 || original.result_state != "PENDING" {
        return Err(InboxError::Denied);
    }
    let raw = unhex_native(&original.request_hex)?;
    let frame = raw.strip_suffix(b"\n").ok_or(InboxError::Denied)?;
    let text = std::str::from_utf8(frame).map_err(|_| InboxError::Denied)?;
    let parsed = Parser::parse(text)?;
    if parsed.canonical().as_bytes() != frame { return Err(InboxError::Denied); }
    let Json::Object(fields) = parsed else { return Err(InboxError::Denied); };
    if fields.len() != 13 || internal_text(&fields, "schema")? != "gogoke.37.host-rule-inbox.v1" ||
        internal_text(&fields, "actor")? != HOST_RULE_ACTOR ||
        internal_text(&fields, "operation")? != "enqueue" ||
        internal_text(&fields, "domainId")? != domain ||
        internal_text(&fields, "messageId")? != message_id { return Err(InboxError::Denied); }
    let escalation_request = internal_text(&fields, "escalationRequestId")?;
    let trigger = internal_text(&fields, "triggerId")?;
    let cause = internal_text(&fields, "causeEventId")?;
    let source = internal_text(&fields, "sourceSeatId")?;
    let destination = internal_text(&fields, "destinationSeatId")?;
    let policy_revision = internal_text(&fields, "policyRevision")?
        .parse::<i64>().map_err(|_| InboxError::Denied)?;
    let route_revision = internal_text(&fields, "routeRevision")?
        .parse::<i64>().map_err(|_| InboxError::Denied)?;
    let body = internal_text(&fields, "body")?;
    if [escalation_request.as_str(), trigger.as_str(), cause.as_str(), source.as_str(),
        destination.as_str()].iter().any(|value| !valid_id(value)) ||
        policy_revision < 1 || route_revision < 1 || route_revision > policy_revision ||
        body.is_empty() || message.seat_id != destination || message.body != body {
        return Err(InboxError::Denied);
    }
    let historical = HistoricalHost { domain: domain.into(), escalation_request,
        trigger, cause, source, destination, policy_revision, route_revision,
        body, ids, message };
    if identity_fields(&historical.fields()) != historical.ids ||
        original_request_fields(&historical.fields(), &historical.ids, "enqueue", None) != raw {
        return Err(InboxError::Denied);
    }
    let intent = Statement::prepare(db.as_ptr(),
        "SELECT request_id,from_seat_id,to_seat_id,reason,state FROM
           main.gogoke_v37_seat_policy_escalations
          WHERE domain_id=?1 AND trigger_id=?2")?;
    intent.bind_text(1, domain)?;
    intent.bind_text(2, &historical.trigger)?;
    if !intent.step_row()? || intent.column_text(0)? != historical.escalation_request ||
        intent.column_text(1)? != historical.source ||
        intent.column_text(2)? != historical.destination ||
        intent.column_text(3)? != "REJECT_CAP" ||
        !matches!(intent.column_text(4)?.as_str(), "INTENT" | "UNKNOWN" | "DELIVERED") ||
        intent.step_row()? { return Err(InboxError::Denied); }
    let event = Statement::prepare(db.as_ptr(),
        "SELECT operation,target_id,policy_revision,state,detail FROM
           main.gogoke_v37_seat_policy_events WHERE domain_id=?1 AND event_id=?2")?;
    event.bind_text(1, domain)?;
    event.bind_text(2, &historical.escalation_request)?;
    if !event.step_row()? || event.column_text(0)? != "escalate" ||
        event.column_text(1)? != historical.trigger ||
        event.column_text(2)? != historical.policy_revision.to_string() ||
        event.column_text(3)? != "INTENT" ||
        event.column_text(4)? != historical.cause || event.step_row()? {
        return Err(InboxError::Denied);
    }
    Ok(historical)
}

fn observed_codex_turn(db: &VerifiedDatabaseConnection<'_>, historical: &HistoricalHost,
    target: &HostDeliveryTarget<'_>, record: &crate::store::session_transport::StdinJournalRecord,
    turn_id: &str) -> Result<(), InboxError> {
    let step_id = format!("send-{}",
        &crate::store::digest::sha256_hex(&record.request_bytes)[..40]);
    let source = Statement::prepare(db.as_ptr(),
        "SELECT s.command_hex,hex(r.raw_bytes),s.open_request_id FROM
           main.gogoke_v37_rpc_steps s
           JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
             AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
             AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
             AND r.domain_id=s.domain_id AND r.session_id=s.session_id
             AND r.generation=s.generation
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=s.process_operation_id AND c.domain_id=s.domain_id
             AND c.generation=s.generation AND c.ticket=s.ticket
             AND c.custodian_nonce=s.custodian_nonce
          WHERE s.domain_id=?1 AND s.session_id=?2 AND s.process_operation_id=?3
            AND s.generation=?4 AND s.ticket=?5 AND s.custodian_nonce=?6
            AND s.step_id=?7 AND s.phase='OBSERVED' AND s.requires_response=1
            AND r.state='NO_EVENT' AND r.no_event_reason='CODEX_RPC_RESPONSE'")?;
    for (index, value) in [historical.domain.as_str(), target.session_id,
        record.process_operation_id.as_str(), target.generation, target.ticket,
        record.custodian_nonce.as_str(), step_id.as_str()].iter().enumerate() {
        source.bind_text((index + 1) as i32, value)?;
    }
    if !source.step_row()? { return Err(InboxError::Unknown); }
    let command_bytes = unhex_native(&source.column_text(0)?)?;
    let response_bytes = unhex_native(&source.column_text(1)?)?;
    let open_request_id = source.column_text(2)?;
    if source.step_row()? { return Err(InboxError::Conflict); }
    drop(source);
    let (rpc_id, command) = codex_rpc::decode_stored_turn_start(&command_bytes)
        .map_err(InboxError::Codec)?;
    let codex_rpc::Command::TurnStart {thread_id, text, ..} = &command else {
        return Err(InboxError::Denied);
    };
    if text != &historical.body { return Err(InboxError::Denied); }
    let original_thread = crate::store::session_transport::rpc_journal::observed_thread_id(
        db, &historical.domain, target.session_id, &record.process_operation_id,
        target.generation, &open_request_id, target.ticket, &record.custodian_nonce)
        .map_err(|error| InboxError::InvalidEvidence(format!("H original thread: {error:?}")))?;
    if thread_id != &original_thread { return Err(InboxError::Denied); }
    match codex_rpc::decode(&response_bytes, Some((&rpc_id, &command)))
        .map_err(InboxError::Codec)? {
        codex_rpc::Reply::Turn {turn_id: observed, status:
            codex_rpc::TurnStatus::InProgress | codex_rpc::TurnStatus::Completed, ..}
            if observed == turn_id => Ok(()),
        _ => Err(InboxError::Denied),
    }
}

/// Read H's original ordinary send and its observed TurnStart result through
/// the custody-bound H journal. Historical E/C records bind the old request
/// even when Owner later changes the route; no new send is authorized here.
pub(crate) fn settle_host_turn_start_observed(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, domain: &str, message_id: &str,
    target: &HostDeliveryTarget<'_>)
    -> Result<StoredOperation, InboxError> {
    if !target_valid(target) { return Err(InboxError::Invalid("host delivery target")); }
    transact(db, |db| {
        let historical = historical_host(db, owner, domain, message_id)?;
        let ids = &historical.ids;
        let bytes = original_request_fields(&historical.fields(), ids, "deliver", Some(target));
        let message = &historical.message;
        let prior = replay(db, domain, &ids.delivery_request_id,
            message_id, &bytes)?.ok_or(InboxError::Conflict)?;
        if prior.phase == "APPLIED" { return Ok(prior); }
        if prior.phase != "UNKNOWN" || message.state != "UNKNOWN" ||
            message.revision != prior.revision || !message.turn_id.is_empty() ||
            !message.generation.is_empty() { return Err(InboxError::Conflict); }
        let key = StdinJournalKey { domain_id: domain,
            request_id: &ids.send_request_id, session_id: target.session_id,
            ticket: target.ticket, generation: target.generation };
        let record = read_stdin_journal(db, &key)
            .map_err(|error| InboxError::InvalidEvidence(format!("H host send: {error:?}")))?
            .ok_or(InboxError::Unknown)?;
        if record.state != JournalState::Receipted ||
            record.receipt_status != Some(V37Status::Applied) {
            return Err(InboxError::Unknown);
        }
        let request = decode_request(&record.request_bytes)
            .map_err(|error| InboxError::InvalidEvidence(format!("H original send: {error:?}")))?;
        if request.family != "K-SESSION" || request.operation != "send" ||
            request.domain_id != domain ||
            request.request_id != ids.send_request_id ||
            request.target_id != target.session_id || request.payload.len() != 2 ||
            payload_text(&request, "body")? != historical.body ||
            payload_text(&request, "generation")? != target.generation {
            return Err(InboxError::Denied);
        }
        // H rechecks the exact current seat binding independently of the
        // request's body. A model/User cannot select another recipient session.
        original_target(db, domain, &historical.destination, target, true)?;
        let receipt_bytes = record.receipt_bytes.as_deref().ok_or(InboxError::Unknown)?;
        let receipt = decode_receipt(receipt_bytes)
            .map_err(|error| InboxError::InvalidEvidence(format!("H send receipt: {error:?}")))?;
        if receipt.request_id != ids.send_request_id || receipt.operation != "send" ||
            receipt.target_id != target.session_id || receipt.status != V37Status::Applied {
            return Err(InboxError::Denied);
        }
        let result = receipt.into_result();
        if !matches!(result.get(&JsonString::from_str("createdTurn")), Some(Json::Bool(true))) {
            return Err(InboxError::Unknown);
        }
        let turn_id = match result.get(&JsonString::from_str("turnId")) {
            Some(Json::String(value)) => value.to_well_formed_string().ok_or(InboxError::Denied)?,
            _ => return Err(InboxError::Unknown),
        };
        let receipt_id = match result.get(&JsonString::from_str("receiptId")) {
            Some(Json::String(value)) => value.to_well_formed_string().ok_or(InboxError::Denied)?,
            _ => return Err(InboxError::Unknown),
        };
        if turn_id.is_empty() || !valid_id(&receipt_id) { return Err(InboxError::Denied); }
        observed_codex_turn(db, &historical, target, &record, &turn_id)?;
        let used = Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_inbox_operations
              WHERE domain_id=?1 AND native_receipt_id=?2 AND phase='APPLIED' LIMIT 1")?;
        used.bind_text(1, domain)?;
        used.bind_text(2, &receipt_id)?;
        if used.step_row()? { return Err(InboxError::Conflict); }
        let row = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_inbox_messages SET state='DELIVERED',turn_id=?1,
              generation=?2 WHERE domain_id=?3 AND message_id=?4 AND state='UNKNOWN'
              AND revision=?5 AND turn_id='' AND generation=''")?;
        for (index, value) in [turn_id.as_str(), target.generation, domain,
            ids.message_id.as_str(), &prior.revision.to_string()].iter().enumerate() {
            row.bind_text((index + 1) as i32, value)?;
        }
        row.step_done()?;
        require_one_change(db)?;
        let row = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_inbox_operations SET phase='APPLIED',result_state='DELIVERED',
              native_receipt_id=?1 WHERE domain_id=?2 AND request_id=?3 AND phase='UNKNOWN'
              AND revision=?4")?;
        for (index, value) in [receipt_id.as_str(), domain,
            ids.delivery_request_id.as_str(), &prior.revision.to_string()].iter().enumerate() {
            row.bind_text((index + 1) as i32, value)?;
        }
        row.step_done()?;
        require_one_change(db)?;
        read_operation(db, domain, &ids.delivery_request_id)?.ok_or(InboxError::Unknown)
    })
}

#[cfg(all(test, windows))]
#[path = "host_rule_tests.rs"]
mod tests;
