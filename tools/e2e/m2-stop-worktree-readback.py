"""Immutable H/F stop-gate readback after normal close.

Signed Python: STATE_ROOT OUTPUT JOURNAL stopped|final. No DB writes,
credentials, model output, Git invocation, or product launch occur here.
"""
import hashlib
import json
import sqlite3
import sys
from pathlib import Path

import installed_native_readback as native


def check(value, message):
    if not value:
        raise RuntimeError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def local_spelling(value):
    text = str(value).replace("/", "\\")
    return (text[4:] if text.startswith("\\\\?\\") else text).casefold()


def rows(db, sql, args=()):
    return [dict(row) for row in db.execute(sql, args)]


def one(db, sql, args=()):
    found = rows(db, sql, args)
    check(len(found) == 1, f"One original row required, found {len(found)}")
    return found[0]


def operation(journal, request_id):
    found = [entry for entry in journal["operations"]
             if entry.get("request", {}).get("requestId") == request_id]
    check(len(found) == 1, "One original User operation required")
    entry = found[0]
    check(json.loads(entry["rawFrame"]) == entry["request"] and
          entry["receipt"] and entry["receipt"]["requestId"] == request_id and
          entry["receipt"]["operation"] == entry["request"]["operation"] and
          entry["receipt"]["targetId"] == entry["request"]["targetId"],
          "Original User request/receipt wire identity differs")
    return entry


def child_terminal_output_receipt(journal, session, child):
    entry = operation(journal, child.get("terminalOutputRequestId"))
    request, page = entry["request"], entry["receipt"].get("result", {})
    matches = [item for item in page.get("nativeInputReceipts", [])
               if item.get("requestId") == child.get("sendRequestId")]
    check(request.get("family") == "K-SESSION" and request.get("operation") == "output-stream" and
          request.get("targetId") == session["id"] and
          request.get("payload", {}).get("generation") == session["generation"] and
          isinstance(request.get("payload", {}).get("afterCursor"), str) and
          page.get("generation") == session["generation"] and
          isinstance(page.get("rawHighwater"), str) and page["rawHighwater"].isdecimal() and
          int(page["rawHighwater"]) >= int(child["terminalCursor"]) and len(matches) == 1 and
          matches[0].get("phase") == "RECEIPTED" and
          matches[0].get("receipt") == child.get("terminalReceipt"),
          "Original H output-stream does not carry the same send ID's exact typed receipt")
    return {"outputRequestId": child["terminalOutputRequestId"],
            "sendRequestId": child["sendRequestId"], "rawHighwater": page["rawHighwater"],
            "receipt": matches[0]["receipt"]}


def session_facts(db, journal, case):
    session = case["session"]
    domain, sid = case["domainId"], session["id"]
    registration = one(db, "SELECT domain_id,seat_id,purpose,COALESCE(side_id,'') AS side_id "
                       "FROM v37_ledger_session WHERE domain_id=? AND session_id=?", (domain, sid))
    check(registration["seat_id"] == case["seatId"] and registration["purpose"] == "WORK" and
          registration["side_id"] == "", "Original A session registration differs")
    seat = one(db, "SELECT incarnation FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",
               (domain, case["seatId"]))
    claim = one(db, "SELECT process_operation_id FROM gogoke_v37_h_claim "
                "WHERE domain_id=? AND session_id=?", (domain, sid))
    request_ids = {"admission-reserve": session["reserveRequestId"],
                   "admission-commit": session["commitRequestId"],
                   "open": session["openRequestId"], "stop": session["stopRequestId"],
                   "admission-release": session["releaseRequestId"]}
    facts = native.verify_original_h_stop(
        db, journal=journal, domain=domain, session_id=sid,
        generation=session["generation"], request_ids=request_ids,
        tree_id=case["worktreeId"], repository_id=case["repositoryId"],
        seat_id=case["seatId"], seat_incarnation=seat["incarnation"],
        instance_id=case["instanceId"], process_operation_id=claim["process_operation_id"],
        stop_fact_id=session["stopFactId"], binding_mode="native_v2")
    custody = one(db, "SELECT operation_id,domain_id,generation,state,stop_proof_hash,ticket,custodian_nonce "
                  "FROM gogoke_coordination_process_custody WHERE domain_id=? AND operation_id=?",
                  (domain, facts["processOperationId"]))
    check(custody["operation_id"] == facts["processOperationId"] == claim["process_operation_id"] and
          custody["domain_id"] == domain and custody["generation"] == facts["generation"] == session["generation"] and
          custody["state"] == facts["custodyState"] == "STOPPED" and
          custody["stop_proof_hash"] == facts["stopFactId"] == session["stopFactId"] and
          isinstance(custody["ticket"], str) and custody["ticket"] and
          isinstance(custody["custodian_nonce"], str) and custody["custodian_nonce"],
          "Original H process custody ticket/nonce differs from the verified StopFact tuple")
    return {**facts, "seatId": case["seatId"], "instanceId": case["instanceId"],
            "nativeBinding": facts["bindingEvidence"]["binding"],
            "nativeSelection": facts["bindingEvidence"]["selection"],
            "processTicket": custody["ticket"],
            "custodianNonce": custody["custodian_nonce"]}


