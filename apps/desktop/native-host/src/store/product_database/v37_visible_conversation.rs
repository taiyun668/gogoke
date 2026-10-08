//! USER-only projections of existing E/F/H/A facts. The selection journal is
//! an explicit UI association, never a session, process, grant or history store.
use super::*;
use super::v37_seat::string_field;
use crate::store::atomic::Parser;

const TABLE: &str = "gogoke_v37_visible_conversation_selection";
const SCHEMA: &str = "CREATE TABLE gogoke_v37_visible_conversation_selection(workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,route TEXT NOT NULL CHECK(route IN ('LEGACY','NATIVE')),selection_hex TEXT NOT NULL,association_json TEXT NOT NULL,repository_id TEXT NOT NULL,worktree_id TEXT NOT NULL,thread_id TEXT NOT NULL,open_request_id TEXT NOT NULL,open_generation TEXT NOT NULL,open_operation_id TEXT NOT NULL,ack_source_id INTEGER NOT NULL,started_source_id INTEGER NOT NULL,CHECK((route='LEGACY' AND association_json='null' AND repository_id='' AND worktree_id='' AND thread_id='' AND open_request_id='' AND open_generation='' AND open_operation_id='' AND ack_source_id=0 AND started_source_id=0) OR (route='NATIVE' AND association_json<>'null' AND repository_id<>'' AND worktree_id<>'' AND thread_id<>'' AND open_request_id<>'' AND open_generation<>'' AND open_operation_id<>'' AND ack_source_id>0 AND started_source_id>0)),PRIMARY KEY(workspace_id,request_id)) STRICT";

