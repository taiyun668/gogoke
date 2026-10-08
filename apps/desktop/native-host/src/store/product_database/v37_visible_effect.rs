//! Finite USER actions over the existing E/F/H/C/A stores. This journal binds
//! original UI bytes to their exact native requests; it grants no model proof.
use super::*;
use super::v37_seat::string_field;
use super::v37_visible_conversation::{Association,Selection,k,s,object,exact,decimal,
    copy_json,copy_fields,is_text,encode_hex,decode_hex,source_json};
use crate::store::atomic::Parser;
use crate::store::session_transport::{self as h,codex_rpc::{Command,RpcId},rpc_journal};

const TABLE:&str="gogoke_v37_visible_effect";
const SCHEMA:&str="CREATE TABLE gogoke_v37_visible_effect(workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,domain_id TEXT NOT NULL,selection_row INTEGER NOT NULL,association_json TEXT NOT NULL,request_hex TEXT NOT NULL,method TEXT NOT NULL,params_json TEXT NOT NULL,translation_json TEXT NOT NULL,thread_id TEXT NOT NULL,turn_id TEXT NOT NULL,process_operation_id TEXT NOT NULL,ticket TEXT NOT NULL,custodian_nonce TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('UNKNOWN','APPLIED','DENIED','UNSUPPORTED')),reply_json TEXT NOT NULL,original_error TEXT NOT NULL,PRIMARY KEY(workspace_id,request_id),UNIQUE(domain_id,request_id)) STRICT";

pub(super) fn initialize_schema(db:&mut VerifiedDatabaseConnection<'_>)->Result<()> {
    if !schema_state(db)? {
        db.execute(SCHEMA).map_err(|cause|OrchestrationError::V37StoreFailure(
            format!("visible USER effect schema creation: {cause:?}")))?;
    }
    if !schema_state(db)? {return Err(OrchestrationError::Invalid("visible effect schema"));}Ok(())
}
fn schema_state(db:&VerifiedDatabaseConnection<'_>)->Result<bool> {
    let effect=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM temp.sqlite_schema WHERE lower(name)=?1 OR lower(tbl_name)=?1 UNION ALL SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND lower(tbl_name)=?1")?;
    effect.bind_text(1,TABLE)?;
    if effect.step_row()? {return Err(OrchestrationError::Invalid("visible effect schema effects"));}
    let row=Statement::prepare(db.as_ptr(),"SELECT type,sql FROM main.sqlite_schema WHERE lower(name)=?1")?;
    row.bind_text(1,TABLE)?;
    if !row.step_row()? {return Ok(false);}
    if row.column_text(0)?!="table"||row.column_text(1)?!=SCHEMA||row.step_row()? {
        return Err(OrchestrationError::Invalid("visible effect schema drift"));}Ok(true)
}

struct Action {
    workspace:String,id:String,selection:i64,association:Association,original:Vec<u8>,
    method:String,params:Json,translations:Vec<Vec<u8>>,thread:String,turn:String,
    operation:String,ticket:String,nonce:String,phase:String,reply:Json,error:String,
}
pub(super) struct VisibleInterruptPermission<'a,'origin> {
    input:&'a VerifiedDirectUserInput<'origin>,workspace:&'a str,id:&'a str,
}
impl VisibleInterruptPermission<'_,'_> {
    pub(super) fn verify_in_transaction(&self,db:&VerifiedDatabaseConnection<'_>,association:&Association,
        thread:&str,turn:&str,step:&str)->Result<()> {
        self.input.origin.verify_live_origin().map_err(OrchestrationError::Ipc)?;
        if self.input.visible_translation.is_some()||!schema_state(db)? {return Err(OrchestrationError::AccessDenied);}
        let action=read_action(db,self.workspace,self.id)?.ok_or(OrchestrationError::AccessDenied)?;
        if action.phase!="UNKNOWN"||action.method!="turn/interrupt"||action.original!=self.input.frame
            ||action.association!=*association||action.thread!=thread||action.turn!=turn
            ||step!=visible_control_step(self.workspace,self.id) {return Err(OrchestrationError::AccessDenied);}
        Ok(())
    }
}
fn read_action(db:&VerifiedDatabaseConnection<'_>,workspace:&str,id:&str)->Result<Option<Action>> {
    let row=Statement::prepare(db.as_ptr(),
        "SELECT selection_row,association_json,request_hex,method,params_json,translation_json,thread_id,turn_id,process_operation_id,ticket,custodian_nonce,phase,reply_json,original_error FROM main.gogoke_v37_visible_effect WHERE workspace_id=?1 AND request_id=?2")?;
    row.bind_text(1,workspace)?;row.bind_text(2,id)?;
    if !row.step_row()? {return Ok(None);}
    let Json::Array(translations)=Parser::parse(&row.column_text(5)?)? else {return Err(OrchestrationError::OperationConflict);};
    let translations=translations.iter().map(|value| {
        let Json::String(value)=value else {return Err(OrchestrationError::OperationConflict);};
        decode_hex(&value.to_well_formed_string().ok_or(OrchestrationError::OperationConflict)?)
    }).collect::<Result<Vec<_>>>()?;
    let action=Action {workspace:workspace.into(),id:id.into(),selection:decimal(&row.column_text(0)?)?,
        association:Association::parse(&Parser::parse(&row.column_text(1)?)?)?,original:decode_hex(&row.column_text(2)?)?,
        method:row.column_text(3)?,params:Parser::parse(&row.column_text(4)?)?,translations,thread:row.column_text(6)?,
        turn:row.column_text(7)?,operation:row.column_text(8)?,ticket:row.column_text(9)?,nonce:row.column_text(10)?,
        phase:row.column_text(11)?,reply:Parser::parse(&row.column_text(12)?)?,error:row.column_text(13)?};
    if row.step_row()? {return Err(OrchestrationError::OperationConflict);}Ok(Some(action))
}

fn input_body(params:&BTreeMap<JsonString,Json>)->Result<String> {
    let Some(Json::Array(input))=params.get(&k("input")) else {return Err(OrchestrationError::Invalid("visible text input"));};
    let [Json::Object(input)]=input.as_slice() else {return Err(OrchestrationError::Invalid("visible single text input"));};
    exact(input,&["type","text"],&[])?;
    if !is_text(input.get(&k("type")),"text") {return Err(OrchestrationError::Invalid("visible text input type"));}
    string_field(input,"text")
}
fn finite_params(method:&str,params:&BTreeMap<JsonString,Json>)->Result<()> {
    let fields:&[&str]=match method {
        "thread/start"|"physical-stop"=>&[],"thread/resume"=>&["threadId"],
        "turn/start"=>&["threadId","input"],"turn/steer"=>&["threadId","expectedTurnId","input"],
        "turn/interrupt"=>&["threadId","turnId"],"original-question-answer"=>&["requestId","result"],
        _=>return Err(OrchestrationError::Invalid("visible effect method")),
    };
    exact(params,fields,&[])?;
    if matches!(method,"turn/start"|"turn/steer") {input_body(params)?;}
    Ok(())
}
fn native_request(family:&str,operation:&str,id:&str,target:&str,domain:&str,revision:u64,
    payload:BTreeMap<JsonString,Json>)->Result<Vec<u8>> {
    let bytes=Json::Object(BTreeMap::from([(k("schema"),s("gogoke.37.operations.v1")),
        (k("family"),s(family)),(k("operation"),s(operation)),(k("requestId"),s(id)),
        (k("targetId"),s(target)),(k("domainId"),s(domain)),
        (k("expectedRevision"),s(&revision.to_string())),(k("payload"),Json::Object(payload))])).canonical().into_bytes();
    h::decode_request(&bytes).map_err(|error|OrchestrationError::V37StoreFailure(format!("finite visible native request: {error:?}")))?;
    Ok(bytes)
}
fn source_ref(operation:&str,epoch:&str,cursor:&str)->Json {
    Json::Object(BTreeMap::from([(k("operationId"),s(operation)),(k("sourceEpoch"),s(epoch)),(k("sourceCursor"),s(cursor))]))
}