def child_identity_facts(child, marker):
    identity = child.get("processIdentity") if isinstance(child, dict) else None
    pid, nonce = child.get("childPid"), child.get("childNonce")
    check(isinstance(identity, dict) and isinstance(pid, str) and pid.isdecimal() and
          isinstance(nonce, str) and nonce and isinstance(marker, dict) and
          set(marker) == {"nonce", "pid", "ppid"} and marker.get("nonce") == nonce and
          str(marker.get("pid")) == pid and str(marker.get("ppid", "")).isdecimal() and
          identity.get("processId") == pid and identity.get("imageSha256") == child.get("signedNodeSha256") and
          isinstance(identity.get("creationTime100ns"), str) and identity["creationTime100ns"].isdecimal() and
          identity.get("waitForSingleObjectResult") == 0x102 and
          identity.get("aliveAtObservation") is True and isinstance(identity.get("observationTimeUtc"), str) and
          identity.get("observationTimeUtc"),
          "Original same-handle process identity, WAIT_TIMEOUT, and ready marker do not bind")
    return {"processId": pid, "creationTime100ns": identity["creationTime100ns"],
            "imageSha256": identity["imageSha256"], "waitForSingleObjectResult": 0x102,
            "aliveAtObservation": True, "observationTimeUtc": identity["observationTimeUtc"],
            "readyMarkerSha256": child["readyMarkerSha256"], "readyMarker": marker}