pub(super) fn k(name: &str) -> JsonString { JsonString::from_str(name) }
pub(super) fn s(value: &str) -> Json { Json::String(k(value)) }
pub(super) fn is_text(value: Option<&Json>, expected: &str) -> bool {
    matches!(value,Some(Json::String(value)) if value.to_well_formed_string().as_deref()==Some(expected))
}
pub(super) fn same_json(left: Option<&Json>, right: Option<&Json>) -> bool {
    match (left,right) {(Some(left),Some(right))=>left.canonical()==right.canonical(),_=>false}
}
pub(super) fn copy_json(value: &Json) -> Json {
    match value {
        Json::Null=>Json::Null,Json::Bool(value)=>Json::Bool(*value),
        Json::Number(value)=>Json::Number(value.clone()),Json::String(value)=>Json::String(value.clone()),
        Json::Array(values)=>Json::Array(copy_array(values)),Json::Object(fields)=>Json::Object(copy_fields(fields)),
    }
}
pub(super) fn copy_array(values: &[Json]) -> Vec<Json> {values.iter().map(copy_json).collect()}
pub(super) fn copy_fields(fields: &BTreeMap<JsonString,Json>) -> BTreeMap<JsonString,Json> {
    fields.iter().map(|(name,value)|(name.clone(),copy_json(value))).collect()
}
pub(super) fn object(value: &Json) -> Result<&BTreeMap<JsonString,Json>> {
    if let Json::Object(fields)=value {Ok(fields)} else {
        Err(OrchestrationError::Invalid("visible conversation object"))
    }
}
pub(super) fn exact(fields: &BTreeMap<JsonString,Json>, required: &[&str], optional: &[&str]) -> Result<()> {
    if required.iter().any(|name|!fields.contains_key(&k(name)))
        || fields.keys().any(|name|!required.iter().chain(optional).any(|allowed|name==&k(allowed))) {
        return Err(OrchestrationError::Invalid("visible conversation fields"));
    }
    Ok(())
}
pub(super) fn decimal(value: &str) -> Result<i64> {
    let number=value.parse::<i64>().map_err(|error|OrchestrationError::V37StoreFailure(
        format!("visible conversation generation: {error}")))?;
    if number<0 || number.to_string()!=value {return Err(OrchestrationError::Invalid("visible generation"));}
    Ok(number)
}
pub(super) fn encode_hex(bytes: &[u8]) -> String {bytes.iter().map(|byte|format!("{byte:02x}")).collect()}
pub(super) fn decode_hex(value: &str) -> Result<Vec<u8>> {
    if value.len()%2!=0 {return Err(OrchestrationError::Invalid("visible source hex"));}
    value.as_bytes().chunks_exact(2).map(|pair| {
        let high=hex_nibble(pair[0]).ok_or(OrchestrationError::Invalid("visible source hex"))?;
        let low=hex_nibble(pair[1]).ok_or(OrchestrationError::Invalid("visible source hex"))?;
        Ok(high*16+low)
    }).collect()
}
pub(super) fn source_json(value: &str) -> Result<Json> {
    let bytes=decode_hex(value)?;
    let source=std::str::from_utf8(&bytes).map_err(|error|OrchestrationError::V37StoreFailure(
        format!("visible original source UTF-8: {error}")))?;
    Parser::parse(source.trim_end_matches(['\r','\n'])).map_err(OrchestrationError::Atomic)
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(super) struct Association {
    pub(super) domain: String, pub(super) session: String, pub(super) seat: String, pub(super) incarnation: String,
    pub(super) authorization: String, pub(super) generation: String, pub(super) instance: String,
}
impl Association {
    pub(super) fn parse(value: &Json) -> Result<Self> {
        let fields=object(value)?;
        exact(fields,&["domainId","sessionId","seatId","incarnation",
            "authorizationGeneration","bindingGeneration","instanceId"],&[])?;
        let value=Self {domain:string_field(fields,"domainId")?,session:string_field(fields,"sessionId")?,
            seat:string_field(fields,"seatId")?,incarnation:string_field(fields,"incarnation")?,
            authorization:string_field(fields,"authorizationGeneration")?,
            generation:string_field(fields,"bindingGeneration")?,instance:string_field(fields,"instanceId")?};
        decimal(&value.authorization)?;decimal(&value.generation)?;
        Ok(value)
    }
    pub(super) fn json(&self) -> Json {Json::Object(BTreeMap::from([
        (k("domainId"),s(&self.domain)),(k("sessionId"),s(&self.session)),
        (k("seatId"),s(&self.seat)),(k("incarnation"),s(&self.incarnation)),
        (k("authorizationGeneration"),s(&self.authorization)),
        (k("bindingGeneration"),s(&self.generation)),(k("instanceId"),s(&self.instance)),
    ]))}
}
#[derive(Clone)]
pub(super) struct Selection {
    pub(super) row: i64, pub(super) route: String, pub(super) association: Option<Association>, pub(super) repository: String,
    pub(super) worktree: String, pub(super) thread: String, pub(super) open_request: String, pub(super) open_generation: String,
    pub(super) open_operation: String, pub(super) ack: i64, pub(super) started: i64,
}
impl Selection {
    fn from_row(row: &Statement) -> Result<Self> {
        let association=Parser::parse(&row.column_text(2)?)?;
        Ok(Self {row:decimal(&row.column_text(0)?)?,route:row.column_text(1)?,
            association:if matches!(association,Json::Null) {None} else {Some(Association::parse(&association)?)},
            repository:row.column_text(3)?,worktree:row.column_text(4)?,thread:row.column_text(5)?,
            open_request:row.column_text(6)?,open_generation:row.column_text(7)?,
            open_operation:row.column_text(8)?,ack:decimal(&row.column_text(9)?)?,
            started:decimal(&row.column_text(10)?)?})
    }
}

pub(super) fn initialize_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<()> {
    if !schema_state(db)? {
        db.execute(SCHEMA).map_err(|cause|OrchestrationError::V37StoreFailure(
            format!("visible conversation selection schema creation: {cause:?}")))?;
    }
    if !schema_state(db)? {return Err(OrchestrationError::Invalid("visible selection schema"));}
    Ok(())
}
fn schema_state(db: &VerifiedDatabaseConnection<'_>) -> Result<bool> {
    let effects=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM temp.sqlite_schema WHERE lower(name)=?1 OR lower(tbl_name)=?1 UNION ALL SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND lower(tbl_name)=?1")?;
    effects.bind_text(1,TABLE)?;
    if effects.step_row()? {return Err(OrchestrationError::Invalid("visible selection schema effects"));}
    let row=Statement::prepare(db.as_ptr(),"SELECT type,sql FROM main.sqlite_schema WHERE lower(name)=?1")?;
    row.bind_text(1,TABLE)?;
    if !row.step_row()? {return Ok(false);}
    if row.column_text(0)?!="table" || row.column_text(1)?!=SCHEMA || row.step_row()? {
        return Err(OrchestrationError::Invalid("visible selection schema drift"));
    }
    Ok(true)
}

impl<'root> ProductDatabase<'root> {
    pub(super) fn dispatch_visible_conversation_configuration(&mut self,
        command: &str, fields: &BTreeMap<JsonString,Json>, frame: &[u8]) -> Result<Vec<u8>> {
        let workspace=string_field(fields,"workspaceId")?;
        if workspace.len()>128 {return Err(OrchestrationError::Invalid("visible workspace id"));}
        let write=command=="visible-conversation-select";
        self.connection.execute(if write {"BEGIN IMMEDIATE"} else {"BEGIN"})
            .map_err(OrchestrationError::CommitUnknownWithCause)?;
        let result=(|| {
            if !schema_state(&self.connection)? {return Err(OrchestrationError::Invalid("visible selection schema"));}
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            self.visible_configuration_in_transaction(command,fields,frame,&workspace)
        })();
        match result {
            Ok(reply)=>{
                let bytes=reply.canonical().into_bytes();
                if bytes.len()>crate::ipc::MAX_FRAME_BYTES {
                    self.connection.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?;
                    return Err(OrchestrationError::Invalid("visible response frame bound"));
                }
                self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;
                Ok(bytes)
            },
            Err(error)=>{
                self.connection.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?;
                Err(error)
            },
        }
    }
    pub(super) fn visible_reply(&self, workspace: &str, state: &str, association: Option<&Association>, reason: Option<&str>)
        -> BTreeMap<JsonString,Json> {
        let mut reply=BTreeMap::from([(k("schema"),s("gogoke.37.visible-conversation.v1")),
            (k("workspaceId"),s(workspace)),(k("state"),s(state))]);
        if let Some(association)=association {reply.insert(k("association"),association.json());}
        if let Some(reason)=reason {reply.insert(k("reason"),s(reason));}
        reply
    }
    fn visible_tables_available(&self) -> Result<bool> {
        for name in ["gogoke_v37_session_binding_v2","gogoke_v37_native_selection",
            "gogoke_v37_worktrees","gogoke_v37_worktree_sources","gogoke_v37_seats",
            "gogoke_v37_h_claim","gogoke_v37_h_process_episode","gogoke_v37_h_generation",
            "gogoke_v37_rpc_steps","v37_ledger_raw_source","v37_ledger_session"] {
            let row=Statement::prepare(self.connection.as_ptr(),
                "SELECT 1 FROM main.sqlite_schema WHERE type='table' AND name=?1")?;
            row.bind_text(1,name)?;
            if !row.step_row()? {return Ok(false);}
        }
        Ok(true)
    }
    pub(super) fn visible_selection(&self, workspace: &str, association: Option<&Association>) -> Result<Option<Selection>> {
        let row=Statement::prepare(self.connection.as_ptr(),
            "SELECT rowid,route,association_json,repository_id,worktree_id,thread_id,open_request_id,open_generation,open_operation_id,ack_source_id,started_source_id FROM main.gogoke_v37_visible_conversation_selection WHERE workspace_id=?1 ORDER BY rowid DESC")?;
        row.bind_text(1,workspace)?;
        while row.step_row()? {
            let selection=Selection::from_row(&row)?;
            if association.is_none() || selection.association.as_ref()==association {return Ok(Some(selection));}
        }
        Ok(None)
    }
    /// Verify an explicit existing relationship; directory names and vendor IDs
    /// never choose an E seat, F repository, H session or instance.
    pub(super) fn visible_candidate(&self, association: &Association, current: bool) -> Result<Selection> {
        use crate::store::session_transport::session_binding::{self,Provenance};
        if association.domain=="global" {return Err(OrchestrationError::AccessDenied);}
        let binding=session_binding::read(&self.connection,&association.domain,&association.session)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("visible H binding: {error:?}")))?
            .ok_or(OrchestrationError::OperationConflict)?;
        if binding.provenance!=Provenance::NativeV2 || binding.seat_id!=association.seat
            || binding.seat_incarnation!=association.incarnation
            || binding.seat_authorization_generation.to_string()!=association.authorization
            || binding.selected_instance_id!=association.instance {return Err(OrchestrationError::AccessDenied);}
        let seat=Statement::prepare(self.connection.as_ptr(),
            "SELECT s.generation,s.state,i.driver_id,i.version FROM main.gogoke_v37_seats s JOIN main.gogoke_v37_instances i ON i.instance_id=?4 JOIN main.gogoke_v37_native_selection n ON n.domain_id=s.domain_id AND n.session_id=?5 AND n.seat_id=s.seat_id AND n.seat_incarnation=s.incarnation AND n.seat_authorization_generation=?6 AND n.selected_instance_id=i.instance_id WHERE s.domain_id=?1 AND s.seat_id=?2 AND s.incarnation=?3 AND s.layer='USER' AND s.parent_seat_id IS NULL")?;
        for (index,value) in [association.domain.as_str(),association.seat.as_str(),
            association.incarnation.as_str(),association.instance.as_str(),association.session.as_str(),
            association.authorization.as_str()].iter().enumerate() {seat.bind_text((index+1) as i32,value)?;}
        if !seat.step_row()? {return Err(OrchestrationError::AccessDenied);}
        if decimal(&seat.column_text(0)?)?<decimal(&association.authorization)? || seat.column_text(1)?=="RECLAIMED"
            || seat.column_text(2)?!="codex" || seat.column_text(3)?!="0.160.0" || seat.step_row()? {
            return Err(OrchestrationError::OperationConflict);
        }
        drop(seat);
        if current {
            let relationship=session_binding::current_relationship(&self.connection,&association.domain,&association.session)
                .map_err(|error|OrchestrationError::V37StoreFailure(format!("visible current E/H: {error:?}")))?
                .ok_or(OrchestrationError::OperationConflict)?;
            if !relationship.native_v2 || relationship.seat_id!=association.seat
                || relationship.seat_incarnation!=association.incarnation
                || relationship.seat_authorization_generation.to_string()!=association.authorization
                || relationship.session_generation!=association.generation || relationship.instance_id!=association.instance {
                return Err(OrchestrationError::AccessDenied);
            }
        }
        let registration=crate::store::ledger::read_registered_session(&self.connection,&association.session)?
            .ok_or(OrchestrationError::OperationConflict)?;
        if registration.domain_id!=association.domain || registration.seat_id!=association.seat
            || registration.purpose!=crate::store::ledger::SessionPurpose::Work || registration.side_id.is_some() {
            return Err(OrchestrationError::AccessDenied);
        }
        let (repository,worktree,thread)=self.original_native_continuation(&association.domain,&association.session)?;
        let wt=Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_worktrees w JOIN main.gogoke_v37_worktree_sources r ON r.repository_id=w.repository_id WHERE w.worktree_id=?1 AND w.repository_id=?2 AND w.domain_id=?3 AND w.seat_id=?4 AND w.seat_incarnation=?5 AND w.state='REGISTERED' AND w.revision=1")?;
        // F records creation provenance. E/H record later authorized instance
        // and generation changes; those need not equal the creation snapshot.
        for (index,value) in [worktree.as_str(),repository.as_str(),association.domain.as_str(),
            association.seat.as_str(),association.incarnation.as_str()].iter().enumerate() {wt.bind_text((index+1) as i32,value)?;}
        if !wt.step_row()? || wt.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let original=Statement::prepare(self.connection.as_ptr(),
            "SELECT e.request_id,e.generation,e.process_operation_id,r.rowid,s.ticket,s.custodian_nonce FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_rpc_steps s ON s.domain_id=e.domain_id AND s.session_id=e.session_id AND s.generation=e.generation AND s.process_operation_id=e.process_operation_id AND s.open_request_id=e.request_id AND s.step_id='thread-start' AND s.phase='OBSERVED' JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce AND r.domain_id=s.domain_id AND r.session_id=s.session_id AND r.generation=s.generation AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor WHERE e.domain_id=?1 AND e.session_id=?2 AND e.old_generation IS NULL")?;
        original.bind_text(1,&association.domain)?;original.bind_text(2,&association.session)?;
        if !original.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let open_request=original.column_text(0)?;let open_generation=original.column_text(1)?;
        let open_operation=original.column_text(2)?;let ack=decimal(&original.column_text(3)?)?;
        let ticket=original.column_text(4)?;let nonce=original.column_text(5)?;
        if original.step_row()? {return Err(OrchestrationError::OperationConflict);}drop(original);
        // The original vendor thread notification is required as well as its
        // correlated ACK. Neither one alone establishes the visible identity.
        let started=Statement::prepare(self.connection.as_ptr(),
            "SELECT rowid,hex(raw_bytes) FROM main.v37_ledger_raw_source WHERE domain_id=?1 AND session_id=?2 AND operation_id=?3 AND process_ticket=?4 AND custodian_nonce=?5 AND generation=?6 ORDER BY rowid")?;
        for (index,value) in [association.domain.as_str(),association.session.as_str(),open_operation.as_str(),
            ticket.as_str(),nonce.as_str(),open_generation.as_str()].iter().enumerate() {started.bind_text((index+1) as i32,value)?;}
        let mut source_id=None;
        while started.step_row()? {
            let source=source_json(&started.column_text(1)?)?;
            let envelope=object(&source)?;
            if !is_text(envelope.get(&k("method")),"thread/started") {continue;}
            let params=object(envelope.get(&k("params")).ok_or(OrchestrationError::OperationConflict)?)?;
            let vendor=object(params.get(&k("thread")).ok_or(OrchestrationError::OperationConflict)?)?;
            if string_field(vendor,"id")?!=thread || source_id.is_some() {return Err(OrchestrationError::OperationConflict);}
            source_id=Some(decimal(&started.column_text(0)?)?);
        }
        let started=source_id.ok_or(OrchestrationError::OperationConflict)?;
        // Historical association generations remain readable only through the
        // actual H episode and its own correlated provider continuation ACK.
        let episode=Statement::prepare(self.connection.as_ptr(),
            "SELECT e.request_id,e.process_operation_id,c.ticket,c.custodian_nonce FROM main.gogoke_v37_h_generation g JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=g.domain_id AND e.session_id=g.session_id AND e.generation=g.generation AND e.request_id=g.request_id AND e.process_operation_id=g.process_operation_id JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id AND c.generation=e.generation WHERE g.domain_id=?1 AND g.session_id=?2 AND g.generation=?3 AND e.instance_id=?4 AND e.seat_id=?5 AND e.seat_incarnation=?6")?;
        for (index,value) in [association.domain.as_str(),association.session.as_str(),association.generation.as_str(),
            association.instance.as_str(),association.seat.as_str(),association.incarnation.as_str()].iter().enumerate() {episode.bind_text((index+1) as i32,value)?;}
        if !episode.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let request=episode.column_text(0)?;let operation=episode.column_text(1)?;
        let ticket=episode.column_text(2)?;let nonce=episode.column_text(3)?;
        if episode.step_row()? {return Err(OrchestrationError::OperationConflict);}drop(episode);
        let observed=crate::store::session_transport::rpc_journal::observed_thread_id(&self.connection,
            &association.domain,&association.session,&operation,&association.generation,&request,&ticket,&nonce)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("visible generation thread ACK: {error:?}")))?;
        if observed!=thread {return Err(OrchestrationError::OperationConflict);}
        Ok(Selection {row:0,route:"NATIVE".into(),association:Some(association.clone()),repository,
            worktree,thread,open_request,open_generation,open_operation,ack,started})
    }

    pub(super) fn visible_verify_saved(&self, saved: &Selection) -> Result<()> {
        let association=saved.association.as_ref().ok_or(OrchestrationError::AccessDenied)?;
        let actual=self.visible_candidate(association,false)?;
        if actual.repository!=saved.repository || actual.worktree!=saved.worktree || actual.thread!=saved.thread
            || actual.open_request!=saved.open_request || actual.open_generation!=saved.open_generation
            || actual.open_operation!=saved.open_operation || actual.ack!=saved.ack || actual.started!=saved.started {
            return Err(OrchestrationError::OperationConflict);
        }
        Ok(())
    }
    /// Read the selected generation from H's original claim, episode and
    /// custody. A retained status is insufficient to establish a live child;
    /// only this holder's exact process handle can do that. Conversely, an
    /// absent handle is not a physical StopFact.
    fn visible_live_state(&self, association: &Association, selected: &Selection)
        -> Result<(&'static str, Option<String>)> {
        if let Err(error)=self.visible_candidate(association,true) {
            return Ok(("UNKNOWN",Some(format!("Original current E/F/H/A association: {error:?}"))));
        }
        let row=Statement::prepare(self.connection.as_ptr(),
            "SELECT a.state,COALESCE(a.stop_fact_id,''),e.phase,COALESCE(e.stop_fact_id,''),
                    c.state,COALESCE(c.stop_proof_hash,''),c.operation_id,c.ticket,
                    c.custodian_nonce,c.pid,c.creation_time_100ns,c.image_path,
                    c.binary_digest_sha256,c.profile_id,e.request_id
               FROM main.gogoke_v37_h_claim a
               JOIN main.gogoke_v37_h_generation g ON g.domain_id=a.domain_id
                    AND g.session_id=a.session_id AND g.generation=a.generation
                    AND g.process_operation_id=a.process_operation_id
               JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=g.domain_id
                    AND e.session_id=g.session_id AND e.generation=g.generation
                    AND e.request_id=g.request_id AND e.process_operation_id=g.process_operation_id
                    AND e.instance_id=a.instance_id AND e.home_id=a.home_id
                    AND e.binding_id=a.binding_id
               JOIN main.gogoke_coordination_process_custody c
                    ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
                    AND c.generation=e.generation
              WHERE a.domain_id=?1 AND a.session_id=?2 AND a.generation=?3
                    AND a.instance_id=?4 AND e.seat_id=?5 AND e.seat_incarnation=?6")?;
        for (index,value) in [association.domain.as_str(),association.session.as_str(),
            association.generation.as_str(),association.instance.as_str(),
            association.seat.as_str(),association.incarnation.as_str()].iter().enumerate() {
            row.bind_text((index+1) as i32,value)?;
        }
        if !row.step_row()? {
            return Ok(("UNKNOWN",Some("Original current H claim/generation/episode/custody tuple is absent.".into())));
        }
        let facts=(0..15).map(|index|row.column_text(index)).collect::<std::result::Result<Vec<_>,_>>()?;
        if row.step_row()? {
            return Ok(("UNKNOWN",Some("Original current H process tuple is ambiguous.".into())));
        }
        drop(row);
        let [claim,claim_stop,episode,episode_stop,custody,custody_stop,
            operation,ticket,nonce,pid,creation,image,digest,profile,request]:[String;15]=
            facts.try_into().map_err(|_|OrchestrationError::OperationConflict)?;
        if claim=="STOPPED" && episode=="STOPPED" && custody=="STOPPED"
            && !claim_stop.is_empty() && claim_stop==episode_stop && claim_stop==custody_stop {
            return Ok(("STOPPED",None));
        }
        if claim!="COMMITTED" || episode!="ACTIVE" || custody!="ACTIVE"
            || !claim_stop.is_empty() || !episode_stop.is_empty() || !custody_stop.is_empty() {
            return Ok(("UNKNOWN",Some(format!("Original H process is not a proven live or stopped tuple: claim={claim}, episode={episode}, custody={custody}, stopProofPresent={}",
                !claim_stop.is_empty() || !episode_stop.is_empty() || !custody_stop.is_empty()))));
        }
        let key=(association.domain.clone(),association.session.clone());
        let Some(run)=self.native_sessions.get(&key) else {
            return Ok(("UNKNOWN",Some("Original current H holder has no retained live process custody.".into())));
        };
        if run.operation_id!=operation || run.open_request_id!=request
            || run.evidence.driver_id()!="codex" || run.evidence.seat_id()!=association.seat
            || run.evidence.seat_incarnation()!=association.incarnation
            || run.custody.binding.domain_id!=association.domain
            || run.custody.binding.generation!=association.generation
            || run.thread_id.as_deref()!=Some(selected.thread.as_str())
            || run.custody.ticket.opaque()!=ticket || run.custody.custodian_nonce!=nonce
            || run.custody.identity.pid.to_string()!=pid
            || run.custody.identity.creation_time_100ns.to_string()!=creation
            || run.custody.identity.image_path.to_string_lossy()!=image
            || run.custody.binding.binary_digest_sha256!=digest
            || run.custody.binding.profile_id!=profile {
            return Ok(("UNKNOWN",Some("Original H process identity, generation, thread or custody differs from the retained holder.".into())));
        }
        let Some(process)=self.process_custodian.active(&run.custody.ticket) else {
            return Ok(("UNKNOWN",Some("Original H process handle is absent; absence does not prove STOPPED.".into())));
        };
        if process.identity()!=&run.custody.identity {
            return Ok(("UNKNOWN",Some("Original H process handle identity differs from PID and creation time custody.".into())));
        }
        match process.wait(std::time::Duration::ZERO) {
            Ok(false)=>{},
            Ok(true)=>return Ok(("UNKNOWN",Some("Original H process handle is signaled; no physical StopFact is recorded.".into()))),
            Err(error)=>return Ok(("UNKNOWN",Some(format!("Original H process handle wait observation: {error}")))),
        }
        match process.exit_code() {
            Ok(None)=>{},
            Ok(Some(code))=>return Ok(("UNKNOWN",Some(format!("Original H process exited with code {code}; no physical StopFact is recorded.")))),
            Err(error)=>return Ok(("UNKNOWN",Some(format!("Original H process exit observation: {error}")))),
        }
        Ok(("LIVE",None))
    }
    fn visible_configuration_in_transaction(&mut self, command: &str,
        fields: &BTreeMap<JsonString,Json>, frame: &[u8], workspace: &str) -> Result<Json> {
        let base=["schema","command","workspaceId"];
        match command {
            "visible-conversation-route"=>{
                exact(fields,&base,&[])?;
                let Some(selected)=self.visible_selection(workspace,None)? else {
                    return Ok(Json::Object(self.visible_reply(workspace,"NEEDS_SETUP",None,
                        Some("No explicit USER visible-conversation selection is recorded."))));
                };
                if selected.route=="LEGACY" {return Ok(Json::Object(self.visible_reply(workspace,"NEEDS_SETUP",None,
                    Some("The retained historical LEGACY selection grants no Gogoke USER model execution; an explicit qualified native association is required."))));}
                let association=selected.association.as_ref().ok_or(OrchestrationError::OperationConflict)?;
                match self.visible_verify_saved(&selected) {
                    Ok(())=>Ok(Json::Object(self.visible_reply(workspace,"NATIVE",Some(association),None))),
                    Err(error)=>Ok(Json::Object(self.visible_reply(workspace,"UNKNOWN",Some(association),
                        Some(&format!("Original selected E/F/H/A association is unresolved: {error:?}"))))),
                }
            },
            "visible-conversation-choices"=>{
                exact(fields,&base,&[])?;
                if !self.visible_tables_available()? {return Ok(Json::Object(self.visible_reply(workspace,"UNKNOWN",None,
                    Some("Existing E/F/H/A source tables are not available; no choices producer is complete."))));}
                let query=Statement::prepare(self.connection.as_ptr(),
                    "SELECT n.domain_id,n.session_id,n.seat_id,n.seat_incarnation,n.seat_authorization_generation,COALESCE(h.generation,''),n.selected_instance_id,i.driver_id FROM main.gogoke_v37_native_selection n JOIN main.gogoke_v37_seats e ON e.domain_id=n.domain_id AND e.seat_id=n.seat_id AND e.incarnation=n.seat_incarnation AND e.layer='USER' AND e.parent_seat_id IS NULL LEFT JOIN main.gogoke_v37_h_claim h ON h.domain_id=n.domain_id AND h.session_id=n.session_id JOIN main.gogoke_v37_instances i ON i.instance_id=n.selected_instance_id WHERE n.domain_id<>'global' ORDER BY n.domain_id,n.session_id")?;
                let mut choices=Vec::new();
                while query.step_row()? {
                    let association=Association {domain:query.column_text(0)?,session:query.column_text(1)?,
                        seat:query.column_text(2)?,incarnation:query.column_text(3)?,authorization:query.column_text(4)?,
                        generation:query.column_text(5)?,instance:query.column_text(6)?};
                    let mut choice=BTreeMap::from([(k("association"),association.json()),(k("provider"),s(&query.column_text(7)?))]);
                    match self.visible_candidate(&association,true) {
                        Ok(selected)=>{
                            choice.insert(k("state"),s("NATIVE"));
                            choice.insert(k("repositoryId"),s(&selected.repository));
                            choice.insert(k("worktreeId"),s(&selected.worktree));
                            choice.insert(k("threadId"),s(&selected.thread));
                        },
                        Err(error)=>{
                            choice.insert(k("state"),s("UNKNOWN"));
                            choice.insert(k("reason"),s(&format!("Original existing E/F/H/A choice: {error:?}")));
                        },
                    }
                    choices.push(Json::Object(choice));
                }
                let mut reply=self.visible_reply(workspace,"APPLIED",None,None);
                if choices.is_empty() {
                    reply=self.visible_reply(workspace,"UNKNOWN",None,
                        Some("No original existing lead session can be offered; no session was created."));
                }
                reply.insert(k("choices"),Json::Array(choices));
                Ok(Json::Object(reply))
            },
            "visible-conversation-select"=>{
                let route=string_field(fields,"route")?;
                if route=="LEGACY" {exact(fields,&["schema","command","workspaceId","requestId","route"],&[])?;}
                else if route=="NATIVE" {exact(fields,&["schema","command","workspaceId","requestId","route",
                    "repositoryId","threadId","association"],&[])?;}
                else {return Err(OrchestrationError::Invalid("visible route"));}
                let request=string_field(fields,"requestId")?;
                if request.len()>128 {return Err(OrchestrationError::Invalid("visible request id"));}
                let prior=Statement::prepare(self.connection.as_ptr(),
                    "SELECT selection_hex FROM main.gogoke_v37_visible_conversation_selection WHERE workspace_id=?1 AND request_id=?2")?;
                prior.bind_text(1,workspace)?;prior.bind_text(2,&request)?;
                let exists=prior.step_row()?;
                if exists && (prior.column_text(0)?!=encode_hex(frame) || prior.step_row()?) {
                    return Err(OrchestrationError::OperationConflict);
                }
                drop(prior);
                if route=="LEGACY" {
                    let mut reply=self.visible_reply(workspace,"UNSUPPORTED",None,Some(
                        "Gogoke USER model conversations require LPAC native custody; LEGACY selection cannot grant ordinary CLI execution."));
                    reply.insert(k("requestId"),s(&request));return Ok(Json::Object(reply));
                }
                let selected={
                    let association=Association::parse(fields.get(&k("association")).ok_or(OrchestrationError::AccessDenied)?)?;
                    match self.visible_candidate(&association,!exists) {
                        Ok(selected) if selected.repository==string_field(fields,"repositoryId")?
                            && selected.thread==string_field(fields,"threadId")?=>selected,
                        result=>{
                            let reason=match result {Err(error)=>format!("Original selection verification: {error:?}"),
                                Ok(_)=>"Explicit repository/thread differs from original H/A facts.".into()};
                            let mut reply=self.visible_reply(workspace,"UNKNOWN",Some(&association),Some(&reason));
                            reply.insert(k("requestId"),s(&request));return Ok(Json::Object(reply));
                        },
                    }
                };
                if !exists {
                    let insert=Statement::prepare(self.connection.as_ptr(),
                        "INSERT INTO main.gogoke_v37_visible_conversation_selection(workspace_id,request_id,route,selection_hex,association_json,repository_id,worktree_id,thread_id,open_request_id,open_generation,open_operation_id,ack_source_id,started_source_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)")?;
                    let raw=encode_hex(frame);let association=selected.association.as_ref().map(Association::json).unwrap_or(Json::Null).canonical();
                    for (index,value) in [workspace,request.as_str(),route.as_str(),raw.as_str(),association.as_str(),
                        selected.repository.as_str(),selected.worktree.as_str(),selected.thread.as_str(),selected.open_request.as_str(),
                        selected.open_generation.as_str(),selected.open_operation.as_str()].iter().enumerate() {insert.bind_text((index+1) as i32,value)?;}
                    insert.bind_i64(12,selected.ack)?;insert.bind_i64(13,selected.started)?;
                    insert.step_done()?;
                }
                let mut reply=self.visible_reply(workspace,&route,selected.association.as_ref(),None);
                reply.insert(k("requestId"),s(&request));Ok(Json::Object(reply))
            },
            "visible-conversation-read" | "visible-conversation-operate" | "visible-conversation-recover"=>{
                let operation=command!="visible-conversation-read";
                let mut required=vec!["schema","command","workspaceId","expectedAssociation"];
                if operation {required.push("requestId");}
                if command!="visible-conversation-recover" {required.push("method");}
                exact(fields,&required,if command=="visible-conversation-recover" {&[]} else {&["params"]})?;
                let association=Association::parse(fields.get(&k("expectedAssociation")).ok_or(OrchestrationError::AccessDenied)?)?;
                let selected=self.visible_selection(workspace,Some(&association))?;
                let mut reply=self.visible_reply(workspace,"UNKNOWN",Some(&association),None);
                if operation {reply.insert(k("requestId"),s(&string_field(fields,"requestId")?));}
                let Some(selected)=selected else {
                    reply.insert(k("state"),s("DENIED"));reply.insert(k("reason"),s("No exact USER selection in this workspace matches all expected association fields."));
                    return Ok(Json::Object(reply));
                };
                if let Err(error)=self.visible_verify_saved(&selected) {
                    reply.insert(k("reason"),s(&format!("Original selected E/F/H/A read verification: {error:?}")));
                    return Ok(Json::Object(reply));
                }
                if operation {
                    reply.insert(k("state"),s("UNSUPPORTED"));reply.insert(k("reason"),s(
                        "Visible conversation effects and effect recovery are not implemented in this read-only producer phase. No provider request was sent."));
                    return Ok(Json::Object(reply));
                }
                let method=string_field(fields,"method")?;
                let empty=BTreeMap::new();
                let params=match fields.get(&k("params")) {None=>&empty,Some(value)=>object(value)?};
                match method.as_str() {
                    "live-state"=>{
                        exact(params,&[],&[])?;
                        let (state,reason)=match self.visible_live_state(&association,&selected) {
                            Ok(observed)=>observed,
                            Err(error)=>("UNKNOWN",Some(format!("Original H live-state observation: {error:?}"))),
                        };
                        if matches!(state,"LIVE"|"STOPPED") && reason.is_none() {
                            reply.insert(k("state"),s("APPLIED"));
                        }
                        if let Some(reason)=reason {reply.insert(k("reason"),s(&reason));}
                        reply.insert(k("live"),Json::Object(BTreeMap::from([(k("state"),s(state)),
                            (k("pendingQuestions"),self.visible_pending_questions(&association,&selected.thread)?)])));
                    },
                    "thread/read"=>{
                        exact(params,&["threadId","includeTurns"],&["cursor"])?;
                        if string_field(params,"threadId")?!=selected.thread || !matches!(params.get(&k("includeTurns")),Some(Json::Bool(true))) {
                            reply.insert(k("state"),s("DENIED"));reply.insert(k("reason"),s("Thread read must name the exact selected vendor thread and include its original turns."));
                        } else if let Err(error)=self.visible_thread_page(workspace,&selected,params,&mut reply) {
                            reply.insert(k("state"),s("UNKNOWN"));reply.insert(k("reason"),s(&format!("Original conversation read: {error:?}")));
                        }
                    },
                    "native-events"=>{
                        exact(params,&[],&["cursor","resumeCursor"])?;
                        if let Err(error)=self.visible_native_events(workspace,&association,&selected,params,&mut reply) {
                            reply.insert(k("state"),s("UNKNOWN"));
                            reply.insert(k("reason"),s(&format!("Original native event read: {error:?}")));
                            reply.insert(k("response"),visible_native_event_unknown());
                        }
                    },
                    "thread/list"=>{
                        exact(params,&[],&["cursor","limit"])?;
                        if let Err(error)=self.visible_thread_list(workspace,&association,params,&mut reply) {
                            reply.insert(k("state"),s("UNKNOWN"));reply.insert(k("reason"),s(&format!("Original workspace thread list: {error:?}")));
                        }
                    },
                    _=>{
                        reply.insert(k("state"),s("UNSUPPORTED"));reply.insert(k("reason"),s("This producer only supports qualified thread/read, thread/list, native-events and live-state reads."));
                    },
                }
                Ok(Json::Object(reply))
            },
            _=>Err(OrchestrationError::Invalid("visible configuration command")),
        }
    }

    fn visible_high_water(&self, selected: &Selection) -> Result<i64> {
        let association=selected.association.as_ref().ok_or(OrchestrationError::AccessDenied)?;
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT COALESCE(MAX(rowid),0) FROM main.v37_ledger_raw_source WHERE domain_id=?1 AND session_id=?2")?;
        query.bind_text(1,&association.domain)?;query.bind_text(2,&association.session)?;
        if !query.step_row()? {return Err(OrchestrationError::OperationConflict);}
        decimal(&query.column_text(0)?)
    }
    /// Read A's captured provider frames from the selected current H episode.
    /// The page boundary is an original raw rowid, never a completed Turn or
    /// a reconstruction from thread/read. A resume advances to a fresh upper
    /// bound; a page cursor remains on the bound of its first page.
    fn visible_native_events(&self, workspace: &str, association: &Association,
        selected: &Selection, params: &BTreeMap<JsonString,Json>,
        reply: &mut BTreeMap<JsonString,Json>) -> Result<()> {
        if self.visible_selection(workspace,None)?.as_ref().map(|latest|latest.row)!=Some(selected.row) {
            return Err(OrchestrationError::V37StoreFailure(
                "Original USER workspace selection changed before native event read.".into()));
        }
        let current=self.visible_candidate(association,true)?;
        if current.thread!=selected.thread || current.repository!=selected.repository
            || current.worktree!=selected.worktree {
            return Err(OrchestrationError::OperationConflict);
        }
        let episode=Statement::prepare(self.connection.as_ptr(),
            "SELECT e.process_operation_id,c.ticket,c.custodian_nonce FROM main.gogoke_v37_h_generation g JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=g.domain_id AND e.session_id=g.session_id AND e.generation=g.generation AND e.request_id=g.request_id AND e.process_operation_id=g.process_operation_id JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id AND c.generation=e.generation WHERE g.domain_id=?1 AND g.session_id=?2 AND g.generation=?3 AND e.instance_id=?4 AND e.seat_id=?5 AND e.seat_incarnation=?6")?;
        for (index,value) in [association.domain.as_str(),association.session.as_str(),association.generation.as_str(),
            association.instance.as_str(),association.seat.as_str(),association.incarnation.as_str()].iter().enumerate() {
            episode.bind_text((index+1) as i32,value)?;
        }
        if !episode.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let operation=episode.column_text(0)?;
        let ticket=episode.column_text(1)?;
        let nonce=episode.column_text(2)?;
        if episode.step_row()? {return Err(OrchestrationError::OperationConflict);}
        drop(episode);
        let maximum=Statement::prepare(self.connection.as_ptr(),
            "SELECT COALESCE(MAX(rowid),0) FROM main.v37_ledger_raw_source WHERE domain_id=?1 AND session_id=?2 AND generation=?3")?;
        maximum.bind_text(1,&association.domain)?;maximum.bind_text(2,&association.session)?;
        maximum.bind_text(3,&association.generation)?;
        if !maximum.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let maximum=decimal(&maximum.column_text(0)?)?;
        let scope=format!("native-events\n{workspace}\n{}\n{}\n{}\n{}",selected.row,
            selected.thread,association.json().canonical(),operation);
        let (high,after)=visible_native_event_position(params,maximum,&scope)?;
        let predecessor=Statement::prepare(self.connection.as_ptr(),
            "SELECT source_cursor,operation_id,process_ticket,custodian_nonce,source_epoch FROM main.v37_ledger_raw_source WHERE domain_id=?1 AND session_id=?2 AND generation=?3 AND rowid<=?4 ORDER BY rowid DESC LIMIT 1")?;
        predecessor.bind_text(1,&association.domain)?;predecessor.bind_text(2,&association.session)?;
        predecessor.bind_text(3,&association.generation)?;predecessor.bind_i64(4,after)?;
        let mut ordinal=if predecessor.step_row()? {
            let ordinal=decimal(&predecessor.column_text(0)?)?;
            if predecessor.column_text(1)?!=operation || predecessor.column_text(2)?!=ticket
                || predecessor.column_text(3)?!=nonce || predecessor.column_text(4)?!=nonce {
                return Err(OrchestrationError::V37StoreFailure(
                    "Original native event cursor predecessor differs from H episode/custody.".into()));
            }
            ordinal
        } else {0};
        drop(predecessor);
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT rowid,operation_id,process_ticket,custodian_nonce,source_epoch,source_cursor FROM main.v37_ledger_raw_source WHERE domain_id=?1 AND session_id=?2 AND generation=?3 AND rowid>?4 AND rowid<=?5 ORDER BY rowid")?;
        query.bind_text(1,&association.domain)?;query.bind_text(2,&association.session)?;
        query.bind_text(3,&association.generation)?;query.bind_i64(4,after)?;query.bind_i64(5,high)?;
        let mut notifications=Vec::new();
        let mut refs=Vec::new();
        let mut last=after;
        let mut more=false;
        while query.step_row()? {
            let id=decimal(&query.column_text(0)?)?;
            let row_operation=query.column_text(1)?;
            let row_ticket=query.column_text(2)?;
            let row_nonce=query.column_text(3)?;
            let epoch=query.column_text(4)?;
            let cursor=query.column_text(5)?;
            let next=ordinal.checked_add(1).ok_or(OrchestrationError::Invalid("native raw cursor overflow"))?;
            if decimal(&cursor)?!=next {
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "Original native source {id} has a missing or repeated frame ordinal after {ordinal}.")));
            }
            ordinal=next;
            if row_operation!=operation || row_ticket!=ticket || row_nonce!=nonce || epoch!=nonce {
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "Original native source {id} disagrees with selected H generation/episode/custody.")));
            }
            let raw=crate::store::ledger::read_captured_raw_source(&self.connection,&row_operation,&epoch,&cursor)?
                .ok_or_else(||OrchestrationError::V37StoreFailure(format!("Original native source {id} disappeared.")))?;
            if raw.domain_id!=association.domain || raw.session_id!=association.session
                || raw.generation!=association.generation || raw.process_ticket!=ticket
                || raw.custodian_nonce!=nonce {
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "Original native source {id} has inconsistent A/H identity.")));
            }
            let source=std::str::from_utf8(&raw.raw_bytes).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("Original native source {id} UTF-8: {error}")))?;
            let notification=Parser::parse(source.trim_end_matches(['\r','\n']))
                .map_err(|error|OrchestrationError::V37StoreFailure(format!(
                    "Original native source {id} JSON: {error:?}")))?;
            let envelope=object(&notification)?;
            // A JSON-RPC method with an ID is a server request, not a
            // notification for app-server-event. Its separate USER question
            // and response path must retain ownership of that frame.
            if envelope.contains_key(&k("id")) {last=id;continue;}
            let method=match envelope.get(&k("method")) {
                None=>{last=id;continue;},
                Some(Json::String(value))=>value.to_well_formed_string()
                    .ok_or(OrchestrationError::Invalid("native event method"))?,
                _=>return Err(OrchestrationError::V37StoreFailure(format!(
                    "Original native source {id} has a non-text method."))),
            };
            let relevant=method.starts_with("item/") || method.starts_with("turn/")
                || method.starts_with("thread/") || method=="error";
            let params=match envelope.get(&k("params")) {
                Some(Json::Object(fields))=>fields,
                _ if !relevant=>{last=id;continue;},
                _=>return Err(OrchestrationError::V37StoreFailure(format!(
                    "Original native notification {id} lacks params."))),
            };
            let thread=if method=="thread/started" {
                let started=object(params.get(&k("thread")).ok_or_else(||
                    OrchestrationError::V37StoreFailure(format!("Original thread/started source {id} lacks thread.")))?)?;
                Some(string_field(started,"id")?)
            } else {
                match params.get(&k("threadId")) {
                    Some(Json::String(value))=>Some(value.to_well_formed_string()
                        .ok_or(OrchestrationError::Invalid("native event thread"))?),
                    None if !relevant=>None,
                    _=>return Err(OrchestrationError::V37StoreFailure(format!(
                        "Original native notification {id} lacks exact threadId."))),
                }
            };
            let Some(thread)=thread else {last=id;continue;};
            if thread!=selected.thread {return Err(OrchestrationError::V37StoreFailure(format!(
                "Original native notification {id} names another vendor thread.")));}
            let source_ref=self.visible_source_ref(id,selected)?;
            let mut trial=copy_array(&notifications);
            trial.push(Json::Object(BTreeMap::from([(k("notification"),copy_json(&notification)),
                (k("sourceRef"),copy_json(&source_ref))])));
            let trial_refs={let mut values=copy_array(&refs);values.push(copy_json(&source_ref));values};
            let shell=visible_native_event_envelope(copy_array(&trial),high,None,None,trial_refs,"PARTIAL",after);
            let mut trial_reply=copy_fields(reply);trial_reply.insert(k("response"),shell);
            if notifications.len()>=64 || Json::Object(trial_reply).canonical().len()+512>crate::ipc::MAX_FRAME_BYTES {
                if notifications.is_empty() {return Err(OrchestrationError::V37StoreFailure(format!(
                    "Original native notification {id} exceeds the USER response frame.")));}
                more=true;break;
            }
            notifications=trial;refs.push(source_ref);last=id;
        }
        let state=if more {"PARTIAL"} else {"COMPLETE"};
        let next=if more {Some(visible_native_event_page_token(high,last,&scope))} else {None};
        let resume=if more {None} else {Some(visible_native_event_resume_token(high,&scope))};
        reply.insert(k("state"),s(if more {"PARTIAL"} else {"APPLIED"}));
        reply.insert(k("response"),visible_native_event_envelope(notifications,high,
            next.as_deref(),resume.as_deref(),refs,state,after));
        Ok(())
    }
    fn visible_original_thread(&self, selected: &Selection) -> Result<Json> {
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT hex(raw_bytes) FROM main.v37_ledger_raw_source WHERE rowid=?1 AND operation_id=?2")?;
        query.bind_i64(1,selected.ack)?;query.bind_text(2,&selected.open_operation)?;
        if !query.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let source=source_json(&query.column_text(0)?)?;
        let envelope=object(&source)?;
        let result=object(envelope.get(&k("result")).ok_or(OrchestrationError::OperationConflict)?)?;
        let thread=copy_json(result.get(&k("thread")).ok_or(OrchestrationError::OperationConflict)?);
        if string_field(object(&thread)?,"id")?!=selected.thread {return Err(OrchestrationError::OperationConflict);}
        Ok(thread)
    }
    fn visible_source_ref(&self, id: i64, selected: &Selection) -> Result<Json> {
        let association=selected.association.as_ref().ok_or(OrchestrationError::AccessDenied)?;
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT operation_id,generation,source_epoch,source_cursor FROM main.v37_ledger_raw_source WHERE rowid=?1 AND domain_id=?2 AND session_id=?3")?;
        query.bind_i64(1,id)?;query.bind_text(2,&association.domain)?;query.bind_text(3,&association.session)?;
        if !query.step_row()? {return Err(OrchestrationError::OperationConflict);}
        Ok(Json::Object(BTreeMap::from([(k("rawSourceId"),s(&id.to_string())),
            (k("operationId"),s(&query.column_text(0)?)),(k("generation"),s(&query.column_text(1)?)),
            (k("sourceEpoch"),s(&query.column_text(2)?)),(k("sourceCursor"),s(&query.column_text(3)?))])))
    }
    /// Input membership comes from the exact ACK already selected by the
    /// snapshot raw-source query, never from current H receipt totals/phases.
    fn visible_snapshot_input(&self,selected:&Selection,source:&Statement,step_id:&str,
        command_bytes:&[u8],operation:&str)->Result<bool> {
        use crate::store::session_transport::{decode_request,codex_rpc};
        let association=selected.association.as_ref().ok_or(OrchestrationError::AccessDenied)?;
        let inputs=Statement::prepare(self.connection.as_ptr(),
            "SELECT request_id,request_hex FROM main.gogoke_v37_h_stdin_journal WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3 AND generation=?4 AND ticket=?5 AND custodian_nonce=?6 AND operation=?7")?;
        for (index,value) in [association.domain.as_str(),association.session.as_str(),source.column_text(3)?.as_str(),
            source.column_text(4)?.as_str(),source.column_text(11)?.as_str(),source.column_text(13)?.as_str(),operation]
            .iter().enumerate() {inputs.bind_text((index+1) as i32,value)?;}
        let mut matched=false;
        while inputs.step_row()? {
            let raw=decode_hex(&inputs.column_text(1)?)?;
            let expected=format!("{}-{}",if operation=="send" {"send"} else {"append"},&crate::store::digest::sha256_hex(&raw)[..40]);
            if expected!=step_id {continue;}
            let request=decode_request(&raw).map_err(|cause|OrchestrationError::V37StoreFailure(
                format!("original snapshot H input decode: {cause:?}")))?;
            let (_,command)=(if operation=="send" {codex_rpc::decode_stored_turn_start(command_bytes)}
                else {codex_rpc::decode_stored_append(command_bytes)})
                .map_err(|cause|OrchestrationError::V37StoreFailure(format!("original snapshot H command decode: {cause:?}")))?;
            let (thread,text)=match command {
                codex_rpc::Command::TurnStart {thread_id,text,..}|codex_rpc::Command::AppendWithoutTurn {thread_id,text}=>(thread_id,text),
                _=>return Err(OrchestrationError::OperationConflict),
            };
            if matched||request.family!="K-SESSION"||request.operation!=operation
                ||request.request_id!=inputs.column_text(0)?||request.domain_id!=association.domain
                ||request.target_id!=association.session||request.payload.len()!=2
                ||!is_text(request.payload.get(&k("generation")),&source.column_text(4)?)
                ||!is_text(request.payload.get(&k("body")),&text)||thread!=selected.thread {
                return Err(OrchestrationError::OperationConflict);
            }
            matched=true;
        }
        Ok(matched)
    }
    /// Each page is rebuilt against its first page's immutable raw-source
    /// high-water. Complete original vendor Turn objects are the page unit;
    /// partial notifications are never promoted to a complete empty history.
    fn visible_thread_page(&self, workspace: &str, selected: &Selection,
        params: &BTreeMap<JsonString,Json>, reply: &mut BTreeMap<JsonString,Json>) -> Result<()> {
        let association=selected.association.as_ref().ok_or(OrchestrationError::AccessDenied)?;
        let maximum=self.visible_high_water(selected)?;
        let scope=format!("thread/read\n{workspace}\n{}\n{}\n{}",selected.row,selected.thread,association.json().canonical());
        let (high,after)=visible_page_position(params,maximum,&scope)?;
        if high<selected.ack.max(selected.started) {return Err(OrchestrationError::AccessDenied);}
        let mut thread=self.visible_original_thread(selected)?;
        let thread_fields=object(&thread)?;
        let mut turns=match thread_fields.get(&k("turns")) {
            Some(Json::Array(turns))=>copy_array(turns),
            _=>{
                reply.insert(k("state"),s("UNSUPPORTED"));reply.insert(k("reason"),s(
                    "The original qualified thread/start ACK lacks a complete turns array; history cannot be inferred from it."));return Ok(());
            },
        };
        let mut turn_refs:Vec<Vec<Json>>=turns.iter().map(|_|Vec::new()).collect();
        let mut turn_ids=Vec::new();
        for turn in &turns {turn_ids.push(string_field(object(turn)?,"id")?);}
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT r.rowid,hex(r.raw_bytes),r.state,r.operation_id,r.generation,r.source_epoch,r.source_cursor,COALESCE(e.seat_id,''),COALESCE(e.seat_incarnation,''),COALESCE(e.instance_id,''),COALESCE(c.ticket,''),r.process_ticket,COALESCE(c.custodian_nonce,''),r.custodian_nonce FROM main.v37_ledger_raw_source r LEFT JOIN main.gogoke_v37_h_generation g ON g.domain_id=r.domain_id AND g.session_id=r.session_id AND g.generation=r.generation AND g.process_operation_id=r.operation_id LEFT JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=g.domain_id AND e.request_id=g.request_id AND e.session_id=g.session_id AND e.generation=g.generation AND e.process_operation_id=g.process_operation_id LEFT JOIN main.gogoke_coordination_process_custody c ON c.operation_id=r.operation_id AND c.domain_id=r.domain_id AND c.generation=r.generation WHERE r.domain_id=?1 AND r.session_id=?2 AND r.rowid<=?3 ORDER BY r.rowid")?;
        query.bind_text(1,&association.domain)?;query.bind_text(2,&association.session)?;query.bind_i64(3,high)?;
        let mut complete=true;
        let mut original_reason=None;
        let mut observed_items:BTreeMap<String,Vec<(String,Option<Json>,Option<Json>)>>=BTreeMap::new();
        let mut observed_turn_acks:Vec<(String,i64)>=Vec::new();
        while query.step_row()? {
            let row=decimal(&query.column_text(0)?)?;
            if query.column_text(7)?!=association.seat || query.column_text(8)?!=association.incarnation
                || query.column_text(9)?!=association.instance || query.column_text(10)?!=query.column_text(11)?
                || query.column_text(12)?!=query.column_text(13)? {
                complete=false;original_reason=Some(format!("Original raw source {row} has no matching H generation/episode/custody identity."));continue;
            }
            let source=source_json(&query.column_text(1)?)?;
            let envelope=object(&source)?;
            let Some(Json::String(method))=envelope.get(&k("method")) else {
                // Correlate the original response to the exact retained command;
                // string "7" and numeric 7 remain distinct JSON-RPC identities.
                let step=Statement::prepare(self.connection.as_ptr(),
                    "SELECT command_hex,step_id FROM main.gogoke_v37_rpc_steps WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3 AND generation=?4 AND source_epoch=?5 AND source_cursor=?6 AND ticket=?7 AND custodian_nonce=?8 AND phase='OBSERVED'")?;
                for (index,value) in [association.domain.as_str(),association.session.as_str(),
                    query.column_text(3)?.as_str(),query.column_text(4)?.as_str(),query.column_text(5)?.as_str(),
                    query.column_text(6)?.as_str(),query.column_text(11)?.as_str(),query.column_text(13)?.as_str()].iter().enumerate() {step.bind_text((index+1) as i32,value)?;}
                if step.step_row()? {
                    let command_bytes=decode_hex(&step.column_text(0)?)?;
                    let step_id=step.column_text(1)?;
                    let command=source_json(&encode_hex(&command_bytes))?;let command=object(&command)?;
                    if !same_json(command.get(&k("id")),envelope.get(&k("id"))) || step.step_row()? {
                        complete=false;original_reason=Some(format!("Original RPC response {row} does not match its typed command ID."));continue;
                    }
                    if is_text(command.get(&k("method")),"turn/start") {
                        if !self.visible_snapshot_input(selected,&query,&step_id,&command_bytes,"send")? {
                            complete=false;original_reason=Some(format!("Original turn/start ACK source {row} has no exact original H send request."));continue;
                        }
                        let params=object(command.get(&k("params")).ok_or(OrchestrationError::OperationConflict)?)?;
                        if !is_text(params.get(&k("threadId")),&selected.thread) {return Err(OrchestrationError::AccessDenied);}
                        let result=object(envelope.get(&k("result")).ok_or_else(||OrchestrationError::V37StoreFailure(
                            format!("Original turn/start response source {row}: {}",source.canonical())))?)?;
                        let turn=result.get(&k("turn")).ok_or(OrchestrationError::OperationConflict)?;
                        let id=string_field(object(turn)?,"id")?;
                        observed_turn_acks.push((id.clone(),row));
                        if !turn_ids.contains(&id) {
                            turn_ids.push(id);turns.push(copy_json(turn));turn_refs.push(Vec::new());
                        }
                    } else if is_text(command.get(&k("method")),"thread/inject_items") {
                        let matched=self.visible_snapshot_input(selected,&query,&step_id,&command_bytes,"append-without-turn")?;
                        complete=false;original_reason=Some(format!("Original append-without-turn ACK source {row} {}.",
                            if matched {"has no qualified vendor history projection"} else {"has no exact original H input request"}));
                    }
                } else if query.column_text(2)?=="PENDING" {
                    complete=false;original_reason=Some(format!("Original RPC response {row} has no correlated H command."));
                }
                continue;
            };
            let method=method.to_well_formed_string().ok_or(OrchestrationError::Invalid("visible source method"))?;
            let Some(Json::Object(params))=envelope.get(&k("params")) else {continue;};
            if let Some(thread_id)=params.get(&k("threadId")) {
                if !is_text(Some(thread_id),&selected.thread) {
                    complete=false;original_reason=Some(format!("Original A raw source {row} names another vendor thread."));continue;
                }
            }
            if method=="turn/started" || method=="turn/completed" {
                if !is_text(params.get(&k("threadId")),&selected.thread) {
                    complete=false;original_reason=Some(format!("Original A turn source {row} has no exact vendor thread."));continue;
                }
                let Some(turn)=params.get(&k("turn")) else {return Err(OrchestrationError::OperationConflict);};
                let fields=object(turn)?;let id=string_field(fields,"id")?;
                let index=if let Some(index)=turn_ids.iter().position(|old|old==&id) {index}
                    else {turn_ids.push(id.clone());turns.push(copy_json(turn));turn_refs.push(Vec::new());turns.len()-1};
                turns[index]=copy_json(turn);
                turn_refs[index]=vec![self.visible_source_ref(row,selected)?];
            } else if method=="item/started" || method=="item/completed" {
                if !is_text(params.get(&k("threadId")),&selected.thread) {return Err(OrchestrationError::AccessDenied);}
                let turn=string_field(params,"turnId")?;
                let item=params.get(&k("item")).ok_or(OrchestrationError::OperationConflict)?;
                let id=string_field(object(item)?,"id")?;
                let items=observed_items.entry(turn).or_default();
                let index=if let Some(index)=items.iter().position(|(old,_,_)|old==&id) {index}
                    else {items.push((id,None,None));items.len()-1};
                if method=="item/completed" {items[index].1=Some(copy_json(item));items[index].2=Some(self.visible_source_ref(row,selected)?);}
            } else if method.starts_with("item/") {
                if let (Some(Json::String(turn)),Some(Json::String(item)))=(params.get(&k("turnId")),params.get(&k("itemId"))) {
                    let turn=turn.to_well_formed_string().ok_or(OrchestrationError::Invalid("visible turn id"))?;
                    let item=item.to_well_formed_string().ok_or(OrchestrationError::Invalid("visible item id"))?;
                    let items=observed_items.entry(turn).or_default();
                    if !items.iter().any(|(id,_,_)|id==&item) {items.push((item,None,None));}
                }
            }
        }
        for (turn,items) in observed_items {
            let Some(index)=turn_ids.iter().position(|id|id==&turn) else {
                complete=false;original_reason=Some(format!("Original item sources for turn {turn} have no matching original turn ID."));continue;
            };
            let fields=object(&turns[index])?;
            let mut projection=copy_fields(fields);
            let mut projected=match fields.get(&k("items")) {Some(Json::Array(items))=>copy_array(items),
                _=>{complete=false;original_reason=Some(format!("Original turn {turn} has no vendor items field."));Vec::new()}};
            for (id,snapshot,source) in items {
                if let Some(snapshot)=snapshot {
                    if let Some(index)=projected.iter().position(|item|matches!(item,Json::Object(fields)
                        if is_text(fields.get(&k("id")),&id))) {projected[index]=snapshot;}
                    else {projected.push(snapshot);}
                    turn_refs[index].push(source.ok_or(OrchestrationError::OperationConflict)?);
                } else if !projected.iter().any(|item|matches!(item,Json::Object(fields) if is_text(fields.get(&k("id")),&id))) {
                    complete=false;original_reason=Some(format!("Original item {id} in turn {turn} has no final vendor item snapshot."));
                }
            }
            projection.insert(k("items"),Json::Array(projected));turns[index]=Json::Object(projection);
        }
        for (turn,row) in &observed_turn_acks {
            let index=turn_ids.iter().position(|id|id==turn).ok_or(OrchestrationError::OperationConflict)?;
            // Retain the actual input ACK as well as later item/turn snapshots.
            turn_refs[index].push(self.visible_source_ref(*row,selected)?);
            let fields=object(&turns[index])?;
            if !matches!(fields.get(&k("items")),Some(Json::Array(items)) if items.iter().any(|item|
                matches!(item,Json::Object(fields) if is_text(fields.get(&k("type")),"userMessage")))) {
                complete=false;original_reason=Some(format!("Original turn/start ACK for turn {turn} has no actual userMessage item source."));
            }
        }
        if after as usize>turns.len() {return Err(OrchestrationError::Invalid("visible page position"));}
        let mut page=Vec::new();let mut refs=Vec::new();
        if after==0 {refs.push(self.visible_source_ref(selected.ack,selected)?);refs.push(self.visible_source_ref(selected.started,selected)?);}
        let mut index=after as usize;
        let Json::Object(ref mut metadata)=thread else {return Err(OrchestrationError::OperationConflict);};
        metadata.remove(&k("turns"));
        while index<turns.len() {
            let mut trial=copy_array(&page);trial.push(copy_json(&turns[index]));
            let mut trial_refs=copy_array(&refs);trial_refs.extend(copy_array(&turn_refs[index]));
            let mut trial_thread=copy_fields(metadata);trial_thread.insert(k("turns"),Json::Array(copy_array(&trial)));
            let shell=visible_read_envelope(Json::Object(trial_thread),high,Some("reserved-continuation-token"),copy_array(&trial_refs),"PARTIAL");
            let mut trial_reply=copy_fields(reply);trial_reply.insert(k("response"),shell);
            // Reserve the full opaque continuation and possible original error.
            if Json::Object(trial_reply).canonical().len()+512>crate::ipc::MAX_FRAME_BYTES {
                if page.is_empty() {
                    reply.insert(k("state"),s("UNKNOWN"));reply.insert(k("reason"),s(
                        "An original vendor turn exceeds the USER response frame; this producer cannot provide complete history without splitting the original turn."));return Ok(());
                }
                break;
            }
            page=trial;refs=trial_refs;index+=1;
        }
        let more=index<turns.len();
        let state=if !complete {"UNKNOWN"} else if more {"PARTIAL"} else {"APPLIED"};
        let history_state=if !complete {"UNKNOWN"} else if more {"PARTIAL"} else {"COMPLETE"};
        let cursor=if more && complete {Some(visible_page_token(high,index as i64,&scope))} else {None};
        metadata.insert(k("turns"),Json::Array(page));
        reply.insert(k("state"),s(state));
        if !complete {reply.insert(k("reason"),s(original_reason.as_deref().unwrap_or(
            "Original conversation sources cannot be completely projected.")));}
        reply.insert(k("response"),visible_read_envelope(thread,high,cursor.as_deref(),refs,history_state));
        Ok(())
    }

    fn visible_thread_list(&self, workspace: &str, association: &Association,
        params: &BTreeMap<JsonString,Json>, reply: &mut BTreeMap<JsonString,Json>) -> Result<()> {
        let limit=match params.get(&k("limit")) {
            None|Some(Json::Null)=>100usize,
            Some(Json::Number(value))=>{let count=decimal(value)?;
                if !(1..=100).contains(&count) {return Err(OrchestrationError::Invalid("visible list limit"));}count as usize},
            _=>return Err(OrchestrationError::Invalid("visible list limit")),
        };
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT COALESCE(MAX(rowid),0) FROM main.gogoke_v37_visible_conversation_selection WHERE workspace_id=?1")?;
        query.bind_text(1,workspace)?;
        if !query.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let maximum=decimal(&query.column_text(0)?)?;
        let scope=format!("thread/list\n{workspace}\n{}",association.json().canonical());
        let (high,after)=visible_page_position(params,maximum,&scope)?;
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT rowid,route,association_json,repository_id,worktree_id,thread_id,open_request_id,open_generation,open_operation_id,ack_source_id,started_source_id FROM main.gogoke_v37_visible_conversation_selection WHERE workspace_id=?1 AND route='NATIVE' AND rowid<=?2 ORDER BY rowid")?;
        query.bind_text(1,workspace)?;query.bind_i64(2,high)?;
        let mut all=Vec::new();let mut identities=Vec::new();
        while query.step_row()? {
            let selected=Selection::from_row(&query)?;
            if let Err(error)=self.visible_verify_saved(&selected) {
                reply.insert(k("state"),s("UNKNOWN"));reply.insert(k("reason"),s(&format!(
                    "Original workspace historical selection {} is unresolved: {error:?}",selected.row)));return Ok(());
            }
            let bound=selected.association.as_ref().ok_or(OrchestrationError::AccessDenied)?;
            let identity=(bound.domain.clone(),bound.session.clone(),selected.thread.clone());
            if !identities.contains(&identity) {identities.push(identity);all.push(selected);}
        }
        if after as usize>all.len() {return Err(OrchestrationError::Invalid("visible list position"));}
        let mut data=Vec::new();let mut refs=Vec::new();let mut index=after as usize;
        while index<all.len() && data.len()<limit {
            let selected=&all[index];let mut thread=self.visible_original_thread(selected)?;
            if let Json::Object(ref mut fields)=thread {fields.insert(k("turns"),Json::Array(Vec::new()));}
            let mut trial=copy_array(&data);trial.push(copy_json(&thread));
            let mut trial_refs=copy_array(&refs);trial_refs.push(self.visible_source_ref(selected.ack,selected)?);
            let shell=Json::Object(BTreeMap::from([(k("result"),Json::Object(BTreeMap::from([
                (k("data"),Json::Array(trial)),(k("nextCursor"),s("reserved-continuation-token")),
                (k("nativeHistory"),visible_history(high,Some("reserved-continuation-token"),trial_refs,"PARTIAL")),
            ])))]));
            let mut trial_reply=copy_fields(reply);trial_reply.insert(k("response"),shell);
            if Json::Object(trial_reply).canonical().len()+512>crate::ipc::MAX_FRAME_BYTES {
                if data.is_empty() {reply.insert(k("state"),s("UNKNOWN"));reply.insert(k("reason"),s("Original thread metadata exceeds the USER response frame."));return Ok(());}break;
            }
            data.push(thread);refs.push(self.visible_source_ref(selected.ack,selected)?);index+=1;
        }
        let more=index<all.len();let cursor=if more {Some(visible_page_token(high,index as i64,&scope))} else {None};
        reply.insert(k("state"),s(if more {"PARTIAL"} else {"APPLIED"}));
        reply.insert(k("response"),Json::Object(BTreeMap::from([(k("result"),Json::Object(BTreeMap::from([
            (k("data"),Json::Array(data)),(k("nextCursor"),cursor.as_deref().map(s).unwrap_or(Json::Null)),
            (k("nativeHistory"),visible_history(high,cursor.as_deref(),refs,if more {"PARTIAL"} else {"COMPLETE"})),
        ])))])));
        Ok(())
    }
}

