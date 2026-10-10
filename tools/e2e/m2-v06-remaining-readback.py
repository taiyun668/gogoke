"""Closed, immutable native E/H/A/F facts for the bounded V06 continuation."""
import hashlib
import json
import sqlite3
import sys
from pathlib import Path

def check(ok, why):
    if not ok:
        raise RuntimeError(why)

def one(db, sql, args=()):
    values=[dict(row) for row in db.execute(sql,args)]
    check(len(values)==1,f"One original row required, found {len(values)}")
    return values[0]

def rows(db,sql,args=()):
    return [dict(row) for row in db.execute(sql,args)]

def digest(data):
    return hashlib.sha256(data).hexdigest()

ORIGINAL_READER_SHA="1047fa57c14333719529669dbd912ca8fc3c5973b2d2164ae938c79dfed03560"
ORIGINAL_DRIVER_SHA="225827e0b796bb886920e3e65ae87cd0babc356c68180806e687fafc53dd8781"
ORIGINAL_HOST_SHA="5096fc2cc19b925b2f6ae8fddafac33c861ee8e0500c222e5ae92f9be3c9241c"
ORIGINAL_CASE="m2V06Remaining_21ef8d13f8634c1497b159080bc2238c"
ORIGINAL_SOURCE="b0b2f31edf409fdc627d4b1558a50c29f319a697"

def journal_entry(j,request_id,family,operation,target):
    found=[x for x in j["operations"] if x.get("request",{}).get("requestId")==request_id]
    check(len(found)==1,"Exact original request entry missing or duplicate")
    e=found[0]
    q,r=e["request"],e.get("receipt")
    check(q["family"]==family and q["operation"]==operation and q["targetId"]==target and
          e["rawFrame"]==json.dumps(q,ensure_ascii=False,separators=(",",":")) and
          r is not None and e.get("rawReceipt") and json.loads(e["rawReceipt"])==r and
          r["schema"]==q["schema"] and r["family"]==family and
          r["operation"]==operation and r["requestId"]==request_id and
          r["targetId"]==target,
          "Original User/H frame or receipt bytes differ")
    return e

def user_write(db,j,request_id,operation,target,expected="APPLIED"):
    e=journal_entry(j,request_id,"K-SEAT",operation,target)
    q,r=e["request"],e["receipt"]
    check(r["status"]==expected and q["domainId"]==j["fixture"]["domainId"],
          "Original User operation status/domain differs")
    durable=rows(db,"SELECT o.seat_id,o.layer,o.parent_seat_id,o.state,o.revision,"
                   "o.instance_id,s.settings_json FROM gogoke_v37_seat_operations o "
                   "LEFT JOIN gogoke_v37_seat_operation_snapshots s "
                   "USING(domain_id,request_id) WHERE o.domain_id=? AND o.request_id=?",
                 (q["domainId"],request_id))
    check((len(durable)==1)==(expected=="APPLIED"),
          "Original User durable operation count differs")
    if expected=="APPLIED":
        d=durable[0]
        check(d["seat_id"]==target and d["layer"]==r["result"]["layer"] and
              d["state"]==r["result"]["state"] and
              d["revision"]==int(r["revision"]) and
              (d["instance_id"] or None)==r["result"]["instanceId"] and
              json.loads(d["settings_json"])==r["result"]["settings"],
              "Original User durable snapshot differs from native receipt")
    return e

def state_card(db,j,receipt,target):
    e=journal_entry(j,receipt["requestId"],"K-SEAT","state-card",target)
    check(e["receipt"]==receipt and receipt["status"]=="APPLIED",
          "Original live E card was not the recorded direct read")
    row=one(db,"SELECT request_bytes,receipt_bytes FROM v37_ledger_receipt "
               "WHERE family='K-SEAT' AND domain_id=? AND request_id=?",
            (j["fixture"]["domainId"],receipt["requestId"]))
    check(bytes(row["request_bytes"]).decode()==e["rawFrame"] and
          bytes(row["receipt_bytes"]).decode()==e["rawReceipt"],
          "Original E card ledger bytes differ")