def child_tool_source_facts(source_rows, init_frame, child, init_raw_bytes=None):
    before, terminal = int(child["beforeRawHighwater"]), int(child["terminalCursor"])
    epoch, session_id = child["sourceEpoch"], child["vendorSessionId"]
    check(init_frame.get("type") == "system" and init_frame.get("subtype") == "init" and
          init_frame.get("session_id") == session_id and
          init_frame.get("claude_code_version") == child["cliVersion"] == "2.1.196" and
          local_spelling(init_frame.get("cwd", "")) == local_spelling(child["worktreePath"]),
          "Original Claude init does not bind the fixed CLI and exact F cwd")
    decoded = []
    cursors = []
    for row in source_rows:
        cursor = row["source_cursor"]
        check(row["source_epoch"] == epoch and str(cursor).isdecimal() and
              before < int(cursor) <= terminal and row["operation_id"] == child["processOperationId"] and
              row["generation"] == child["generation"] and row["process_ticket"] == child["ticket"] and
              row["custodian_nonce"] == child["custodianNonce"],
              "Original child A frame escaped its H process/source interval")
        cursors.append(int(cursor))
        raw = bytes(row["raw_bytes"])
        decoded.append((row, json.loads(raw.decode("utf-8")), raw))
    check(cursors == list(range(before + 1, terminal + 1)),
          "Original child H/A source interval has a cursor gap or duplicate")
    echoes = [(row, frame, raw) for row, frame, raw in decoded if frame.get("type") == "user" and
              frame.get("session_id") == session_id and frame.get("uuid") == child["userUuid"] and
              frame.get("message") == {"role": "user", "content": [{"type": "text", "text": child["body"]}]}]
    terminal_rows = [(row, frame, raw) for row, frame, raw in decoded
                     if str(row["source_cursor"]) == str(child["terminalCursor"])]
    check(len(echoes) == 1 and echoes[0][0]["state"] == "NO_EVENT" and
          echoes[0][0]["no_event_reason"] == "CLAUDE_STDIN_ACK" and
          len(terminal_rows) == 1,
          "Original Claude user replay or typed terminal row is missing or ambiguous")
    terminal_row, terminal_frame, terminal_raw = terminal_rows[0]
    check(terminal_frame.get("type") == "result" and terminal_frame.get("subtype") == "success" and
          terminal_frame.get("is_error") is False and terminal_frame.get("session_id") == session_id and
          terminal_row["state"] == "NO_EVENT" and
          terminal_row["no_event_reason"] == "CLAUDE_RESULT_RESPONSE" and
          digest(terminal_raw) == child["rawResultSha256"],
          "Original Claude Result is not the typed H-receipted terminal source")
    uses, tool_results, assistant_texts = [], [], []
    for row, frame, raw in decoded:
        if frame.get("type") == "assistant":
            message = frame.get("message")
            content = message.get("content") if isinstance(message, dict) else None
            if isinstance(content, list):
                uses.extend((row, frame, block, raw) for block in content
                            if isinstance(block, dict) and block.get("type") == "tool_use")
                if frame.get("session_id") == session_id:
                    assistant_texts.extend((row, block) for block in content
                        if row["state"] == "RESOLVED" and row["resolved_event_id"] is not None and
                        isinstance(block, dict) and block.get("type") == "text" and
                        isinstance(block.get("text"), str))
        if frame.get("type") == "user":
            message = frame.get("message")
            content = message.get("content") if isinstance(message, dict) else None
            if isinstance(content, list):
                tool_results.extend((row, frame, block, raw) for block in content
                    if isinstance(block, dict) and block.get("type") == "tool_result")
    check(len(uses) == len(tool_results) == 1,
          "Original Claude A interval does not contain exactly one tool_use and one tool_result")
    use_row, use_frame, use, use_raw = uses[0]
    result_row, result_frame, tool_result, result_raw = tool_results[0]
    assistant_answers = [(row, block) for row, block in assistant_texts
                         if int(row["source_cursor"]) > int(result_row["source_cursor"])]
    tool_input = use.get("input")
    result_message = result_frame.get("message")
    result_content = result_message.get("content") if isinstance(result_message, dict) else None
    check(use_frame.get("session_id") == result_frame.get("session_id") == session_id and
          use.get("name") == child["toolName"] == "Bash" and isinstance(tool_input, dict) and
          tool_input.get("command") == child["exactCommand"] and
          ("cwd" not in tool_input or local_spelling(tool_input["cwd"]) == local_spelling(child["worktreePath"])) and
          result_frame.get("type") == "user" and result_frame.get("parent_tool_use_id") in (None, "") and
          isinstance(result_message, dict) and result_message.get("role") == "user" and
          isinstance(result_content, list) and len(result_content) == 1 and result_content[0] == tool_result and
          isinstance(use.get("id"), str) and use.get("id") and
          use.get("id") == tool_result.get("tool_use_id") and
          use_row["state"] in ("PENDING", "NO_EVENT") and use_row["resolved_event_id"] is None and
          result_row["state"] in ("PENDING", "NO_EVENT") and result_row["resolved_event_id"] is None and
          int(result_row["source_cursor"]) > int(use_row["source_cursor"]) and
          int(result_row["source_cursor"]) < terminal,
          "Original Bash use/result IDs, command, cwd, source order, or unhandled state differ")
    content_value = tool_result.get("content")
    if isinstance(content_value, str):
        result_text = content_value
    elif isinstance(content_value, list) and content_value and all(
            isinstance(part, dict) and part.get("type") == "text" and isinstance(part.get("text"), str)
            for part in content_value):
        result_text = "".join(part["text"] for part in content_value)
    else:
        result_text = None
    try:
        tool_output = json.loads(result_text.strip()) if isinstance(result_text, str) else None
    except (json.JSONDecodeError, TypeError):
        tool_output = None
    expected_output = {"nonce": child["childNonce"], "pid": int(child["childPid"]),
                       "readyMarkerPath": child["readyMarkerPath"]}
    answer_text = "".join(block["text"] for _, block in assistant_answers)
    try:
        assistant_output = json.loads(answer_text.strip())
    except (json.JSONDecodeError, TypeError):
        assistant_output = None
    check(tool_result.get("is_error") is False and tool_output == expected_output and
          assistant_output == expected_output and answer_text.strip() == result_text.strip() ==
          terminal_frame.get("result"),
          "Original tool_result, final assistant echo, and nonce/PID marker payload differ")
    return {"toolUseId": use["id"], "toolName": use["name"], "exactCommand": tool_input["command"],
            "cwd": child["worktreePath"], "toolUseInputCwd": tool_input.get("cwd"),
            "cwdBasis": "ORIGINAL_TOOL_INPUT_CWD" if "cwd" in tool_input else "ORIGINAL_CLAUDE_INIT_CWD",
            "toolUseSourceCursor": str(use_row["source_cursor"]),
            "toolUseRawSha256": digest(use_raw), "toolResultSourceCursor": str(result_row["source_cursor"]),
            "toolResultRawSha256": digest(result_raw), "toolResultState": result_row["state"],
            "toolResultNoEventReason": result_row["no_event_reason"],
            "toolResultClassification": "ORIGINAL_UNHANDLED_CLAUDE_TOOL_RESULT_NO_SUCCESS_CREDIT",
            "toolOutput": tool_output, "terminalSourceCursor": str(terminal_row["source_cursor"]),
            "rawResultSha256": digest(terminal_raw), "assistantOutput": assistant_output,
            "rawUserReplayHex": echoes[0][2].hex(), "rawToolUseHex": use_raw.hex(),
            "rawToolResultHex": result_raw.hex(), "rawTerminalResultHex": terminal_raw.hex(),
            "rawInitHex": bytes(init_raw_bytes).hex() if init_raw_bytes is not None else None,
            "rawSourceFrames": [{"sourceCursor": str(row["source_cursor"]), "state": row["state"],
                "noEventReason": row["no_event_reason"], "rawSha256": digest(raw), "rawBytesHex": raw.hex()}
                for row, _frame, raw in decoded]}


