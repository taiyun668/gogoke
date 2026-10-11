"""Readonly post-close observer for real provider source and question cases.

signed Python m2-provider-readback.py STATE_ROOT NEW_PRIVATE_OUTPUT ORIGINAL_JOURNAL
Never opens a credential, active WAL, vendor memory file or authentication page.
"""
import hashlib
import json
import os
import sqlite3
import sys
from pathlib import Path

if len(sys.argv) != 4:
    raise RuntimeError("Expected candidate state root, new private output and original journal")
root = Path(sys.argv[1]).resolve(strict=True)
output = Path(sys.argv[2])
journal_file = Path(sys.argv[3]).resolve(strict=True)
if output.exists():
    raise RuntimeError("Readonly evidence output already exists")
if output.resolve(strict=False).is_relative_to(root):
    raise RuntimeError("Provider evidence output must stay outside the candidate state root")
db_file = root / "state.sqlite"
wal = Path(str(db_file) + "-wal")
if not db_file.is_file() or wal.exists() and wal.stat().st_size:
    raise RuntimeError("Actual candidate must be normally closed with an empty WAL")
journal = json.loads(journal_file.read_text(encoding="utf-8-sig"))
cases = journal.get("providerBoundaryCases")
if not isinstance(cases, list) or not cases or journal.get("providerBoundarySummary", {}).get("acceptance") is not False:
    raise RuntimeError("Original provider case journal absent or falsely accepted")

def hash_bytes(data):
    return hashlib.sha256(data).hexdigest()

def file_facts():
    return {p.name: {"length": p.stat().st_size, "sha256": hash_bytes(p.read_bytes())}
            for p in (db_file, wal, Path(str(db_file) + "-shm")) if p.exists()}

def one(db, sql, args=()):
    found = db.execute(sql, args).fetchall()
    if len(found) != 1:
        raise RuntimeError(f"Expected exactly one original row, found {len(found)}: {sql.split(' FROM ')[0]}")
    return found[0]

def local_path(value):
    text = str(value)
    if text.startswith("\\\\?\\UNC\\"):
        raise RuntimeError("Network provider worktree not in this candidate")
    target = Path(text[4:] if text.startswith("\\\\?\\") else text)
    if target.is_symlink():
        raise RuntimeError("Provider worktree root is a link")
    return target.resolve(strict=True)

def within(parent, child):
    parent_text, child_text = os.path.normcase(str(parent)), os.path.normcase(str(child))
    return os.path.commonpath((parent_text, child_text)) == parent_text

