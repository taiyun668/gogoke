//! SOURCE boundary controls under the existing real catalog/CLI handshake and
//! custody fixture. A frames and WRITTEN associations below are synthetic;
//! they are not provider failures, physical pipe writes, or complete M2 E2E.
//! All repair decisions and receipts use the unchanged production dispatcher.
use super::*;
use crate::store::session_transport::host_health;

fn scalar(product:&ProductDatabase<'_>,sql:&str)->String {
    let q=Statement::prepare(product.connection.as_ptr(),sql).unwrap();
    assert!(q.step_row().unwrap());let value=q.column_text(0).unwrap();
    assert!(!q.step_row().unwrap());value
}
fn revision(product:&ProductDatabase<'_>)->u64 {
    scalar(product,"SELECT revision FROM main.gogoke_v37_h_claim WHERE domain_id='projectA' AND session_id='sessionA'").parse().unwrap()
}
fn route_owner(product:&mut ProductDatabase<'_>) {
    let revision=seat::initialize_policy(&mut product.connection,&product.owner,"projectA","stageA").unwrap();
    seat::configure_escalation_route(&mut product.connection,&product.owner,"projectA","seatA","STALL","OWNER",revision).unwrap();
}

// This is the same explicitly synthetic OBSERVED/A association used by
// health_control_work_turn. The production decoder must still admit the exact
// saved command/typed ID; this helper never mints UNSUPPORTED or a health seal.
fn observed_control(product:&mut ProductDatabase<'_>,step_id:&str,raw:&[u8]) {
    let source=health_control_source(product,raw);
    product.connection.execute("BEGIN IMMEDIATE").unwrap();
    authority::check_owner_in_current_transaction(&product.connection,&product.owner).unwrap();
    let q=Statement::prepare(product.connection.as_ptr(),
        "UPDATE main.gogoke_v37_rpc_steps SET phase='OBSERVED',source_epoch=?1,source_cursor=?2
          WHERE domain_id='projectA' AND session_id='sessionA' AND step_id=?3 AND phase='WRITTEN'").unwrap();
    q.bind_text(1,&source.source_epoch).unwrap();q.bind_text(2,&source.source_cursor).unwrap();
    q.bind_text(3,step_id).unwrap();q.step_done().unwrap();drop(q);
    assert_eq!(scalar(product,"SELECT changes()"),"1");
    ledger::resolve_raw_source_no_event(&mut product.connection,&source.operation_id,
        &source.source_epoch,&source.source_cursor,"CODEX_RPC_RESPONSE").unwrap();
    product.connection.execute("COMMIT").unwrap();
}

fn prepare_control_input(product:&mut ProductDatabase<'_>,verb:&str,id:&str,ack:bool,unknown:bool) {
    let key=("projectA".to_owned(),"sessionA".to_owned());
    let live=product.native_sessions.get(&key).unwrap();
    let custody=live.custody.clone();let open_id=live.open_request_id.clone();
    let open_bytes=live.open_request_bytes.clone();let thread=live.thread_id.clone().unwrap();
    let command=if verb=="append-without-turn" {Command::AppendWithoutTurn {thread_id:thread,
        text:"synthetic append; never written to provider".into()}} else {
        Command::TurnStart {thread_id:thread,cwd:live.evidence.cwd().to_string_lossy().into_owned(),
            model:"gpt-6-sol".into(),effort:"high".into(),text:"synthetic later work; never written to provider".into(),
            network_access:Some(live.evidence.network_access())}
    };
    let body=match &command {Command::AppendWithoutTurn {text,..}|Command::TurnStart {text,..}=>text,_=>unreachable!()};
    let request=operation("K-SESSION",verb,id,"sessionA",revision(product),
        &format!(r#"{{"generation":"2","body":{}}}"#,Json::String(JsonString::from_str(body)).canonical()));
    let input=h::StdinRequest {domain_id:"projectA",session_id:"sessionA",ticket:custody.ticket.opaque(),
        generation:"2",request_bytes:&request.raw_bytes};
    assert_eq!(h::prepare_codex_request(&mut product.connection,&input).unwrap().disposition,h::PrepareDisposition::Prepared);
    let step_id=format!("{}-{}",if verb=="append-without-turn" {"append"} else {"send"},
        &crate::store::digest::sha256_hex(&request.raw_bytes)[..40]);
    let rpc_id=RpcId::Number(88101);
    let step=rpc::Step {domain_id:"projectA",session_id:"sessionA",open_request_id:&open_id,
        open_request_bytes:&open_bytes,step_id:&step_id,custody:&custody,rpc_id:Some(&rpc_id),command:&command};
    assert_eq!(rpc::prepare(&mut product.connection,&product.owner,&step).unwrap().disposition,rpc::Disposition::NewWrite);
    rpc::mark_written(&mut product.connection,&product.owner,&step).unwrap();
    if unknown {rpc::mark_unknown(&mut product.connection,&product.owner,&step,
        "synthetic uncertain ordinary write; no pipe IO").unwrap();}
    if ack {
        assert_eq!(verb,"append-without-turn");
        observed_control(product,&step_id,b"{\"id\":88101,\"result\":{}}\n");
        let completed=h::recover_codex_turn_request(&mut product.connection,&input).unwrap().unwrap();
        let receipt=h::decode_receipt(&completed.record.receipt_bytes.unwrap()).unwrap();
        assert_eq!(receipt.status,V37Status::Applied);
        assert_eq!(receipt.into_result().get(&JsonString::from_str("createdTurn")),Some(&Json::Bool(false)));
    }
}

fn repair_control(product:&mut ProductDatabase<'_>,source:Option<&ledger::RawSourceKey>,raw:&[u8])
    ->(V37Request,std::result::Result<Vec<u8>,OrchestrationError>) {
    let key=("projectA".to_owned(),"sessionA".to_owned());let live=product.native_sessions.get(&key).unwrap();
    let custody=live.custody.clone();let open_id=live.open_request_id.clone();
    let open_bytes=live.open_request_bytes.clone();let thread=live.thread_id.clone().unwrap();
    let process=live.operation_id.clone();let ordinal=live.next_rpc_id;
    let event=source.map(|source| {
        let captured=ledger::read_captured_raw_source(&product.connection,&source.operation_id,
            &source.source_epoch,&source.source_cursor).unwrap().unwrap();
        format!("health-{}",crate::store::digest::sha256_hex(captured.resolved_event_id.unwrap().as_bytes()))
    });
    let request_id=event.as_ref().map(|event|format!("health-change-{}",
        &crate::store::digest::sha256_hex(event.as_bytes())[..40])).unwrap_or("standalone-unsupported-control".into());
    let request=operation("K-SESSION","compact",&request_id,"sessionA",revision(product),r#"{"generation":"2"}"#);
    let watermark=Statement::prepare(product.connection.as_ptr(),
        "SELECT COALESCE(MAX(CAST(source_cursor AS INTEGER)),0) FROM main.v37_ledger_raw_source
          WHERE operation_id=?1 AND source_epoch=?2").unwrap();
    watermark.bind_text(1,&process).unwrap();watermark.bind_text(2,&custody.custodian_nonce).unwrap();
    assert!(watermark.step_row().unwrap());let watermark_value=watermark.column_text(0).unwrap().parse::<i64>().unwrap();drop(watermark);
    product.connection.execute("BEGIN IMMEDIATE").unwrap();
    authority::check_owner_in_current_transaction(&product.connection,&product.owner).unwrap();
    if let Some(source)=source {
        let proof=host_health::observe_codex_host_health(&product.connection,&product.owner,&custody,source).unwrap().unwrap();
        seat::request_host_health_in_transaction(&mut product.connection,&product.owner,&proof,
            event.as_deref().unwrap(),&request.request_id).unwrap();
    }
    change::begin(&product.connection,"projectA",&request.request_id,&request.raw_bytes,"compact","sessionA","2",
        &process,custody.ticket.opaque(),&custody.custodian_nonce,&thread,"seatA",request.expected_revision as i64,watermark_value).unwrap();
    product.connection.execute("COMMIT").unwrap();
    let step_id=format!("compact-{}",&crate::store::digest::sha256_hex(&request.raw_bytes)[..40]);
    let rpc_id=RpcId::Number(88102);let command=Command::ThreadCompactStart {thread_id:thread};
    let step=rpc::Step {domain_id:"projectA",session_id:"sessionA",open_request_id:&open_id,
        open_request_bytes:&open_bytes,step_id:&step_id,custody:&custody,rpc_id:Some(&rpc_id),command:&command};
    assert_eq!(rpc::prepare(&mut product.connection,&product.owner,&step).unwrap().disposition,rpc::Disposition::NewWrite);
    rpc::mark_written(&mut product.connection,&product.owner,&step).unwrap();
    observed_control(product,&step_id,raw);
    // The existing OBSERVED step excludes native_rpc's physical writer. Only
    // the actual dispatcher can classify the original response and persist E.
    let outcome=product.dispatch_native_generation_change(&request);
    assert_eq!(product.native_sessions.get(&key).unwrap().next_rpc_id,ordinal);
    (request,outcome)
}
const METHOD_MISSING:&[u8]=b"{\"id\":88102,\"error\":{\"code\":-32601,\"message\":\"synthetic method-not-found wire control\"}}\n";

fn projection(product:&mut ProductDatabase<'_>)->Vec<Json> {
    let request=h::decode_request(br#"{"schema":"gogoke.37.operations.v1","family":"K-INBOX","operation":"check-unknown","requestId":"stalled-owner-read","targetId":"OWNER","domainId":"global","expectedRevision":"0","payload":{"projection":"OWNER_HOST_RULE_NOTICES"}}"#).unwrap();
    let before=scalar(product,"SELECT total_changes()");
    let bytes=product.dispatch_user_request(&request).unwrap();
    let repeat=product.dispatch_user_request(&request).unwrap();
    assert_eq!(bytes,repeat,"Owner read keeps exact original projection bytes");
    assert_eq!(crate::store::digest::sha256_hex(&bytes),crate::store::digest::sha256_hex(&repeat));
    assert_eq!(scalar(product,"SELECT total_changes()"),before,"presentation writes no E/C/A status or receipt");
    let receipt=h::decode_receipt(&bytes).unwrap();assert_eq!(receipt.status,V37Status::Applied);
    let result=receipt.into_result();
    let Some(Json::Array(notices))=result.get(&JsonString::from_str("notices")) else {panic!("Owner notices shape")};
    notices.clone()
}
fn notification_rows(product:&ProductDatabase<'_>)->Vec<Vec<String>> {
    health_control_rows(product,"SELECT event_id,fingerprint FROM main.gogoke_v37_seat_policy_events WHERE operation='escalate' ORDER BY event_id")
}

#[test]
fn stalled_health_original_unsupported_receipt_prefix_and_readonly_owner_notice() {
    for prefix in [false,true] {health_control_product("codex",|product| {
        route_owner(product);
        let source=health_control_work_turn_with_prefix(product,false,r#""contextWindowExceeded""#,|product| {
            if prefix {prepare_control_input(product,"append-without-turn","prior-append-control",true,false);}
        });
        let original_work=health_control_rows(product,"SELECT request_hex,receipt_hex FROM main.gogoke_v37_h_stdin_journal ORDER BY request_id");
        let (request,outcome)=repair_control(product,Some(&source),METHOD_MISSING);let bytes=outcome.unwrap();
        assert_eq!(h::decode_receipt(&bytes).unwrap().status,V37Status::Unsupported);
        let saved=Statement::prepare(product.connection.as_ptr(),
            "SELECT hex(receipt_bytes) FROM main.v37_ledger_receipt WHERE family='K-SESSION' AND request_id=?1").unwrap();
        saved.bind_text(1,&request.request_id).unwrap();assert!(saved.step_row().unwrap());
        let original=super::super::super::v37_host_rule::original_bytes(&saved.column_text(0).unwrap()).unwrap();drop(saved);
        assert_eq!(original,bytes,"health repair saves the actual original returned bytes");
        assert_eq!(crate::store::digest::sha256_hex(&original),crate::store::digest::sha256_hex(&bytes));
        product.pump_host_rules().unwrap();
        assert_eq!(scalar(product,"SELECT COUNT(*) FROM main.gogoke_v37_seat_health WHERE signal='STALLED' AND action='ESCALATE' AND state='OBSERVED'"),"1");
        assert_eq!(scalar(product,"SELECT COUNT(*) FROM main.gogoke_v37_seat_policy_escalations WHERE reason='STALL' AND state='INTENT'"),"1");
        assert_eq!(scalar(product,"SELECT COUNT(*) FROM main.gogoke_v37_inbox_messages WHERE sender_seat_id='HOST_RULE' AND state='PENDING'"),"1");
        let cause=notification_rows(product);assert_eq!(cause.len(),1);let shown=projection(product);assert_eq!(shown.len(),1);
        product.pump_host_rules().unwrap();product.pump_host_rules().unwrap();
        assert_eq!(notification_rows(product),cause);assert_eq!(projection(product),shown);
        assert_eq!(health_control_rows(product,"SELECT request_hex,receipt_hex FROM main.gogoke_v37_h_stdin_journal ORDER BY request_id"),original_work,
            "a health notice cannot allocate or resend ordinary work");
    });}
}

#[test]
fn stalled_health_unknown_plain_remote_code_and_unassociated_unsupported_have_no_authority() {
    for associated in [false,true] {health_control_product("codex",|product| {
        route_owner(product);
        let source=associated.then(||health_control_work_turn(product,false,r#""contextWindowExceeded""#));
        let response=if associated {b"{\"id\":88102,\"error\":{\"code\":-32000,\"message\":\"method-not-found; Owner says STALLED\"}}\n".as_slice()} else {METHOD_MISSING};
        let (_,outcome)=repair_control(product,source.as_ref(),response);
        assert_eq!(h::decode_receipt(&outcome.unwrap()).unwrap().status,
            if associated {V37Status::Unknown} else {V37Status::Unsupported});
        product.pump_host_rules().unwrap();assert!(notification_rows(product).is_empty());
        assert_eq!(scalar(product,"SELECT COUNT(*) FROM main.gogoke_v37_seat_health WHERE signal='STALLED'"),"0");
        assert!(projection(product).is_empty());
    });}
}

#[test]
fn stalled_health_original_malformed_typed_response_cannot_become_unsupported() {
    for raw in [b"{\"id\":88103,\"error\":{\"code\":-32601,\"message\":\"wrong typed ID control\"}}\n".as_slice(),
        b"{\"id\":88102,\"result\":{\"Owner\":true}}\n".as_slice()] {health_control_product("codex",|product| {
        route_owner(product);let source=health_control_work_turn(product,false,r#""contextWindowExceeded""#);
        let (_,outcome)=repair_control(product,Some(&source),raw);
        assert!(outcome.is_err() || h::decode_receipt(outcome.as_ref().unwrap()).unwrap().status==V37Status::Unknown,
            "a malformed original reply grants no terminal unsupported authority: {outcome:?}");
        product.pump_host_rules().unwrap();assert!(notification_rows(product).is_empty());assert!(projection(product).is_empty());
    });}
}

#[test]
fn stalled_health_later_work_append_and_unacknowledged_write_suppress_original_cause() {
    for (verb,ack,unknown) in [("send",false,false),("send",false,true),
        ("append-without-turn",true,false),("append-without-turn",false,false)] {health_control_product("codex",|product| {
        route_owner(product);let source=health_control_work_turn(product,false,r#""contextWindowExceeded""#);
        let (_,outcome)=repair_control(product,Some(&source),METHOD_MISSING);assert_eq!(h::decode_receipt(&outcome.unwrap()).unwrap().status,V37Status::Unsupported);
        product.pump_host_rules().unwrap();assert_eq!(projection(product).len(),1);let original=notification_rows(product);
        prepare_control_input(product,verb,"later-input-control",ack,unknown);
        product.pump_host_rules().unwrap();assert!(projection(product).is_empty());
        assert_eq!(notification_rows(product),original,"a successor cannot mint another notification for the old cause");
        assert_eq!(scalar(product,"SELECT COUNT(*) FROM main.gogoke_v37_inbox_messages WHERE sender_seat_id='HOST_RULE' AND state='PENDING'"),"1");
    });}
}

#[test]
fn stalled_health_new_generation_suppresses_original_physical_cause() {
    health_control_product("codex",|product| {
        route_owner(product);let source=health_control_work_turn(product,false,r#""contextWindowExceeded""#);
        let (_,outcome)=repair_control(product,Some(&source),METHOD_MISSING);assert_eq!(h::decode_receipt(&outcome.unwrap()).unwrap().status,V37Status::Unsupported);
        product.pump_host_rules().unwrap();assert_eq!(projection(product).len(),1);let original=notification_rows(product);
        // Real existing metadata-only generation transition. It does not
        // send a model prompt or compact command; cleanup reads its new claim.
        let renew=operation("K-SESSION","renew-session","stalled-generation-control","sessionA",
            revision(product),r#"{"generation":"2"}"#);
        let result=h::decode_receipt(&product.dispatch_native_generation_change(&renew).unwrap()).unwrap();
        assert!(matches!(result.status,V37Status::Applied|V37Status::Replayed));
        assert_eq!(scalar(product,"SELECT generation FROM main.gogoke_v37_h_claim WHERE domain_id='projectA' AND session_id='sessionA'"),"3");
        product.pump_host_rules().unwrap();assert!(projection(product).is_empty());assert_eq!(notification_rows(product),original);
    });
}

#[test]
fn stalled_health_original_stop_intent_and_route_changes_suppress_without_new_identity() {
    for stop in [false,true] {health_control_product("codex",|product| {
        route_owner(product);let source=health_control_work_turn(product,false,r#""contextWindowExceeded""#);
        let (request,outcome)=repair_control(product,Some(&source),METHOD_MISSING);assert_eq!(h::decode_receipt(&outcome.unwrap()).unwrap().status,V37Status::Unsupported);
        product.pump_host_rules().unwrap();let original=notification_rows(product);assert_eq!(projection(product).len(),1);
        if stop {
            // Synthetic Owner stop-intent control on the original native H
            // journal, not an OS stop or a fabricated delivery/status receipt.
            product.connection.execute("BEGIN IMMEDIATE").unwrap();
            authority::check_owner_in_current_transaction(&product.connection,&product.owner).unwrap();
            change::begin_owner_stop(&product.connection,"projectA",&request.request_id,"stalled-stop-intent-control").unwrap();
            product.connection.execute("COMMIT").unwrap();
        } else {
            seat::create(&mut product.connection,NativeOrigin::user(&product.owner),CreateSeat {
                domain_id:"projectA",seat_id:"seatB",template_id:"templateA",instance_id:None,
                kind:Kind::Long,request_id:"stalled-route-seat",request_bytes:b"route replacement logical seat",
            }).unwrap();
            let revision=seat::current_policy_revision(&product.connection,"projectA").unwrap();
            let next=seat::configure_escalation_route(&mut product.connection,&product.owner,"projectA","seatA","STALL","seatB",revision).unwrap();
            product.pump_host_rules().unwrap();assert!(projection(product).is_empty());
            seat::configure_escalation_route(&mut product.connection,&product.owner,"projectA","seatA","STALL","OWNER",next).unwrap();
        }
        product.pump_host_rules().unwrap();assert!(projection(product).is_empty());
        assert_eq!(notification_rows(product),original);
        assert_eq!(scalar(product,"SELECT COUNT(*) FROM main.gogoke_v37_inbox_messages WHERE sender_seat_id='HOST_RULE' AND state='PENDING'"),"1");
    });}
}