fn visible_history(high: i64, cursor: Option<&str>, refs: Vec<Json>, state: &str) -> Json {
    Json::Object(BTreeMap::from([(k("state"),s(state)),(k("highWater"),s(&high.to_string())),
        (k("nextCursor"),cursor.map(s).unwrap_or(Json::Null)),(k("sourceRefs"),Json::Array(refs))]))
}
fn visible_read_envelope(thread: Json, high: i64, cursor: Option<&str>, refs: Vec<Json>, state: &str) -> Json {
    Json::Object(BTreeMap::from([(k("result"),Json::Object(BTreeMap::from([(k("thread"),thread),
        (k("nativeHistory"),visible_history(high,cursor,refs,state))])))]))
}
fn visible_native_event_envelope(notifications: Vec<Json>, high: i64,
    next: Option<&str>, resume: Option<&str>, refs: Vec<Json>, state: &str, after: i64) -> Json {
    Json::Object(BTreeMap::from([(k("result"),Json::Object(BTreeMap::from([
        (k("notifications"),Json::Array(notifications)),
        (k("nativeEvents"),Json::Object(BTreeMap::from([
            (k("state"),s(state)),(k("highWater"),s(&high.to_string())),
            (k("afterSourceId"),s(&after.to_string())),
            (k("nextCursor"),next.map(s).unwrap_or(Json::Null)),
            (k("resumeCursor"),resume.map(s).unwrap_or(Json::Null)),
            (k("sourceRefs"),Json::Array(refs)),
        ]))),
    ])))]))
}
fn visible_native_event_unknown() -> Json {
    Json::Object(BTreeMap::from([(k("result"),Json::Object(BTreeMap::from([
        (k("notifications"),Json::Array(Vec::new())),
        (k("nativeEvents"),Json::Object(BTreeMap::from([
            (k("state"),s("UNKNOWN")),(k("highWater"),Json::Null),
            (k("afterSourceId"),Json::Null),(k("nextCursor"),Json::Null),
            (k("resumeCursor"),Json::Null),(k("sourceRefs"),Json::Array(Vec::new())),
        ]))),
    ])))]))
}
fn visible_native_event_page_token(high: i64, after: i64, scope: &str) -> String {
    let digest=crate::store::digest::sha256_hex(format!("native-page\n{scope}\n{high}\n{after}").as_bytes());
    format!("page:{high}:{after}:{digest}")
}
fn visible_native_event_resume_token(high: i64, scope: &str) -> String {
    let digest=crate::store::digest::sha256_hex(format!("native-resume\n{scope}\n{high}").as_bytes());
    format!("resume:{high}:{digest}")
}
fn visible_native_event_position(params: &BTreeMap<JsonString,Json>, maximum: i64,
    scope: &str) -> Result<(i64,i64)> {
    if params.contains_key(&k("cursor")) && params.contains_key(&k("resumeCursor")) {
        return Err(OrchestrationError::Invalid("native event cursors"));
    }
    if let Some(value)=params.get(&k("cursor")) {
        let Json::String(value)=value else {return Err(OrchestrationError::Invalid("native event page cursor"));};
        let token=value.to_well_formed_string().ok_or(OrchestrationError::Invalid("native event page cursor"))?;
        let parts:Vec<_>=token.split(':').collect();
        if parts.len()!=4 || parts[0]!="page" {return Err(OrchestrationError::Invalid("native event page cursor"));}
        let high=decimal(parts[1])?;let after=decimal(parts[2])?;
        if high>maximum || after==0 || after>=high || token!=visible_native_event_page_token(high,after,scope) {
            return Err(OrchestrationError::AccessDenied);
        }
        return Ok((high,after));
    }
    if let Some(value)=params.get(&k("resumeCursor")) {
        let Json::String(value)=value else {return Err(OrchestrationError::Invalid("native event resume cursor"));};
        let token=value.to_well_formed_string().ok_or(OrchestrationError::Invalid("native event resume cursor"))?;
        let parts:Vec<_>=token.split(':').collect();
        if parts.len()!=3 || parts[0]!="resume" {return Err(OrchestrationError::Invalid("native event resume cursor"));}
        let after=decimal(parts[1])?;
        if after>maximum || token!=visible_native_event_resume_token(after,scope) {
            return Err(OrchestrationError::AccessDenied);
        }
        return Ok((maximum,after));
    }
    Ok((maximum,0))
}
fn visible_page_token(high: i64, after: i64, scope: &str) -> String {
    let digest=crate::store::digest::sha256_hex(format!("{scope}\n{high}\n{after}").as_bytes());
    format!("{high}:{after}:{digest}")
}
fn visible_page_position(params: &BTreeMap<JsonString,Json>, maximum: i64, scope: &str) -> Result<(i64,i64)> {
    let cursor=match params.get(&k("cursor")) {None|Some(Json::Null)=>return Ok((maximum,0)),
        Some(Json::String(value))=>value.to_well_formed_string().ok_or(OrchestrationError::Invalid("visible cursor"))?,
        _=>return Err(OrchestrationError::Invalid("visible cursor")),
    };
    let parts:Vec<_>=cursor.split(':').collect();
    if parts.len()!=3 {return Err(OrchestrationError::Invalid("visible cursor"));}
    let high=decimal(parts[0])?;let after=decimal(parts[1])?;
    if high>maximum || after==0 || cursor!=visible_page_token(high,after,scope) {
        return Err(OrchestrationError::AccessDenied);
    }
    Ok((high,after))
}