def original_claude_question(db, case, journal, operation_rows, raw_rows, tree_path, worktree_identity):
    domain, session = case["domainId"], case["sessionId"]
    send_id = case["sendRequestId"]
    original_send = operation_rows[send_id]
    send_wire = original_send["rawFrame"].encode()
    if original_send["request"].get("payload", {}).get("body") != case["prompt"]:
        raise RuntimeError("Claude H User prompt differs from original case")
    send = one(db,
        "SELECT phase,receipt_status,request_hex,receipt_hex,process_operation_id,generation,ticket,custodian_nonce "
        "FROM gogoke_v37_h_stdin_journal WHERE domain_id=? AND session_id=? AND request_id=? AND operation='send'",
        (domain, session, send_id))
    if send[:3] != ("RECEIPTED", "APPLIED", send_wire.hex()):
        raise RuntimeError("Claude original User did not complete through H")
    h_receipt = json.loads(bytes.fromhex(send[3]))
    if h_receipt.get("status") != "APPLIED" or h_receipt.get("result", {}).get("deliveryBasis") != "CLAUDE_USER_REPLAY_AND_RESULT":
        raise RuntimeError("Claude H receipt is not original echoed User plus Result")
    epoch, result_cursor = (h_receipt["result"][name] for name in ("sourceEpoch", "sourceCursor"))
    by_key = {(str(row[2]), str(row[3])): row for row in raw_rows}
    def original(epoch_cursor):
        row = by_key.get(epoch_cursor)
        if row is None or row[1] != send[4] or row[0] != send[5] or row[6:] != (send[6], send[7]):
            raise RuntimeError("Claude original A source is outside the same physical H custody")
        return row, json.loads(bytes(row[4]))
    result_row, terminal = original((epoch, result_cursor))
    result_reason = one(db,
        "SELECT no_event_reason FROM v37_ledger_raw_source WHERE operation_id=? AND source_epoch=? AND source_cursor=?",
        (send[4], epoch, result_cursor))[0]
    if terminal.get("type") != "result" or terminal.get("session_id") != h_receipt["result"].get("vendorSessionId") or \
            terminal.get("subtype") != "success" or terminal.get("is_error") is not False or \
            hash_bytes(bytes(result_row[4])) != h_receipt["result"].get("rawResultSha256") or \
            result_row[5] != "NO_EVENT" or result_reason != "CLAUDE_RESULT_RESPONSE":
        raise RuntimeError("Claude CLI success Result does not match original H terminal receipt")
    if db.execute(
            "SELECT 1 FROM gogoke_v37_seat_health e JOIN v37_ledger_raw_source r "
            "ON r.resolved_event_id=e.source_event_id AND r.domain_id=e.domain_id "
            "WHERE r.domain_id=? AND r.session_id=? AND r.operation_id=? LIMIT 1",
            (domain, session, send[4])).fetchone() is not None or db.execute(
            "SELECT 1 FROM gogoke_v37_h_generation_change WHERE domain_id=? AND session_id=? LIMIT 1",
            (domain, session)).fetchone() is not None:
        raise RuntimeError("Normal real Claude session created a Codex-only health action")
    send_step = "claude-send-" + hash_bytes(send_wire)[:40]
    echo = one(db,
        "SELECT phase,command_hex,source_epoch,source_cursor,process_operation_id,ticket,custodian_nonce "
        "FROM gogoke_v37_rpc_steps WHERE domain_id=? AND session_id=? AND step_id=?",
        (domain, session, send_step))
    if echo[0] != "OBSERVED" or echo[4:] != (send[4], send[6], send[7]):
        raise RuntimeError("Claude original H User echo step is not physically bound")
    echo_row, user_echo = original((echo[2], echo[3]))
    echo_reason = one(db,
        "SELECT no_event_reason FROM v37_ledger_raw_source WHERE operation_id=? AND source_epoch=? AND source_cursor=?",
        (send[4], echo[2], echo[3]))[0]
    if user_echo.get("type") != "user" or user_echo.get("uuid") != h_receipt["result"].get("userUuid") or \
            user_echo.get("session_id") != terminal["session_id"] or \
            user_echo.get("message", {}).get("content", [{}])[0].get("text") != case["prompt"] or \
            int(echo[3]) >= int(result_cursor) or echo_reason != "CLAUDE_STDIN_ACK":
        raise RuntimeError("Claude original echoed User text/UUID/session differs")
    card_ref = case["card"]
    card = one(db,
        "SELECT revision,state,vendor_request_id,vendor_thread_id,vendor_item_id,question_payload,"
        "question_id,question_header,question_text,answer_shape,seat_id,turn_id,generation,answer_kind,answer "
        "FROM gogoke_v37_qcard_native WHERE domain_id=? AND card_id=?",
        (domain, card_ref["cardId"]))
    payload = json.loads(card[5])
    if card[1] != "ANSWERED" or card[3] != terminal["session_id"] or card[6:9] != \
            ("host0", case["claudeQuestion"]["header"], case["claudeQuestion"]["text"]) or \
            card[10:14] != (case["seatId"], send_id, send[5], "WIRE") or \
            payload.get("provider") != "claude" or payload.get("idOrigin") != "HOST_DERIVED_ARRAY_INDEX" or \
            payload.get("originalInput") != card_ref["originalInput"] or \
            payload.get("questions", [{}])[0].get("hostQuestionId") != "host0" or \
            payload.get("questions", [{}])[0].get("multiSelect") is not False:
        raise RuntimeError("Claude C answer does not retain complete original current-turn question")
    options = db.execute(
        "SELECT option_id,label,description FROM gogoke_v37_qcard_native_options "
        "WHERE domain_id=? AND card_id=? ORDER BY CAST(ordinal AS INTEGER)",
        (domain, card_ref["cardId"])).fetchall()
    if options != [("option0", "JSON", "Write the specified JSON marker."),
                   ("option1", "Plain", "Write plain text instead.")]:
        raise RuntimeError("Claude original two options differ from the User case")
    raised = one(db,
        "SELECT request_hex FROM gogoke_v37_qcard_native_operations "
        "WHERE domain_id=? AND card_id=? AND state='RAISED'",
        (domain, card_ref["cardId"]))[0]
    descriptor = json.loads(bytes.fromhex(raised))
    if descriptor.get("operationId") != send[4] or descriptor.get("sourceEpoch") != epoch or \
            descriptor.get("frameSha256") is None:
        raise RuntimeError("Claude C source descriptor is not from this H process")
    question_row, asking = original((descriptor["sourceEpoch"], descriptor["sourceCursor"]))
    question_reason = one(db,
        "SELECT no_event_reason FROM v37_ledger_raw_source WHERE operation_id=? AND source_epoch=? AND source_cursor=?",
        (send[4], descriptor["sourceEpoch"], descriptor["sourceCursor"]))[0]
    request = asking.get("request", {})
    if question_row[5] != "NO_EVENT" or question_reason != "NATIVE_CLAUDE_QUESTION_CARD" or \
            hash_bytes(bytes(question_row[4])) != descriptor["frameSha256"] or \
            not int(echo[3]) < int(descriptor["sourceCursor"]) < int(result_cursor) or \
            asking.get("type") != "control_request" or request.get("subtype") != "can_use_tool" or \
            request.get("tool_name") != "AskUserQuestion" or request.get("input") != payload["originalInput"] or \
            json.dumps(asking.get("request_id"), ensure_ascii=False, separators=(",", ":")) != card[2] or \
            request.get("tool_use_id") != card[4]:
        raise RuntimeError("Claude C card is not the original fixed AskUserQuestion A frame")
    recovered = operation_rows.get(next((entry["request"]["requestId"] for entry in journal["operations"]
        if entry.get("request", {}).get("family") == "K-QCARD" and
        entry["request"].get("operation") == "recover" and
        entry["request"].get("targetId") == card_ref["cardId"]), ""))
    if recovered is None or recovered.get("receipt", {}).get("result", {}).get("nativeQuestion") != payload or \
            recovered["receipt"]["result"].get("turnId") != send_id:
        raise RuntimeError("Claude original User C recover did not show complete question/host turn")
    answer_id = card_ref["answerRequestId"]
    answer_user = operation_rows.get(answer_id)
    if answer_user is None or answer_user["request"].get("family") != "K-QCARD" or \
            answer_user["request"].get("operation") != "answer" or \
            answer_user["request"].get("payload", {}).get("answers") != {"host0": ["JSON"]}:
        raise RuntimeError("Claude original User answer request absent")
    answer = one(db,
        "SELECT request_hex,state,answer_kind,answer,native_receipt_id FROM gogoke_v37_qcard_native_operations "
        "WHERE domain_id=? AND card_id=? AND request_id=?",
        (domain, card_ref["cardId"], answer_id))
    if answer[0] != answer_user["rawFrame"].encode().hex() or answer[1:4] != ("ANSWERED", "WIRE", card[14]) or \
            not answer[4].startswith("h-qanswer-"):
        raise RuntimeError("Claude C answer CAS and exact H receipt differ")
    wire = json.loads(card[14])
    if wire.get("type") != "control_response" or wire.get("response", {}).get("request_id") != asking["request_id"] or \
            wire["response"].get("response", {}).get("toolUseID") != card[4] or \
            wire["response"]["response"].get("updatedInput", {}).get("answers") != \
                {case["claudeQuestion"]["text"]: "JSON"}:
        raise RuntimeError("Claude H control_response did not answer the original question")
    answer_step = "qanswer" + hash_bytes(f"{domain}\n{session}\n{answer_id}".encode())
    h_answer = one(db,
        "SELECT phase,requires_response,command_hex,process_operation_id,ticket,custodian_nonce,generation "
        "FROM gogoke_v37_rpc_steps WHERE domain_id=? AND session_id=? AND step_id=?",
        (domain, session, answer_step))
    if h_answer[:2] != ("WRITTEN", 0) or h_answer[2] != (card[14] + "\n").encode().hex() or \
            h_answer[3:] != (send[4], send[6], send[7], send[5]):
        raise RuntimeError("Claude sole H stdin answer exactwrite is not retained")
    resolved = []
    for row in raw_rows:
        if row[2] != epoch or row[1] != send[4] or row[0] != send[5] or \
                row[6:] != (send[6], send[7]) or \
                not int(descriptor["sourceCursor"]) < int(row[3]) < int(result_cursor):
            continue
        frame = json.loads(bytes(row[4]))
        if frame.get("type") != "user" or frame.get("session_id") != terminal["session_id"]:
            continue
        parts = frame.get("message", {}).get("content")
        if isinstance(parts, list) and parts and all(isinstance(part, dict) and part.get("type") == "tool_result"
            for part in parts) and any(part.get("tool_use_id") == card[4] for part in parts):
            resolved.append(row)
    if len(resolved) != 1:
        raise RuntimeError("Claude original AskUserQuestion tool_result is absent or ambiguous")
    post = [row for row in raw_rows if row[2] == epoch and row[1] == send[4] and row[0] == send[5]
            and row[6:] == (send[6], send[7]) and
            int(resolved[0][3]) < int(row[3]) < int(result_cursor)
            and json.loads(bytes(row[4])).get("type") == "assistant" and
            case["claudeQuestion"]["markerFile"] in bytes(row[4]).decode("utf-8")]
    if not post:
        raise RuntimeError("Claude did not continue with original assistant marker output after answering")
    marker_name = case["claudeQuestion"]["markerFile"]
    if Path(marker_name).name != marker_name or not case["claudeQuestion"].get("markerAbsentBeforeSend") \
            or not case["claudeQuestion"].get("markerAbsentBeforeAnswer"):
        raise RuntimeError("Claude original marker absence or relative path not recorded")
    if local_path(case["claudeQuestion"]["worktreeRoot"]) != tree_path or not worktree_identity:
        raise RuntimeError("Claude configured worktree root is not original F registered worktree")
    marker = tree_path / marker_name
    if not marker.is_file() or marker.is_symlink() or not within(tree_path, marker.resolve(strict=True)):
        raise RuntimeError("Claude marker is not a new local file in the F worktree")
    marker_bytes = marker.read_bytes()
    if len(marker_bytes) > 4096 or json.loads(marker_bytes.decode("utf-8")) != case["claudeQuestion"]["marker"]:
        raise RuntimeError("Claude post-answer marker differs from prescribed JSON value")
    return {"state": "DIRECT_ORIGINAL_CLAUDE_QUESTION_ANSWER_CONTINUATION_REQUIRES_REVIEW",
            "cardId": card_ref["cardId"], "requestId": asking["request_id"],
            "toolUseId": card[4], "hostTurnId": send_id, "vendorSessionId": terminal["session_id"],
            "answerRequestId": answer_id, "answerStepId": answer_step,
            "questionSource": descriptor, "toolResultSourceCursor": resolved[0][3],
            "resultSourceCursor": result_cursor, "markerFile": marker_name,
            "markerSha256": hash_bytes(marker_bytes), "acceptance": False}

