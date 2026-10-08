//! Secretary's narrow model proposal becomes an E routine only after H's
//! original USER send and current model custody are rechecked atomically.
use super::*;
use crate::store::atomic::Parser;
use crate::store::digest::sha256_hex;
use crate::store::seat::{self,SecretaryRoutine,SecretaryRoutineCreate};
use crate::store::session_transport::secretary_user_turn;
use std::time::{SystemTime,UNIX_EPOCH};

/// Root's host resolver parses the full original USER text. It must reject
/// ambiguous timing, unsupported zones and a zone not authorized by that text.
/// HOST_DEFAULT means the current host zone, never a model-chosen IANA zone.
pub(crate) struct ResolvedRoutineSchedule {
    pub(crate) schedule_raw:String,
    pub(crate) timezone:String,
    pub(crate) next_due_ms:i64,
}

fn key(name:&str)->JsonString {JsonString::from_str(name)}
fn text(value:&str)->Json {Json::String(JsonString::from_str(value))}
fn field(fields:&BTreeMap<JsonString,Json>,name:&str)->Result<String> {
    let Some(Json::String(value))=fields.get(&key(name)) else {
        return Err(OrchestrationError::Invalid("secretary routine proposal"));
    };
    value.to_well_formed_string().filter(|value|!value.is_empty()&&!value.contains('\0'))
        .ok_or(OrchestrationError::Invalid("secretary routine proposal"))
}
fn proposal(caller:&seat::NativeSeatCall)->Result<(String,String)> {
    if caller.tool()!=Some("gogoke_routine") {return Err(OrchestrationError::AccessDenied);}
    let raw=caller.arguments_json().ok_or(OrchestrationError::AccessDenied)?;
    let Json::Object(fields)=Parser::parse(raw)? else {
        return Err(OrchestrationError::Invalid("secretary routine proposal"));
    };
    // No identity, routine ID, request ID, source locator, now or due time
    // can be proposed by the model. There is one create action only.
    if fields.len()!=3 || ["operation","scheduleSpan","timezone"]
        .iter().any(|name|!fields.contains_key(&key(name)))
        || field(&fields,"operation")?!="create" {
        return Err(OrchestrationError::Invalid("secretary routine proposal fields"));
    }
    Ok((field(&fields,"scheduleSpan")?,field(&fields,"timezone")?))
}
fn now_ms()->Result<i64> {
    let elapsed=SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|error|OrchestrationError::V37StoreFailure(format!("system clock: {error}")))?;
    i64::try_from(elapsed.as_millis()).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("system clock range: {error}")))
}
fn response(status:&str,row:&SecretaryRoutine,request_id:&str)->Vec<u8> {
    Json::Object(BTreeMap::from([
        (key("schema"),text("gogoke.37.secretary-routines.v1")),
        (key("command"),text("secretary-routine-create")),
        (key("status"),text(status)),
        (key("requestId"),text(request_id)),
        (key("routineId"),text(&row.routine_id)),
        (key("revision"),text(&row.revision.to_string())),
        (key("state"),text(&row.state)),
        (key("nextDueMs"),text(&row.next_due_ms.to_string())),
    ])).canonical().into_bytes()
}

impl<'root> ProductDatabase<'root> {
    /// The injected resolver is host code, not a model callback. It receives
    /// `(full original USER body, exact proposed span, proposed zone, trusted now)`.
    pub(super) fn dispatch_model_secretary_routine<F>(&mut self,
        caller:&seat::NativeSeatCall,resolve:F)->Result<Vec<u8>>
    where F:FnOnce(&str,&str,&str,i64)->Result<ResolvedRoutineSchedule> {
        let (span,proposed_zone)=proposal(caller)?;
        let request_id=caller.host_request_id().ok_or(OrchestrationError::AccessDenied)?;
        let raw=caller.raw_request_bytes().ok_or(OrchestrationError::AccessDenied)?;
        let routine_id=format!("routine-{}",&sha256_hex(request_id.as_bytes())[..40]);
        self.connection.execute("BEGIN IMMEDIATE")
            .map_err(OrchestrationError::CommitUnknownWithCause)?;
        let outcome=(||->Result<Vec<u8>> {
            let original=secretary_user_turn::read_original_user_turn_in_transaction(
                &self.connection,caller).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("secretary USER source: {error:?}")))?;
            // Exact, unique slice of the stored USER body. Model paraphrases,
            // text from a project/side turn and fabricated times cannot pass.
            if original.body.match_indices(&span).count()!=1 {
                return Err(OrchestrationError::Invalid("secretary schedule source"));
            }
            if proposed_zone!="HOST_DEFAULT" && !original.body.contains(&proposed_zone) {
                return Err(OrchestrationError::Invalid("secretary timezone source"));
            }
            if let Some(row)=seat::replay_secretary_routine_from_model_in_transaction(
                &self.connection,caller,&routine_id,request_id,raw,&original.body,
                &original.source_operation_id,&original.source_epoch,&original.source_cursor)? {
                return Ok(response("REPLAYED",&row,request_id));
            }
            let now=now_ms()?;
            let resolved=resolve(&original.body,&span,&proposed_zone,now)?;
            if resolved.schedule_raw!=span || resolved.timezone.is_empty()
                || resolved.next_due_ms<=now {
                return Err(OrchestrationError::Invalid("secretary schedule resolution"));
            }
            // The host resolver may canonicalize an explicitly named zone or
            // resolve HOST_DEFAULT. It alone validates that choice against
            // the full original USER text.
            let (row,replayed)=seat::create_secretary_routine_from_model_in_transaction(
                &self.connection,caller,SecretaryRoutineCreate {
                    routine_id:&routine_id,request_id,request_bytes:raw,
                    original_text:&original.body,
                    source_operation_id:&original.source_operation_id,
                    source_epoch:&original.source_epoch,source_cursor:&original.source_cursor,
                    schedule_raw:&resolved.schedule_raw,timezone:&resolved.timezone,
                    next_due_ms:resolved.next_due_ms,now_ms:now,
                })?;
            Ok(response(if replayed {"REPLAYED"} else {"APPLIED"},&row,request_id))
        })();
        match outcome {
            Ok(bytes) if bytes.len()<=crate::ipc::MAX_FRAME_BYTES => {
                self.connection.execute("COMMIT")
                    .map_err(OrchestrationError::CommitUnknownWithCause)?;
                Ok(bytes)
            },
            Ok(_) => {
                self.connection.execute("ROLLBACK")
                    .map_err(OrchestrationError::CommitUnknownWithCause)?;
                Err(OrchestrationError::Invalid("secretary routine response bound"))
            },
            Err(primary) => {
                if let Err(error)=self.connection.execute("ROLLBACK") {
                    return Err(OrchestrationError::V37StoreFailure(format!(
                        "secretary routine create: {primary:?}; rollback: {error:?}")));
                }
                Err(primary)
            },
        }
    }
}