def child_case_facts(db, journal, case, tree, phase, stop):
    child = case.get("childCase")
    if not isinstance(child, dict) or child.get("state") != "PRESTOP_IDENTITY_VERIFIED":
        return {"state": "NOT_RUN_OR_INCOMPLETE", "acceptance": False}
    child = {**child, "processOperationId": stop["processOperationId"],
             "ticket": stop["processTicket"], "custodianNonce": stop["custodianNonce"]}
    send_id = child.get("sendRequestId")
    sent = operation(journal, send_id)
    output_receipt = child_terminal_output_receipt(journal, case["session"], child)
    request = sent["request"]
    pre_read_id = child.get("beforeRawHighwaterRequestId")
    pre_read = operation(journal, pre_read_id)
    operations = journal.get("operations", [])
    pre_position = next((index for index, entry in enumerate(operations)
                         if entry.get("request", {}).get("requestId") == pre_read_id), -1)
    send_position = next((index for index, entry in enumerate(operations)
                          if entry.get("request", {}).get("requestId") == send_id), -1)
    pre_result = pre_read["receipt"].get("result", {})
    check(request.get("family") == "K-SESSION" and request.get("operation") == "send" and
          request.get("domainId") == case["domainId"] and request.get("targetId") == case["session"]["id"] and
          request.get("payload") == {"generation": case["session"]["generation"], "body": child["body"]} and
          pre_read["request"].get("family") == "K-SESSION" and
          pre_read["request"].get("operation") == "output-stream" and
          pre_read["request"].get("domainId") == case["domainId"] and
          pre_read["request"].get("targetId") == case["session"]["id"] and
          pre_read["request"].get("payload") == {"generation": case["session"]["generation"], "afterCursor": "0"} and
          pre_read["receipt"].get("status") == "APPLIED" and
          pre_result.get("rawHighwater") == child.get("beforeRawHighwater") and
          0 <= pre_position < send_position and
          isinstance(child.get("beforeRawHighwater"), str) and child["beforeRawHighwater"].isdecimal() and
          child.get("sourceEpoch") and isinstance(child.get("terminalCursor"), str) and
          child["terminalCursor"].isdecimal() and
          int(child["terminalCursor"]) > int(child["beforeRawHighwater"]),
          "Original child H send request or source interval is not bound")
    stdin = one(db, "SELECT phase,receipt_status,request_hex,session_id,generation,ticket,process_operation_id,"
               "custodian_nonce,receipt_hex,request_id FROM gogoke_v37_h_stdin_journal "
               "WHERE domain_id=? AND session_id=? AND request_id=? AND operation='send'",
               (case["domainId"], case["session"]["id"], send_id))
    request_wire = json.loads(bytes.fromhex(stdin["request_hex"]).decode("utf-8"))
    receipt_wire = json.loads(bytes.fromhex(stdin["receipt_hex"]).decode("utf-8"))
    initial_receipt = sent["receipt"]
    receipt = child.get("terminalReceipt")
    send_rows = rows(db, "SELECT request_id FROM gogoke_v37_h_stdin_journal "
                     "WHERE domain_id=? AND session_id=? AND operation='send' ORDER BY rowid",
                     (case["domainId"], case["session"]["id"]))
    check(send_rows == [{"request_id": send_id}] and
          stdin["phase"] == "RECEIPTED" and stdin["receipt_status"] == "APPLIED" and
          stdin["request_id"] == send_id and stdin["session_id"] == case["session"]["id"] and
          stdin["generation"] == case["session"]["generation"] and
          stdin["process_operation_id"] == child["processOperationId"] and
          stdin["ticket"] == child["ticket"] and stdin["custodian_nonce"] == child["custodianNonce"] and
          request_wire == request and json.loads(sent["rawReceipt"]) == initial_receipt and
          isinstance(receipt, dict) and receipt_wire == receipt and receipt.get("requestId") == send_id and
          receipt["status"] == "APPLIED" and
          receipt["result"].get("createdTurn") is True and
          receipt["result"].get("deliveryBasis") == "CLAUDE_USER_REPLAY_AND_RESULT" and
          receipt["result"].get("resultSubtype") == "success" and
          receipt["result"].get("userUuid") == child["userUuid"] and
          receipt["result"].get("vendorSessionId") == child["vendorSessionId"] and
          receipt["result"].get("sourceEpoch") == child["sourceEpoch"] and
          receipt["result"].get("sourceCursor") == child["terminalCursor"] and
          receipt["result"].get("rawResultSha256") == child["rawResultSha256"],
          "Original child H send is not the exact typed successful Claude receipt")
    raw_rows = rows(db, "SELECT generation,operation_id,source_epoch,source_cursor,raw_bytes,state,"
                     "process_ticket,custodian_nonce,no_event_reason,resolved_event_id "
                     "FROM v37_ledger_raw_source WHERE domain_id=? AND session_id=? AND operation_id=? "
                     "AND generation=? AND process_ticket=? AND custodian_nonce=? "
                     "AND source_epoch=? AND CAST(source_cursor AS INTEGER)>? AND CAST(source_cursor AS INTEGER)<=? "
                     "ORDER BY CAST(source_cursor AS INTEGER)",
                     (case["domainId"], case["session"]["id"], child["processOperationId"],
                      case["session"]["generation"], child["ticket"], child["custodianNonce"],
                      child["sourceEpoch"], child["beforeRawHighwater"], child["terminalCursor"]))
    prior_inits = rows(db, "SELECT source_epoch,source_cursor,raw_bytes,state,no_event_reason "
                      "FROM v37_ledger_raw_source WHERE domain_id=? AND session_id=? "
                      "AND operation_id=? AND generation=? AND process_ticket=? AND custodian_nonce=? "
                      "AND source_epoch=? ORDER BY CAST(source_cursor AS INTEGER)",
                      (case["domainId"], case["session"]["id"], child["processOperationId"],
                       case["session"]["generation"], child["ticket"], child["custodianNonce"], child["sourceEpoch"]))
    check(all(str(row["source_cursor"]).isdecimal() for row in prior_inits),
          "Original Claude source cursor is not decimal")
    prior_cursors = [int(row["source_cursor"]) for row in prior_inits
                     if str(row["source_cursor"]).isdecimal() and
                     int(row["source_cursor"]) <= int(child["beforeRawHighwater"])]
    check((max(prior_cursors) if prior_cursors else 0) == int(child["beforeRawHighwater"]),
          "Pre-send H raw highwater is not the exact original A cursor before the send")
    init_sources = [(row, json.loads(bytes(row["raw_bytes"]).decode("utf-8"))) for row in prior_inits]
    all_inits = [(row, frame) for row, frame in init_sources
                 if frame.get("type") == "system" and frame.get("subtype") == "init"]
    matching_init = [(row, frame) for row, frame in all_inits
                     if frame.get("session_id") == child["vendorSessionId"]]
    check(len(matching_init) >= 1 and len(all_inits) == len(matching_init) and
          all(frame.get("claude_code_version") == "2.1.196" and
              local_spelling(frame.get("cwd", "")) == local_spelling(tree["worktree_path"])
              for _row, frame in all_inits) and
          any(int(row["source_cursor"]) <= int(child["beforeRawHighwater"])
              for row, _frame in matching_init),
          "Original child H/A process has no exact same-session init/cwd/version binding")
    init_row, init_frame = matching_init[0]
    tool_facts = child_tool_source_facts(raw_rows, init_frame, child, bytes(init_row["raw_bytes"]))
    identity_facts = child_identity_facts(child, child.get("readyMarker"))
    cleanup_entry = operation(journal, case["preStopCleanupRequestId"])
    stop_entry = operation(journal, case["session"]["stopRequestId"])
    cleanup_position = next((index for index, entry in enumerate(journal["operations"])
                             if entry.get("request", {}).get("requestId") == case["preStopCleanupRequestId"]), -1)
    stop_position = next((index for index, entry in enumerate(journal["operations"])
                          if entry.get("request", {}).get("requestId") == case["session"]["stopRequestId"]), -1)
    check(type(child.get("identityJournalOperationCount")) is int and
          child["identityJournalOperationCount"] == cleanup_position and
          0 <= cleanup_position < stop_position and
          cleanup_entry["request"].get("operation") == "cleanup" and
          stop_entry["request"].get("operation") == "stop",
          "Original same-handle live observation did not precede denied cleanup and H stop")
    marker_file_facts = None
    if phase == "stopped":
        marker_path = native.contained_physical(tree["worktree_path"], child["readyMarkerPath"])
        marker_raw = marker_path.read_bytes()
        marker = json.loads(marker_raw.decode("utf-8"))
        check(digest(marker_raw) == child["readyMarkerSha256"] and marker == child["readyMarker"],
              "Original test-owned F ready marker bytes or identity changed")
        marker_file_facts = {"sha256": digest(marker_raw), "length": len(marker_raw),
                             "path": str(marker_path), "rawBytesHex": marker_raw.hex()}
    else:
        marker_file_facts = child.get("readyMarkerReadback")
        check(isinstance(marker_file_facts, dict) and marker_file_facts.get("sha256") == child["readyMarkerSha256"],
              "Final phase lacks the exact stopped-phase ready marker readback")
    return {"state": "SOURCE_AND_STOP_FACT_RECORDED_REQUIRES_FINAL_COMMON_CHECK", "acceptance": False,
            "sendRequestId": send_id, "processOperationId": child["processOperationId"],
            "outputReceipt": output_receipt,
            "sourceEpoch": child["sourceEpoch"], "beforeRawHighwater": child["beforeRawHighwater"],
            "terminalCursor": child["terminalCursor"], "vendorSessionId": child["vendorSessionId"],
            "rawSendRequestHex": stdin["request_hex"], "rawTerminalReceiptHex": stdin["receipt_hex"],
            "identity": identity_facts, "readyMarkerReadback": marker_file_facts,
            "toolEvidence": tool_facts, "stopFactId": case["session"]["stopFactId"]}