result = {"schema": "gogoke.37.private-m2-provider-boundaries.v1", "databaseWrites": False,
          "credentialReads": False, "acceptance": False, "filesBefore": file_facts(),
          "rootIdentity": [str(root.stat().st_dev), str(root.stat().st_ino)],
          "cases": [], "frames": [], "commands": [], "unknownFrames": [],
          "checks": {"V03b": "NOT_RUN", "V04b": "NOT_RUN", "V10": "NOT_RUN"}}
with sqlite3.connect(db_file.as_uri() + "?mode=ro&immutable=1", uri=True) as db:
    db.execute("PRAGMA query_only=ON")
    for case in cases:
        driver = case["driverId"]
        if driver == "antigravity" or case["state"] in (
                "NOT_RUN_NOT_LOGGED_IN", "NOT_RUN_NOT_SELECTED_FOR_THIS_ORIGINAL_CASE"):
            result["cases"].append({"caseId": case["caseId"], "driverId": driver,
                "state": case["state"], "checks": case["checks"], "source": "ORIGINAL_STATUS_ONLY_NO_SESSION"})
            continue
        direct_claude = driver == "claude" and case["state"] == \
            "CLAUDE_QUESTION_FLOW_DIRECT_READBACK_REQUIRED"
        if driver not in ("claude", "opencode", "grok") or not (direct_claude or
                case["state"] == "REAL_FIXED_CAPABILITY_PREFLIGHT_ONLY"):
            raise RuntimeError("Provider case did not finish its original H flow")
        domain, session_id = case["domainId"], case["sessionId"]
        original_session = next((row for row in journal["sessions"] if row["id"] == session_id), None)
        if original_session is None or original_session.get("caseOwner") != case["caseId"]:
            raise RuntimeError("Original provider session ID not bound to this case")
        instance = one(db,
            "SELECT driver_id,version,program_digest,home_ref,home_identity FROM gogoke_v37_instances "
            "WHERE instance_id=?", (case["instanceId"],))
        if instance[:3] != (driver, case["expectedVersion"], case["expectedDigest"]):
            raise RuntimeError("Actual F instance program identity differs from original case")
        seat = one(db,
            "SELECT instance_id,incarnation,state FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",
            (domain, case["seatId"]))
        if seat[0] != case["instanceId"] or seat[2] != "IDLE":
            raise RuntimeError("Original provider seat not released to Idle")
        worktree = one(db,
            "SELECT repository_id,domain_id,seat_id,instance_id,worktree_path,worktree_identity,state "
            "FROM gogoke_v37_worktrees WHERE worktree_id=?", (case["worktreeId"],))
        if worktree[:4] != (journal["repositoryId"], domain, case["seatId"], case["instanceId"]) \
                or worktree[6] != "REGISTERED":
            raise RuntimeError("Actual F worktree registration does not match provider seat")
        worktree_path = local_path(worktree[4])
        if not within(root, worktree_path):
            raise RuntimeError("Provider worktree is outside actual candidate root")
        worktree_stat = worktree_path.stat()
        for source_name, option_name in (("crossProjectGraph", "crossProject"),
                                         ("reviewSourceGraph", "reviewSource")):
            if source_name not in case:
                continue
            expected = next((row.get(option_name) for row in journal.get("providerBoundaryConfig", [])
                             if row.get("driverId") == driver), None)
            if not isinstance(expected, dict):
                raise RuntimeError("Original optional provider F binding config absent")
            target = one(db,
                "SELECT repository_id,domain_id,seat_id,instance_id,state "
                "FROM gogoke_v37_worktrees WHERE worktree_id=?", (expected["worktreeId"],))
            expected_instance = case["instanceId"] if option_name == "crossProject" else expected["instanceId"]
            if target != (expected.get("repositoryId", journal["repositoryId"]), domain,
                          expected["seatId"], expected_instance, "REGISTERED"):
                raise RuntimeError("Optional provider F binding changed since original preflight")
        registration = one(db,
            "SELECT purpose,seat_id FROM v37_ledger_session WHERE domain_id=? AND session_id=?",
            (domain, session_id))
        if registration != ("WORK", case["seatId"]):
            raise RuntimeError("Capability preflight must remain WORK, never a forged formal review")
        episodes = db.execute(
            "SELECT generation,process_operation_id,phase,stop_fact_id,instance_id,seat_id,seat_incarnation "
            "FROM gogoke_v37_h_process_episode WHERE domain_id=? AND session_id=? ORDER BY rowid",
            (domain, session_id)).fetchall()
        if len(episodes) != 1 or episodes[0][2] != "STOPPED" or not episodes[0][3] \
                or episodes[0][4:] != (case["instanceId"], case["seatId"], seat[1]):
            raise RuntimeError("Original provider H process episode was not stopped")
        custody = one(db,
            "SELECT state,binary_digest_sha256,profile_id,domain_id,generation,stop_proof_hash "
            "FROM gogoke_coordination_process_custody WHERE operation_id=?",
            (episodes[0][1],))
        if custody[0] != "STOPPED" or custody[1] != instance[2] or \
                custody[2:5] != (case["instanceId"], domain, episodes[0][0]) or not custody[5]:
            raise RuntimeError("Actual H physical custody does not match F fixed binary")
        claim = one(db,
            "SELECT state,instance_id FROM gogoke_v37_h_claim WHERE domain_id=? AND session_id=?",
            (domain, session_id))
        if claim != ("RELEASED", case["instanceId"]):
            raise RuntimeError("Provider H claim was not durably released")
        original_operations = {entry["request"]["requestId"]: entry for entry in journal["operations"]
                               if entry.get("request", {}).get("requestId")}
        for graph_name in ("graph", "crossProjectGraph", "reviewSourceGraph"):
            if graph_name not in case:
                continue
            graph_receipt = case[graph_name]
            originals = [entry for entry in journal["operations"]
                if entry.get("request", {}).get("family") == "K-WORKTREE" and
                entry["request"].get("operation") == "graph-query" and
                entry.get("receipt") == graph_receipt and
                isinstance(entry.get("rawFrame"), str) and
                json.loads(entry["rawFrame"]) == entry["request"]]
            if len(originals) != 1 or graph_receipt.get("status") != "APPLIED":
                raise RuntimeError("Original provider F graph-query receipt missing or ambiguous")
        for step in case["requestIds"]:
            entry = original_operations.get(step["requestId"])
            if not entry or entry["request"].get("targetId") != session_id or \
                    entry["request"].get("operation") != step["action"]:
                raise RuntimeError("Original User provider request identity absent")
            raw = entry.get("rawFrame")
            if not isinstance(raw, str) or json.loads(raw) != entry["request"]:
                raise RuntimeError("Exact User wire frame missing before dispatch")
            if step["action"] in ("admission-reserve", "admission-commit", "open", "stop", "admission-release"):
                saved = one(db,
                    "SELECT raw_hex,status FROM gogoke_v37_h_operation WHERE domain_id=? AND request_id=?",
                    (domain, step["requestId"]))
                if saved[0].lower() != raw.encode().hex() or saved[1] != "APPLIED":
                    raise RuntimeError("Original H provider request bytes or durable status differ")
        cap_step = next(row for row in case["requestIds"] if row["action"] == "capability-probe")
        cap_entry = original_operations[cap_step["requestId"]]
        cap = one(db,
            "SELECT request_bytes,receipt_bytes FROM v37_ledger_receipt "
            "WHERE family='K-SESSION' AND domain_id=? AND request_id=?",
            (domain, cap_step["requestId"]))
        if bytes(cap[0]) != cap_entry["rawFrame"].encode() or \
                json.loads(cap[1]) != case["capability"]:
            raise RuntimeError("Actual capability report is not the original H receipt")
        report = json.loads(cap[1])["result"]
        question_mode = "SOURCE_PRESENT_NATIVE_ASK_USER_BEHAVIOUR_NOT_RUN" if driver == "claude" \
            else "UNSUPPORTED_REPLY_ENCODER"
        if report.get("driverId") != driver or report.get("version") != instance[1] or \
                report.get("binaryDigest") != instance[2] or \
                report.get("capabilities", {}).get("nativeQuestionCard") != question_mode or \
                report.get("capabilities", {}).get("memoryOffLaunch") != "NOT_RUN":
            raise RuntimeError("Current provider capability boundary differs; update the case contract first")
        sends = db.execute("SELECT request_id FROM gogoke_v37_h_stdin_journal "
                           "WHERE domain_id=? AND session_id=? AND operation='send'",
                           (domain, session_id)).fetchall()
        if not direct_claude and sends:
            raise RuntimeError("Preflight unexpectedly sent a model prompt")
        if direct_claude and sends != [(case["sendRequestId"],)]:
            raise RuntimeError("Claude direct case must have exactly one original User send")
        raw_rows = db.execute(
            "SELECT generation,operation_id,source_epoch,source_cursor,raw_bytes,state,process_ticket,custodian_nonce "
            "FROM v37_ledger_raw_source WHERE domain_id=? AND session_id=? ORDER BY rowid",
            (domain, session_id)).fetchall()
        unknown = []
        for generation, operation, epoch, cursor, raw, state, ticket, nonce in raw_rows:
            frame = bytes(raw).decode("utf-8")
            result["frames"].append({"direction": "in", "caseId": case["caseId"],
                "sessionId": session_id, "generation": generation, "operationId": operation,
                "sourceEpoch": epoch, "sourceCursor": cursor, "processTicket": ticket,
                "custodianNonce": nonce, "state": state, "originalFrame": frame})
            if state == "PENDING":
                unknown.append({"sourceEpoch": epoch, "sourceCursor": cursor,
                                "method": json.loads(frame).get("method")})
        commands = db.execute(
            "SELECT generation,process_operation_id,step_id,phase,command_hex,source_epoch,source_cursor "
            "FROM gogoke_v37_rpc_steps WHERE domain_id=? AND session_id=? ORDER BY rowid",
            (domain, session_id)).fetchall()
        for generation, operation, step, phase, command, epoch, cursor in commands:
            result["commands"].append({"direction": "out", "caseId": case["caseId"],
                "sessionId": session_id, "generation": generation, "operationId": operation,
                "stepId": step, "phase": phase, "sourceEpoch": epoch, "sourceCursor": cursor,
                "originalFrame": bytes.fromhex(command).decode("utf-8"),
                "confirmedWrite": phase in ("WRITTEN", "OBSERVED")})
        normalized = db.execute(
            "SELECT i.cursor,i.source_epoch,i.source_cursor,i.update_json,r.operation_id,r.generation,"
            "r.process_ticket,r.custodian_nonce FROM v37_ledger_index i "
            "LEFT JOIN v37_ledger_raw_source r ON r.resolved_event_id=i.source_event_id "
            "AND r.domain_id=i.domain_id AND r.session_id=i.session_id "
            "WHERE i.source_kind='v37' AND i.domain_id=? AND i.session_id=? ORDER BY i.cursor",
            (domain, session_id)).fetchall()
        result["unknownFrames"].extend({"caseId": case["caseId"], **item} for item in unknown)
        direct = original_claude_question(db, case, journal, original_operations,
            raw_rows, worktree_path, worktree[5]) if direct_claude else None
        if direct is not None:
            result["checks"]["V03b"] = "DIRECT_CLAUDE_EVIDENCE_REQUIRES_INDEPENDENT_REVIEW"
        result["cases"].append({"caseId": case["caseId"], "driverId": driver,
            "sessionId": session_id,
            "state": "DIRECT_CLAUDE_QUESTION_REQUIRES_REVIEW" if direct_claude
                     else "DIRECT_FIXED_CAPABILITY_PREFLIGHT_ONLY",
            "actualVersion": instance[1], "actualDigest": instance[2],
            "worktreePath": str(worktree_path), "worktreeIdentity": worktree[5],
            "worktreeStat": {"observer": "python-stat", "device": str(worktree_stat.st_dev),
                             "inode": str(worktree_stat.st_ino)},
            "H": {"episode": episodes[0], "custody": custody, "claim": claim},
            "capability": report, "rawFrameCount": len(raw_rows),
            "normalized": [{"cursor": str(cursor), "sourceEpoch": epoch,
                "ledgerSourceCursor": source_cursor, "operationId": operation,
                "generation": generation, "processTicket": ticket,
                "custodianNonce": nonce, "update": json.loads(update)}
                for cursor, epoch, source_cursor, update, operation, generation, ticket, nonce in normalized],
            "missingNormalized": len(normalized) == 0,
            "checks": case["checks"], "unknownFrameCount": len(unknown),
            "directQuestion": direct})

result["memoryObserver"] = {"basis": "NOT_OBSERVED_AFTER_NORMAL_CLOSE", "V04b": "NOT_RUN"}
memory_refs = journal.get("snapshots", {})
if all(name in memory_refs for name in ("memory-before", "memory-after")):
    measured = {}
    for phase in ("before", "after"):
        reference = memory_refs[f"memory-{phase}"]
        if Path(reference["file"]).name != reference["file"] or \
                any(word in reference["file"].lower() for word in ("auth", "key", "token", "credential")):
            raise RuntimeError("Private memory snapshot reference is not a basename")
        measured[phase] = {"file": reference["file"], "sha256": reference["sha256"],
                           "contentsReadByProviderObserver": False}
    result["memoryObserver"] = {"basis": "EXISTING_SNAPSHOT_REFERENCE_ONLY_NOT_VENDOR_MEMORY_PROOF",
                                 "snapshots": measured, "V04b": "NOT_RUN"}

result["filesAfter"] = file_facts()
result["measurementPreservedDatabaseBytes"] = result["filesBefore"] == result["filesAfter"]
output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
if not result["measurementPreservedDatabaseBytes"]:
    raise RuntimeError("Readonly provider measurement changed candidate database bytes")
