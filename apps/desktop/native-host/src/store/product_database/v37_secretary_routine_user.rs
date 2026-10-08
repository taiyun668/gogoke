//! E.3 USER operations use the existing authenticated Owner configuration
//! envelope. The durable E store remains the sole writer. There is no model,
//! clock, or delivery producer in this module.
use super::*;
use crate::store::seat::{self, SecretaryRoutine, SecretaryRoutineChange,
    SecretaryRoutineCommand, SeatError};
use std::time::{SystemTime, UNIX_EPOCH};

fn key(name: &str) -> JsonString { JsonString::from_str(name) }
fn text(value: &str) -> Json { Json::String(JsonString::from_str(value)) }

fn exact(fields: &BTreeMap<JsonString, Json>, names: &[&str]) -> bool {
    fields.len() == names.len() && names.iter().all(|name| fields.contains_key(&key(name)))
}

fn positive_number(fields: &BTreeMap<JsonString, Json>, name: &'static str) -> Result<i64> {
    let value = super::v37_seat::string_field(fields, name)?;
    value.parse::<i64>().ok().filter(|value| *value > 0)
        .ok_or(OrchestrationError::Invalid(name))
}

fn current_time_ms() -> Result<i64> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|error| OrchestrationError::V37StoreFailure(format!("system clock: {error}")))?;
    i64::try_from(elapsed.as_millis())
        .map_err(|error| OrchestrationError::V37StoreFailure(format!("system clock range: {error}")))
}

fn routine(row: &SecretaryRoutine) -> Json {
    Json::Object(BTreeMap::from([
        (key("routineId"), text(&row.routine_id)),
        (key("seatId"), text(&row.seat_id)),
        (key("incarnation"), text(&row.incarnation)),
        (key("originalText"), text(&row.original_text)),
        (key("sourceOperationId"), text(&row.source_operation_id)),
        (key("sourceEpoch"), text(&row.source_epoch)),
        (key("sourceCursor"), text(&row.source_cursor)),
        (key("scheduleRaw"), text(&row.schedule_raw)),
        (key("timezone"), text(&row.timezone)),
        (key("nextDueMs"), text(&row.next_due_ms.to_string())),
        (key("state"), text(&row.state)),
        (key("revision"), text(&row.revision.to_string())),
        (key("lastOccurrenceId"), text(&row.last_occurrence_id)),
        (key("lastResult"), text(&row.last_result)),
        (key("lastReason"), text(&row.last_reason)),
    ]))
}

fn reply(command: &str, status: &str, fields: impl IntoIterator<Item = (JsonString, Json)>) -> Vec<u8> {
    let mut result = BTreeMap::from([
        (key("schema"), text("gogoke.37.secretary-routines.v1")),
        (key("command"), text(command)),
        (key("status"), text(status)),
    ]);
    result.extend(fields);
    Json::Object(result).canonical().into_bytes()
}

fn change_status(error: &SeatError) -> &'static str {
    match error {
        SeatError::Invalid(_) | SeatError::Denied => "DENIED",
        SeatError::Busy | SeatError::Conflict => "CONFLICT",
        SeatError::Unknown | SeatError::Store(_) | SeatError::Open(_)
        | SeatError::CommitUnknown(_) | SeatError::RollbackUnknown(_)
        | SeatError::HostResourceObservation(_) | SeatError::HostHealthObservation(_)
        | SeatError::InstanceManagement(_) | SeatError::SchemaDrift => "UNKNOWN",
    }
}