def worktree_facts(db, journal, case, phase):
    row = one(db, "SELECT worktree_id,path_id,repository_id,domain_id,seat_id,seat_incarnation,"
                "seat_generation,instance_id,worktree_path,worktree_identity,git_pointer_hash,"
                "git_pointer_identity,common_identity,baseline_commit,state,revision "
                "FROM gogoke_v37_worktrees WHERE worktree_id=?", (case["worktreeId"],))
    check((row["repository_id"], row["domain_id"], row["seat_id"], row["instance_id"], row["state"]) ==
          (case["repositoryId"], case["domainId"], case["seatId"], case["instanceId"], "REGISTERED"),
          "Original F registered tree identity differs")
    root = Path(case["stateRoot"]).resolve(strict=True)
    worktree_path = native.contained_physical(root / "v37-worktrees", row["worktree_path"],
                                              child_must_exist=phase == "stopped")
    exists = worktree_path.is_dir() and not worktree_path.is_symlink()
    stop = session_facts(db, journal, case)
    child_evidence = child_case_facts(db, journal, case, row, phase, stop)
    lifecycle = rows(db, "SELECT state,revision,COALESCE(stop_fact_id,'') AS stop_fact_id "
                     "FROM gogoke_v37_worktree_lifecycle WHERE worktree_id=?", (case["worktreeId"],))
    cleanup_rows = rows(db, "SELECT request_id,request_hash,worktree_id,operation,phase "
                        "FROM gogoke_v37_worktree_lifecycle_ops WHERE worktree_id=? AND operation='CLEANUP' "
                        "ORDER BY rowid", (case["worktreeId"],))
    denied = operation(journal, case["preStopCleanupRequestId"])
    request = denied["request"]
    check(request["family"] == "K-WORKTREE" and request["operation"] == "cleanup" and
          request["targetId"] == case["worktreeId"] and request["domainId"] == case["domainId"] and
          request["expectedRevision"] == str(case["graphRevision"]) and request["payload"] == {} and
          denied["receipt"]["status"] == "DENIED" and
          denied["receipt"]["revision"] == str(case["graphRevision"]) and
          all(item["request_id"] != case["preStopCleanupRequestId"] for item in cleanup_rows),
          "Original denied User cleanup request wrote an F cleanup intent or its receipt differs")
    if phase == "stopped":
        check(exists and not cleanup_rows and lifecycle == [{
              "state": "REGISTERED", "revision": int(case["graphRevision"]), "stop_fact_id": ""}],
              "Stopped-only readback must retain the registered tree and no cleanup intent")
        return {"worktreeId": case["worktreeId"], "repositoryId": row["repository_id"],
                "domainId": row["domain_id"], "seatId": row["seat_id"], "instanceId": row["instance_id"],
                "worktreeIdentity": row["worktree_identity"], "gitPointerHash": row["git_pointer_hash"],
                "gitPointerIdentity": row["git_pointer_identity"], "commonIdentity": row["common_identity"],
                "baselineCommit": row["baseline_commit"], "pathDevice": str(worktree_path.stat().st_dev),
                "pathInode": str(worktree_path.stat().st_ino), "exists": True,
                "lifecycleState": "REGISTERED", "lifecycleRevision": case["graphRevision"],
                "stopFactId": stop["stopFactId"], "session": stop,
                "childEvidence": child_evidence}
    cleanup = operation(journal, case["cleanupRequestId"])
    raw = cleanup["rawFrame"].encode("utf-8")
    lifecycle_row = one(db, "SELECT state,revision,COALESCE(stop_fact_id,'') AS stop_fact_id "
                        "FROM gogoke_v37_worktree_lifecycle WHERE worktree_id=?", (case["worktreeId"],))
    applied = one(db, "SELECT request_hash,worktree_id,operation,phase FROM gogoke_v37_worktree_lifecycle_ops "
                     "WHERE request_id=?", (case["cleanupRequestId"],))
    graph = case["graphRevision"]
    check(not exists and cleanup["receipt"]["status"] == "APPLIED" and
          len(cleanup_rows) == 1 and cleanup_rows[0]["request_id"] == case["cleanupRequestId"] and
          cleanup["receipt"]["result"].get("worktreeId") == case["worktreeId"] and
          cleanup["receipt"]["result"].get("stopFactId") == stop["stopFactId"] and
          lifecycle_row["state"] == "CLEANED" and lifecycle_row["revision"] == int(graph) + 1 and
          lifecycle_row["stop_fact_id"] == stop["stopFactId"] and
          applied == {"request_hash": digest(raw), "worktree_id": case["worktreeId"],
                      "operation": "CLEANUP", "phase": "APPLIED"} and
          cleanup["receipt"]["revision"] == str(lifecycle_row["revision"]),
          "Original F cleanup request/hash/APPLIED lifecycle/physical removal/StopFact chain differs")
    return {"worktreeId": case["worktreeId"], "repositoryId": row["repository_id"],
            "domainId": row["domain_id"], "seatId": row["seat_id"], "instanceId": row["instance_id"],
            "worktreeIdentity": row["worktree_identity"], "gitPointerHash": row["git_pointer_hash"],
            "gitPointerIdentity": row["git_pointer_identity"], "commonIdentity": row["common_identity"],
            "baselineCommit": row["baseline_commit"], "exists": False,
            "lifecycleState": lifecycle_row["state"], "lifecycleRevision": lifecycle_row["revision"],
            "stopFactId": lifecycle_row["stop_fact_id"], "cleanupRequestId": case["cleanupRequestId"],
            "session": stop, "childEvidence": child_evidence}