#[cfg(all(test,windows))]
mod tests {
    use super::*;
    use crate::store::same_open::route_b_test_guard;

    fn fixture(run: impl FnOnce(&mut ProductDatabase<'_>)) {
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-visible-user-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();let database=path.join("state.sqlite");
        let mut product=ProductDatabase::open(&root,&database).unwrap();
        run(&mut product);product.close_checked().unwrap();drop(root);
        std::fs::remove_file(database).unwrap();
        std::fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).unwrap();
        if let Err(error)=std::fs::remove_dir(&path) {eprintln!("owned USER boundary fixture retained: {error}");}
    }
    fn reply(product: &mut ProductDatabase<'_>, frame: &str) -> Json {
        let bytes=product.configure_user_v37(frame.as_bytes()).unwrap();
        Parser::parse(std::str::from_utf8(&bytes).unwrap()).unwrap()
    }
    #[test]
    fn visible_user_selection_requires_explicit_route_and_preserves_original_bytes() {
        fixture(|product| {
            let route=r#"{"schema":"gogoke.37.owner-configuration.v1","command":"visible-conversation-route","workspaceId":"workspaceA"}"#;
            assert!(is_text(object(&reply(product,route)).unwrap().get(&k("state")),"NEEDS_SETUP"));
            let select=r#"{"schema":"gogoke.37.owner-configuration.v1","command":"visible-conversation-select","workspaceId":"workspaceA","requestId":"explicitLegacy","route":"LEGACY"}"#;
            let denied=reply(product,select);
            assert!(is_text(object(&denied).unwrap().get(&k("state")),"UNSUPPORTED"));
            assert!(object(&denied).unwrap().contains_key(&k("reason")));
            let count=Statement::prepare(product.connection.as_ptr(),"SELECT COUNT(*) FROM main.gogoke_v37_visible_conversation_selection").unwrap();
            assert!(count.step_row().unwrap());assert_eq!(count.column_text(0).unwrap(),"0");drop(count);
            // Retained pre-policy history is not a new model capability.
            let retained=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_visible_conversation_selection VALUES('workspaceA','explicitLegacy','LEGACY',?1,'null','','','','','','',0,0)").unwrap();
            retained.bind_text(1,&encode_hex(select.as_bytes())).unwrap();retained.step_done().unwrap();drop(retained);
            assert!(is_text(object(&reply(product,route)).unwrap().get(&k("state")),"NEEDS_SETUP"));
            assert!(product.configure_user_v37(format!("{select} ").as_bytes()).is_err(),"same request ID cannot rewrite original selection bytes");
            assert!(product.configure_user_v37(route.replace("workspaceA\"}","workspaceA\",\"sql\":\"DROP TABLE\"}").as_bytes()).is_err(),"configuration is a closed set, not a SQL transport");
            assert!(is_text(object(&reply(product,&route.replace("workspaceA","workspaceB"))).unwrap().get(&k("state")),"NEEDS_SETUP"),"workspace IDs never imply a shared route");
            product.connection.execute("CREATE TEMP TRIGGER injected_visible_effect BEFORE INSERT ON main.gogoke_v37_visible_conversation_selection BEGIN SELECT RAISE(ABORT,'injected USER journal effect'); END").unwrap();
            assert!(product.configure_user_v37(route.as_bytes()).is_err(),"the actual producer rejects extra schema effects");
            product.connection.execute("DROP TRIGGER temp.injected_visible_effect").unwrap();
            assert!(is_text(object(&reply(product,route)).unwrap().get(&k("state")),"NEEDS_SETUP"));
        });
    }
    #[test]
    fn visible_user_read_checks_workspace_and_every_association_field_before_sources() {
        fixture(|product| {
            let association=Association {domain:"projectA".into(),session:"nativeSession".into(),seat:"leadA".into(),
                incarnation:"incarnationA".into(),authorization:"1".into(),generation:"2".into(),instance:"instanceA".into()};
            let insert=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_visible_conversation_selection VALUES('workspaceA','selectionA','NATIVE','00',?1,'repositoryA','worktreeA','vendorThreadA','openA','1','processA',1,2)").unwrap();
            insert.bind_text(1,&association.json().canonical()).unwrap();insert.step_done().unwrap();drop(insert);
            let recover=|workspace: &str, association: &Json| format!(r#"{{"schema":"gogoke.37.owner-configuration.v1","command":"visible-conversation-recover","workspaceId":"{workspace}","requestId":"originalEffect","expectedAssociation":{}}}"#,association.canonical());
            for field in ["domainId","sessionId","seatId","incarnation","authorizationGeneration","bindingGeneration","instanceId"] {
                let mut changed=copy_fields(object(&association.json()).unwrap());
                changed.insert(k(field),s(if field.ends_with("Generation") {"9"} else {"foreignIdentity"}));
                let response=reply(product,&recover("workspaceA",&Json::Object(changed)));
                let response=object(&response).unwrap();
                assert!(is_text(response.get(&k("state")),"DENIED"),"association field {field} is an actual scope boundary");
                assert!(is_text(response.get(&k("requestId")),"originalEffect"));
                assert!(response.contains_key(&k("reason")));
            }
            let response=reply(product,&recover("workspaceB",&association.json()));
            assert!(is_text(object(&response).unwrap().get(&k("state")),"DENIED"));
            let malformed=recover("workspaceB",&association.json()).replace("\"bindingGeneration\":\"2\"","\"bindingGeneration\":2");
            assert!(product.configure_user_v37(malformed.as_bytes()).is_err(),"vendor/native identity fields do not coerce JSON number and string");
            let count=Statement::prepare(product.connection.as_ptr(),"SELECT COUNT(*) FROM main.gogoke_v37_visible_conversation_selection").unwrap();
            assert!(count.step_row().unwrap());assert_eq!(count.column_text(0).unwrap(),"1","denied recovery cannot create or resend an effect");
            drop(count);
            let old=product.visible_selection("workspaceA",Some(&association)).unwrap().unwrap();
            let switched=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_visible_conversation_selection VALUES('workspaceA','laterLegacy','LEGACY','00','null','','','','','','',0,0)").unwrap();
            switched.step_done().unwrap();drop(switched);
            let mut result=product.visible_reply("workspaceA","UNKNOWN",Some(&association),None);
            let error=product.visible_native_events("workspaceA",&association,&old,&BTreeMap::new(),&mut result).unwrap_err();
            assert!(format!("{error:?}").contains("selection changed"),
                "an older matching association cannot append events after workspace selection changed");
        });
    }
    #[test]
    fn visible_history_snapshot_uses_original_send_ack_not_later_applied_inputs() {
        use crate::store::session_transport::codex_rpc::{self,Command,RpcId,Reply,TurnStatus};
        fixture(|product| {
            let association=Association {domain:"projectA".into(),session:"sessionA".into(),seat:"leadA".into(),
                incarnation:"incarnationA".into(),authorization:"1".into(),generation:"1".into(),instance:"instanceA".into()};
            let selected=Selection {row:1,route:"NATIVE".into(),association:Some(association),repository:"repositoryA".into(),
                worktree:"worktreeA".into(),thread:"threadA".into(),open_request:"openA".into(),open_generation:"1".into(),
                open_operation:"processA".into(),ack:1,started:2};
            product.connection.execute("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase,stop_fact_id) VALUES('projectA','openA','sessionA','1',NULL,'00',0,1,'processA','instanceA','homeA','bindingA','leadA','incarnationA','STOPPED','syntheticFixtureStop')").unwrap();
            product.connection.execute("INSERT INTO main.gogoke_v37_h_generation VALUES('projectA','sessionA','1','openA','processA')").unwrap();
            product.connection.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,stop_proof_hash) VALUES('processA','pct1_ticketA','nonceA','11','1','fixture','sha256:fixture','profileA','projectA','1','STOPPED','syntheticFixtureStop')").unwrap();
            let raw=|cursor:&str,bytes:&[u8]| {
                let row=Statement::prepare(product.connection.as_ptr(),
                    "INSERT INTO main.v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,generation,source_epoch,source_cursor,raw_bytes,state) VALUES('processA','pct1_ticketA','nonceA','projectA','sessionA','1','nonceA',?1,?2,'PENDING')").unwrap();
                row.bind_text(1,cursor).unwrap();row.bind_blob(2,bytes).unwrap();row.step_done().unwrap();
            };
            // Synthetic protocol frames exercise the real producer without a
            // CLI process, ModelCallProof or inferred provider/stop fact.
            let open_ack=b"{\"id\":1,\"result\":{\"thread\":{\"id\":\"threadA\",\"cwd\":\"fixture\",\"turns\":[]}}}\n";
            let started=b"{\"method\":\"thread/started\",\"params\":{\"thread\":{\"id\":\"threadA\",\"cwd\":\"fixture\",\"turns\":[]}}}\n";
            let opened=Command::ThreadStart {cwd:"fixture".into(),model:"fixture".into()}.encode(Some(&RpcId::Number(1))).unwrap();
            assert_eq!(codex_rpc::decode_stored_thread_start(&opened,open_ack).unwrap(),"threadA");
            assert!(matches!(codex_rpc::decode(started,None).unwrap(),Reply::Event {method,..} if method=="thread/started"));
            raw("1",open_ack);raw("2",started);
            let open_rpc=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,command_hex,requires_response,phase,source_epoch,source_cursor) VALUES('projectA','sessionA','openA','thread-start','processA','pct1_ticketA','nonceA','11','1','fixture','sha256:fixture','profileA','1',?1,1,'OBSERVED','nonceA','1')").unwrap();
            open_rpc.bind_text(1,&encode_hex(&opened)).unwrap();open_rpc.step_done().unwrap();drop(open_rpc);
            let input=|id:&str,cursor:&str,text:&str,turn:&str,ack:&[u8]| {
                let previous=cursor.parse::<u64>().unwrap().checked_sub(1).unwrap().to_string();
                let next=cursor.parse::<u64>().unwrap().to_string();
                let request=format!(r#"{{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"send","requestId":"{id}","targetId":"sessionA","domainId":"projectA","expectedRevision":"{previous}","payload":{{"generation":"1","body":"{text}"}}}}"#);
                assert!(request.len()<=1024*1024,"original H request must fit the fixed 1 MiB bound");
                let rpc_id=RpcId::String(id.into());
                let command=Command::TurnStart {thread_id:"threadA".into(),cwd:"fixture".into(),model:"fixture".into(),
                    effort:"low".into(),text:text.into(),network_access:None};
                let encoded=command.encode(Some(&rpc_id)).unwrap();
                assert!(encoded.len()<=1024*1024,"original Codex command must fit the fixed 1 MiB bound");
                assert!(ack.len()<=1024*1024,"original Codex response must fit the fixed 1 MiB bound");
                let (stored_id,stored_command)=codex_rpc::decode_stored_turn_start(&encoded).unwrap();
                assert_eq!(stored_id,rpc_id);
                assert!(matches!(codex_rpc::decode(ack,Some((&stored_id,&stored_command))).unwrap(),
                    Reply::Turn {id,turn_id,status:TurnStatus::InProgress} if id==stored_id && turn_id==turn));
                let journal=Statement::prepare(product.connection.as_ptr(),
                    "INSERT INTO main.gogoke_v37_h_stdin_journal(domain_id,request_id,operation,ticket,process_operation_id,custodian_nonce,session_id,generation,request_hex,phase,receipt_hex,receipt_status,expected_revision,receipt_previous_revision,receipt_revision) VALUES('projectA',?1,'send','pct1_ticketA','processA','nonceA','sessionA','1',?2,'RECEIPTED','00','APPLIED',?3,?3,?4)").unwrap();
                journal.bind_text(1,id).unwrap();journal.bind_text(2,&encode_hex(request.as_bytes())).unwrap();
                journal.bind_text(3,&previous).unwrap();journal.bind_text(4,&next).unwrap();journal.step_done().unwrap();drop(journal);
                let step=format!("send-{}",&crate::store::digest::sha256_hex(request.as_bytes())[..40]);
                let rpc=Statement::prepare(product.connection.as_ptr(),
                    "INSERT INTO main.gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,command_hex,requires_response,phase,source_epoch,source_cursor) VALUES('projectA','sessionA','openA',?1,'processA','pct1_ticketA','nonceA','11','1','fixture','sha256:fixture','profileA','1',?2,1,'OBSERVED','nonceA',?3)").unwrap();
                rpc.bind_text(1,&step).unwrap();rpc.bind_text(2,&encode_hex(&encoded)).unwrap();rpc.bind_text(3,cursor).unwrap();rpc.step_done().unwrap();
            };
            // This synthetic source history checks pagination, not physical
            // H admission or an actual CLI session. Seven codec-valid frames
            // cross the 4 MiB USER page bound; each stays within 1 MiB.
            let large="x".repeat(700_000);
            for number in 0..7 {
                let id=format!("send{number}");let cursor=(number+3).to_string();
                let turn=format!("turn{number}");
                let ack=format!("{{\"id\":\"{id}\",\"result\":{{\"turn\":{{\"id\":\"{turn}\",\"status\":\"inProgress\",\"items\":[{{\"id\":\"message{number}\",\"type\":\"userMessage\",\"text\":\"{large}\"}}]}}}}}}\n");
                input(&id,&cursor,&large,&turn,ack.as_bytes());
                raw(&cursor,ack.as_bytes());
            }
            let mut first=product.visible_reply("workspaceA","UNKNOWN",selected.association.as_ref(),None);
            product.visible_thread_page("workspaceA",&selected,&BTreeMap::new(),&mut first).unwrap();
            assert!(is_text(first.get(&k("state")),"PARTIAL"));
            let response=object(first.get(&k("response")).unwrap()).unwrap();
            let result=object(response.get(&k("result")).unwrap()).unwrap();
            let history=object(result.get(&k("nativeHistory")).unwrap()).unwrap();
            let cursor=string_field(history,"nextCursor").unwrap();
            let fixed=BTreeMap::from([(k("cursor"),s(&cursor))]);
            let mut baseline=product.visible_reply("workspaceA","UNKNOWN",selected.association.as_ref(),None);
            product.visible_thread_page("workspaceA",&selected,&fixed,&mut baseline).unwrap();
            assert!(is_text(baseline.get(&k("state")),"APPLIED"));
            let later=b"{\"id\":\"sendLater\",\"result\":{\"turn\":{\"id\":\"turnLater\",\"status\":\"inProgress\",\"items\":[{\"id\":\"messageLater\",\"type\":\"userMessage\"}]}}}\n";
            input("sendLater","10","later body","turnLater",later);
            raw("10",later);
            let mut repeated=product.visible_reply("workspaceA","UNKNOWN",selected.association.as_ref(),None);
            product.visible_thread_page("workspaceA",&selected,&fixed,&mut repeated).unwrap();
            assert_eq!(Json::Object(baseline).canonical(),Json::Object(repeated).canonical(),"later real H input/ACK cannot change a page from its original high-water cursor");
            let mut fresh=product.visible_reply("workspaceA","UNKNOWN",selected.association.as_ref(),None);
            product.visible_thread_page("workspaceA",&selected,&BTreeMap::new(),&mut fresh).unwrap();
            assert!(is_text(fresh.get(&k("state")),"PARTIAL"));
            let response=object(fresh.get(&k("response")).unwrap()).unwrap();
            let result=object(response.get(&k("result")).unwrap()).unwrap();
            let history=object(result.get(&k("nativeHistory")).unwrap()).unwrap();
            let fresh_cursor=string_field(history,"nextCursor").unwrap();
            let mut fresh_continuation=product.visible_reply("workspaceA","UNKNOWN",selected.association.as_ref(),None);
            product.visible_thread_page("workspaceA",&selected,&BTreeMap::from([(k("cursor"),s(&fresh_cursor))]),&mut fresh_continuation).unwrap();
            assert!(Json::Object(fresh_continuation).canonical().contains("turnLater"),"fresh high-water must retain the later input's actual ACK");
        });
    }
    #[test]
    fn visible_history_cursor_is_bound_to_exact_scope_and_preserves_typed_rpc_ids() {
        let cursor=visible_page_token(55,3,"workspaceA / exact association / threadA");
        let params=BTreeMap::from([(k("cursor"),s(&cursor))]);
        assert_eq!(visible_page_position(&params,80,"workspaceA / exact association / threadA").unwrap(),(55,3));
        assert!(visible_page_position(&params,80,"workspaceB / exact association / threadA").is_err());
        assert!(visible_page_position(&params,54,"workspaceA / exact association / threadA").is_err());
        assert!(!same_json(Some(&Json::Number("7".into())),Some(&s("7"))));
    }
    #[test]
    fn native_event_cursor_separates_snapshot_page_from_fresh_resume() {
        let scope="workspaceA / selected USER association / current episode";
        let other="workspaceA / changed USER association / current episode";
        let page=visible_native_event_page_token(12,7,scope);
        let params=BTreeMap::from([(k("cursor"),s(&page))]);
        assert_eq!(visible_native_event_position(&params,14,scope).unwrap(),(12,7));
        assert!(visible_native_event_position(&params,14,other).is_err());
        let resume=visible_native_event_resume_token(12,scope);
        let params=BTreeMap::from([(k("resumeCursor"),s(&resume))]);
        assert_eq!(visible_native_event_position(&params,14,scope).unwrap(),(14,12));
        assert!(visible_native_event_position(&params,14,other).is_err());
        assert!(visible_native_event_position(&params,11,scope).is_err());
        let empty=visible_native_event_resume_token(0,scope);
        let params=BTreeMap::from([(k("resumeCursor"),s(&empty))]);
        assert_eq!(visible_native_event_position(&params,0,scope).unwrap(),(0,0));
        assert_eq!(visible_native_event_position(&params,5,scope).unwrap(),(5,0));
    }
}