def thread_manifest(db,j,s,registered):
    f=j["fixture"]
    claim=one(db,"SELECT generation,process_operation_id,state FROM gogoke_v37_h_claim "
                 "WHERE domain_id=? AND session_id=?",(f["domainId"],s["id"]))
    custody=one(db,"SELECT generation,ticket,custodian_nonce,state FROM "
                   "gogoke_coordination_process_custody WHERE domain_id=? AND operation_id=?",
                (f["domainId"],s["processOperationId"]))
    found=rows(db,"SELECT step_id,phase,command_hex,generation,ticket,custodian_nonce,"
                  "source_epoch,source_cursor,open_request_id FROM gogoke_v37_rpc_steps "
                  "WHERE domain_id=? AND session_id=? AND process_operation_id=? "
                  "AND step_id='thread-start'",
               (f["domainId"],s["id"],s["processOperationId"]))
    check(len(found)==1,"Exact original H thread/start step missing or duplicated")
    row=found[0]
    command=json.loads(bytes.fromhex(row["command_hex"]))
    params=command.get("params",{})
    check(command.get("method")=="thread/start" and isinstance(params,dict) and
          row["phase"]=="OBSERVED" and row["open_request_id"]==s["openRequestId"] and
          row["source_epoch"] is not None and row["source_cursor"] is not None and
          str(row["generation"])==str(claim["generation"])==str(custody["generation"])==s["generation"] and
          row["ticket"]==custody["ticket"] and
          row["custodian_nonce"]==custody["custodian_nonce"] and
          claim["process_operation_id"]==s["processOperationId"] and
          claim["state"]=="RELEASED" and custody["state"]=="STOPPED",
          "Original H thread/start lacks physical generation/custody backing")
    backing=rows(db,"SELECT source_cursor FROM v37_ledger_raw_source WHERE domain_id=? "
                    "AND session_id=? AND operation_id=? AND generation=? AND source_epoch=? "
                    "AND source_cursor=? AND process_ticket=? AND custodian_nonce=?",
                 (f["domainId"],s["id"],s["processOperationId"],row["generation"],
                  row["source_epoch"],row["source_cursor"],row["ticket"],row["custodian_nonce"]))
    check(len(backing)==1,"Original H thread/start observation lacks exact raw source")
    if registered:
        tools=params.get("dynamicTools")
        check(isinstance(tools,list) and
              len([t for t in tools if isinstance(t,dict) and t.get("name")=="gogoke_seat"])==1,
              "Original USER H native seat tool was not advertised")
    else:
        check("dynamicTools" not in params,
              "Original LEAD H unexpectedly advertised dynamic tools")
    return {"sessionId":s["id"],"phase":row["phase"],
            "dynamicToolsRegistered":registered,
            "commandSha256":digest(bytes.fromhex(row["command_hex"]))}

def unresolved_model_turn(db,j,attempt):
    e=journal_entry(j,attempt["sendRequestId"],"K-SESSION","send",attempt["sessionId"])
    s=next((s for s in j["sessions"] if s["id"]==attempt["sessionId"]),None)
    check(s and e["receipt"]["status"]=="APPLIED" and
          e["receipt"]["result"]["turnId"]==attempt["turnId"] and
          e["request"]["payload"]["body"]==attempt["prompt"],
          "Original LEAD send/turn bytes differ")
    source=rows(db,"SELECT raw_bytes FROM v37_ledger_raw_source WHERE domain_id=? "
                   "AND session_id=? AND operation_id=? ORDER BY rowid",
                (j["fixture"]["domainId"],s["id"],s["processOperationId"]))
    calls=[];completed=[]
    for item in source:
        frame=json.loads(bytes(item["raw_bytes"]))
        params=frame.get("params",{})
        if frame.get("method")=="item/tool/call" and params.get("threadId")==s["threadId"] and params.get("turnId")==attempt["turnId"]:
            calls.append(frame)
        if frame.get("method")=="turn/completed" and params.get("threadId")==s["threadId"] and params.get("turn",{}).get("id")==attempt["turnId"] and params.get("turn",{}).get("status")=="completed":
            completed.append(frame)
    check(len(calls)==0 and len(completed)==1,
          "Original LEAD turn had a tool call or lacked completion")
    return {"state":"NOT_RUN_NO_REGISTERED_TOOL","originalCompletedTurnObserved":True,
            "originalToolCallCount":0}