/// Called again inside H's prepare transaction. This checks the actual
/// authenticated owner frame and the exact persisted send translation.
pub(super) fn verify_user_translation(db:&VerifiedDatabaseConnection<'_>,original:&[u8],
    lowered:&[u8],workspace:&str,id:&str)->Result<bool> {
    if !schema_state(db)? {return Ok(false);}
    let Some(action)=read_action(db,workspace,id)? else {return Ok(false);};
    if action.original!=original || action.method!="turn/start" || action.phase!="UNKNOWN"
        || action.translations.len()!=1 || action.translations[0]!=lowered {return Ok(false);}
    let Json::Object(fields)=Parser::parse(std::str::from_utf8(original).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("original USER translation UTF-8: {error}")))?)? else {return Ok(false);};
    exact(&fields,&["schema","command","workspaceId","requestId","expectedAssociation","method","params"],&[])?;
    if !is_text(fields.get(&k("schema")),"gogoke.37.owner-configuration.v1")
        || !is_text(fields.get(&k("command")),"visible-conversation-operate")
        || !is_text(fields.get(&k("workspaceId")),workspace)||!is_text(fields.get(&k("requestId")),id)
        || !is_text(fields.get(&k("method")),"turn/start") {return Ok(false);}
    let association=Association::parse(fields.get(&k("expectedAssociation")).ok_or(OrchestrationError::AccessDenied)?)?;
    let params=object(fields.get(&k("params")).ok_or(OrchestrationError::AccessDenied)?)?;
    finite_params("turn/start",params)?;
    let request=h::decode_request(lowered).map_err(|error|OrchestrationError::V37StoreFailure(format!("original USER translated H request: {error:?}")))?;
    Ok(association==action.association && request.family=="K-SESSION"&&request.operation=="send"
        &&request.request_id==id&&request.domain_id==association.domain&&request.target_id==association.session
        &&request.payload.len()==2&&user_payload_string(&request,"generation")?==association.generation
        &&user_payload_string(&request,"body")?==input_body(params)?&&string_field(params,"threadId")?==action.thread)
}