def main():
    check(len(sys.argv) == 5 and sys.argv[4] in ("stopped", "final"),
          "Expected state root, output, original M2 journal and stopped|final phase")
    root, output, journal_file = Path(sys.argv[1]).resolve(strict=True), Path(sys.argv[2]).resolve(), Path(sys.argv[3]).resolve(strict=True)
    check(not output.exists() and not output.is_relative_to(root),
          "Fresh private readback must stay outside candidate state")
    journal = json.loads(journal_file.read_text(encoding="utf-8-sig"))
    case = journal.get("stopWorktree")
    check(case and case["schema"] == "gogoke.37.m2-stop-worktree.v1" and
          case["acceptance"] is False and case["sourceCommit"] == journal["sourceCommit"] and
          Path(case["stateRoot"]).resolve(strict=True) == root, "Original stop-worktree case/root required")
    reader_sha256 = digest(Path(__file__).read_bytes())
    check(reader_sha256 == case["readerSha256"], "Executing reader bytes differ from the original case pin")
    evidence = Path(case["evidenceDirectory"]).resolve(strict=True)
    check(output.parent == journal_file.parent == evidence and Path(journal_file.parent).resolve(strict=True) == evidence,
          "Original journal/output must remain in the case evidence directory")
    launches, closes = journal.get("launches", []), journal.get("closes", [])
    check(launches and closes, "Actual installed candidate launch and normal close are required")
    launch, close, endpoint = launches[-1], closes[-1], journal.get("currentEndpoint", {})
    bootstrap = launch.get("bootstrap", {})
    check(launch.get("pid") == close.get("pid") == endpoint.get("pid") and close.get("exitCode") == 0 and
          close.get("forceKill") is False and launch.get("sourceCommit") == case["sourceCommit"] and
          bootstrap.get("version") == case["candidateVersion"] and launch.get("setId") == bootstrap.get("setId") and
          launch.get("generationId") == bootstrap.get("generationId"),
          "Latest actual installed candidate lacks its matching normal-close receipt")
    installed = case.get("candidateInstalledSha256")
    check(isinstance(installed, dict) and installed and all(isinstance(value, str) and len(value) == 64
          and all(ch in "0123456789abcdef" for ch in value) for value in installed.values()),
          "Actual candidate installed byte pins are required")
    st, dbst = root.stat(), (root / "state.sqlite").stat()
    root_identity = {"device": str(st.st_dev), "inode": str(st.st_ino),
                     "databaseDevice": str(dbst.st_dev), "databaseInode": str(dbst.st_ino)}
    candidate = {"sourceCommit": launch["sourceCommit"], "version": bootstrap["version"],
                 "setId": launch["setId"], "generationId": launch["generationId"]}
    if sys.argv[4] == "final":
        ref = case.get("baseline")
        check(ref and Path(ref["file"]).name == ref["file"], "Original stopped-only baseline reference is required")
        base_path = evidence / ref["file"]
        check(digest(base_path.read_bytes()) == ref["sha256"], "Original stopped-only baseline bytes changed")
        baseline = json.loads(base_path.read_text(encoding="utf-8-sig"))
        check(baseline.get("phase") == "stopped" and baseline.get("caseId") == journal["caseId"] and
              baseline.get("sourceCommit") == case["sourceCommit"] and baseline.get("rootIdentity") == root_identity and
              baseline.get("candidateIdentity") == candidate and baseline.get("candidateInstalledSha256") == installed and
              baseline.get("worktree", {}).get("worktreeIdentity") is not None,
              "Final candidate/root/F identity differs from the exact stopped baseline")
    database, wal, shm = root / "state.sqlite", root / "state.sqlite-wal", root / "state.sqlite-shm"
    check(database.is_file() and (not wal.exists() or wal.stat().st_size == 0),
          "Normal close and empty/absent WAL required")
    def files():
        return {item.name: {"length": item.stat().st_size, "sha256": digest(item.read_bytes())}
                for item in (database, wal, shm) if item.exists()}
    before = files()
    with native.closed_database(root) as (db, native_report):
        worktree = worktree_facts(db, journal, case, sys.argv[4])
    after = files()
    check(before == after and (not wal.exists() or wal.stat().st_size == 0),
          "Immutable measurement changed candidate database/WAL/SHM bytes")
    if sys.argv[4] == "final":
        check(worktree.get("stopFactId") == case["session"]["stopFactId"] and
              worktree.get("lifecycleState") == "CLEANED", "Final worktree is not cleaned by its original StopFact")
        check(worktree.get("childEvidence") == baseline.get("worktree", {}).get("childEvidence"),
              "Final raw A/tool-result evidence differs from the same stopped baseline")
        identity_fields = ("worktreeId", "repositoryId", "domainId", "seatId", "instanceId",
                           "worktreeIdentity", "gitPointerHash", "gitPointerIdentity", "commonIdentity", "baselineCommit")
        check(all(worktree[key] == baseline["worktree"][key] for key in identity_fields),
              "Final persistent F identity differs from the stopped baseline")
    proof = {"schema": "gogoke.37.private-m2-stop-worktree-readback.v1", "phase": sys.argv[4],
             "caseId": journal["caseId"], "sourceCommit": case["sourceCommit"], "domainId": case["domainId"],
             "stateRoot": str(root), "evidenceDirectory": str(evidence),
             "readerSha256": reader_sha256, "candidateIdentity": candidate,
             "candidateInstalledSha256": installed, "rootIdentity": root_identity,
             "launch": {"pid": launch["pid"], "sourceCommit": launch["sourceCommit"]},
             "normalClose": {"pid": close["pid"], "exitCode": close["exitCode"], "forceKill": close["forceKill"]},
             "databaseSha256": digest(database.read_bytes()), "filesBefore": before, "filesAfter": after,
              "nativeReadback": native_report,
             "measurementPreservedDatabaseBytes": before == after, "acceptance": False,
             "stopFactId": worktree["stopFactId"], "stopOnlyEvidence": sys.argv[4] == "stopped",
             "cleanupApplied": sys.argv[4] == "final", "directCaseEvidence": sys.argv[4] == "final",
             "worktree": worktree,
             "notRun": case["notRun"]}
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(proof, stream, ensure_ascii=False, indent=2); stream.write("\n")
    check(proof["measurementPreservedDatabaseBytes"], "Readback must preserve original state bytes")
    print(json.dumps({"phase": sys.argv[4], "directCaseEvidence": proof["directCaseEvidence"], "acceptance": False}))


if __name__ == "__main__":
    main()
