//! E.2 host notification authority for an already committed reject-cap fact.
//! This is a child of policy; it never constructs a model caller or a trigger
//! coordinator receipt. The caller owns the existing Owner transaction.

use super::*;
use crate::store::digest::sha256_hex;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HostEscalationProof {
    domain: String,
    gate: String,
    source: String,
    source_incarnation: String,
    destination: String,
    destination_incarnation: Option<String>,
    cause_event: String,
    cause_fingerprint: String,
    cause_policy_revision: i64,
    current_owner_revision: i64,
    policy_revision: i64,
    route_revision: i64,
    gate_fact: (String, String, String, String, i64, i64, String, i64),
    trigger: String,
    request: String,
    notice: String,
}

impl HostEscalationProof {
    pub(crate) fn domain_id(&self) -> &str { &self.domain }
    pub(crate) fn source_seat_id(&self) -> &str { &self.source }
    pub(crate) fn destination_seat_id(&self) -> &str { &self.destination }
    pub(crate) fn cause_event_id(&self) -> &str { &self.cause_event }
    pub(crate) fn policy_revision(&self) -> i64 { self.policy_revision }
    pub(crate) fn route_revision(&self) -> i64 { self.route_revision }
    pub(crate) fn trigger_id(&self) -> &str { &self.trigger }
    pub(crate) fn request_id(&self) -> &str { &self.request }
    pub(crate) fn notice_body(&self) -> &str { &self.notice }
}