impl<'root> ProductDatabase<'root> {
    pub(super) fn dispatch_user_secretary_routine_configuration(&mut self, command: &str,
        fields: &BTreeMap<JsonString, Json>, frame: &[u8]) -> Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        if command == "secretary-routines-read" {
            if !exact(fields, &["schema", "command"])
                && !exact(fields, &["schema", "command", "routineId"]) {
                return Err(OrchestrationError::Invalid("secretary routine read fields"));
            }
            let selected = if fields.contains_key(&key("routineId")) {
                Some(super::v37_seat::string_field(fields, "routineId")?)
            } else { None };
            self.connection.execute("BEGIN").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let readback = (|| {
                let rows = seat::read_secretary_routines_in_transaction(&self.connection, &self.owner)?;
                if let Some(id) = selected.as_deref() {
                    let row = rows.into_iter().find(|row| row.routine_id == id)
                        .ok_or(OrchestrationError::AccessDenied)?;
                    let occurrences = seat::read_secretary_occurrences_in_transaction(
                        &self.connection, &self.owner, id)?;
                    let history = occurrences.into_iter().map(|item| Json::Object(BTreeMap::from([
                        (key("occurrenceId"), text(&item.occurrence_id)),
                        (key("dueMs"), text(&item.due_ms.to_string())),
                        (key("state"), text(&item.state)),
                        (key("hReceiptId"), text(&item.h_receipt_id)),
                        (key("originalReason"), text(&item.original_reason)),
                    ]))).collect();
                    Ok(reply(command, "READ", [
                        (key("routine"), routine(&row)),
                        (key("occurrences"), Json::Array(history)),
                    ]))
                } else {
                    Ok(reply(command, "READ", [
                        (key("routines"), Json::Array(rows.iter().map(routine).collect())),
                    ]))
                }
            })();
            match readback {
                Ok(bytes) if bytes.len() <= crate::ipc::MAX_FRAME_BYTES => {
                    self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;
                    Ok(bytes)
                }
                Ok(_) => {
                    self.connection.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?;
                    Err(OrchestrationError::Invalid("secretary routine read transport bound"))
                }
                Err(primary) => {
                    if let Err(error) = self.connection.execute("ROLLBACK") {
                        return Err(OrchestrationError::V37StoreFailure(format!(
                            "secretary routine read: {primary:?}; rollback: {error:?}")));
                    }
                    Err(primary)
                }
            }
        } else {
            let resume = command == "secretary-routine-resume";
            let names: &[&str] = if resume {
                &["schema", "command", "routineId", "requestId", "expectedRevision", "nextDueMs"]
            } else {
                &["schema", "command", "routineId", "requestId", "expectedRevision"]
            };
            if !exact(fields, names) { return Err(OrchestrationError::Invalid("secretary routine change fields")); }
            let routine_id = super::v37_seat::string_field(fields, "routineId")?;
            let request_id = super::v37_seat::string_field(fields, "requestId")?;
            let revision = positive_number(fields, "expectedRevision")?;
            let next_due_ms = if resume { Some(positive_number(fields, "nextDueMs")?) } else { None };
            let operation = match command {
                "secretary-routine-pause" => SecretaryRoutineCommand::Pause,
                "secretary-routine-resume" => SecretaryRoutineCommand::Resume,
                "secretary-routine-delete" => SecretaryRoutineCommand::Delete,
                _ => return Err(OrchestrationError::Invalid("secretary routine command")),
            };
            let now_ms = current_time_ms()?;
            let changed = seat::change_secretary_routine(&mut self.connection, &self.owner,
                SecretaryRoutineChange {routine_id: &routine_id, expected_revision: revision,
                    request_id: &request_id, request_bytes: frame, command: operation,
                    next_due_ms, now_ms});
            let bytes = match changed {
                Ok((row, replayed)) => reply(command, if replayed { "REPLAYED" } else { "APPLIED" }, [
                    (key("requestId"), text(&request_id)),
                    (key("routineId"), text(&row.routine_id)),
                    (key("revision"), text(&row.revision.to_string())),
                    (key("state"), text(&row.state)),
                    (key("nextDueMs"), text(&row.next_due_ms.to_string())),
                    (key("lastResult"), text(&row.last_result)),
                ]),
                Err(error) => reply(command, change_status(&error), [
                    (key("requestId"), text(&request_id)),
                    (key("routineId"), text(&routine_id)),
                    (key("reason"), text(&format!("{error:?}"))),
                ]),
            };
            if bytes.len() > crate::ipc::MAX_FRAME_BYTES {
                return Err(OrchestrationError::Invalid("secretary routine change transport bound"));
            }
            Ok(bytes)
        }
    }
}