def model_tool(db,j,attempt,expected):
    f=j["fixture"]
    e=journal_entry(j,attempt["sendRequestId"],"K-SESSION","send",attempt["sessionId"])
    q,r=e["request"],e["receipt"]
    check(q["domainId"]==f["domainId"] and q["payload"]["body"]==attempt["prompt"] and
          r["status"]=="APPLIED" and r["result"]["createdTurn"] is True and
          r["result"]["turnId"]==attempt["turnId"],
          "Original H send and turn ID differ")
    s=next((s for s in j["sessions"] if s["id"]==attempt["sessionId"]),None)
    check(s and s["seatId"]==attempt["parentSeatId"] and s.get("threadId") and
          s.get("stopFact") and s.get("releaseRequestId"),
          "Original model seat H session lacks exact closed lifecycle")
    binding=one(db,"SELECT seat_id,selected_instance_id FROM gogoke_v37_native_selection "
                   "WHERE domain_id=? AND session_id=?",(f["domainId"],s["id"]))
    check(binding["seat_id"]==s["seatId"] and
          binding["selected_instance_id"]==f["instanceId"],
          "Original H model session selected another seat or CLI instance")
    claim=one(db,"SELECT generation,process_operation_id,state FROM gogoke_v37_h_claim "
                 "WHERE domain_id=? AND session_id=?",(f["domainId"],s["id"]))
    check(str(claim["generation"])==s["generation"] and claim["state"]=="RELEASED" and
          claim["process_operation_id"]==s["processOperationId"],
          "Original H claim differs from live process episode")
    stdin=one(db,"SELECT phase,receipt_status,request_hex,receipt_hex,process_operation_id "
                 "FROM gogoke_v37_h_stdin_journal WHERE domain_id=? AND request_id=?",
              (f["domainId"],attempt["sendRequestId"]))
    check(stdin["phase"]=="RECEIPTED" and stdin["receipt_status"]=="APPLIED" and
          stdin["process_operation_id"]==s["processOperationId"] and
          bytes.fromhex(stdin["request_hex"]).decode()==e["rawFrame"] and
          json.loads(bytes.fromhex(stdin["receipt_hex"]))==r,
          "Original H stdin journal/request/receipt mismatch")
    source=rows(db,"SELECT raw_bytes FROM v37_ledger_raw_source WHERE domain_id=? AND session_id=? "
                   "AND operation_id=? ORDER BY rowid",
                (f["domainId"],s["id"],s["processOperationId"]))
    calls=[];completed=[];all_turn_calls=[]
    for item in source:
        frame=json.loads(bytes(item["raw_bytes"]))
        params=frame.get("params",{})
        if (frame.get("method")=="item/tool/call" and
            params.get("threadId")==s["threadId"] and
            params.get("turnId")==attempt["turnId"]):
            all_turn_calls.append(frame)
        if (frame.get("method")=="item/tool/call" and params.get("tool")=="gogoke_seat" and
            params.get("threadId")==s["threadId"] and
            params.get("turnId")==attempt["turnId"] and
            params.get("arguments")==attempt["expectedArguments"]):
            calls.append(frame)
        if (frame.get("method")=="turn/completed" and
            params.get("threadId")==s["threadId"] and
            params.get("turn",{}).get("id")==attempt["turnId"] and
            params.get("turn",{}).get("status")=="completed"):
            completed.append(frame)
    check(len(calls)==len(completed)==len(all_turn_calls)==1,
          "Original A tool call/turn completion absent or duplicated")
    rpc=[]
    for row in rows(db,"SELECT command_hex,phase,step_id FROM gogoke_v37_rpc_steps "
                       "WHERE domain_id=? AND session_id=? AND process_operation_id=?",
                    (f["domainId"],s["id"],s["processOperationId"])):
        command=json.loads(bytes.fromhex(row["command_hex"]))
        if "method" not in command and command.get("id")==calls[0]["id"]:
            rpc.append((row,command))
    check(len(rpc)==1 and rpc[0][0]["phase"] in ("WRITTEN","OBSERVED"),
          "Original H-written RPC response absent")
    row,command=rpc[0]
    content=command.get("result",{}).get("contentItems",[])
    check(len(content)==1 and content[0].get("type")=="inputText",
          "Original H tool response shape differs")
    native=json.loads(content[0]["text"])
    check(native["schema"]=="gogoke.37.operations.v1" and native["family"]=="K-SEAT" and
          native["operation"]==attempt["expectedArguments"]["operation"] and
          native["requestId"]==row["step_id"] and
          native["targetId"]==attempt["targetId"] and
          native["status"]==expected and
          command["result"]["success"] is (expected=="APPLIED"),
          "Original H-written native K-SEAT status differs")
    durable=rows(db,"SELECT seat_id,layer,parent_seat_id FROM gogoke_v37_seat_operations "
                    "WHERE domain_id=? AND request_id=?",
                 (f["domainId"],row["step_id"]))
    check((len(durable)==1)==(expected=="APPLIED"),
          "Original native E write count differs")
    if expected=="APPLIED":
        check(durable[0]["seat_id"]==attempt["targetId"] and
              durable[0]["layer"]=="LEAD" and
              durable[0]["parent_seat_id"]==f["parentSeatId"],
              "Original native child belongs to another parent")
    return {"phase":attempt["phase"],"sendRequestId":attempt["sendRequestId"],
            "rpcStepId":row["step_id"],"status":native["status"]}

