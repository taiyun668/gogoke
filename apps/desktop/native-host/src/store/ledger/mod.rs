//! A.1 ledger storage on the product's existing verified SQLite connection.
//! Legacy event bodies stay in orchestration_events. V37 updates and both
//! histories share one cursor on the product's verified SQLite connection.
//! These storage functions do not authenticate a caller: native H must bind
//! the session and reader identity before invoking them.

use super::atomic::{
    exec, require_canonical_json, AtomicError, Json, JsonString, Parser, Statement,
};
use super::same_open::VerifiedDatabaseConnection;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LedgerPosition {
    pub(crate) epoch: String,
    pub(crate) cursor: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionPurpose {
    Work,
    Handoff,
    SideChat,
    FormalReview,
}

impl SessionPurpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::Work => "WORK",
            Self::Handoff => "HANDOFF",
            Self::SideChat => "SIDE_CHAT",
            Self::FormalReview => "FORMAL_REVIEW",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Tier {
    Project,
    Seat,
    Session,
    Side,
    Global,
}

impl Tier {
    fn as_str(self) -> &'static str {
        match self {
            Self::Project => "PROJECT",
            Self::Seat => "SEAT",
            Self::Session => "SESSION",
            Self::Side => "SIDE",
            Self::Global => "GLOBAL",
        }
    }

    fn parse(value: &str) -> Result<Self, AtomicError> {
        match value {
            "PROJECT" => Ok(Self::Project),
            "SEAT" => Ok(Self::Seat),
            "SESSION" => Ok(Self::Session),
            "SIDE" => Ok(Self::Side),
            "GLOBAL" => Ok(Self::Global),
            _ => Err(AtomicError::DurabilityContractFailed(format!(
                "unknown ledger tier: {value}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionRegistration {
    pub(crate) domain_id: String,
    pub(crate) seat_id: String,
    pub(crate) session_id: String,
    pub(crate) purpose: SessionPurpose,
    pub(crate) side_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EventInput {
    pub(crate) event_id: String,
    pub(crate) source_epoch: String,
    pub(crate) source_cursor: String,
    pub(crate) domain_id: String,
    pub(crate) seat_id: String,
    pub(crate) session_id: String,
    pub(crate) tier: Tier,
    pub(crate) side_id: Option<String>,
    pub(crate) occurred_at: String,
    /// Canonical ACP session/update object, produced by the adapter normalizer.
    pub(crate) update_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LedgerEvent {
    pub(crate) cursor: u64,
    pub(crate) input: EventInput,
}

/// A reader is always an already-registered native session. Formal reviews
/// have no ledger read path. Secretary access is deliberately a separate API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Reader {
    pub(crate) domain_id: String,
    pub(crate) seat_id: String,
    pub(crate) session_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EventPage {
    pub(crate) position: LedgerPosition,
    pub(crate) events: Vec<LedgerEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Subscription {
    pub(crate) id: String,
    pub(crate) reader_session_id: String,
    pub(crate) epoch: String,
    pub(crate) cursor: u64,
    pub(crate) revision: u64,
    pub(crate) active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SubscriptionPage {
    pub(crate) subscription: Subscription,
    pub(crate) events: Vec<LedgerEvent>,
}

fn required(value: &str, field: &'static str) -> Result<(), AtomicError> {
    if value.is_empty() || value.len() > 4096 || value.contains('\0') {
        Err(AtomicError::InvalidRecord(field))
    } else {
        Ok(())
    }
}

fn cursor(value: &str) -> Result<u64, AtomicError> {
    value.parse::<u64>().map_err(|error| {
        AtomicError::DurabilityContractFailed(format!("invalid ledger cursor {value}: {error}"))
    })
}

fn registered(
    connection: &VerifiedDatabaseConnection<'_>,
    session_id: &str,
) -> Result<Option<SessionRegistration>, AtomicError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT domain_id, seat_id, session_id, purpose, COALESCE(side_id, '')
         FROM v37_ledger_session WHERE session_id = ?",
    )?;
    statement.bind_text(1, session_id)?;
    if !statement.step_row()? {
        return Ok(None);
    }
    let purpose = match statement.column_text(3)?.as_str() {
        "WORK" => SessionPurpose::Work,
        "HANDOFF" => SessionPurpose::Handoff,
        "SIDE_CHAT" => SessionPurpose::SideChat,
        "FORMAL_REVIEW" => SessionPurpose::FormalReview,
        value => {
            return Err(AtomicError::DurabilityContractFailed(format!(
                "unknown session purpose: {value}"
            )))
        }
    };
    let side = statement.column_text(4)?;
    let result = SessionRegistration {
        domain_id: statement.column_text(0)?,
        seat_id: statement.column_text(1)?,
        session_id: statement.column_text(2)?,
        purpose,
        side_id: if side.is_empty() { None } else { Some(side) },
    };
    if statement.step_row()? {
        return Err(AtomicError::DurabilityContractFailed(
            "duplicate session registration".into(),
        ));
    }
    Ok(Some(result))
}

/// Called by native H only after a fresh session has been admitted. Replaying
/// an identical registration is safe; changing purpose or binding is refused.
pub(crate) fn register_session(
    connection: &mut VerifiedDatabaseConnection<'_>,
    registration: &SessionRegistration,
) -> Result<(), AtomicError> {
    for (value, field) in [
        (&registration.domain_id, "domainId"),
        (&registration.seat_id, "seatId"),
        (&registration.session_id, "sessionId"),
    ] {
        required(value, field)?;
    }
    if (registration.purpose == SessionPurpose::SideChat) != registration.side_id.is_some() {
        return Err(AtomicError::InvalidRecord("sideId"));
    }
    if let Some(side) = &registration.side_id {
        required(side, "sideId")?;
    }
    if let Some(existing) = registered(connection, &registration.session_id)? {
        return if existing == *registration {
            Ok(())
        } else {
            Err(AtomicError::OperationConflict)
        };
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        "INSERT INTO v37_ledger_session (domain_id, seat_id, session_id, purpose, side_id)
         VALUES (?, ?, ?, ?, ?)",
    )?;
    statement.bind_text(1, &registration.domain_id)?;
    statement.bind_text(2, &registration.seat_id)?;
    statement.bind_text(3, &registration.session_id)?;
    statement.bind_text(4, registration.purpose.as_str())?;
    if let Some(side) = &registration.side_id {
        statement.bind_text(5, side)?;
    }
    statement.step_done()
}

/// L0 supplies the verified old thread identity. An unbound old row stays
/// unreadable; storage never guesses a seat from its event payload.
pub(crate) fn bind_legacy_thread(
    connection: &mut VerifiedDatabaseConnection<'_>,
    thread_id: &str,
    registration: &SessionRegistration,
) -> Result<(), AtomicError> {
    required(thread_id, "threadId")?;
    if registration.purpose != SessionPurpose::Work
        || registered(connection, &registration.session_id)?.as_ref() != Some(registration)
    {
        return Err(AtomicError::InvalidRecord("legacy session binding"));
    }
    let existing = Statement::prepare(
        connection.as_ptr(),
        "SELECT domain_id, seat_id, session_id FROM v37_ledger_legacy_binding WHERE thread_id = ?",
    )?;
    existing.bind_text(1, thread_id)?;
    if existing.step_row()? {
        return if existing.column_text(0)? == registration.domain_id
            && existing.column_text(1)? == registration.seat_id
            && existing.column_text(2)? == registration.session_id
        {
            Ok(())
        } else {
            Err(AtomicError::OperationConflict)
        };
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        "INSERT INTO v37_ledger_legacy_binding (thread_id, domain_id, seat_id, session_id)
         VALUES (?, ?, ?, ?)",
    )?;
    statement.bind_text(1, thread_id)?;
    statement.bind_text(2, &registration.domain_id)?;
    statement.bind_text(3, &registration.seat_id)?;
    statement.bind_text(4, &registration.session_id)?;
    statement.step_done()
}

/// D calls this in its side-chat deletion transaction. Referenced lead
/// events live outside this tier and are never touched.
pub(crate) fn delete_side_events(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    side_id: &str,
) -> Result<(), AtomicError> {
    required(domain_id, "domainId")?;
    required(side_id, "sideId")?;
    // Preserve the source stream's terminal cursor before removing its
    // projected rows.  A deleted side stream is a durable tombstone: it may
    // not be resumed with a cursor that would hide the deletion.
    let mark = Statement::prepare(
        connection.as_ptr(),
        "UPDATE v37_ledger_source_stream
         SET state = 'TOMBSTONED'
         WHERE EXISTS (
             SELECT 1 FROM v37_ledger_index AS i
             WHERE i.source_kind = 'v37' AND i.domain_id = ?
               AND i.tier = 'SIDE' AND i.side_id = ?
               AND i.session_id = v37_ledger_source_stream.session_id
               AND i.source_epoch = v37_ledger_source_stream.source_epoch
         )",
    )?;
    mark.bind_text(1, domain_id)?;
    mark.bind_text(2, side_id)?;
    mark.step_done()?;
    let statement = Statement::prepare(
        connection.as_ptr(),
        "DELETE FROM v37_ledger_index WHERE source_kind = 'v37'
         AND domain_id = ? AND tier = 'SIDE' AND side_id = ?",
    )?;
    statement.bind_text(1, domain_id)?;
    statement.bind_text(2, side_id)?;
    statement.step_done()
}

fn read_event(statement: &Statement) -> Result<LedgerEvent, AtomicError> {
    let side = statement.column_text(8)?;
    let update_json = if statement.column_text(11)? == "legacy" {
        let mut meta = BTreeMap::new();
        meta.insert(
            JsonString::from("legacyEventType"),
            Json::String(statement.column_text(12)?.into()),
        );
        meta.insert(
            JsonString::from("legacyEventId"),
            Json::String(statement.column_text(1)?.into()),
        );
        meta.insert(
            JsonString::from("legacyPayload"),
            Parser::parse(&statement.column_text(13)?)?,
        );
        let mut update = BTreeMap::new();
        update.insert(
            JsonString::from("sessionUpdate"),
            Json::String("session_info_update".into()),
        );
        update.insert(JsonString::from("_meta"), Json::Object(meta));
        Json::Object(update).canonical()
    } else {
        statement.column_text(10)?
    };
    Ok(LedgerEvent {
        cursor: cursor(&statement.column_text(0)?)?,
        input: EventInput {
            event_id: statement.column_text(1)?,
            source_cursor: statement.column_text(2)?,
            source_epoch: statement.column_text(3)?,
            domain_id: statement.column_text(4)?,
            seat_id: statement.column_text(5)?,
            session_id: statement.column_text(6)?,
            tier: Tier::parse(&statement.column_text(7)?)?,
            side_id: if side.is_empty() { None } else { Some(side) },
            occurred_at: statement.column_text(9)?,
            update_json,
        },
    })
}

const EVENT_COLUMNS: &str = "i.cursor, i.source_event_id,
    COALESCE(i.source_cursor, CAST(e.sequence AS TEXT)),
    COALESCE(i.source_epoch, (SELECT epoch FROM v37_ledger_meta WHERE singleton = 1)),
    COALESCE(i.domain_id, b.domain_id), COALESCE(i.seat_id, b.seat_id),
    COALESCE(i.session_id, b.session_id), COALESCE(i.tier, 'SEAT'),
    COALESCE(i.side_id, ''), COALESCE(i.occurred_at, e.occurred_at),
    COALESCE(i.update_json, ''), i.source_kind,
    COALESCE(e.event_type, ''), COALESCE(e.payload_json, '')";
const EVENT_SOURCE: &str = "FROM v37_ledger_index AS i
    LEFT JOIN orchestration_events AS e
      ON i.source_kind = 'legacy' AND e.event_id = i.source_event_id
    LEFT JOIN v37_ledger_legacy_binding AS b
      ON i.source_kind = 'legacy' AND b.thread_id = e.stream_id";

fn existing_event(
    connection: &VerifiedDatabaseConnection<'_>,
    event_id: &str,
) -> Result<Option<LedgerEvent>, AtomicError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        &format!(
            "SELECT {EVENT_COLUMNS} {EVENT_SOURCE}
                  WHERE i.source_event_id = ? AND i.source_kind = 'v37'"
        ),
    )?;
    statement.bind_text(1, event_id)?;
    if !statement.step_row()? {
        return Ok(None);
    }
    Ok(Some(read_event(&statement)?))
}

/// Append one normalized update. The source ID is the idempotency key; a
/// replay with different bytes or labels conflicts. Caller serializes writes
/// on the same verified connection and never exposes this as raw IPC.
pub(crate) fn record(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &EventInput,
) -> Result<LedgerEvent, AtomicError> {
    for (value, field) in [
        (&input.event_id, "eventId"),
        (&input.source_epoch, "sourceEpoch"),
        (&input.source_cursor, "sourceCursor"),
        (&input.domain_id, "domainId"),
        (&input.seat_id, "seatId"),
        (&input.session_id, "sessionId"),
        (&input.occurred_at, "occurredAt"),
    ] {
        required(value, field)?;
    }
    if input
        .source_cursor
        .parse::<u64>()
        .ok()
        .filter(|number| number.to_string() == input.source_cursor)
        .is_none()
    {
        return Err(AtomicError::InvalidRecord("sourceCursor"));
    }
    require_canonical_json(input.update_json.as_bytes(), "session/update")?;
    let Json::Object(update) = Parser::parse(&input.update_json)? else {
        return Err(AtomicError::InvalidRecord("session/update object"));
    };
    let Some(Json::String(kind)) = update.get(&JsonString::from("sessionUpdate")) else {
        return Err(AtomicError::InvalidRecord("sessionUpdate"));
    };
    if !matches!(
        kind.to_well_formed_string().as_deref(),
        Some(
            "user_message_chunk"
                | "agent_message_chunk"
                | "agent_thought_chunk"
                | "tool_call"
                | "tool_call_update"
                | "plan"
                | "available_commands_update"
                | "current_mode_update"
                | "config_option_update"
                | "session_info_update"
                | "usage_update"
        )
    ) {
        return Err(AtomicError::InvalidRecord("sessionUpdate"));
    }
    let session = registered(connection, &input.session_id)?
        .ok_or(AtomicError::InvalidRecord("unregistered session"))?;
    if session.domain_id != input.domain_id || session.seat_id != input.seat_id {
        return Err(AtomicError::OperationConflict);
    }
    if input.tier == Tier::Side {
        if session.purpose != SessionPurpose::SideChat || session.side_id != input.side_id {
            return Err(AtomicError::InvalidRecord("side-chat tier"));
        }
    } else if input.side_id.is_some() || session.purpose == SessionPurpose::SideChat {
        return Err(AtomicError::InvalidRecord("side-chat event"));
    }
    if session.purpose == SessionPurpose::FormalReview && input.tier == Tier::Global {
        return Err(AtomicError::InvalidRecord("formal review global tier"));
    }
    if let Some(existing) = existing_event(connection, &input.event_id)? {
        return if existing.input == *input {
            Ok(existing)
        } else {
            Err(AtomicError::OperationConflict)
        };
    }
    let source_cursor = cursor(&input.source_cursor)?;
    if source_cursor > i64::MAX as u64 {
        return Err(AtomicError::InvalidRecord("sourceCursor"));
    }
    let stream = Statement::prepare(
        connection.as_ptr(),
        "SELECT last_cursor, state FROM v37_ledger_source_stream
         WHERE session_id = ? AND source_epoch = ?",
    )?;
    stream.bind_text(1, &input.session_id)?;
    stream.bind_text(2, &input.source_epoch)?;
    let stream_exists = stream.step_row()?;
    if stream_exists {
        let last = cursor(&stream.column_text(0)?)?;
        let state = stream.column_text(1)?;
        if state != "ACTIVE" || source_cursor != last.saturating_add(1) {
            return Err(AtomicError::DurabilityContractFailed(format!(
                "A.1 source stream violation: session={} epoch={} expected={} received={} state={state}",
                input.session_id,
                input.source_epoch,
                last.saturating_add(1),
                source_cursor,
            )));
        }
    } else if source_cursor != 1 {
        return Err(AtomicError::DurabilityContractFailed(format!(
            "A.1 source stream gap: session={} epoch={} expected=1 received={source_cursor}",
            input.session_id, input.source_epoch
        )));
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        "INSERT INTO v37_ledger_index
         (source_event_id, source_kind, source_cursor, source_epoch, domain_id, seat_id,
          session_id, tier, side_id, occurred_at, update_json)
         VALUES (?, 'v37', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )?;
    statement.bind_text(1, &input.event_id)?;
    statement.bind_text(2, &input.source_cursor)?;
    statement.bind_text(3, &input.source_epoch)?;
    statement.bind_text(4, &input.domain_id)?;
    statement.bind_text(5, &input.seat_id)?;
    statement.bind_text(6, &input.session_id)?;
    statement.bind_text(7, input.tier.as_str())?;
    if let Some(side) = &input.side_id {
        statement.bind_text(8, side)?;
    }
    statement.bind_text(9, &input.occurred_at)?;
    statement.bind_text(10, &input.update_json)?;
    statement.step_done()?;
    if stream_exists {
        let update = Statement::prepare(
            connection.as_ptr(),
            "UPDATE v37_ledger_source_stream SET last_cursor = ?
             WHERE session_id = ? AND source_epoch = ? AND state = 'ACTIVE'",
        )?;
        update.bind_i64(1, source_cursor as i64)?;
        update.bind_text(2, &input.session_id)?;
        update.bind_text(3, &input.source_epoch)?;
        update.step_done()?;
    } else {
        let insert = Statement::prepare(
            connection.as_ptr(),
            "INSERT INTO v37_ledger_source_stream
             (session_id, source_epoch, last_cursor, state)
             VALUES (?, ?, ?, 'ACTIVE')",
        )?;
        insert.bind_text(1, &input.session_id)?;
        insert.bind_text(2, &input.source_epoch)?;
        insert.bind_i64(3, source_cursor as i64)?;
        insert.step_done()?;
    }
    existing_event(connection, &input.event_id)?
        .ok_or_else(|| AtomicError::DurabilityContractFailed("appended event missing".into()))
}

/// Read only rows permitted by the persisted reader session. Cursor gaps are
/// expected: hidden rows retain their global positions and are never emitted.
pub(crate) fn query(
    connection: &VerifiedDatabaseConnection<'_>,
    reader: &Reader,
    after: &LedgerPosition,
    limit: u32,
) -> Result<EventPage, AtomicError> {
    let session = registered(connection, &reader.session_id)?
        .ok_or(AtomicError::InvalidRecord("unregistered reader"))?;
    if session.domain_id != reader.domain_id || session.seat_id != reader.seat_id {
        return Err(AtomicError::OperationConflict);
    }
    if session.purpose == SessionPurpose::FormalReview {
        return Err(AtomicError::InvalidRecord(
            "formal review cannot read ledger",
        ));
    }
    let position = recover(connection)?;
    if after.epoch != position.epoch || after.cursor > position.cursor {
        return Err(AtomicError::OperationConflict);
    }
    if limit == 0 || limit > 1000 {
        return Err(AtomicError::InvalidRecord("limit"));
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        &format!(
            "SELECT {EVENT_COLUMNS} {EVENT_SOURCE}
         WHERE i.cursor > ? AND COALESCE(i.domain_id, b.domain_id) = ?
           AND (i.source_kind = 'v37' OR b.thread_id IS NOT NULL)
           AND (COALESCE(i.tier, 'SEAT') = 'PROJECT' OR
                (COALESCE(i.tier, 'SEAT') = 'SEAT' AND
                    (? = 'SIDE_CHAT' OR COALESCE(i.seat_id, b.seat_id) = ?)) OR
                (i.tier = 'SESSION' AND (? = 'SIDE_CHAT' OR i.session_id = ?)) OR
                (i.tier = 'SIDE' AND i.side_id = ?))
         ORDER BY i.cursor LIMIT ?"
        ),
    )?;
    statement.bind_i64(1, after.cursor as i64)?;
    statement.bind_text(2, &reader.domain_id)?;
    statement.bind_text(3, session.purpose.as_str())?;
    statement.bind_text(4, &reader.seat_id)?;
    statement.bind_text(5, session.purpose.as_str())?;
    statement.bind_text(6, &reader.session_id)?;
    statement.bind_text(7, session.side_id.as_deref().unwrap_or(""))?;
    statement.bind_i64(8, i64::from(limit))?;
    let mut events = Vec::new();
    while statement.step_row()? {
        events.push(read_event(&statement)?);
    }
    Ok(EventPage { position, events })
}

/// Privileged secretary route. Native H must supply a verified global seat;
/// project and side readers must never be routed to this function.
pub(crate) fn query_global(
    connection: &VerifiedDatabaseConnection<'_>,
    after: &LedgerPosition,
    limit: u32,
) -> Result<EventPage, AtomicError> {
    let position = recover(connection)?;
    if after.epoch != position.epoch || after.cursor > position.cursor {
        return Err(AtomicError::OperationConflict);
    }
    if limit == 0 || limit > 1000 {
        return Err(AtomicError::InvalidRecord("limit"));
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        &format!(
            "SELECT {EVENT_COLUMNS} {EVENT_SOURCE}
             WHERE i.cursor > ? AND (i.source_kind = 'v37' OR b.thread_id IS NOT NULL)
             ORDER BY i.cursor LIMIT ?"
        ),
    )?;
    statement.bind_i64(1, after.cursor as i64)?;
    statement.bind_i64(2, i64::from(limit))?;
    let mut events = Vec::new();
    while statement.step_row()? {
        events.push(read_event(&statement)?);
    }
    Ok(EventPage { position, events })
}

fn subscription(
    connection: &VerifiedDatabaseConnection<'_>,
    id: &str,
) -> Result<Option<Subscription>, AtomicError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT subscription_id, reader_id, epoch, cursor, revision, state
         FROM v37_ledger_subscription WHERE subscription_id = ?",
    )?;
    statement.bind_text(1, id)?;
    if !statement.step_row()? {
        return Ok(None);
    }
    let result = Subscription {
        id: statement.column_text(0)?,
        reader_session_id: statement.column_text(1)?,
        epoch: statement.column_text(2)?,
        cursor: cursor(&statement.column_text(3)?)?,
        revision: cursor(&statement.column_text(4)?)?,
        active: statement.column_text(5)? == "ACTIVE",
    };
    if statement.step_row()? {
        return Err(AtomicError::DurabilityContractFailed(
            "duplicate subscription".into(),
        ));
    }
    Ok(Some(result))
}

/// A subscription starts from a checked position and is durably pinned to
/// one reader session. Polling it after restart resumes from its stored cursor.
pub(crate) fn subscribe(
    connection: &mut VerifiedDatabaseConnection<'_>,
    reader: &Reader,
    id: &str,
    after: &LedgerPosition,
    limit: u32,
) -> Result<SubscriptionPage, AtomicError> {
    required(id, "subscriptionId")?;
    if subscription(connection, id)?.is_some() {
        return Err(AtomicError::OperationConflict);
    }
    let page = query(connection, reader, after, limit)?;
    let position = if page.events.len() == usize::try_from(limit).unwrap_or(usize::MAX) {
        page.events
            .last()
            .map_or(after.cursor, |event| event.cursor)
    } else {
        page.position.cursor
    };
    let registration = registered(connection, &reader.session_id)?
        .ok_or(AtomicError::InvalidRecord("unregistered reader"))?;
    let kind = if registration.purpose == SessionPurpose::SideChat {
        "SIDE"
    } else {
        "PROJECT"
    };
    let statement = Statement::prepare(
        connection.as_ptr(),
        "INSERT INTO v37_ledger_subscription
         (subscription_id, reader_kind, domain_id, reader_id, cursor, epoch, revision, state)
         VALUES (?, ?, ?, ?, ?, ?, 1, 'ACTIVE')",
    )?;
    statement.bind_text(1, id)?;
    statement.bind_text(2, kind)?;
    statement.bind_text(3, &reader.domain_id)?;
    statement.bind_text(4, &reader.session_id)?;
    statement.bind_i64(5, position as i64)?;
    statement.bind_text(6, &page.position.epoch)?;
    statement.step_done()?;
    Ok(SubscriptionPage {
        subscription: subscription(connection, id)?.ok_or_else(|| {
            AtomicError::DurabilityContractFailed("subscription missing after insert".into())
        })?,
        events: page.events,
    })
}

/// Every poll rechecks the persisted reader binding, purpose and scope.
/// A requested cursor must equal the durable cursor: silent rewind and gap
/// acknowledgement are both refused.
pub(crate) fn resume_subscription(
    connection: &mut VerifiedDatabaseConnection<'_>,
    reader: &Reader,
    id: &str,
    after: &LedgerPosition,
    limit: u32,
) -> Result<SubscriptionPage, AtomicError> {
    let old = subscription(connection, id)?.ok_or(AtomicError::InvalidRecord("subscriptionId"))?;
    if !old.active
        || old.reader_session_id != reader.session_id
        || old.epoch != after.epoch
        || old.cursor != after.cursor
    {
        return Err(AtomicError::OperationConflict);
    }
    let page = query(connection, reader, after, limit)?;
    let position = if page.events.len() == usize::try_from(limit).unwrap_or(usize::MAX) {
        page.events
            .last()
            .map_or(after.cursor, |event| event.cursor)
    } else {
        page.position.cursor
    };
    let statement = Statement::prepare(
        connection.as_ptr(),
        "UPDATE v37_ledger_subscription SET cursor = ?, revision = revision + 1
         WHERE subscription_id = ? AND reader_id = ? AND cursor = ?
           AND revision = ? AND state = 'ACTIVE'",
    )?;
    statement.bind_i64(1, position as i64)?;
    statement.bind_text(2, id)?;
    statement.bind_text(3, &reader.session_id)?;
    statement.bind_i64(4, old.cursor as i64)?;
    statement.bind_i64(5, old.revision as i64)?;
    statement.step_done()?;
    let next = subscription(connection, id)?.ok_or_else(|| {
        AtomicError::DurabilityContractFailed("subscription missing after update".into())
    })?;
    if next.revision != old.revision + 1 || next.cursor != position {
        return Err(AtomicError::OperationConflict);
    }
    Ok(SubscriptionPage {
        subscription: next,
        events: page.events,
    })
}

pub(crate) fn end_subscription(
    connection: &mut VerifiedDatabaseConnection<'_>,
    reader: &Reader,
    id: &str,
    expected_revision: u64,
) -> Result<Subscription, AtomicError> {
    let old = subscription(connection, id)?.ok_or(AtomicError::InvalidRecord("subscriptionId"))?;
    if !old.active
        || old.reader_session_id != reader.session_id
        || old.revision != expected_revision
    {
        return Err(AtomicError::OperationConflict);
    }
    // Validate the current reader and purpose at end as on every read.
    let position = recover(connection)?;
    let _ = query(
        connection,
        reader,
        &LedgerPosition {
            epoch: position.epoch,
            cursor: old.cursor,
        },
        1,
    )?;
    let statement = Statement::prepare(
        connection.as_ptr(),
        "UPDATE v37_ledger_subscription SET state = 'ENDED', revision = revision + 1
         WHERE subscription_id = ? AND reader_id = ? AND revision = ? AND state = 'ACTIVE'",
    )?;
    statement.bind_text(1, id)?;
    statement.bind_text(2, &reader.session_id)?;
    statement.bind_i64(3, expected_revision as i64)?;
    statement.step_done()?;
    let next = subscription(connection, id)?.ok_or_else(|| {
        AtomicError::DurabilityContractFailed("subscription missing after end".into())
    })?;
    if next.active || next.revision != old.revision + 1 {
        return Err(AtomicError::OperationConflict);
    }
    Ok(next)
}

fn scalar(connection: &VerifiedDatabaseConnection<'_>, sql: &str) -> Result<String, AtomicError> {
    let statement = Statement::prepare(connection.as_ptr(), sql)?;
    if !statement.step_row()? {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 ledger scalar row missing".into(),
        ));
    }
    let value = statement.column_text(0)?;
    if statement.step_row()? {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 ledger scalar returned multiple rows".into(),
        ));
    }
    Ok(value)
}