impl<'root> ProductDatabase<'root> {
    pub(super) fn dispatch_visible_effect(&mut self,command:&str,fields:&BTreeMap<JsonString,Json>,
        frame:&[u8],user_input:Option<&VerifiedDirectUserInput<'_>>)->Result<Vec<u8>> {
        let workspace=string_field(fields,"workspaceId")?;let id=string_field(fields,"requestId")?;
        let association=Association::parse(fields.get(&k("expectedAssociation")).ok_or(OrchestrationError::AccessDenied)?)?;
        let recover=command=="visible-conversation-recover";
        exact(fields,if recover {&["schema","command","workspaceId","requestId","expectedAssociation"]}
            else {&["schema","command","workspaceId","requestId","expectedAssociation","method","params"]},&[])?;
        if workspace.len()>128 || !(1..=64).contains(&id.len()) || !id.as_bytes()[0].is_ascii_alphabetic()
            || !id.bytes().all(|byte|byte.is_ascii_alphanumeric()||matches!(byte,b'-'|b'_')) {
            return Err(OrchestrationError::Invalid("visible original request identity"));
        }
        let denied=|product:&Self,reason:&str| {
            let mut reply=product.visible_reply(&workspace,"DENIED",Some(&association),Some(reason));
            reply.insert(k("requestId"),s(&id));Json::Object(reply).canonical().into_bytes()
        };
        let Some(input)=user_input else {return Ok(denied(self,"The original authenticated USER process proof is required."));};
        if input.visible_translation.is_some() || !input.matches_live_frame(frame).map_err(OrchestrationError::Ipc)? {
            return Ok(denied(self,"The supplied frame is not the exact original authenticated USER frame."));
        }
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let prepared=(||->Result<(Action,bool)> {
            if !schema_state(&self.connection)? {return Err(OrchestrationError::Invalid("visible effect schema"));}
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            if let Some(action)=read_action(&self.connection,&workspace,&id)? {
                if action.association!=association || (!recover&&action.original!=frame) {return Err(OrchestrationError::AccessDenied);}
                self.verify_visible_action_history(&action)?;return Ok((action,false));
            }
            if recover {return Err(OrchestrationError::Invalid("original visible request is not recorded"));}
            let method=string_field(fields,"method")?;let params=fields.get(&k("params")).ok_or(OrchestrationError::AccessDenied)?;
            finite_params(&method,object(params)?)?;
            let selected=self.visible_selection(&workspace,Some(&association))?.ok_or(OrchestrationError::AccessDenied)?;
            let latest=self.visible_selection(&workspace,None)?.ok_or(OrchestrationError::AccessDenied)?;
            if latest.row!=selected.row {
                return Err(OrchestrationError::V37StoreFailure(
                    "Original workspace selection changed before new visible action.".into()));
            }
            self.visible_verify_saved(&selected)?;self.visible_candidate(&association,true)?;
            let action=self.prepare_visible_action(&workspace,&id,frame,&association,&selected,&method,params)?;
            self.insert_visible_action(&action)?;Ok((action,true))
        })();
        let (action,new)=match prepared {
            Ok(value)=>{self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;value},
            Err(error)=>{self.connection.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?;
                return Ok(denied(self,&format!("Original USER action association/translation: {error:?}")));},
        };
        if !new {return self.recover_visible_action(&action);}
        let observed=self.execute_visible_action(&action,input);
        match observed {
            Ok(reply)=>self.store_visible_reply(&action,reply),
            Err(error)=>{
                let reason=format!("Original visible {} request {}: {error:?}",action.method,action.id);
                let reply=self.action_reply(&action,"UNKNOWN",Some(&reason),None);
                self.store_visible_reply(&action,reply)
            },
        }
    }
    fn verify_visible_action_history(&self,action:&Action)->Result<()> {
        // Mutable current E choice/state and live maps cannot erase a durable
        // old USER result. The original selected row and immutable H binding
        // still have to match all seven fields and the original vendor thread.
        let row=Statement::prepare(self.connection.as_ptr(),
            "SELECT association_json,thread_id FROM main.gogoke_v37_visible_conversation_selection WHERE workspace_id=?1 AND rowid=?2 AND route='NATIVE'")?;
        row.bind_text(1,&action.workspace)?;row.bind_i64(2,action.selection)?;
        if !row.step_row()? || row.column_text(0)?!=action.association.json().canonical()
            || row.column_text(1)?!=action.thread || row.step_row()? {return Err(OrchestrationError::AccessDenied);}
        let binding=h::session_binding::read(&self.connection,&action.association.domain,&action.association.session)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("original visible H binding: {error:?}")))?
            .ok_or(OrchestrationError::AccessDenied)?;
        if binding.provenance!=h::session_binding::Provenance::NativeV2||binding.seat_id!=action.association.seat
            ||binding.seat_incarnation!=action.association.incarnation||binding.selected_instance_id!=action.association.instance
            ||binding.seat_authorization_generation.to_string()!=action.association.authorization {
            return Err(OrchestrationError::AccessDenied);
        }
        let custody=Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_h_generation g ON g.domain_id=e.domain_id AND g.session_id=e.session_id AND g.generation=e.generation AND g.process_operation_id=e.process_operation_id JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id AND c.generation=e.generation WHERE e.domain_id=?1 AND e.session_id=?2 AND e.generation=?3 AND e.seat_id=?4 AND e.seat_incarnation=?5 AND e.instance_id=?6 AND e.process_operation_id=?7 AND c.ticket=?8 AND c.custodian_nonce=?9")?;
        for (index,value) in [action.association.domain.as_str(),action.association.session.as_str(),action.association.generation.as_str(),
            action.association.seat.as_str(),action.association.incarnation.as_str(),action.association.instance.as_str(),
            action.operation.as_str(),action.ticket.as_str(),action.nonce.as_str()].iter().enumerate() {custody.bind_text((index+1) as i32,value)?;}
        if !custody.step_row()? || custody.step_row()? {return Err(OrchestrationError::AccessDenied);}Ok(())
    }
    fn prepare_visible_action(&mut self,workspace:&str,id:&str,frame:&[u8],association:&Association,
        selected:&Selection,method:&str,params:&Json)->Result<Action> {
        let fields=object(params)?;
        if fields.contains_key(&k("threadId"))&&!is_text(fields.get(&k("threadId")),&selected.thread) {
            return Err(OrchestrationError::AccessDenied);
        }
        let claim=Statement::prepare(self.connection.as_ptr(),
            "SELECT h.revision,h.state,c.operation_id,c.ticket,c.custodian_nonce FROM main.gogoke_v37_h_claim h JOIN main.gogoke_coordination_process_custody c ON c.operation_id=h.process_operation_id AND c.domain_id=h.domain_id AND c.generation=h.generation WHERE h.domain_id=?1 AND h.session_id=?2 AND h.generation=?3 AND h.instance_id=?4")?;
        for (index,value) in [association.domain.as_str(),association.session.as_str(),association.generation.as_str(),association.instance.as_str()].iter().enumerate() {claim.bind_text((index+1) as i32,value)?;}
        if !claim.step_row()? {return Err(OrchestrationError::AccessDenied);}
        let revision=decimal(&claim.column_text(0)?)? as u64;let state=claim.column_text(1)?;
        let operation=claim.column_text(2)?;let ticket=claim.column_text(3)?;let nonce=claim.column_text(4)?;
        if claim.step_row()? {return Err(OrchestrationError::OperationConflict);}drop(claim);
        let mut turn=String::new();let mut translations=Vec::new();
        match method {
            "thread/start"=>{},
            "thread/resume"=>{
                if state!="STOPPED" {return Err(OrchestrationError::Invalid("selected H session is not positively stopped for resume"));}
                translations.push(native_request("K-SESSION","resume",id,&association.session,&association.domain,revision,
                    BTreeMap::from([(k("generation"),s(&association.generation))]))?);
            },
            "turn/start"=>{
                self.verify_visible_runtime_target(association,&selected.thread,None,true)?;
                translations.push(native_request("K-SESSION","send",id,&association.session,&association.domain,revision,
                    BTreeMap::from([(k("generation"),s(&association.generation)),(k("body"),s(&input_body(fields)?))]))?);
            },
            "turn/steer"=>{
                turn=string_field(fields,"expectedTurnId")?;
                self.verify_visible_runtime_target(association,&selected.thread,Some(&turn),false)?;
                let message=format!("visible{}",&crate::store::digest::sha256_hex(format!("{workspace}\n{id}").as_bytes())[..40]);
                translations.push(native_request("K-INBOX","enqueue",&format!("{id}-enqueue"),&message,&association.domain,0,
                    BTreeMap::from([(k("seatId"),s(&association.seat)),(k("turnId"),s(&turn)),
                    (k("generation"),s(&association.generation)),(k("body"),s(&input_body(fields)?))]))?);
                translations.push(native_request("K-INBOX","steer",&format!("{id}-steer"),&message,&association.domain,1,
                    BTreeMap::from([(k("turnId"),s(&turn)),(k("generation"),s(&association.generation))]))?);
            },
            "turn/interrupt"=>{
                turn=string_field(fields,"turnId")?;
                self.verify_visible_runtime_target(association,&selected.thread,Some(&turn),false)?;
            },
            "physical-stop"=>{
                translations.push(native_request("K-SESSION","stop",id,&association.session,&association.domain,revision,
                    BTreeMap::from([(k("seatId"),s(&association.seat)),(k("generation"),s(&association.generation))]))?);
            },
            "original-question-answer"=>{
                let (card,card_revision,card_turn,answers)=self.visible_question_translation(association,&selected.thread,fields)?;
                turn=card_turn;
                translations.push(native_request("K-QCARD","answer",id,&card,&association.domain,card_revision,
                    BTreeMap::from([(k("generation"),s(&association.generation)),(k("answers"),answers)]))?);
            },
            _=>return Err(OrchestrationError::Invalid("visible effect method")),
        }
        Ok(Action {workspace:workspace.into(),id:id.into(),selection:selected.row,association:association.clone(),original:frame.to_vec(),
            method:method.into(),params:copy_json(params),translations,thread:selected.thread.clone(),turn,operation,ticket,nonce,
            phase:"UNKNOWN".into(),reply:Json::Null,error:String::new()})
    }
    fn insert_visible_action(&self,action:&Action)->Result<()> {
        let insert=Statement::prepare(self.connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_visible_effect(workspace_id,request_id,domain_id,selection_row,association_json,request_hex,method,params_json,translation_json,thread_id,turn_id,process_operation_id,ticket,custodian_nonce,phase,reply_json,original_error) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,'UNKNOWN','null','')")?;
        for (index,value) in [action.workspace.as_str(),action.id.as_str(),action.association.domain.as_str()].iter().enumerate() {insert.bind_text((index+1) as i32,value)?;}
        insert.bind_i64(4,action.selection)?;
        let association=action.association.json().canonical();let original=encode_hex(&action.original);
        let params=action.params.canonical();let translations=Json::Array(action.translations.iter().map(|bytes|s(&encode_hex(bytes))).collect()).canonical();
        for (index,value) in [association.as_str(),original.as_str(),action.method.as_str(),params.as_str(),translations.as_str(),
            action.thread.as_str(),action.turn.as_str(),action.operation.as_str(),action.ticket.as_str(),action.nonce.as_str()].iter().enumerate() {insert.bind_text((index+5) as i32,value)?;}
        insert.step_done()?;Ok(())
    }
    fn action_reply(&self,action:&Action,state:&str,reason:Option<&str>,response:Option<Json>)->Json {
        let mut reply=self.visible_reply(&action.workspace,state,Some(&action.association),reason);
        reply.insert(k("requestId"),s(&action.id));reply.insert(k("method"),s(&action.method));
        reply.insert(k("originalRequestRef"),Json::Object(BTreeMap::from([
            (k("frameSha256"),s(&crate::store::digest::sha256_hex(&action.original))),
            (k("selectionRowId"),s(&action.selection.to_string()))])));
        if let Some(response)=response {reply.insert(k("response"),response);}
        Json::Object(reply)
    }
    fn execute_visible_action(&mut self,action:&Action,input:&VerifiedDirectUserInput<'_>)->Result<Json> {
        let decoded=action.translations.iter().map(|bytes|h::decode_request(bytes).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("original visible translation: {error:?}"))))
            .collect::<Result<Vec<_>>>()?;
        match action.method.as_str() {
            "thread/start"=>Ok(self.action_reply(action,"UNSUPPORTED",Some(
                "The explicit selection identifies an already opened H session. No new-session identity, admission or USER open intention is present."),None)),
            "turn/start"=>{
                let request=decoded.first().ok_or(OrchestrationError::OperationConflict)?;
                let translated=VerifiedDirectUserInput {origin:input.origin,frame:input.frame,
                    observed_at_ms:input.observed_at_ms,visible_translation:Some((&action.workspace,&action.id))};
                let bytes=self.dispatch_verified_user_session(request,&translated)?;
                self.visible_native_outcome(action,request,&bytes)
            },
            "turn/steer"=>{
                if decoded.len()!=2 {return Err(OrchestrationError::OperationConflict);}
                let enqueued=self.dispatch_native_inbox(&decoded[0])?;
                let receipt=h::decode_receipt(&enqueued).map_err(|error|OrchestrationError::V37StoreFailure(format!("original visible enqueue receipt: {error:?}")))?;
                if !matches!(receipt.status,V37Status::Applied|V37Status::Replayed) {
                    return self.visible_native_outcome(action,&decoded[0],&enqueued);
                }
                let bytes=self.dispatch_native_inbox(&decoded[1])?;
                self.visible_native_outcome(action,&decoded[1],&bytes)
            },
            "turn/interrupt"=>{
                let step=visible_control_step(&action.workspace,&action.id);
                let permit=VisibleInterruptPermission {input,workspace:&action.workspace,id:&action.id};
                self.native_visible_interrupt_rpc(&action.association,&action.thread,&action.turn,&step,&permit)?;
                self.visible_control_result(action,&step)
            },
            "original-question-answer"=>{
                let request=decoded.first().ok_or(OrchestrationError::OperationConflict)?;
                let bytes=self.dispatch_user_qcard(request)?;self.visible_native_outcome(action,request,&bytes)
            },
            "physical-stop"|"thread/resume"=>{
                let request=decoded.first().ok_or(OrchestrationError::OperationConflict)?;
                let bytes=self.dispatch_user_session(request)?;self.visible_native_outcome(action,request,&bytes)
            },
            _=>Err(OrchestrationError::Invalid("finite visible effect")),
        }
    }
    fn visible_native_outcome(&mut self,action:&Action,request:&V37Request,bytes:&[u8])->Result<Json> {
        let receipt=h::decode_receipt(bytes).map_err(|error|OrchestrationError::V37StoreFailure(format!("original native action receipt: {error:?}")))?;
        if receipt.family!=request.family||receipt.operation!=request.operation||receipt.request_id!=request.request_id
            ||receipt.target_id!=request.target_id {return Err(OrchestrationError::OperationConflict);}
        if !matches!(receipt.status,V37Status::Applied|V37Status::Replayed) {
            let status=match receipt.status {V37Status::Denied|V37Status::Stale|V37Status::Conflict=>"DENIED",
                V37Status::Unsupported=>"UNSUPPORTED",_=>"UNKNOWN"};
            return Ok(self.action_reply(action,status,Some(&format!("Original native receipt: {}",
                std::str::from_utf8(bytes).map_err(|error|OrchestrationError::V37StoreFailure(format!("native receipt UTF-8: {error}")))?)),None));
        }
        match action.method.as_str() {
            "turn/start"=>self.visible_send_result(action),
            "turn/steer"=>{
                let step=format!("inbox{}",crate::store::digest::sha256_hex(format!("{}\n{}\n{}",action.association.domain,action.association.session,request.request_id).as_bytes()));
                self.visible_control_result(action,&step)
            },
            "thread/resume"=>self.visible_resume_result(action,request),
            "physical-stop"=>self.visible_stop_result(action,request),
            "original-question-answer"=>self.visible_answer_result(action,request),
            _=>Err(OrchestrationError::OperationConflict),
        }
    }
    fn store_visible_reply(&mut self,action:&Action,reply:Json)->Result<Vec<u8>> {
        let fields=object(&reply)?;let state=string_field(fields,"state")?;
        if !matches!(state.as_str(),"UNKNOWN"|"APPLIED"|"DENIED"|"UNSUPPORTED") {return Err(OrchestrationError::OperationConflict);}
        let reason=if fields.contains_key(&k("reason")) {string_field(fields,"reason")?} else {String::new()};
        let bytes=reply.canonical().into_bytes();
        if bytes.len()>crate::ipc::MAX_FRAME_BYTES {return Err(OrchestrationError::Invalid("visible effect response bound"));}
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let outcome=(||->Result<()> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            self.verify_visible_action_history(action)?;
            let update=Statement::prepare(self.connection.as_ptr(),
                "UPDATE main.gogoke_v37_visible_effect SET phase=?1,reply_json=?2,original_error=?3 WHERE workspace_id=?4 AND request_id=?5 AND request_hex=?6 AND association_json=?7 AND phase='UNKNOWN'")?;
            let serialized=std::str::from_utf8(&bytes).map_err(|error|OrchestrationError::V37StoreFailure(format!("visible reply UTF-8: {error}")))?;
            let original=encode_hex(&action.original);let association=action.association.json().canonical();
            for (index,value) in [state.as_str(),serialized,reason.as_str(),action.workspace.as_str(),action.id.as_str(),original.as_str(),association.as_str()].iter().enumerate() {update.bind_text((index+1) as i32,value)?;}
            update.step_done()?;
            Ok(())
        })();
        match outcome {Ok(())=>self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?,
            Err(error)=>{self.connection.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?;return Err(error);}}
        // The original result is durable before optional current-choice CAS.
        // A later legitimate H/E transition cannot roll back historical ACKs.
        if action.method=="thread/resume"&&state=="APPLIED" {
            self.reconcile_visible_resume_association(action,&reply)?;
        }
        Ok(bytes)
    }
    /// The original APPLIED receipt commits first. A crash in that gap leaves
    /// this optional selection projection pending; a repeat of the same USER
    /// operation must retry only this CAS, never repeat the H resume.
    fn reconcile_visible_resume_association(&mut self,action:&Action,reply:&Json)->Result<()> {
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let outcome=(||->Result<()> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            self.verify_visible_action_history(action)?;
            self.append_visible_resume_association(action,reply)
        })();
        match outcome {Ok(())=>self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause),
            Err(error)=>{self.connection.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?;Err(error)}}
    }
    fn append_visible_resume_association(&self,action:&Action,reply:&Json)->Result<()> {
        let Some(latest)=self.visible_selection(&action.workspace,None)? else {return Ok(());};
        if latest.row!=action.selection||latest.association.as_ref()!=Some(&action.association) {return Ok(());}
        let reply=object(reply)?;
        let response=object(reply.get(&k("response")).ok_or(OrchestrationError::OperationConflict)?)?;
        let result=object(response.get(&k("result")).ok_or(OrchestrationError::OperationConflict)?)?;
        let next=Association::parse(result.get(&k("nativeAssociation")).ok_or(OrchestrationError::OperationConflict)?)?;
        let mut expected=action.association.clone();expected.generation=next.generation.clone();
        if expected!=next {return Err(OrchestrationError::AccessDenied);}
        // This is the same explicit USER resume, appended as a new selection
        // generation only while its original selection is still current.
        let current=h::session_binding::current_relationship(&self.connection,&next.domain,&next.session)
            .map_err(|cause|OrchestrationError::V37StoreFailure(format!("resume choice current E/H qualification: {cause:?}")))?;
        let Some(current)=current else {return Ok(());};
        if !current.native_v2||current.seat_id!=next.seat||current.seat_incarnation!=next.incarnation
            ||current.seat_authorization_generation.to_string()!=next.authorization
            ||current.session_generation!=next.generation||current.instance_id!=next.instance {return Ok(());}
        let eligible=Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_instances i JOIN main.gogoke_v37_worktrees w ON w.worktree_id=?2 AND w.repository_id=?3 JOIN main.gogoke_v37_worktree_sources f ON f.repository_id=w.repository_id WHERE i.instance_id=?1 AND i.driver_id='codex' AND i.version='0.160.0' AND w.state='REGISTERED' AND w.revision=1 AND w.domain_id=?4 AND w.seat_id=?5 AND w.seat_incarnation=?6")?;
        for (index,value) in [next.instance.as_str(),latest.worktree.as_str(),latest.repository.as_str(),next.domain.as_str(),
            next.seat.as_str(),next.incarnation.as_str()].iter().enumerate() {eligible.bind_text((index+1) as i32,value)?;}
        if !eligible.step_row()? {return Ok(());}
        if eligible.step_row()? {return Err(OrchestrationError::OperationConflict);}drop(eligible);
        // Current eligibility is now proven. Original source/binding/format
        // failures are not stale-cache outcomes and must retain their causes.
        let selected=self.visible_candidate(&next,false)?;
        let insert=Statement::prepare(self.connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_visible_conversation_selection(workspace_id,request_id,route,selection_hex,association_json,repository_id,worktree_id,thread_id,open_request_id,open_generation,open_operation_id,ack_source_id,started_source_id) VALUES(?1,?2,'NATIVE',?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)")?;
        let id=format!("{}-resume-association",action.id);let original=encode_hex(&action.original);let association=next.json().canonical();
        for (index,value) in [action.workspace.as_str(),id.as_str(),original.as_str(),association.as_str(),selected.repository.as_str(),
            selected.worktree.as_str(),selected.thread.as_str(),selected.open_request.as_str(),selected.open_generation.as_str(),selected.open_operation.as_str()].iter().enumerate() {insert.bind_text((index+1) as i32,value)?;}
        insert.bind_i64(11,selected.ack)?;insert.bind_i64(12,selected.started)?;insert.step_done()?;Ok(())
    }
    fn original_rpc_reply(&self,action:&Action,observed:rpc_journal::VisibleRpcResponse)->Result<Json> {
        let mut response=source_json(&encode_hex(&observed.bytes))?;
        let Json::Object(ref mut envelope)=response else {return Err(OrchestrationError::OperationConflict);};
        let Some(Json::Object(result))=envelope.get_mut(&k("result")) else {return Err(OrchestrationError::OperationConflict);};
        result.insert(k("nativeSource"),source_ref(&observed.source.operation_id,&observed.source.source_epoch,&observed.source.source_cursor));
        Ok(self.action_reply(action,"APPLIED",None,Some(response)))
    }
    fn visible_send_result(&mut self,action:&Action)->Result<Json> {
        let request=action.translations.first().ok_or(OrchestrationError::OperationConflict)?;
        let input=h::StdinRequest {domain_id:&action.association.domain,session_id:&action.association.session,
            ticket:&action.ticket,generation:&action.association.generation,request_bytes:request};
        let existing=h::read_stdin_journal(&self.connection,&h::StdinJournalKey {domain_id:&action.association.domain,
            request_id:&action.id,session_id:&action.association.session,ticket:&action.ticket,generation:&action.association.generation})
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("original visible H input: {error:?}")))?;
        let Some(mut existing)=existing else {return Ok(self.action_reply(action,"UNKNOWN",Some("The original H input journal has no recorded send; no replay is permitted."),None));};
        if existing.request_bytes!=*request||existing.process_operation_id!=action.operation||existing.custodian_nonce!=action.nonce {
            return Err(OrchestrationError::AccessDenied);
        }
        if existing.state!=h::JournalState::Receipted {
            if let Some(recovered)=h::recover_codex_turn_request(&mut self.connection,&input).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("original visible H send recovery: {error:?}")))? {existing=recovered.record;}
        }
        let Some(receipt_bytes)=existing.receipt_bytes else {
            return Ok(self.action_reply(action,"UNKNOWN",Some("The original H send has no completed receipt; UNKNOWN cannot allocate another write or ID."),None));
        };
        let receipt=h::decode_receipt(&receipt_bytes).map_err(|error|OrchestrationError::V37StoreFailure(format!("original visible send receipt: {error:?}")))?;
        if !matches!(receipt.status,V37Status::Applied|V37Status::Replayed) {
            return Ok(self.action_reply(action,"UNKNOWN",Some(&format!("Original H send receipt: {}",
                std::str::from_utf8(&receipt_bytes).map_err(|error|OrchestrationError::V37StoreFailure(format!("send receipt UTF-8: {error}")))?)),None));
        }
        let step=format!("send-{}",&crate::store::digest::sha256_hex(request)[..40]);
        let body=input_body(object(&action.params)?)?;
        let observed=rpc_journal::read_visible_original_response(&self.connection,&action.association.domain,&action.association.session,
            &action.association.generation,&action.operation,&action.ticket,&action.nonce,&step,
            rpc_journal::VisibleRpcExpectation::Start {thread:&action.thread,body:&body})
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("original visible turn/start source: {error:?}")))?;
        match observed {Some(observed)=>self.original_rpc_reply(action,observed),None=>Ok(self.action_reply(action,"UNKNOWN",Some(
            "The original H receipt has no exact typed turn/start ACK source."),None))}
    }
    fn visible_control_result(&mut self,action:&Action,step:&str)->Result<Json> {
        let body=if action.method=="turn/steer" {Some(input_body(object(&action.params)?)?)} else {None};
        let expected=if let Some(body)=&body {rpc_journal::VisibleRpcExpectation::Steer {thread:&action.thread,turn:&action.turn,body}}
            else {rpc_journal::VisibleRpcExpectation::Interrupt {thread:&action.thread,turn:&action.turn}};
        let observed=rpc_journal::reconcile_visible_original_response(&mut self.connection,&self.owner,&action.association.domain,&action.association.session,
            &action.association.generation,&action.operation,&action.ticket,&action.nonce,step,expected)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("original visible control ACK: {error:?}")))?;
        match observed {Some(observed)=>self.original_rpc_reply(action,observed),None=>Ok(self.action_reply(action,"UNKNOWN",Some(
            "No exact original H WRITTEN/OBSERVED control and typed ACK are available; no command was resent."),None))}
    }
    fn visible_resume_result(&self,action:&Action,request:&V37Request)->Result<Json> {
        let row=Statement::prepare(self.connection.as_ptr(),
            "SELECT e.generation,e.process_operation_id,c.ticket,c.custodian_nonce FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_h_generation g ON g.domain_id=e.domain_id AND g.session_id=e.session_id AND g.generation=e.generation AND g.request_id=e.request_id AND g.process_operation_id=e.process_operation_id JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id AND c.generation=e.generation WHERE e.domain_id=?1 AND e.session_id=?2 AND e.request_id=?3 AND e.raw_hex=?4 AND e.old_generation=?5 AND e.seat_id=?6 AND e.seat_incarnation=?7 AND e.instance_id=?8 AND e.phase IN ('ACTIVE','STOPPED')")?;
        let raw=encode_hex(&request.raw_bytes);
        for (index,value) in [action.association.domain.as_str(),action.association.session.as_str(),request.request_id.as_str(),raw.as_str(),
            action.association.generation.as_str(),action.association.seat.as_str(),action.association.incarnation.as_str(),action.association.instance.as_str()].iter().enumerate() {row.bind_text((index+1) as i32,value)?;}
        if !row.step_row()? {return Ok(self.action_reply(action,"UNKNOWN",Some("The original resume has no promoted H generation/episode; recovery cannot launch a process."),None));}
        let generation=row.column_text(0)?;let operation=row.column_text(1)?;let ticket=row.column_text(2)?;let nonce=row.column_text(3)?;
        if row.step_row()? {return Err(OrchestrationError::OperationConflict);}drop(row);
        if decimal(&action.association.generation)?.checked_add(1)!=Some(decimal(&generation)?) {
            return Err(OrchestrationError::OperationConflict);
        }
        let observed=rpc_journal::read_visible_original_response(&self.connection,&action.association.domain,&action.association.session,
            &generation,&operation,&ticket,&nonce,&format!("{operation}-thread-resume"),
            rpc_journal::VisibleRpcExpectation::Resume {thread:&action.thread})
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("original visible resume ACK: {error:?}")))?;
        let Some(observed)=observed else {return Ok(self.action_reply(action,"UNKNOWN",Some("The original promoted H resume has no exact typed vendor ACK."),None));};
        let started=Statement::prepare(self.connection.as_ptr(),
            "SELECT source_epoch,source_cursor,hex(raw_bytes),rowid FROM main.v37_ledger_raw_source WHERE domain_id=?1 AND session_id=?2 AND generation=?3 AND operation_id=?4 AND process_ticket=?5 AND custodian_nonce=?6 ORDER BY rowid")?;
        for (index,value) in [action.association.domain.as_str(),action.association.session.as_str(),generation.as_str(),operation.as_str(),ticket.as_str(),nonce.as_str()].iter().enumerate() {started.bind_text((index+1) as i32,value)?;}
        let mut started_source=None;
        while started.step_row()? {
            let original=source_json(&started.column_text(2)?)?;let original=object(&original)?;
            if !is_text(original.get(&k("method")),"thread/started") {continue;}
            let params=object(original.get(&k("params")).ok_or(OrchestrationError::OperationConflict)?)?;
            let thread=object(params.get(&k("thread")).ok_or(OrchestrationError::OperationConflict)?)?;
            if !is_text(thread.get(&k("id")),&action.thread)||started_source.is_some() {return Err(OrchestrationError::OperationConflict);}
            let mut source=copy_fields(object(&source_ref(&operation,&started.column_text(0)?,&started.column_text(1)?))?);
            source.insert(k("kind"),s("THREAD_STARTED"));source.insert(k("generation"),s(&generation));
            source.insert(k("rawSourceId"),s(&started.column_text(3)?));started_source=Some(Json::Object(source));
        }
        let Some(started_source)=started_source else {return Ok(self.action_reply(action,"UNKNOWN",Some(
            "The promoted H resume has no original thread/started source from its new process generation; cache identity cannot advance."),None));};
        let mut response=source_json(&encode_hex(&observed.bytes))?;
        let Json::Object(ref mut envelope)=response else {return Err(OrchestrationError::OperationConflict);};
        let Some(Json::Object(result))=envelope.get_mut(&k("result")) else {return Err(OrchestrationError::OperationConflict);};
        let mut next=action.association.clone();next.generation=generation;
        result.insert(k("nativeAssociation"),next.json());
        let mut ack=copy_fields(object(&source_ref(&observed.source.operation_id,&observed.source.source_epoch,&observed.source.source_cursor))?);
        ack.insert(k("kind"),s("RESUME_ACK"));ack.insert(k("generation"),s(&next.generation));
        result.insert(k("sourceRefs"),Json::Array(vec![Json::Object(ack),started_source]));
        Ok(self.action_reply(action,"APPLIED",None,Some(response)))
    }
    fn visible_stop_result(&self,action:&Action,request:&V37Request)->Result<Json> {
        let raw=encode_hex(&request.raw_bytes);
        let row=Statement::prepare(self.connection.as_ptr(),
            "SELECT c.stop_proof_hash FROM main.gogoke_v37_h_operation o JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=o.domain_id AND e.session_id=o.session_id AND e.generation=?4 AND e.process_operation_id=?5 JOIN main.gogoke_v37_h_generation g ON g.domain_id=e.domain_id AND g.session_id=e.session_id AND g.generation=e.generation AND g.process_operation_id=e.process_operation_id JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id AND c.generation=e.generation AND c.ticket=?6 AND c.custodian_nonce=?7 WHERE o.domain_id=?1 AND o.session_id=?2 AND o.request_id=?3 AND o.operation='stop' AND o.raw_hex=?8 AND o.status='APPLIED' AND e.phase='STOPPED' AND c.state='STOPPED' AND e.stop_fact_id=c.stop_proof_hash AND c.stop_proof_hash IS NOT NULL")?;
        for (index,value) in [action.association.domain.as_str(),action.association.session.as_str(),request.request_id.as_str(),
            action.association.generation.as_str(),action.operation.as_str(),action.ticket.as_str(),action.nonce.as_str(),raw.as_str()].iter().enumerate() {row.bind_text((index+1) as i32,value)?;}
        if !row.step_row()? {return Ok(self.action_reply(action,"UNKNOWN",Some("The original H stop has no completed operation and exact durable physical stop proof; process absence is not a StopFact."),None));}
        let fact=row.column_text(0)?;
        if fact.is_empty()||row.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let mut reply=copy_fields(object(&self.action_reply(action,"APPLIED",None,Some(Json::Object(BTreeMap::from([
            (k("result"),Json::Object(BTreeMap::from([(k("stopFact"),s(&fact))])))])))))?);
        reply.insert(k("stopFact"),s(&fact));Ok(Json::Object(reply))
    }
    fn visible_answer_result(&self,action:&Action,request:&V37Request)->Result<Json> {
        match self.visible_original_answer_receipt(&action.association,request,&action.thread)? {
            Some(response)=>Ok(self.action_reply(action,"APPLIED",None,Some(response))),
            None=>Ok(self.action_reply(action,"UNKNOWN",Some("Original C answer is not ANSWERED with its exact H WRITTEN proof; UNKNOWN cannot resend a response."),None)),
        }
    }
    fn recover_visible_action(&mut self,action:&Action)->Result<Vec<u8>> {
        // Durable refusal/unsupported results have no H effect to recover.
        if matches!(action.phase.as_str(),"DENIED"|"UNSUPPORTED") {return Ok(action.reply.canonical().into_bytes());}
        let observed=(||->Result<Json> {
            match action.method.as_str() {
                "turn/start"=>self.visible_send_result(action),
                "turn/interrupt"=>self.visible_control_result(action,&visible_control_step(&action.workspace,&action.id)),
                "turn/steer"=>{
                    let bytes=action.translations.get(1).ok_or(OrchestrationError::OperationConflict)?;
                    let request=h::decode_request(bytes).map_err(|error|OrchestrationError::V37StoreFailure(format!("original steer request: {error:?}")))?;
                    let step=format!("inbox{}",crate::store::digest::sha256_hex(format!("{}\n{}\n{}",action.association.domain,action.association.session,request.request_id).as_bytes()));
                    // C remains the delivery authority. Its original request
                    // branch only settles from A/H and never acquires a writer.
                    let prior=Statement::prepare(self.connection.as_ptr(),"SELECT request_hex,phase FROM main.gogoke_v37_inbox_operations WHERE domain_id=?1 AND request_id=?2")?;
                    prior.bind_text(1,&action.association.domain)?;prior.bind_text(2,&request.request_id)?;
                    if !prior.step_row()? {return Ok(self.action_reply(action,"UNKNOWN",Some("Original C steer intent is absent; recovery cannot enqueue or deliver."),None));}
                    if prior.column_text(0)?!=encode_hex(bytes) {return Err(OrchestrationError::AccessDenied);}
                    let phase=prior.column_text(1)?;drop(prior);
                    let ack=self.visible_control_result(action,&step)?;
                    if phase!="APPLIED" {
                        if !is_text(object(&ack)?.get(&k("state")),"APPLIED") {return Ok(ack);}
                        let envelope=crate::store::inbox::InboxEnvelope {domain_id:&request.domain_id,request_id:&request.request_id,
                            request_bytes:&request.raw_bytes,message_id:&request.target_id,expected_revision:request.expected_revision};
                        let settled=crate::store::inbox::settle_native_delivery_observed(&mut self.connection,&self.owner,&envelope,
                            &action.association.session,&step,crate::store::inbox::NativeDeliveryKind::Steer)
                            .map_err(|error|OrchestrationError::V37StoreFailure(format!("original C steer receipt recovery: {error:?}")))?;
                        if settled.phase!="APPLIED" {return Ok(self.action_reply(action,"UNKNOWN",Some(&format!(
                            "Original C steer result: {} / {}",settled.phase,settled.reason)),None));}
                    }
                    Ok(ack)
                },
                "thread/resume"|"physical-stop"|"original-question-answer"=>{
                    let request=h::decode_request(action.translations.first().ok_or(OrchestrationError::OperationConflict)?)
                        .map_err(|error|OrchestrationError::V37StoreFailure(format!("original visible request recovery: {error:?}")))?;
                    match action.method.as_str() {"thread/resume"=>self.visible_resume_result(action,&request),
                        "physical-stop"=>self.visible_stop_result(action,&request),_=>self.visible_answer_result(action,&request)}
                },
                "thread/start"=>Ok(self.action_reply(action,"UNSUPPORTED",Some("The original selection contains no new-session admission identity."),None)),
                _=>Err(OrchestrationError::Invalid("original effect method")),
            }
        })();
        let reply=match observed {Ok(reply)=>reply,Err(error)=>self.action_reply(action,"UNKNOWN",Some(&format!(
            "Original visible {} recovery: {error:?}; recorded failure: {}",action.method,action.error)),None)};
        if action.phase=="APPLIED" {
            // Revalidate the original receipt every time; never return a saved
            // success when its actual H/C/A source can no longer be proven.
            // Reconcile a resume selection after the durable APPLIED/reselection
            // crash gap. The exact old selection must still be latest; current
            // E/H movement or another USER selection simply leaves history as is.
            if action.method=="thread/resume" && is_text(object(&reply)?.get(&k("state")),"APPLIED") {
                self.reconcile_visible_resume_association(action,&reply)?;
            }
            return Ok(reply.canonical().into_bytes());
        }
        self.store_visible_reply(action,reply)
    }
}