def h_stop_release(db,j,s):
    f=j["fixture"]
    check(s.get("stopRequestId") and s.get("releaseRequestId") and
          isinstance(s.get("stopFact"),str) and s["stopFact"].startswith("sha256:"),
          "Original H session lacks StopFact or release IDs")
    stop=journal_entry(j,s["stopRequestId"],"K-SESSION","stop",s["id"])
    release=journal_entry(j,s["releaseRequestId"],"K-SESSION","admission-release",s["id"])
    check(stop["receipt"]["status"]==release["receipt"]["status"]=="APPLIED" and
          stop["receipt"]["result"]["stopFact"]==s["stopFact"] and
          release["request"]["expectedRevision"]==stop["receipt"]["revision"],
          "Original same-session H StopFact/release receipts differ")
    operations=[one(db,"SELECT operation,status,revision FROM gogoke_v37_h_operation "
                       "WHERE domain_id=? AND request_id=?",
                    (f["domainId"],request_id))
                for request_id in (s["stopRequestId"],s["releaseRequestId"])]
    check([r["operation"] for r in operations]==["stop","admission-release"] and
          all(r["status"]=="APPLIED" for r in operations),
          "Original H durable stop/release operations differ")
    claim=one(db,"SELECT state,process_operation_id,stop_fact_id FROM gogoke_v37_h_claim "
                 "WHERE domain_id=? AND session_id=?",(f["domainId"],s["id"]))
    episode=one(db,"SELECT phase,stop_fact_id FROM gogoke_v37_h_process_episode "
                   "WHERE domain_id=? AND session_id=? AND process_operation_id=?",
                (f["domainId"],s["id"],claim["process_operation_id"]))
    custody=one(db,"SELECT state,stop_proof_hash FROM gogoke_coordination_process_custody "
                   "WHERE operation_id=?",(claim["process_operation_id"],))
    check(claim["state"]=="RELEASED" and
          episode["phase"]==custody["state"]=="STOPPED" and
          s["stopFact"]==stop["receipt"]["result"]["stopFact"]==
            claim["stop_fact_id"]==episode["stop_fact_id"]==
            custody["stop_proof_hash"] and
          s["processOperationId"]==claim["process_operation_id"],
          "Original physical H episode is not stopped and released")
    return {"sessionId":s["id"],"stopRequestId":s["stopRequestId"],
            "releaseRequestId":s["releaseRequestId"],"stopFact":s["stopFact"]}

