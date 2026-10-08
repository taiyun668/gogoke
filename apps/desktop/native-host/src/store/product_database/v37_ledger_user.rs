//! User reads select a logical registered session. H/E and the native Owner
//! issuer determine its reader identity; wire scope never grants GLOBAL.
use super::*;
use crate::store::ledger::{self,LedgerEvent,LedgerPosition,Reader,Subscription};

fn text(value:&str)->Json {Json::String(JsonString::from_str(value))}
fn number(value:&str)->Result<u64> {
    let parsed=value.parse::<u64>().map_err(|error|OrchestrationError::V37StoreFailure(format!("ledger cursor: {error}")))?;
    if parsed.to_string()!=value || parsed>i64::MAX as u64 {return Err(OrchestrationError::Invalid("ledger cursor"));}
    Ok(parsed)
}

#[cfg(all(test,windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::route_b_test_guard;
    use crate::store::seat::{self,NativeOrigin,CreateSeat,Kind,StoreTemplate};
    use std::time::{SystemTime,UNIX_EPOCH};

    fn request(verb:&str,id:&str,target:&str,revision:u64,payload:&str)->V37Request {
        decode_request(format!(r#"{{"schema":"gogoke.37.operations.v1","family":"K-LEDGER","operation":"{verb}","requestId":"{id}","targetId":"{target}","domainId":"projectA","expectedRevision":"{revision}","payload":{payload}}}"#).as_bytes()).unwrap()
    }
    fn global_request(verb:&str,id:&str,target:&str,revision:u64,payload:&str)->V37Request {
        decode_request(format!(r#"{{"schema":"gogoke.37.operations.v1","family":"K-LEDGER","operation":"{verb}","requestId":"{id}","targetId":"{target}","domainId":"global","expectedRevision":"{revision}","payload":{payload}}}"#).as_bytes()).unwrap()
    }
    fn reply(product:&mut ProductDatabase<'_>,request:&V37Request)->crate::store::session_transport::V37Receipt {
        crate::store::session_transport::decode_receipt(&product.dispatch_user_request(request).unwrap()).unwrap()
    }
    fn markers(rows:&[Json])->std::collections::BTreeSet<String> {
        rows.iter().map(|row| {
            let Json::Object(row)=row else {panic!("event envelope");};
            let Some(Json::Object(update))=row.get(&JsonString::from_str("update")) else {panic!("event update");};
            let Some(Json::Object(content))=update.get(&JsonString::from_str("content")) else {panic!("marker content");};
            let Some(Json::String(value))=content.get(&JsonString::from_str("text")) else {panic!("marker text");};
            value.to_well_formed_string().unwrap()
        }).collect()
    }
    fn append(product:&mut ProductDatabase<'_>,cursor:u64,tier:ledger::Tier,marker:&str) {
        let update=Json::Object(BTreeMap::from([
            (JsonString::from_str("content"),Json::Object(BTreeMap::from([
                (JsonString::from_str("text"),text(marker)),(JsonString::from_str("type"),text("text"))]))),
            (JsonString::from_str("sessionUpdate"),text("agent_message_chunk")),
        ])).canonical();
        ledger::record(&mut product.connection,&ledger::EventInput {event_id:format!("event{cursor}"),
            source_epoch:"synthetic-source".into(),source_cursor:cursor.to_string(),domain_id:"projectA".into(),
            seat_id:"seatA".into(),session_id:"sessionA".into(),tier,side_id:None,
            occurred_at:"synthetic-time".into(),update_json:update}).unwrap();
    }
    #[test]
    fn secretary_user_global_read_subscribe_and_released_history() {
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-secretary-ledger-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&path).unwrap();let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");let mut product=ProductDatabase::open(&root,&database).unwrap();
        // All rows below are synthetic qualification facts, not an observed H
        // process, actual model output, installed UI, login or grant.
        product.connection.execute("INSERT INTO gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','syntheticHome','syntheticIdentity','sha256:fixture','fixture','INSTALLED','LOGGED_OUT',1)").unwrap();
        seat::store_template(&mut product.connection,NativeOrigin::user(&product.owner),StoreTemplate {
            domain_id:"global",template_id:"secretaryTemplate",settings_json:br#"{}"#}).unwrap();
        let seat=seat::create(&mut product.connection,NativeOrigin::user(&product.owner),CreateSeat {
            domain_id:"global",seat_id:"secretarySeat",template_id:"secretaryTemplate",instance_id:None,
            kind:Kind::Long,request_id:"createSecretarySeat",request_bytes:b"synthetic secretary seat"}).unwrap().seat;
        seat::designate_secretary(&mut product.connection,&product.owner,&seat.seat_id,&seat.incarnation,
            "designateSecretary",b"synthetic designation").unwrap();
        let q=Statement::prepare(product.connection.as_ptr(),
            "UPDATE gogoke_v37_seats SET state='BUSY',instance_id='instanceA' WHERE domain_id='global' AND seat_id='secretarySeat'").unwrap();q.step_done().unwrap();drop(q);
        let generation="synthetic-generation";
        let q=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO gogoke_v37_h_owner_binding VALUES('secretaryBinding','instanceA','global','SESSION','secretarySession',?1,'ACTIVE')").unwrap();
        q.bind_text(1,generation).unwrap();q.step_done().unwrap();drop(q);
        let q=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,state,revision) VALUES('secretaryHome','instanceA','global','SESSION','secretarySession',?1,'ACTIVE',1)").unwrap();
        q.bind_text(1,generation).unwrap();q.step_done().unwrap();drop(q);
        let q=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES('global','secretarySession','instanceA','secretaryHome','secretaryBinding',?1,'COMMITTED',2)").unwrap();
        q.bind_text(1,generation).unwrap();q.step_done().unwrap();drop(q);
        let q=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO gogoke_v37_session_binding_v2(domain_id,session_id,seat_id,seat_incarnation,seat_authorization_generation,selected_instance_id,provenance) VALUES('global','secretarySession','secretarySeat',?1,?2,'instanceA','NATIVE_V2')").unwrap();
        q.bind_text(1,&seat.incarnation).unwrap();q.bind_i64(2,seat.generation).unwrap();q.step_done().unwrap();drop(q);
        ledger::register_session(&mut product.connection,&ledger::SessionRegistration {domain_id:"global".into(),
            seat_id:"secretarySeat".into(),session_id:"secretarySession".into(),purpose:ledger::SessionPurpose::Secretary,side_id:None}).unwrap();
        for (cursor,domain,seat_id,session,marker) in [(1,"projectA","seatA","workA","PROJECT_A"),
            (2,"projectB","seatB","workB","PROJECT_B"),(3,"global","secretarySeat","secretarySession","OWN_HISTORY")] {
            if domain!="global" {ledger::register_session(&mut product.connection,&ledger::SessionRegistration {
                domain_id:domain.into(),seat_id:seat_id.into(),session_id:session.into(),purpose:ledger::SessionPurpose::Work,side_id:None}).unwrap();}
            let update=Json::Object(BTreeMap::from([
                (JsonString::from_str("content"),Json::Object(BTreeMap::from([
                    (JsonString::from_str("text"),text(marker)),(JsonString::from_str("type"),text("text"))]))),
                (JsonString::from_str("sessionUpdate"),text("agent_message_chunk")),
            ])).canonical();
            ledger::record(&mut product.connection,&ledger::EventInput {event_id:format!("secretaryEvent{cursor}"),
                source_epoch:"synthetic-source".into(),source_cursor:"1".into(),domain_id:domain.into(),
                seat_id:seat_id.into(),session_id:session.into(),tier:if domain=="global" {ledger::Tier::Global} else {ledger::Tier::Project},
                side_id:None,occurred_at:"synthetic-time".into(),update_json:update}).unwrap();
        }
        let position=ledger::recover(&product.connection).unwrap();
        let payload=format!(r#"{{"readerSessionId":"secretarySession","scope":"GLOBAL","epoch":{},"afterCursor":"0"}}"#,text(&position.epoch).canonical());
        let query=global_request("scoped-query","globalQuery","ledger",position.cursor,&payload);
        let body=String::from_utf8(product.dispatch_user_request(&query).unwrap()).unwrap();
        for fact in ["PROJECT_A","PROJECT_B","OWN_HISTORY","domainId","sourceEpoch","sourceCursor"] {assert!(body.contains(fact),"missing {fact}");}
        assert_eq!(reply(&mut product,&query).status,V37Status::Replayed);
        let subscription=global_request("subscribe","globalSubscribe","globalSub",0,&payload);
        assert_eq!(reply(&mut product,&subscription).status,V37Status::Applied);
        product.connection.execute("UPDATE gogoke_v37_h_claim SET state='RELEASED' WHERE session_id='secretarySession'").unwrap();
        product.connection.execute("UPDATE gogoke_v37_h_owner_binding SET state='REVOKED' WHERE binding_id='secretaryBinding'").unwrap();
        product.connection.execute("UPDATE gogoke_v37_seats SET state='IDLE',generation=generation+1 WHERE domain_id='global' AND seat_id='secretarySeat'").unwrap();
        product.close_checked().unwrap();let mut product=ProductDatabase::open(&root,&database).unwrap();
        assert_eq!(reply(&mut product,&query).status,V37Status::Replayed);
        let resume=global_request("resume-subscription","globalResume","globalSub",1,
            &format!(r#"{{"epoch":{},"afterCursor":"{}"}}"#,text(&position.epoch).canonical(),position.cursor));
        assert_eq!(reply(&mut product,&resume).status,V37Status::Applied);
        let end=global_request("end-subscription","globalEnd","globalSub",2,"{}");
        assert_eq!(reply(&mut product,&end).status,V37Status::Applied);
        product.connection.execute("UPDATE gogoke_v37_session_binding_v2 SET provenance='LEGACY_V1' WHERE session_id='secretarySession'").unwrap();
        assert_eq!(reply(&mut product,&query).status,V37Status::Denied);
        product.connection.execute("UPDATE gogoke_v37_session_binding_v2 SET provenance='NATIVE_V2' WHERE session_id='secretarySession'").unwrap();
        product.connection.execute("UPDATE gogoke_v37_seats SET state='RECLAIMED' WHERE domain_id='global' AND seat_id='secretarySeat'").unwrap();
        assert_eq!(reply(&mut product,&query).status,V37Status::Denied);
        product.connection.execute("UPDATE gogoke_v37_seats SET state='IDLE' WHERE domain_id='global' AND seat_id='secretarySeat'").unwrap();
        product.connection.execute("UPDATE v37_ledger_session SET purpose='WORK' WHERE session_id='secretarySession'").unwrap();
        assert_eq!(reply(&mut product,&query).status,V37Status::Denied);
        product.close_checked().unwrap();drop(root);std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn native_user_ledger_scopes_replay_and_subscription_survive_reopen() {
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-user-ledger-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&path).unwrap();let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");let mut product=ProductDatabase::open(&root,&database).unwrap();
        // Synthetic native-store authorization and marker data, not a CLI,
        // provider event, login observation or process-custody proof.
        product.connection.execute("INSERT INTO gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','syntheticHome','syntheticIdentity','sha256:fixture','fixture','INSTALLED','LOGGED_OUT',1)").unwrap();
        seat::store_template(&mut product.connection,NativeOrigin::user(&product.owner),StoreTemplate {
            domain_id:"projectA",template_id:"templateA",settings_json:br#"{"model":"gpt-6-sol"}"#}).unwrap();
        seat::create(&mut product.connection,NativeOrigin::user(&product.owner),CreateSeat {
            domain_id:"projectA",seat_id:"seatA",template_id:"templateA",instance_id:Some("instanceA"),
            kind:Kind::Long,request_id:"createSeat",request_bytes:b"synthetic ledger authority control"}).unwrap();
        let seat=seat::get(&product.connection,"projectA","seatA").unwrap().unwrap();
        let generation=seat.generation.to_string();
        product.connection.execute("UPDATE gogoke_v37_seats SET state='BUSY' WHERE domain_id='projectA' AND seat_id='seatA'").unwrap();
        let row=Statement::prepare(product.connection.as_ptr(),"INSERT INTO gogoke_v37_h_owner_binding VALUES('bindingA','instanceA','projectA','SESSION','sessionA',?1,'ACTIVE')").unwrap();
        row.bind_text(1,&generation).unwrap();row.step_done().unwrap();drop(row);
        let row=Statement::prepare(product.connection.as_ptr(),"INSERT INTO gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,state,revision) VALUES('syntheticHome','instanceA','projectA','SESSION','sessionA',?1,'ACTIVE',1)").unwrap();
        row.bind_text(1,&generation).unwrap();row.step_done().unwrap();drop(row);
        let row=Statement::prepare(product.connection.as_ptr(),"INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES('projectA','sessionA','instanceA','syntheticHome','bindingA',?1,'COMMITTED',2)").unwrap();
        row.bind_text(1,&generation).unwrap();row.step_done().unwrap();drop(row);
        let row=Statement::prepare(product.connection.as_ptr(),"INSERT INTO gogoke_v37_h_seat_binding VALUES('projectA','sessionA','seatA',?1,?2)").unwrap();
        row.bind_text(1,&seat.incarnation).unwrap();row.bind_text(2,&generation).unwrap();row.step_done().unwrap();drop(row);
        ledger::register_session(&mut product.connection,&ledger::SessionRegistration {domain_id:"projectA".into(),
            seat_id:"seatA".into(),session_id:"sessionA".into(),purpose:ledger::SessionPurpose::Work,side_id:None}).unwrap();
        append(&mut product,1,ledger::Tier::Project,"PROJECT_MARKER");
        append(&mut product,2,ledger::Tier::Global,"GLOBAL_PRIVATE_MARKER");
        let position=ledger::recover(&product.connection).unwrap();
        let payload=format!(r#"{{"readerSessionId":"sessionA","scope":"PROJECT","epoch":{},"afterCursor":"0"}}"#,text(&position.epoch).canonical());
        let query=request("scoped-query","queryA","ledger",position.cursor,&payload);
        let first=product.dispatch_user_request(&query).unwrap();
        let body=std::str::from_utf8(&first).unwrap();assert!(body.contains("PROJECT_MARKER"));assert!(!body.contains("GLOBAL_PRIVATE_MARKER"));
        assert_eq!(reply(&mut product,&query).status,V37Status::Replayed);
        let collision=request("scoped-query","queryA","anotherLedger",position.cursor,&payload);
        assert_eq!(reply(&mut product,&collision).status,V37Status::Conflict);
        let global=request("scoped-query","globalDenied","ledger",position.cursor,&payload.replace("PROJECT","GLOBAL"));
        assert_eq!(reply(&mut product,&global).status,V37Status::Denied);
        let subscribe=request("subscribe","subscribeA","subA",0,&payload);
        let first=reply(&mut product,&subscribe);assert_eq!(first.status,V37Status::Applied);assert_eq!(first.revision,1);
        let original=first.into_result();
        let replay=reply(&mut product,&subscribe).into_result();
        assert_eq!(Json::Object(original).canonical(),Json::Object(replay).canonical(),"write replay preserves the full immutable result");
        product.close_checked().unwrap();let mut product=ProductDatabase::open(&root,&database).unwrap();
        assert_eq!(reply(&mut product,&subscribe).status,V37Status::Replayed);
        append(&mut product,3,ledger::Tier::Project,"AFTER_RESTART");
        assert_eq!(reply(&mut product,&query).status,V37Status::Stale);
        let resume_payload=format!(r#"{{"epoch":{},"afterCursor":"{}"}}"#,text(&position.epoch).canonical(),position.cursor);
        let resume=request("resume-subscription","resumeA","subA",1,&resume_payload);
        let resumed=product.dispatch_user_request(&resume).unwrap();assert!(std::str::from_utf8(&resumed).unwrap().contains("AFTER_RESTART"));
        assert_eq!(crate::store::session_transport::decode_receipt(&resumed).unwrap().revision,2);
        let end=request("end-subscription","endA","subA",2,"{}");assert_eq!(reply(&mut product,&end).status,V37Status::Applied);
        let ended=request("resume-subscription","resumeEnded","subA",3,&resume_payload);
        assert_eq!(reply(&mut product,&ended).status,V37Status::Conflict);
        let wrong_revision=request("subscribe","wrongSubscribeRevision","newSub",1,&payload);
        assert_eq!(reply(&mut product,&wrong_revision).status,V37Status::Stale);
        let duplicate=request("subscribe","duplicateSubscription","subA",3,&payload);
        assert_eq!(reply(&mut product,&duplicate).status,V37Status::Conflict);
        let wrong_epoch=request("subscribe","oldEpoch","newSub",0,
            &payload.replace(&text(&position.epoch).canonical(),"\"obsolete-epoch\""));
        assert_eq!(reply(&mut product,&wrong_epoch).status,V37Status::Stale);
        let ahead=request("subscribe","cursorAhead","newSub",0,&format!(r#"{{"readerSessionId":"sessionA","scope":"PROJECT","epoch":{},"afterCursor":"{}"}}"#,text(&position.epoch).canonical(),position.cursor+100));
        assert_eq!(reply(&mut product,&ahead).status,V37Status::Conflict);
        for cursor in 4..=35 {append(&mut product,cursor,ledger::Tier::Project,&format!("PAGED_MARKER_{cursor}"));}
        let head=ledger::recover(&product.connection).unwrap();
        let first_page=request("scoped-query","pageOne","ledger",head.cursor,&payload);
        let page=reply(&mut product,&first_page).into_result();
        let Some(Json::Array(rows))=page.get(&JsonString::from_str("events")) else {panic!("page events");};
        assert_eq!(rows.len(),32);
        let mut seen=markers(rows);
        let Some(Json::String(cursor))=page.get(&JsonString::from_str("cursor")) else {panic!("page cursor");};
        let cursor=number(&cursor.to_well_formed_string().unwrap()).unwrap();
        assert!(cursor<head.cursor,"page continuation cannot jump to the source high-water");
        let second_page=request("scoped-query","pageTwo","ledger",head.cursor,&format!(r#"{{"readerSessionId":"sessionA","scope":"PROJECT","epoch":{},"afterCursor":"{cursor}"}}"#,text(&head.epoch).canonical()));
        let page=reply(&mut product,&second_page).into_result();
        let Some(Json::Array(rows))=page.get(&JsonString::from_str("events")) else {panic!("tail events");};
        assert_eq!(rows.len(),2,"every visible event after the full page remains readable");
        seen.extend(markers(rows));
        let expected=std::iter::once("PROJECT_MARKER".to_owned()).chain(std::iter::once("AFTER_RESTART".to_owned()))
            .chain((4..=35).map(|cursor|format!("PAGED_MARKER_{cursor}"))).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(seen,expected,"both pages contain every original visible marker exactly once and no global marker");
        product.connection.execute("UPDATE gogoke_v37_h_owner_binding SET state='REVOKED' WHERE binding_id='bindingA'").unwrap();
        assert_eq!(reply(&mut product,&subscribe).status,V37Status::Denied,"valid requests receive an explicit current-authority denial, including immutable replay");
        product.close_checked().unwrap();drop(root);std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn ledger_commit_checks_the_actual_replayed_frame_boundary() {
        let request=request("subscribe","frameBoundary","subBoundary",0,"{}");
        let result=|length:usize|BTreeMap::from([(JsonString::from_str("padding"),text(&"x".repeat(length)))]);
        let overhead=encode_receipt(&request,V37Status::Applied,0,1,result(0)).len();
        let exact=crate::ipc::MAX_FRAME_BYTES-overhead;
        assert_eq!(encode_receipt(&request,V37Status::Applied,0,1,result(exact)).len(),crate::ipc::MAX_FRAME_BYTES);
        assert!(committed_receipt(&request,0,1,result(exact)).is_err(),"a first reply that cannot be replayed must not commit");
        let first=committed_receipt(&request,0,1,result(exact-1)).unwrap();
        let original=crate::store::session_transport::decode_receipt(&first).unwrap();
        let replay=encode_receipt(&request,V37Status::Replayed,original.previous_revision,original.revision,original.into_result());
        assert_eq!(replay.len(),crate::ipc::MAX_FRAME_BYTES,"the largest admitted original still fits the actual replay encoding");
    }
}
fn events(values:&[LedgerEvent])->Result<Json> {
    values.iter().map(|event| {
        Ok(Json::Object(BTreeMap::from([
            (JsonString::from_str("cursor"),text(&event.cursor.to_string())),
            (JsonString::from_str("sourceEventId"),text(&event.input.event_id)),
            (JsonString::from_str("sourceEpoch"),text(&event.input.source_epoch)),
            (JsonString::from_str("sourceCursor"),text(&event.input.source_cursor)),
            (JsonString::from_str("domainId"),text(&event.input.domain_id)),
            (JsonString::from_str("seatId"),text(&event.input.seat_id)),
            (JsonString::from_str("sessionId"),text(&event.input.session_id)),
            (JsonString::from_str("update"),crate::store::atomic::Parser::parse(&event.input.update_json)?),
        ])))
    }).collect::<Result<Vec<_>>>().map(Json::Array)
}
fn subscription_result(sub:&Subscription,page:&[LedgerEvent])->Result<BTreeMap<JsonString,Json>> {
    Ok(BTreeMap::from([
        (JsonString::from_str("subscriptionId"),text(&sub.id)),
        (JsonString::from_str("epoch"),text(&sub.epoch)),
        (JsonString::from_str("cursor"),text(&sub.cursor.to_string())),
        (JsonString::from_str("state"),text(if sub.active {"ACTIVE"} else {"ENDED"})),
        (JsonString::from_str("events"),events(page)?),
    ]))
}
fn committed_receipt(request:&V37Request,previous:u64,revision:u64,result:BTreeMap<JsonString,Json>)->Result<Vec<u8>> {
    let bytes=encode_receipt(request,V37Status::Applied,previous,revision,result);
    // Same-result replay changes only this status token. Reserve exactly its
    // encoded length difference before committing a subscription or receipt.
    let replay_length=bytes.len().checked_add("REPLAYED".len()-"APPLIED".len())
        .ok_or(OrchestrationError::Invalid("ledger receipt length overflow"))?;
    if replay_length>crate::ipc::MAX_FRAME_BYTES {return Err(OrchestrationError::Invalid("ledger receipt replay bound"));}
    Ok(bytes)
}

impl<'root> ProductDatabase<'root> {
    /// Owner USER read authority over a previously admitted Secretary session.
    /// Original A purpose and H NATIVE_V2 binding remain after H release; E's
    /// singleton still has to designate that exact USER/LONG incarnation.
    fn native_secretary_ledger_reader(&self,session:&str)->Result<Reader> {
        let binding=crate::store::session_transport::session_binding::read(&self.connection,"global",session)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("ledger original H binding: {error:?}")))?
            .ok_or(OrchestrationError::AccessDenied)?;
        if binding.provenance!=crate::store::session_transport::session_binding::Provenance::NativeV2 {
            return Err(OrchestrationError::AccessDenied);
        }
        let designation=crate::store::seat::read_secretary_configuration_in_transaction(&self.connection,&self.owner)?;
        let crate::store::seat::SecretaryConfiguration::Designated {seat_id,incarnation,state,..}=designation else {
            return Err(OrchestrationError::AccessDenied);
        };
        if state==crate::store::seat::State::Reclaimed || seat_id!=binding.seat_id
            || incarnation!=binding.seat_incarnation {
            return Err(OrchestrationError::AccessDenied);
        }
        let row=Statement::prepare(self.connection.as_ptr(),
            "SELECT l.domain_id,l.seat_id,l.session_id
             FROM main.v37_ledger_session l
             JOIN main.gogoke_v37_h_claim h ON h.domain_id=l.domain_id AND h.session_id=l.session_id
             JOIN main.gogoke_v37_h_owner_binding b ON b.binding_id=h.binding_id
               AND b.domain_id=h.domain_id AND b.instance_id=h.instance_id
               AND b.kind='SESSION' AND b.owner_id=h.session_id AND b.generation=h.generation
             JOIN main.gogoke_v37_instance_homes home ON home.home_id=h.home_id
               AND home.domain_id=h.domain_id AND home.instance_id=h.instance_id
               AND home.kind='SESSION' AND home.owner_id=h.session_id AND home.generation=h.generation
             JOIN main.gogoke_v37_session_binding_v2 v ON v.domain_id=h.domain_id
               AND v.session_id=h.session_id AND v.provenance='NATIVE_V2'
               AND v.selected_instance_id=h.instance_id AND v.seat_id=l.seat_id
             JOIN main.gogoke_v37_seat_secretary d ON d.singleton=1 AND d.domain_id='global'
               AND d.seat_id=v.seat_id AND d.incarnation=v.seat_incarnation
             JOIN main.gogoke_v37_seats s ON s.domain_id=d.domain_id AND s.seat_id=d.seat_id
               AND s.incarnation=d.incarnation AND s.layer='USER' AND s.kind='LONG'
               AND s.state<>'RECLAIMED'
             WHERE l.domain_id='global' AND l.session_id=?1 AND l.purpose='SECRETARY'
               AND h.state IN ('COMMITTED','STOPPED','RELEASED')
               AND (b.state='ACTIVE' OR h.state='RELEASED')")?;
        row.bind_text(1,session)?;
        if !row.step_row()? {return Err(OrchestrationError::AccessDenied);}
        let reader=Reader {domain_id:row.column_text(0)?,seat_id:row.column_text(1)?,session_id:row.column_text(2)?};
        if reader.seat_id!=binding.seat_id || reader.session_id!=binding.session_id {
            return Err(OrchestrationError::AccessDenied);
        }
        if row.step_row()? {return Err(OrchestrationError::OperationConflict);}
        Ok(reader)
    }

    fn native_ledger_reader(&self,request:&V37Request,session:&str)->Result<Reader> {
        let relationship=crate::store::session_transport::session_binding::current_relationship(
            &self.connection,&request.domain_id,session)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("ledger H/E relationship: {error:?}")))?
            .ok_or(OrchestrationError::AccessDenied)?;
        let row=Statement::prepare(self.connection.as_ptr(),
            "SELECT h.domain_id,l.seat_id,h.session_id,h.generation FROM main.gogoke_v37_h_claim h
             JOIN main.gogoke_v37_h_owner_binding b ON b.binding_id=h.binding_id
               AND b.domain_id=h.domain_id AND b.instance_id=h.instance_id
               AND b.kind='SESSION' AND b.owner_id=h.session_id AND b.generation=h.generation
             JOIN main.v37_ledger_session l ON l.session_id=h.session_id
               AND l.domain_id=h.domain_id AND l.purpose<>'FORMAL_REVIEW'
             WHERE h.domain_id=?1 AND h.session_id=?2 AND h.state IN ('COMMITTED','STOPPED')
               AND b.state='ACTIVE'")?;
        row.bind_text(1,&request.domain_id)?;row.bind_text(2,session)?;
        if !row.step_row()? {return Err(OrchestrationError::AccessDenied);}
        let reader=Reader {domain_id:row.column_text(0)?,seat_id:row.column_text(1)?,session_id:row.column_text(2)?};
        if reader.seat_id!=relationship.seat_id || row.column_text(3)?!=relationship.session_generation {
            return Err(OrchestrationError::AccessDenied);
        }
        if row.step_row()? {return Err(OrchestrationError::OperationConflict);}
        Ok(reader)
    }

    pub(super) fn dispatch_user_ledger(&mut self,request:&V37Request)->Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection,&self.owner)?;
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let result=self.user_ledger_in_transaction(request);
        match result {
            Ok(bytes) if bytes.len()<=crate::ipc::MAX_FRAME_BYTES=>{self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;Ok(bytes)},
            Ok(_)=>{self.connection.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?;Err(OrchestrationError::Invalid("ledger reply transport bound"))},
            Err(primary)=>{
                if let Err(error)=self.connection.execute("ROLLBACK") {return Err(OrchestrationError::V37StoreFailure(format!("ledger operation: {primary:?}; rollback: {error:?}")));}
                Err(primary)
            }
        }
    }

    fn user_ledger_in_transaction(&mut self,request:&V37Request)->Result<Vec<u8>> {
        authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
        let prior=Statement::prepare(self.connection.as_ptr(),
            "SELECT lower(hex(request_bytes)),receipt_bytes FROM main.v37_ledger_receipt WHERE family='K-LEDGER' AND domain_id=?1 AND request_id=?2")?;
        prior.bind_text(1,&request.domain_id)?;prior.bind_text(2,&request.request_id)?;
        let mut original=if prior.step_row()? {
            let exact:String=request.raw_bytes.iter().map(|value|format!("{value:02x}")).collect();
            if prior.column_text(0)?!=exact {
                return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,request.expected_revision,Default::default()));
            }
            Some(crate::store::session_transport::decode_receipt(prior.column_text(1)?.as_bytes())
                .map_err(|error|OrchestrationError::V37StoreFailure(format!("ledger prior receipt: {error:?}")))?)
        } else {None};drop(prior);
        if !matches!(request.operation.as_str(),"scoped-query"|"subscribe"|"resume-subscription"|"end-subscription") {
            return Ok(encode_receipt(request,V37Status::Unsupported,request.expected_revision,
                request.expected_revision,BTreeMap::from([(JsonString::from_str("reason"),text("LEDGER_RECORD_REQUIRES_NATIVE_SOURCE_FACT"))])));
        }
        let (session,global)=if matches!(request.operation.as_str(),"scoped-query"|"subscribe") {
            (user_payload_string(request,"readerSessionId")?,
                user_payload_string(request,"scope")?=="GLOBAL")
        } else {
            let row=Statement::prepare(self.connection.as_ptr(),
                "SELECT reader_id,reader_kind FROM main.v37_ledger_subscription WHERE subscription_id=?1 AND domain_id=?2")?;
            row.bind_text(1,&request.target_id)?;row.bind_text(2,&request.domain_id)?;
            if !row.step_row()? {return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,request.expected_revision,Default::default()));}
            (row.column_text(0)?,row.column_text(1)?=="GLOBAL")
        };
        let selected=if global && request.domain_id=="global" {
            self.native_secretary_ledger_reader(&session)
        } else if !global {self.native_ledger_reader(request,&session)}
        else {Err(OrchestrationError::AccessDenied)};
        let reader=match selected {
            Ok(reader)=>reader,
            Err(OrchestrationError::AccessDenied)=>return Ok(encode_receipt(request,V37Status::Denied,
                request.expected_revision,request.expected_revision,Default::default())),
            Err(error)=>return Err(error),
        };
        let expected_scope=if global {"GLOBAL"} else {"PROJECT"};
        let position=ledger::recover(&self.connection)?;
        if request.operation!="scoped-query" {
            if let Some(original)=original.take() {
                // Subscription writes replay their immutable receipt while
                // current authority and the same native reader still hold.
                let previous=original.previous_revision;
                let revision=original.revision;
                return Ok(encode_receipt(request,V37Status::Replayed,previous,revision,original.into_result()));
            }
        }
        let (previous,revision,result)=if request.operation=="scoped-query" {
            if request.payload.len()!=4 || user_payload_string(request,"scope")?!=expected_scope {
                return Ok(encode_receipt(request,V37Status::Denied,position.cursor,position.cursor,Default::default()));
            }
            if request.expected_revision!=position.cursor {
                return Ok(encode_receipt(request,V37Status::Stale,position.cursor,position.cursor,Default::default()));
            }
            let after=LedgerPosition {epoch:user_payload_string(request,"epoch")?,cursor:number(&user_payload_string(request,"afterCursor")?)?};
            if after.epoch!=position.epoch {
                return Ok(encode_receipt(request,V37Status::Stale,position.cursor,position.cursor,
                    BTreeMap::from([(JsonString::from_str("reason"),text("STALE_EPOCH"))])));
            }
            if after.cursor>position.cursor {
                return Ok(encode_receipt(request,V37Status::Conflict,position.cursor,position.cursor,
                    BTreeMap::from([(JsonString::from_str("reason"),text("CURSOR_AHEAD"))])));
            }
            let page=if global {ledger::query_global(&self.connection,&after,32)?}
                else {ledger::query(&self.connection,&reader,&after,32)?};
            let next_cursor=if page.events.len()==32 {page.events.last().map_or(after.cursor,|event|event.cursor)}
                else {page.position.cursor};
            (page.position.cursor,page.position.cursor,BTreeMap::from([
                (JsonString::from_str("epoch"),text(&page.position.epoch)),
                (JsonString::from_str("cursor"),text(&next_cursor.to_string())),
                (JsonString::from_str("highWaterCursor"),text(&page.position.cursor.to_string())),
                (JsonString::from_str("events"),events(&page.events)?),
            ]))
        } else {
            let (before,page)=if request.operation=="subscribe" {
                if request.payload.len()!=4 || user_payload_string(request,"scope")?!=expected_scope {
                    return Ok(encode_receipt(request,V37Status::Denied,0,0,Default::default()));
                }
                let row=Statement::prepare(self.connection.as_ptr(),
                    "SELECT revision,domain_id,reader_id,reader_kind FROM main.v37_ledger_subscription WHERE subscription_id=?1")?;
                row.bind_text(1,&request.target_id)?;
                let existing=if row.step_row()? {
                    if row.column_text(1)?!=request.domain_id || row.column_text(2)?!=reader.session_id
                        || row.column_text(3)?!=expected_scope {
                        return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,request.expected_revision,Default::default()));
                    }
                    Some(number(&row.column_text(0)?)?)
                } else {None};drop(row);
                let current=existing.unwrap_or(0);
                if request.expected_revision!=current {return Ok(encode_receipt(request,V37Status::Stale,current,current,Default::default()));}
                if existing.is_some() {return Ok(encode_receipt(request,V37Status::Conflict,current,current,Default::default()));}
                let after=LedgerPosition {epoch:user_payload_string(request,"epoch")?,cursor:number(&user_payload_string(request,"afterCursor")?)?};
                if after.epoch!=position.epoch {return Ok(encode_receipt(request,V37Status::Stale,0,0,Default::default()));}
                if after.cursor>position.cursor {return Ok(encode_receipt(request,V37Status::Conflict,0,0,
                    BTreeMap::from([(JsonString::from_str("reason"),text("CURSOR_AHEAD"))])));}
                (0,if global {ledger::subscribe_global(&mut self.connection,&reader,&request.target_id,&after,32)?}
                    else {ledger::subscribe(&mut self.connection,&reader,&request.target_id,&after,32)?})
            } else {
                let row=Statement::prepare(self.connection.as_ptr(),"SELECT revision,epoch,cursor,state FROM main.v37_ledger_subscription WHERE subscription_id=?1 AND reader_id=?2")?;
                row.bind_text(1,&request.target_id)?;row.bind_text(2,&reader.session_id)?;
                if !row.step_row()? {return Err(OrchestrationError::AccessDenied);}
                let current=number(&row.column_text(0)?)?;
                let epoch=row.column_text(1)?;let cursor=number(&row.column_text(2)?)?;
                let active=row.column_text(3)?=="ACTIVE";drop(row);
                if current!=request.expected_revision {return Ok(encode_receipt(request,V37Status::Stale,current,current,Default::default()));}
                if !active {return Ok(encode_receipt(request,V37Status::Conflict,current,current,Default::default()));}
                if request.operation=="end-subscription" {
                    if !request.payload.is_empty() {return Err(OrchestrationError::Invalid("ledger end payload"));}
                    let ended=if global {ledger::end_global_subscription(&mut self.connection,&reader,&request.target_id,current)?}
                        else {ledger::end_subscription(&mut self.connection,&reader,&request.target_id,current)?};
                    (current,ledger::SubscriptionPage {subscription:ended,events:Vec::new()})
                } else {
                    if request.payload.len()!=2 {return Err(OrchestrationError::Invalid("ledger resume payload"));}
                    let after=LedgerPosition {epoch:user_payload_string(request,"epoch")?,cursor:number(&user_payload_string(request,"afterCursor")?)?};
                    if after.epoch!=epoch {return Ok(encode_receipt(request,V37Status::Stale,current,current,Default::default()));}
                    if after.cursor!=cursor {
                        return Ok(encode_receipt(request,V37Status::Conflict,current,current,BTreeMap::from([
                            (JsonString::from_str("reason"),text(if after.cursor<cursor {"CURSOR_REWIND"} else {"CURSOR_GAP"}))])));
                    }
                    (current,if global {ledger::resume_global_subscription(&mut self.connection,&reader,&request.target_id,&after,32)?}
                        else {ledger::resume_subscription(&mut self.connection,&reader,&request.target_id,&after,32)?})
                }
            };
            (before,page.subscription.revision,subscription_result(&page.subscription,&page.events)?)
        };
        if let Some(original)=original {
            let prior_result=original.into_result();
            let same=prior_result.len()==result.len() && prior_result.iter().all(|(key,value)|result.get(key)
                .is_some_and(|current|current.canonical()==value.canonical()));
            return Ok(encode_receipt(request,if same {V37Status::Replayed} else {V37Status::Stale},previous,revision,
                if same {prior_result} else {BTreeMap::new()}));
        }
        let bytes=committed_receipt(request,previous,revision,result)?;
        let insert=Statement::prepare(self.connection.as_ptr(),
            "INSERT INTO main.v37_ledger_receipt(family,domain_id,request_id,request_bytes,receipt_bytes) VALUES('K-LEDGER',?1,?2,?3,?4)")?;
        insert.bind_text(1,&request.domain_id)?;insert.bind_text(2,&request.request_id)?;
        insert.bind_blob(3,&request.raw_bytes)?;insert.bind_blob(4,&bytes)?;insert.step_done()?;
        Ok(bytes)
    }
}