fn visible_control_step(workspace:&str,id:&str)->String {
    format!("visible{}",crate::store::digest::sha256_hex(format!("{workspace}\n{id}").as_bytes()))
}

#[cfg(all(test,windows))]
mod tests {
    use super::*;
    use crate::ipc::PrivatePipeListener;
    use crate::store::same_open::route_b_test_guard;
    use std::fs::OpenOptions;
    use std::io::Write;

    fn fixture(run:impl FnOnce(&mut ProductDatabase<'_>,&UserOriginProof)) {
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-visible-original-user-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&path).unwrap();let root=RootLock::acquire(&path).unwrap();let database=path.join("state.sqlite");
        let mut product=ProductDatabase::open(&root,&database).unwrap();
        let listener=PrivatePipeListener::bind_user(&format!("visible-{}-{nonce}",std::process::id()),std::process::id()).unwrap();
        let endpoint=listener.path().to_owned();let (release,hold)=std::sync::mpsc::channel::<()>();
        let client=std::thread::spawn(move|| {
            let mut pipe=OpenOptions::new().read(true).write(true).open(endpoint).unwrap();pipe.write_all(&[0x47]).unwrap();hold.recv().unwrap();
        });
        let mut pipe=listener.accept_user().unwrap();let proof=pipe.take_user_origin_proof().unwrap();
        run(&mut product,&proof);drop(proof);drop(pipe);release.send(()).unwrap();client.join().unwrap();
        product.close_checked().unwrap();drop(root);std::fs::remove_file(database).unwrap();
        std::fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).unwrap();
        if let Err(error)=std::fs::remove_dir(&path) {eprintln!("owned original USER fixture retained: {error}");}
    }
    fn association()->Association {Association {domain:"projectA".into(),session:"sessionA".into(),seat:"leadA".into(),
        incarnation:"incarnationA".into(),authorization:"1".into(),generation:"1".into(),instance:"instanceA".into()}}
    fn original(id:&str,method:&str,association:&Association,params:&Json)->Vec<u8> {
        Json::Object(BTreeMap::from([(k("schema"),s("gogoke.37.owner-configuration.v1")),(k("command"),s("visible-conversation-operate")),
            (k("workspaceId"),s("workspaceA")),(k("requestId"),s(id)),(k("expectedAssociation"),association.json()),
            (k("method"),s(method)),(k("params"),copy_json(params))])).canonical().into_bytes()
    }
    fn action(id:&str,method:&str,params:Json,translations:Vec<Vec<u8>>)->Action {
        let association=association();let raw=original(id,method,&association,&params);
        Action {workspace:"workspaceA".into(),id:id.into(),selection:1,association,original:raw,method:method.into(),params,translations,
            thread:"vendorThreadA".into(),turn:"vendorTurnA".into(),operation:"processA".into(),ticket:"pct1_ticketA".into(),nonce:"nonceA".into(),
            phase:"UNKNOWN".into(),reply:Json::Null,error:String::new()}
    }
    /// Synthetic immutable H/selection facts only. There is deliberately no
    /// live native session, current E authority, process launch or model call.
    fn history(product:&mut ProductDatabase<'_>,association:&Association) {
        let selected=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_visible_conversation_selection VALUES('workspaceA','selectionA','NATIVE','00',?1,'repositoryA','worktreeA','vendorThreadA','openA','1','processA',1,2)").unwrap();
        selected.bind_text(1,&association.json().canonical()).unwrap();selected.step_done().unwrap();drop(selected);
        product.connection.execute("INSERT INTO main.gogoke_v37_session_binding_v2 VALUES('projectA','sessionA','leadA','incarnationA',1,'instanceA','NATIVE_V2')").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase,stop_fact_id) VALUES('projectA','openA','sessionA','1',NULL,'00',0,1,'processA','instanceA','homeA','bindingA','leadA','incarnationA','STOPPED','syntheticFixtureStop')").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_v37_h_generation VALUES('projectA','sessionA','1','openA','processA')").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,stop_proof_hash) VALUES('processA','pct1_ticketA','nonceA','11','1','fixture','sha256:fixture','profileA','projectA','1','STOPPED','syntheticFixtureStop')").unwrap();
    }
    #[test]
    fn visible_original_user_translation_rechecks_actual_proof_frame_and_native_body() {
        fixture(|product,proof| {
            let params=Json::Object(BTreeMap::from([(k("threadId"),s("vendorThreadA")),(k("input"),Json::Array(vec![
                Json::Object(BTreeMap::from([(k("type"),s("text")),(k("text"),s("original USER body"))]))]))]));
            let lowered=native_request("K-SESSION","send","sendA","sessionA","projectA",5,
                BTreeMap::from([(k("generation"),s("1")),(k("body"),s("original USER body"))])).unwrap();
            let saved=action("sendA","turn/start",params,vec![lowered.clone()]);product.insert_visible_action(&saved).unwrap();
            let original=VerifiedDirectUserInput {origin:proof,frame:&saved.original,observed_at_ms:Some(100),visible_translation:None};
            assert!(!original.matches_original_in_transaction(&product.connection,&lowered).unwrap(),"a plain direct frame cannot impersonate its translated H request");
            let translated=VerifiedDirectUserInput {origin:proof,frame:&saved.original,observed_at_ms:Some(100),visible_translation:Some(("workspaceA","sendA"))};
            assert!(translated.matches_original_in_transaction(&product.connection,&lowered).unwrap());
            let changed=native_request("K-SESSION","send","sendA","sessionA","projectA",5,
                BTreeMap::from([(k("generation"),s("1")),(k("body"),s("changed USER body"))])).unwrap();
            assert!(!translated.matches_original_in_transaction(&product.connection,&changed).unwrap());
            let changed_frame=saved.original.iter().copied().chain([b' ']).collect::<Vec<_>>();
            let other=VerifiedDirectUserInput {frame:&changed_frame,..translated};
            assert!(!other.matches_original_in_transaction(&product.connection,&lowered).unwrap(),"even equivalent owner JSON is not the original bytes");
            let response=product.configure_user_v37(&saved.original).unwrap();
            assert!(is_text(object(&Parser::parse(std::str::from_utf8(&response).unwrap()).unwrap()).unwrap().get(&k("state")),"DENIED"),"absence of actual USER origin is denied");
        });
    }
    #[test]
    fn visible_unknown_cold_recovery_uses_original_history_and_never_creates_a_writer() {
        fixture(|product,proof| {
            let params=Json::Object(BTreeMap::from([(k("threadId"),s("vendorThreadA")),(k("turnId"),s("vendorTurnA"))]));
            let saved=action("interruptA","turn/interrupt",params,Vec::new());history(product,&saved.association);product.insert_visible_action(&saved).unwrap();
            // A later explicit Legacy selection does not rewrite the old
            // request's history identity or authorize a new native command.
            product.connection.execute("INSERT INTO main.gogoke_v37_visible_conversation_selection VALUES('workspaceA','laterLegacy','LEGACY','00','null','','','','','','',0,0)").unwrap();
            // The original intent remains recoverable, but a new intent may
            // not use the old choice after this workspace selected another row.
            let new_frame=original("newAfterChoice","turn/interrupt",&saved.association,&saved.params);
            let denied=product.dispatch_user_frame(proof,&new_frame).unwrap();
            let denied=Parser::parse(std::str::from_utf8(&denied).unwrap()).unwrap();
            let denied=object(&denied).unwrap();
            assert!(is_text(denied.get(&k("state")),"DENIED"));
            assert!(string_field(denied,"reason").unwrap().contains("Original workspace selection changed before new visible action."));
            assert!(read_action(product.connection.as_ptr(),"workspaceA","newAfterChoice").unwrap().is_none(),
                "a rejected new intent must not create an action or writer");
            let recover=Json::Object(BTreeMap::from([(k("schema"),s("gogoke.37.owner-configuration.v1")),(k("command"),s("visible-conversation-recover")),
                (k("workspaceId"),s("workspaceA")),(k("requestId"),s("interruptA")),(k("expectedAssociation"),saved.association.json())])).canonical();
            let response=product.dispatch_user_frame(proof,recover.as_bytes()).unwrap();
            let response=Parser::parse(std::str::from_utf8(&response).unwrap()).unwrap();
            assert!(is_text(object(&response).unwrap().get(&k("state")),"UNKNOWN"));
            assert!(is_text(object(&response).unwrap().get(&k("requestId")),"interruptA"));
            for field in ["domainId","sessionId","seatId","incarnation","authorizationGeneration","bindingGeneration","instanceId"] {
                let mut changed=copy_fields(object(&saved.association.json()).unwrap());changed.insert(k(field),s(if field.ends_with("Generation") {"9"} else {"foreign"}));
                let mut fields=copy_fields(object(&Parser::parse(&recover).unwrap()).unwrap());fields.insert(k("expectedAssociation"),Json::Object(changed));
                let response=product.dispatch_user_frame(proof,Json::Object(fields).canonical().as_bytes()).unwrap();
                assert!(is_text(object(&Parser::parse(std::str::from_utf8(&response).unwrap()).unwrap()).unwrap().get(&k("state")),"DENIED"),"original association field {field} must match");
            }
            for table in ["gogoke_v37_h_stdin_journal","gogoke_v37_rpc_steps"] {
                let count=Statement::prepare(product.connection.as_ptr(),&format!("SELECT COUNT(*) FROM main.{table}")).unwrap();
                assert!(count.step_row().unwrap());assert_eq!(count.column_text(0).unwrap(),"0","cold UNKNOWN must not create a {table} writer");
            }
        });
    }
    #[test]
    fn visible_resume_original_receipt_survives_ineligible_current_choice_and_reports_cas_format_error() {
        fixture(|product,_| {
            let params=Json::Object(BTreeMap::from([(k("threadId"),s("vendorThreadA"))]));
            let saved=action("resumeHistorical","thread/resume",params,Vec::new());
            history(product,&saved.association);product.insert_visible_action(&saved).unwrap();
            let mut next=saved.association.clone();next.generation="2".into();
            // This test starts after original H/A recovery has produced its
            // result. It tests receipt persistence, not synthetic model ACKs.
            let response=Json::Object(BTreeMap::from([(k("result"),Json::Object(BTreeMap::from([
                (k("nativeAssociation"),next.json())])))]));
            let reply=product.action_reply(&saved,"APPLIED",None,Some(response));
            let original_reply=reply.canonical();
            // No current E authority exists in this fixture: the old selection
            // remains current, but cannot qualify the new generation for CAS.
            assert_eq!(product.store_visible_reply(&saved,reply).unwrap(),original_reply.as_bytes());
            let retained=read_action(&product.connection,"workspaceA","resumeHistorical").unwrap().unwrap();
            assert_eq!(retained.phase,"APPLIED");assert_eq!(retained.reply.canonical(),original_reply);
            assert_eq!(product.visible_selection("workspaceA",None).unwrap().unwrap().row,1);
            // A malformed result is an actual format error, not stale current
            // authority. The committed original result must still survive it.
            let malformed=product.action_reply(&saved,"APPLIED",None,Some(Json::Null));
            assert!(product.store_visible_reply(&saved,malformed).is_err());
            let retained=read_action(&product.connection,"workspaceA","resumeHistorical").unwrap().unwrap();
            assert_eq!(retained.phase,"APPLIED");assert_eq!(retained.reply.canonical(),original_reply);
            assert_eq!(product.visible_selection("workspaceA",None).unwrap().unwrap().row,1);
        });
    }
    #[test]
    fn visible_cold_interrupt_receipt_requires_original_typed_ack_and_never_returns_stop_fact() {
        fixture(|product,proof| {
            let params=Json::Object(BTreeMap::from([(k("threadId"),s("vendorThreadA")),(k("turnId"),s("vendorTurnA"))]));
            let saved=action("interruptB","turn/interrupt",params,Vec::new());history(product,&saved.association);product.insert_visible_action(&saved).unwrap();
            let command=Command::TurnInterrupt {thread_id:"vendorThreadA".into(),turn_id:"vendorTurnA".into()}.encode(Some(&RpcId::Number(7))).unwrap();
            let step=visible_control_step("workspaceA","interruptB");
            let insert=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,command_hex,requires_response,phase) VALUES('projectA','sessionA','openA',?1,'processA','pct1_ticketA','nonceA','11','1','fixture','sha256:fixture','profileA','1',?2,1,'WRITTEN')").unwrap();
            insert.bind_text(1,&step).unwrap();insert.bind_text(2,&encode_hex(&command)).unwrap();insert.step_done().unwrap();drop(insert);
            let raw=|product:&ProductDatabase<'_>,bytes:&[u8]| {
                let insert=Statement::prepare(product.connection.as_ptr(),
                    "INSERT INTO main.v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,generation,source_epoch,source_cursor,raw_bytes,state) VALUES('processA','pct1_ticketA','nonceA','projectA','sessionA','1','nonceA','1',?1,'PENDING')").unwrap();
                insert.bind_blob(1,bytes).unwrap();insert.step_done().unwrap();
            };
            // These are synthetic protocol frames against the real readback
            // function, not an observed provider or process stop experiment.
            raw(product,b"{\"id\":\"7\",\"result\":{}}\n");
            let recover=Json::Object(BTreeMap::from([(k("schema"),s("gogoke.37.owner-configuration.v1")),(k("command"),s("visible-conversation-recover")),
                (k("workspaceId"),s("workspaceA")),(k("requestId"),s("interruptB")),(k("expectedAssociation"),saved.association.json())])).canonical();
            let response=product.dispatch_user_frame(proof,recover.as_bytes()).unwrap();
            assert!(is_text(object(&Parser::parse(std::str::from_utf8(&response).unwrap()).unwrap()).unwrap().get(&k("state")),"UNKNOWN"),"a string vendor ID cannot acknowledge a numeric original command");
            product.connection.execute("DELETE FROM main.v37_ledger_raw_source WHERE operation_id='processA'").unwrap();
            raw(product,b"{\"id\":7,\"result\":{}}\n");
            // Current H can already name a later generation; the original
            // response still belongs exclusively to old processA / generation1.
            product.connection.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('laterBinding','laterInstance','projectA','SESSION','sessionA','2','ACTIVE')").unwrap();
            product.connection.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('projectA','sessionA','laterInstance','laterHome','laterBinding','2','COMMITTED',9,'laterProcess')").unwrap();
            let response=product.dispatch_user_frame(proof,recover.as_bytes()).unwrap();let response=Parser::parse(std::str::from_utf8(&response).unwrap()).unwrap();
            let fields=object(&response).unwrap();assert!(is_text(fields.get(&k("state")),"APPLIED"));
            assert!(is_text(fields.get(&k("method")),"turn/interrupt"));assert!(!fields.contains_key(&k("stopFact")),"an interrupt ACK is not a physical StopFact");
            let rpc=object(fields.get(&k("response")).unwrap()).unwrap();
            assert!(matches!(rpc.get(&k("id")),Some(Json::Number(value)) if value=="7"));
            let count=Statement::prepare(product.connection.as_ptr(),"SELECT COUNT(*) FROM main.gogoke_v37_rpc_steps").unwrap();
            assert!(count.step_row().unwrap());assert_eq!(count.column_text(0).unwrap(),"1","cold recovery never creates a new control step");
        });
    }
}