def main():
    supplementary=(len(sys.argv)==7 and sys.argv[4]=="final" and
                   sys.argv[5]=="--supplementary-original-reader")
    check((len(sys.argv)==5 and sys.argv[4] in ("baseline","final")) or supplementary,
          "Expected STATE_ROOT OUTPUT JOURNAL baseline|final [--supplementary-original-reader D_FILE]")
    root=Path(sys.argv[1]).resolve(strict=True)
    output=Path(sys.argv[2]).resolve(strict=False)
    journal_path=Path(sys.argv[3]).resolve(strict=True)
    phase=sys.argv[4]
    old_reader=Path(sys.argv[6]).resolve(strict=True) if supplementary else None
    check(not output.exists() and not output.is_relative_to(root) and
          output.parent==journal_path.parent and
          output.drive.upper()=="D:" and journal_path.drive.upper()=="D:",
          "Fresh private D reader output required")
    journal_bytes=journal_path.read_bytes()
    old_reader_bytes=old_reader.read_bytes() if old_reader else None
    j=json.loads(journal_bytes.decode("utf-8-sig"))
    f=j["fixture"]
    current_reader_sha=digest(Path(__file__).read_bytes())
    check(j["schema"]=="gogoke.37.m2-v06-remaining-installed.v1" and
          j["acceptance"] is False,"Original V06 journal identity differs")
    original_reader_sha=j["driverBytes"]["m2-v06-remaining-readback.py"]
    if supplementary:
        check(old_reader.drive.upper()=="D:" and old_reader!=Path(__file__).resolve() and
              digest(old_reader_bytes)==original_reader_sha==ORIGINAL_READER_SHA and
              j["driverBytes"]["m2-v06-remaining-win11.mjs"]==ORIGINAL_DRIVER_SHA and
              j["driverBytes"]["m2-v06-remaining-host.mjs"]==ORIGINAL_HOST_SHA and
              j["sourceCommit"]==ORIGINAL_SOURCE and j["caseId"]==ORIGINAL_CASE and
              j["state"]=="FAIL_OR_NOT_RUN_PRESERVE_ORIGINAL" and
              isinstance(j.get("originalError"),str) and j["originalError"] and
              len(j["readbacks"])==1 and j["readbacks"][0]["phase"]=="baseline",
              "Supplementary mode requires exact retained b0 original FAIL and reader bytes")
    else:
        check(current_reader_sha==original_reader_sha,
              "Current V06 reader/journal source differs")
    database=root/"state.sqlite"
    wal=Path(str(database)+"-wal")
    check(database.is_file() and (not wal.exists() or wal.stat().st_size==0),
          "Normal closed state.sqlite and absent/zero WAL required")
    before=digest(database.read_bytes())
    with sqlite3.connect(database.as_uri()+"?mode=ro&immutable=1",uri=True) as db:
        db.row_factory=sqlite3.Row
        db.execute("PRAGMA query_only=ON")
        source=one(db,"SELECT repository_id,remote_kind,baseline_commit FROM gogoke_v37_worktree_sources "
                      "WHERE repository_id=?",(j["repositoryId"],))
        template=one(db,"SELECT settings_json,revision FROM gogoke_v37_seat_templates "
                        "WHERE domain_id=? AND template_id=?",
                     (f["domainId"],f["templateId"]))
        settings=json.loads(template["settings_json"])
        template_match=(settings.get("model")==f["model"] and
                        settings.get("effort",settings.get("reasoningEffort"))==f["effort"] and
                        settings.get("permissionTier")==f["permissionTier"] and
                        settings.get("orchestrationScope",{}).get("instanceIds")==[f["instanceId"]])
        cap=one(db,"SELECT parallel_cap FROM gogoke_v37_seat_project_caps WHERE domain_id=?",
                (f["domainId"],))["parallel_cap"]
        prior=f.get("priorV06")
        prior_proof=None
        if prior:
            prior_path=Path(prior["path"]).resolve(strict=True)
            check(digest(prior_path.read_bytes())==prior["sha256"],
                  "Historical candidate49 direct V06 proof bytes changed")
            prior_proof=json.loads(prior_path.read_text(encoding="utf-8-sig"))
            check(prior_proof["schema"]=="gogoke.37.private-m2-v06-readback.v1" and
                  prior_proof["phase"]=="final" and
                  prior_proof["originalModelReceipts"] is True and
                  prior_proof["deniedTargetAbsent"] is True and
                  prior_proof["childCount"]==1 and
                  prior_proof["acceptance"] is False and
                  prior_proof["measurementPreservedDatabaseBytes"] is True,
                  "Historical candidate49 child-cap proof does not carry exact direct facts")
        pin=one(db,"SELECT driver_id,version,program_digest,login_state FROM gogoke_v37_instances "
                   "WHERE instance_id=?",(f["instanceId"],))
        check(source["remote_kind"]=="HTTPS" and template_match and cap>0 and
              pin["driver_id"]=="codex" and pin["version"]=="0.160.0" and
              pin["program_digest"]=="sha256:"+j["cliSha256"] and
              pin["login_state"]=="LOGGED_IN",
              "Original source/template/project/CLI differs")
        parent=rows(db,"SELECT * FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",
                    (f["domainId"],f["parentSeatId"]))
        child=rows(db,"SELECT * FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",
                   (f["domainId"],f["childSeatId"]))
        trees=[rows(db,"SELECT * FROM gogoke_v37_worktrees WHERE worktree_id=?",(tree,))
               for tree in (f["parentTreeId"],f["childTreeId"])]
        if phase=="baseline":
            check(not parent and not child and not any(trees) and not j["launches"] and
                  not j["operations"],"Original V06 IDs already occupied at baseline")
            facts={"parentAbsent":True,"childAbsent":True,
                   "sourceTemplateMatches":template_match,"projectCap":cap,
                   "historicalChildCap":prior_proof and
                     {"sourceCommit":prior_proof["sourceCommit"],"sha256":prior["sha256"],
                      "newCandidateCoverage":False},
                   "directModelToolEvidence":False,"directBusyStopReleaseEvidence":False}
        else:
            check(len(j["launches"])==len(j["closes"])==1 and
                  j["closes"][0]["pid"]==j["launches"][0]["pid"] and
                  j["closes"][0]["exitCode"]==0 and
                  j["closes"][0]["forceKill"] is False,
                  "Original installed candidate lacks normal-close receipt")
            check(len(parent)==len(child)==1 and
                  parent[0]["layer"]=="USER" and parent[0]["state"]=="IDLE" and
                  child[0]["layer"]=="LEAD" and
                  child[0]["parent_seat_id"]==f["parentSeatId"] and
                  child[0]["state"]=="RECLAIMED" and all(len(t)==1 for t in trees) and
                  all(t[0]["state"]=="REGISTERED" for t in trees),
                  "Original disjoint E/F final identities differ")
            copied=one(db,"SELECT settings_json FROM gogoke_v37_seat_settings "
                          "WHERE domain_id=? AND seat_id=?",
                       (f["domainId"],f["parentSeatId"]))
            scope=json.loads(copied["settings_json"])["orchestrationScope"]
            check(scope["maxConcurrent"]==1 and scope["instanceIds"]==[f["instanceId"]],
                  "Original USER direct-child scope differs")
            for key,operation,target in (
                ("parentCreateRequestId","create-from-template",f["parentSeatId"]),
                ("boundsRequestId","set-orchestration-bounds",f["parentSeatId"]),
                ("bindRequestId","bind-instance",f["parentSeatId"]),
                ("reclaimRequestId","reclaim",f["childSeatId"])):
                user_write(db,j,j[key],operation,target)
            for key,operation in (("busyChangeRequestId","change-instance"),
                                  ("busyReclaimRequestId","reclaim")):
                if j.get(key):
                    refused=user_write(db,j,j[key],operation,f["childSeatId"],"CONFLICT")
                    check(refused["request"]["expectedRevision"]==j["busyCard"]["revision"] and
                          refused["receipt"]["previousRevision"]==j["busyCard"]["revision"] and
                          refused["receipt"]["revision"]==j["busyCard"]["revision"],
                          "Original BUSY User refusal changed the E revision")
            card_names=[("childCard","IDLE"),("busyCard","BUSY"),("idleCard","IDLE")]
            if not supplementary:
                card_names.append(("afterBusyRefusalsCard","BUSY"))
            else:
                card_names.append(("afterDeniedCard","BUSY"))
            for name,state in card_names:
                card=j[name]
                state_card(db,j,card,f["childSeatId"])
                check(card["result"]["state"]==state,
                      "Original transient LEAD card state differs")
            after_busy=j["afterDeniedCard"] if supplementary else j["afterBusyRefusalsCard"]
            check(j["busyCard"]["revision"]==after_busy["revision"] and
                  j["busyCard"]["result"]["generation"]==
                    after_busy["result"]["generation"],
                  "Original BUSY card revision/generation changed")
            if not supplementary:
                busy_index=next(i for i,e in enumerate(j["operations"]) if
                                e.get("request",{}).get("requestId")==j["busyCard"]["requestId"])
                after_index=next(i for i,e in enumerate(j["operations"]) if
                                 e.get("request",{}).get("requestId")==after_busy["requestId"])
                refusal_indices=[next(i for i,e in enumerate(j["operations"]) if
                                      e.get("request",{}).get("requestId")==j[key])
                                 for key in ("busyChangeRequestId","busyReclaimRequestId") if j.get(key)]
                check(busy_index<min(refusal_indices) and max(refusal_indices)<after_index,
                      "BUSY unchanged card was not read after actual User refusals")
            attempts=j["modelAttempts"]
            phases=[a["phase"] for a in attempts]
            check(phases==(["child-create","lead-bounds-denial"] if supplementary else
                           ["child-create"]),"Original H model attempt count differs")
            native=[model_tool(db,j,attempts[0],"APPLIED")]
            check(len(j["sessions"])==2 and
                  [s["seatId"] for s in j["sessions"]]==
                    [f["parentSeatId"],f["childSeatId"]],
                  "Original H parent/child session identities differ")
            parent_session,child_session=j["sessions"]
            manifests=[thread_manifest(db,j,parent_session,True),
                       thread_manifest(db,j,child_session,False)]
            if supplementary:
                denial=unresolved_model_turn(db,j,attempts[1])
            else:
                missing=[n for n in j["notRun"] if n.get("axis")=="MODEL_LEAD_BOUNDS_DENIAL"]
                check(len(missing)==1 and missing[0]["state"]=="NOT_RUN_NO_REGISTERED_TOOL",
                      "Original LEAD capability absence was not recorded as NOT_RUN")
                denial={"state":"NOT_RUN_NO_REGISTERED_TOOL",
                        "originalCompletedTurnObserved":False,"originalToolCallCount":0}
            child_admission=[]
            for operation in ("admission-reserve","admission-commit","open"):
                found=[e for e in j["operations"] if
                       e.get("request",{}).get("family")=="K-SESSION" and
                       e["request"].get("operation")==operation and
                       e["request"].get("targetId")==child_session["id"]]
                check(len(found)==1,"Original child H admission/open count differs")
                e=journal_entry(j,found[0]["request"]["requestId"],
                                "K-SESSION",operation,child_session["id"])
                check(e["receipt"]["status"]=="APPLIED",
                      "Original child H admission/open was not applied")
                child_admission.append(e)
            parent_stop=journal_entry(j,parent_session["stopRequestId"],
                                      "K-SESSION","stop",parent_session["id"])
            parent_release=journal_entry(j,parent_session["releaseRequestId"],
                                         "K-SESSION","admission-release",parent_session["id"])
            order=[j["operations"].index(e) for e in
                   [parent_stop,parent_release,*child_admission]]
            check(child_session["openRequestId"]==
                    child_admission[2]["request"]["requestId"] and
                  parent_stop["receipt"]["status"]==
                    parent_release["receipt"]["status"]=="APPLIED" and
                  all(a<b for a,b in zip(order,order[1:])),
                  "Original parent stop/release did not precede child admission/open")
            h=[h_stop_release(db,j,s) for s in j["sessions"]]
            child_stop=journal_entry(j,child_session["stopRequestId"],"K-SESSION","stop",child_session["id"])
            child_release=journal_entry(j,child_session["releaseRequestId"],"K-SESSION","admission-release",child_session["id"])
            reclaim=journal_entry(j,j["reclaimRequestId"],"K-SEAT","reclaim",f["childSeatId"])
            busy_refusals=[journal_entry(j,j[key],"K-SEAT",operation,f["childSeatId"])
                           for key,operation in (("busyChangeRequestId","change-instance"),
                                                 ("busyReclaimRequestId","reclaim")) if j.get(key)]
            busy_card_entry=journal_entry(j,j["busyCard"]["requestId"],"K-SEAT","state-card",f["childSeatId"])
            idle_card_entry=journal_entry(j,j["idleCard"]["requestId"],"K-SEAT","state-card",f["childSeatId"])
            check(all(j["operations"].index(busy_card_entry)<j["operations"].index(e)<
                      j["operations"].index(child_stop) for e in busy_refusals) and
                  j["operations"].index(child_stop)<j["operations"].index(child_release)<
                  j["operations"].index(idle_card_entry)<j["operations"].index(reclaim) and
                  int(j["idleCard"]["revision"])==int(j["busyCard"]["revision"])+1 and
                  reclaim["request"]["expectedRevision"]==j["idleCard"]["revision"] and
                  int(reclaim["receipt"]["revision"])==int(j["idleCard"]["revision"])+1,
                  "Original BUSY refusals, H stop, IDLE card, reclaim order/revisions differ")
            sends=[x for x in j["operations"] if x.get("request",{}).get("family")=="K-SESSION" and
                   x["request"]["operation"]=="send"]
            turns=[t for s in j["sessions"] for t in s["turns"]]
            expected_phases=["takeover","child-create"]+(["lead-bounds-denial"] if supplementary else [])
            check(len(h)==2 and len(sends)==len(turns)==len(expected_phases) and
                  [t["phase"] for t in turns]==expected_phases and
                  [t["sendRequestId"] for t in turns]==
                    [e["request"]["requestId"] for e in sends] and
                  len(j["takeoverCards"])==1,
                  "Original H send count/turn sequence differs from native ready gate")
            qcard=j["takeoverCards"][0]
            check(qcard["sessionId"]==parent_session["id"] and
                  qcard["turnId"]==parent_session["turns"][0]["turnId"] and
                  qcard["option"] in (f["takeover"]["option"],f["takeover"]["option"]+" (Recommended)"),
                  "Original C card was not the USER parent takeover answer")
            answer=journal_entry(j,qcard["requestId"],"K-QCARD","answer",qcard["cardId"])
            qnative=one(db,"SELECT state,seat_id,turn_id,generation,answer_kind,answer "
                           "FROM gogoke_v37_qcard_native WHERE domain_id=? AND card_id=?",
                        (f["domainId"],qcard["cardId"]))
            qoperation=one(db,"SELECT state,seat_id,turn_id,generation,answer_kind,answer,"
                              "native_receipt_id FROM gogoke_v37_qcard_native_operations "
                              "WHERE domain_id=? AND request_id=?",
                           (f["domainId"],qcard["requestId"]))
            check(answer["receipt"]["status"]=="APPLIED" and
                  answer["receipt"]["result"]["state"]=="ANSWERED" and
                  all(q["state"]=="ANSWERED" and q["seat_id"]==f["parentSeatId"] and
                      q["turn_id"]==qcard["turnId"] and
                      q["generation"]==parent_session["generation"] and
                      q["answer_kind"]=="WIRE" and isinstance(q["answer"],str) and q["answer"]
                      for q in (qnative,qoperation)) and
                  qnative["answer"]==qoperation["answer"] and qoperation["native_receipt_id"],
                  "Original C answer lacks exact native receipt backing")
            check(answer["receipt"]["result"].get("nativeReceiptId")==
                  qoperation["native_receipt_id"] and
                  answer["receipt"]["result"].get("deliveryBasis")=="NATIVE_EXACT_WRITE_RECEIPT",
                  "Original C answer receipt is not the native write receipt")
            parent_cards=[e for e in j["operations"] if e.get("request",{}).get("family")=="K-SEAT" and
                          e["request"].get("operation")=="state-card" and
                          e["request"].get("targetId")==f["parentSeatId"] and
                          e.get("receipt",{}).get("status")=="APPLIED"]
            for e in parent_cards:
                state_card(db,j,e["receipt"],f["parentSeatId"])
            check(any(e["receipt"]["result"].get("takeoverReady") is False and
                      j["operations"].index(e)<j["operations"].index(sends[0]) for e in parent_cards) and
                  any(e["receipt"]["result"].get("takeoverReady") is True and
                      any(a.get("questionId")==f["takeover"]["questionId"] and
                          a.get("answer")==qcard["option"] and a.get("basis")=="CITED" and
                          a.get("sourceRef","").startswith(
                              f'C-QCARD:{qcard["cardId"]}:{qcard["requestId"]}:')
                          for a in e["receipt"]["result"].get("takeoverAnswers",[])) and
                      j["operations"].index(sends[0])<j["operations"].index(e)<
                      j["operations"].index(sends[1]) for e in parent_cards),
                  "Original USER C takeover state-card gate differs")
            child_ready=journal_entry(j,j["childCard"]["requestId"],"K-SEAT", "state-card",f["childSeatId"])
            child_open=journal_entry(j,child_session["openRequestId"],"K-SESSION","open",child_session["id"])
            check(j["childCard"]["result"].get("takeoverReady") is True and
                  j["busyCard"]["result"].get("takeoverReady") is True and
                  j["operations"].index(sends[1])<j["operations"].index(child_ready)<
                  j["operations"].index(child_open)<
                  j["operations"].index(journal_entry(j,j["busyCard"]["requestId"],"K-SEAT","state-card",f["childSeatId"])),
                  "Original LEAD ready gate/card sequence differs")
            facts={"parentAbsent":False,"childAbsent":False,
                   "sourceTemplateMatches":template_match,"projectCap":cap,
                   "historicalChildCap":prior_proof and
                     {"sourceCommit":prior_proof["sourceCommit"],"sha256":prior["sha256"],
                      "newCandidateCoverage":False},
                   "directModelToolEvidence":False,
                   "directChildCreateEvidence":True,
                   "modelLeadBoundsDenial":denial,
                   "directBusyStopReleaseEvidence":True,
                   "nativeModelReceipts":native,"hStopRelease":h,
                   "threadStartManifests":manifests,
                   "nativeTakeoverCardCount":1}
    after=digest(database.read_bytes())
    check(before==after and (not wal.exists() or wal.stat().st_size==0),
          "Immutable V06 reader changed database bytes or WAL")
    check(digest(journal_path.read_bytes())==digest(journal_bytes) and
          (not supplementary or digest(old_reader.read_bytes())==digest(old_reader_bytes)),
          "Original journal or preserved reader bytes changed during supplementary read")
    root_id={"device":str(root.stat().st_dev),"inode":str(root.stat().st_ino),
             "databaseDevice":str(database.stat().st_dev),
             "databaseInode":str(database.stat().st_ino)}
    candidate={"sourceCommit":j["sourceCommit"],
               "version":j["launches"][0]["bootstrap"]["version"] if phase=="final" else j["installedVersion"],
               "setId":j["launches"][0]["setId"] if phase=="final" else j["preflightSetId"],
               "generationId":j["launches"][0]["generationId"] if phase=="final" else j["preflightGenerationId"]}
    proof={"schema":"gogoke.37.private-m2-v06-remaining-readback.v1",
           "phase":phase,"caseId":j["caseId"],"sourceCommit":j["sourceCommit"],
           "readerSha256":current_reader_sha,
           "originalReaderSha256":original_reader_sha,
           "currentReaderSha256":current_reader_sha,
           "supplementary":supplementary,
           "originalFailureRetained":supplementary,
           "state":("SUPPLEMENTARY_PARTIAL_V06_ORIGINAL_FAIL_RETAINED" if supplementary
                    else "PARTIAL_DIRECT_V06_REVIEW_REQUIRED" if phase=="final" else "BASELINE"),
           "acceptance":False,"databaseWrites":False,"credentialReads":False,
           "measurementPreservedDatabaseBytes":True,"databaseSha256":before,
           "rootIdentity":root_id,"candidateIdentity":candidate,**facts}
    output.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    print(json.dumps({"phase":phase,"acceptance":False,
                      "directModelToolEvidence":proof["directModelToolEvidence"]}))

if __name__=="__main__":
    main()