/// The schema script owns one BEGIN IMMEDIATE/COMMIT. On any error the caller
/// must discard the connection; it must never serve it after uncertain COMMIT.
pub(crate) fn initialize_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<LedgerPosition, AtomicError> {
    exec(connection, include_str!("schema.sql"))?;
    recover(connection)
}

/// Recover the durable epoch/cursor and verify every old event still resolves
/// through exactly the original source ID. No second history is consulted.
pub(crate) fn recover(
    connection: &VerifiedDatabaseConnection<'_>,
) -> Result<LedgerPosition, AtomicError> {
    let missing = scalar(
        connection,
        "SELECT COUNT(*) FROM orchestration_events AS e
         LEFT JOIN v37_ledger_index AS i
           ON i.source_event_id = e.event_id AND i.source_kind = 'legacy'
         WHERE i.cursor IS NULL",
    )?;
    let orphaned = scalar(
        connection,
        "SELECT COUNT(*) FROM v37_ledger_index AS i
         LEFT JOIN orchestration_events AS e ON e.event_id = i.source_event_id
         WHERE i.source_kind = 'legacy' AND e.event_id IS NULL",
    )?;
    if missing != "0" || orphaned != "0" {
        return Err(AtomicError::DurabilityContractFailed(format!(
            "A.1 legacy index divergence: missing={missing}, orphaned={orphaned}"
        )));
    }
    let invalid_v37 = scalar(
        connection,
        "SELECT COUNT(*) FROM v37_ledger_index AS i
         LEFT JOIN v37_ledger_session AS s ON s.session_id = i.session_id
         WHERE i.source_kind = 'v37' AND
           (s.session_id IS NULL OR s.domain_id <> i.domain_id OR
            s.seat_id <> i.seat_id OR
            (i.tier = 'SIDE' AND (s.purpose <> 'SIDE_CHAT' OR s.side_id <> i.side_id)) OR
            (i.tier <> 'SIDE' AND (i.side_id IS NOT NULL OR s.purpose = 'SIDE_CHAT')) OR
            (i.tier = 'GLOBAL' AND s.purpose = 'FORMAL_REVIEW'))",
    )?;
    if invalid_v37 != "0" {
        return Err(AtomicError::DurabilityContractFailed(format!(
            "A.1 v37 session/index divergence: {invalid_v37}"
        )));
    }
    let invalid_binding = scalar(
        connection,
        "SELECT COUNT(*) FROM v37_ledger_legacy_binding AS b
         LEFT JOIN v37_ledger_session AS s ON s.session_id = b.session_id
         WHERE s.session_id IS NULL OR s.purpose <> 'WORK' OR
           s.domain_id <> b.domain_id OR s.seat_id <> b.seat_id",
    )?;
    if invalid_binding != "0" {
        return Err(AtomicError::DurabilityContractFailed(format!(
            "A.1 legacy binding divergence: {invalid_binding}"
        )));
    }
    let invalid_stream = scalar(
        connection,
        "SELECT COUNT(*) FROM v37_ledger_source_stream AS s
         WHERE s.last_cursor < 1 OR
           (s.state = 'ACTIVE' AND (
             (SELECT COUNT(*) FROM v37_ledger_index AS i
              WHERE i.source_kind = 'v37' AND i.session_id = s.session_id
                AND i.source_epoch = s.source_epoch) <> s.last_cursor OR
             (SELECT COALESCE(MIN(CAST(i.source_cursor AS INTEGER)), 0)
              FROM v37_ledger_index AS i
              WHERE i.source_kind = 'v37' AND i.session_id = s.session_id
                AND i.source_epoch = s.source_epoch) <> 1 OR
             EXISTS (SELECT 1 FROM v37_ledger_index AS i
              WHERE i.source_kind = 'v37' AND i.session_id = s.session_id
                AND i.source_epoch = s.source_epoch
              GROUP BY i.source_cursor HAVING COUNT(*) <> 1)
           ))",
    )?;
    if invalid_stream != "0" {
        return Err(AtomicError::DurabilityContractFailed(format!(
            "A.1 source stream divergence: {invalid_stream}"
        )));
    }
    let epoch = scalar(
        connection,
        "SELECT epoch FROM v37_ledger_meta WHERE singleton = 1",
    )?;
    if epoch.is_empty() {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 ledger epoch missing".into(),
        ));
    }
    let cursor = scalar(
        connection,
        "SELECT COALESCE((SELECT seq FROM sqlite_sequence
            WHERE name = 'v37_ledger_index'), 0)",
    )?
    .parse::<u64>()
    .map_err(|error| {
        AtomicError::DurabilityContractFailed(format!("A.1 invalid ledger cursor: {error}"))
    })?;
    Ok(LedgerPosition { epoch, cursor })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, open_existing, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch_root() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-v37-ledger-{nonce}"));
        std::fs::create_dir(&path).expect("scratch root");
        path
    }

    fn session(
        domain: &str,
        seat: &str,
        id: &str,
        purpose: SessionPurpose,
        side: Option<&str>,
    ) -> SessionRegistration {
        SessionRegistration {
            domain_id: domain.into(),
            seat_id: seat.into(),
            session_id: id.into(),
            purpose,
            side_id: side.map(str::to_owned),
        }
    }

    fn event_at(
        id: &str,
        session: &SessionRegistration,
        tier: Tier,
        source_cursor: &str,
    ) -> EventInput {
        EventInput {
            event_id: id.into(),
            source_epoch: "source-epoch".into(),
            source_cursor: source_cursor.into(),
            domain_id: session.domain_id.clone(),
            seat_id: session.seat_id.clone(),
            session_id: session.session_id.clone(),
            tier,
            side_id: session.side_id.clone(),
            occurred_at: "2026-09-29T00:00:00Z".into(),
            update_json: r#"{"sessionUpdate":"agent_message_chunk"}"#.into(),
        }
    }

    fn event(id: &str, session: &SessionRegistration, tier: Tier) -> EventInput {
        event_at(id, session, tier, "1")
    }

    #[test]
    fn same_open_append_scope_subscription_and_restart() {
        let _guard = route_b_test_guard();
        let path = scratch_root();
        let root = RootLock::acquire(&path).expect("root");
        let db = path.join("ledger.db");
        let mut connection = create_new(&root, &db).expect("open");
        exec(
            &mut connection,
            "CREATE TABLE orchestration_events
            (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
             occurred_at TEXT, event_type TEXT, payload_json TEXT)",
        )
        .expect("legacy table");
        let start = initialize_schema(&mut connection).expect("schema");
        let lead = session(
            "project-a",
            "lead",
            "lead-session",
            SessionPurpose::Work,
            None,
        );
        let worker = session(
            "project-a",
            "worker",
            "worker-session",
            SessionPurpose::Work,
            None,
        );
        let other = session(
            "project-b",
            "lead",
            "other-session",
            SessionPurpose::Work,
            None,
        );
        let side = session(
            "project-a",
            "owner",
            "side-session",
            SessionPurpose::SideChat,
            Some("side-a"),
        );
        let review = session(
            "project-a",
            "review",
            "review-session",
            SessionPurpose::FormalReview,
            None,
        );
        for item in [&lead, &worker, &other, &side, &review] {
            register_session(&mut connection, item).expect("register");
        }
        assert!(register_session(
            &mut connection,
            &session(
                "project-a",
                "lead",
                "lead-session",
                SessionPurpose::FormalReview,
                None
            )
        )
        .is_err());
        let lead_event = event("lead-private", &lead, Tier::Seat);
        let saved = record(&mut connection, &lead_event).expect("append");
        assert_eq!(record(&mut connection, &lead_event).expect("replay"), saved);
        let mut changed = lead_event.clone();
        changed.update_json = r#"{"sessionUpdate":"usage_update"}"#.into();
        assert!(record(&mut connection, &changed).is_err());
        record(
            &mut connection,
            &event("worker-private", &worker, Tier::Seat),
        )
        .expect("worker");
        record(
            &mut connection,
            &event("other-project", &other, Tier::Project),
        )
        .expect("other");
        record(&mut connection, &event("side-private", &side, Tier::Side)).expect("side");
        record(
            &mut connection,
            &event_at("shared", &lead, Tier::Project, "2"),
        )
        .expect("shared");
        record(
            &mut connection,
            &event_at("global", &lead, Tier::Global, "3"),
        )
        .expect("global");
        record(
            &mut connection,
            &event_at("side-latest", &side, Tier::Side, "2"),
        )
        .expect("latest side");
        let lead_reader = Reader {
            domain_id: lead.domain_id.clone(),
            seat_id: lead.seat_id.clone(),
            session_id: lead.session_id.clone(),
        };
        let worker_reader = Reader {
            domain_id: worker.domain_id.clone(),
            seat_id: worker.seat_id.clone(),
            session_id: worker.session_id.clone(),
        };
        let side_reader = Reader {
            domain_id: side.domain_id.clone(),
            seat_id: side.seat_id.clone(),
            session_id: side.session_id.clone(),
        };
        let review_reader = Reader {
            domain_id: review.domain_id.clone(),
            seat_id: review.seat_id.clone(),
            session_id: review.session_id.clone(),
        };
        assert_eq!(
            query(&connection, &lead_reader, &start, 100)
                .expect("lead query")
                .events
                .len(),
            2
        );
        assert_eq!(
            query(&connection, &worker_reader, &start, 100)
                .expect("worker query")
                .events
                .len(),
            2
        );
        assert_eq!(
            query(&connection, &side_reader, &start, 100)
                .expect("side query")
                .events
                .len(),
            5
        );
        assert!(query(&connection, &review_reader, &start, 100).is_err());
        assert_eq!(
            query_global(&connection, &start, 100)
                .expect("global query")
                .events
                .len(),
            7
        );
        let subscribed =
            subscribe(&mut connection, &lead_reader, "sub-lead", &start, 1).expect("subscribe");
        assert_eq!(subscribed.events.len(), 1);
        assert!(resume_subscription(&mut connection, &lead_reader, "sub-lead", &start, 1).is_err());
        delete_side_events(&mut connection, "project-a", "side-a").expect("delete side entries");
        assert_eq!(
            recover(&connection)
                .expect("high-water after delete")
                .cursor,
            7
        );
        assert_eq!(
            query_global(&connection, &start, 100)
                .expect("remaining")
                .events
                .len(),
            5
        );
        connection.close_checked().expect("close");
        let mut reopened = open_existing(&root, &db).expect("reopen");
        let recovered = recover(&reopened).expect("recover");
        assert_eq!(recovered.cursor, 7);
        assert_eq!(recovered.epoch, start.epoch);
        let next = resume_subscription(
            &mut reopened,
            &lead_reader,
            "sub-lead",
            &LedgerPosition {
                epoch: start.epoch.clone(),
                cursor: subscribed.subscription.cursor,
            },
            100,
        )
        .expect("resume after restart");
        assert_eq!(next.events.len(), 1);
        assert!(
            !end_subscription(
                &mut reopened,
                &lead_reader,
                "sub-lead",
                next.subscription.revision
            )
            .expect("end")
            .active
        );
        reopened.close_checked().expect("close reopened");
    }

    #[test]
    fn legacy_rows_are_read_by_reference_only_after_explicit_binding() {
        let _guard = route_b_test_guard();
        let path = scratch_root();
        let root = RootLock::acquire(&path).expect("root");
        let db = path.join("ledger.db");
        let mut connection = create_new(&root, &db).expect("open");
        exec(
            &mut connection,
            "CREATE TABLE orchestration_events
             (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
              occurred_at TEXT, event_type TEXT, payload_json TEXT)",
        )
        .expect("legacy table");
        let start = initialize_schema(&mut connection).expect("schema");
        let lead = session(
            "project-a",
            "lead",
            "lead-session",
            SessionPurpose::Work,
            None,
        );
        register_session(&mut connection, &lead).expect("register");
        exec(
            &mut connection,
            "INSERT INTO orchestration_events VALUES
             (1, 'old-event', 'old-thread', '2026-09-29T00:00:00Z',
              'thread.message-sent', '{\"role\":\"user\",\"text\":\"hello\"}')",
        )
        .expect("old append and trigger");
        let reader = Reader {
            domain_id: lead.domain_id.clone(),
            seat_id: lead.seat_id.clone(),
            session_id: lead.session_id.clone(),
        };
        assert!(query(&connection, &reader, &start, 100)
            .expect("unbound query")
            .events
            .is_empty());
        bind_legacy_thread(&mut connection, "old-thread", &lead).expect("bind");
        let page = query(&connection, &reader, &start, 100).expect("bound query");
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].input.event_id, "old-event");
        assert!(page.events[0].input.update_json.contains("legacyPayload"));
        assert!(bind_legacy_thread(
            &mut connection,
            "old-thread",
            &session(
                "project-a",
                "worker",
                "worker-session",
                SessionPurpose::Work,
                None
            )
        )
        .is_err());
        connection.close_checked().expect("close");
    }

    #[test]
    fn source_stream_rejects_gap_duplicate_cursor_and_accepts_epoch_rollover() {
        let _guard = route_b_test_guard();
        let path = scratch_root();
        let root = RootLock::acquire(&path).expect("root");
        let db = path.join("ledger.db");
        let mut connection = create_new(&root, &db).expect("open");
        exec(
            &mut connection,
            "CREATE TABLE orchestration_events
             (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
              occurred_at TEXT, event_type TEXT, payload_json TEXT)",
        )
        .expect("legacy table");
        initialize_schema(&mut connection).expect("schema");
        let registration = session(
            "project-a",
            "seat-a",
            "session-a",
            SessionPurpose::Work,
            None,
        );
        register_session(&mut connection, &registration).expect("register");
        record(
            &mut connection,
            &event_at("event-1", &registration, Tier::Seat, "1"),
        )
        .expect("first");
        let gap = event_at("event-3", &registration, Tier::Seat, "3");
        assert!(matches!(
            record(&mut connection, &gap),
            Err(AtomicError::DurabilityContractFailed(message))
                if message.contains("source stream gap")
        ));
        let duplicate_cursor = event_at("event-2", &registration, Tier::Seat, "1");
        assert!(matches!(
            record(&mut connection, &duplicate_cursor),
            Err(AtomicError::DurabilityContractFailed(message))
                if message.contains("source stream violation")
        ));
        record(
            &mut connection,
            &event_at("event-2", &registration, Tier::Seat, "2"),
        )
        .expect("contiguous");
        let rollover = EventInput {
            source_epoch: "source-epoch-2".into(),
            ..event_at("event-epoch-2", &registration, Tier::Seat, "1")
        };
        record(&mut connection, &rollover).expect("epoch rollover");
        assert_eq!(recover(&connection).expect("recover").cursor, 3);
        connection.close_checked().expect("close");
    }
}