/// Seat IDs cannot be recreated, and their incarnation/layer/parent never
/// change in the native engine. The original create operation is the identity
/// anchor even when the physical process has stopped or the seat is reclaimed.
/// Any future ID reuse requires strengthening the gate cause binding first.
fn original_logical_identity(db: &VerifiedDatabaseConnection<'_>, seat: &Seat)
    -> Result<(), SeatError>
{
    let q = Statement::prepare(db.as_ptr(),
        "SELECT incarnation,layer,COALESCE(parent_seat_id,'')
         FROM main.gogoke_v37_seat_operations
         WHERE domain_id=?1 AND seat_id=?2 AND revision=1 AND generation=1")?;
    q.bind_text(1, &seat.domain_id)?;
    q.bind_text(2, &seat.seat_id)?;
    if !q.step_row()? || q.column_text(0)? != seat.incarnation
        || q.column_text(1)? != seat.layer.sql()
        || q.column_text(2)? != seat.parent_seat_id.as_deref().unwrap_or("")
        || q.step_row()? {
        return Err(SeatError::Denied);
    }
    Ok(())
}

pub(crate) fn observe_host_reject_cap_in_transaction(
    db: &VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer, domain: &str, gate: &str,
) -> Result<Option<HostEscalationProof>, SeatError> {
    check_current_owner(db, owner)?;
    if !valid_id(domain) || !valid_id(gate) {
        return Err(SeatError::Invalid("host reject-cap identity"));
    }
    let fact = gate_row(db, domain, gate)?;
    if fact.6 != "ESCALATION_REQUIRED" || fact.5 < fact.4 { return Ok(None); }
    if fact.4 < 1 || fact.7 < 3 || stage(db, domain)? != fact.2 {
        return Err(SeatError::Denied);
    }
    let revision = head_revision(db, domain)?;
    // This native event is written atomically with gate_decide's cap state.
    // A client statement, pending tool output, or a reason string is no cause.
    let q = Statement::prepare(db.as_ptr(),
        "SELECT e.event_id,e.fingerprint,e.policy_revision,e.detail,g.reason
         FROM main.gogoke_v37_seat_policy_events e
         JOIN main.gogoke_v37_seat_policy_gates g
           ON g.domain_id=e.domain_id AND g.gate_id=e.target_id
         WHERE e.domain_id=?1 AND e.target_id=?2 AND e.operation='gate-decide'
           AND e.state='ESCALATION_REQUIRED'")?;
    q.bind_text(1, domain)?;
    q.bind_text(2, gate)?;
    if !q.step_row()? { return Err(SeatError::Denied); }
    let cause = q.column_text(0)?;
    let cause_fingerprint = q.column_text(1)?;
    let cause_revision = q.column_text(2)?.parse::<i64>()
        .map_err(|_| SeatError::SchemaDrift)?;
    let reason = q.column_text(3)?;
    if !valid_id(&cause) || cause_fingerprint.is_empty() || cause_revision < 1
        || cause_revision > revision || reason.is_empty() || reason.len() > 4096
        || reason != q.column_text(4)? || q.step_row()? {
        return Err(SeatError::Denied);
    }
    let source = read(db, domain, &fact.0)?.ok_or(SeatError::Denied)?;
    original_logical_identity(db, &source)?;
    let route = Statement::prepare(db.as_ptr(),
        "SELECT to_seat_id,revision FROM main.gogoke_v37_seat_policy_routes
         WHERE domain_id=?1 AND from_seat_id=?2 AND reason='REJECT_CAP'")?;
    route.bind_text(1, domain)?;
    route.bind_text(2, &source.seat_id)?;
    if !route.step_row()? { return Err(SeatError::Denied); }
    let destination = route.column_text(0)?;
    let route_revision = route.column_text(1)?.parse::<i64>()
        .map_err(|_| SeatError::SchemaDrift)?;
    if !valid_id(&destination) || destination == source.seat_id
        || (source.layer == Layer::Lead && destination == "OWNER")
        || route_revision < 1 || route_revision > revision || route.step_row()? {
        return Err(SeatError::Denied);
    }
    let destination_incarnation = if destination == "OWNER" { None } else {
        let target = read(db, domain, &destination)?.ok_or(SeatError::Denied)?;
        if target.state == State::Reclaimed { return Err(SeatError::Denied); }
        original_logical_identity(db, &target)?;
        Some(target.incarnation)
    };
    // The same cause keeps the same IDs after route/head changes. A changed
    // Owner policy cannot manufacture another send under a fresh trigger ID.
    let identity = fingerprint(&["host-reject-cap", domain, &cause], b"");
    let digest = sha256_hex(identity.as_bytes());
    let trigger = format!("host-reject-cap-{}", &digest[..40]);
    let request = format!("host-escalate-{}", &digest[..40]);
    // Preserve the Owner snapshot recorded by the original INTENT. A later
    // unrelated grant update changes current admission, not this cause's
    // C bytes/identity. Actual route/cause changes still fail prior_event.
    let prior = Statement::prepare(db.as_ptr(),
        "SELECT operation,target_id,policy_revision,state,detail FROM
         main.gogoke_v37_seat_policy_events WHERE domain_id=?1 AND event_id=?2")?;
    prior.bind_text(1,domain)?;
    prior.bind_text(2,&request)?;
    let intent_revision=if prior.step_row()? {
        let original=prior.column_text(2)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
        if prior.column_text(0)?!="escalate" || prior.column_text(1)?!=trigger
            || prior.column_text(3)?!="INTENT" || prior.column_text(4)?!=cause
            || original<cause_revision || original>revision || prior.step_row()? {
            return Err(SeatError::Denied);
        }
        original
    } else {revision};
    let notice = format!(
        "Host rule REJECT_CAP: gate {gate} reached rejection count {} (cap {}). \
         Source seat {}; destination {destination}; cause event {cause}; \
         Original Owner policy revision {intent_revision}. The gate reason remains a reference \
         in the original E event; this notice carries no model instruction.",
        fact.5, fact.4, source.seat_id);
    Ok(Some(HostEscalationProof {
        domain: domain.into(), gate: gate.into(), source: source.seat_id,
        source_incarnation: source.incarnation, destination, destination_incarnation,
        cause_event: cause, cause_fingerprint, cause_policy_revision: cause_revision,
        current_owner_revision: revision, policy_revision: intent_revision, route_revision, gate_fact: fact,
        trigger, request, notice,
    }))
}

pub(crate) fn revalidate_host_escalation_in_transaction(
    db: &VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer, proof: &HostEscalationProof,
) -> Result<(), SeatError> {
    let current = observe_host_reject_cap_in_transaction(db, owner, &proof.domain, &proof.gate)?;
    if current.as_ref() != Some(proof) { return Err(SeatError::Denied); }
    Ok(())
}

fn host_intent_fingerprint(proof:&HostEscalationProof)->String {
    fingerprint(&["host-escalate", &proof.source, &proof.source_incarnation,
        &proof.trigger, &proof.request, &proof.destination,
        proof.destination_incarnation.as_deref().unwrap_or("OWNER"),
        &proof.cause_event, &proof.cause_fingerprint,
        &proof.policy_revision.to_string(), &proof.route_revision.to_string()],
        proof.notice.as_bytes())
}

/// Read the same original reservation used by begin; no new intent is
/// created. A changed route cannot present an old cause as a fresh notice.
pub(crate) fn read_host_escalation_intent_in_transaction(
    db:&VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,proof:&HostEscalationProof,
)->Result<Option<EscalationIntent>,SeatError> {
    revalidate_host_escalation_in_transaction(db,owner,proof)?;
    let fp=host_intent_fingerprint(proof);
    if let Some(old) = prior_event(db, &proof.domain, &proof.request, "escalate", &fp)? {
        let q = Statement::prepare(db.as_ptr(),
            "SELECT from_seat_id,to_seat_id,reason,state,revision FROM
             main.gogoke_v37_seat_policy_escalations
             WHERE domain_id=?1 AND trigger_id=?2 AND request_id=?3")?;
        q.bind_text(1, &proof.domain)?;
        q.bind_text(2, &proof.trigger)?;
        q.bind_text(3, &proof.request)?;
        if !q.step_row()? || old.target_id != proof.trigger || old.detail != proof.cause_event
            || old.state != "INTENT" || old.policy_revision != proof.policy_revision
            || q.column_text(0)? != proof.source || q.column_text(1)? != proof.destination
            || q.column_text(2)? != "REJECT_CAP" {
            return Err(SeatError::Conflict);
        }
        let state = q.column_text(3)?;
        let revision = q.column_text(4)?.parse::<i64>().map_err(|_| SeatError::SchemaDrift)?;
        if !matches!(state.as_str(), "INTENT" | "UNKNOWN" | "DELIVERED")
            || revision < 1 || q.step_row()? { return Err(SeatError::Conflict); }
        return Ok(Some(EscalationIntent { trigger_id: proof.trigger.clone(),
            from_seat_id: proof.source.clone(), to_seat_id: proof.destination.clone(),
            reason: "REJECT_CAP".into(), state, revision, replayed: true }));
    }
    Ok(None)
}

pub(crate) fn begin_host_escalation_in_transaction(
    db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer, proof: &HostEscalationProof,
) -> Result<EscalationIntent, SeatError> {
    if let Some(original)=read_host_escalation_intent_in_transaction(db,owner,proof)? {
        return Ok(original);
    }
    let fp=host_intent_fingerprint(proof);
    let q = Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_seat_policy_escalations
         (domain_id,trigger_id,request_id,from_seat_id,to_seat_id,reason,state,revision)
         VALUES(?1,?2,?3,?4,?5,'REJECT_CAP','INTENT',1)")?;
    for (index, value) in [&proof.domain, &proof.trigger, &proof.request,
        &proof.source, &proof.destination].iter().enumerate() {
        q.bind_text((index + 1) as i32, value)?;
    }
    q.step_done()?;
    record_event(db, &proof.domain, PolicyEvent { event_id: proof.request.clone(),
        operation: "escalate".into(), target_id: proof.trigger.clone(),
        policy_revision: proof.policy_revision, state: "INTENT".into(),
        detail: proof.cause_event.clone(), replayed: false }, &fp)?;
    // No pipe write occurs here. On any error the caller rolls back the whole
    // owning transaction; UNKNOWN/DELIVERED replay never authorizes delivery.
    Ok(EscalationIntent { trigger_id: proof.trigger.clone(),
        from_seat_id: proof.source.clone(), to_seat_id: proof.destination.clone(),
        reason: "REJECT_CAP".into(), state: "INTENT".into(), revision: 1, replayed: false })
}

#[cfg(all(test, windows))]
#[path = "host_rule_tests.rs"]
mod tests;
