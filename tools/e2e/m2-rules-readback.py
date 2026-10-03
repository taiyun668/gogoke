"""Original V08 A/H/policy readback, only after normal candidate close.

signed Python m2-rules-readback.py STATE_ROOT FRESH_PRIVATE_OUTPUT M2_JOURNAL before|final
No product launch, model invocation, credential read or database mutation.
"""
import hashlib
import json
import sqlite3
import sys
from contextlib import closing
from pathlib import Path


def check(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def original_owner_fingerprint(command, domain, raw):
    # Stored policy evidence uses length-prefixed UTF-8, not JSON reserialization.
    parts = [part.encode() for part in ("owner-policy", command, domain)] + [raw.encode()]
    return "sha256:" + digest(b"".join(len(part).to_bytes(8, "big") + part for part in parts))


def select(db, sql, args=()):
    return [dict(row) for row in db.execute(sql, args)]


def one(db, sql, args=()):
    found = select(db, sql, args)
    check(len(found) == 1, f"Expected one original row, found {len(found)}: {sql}")
    return found[0]


def policy(db, domain):
    return {name: select(db, f"SELECT * FROM gogoke_v37_seat_policy_{name} "
                        "WHERE domain_id=? ORDER BY rowid", (domain,))
            for name in ("head", "grants", "gates", "routes", "triggers", "escalations", "events")}


def typed_id(value):
    check(type(value) in (int, str), "Original RPC ID must retain number/string type")
    return type(value).__name__, value


def verify_case(db, journal, case, result):
    domain = journal["domainId"]
    reference = case["baselineReadback"]
    check(Path(reference["file"]).name == reference["file"], "Baseline must be a private artifact basename")
    before_bytes = (output.parent / reference["file"]).read_bytes()
    check(digest(before_bytes) == reference["sha256"], "Original baseline artifact bytes changed")
    before = json.loads(before_bytes)
    check(before["schema"] == result["schema"] and before["phase"] == "before" and
          before["caseId"] == journal["caseId"] and before["sourceCommit"] == journal["sourceCommit"] and
          before["domainId"] == domain and before["databasePath"] == result["databasePath"] and
          before["rootIdentity"] == result["rootIdentity"] and before["measurementPreservedDatabaseBytes"],
          "Original normally closed baseline is for another candidate/domain")
    check(case["ownership"] == {"lifecycle": "EXCLUSIVE_V08_SUBMITTER_AND_REVIEWER",
                                "policy": "EXCLUSIVE_V08_POLICY_DOMAIN"}, "Exclusive V08 ownership missing")
    check(case["domainId"] == domain and case["sourceCommit"] == journal["sourceCommit"], "Case byte identity differs")
    check(case["driverSha256"] == digest(Path(__file__).with_name("m2-rules.mjs").read_bytes()),
          "Actual loaded V08 module bytes differ from the reader's module")
    sessions = {s["id"]: s for s in journal["sessions"]}
    submitter, reviewer = (sessions[case[name]] for name in ("submitterSession", "reviewerSession"))
    check(submitter["id"] != reviewer["id"] and submitter["seatId"] != reviewer["seatId"] and
          submitter["worktreeId"] != reviewer["worktreeId"], "Separate actual test seats/worktrees required")
    head = before["policy"]["head"][0]
    check(case["fromStage"] == head["current_stage"] and case["toStage"] != head["current_stage"],
          "Legal stage must start at original policy head")
    reject, passed = case["rejectGate"], case["passGate"]
    reasons = case["rejectReasons"]
    operations = {row["request"].get("requestId"): row for row in journal["operations"]
                  if row["request"].get("requestId")}
    check(len(operations) == sum(bool(row["request"].get("requestId")) for row in journal["operations"]),
          "Original request IDs duplicated; mutating requests must be sent once")
    check(len(case["seatCards"]) == 2, "Original User K-SEAT cards required")
    for key, session in zip(case["seatCards"], (submitter, reviewer)):
        observed = operations[key]
        stored = one(db, "SELECT request_bytes,receipt_bytes FROM v37_ledger_receipt "
                     "WHERE family='K-SEAT' AND domain_id=? AND request_id=?", (domain, key))
        request, receipt = json.loads(bytes(stored["request_bytes"])), json.loads(bytes(stored["receipt_bytes"]))
        check(bytes(stored["request_bytes"]).decode() == observed["rawFrame"] and request == observed["request"] and
              receipt == observed["receipt"] and request["operation"] == "state-card" and
              request["targetId"] == session["seatId"] and receipt["status"] == "APPLIED" and
              receipt["result"]["state"] == "BUSY" and receipt["result"]["instanceId"] == session["instanceId"] and
              str(receipt["result"]["generation"]) == session["generation"], "Stored User E card/current H identity differs")
        result["seatCards"].append({"rawFrame": observed["rawFrame"], "originalReceipt": bytes(stored["receipt_bytes"]).decode()})
    expected_events = {row["event_id"]: row for row in before["policy"]["events"]}
    revision = head["revision"]
    configurations = [operations[key] for key in case["ownerConfigurations"]]
    check(len(configurations) == 5, "Five original Owner configuration operations required")
    expected_config = [
        ("policy-call-grant", {"callerSeatId": submitter["seatId"], "targetId": reviewer["seatId"], "action": "REVIEW", "expiresAtMs": None}),
        *[("policy-gate", {"gateId": gate, "submitterSeatId": submitter["seatId"], "reviewerSeatId": reviewer["seatId"],
                          "fromStage": case["fromStage"], "toStage": case["toStage"], "rejectCap": 2}) for gate in (reject, passed)],
        ("policy-call-grant", {"callerSeatId": submitter["seatId"], "targetId": reviewer["seatId"], "action": "REVIEW", "expiresAtMs": "1"}),
        ("policy-call-grant", {"callerSeatId": submitter["seatId"], "targetId": reviewer["seatId"], "action": "REVIEW", "expiresAtMs": None}),
    ]
    for entry, (command, fields) in zip(configurations, expected_config):
        request, receipt, raw = entry["request"], entry["receipt"], entry["rawFrame"]
        check(json.loads(raw) == request and request == {"schema": "gogoke.37.owner-configuration.v1",
              "command": command, "domainId": domain, "requestId": request["requestId"],
              **fields, "expectedRevision": str(revision)}, "Exact original Owner configuration bytes/fields differ")
        revision += 1
        check(json.loads(entry["rawReceipt"]) == receipt and receipt["status"] == "APPLIED" and
              receipt["requestId"] == request["requestId"] and receipt["revision"] == str(revision), "Owner CAS receipt differs")
        event = one(db, "SELECT * FROM gogoke_v37_seat_policy_events WHERE domain_id=? AND event_id=?",
                    (domain, request["requestId"]))
        check(event["operation"] == command and event["target_id"] == domain and event["state"] == "APPLIED" and
              event["policy_revision"] == revision and event["detail"] == "" and
              event["fingerprint"] == original_owner_fingerprint(command, domain, raw),
              "Stored original Owner policy event/fingerprint differs")
        expected_events[event["event_id"]] = event
        result["ownerFrames"].append({"rawFrame": raw, "rawReceipt": entry["rawReceipt"], "storedEvent": event})

    initial_revision = head["revision"] + 3
    cases = [
        ("V08_SUBMIT_REJECT_GATE", submitter, "gate-submit", reject, "1", {}, "APPLIED", "SUBMITTED", "", initial_revision),
        ("V08_REJECT_WITH_REASON", reviewer, "gate-decide", reject, "2", {"decision": "REJECT", "reason": reasons[0]}, "APPLIED", "REJECTED", reasons[0], initial_revision),
        ("V08_MODEL_BYPASS_GATE", submitter, "stage-transition", reject, "3", {}, "DENIED", None, "", initial_revision),
        ("V08_RESUBMIT", submitter, "gate-submit", reject, "3", {}, "APPLIED", "SUBMITTED", "", initial_revision),
        ("V08_REJECT_CAP_STATE_ONLY", reviewer, "gate-decide", reject, "4", {"decision": "REJECT", "reason": reasons[1]}, "APPLIED", "ESCALATION_REQUIRED", reasons[1], initial_revision),
        ("V08_MODEL_CAP_BLOCKS_SUBMIT", submitter, "gate-submit", reject, "5", {}, "DENIED", None, "", initial_revision),
        ("V08_MODEL_EXPIRED_GRANT", submitter, "gate-submit", passed, "1", {}, "DENIED", None, "", initial_revision + 1),
        ("V08_SUBMIT_PASS_GATE", submitter, "gate-submit", passed, "1", {}, "APPLIED", "SUBMITTED", "", initial_revision + 2),
        ("V08_MODEL_WRONG_REVIEWER", submitter, "gate-decide", passed, "2", {"decision": "PASS"}, "DENIED", None, "", initial_revision + 2),
        ("V08_MODEL_EMPTY_REJECT_REASON", reviewer, "gate-decide", passed, "2", {"decision": "REJECT", "reason": ""}, "DENIED", None, "", initial_revision + 2),
        ("V08_APPROVE", reviewer, "gate-decide", passed, "2", {"decision": "PASS"}, "APPLIED", "PASSED", "", initial_revision + 2),
        ("V08_LEGAL_STAGE", submitter, "stage-transition", passed, "3", {}, "APPLIED", "ADVANCED", case["toStage"], initial_revision + 3),
    ]
    check(len(case["actions"]) == len(cases), "Incomplete or additional model actions")
    checked_sessions = set()
    for action, (key, session, operation, gate, gate_rev, payload, status, state, reason, policy_rev) in zip(case["actions"], cases):
        arguments = {"operation": operation, "targetId": gate, "expectedRevision": gate_rev, "payload": payload}
        check(action["caseId"] == key and action["sessionId"] == session["id"] and action["arguments"] == arguments,
              "Model action identity/arguments differ")
        sent = operations[action["sendRequestId"]]
        request = sent["request"]
        check(json.loads(sent["rawFrame"]) == request and request["family"] == "K-SESSION" and
              request["operation"] == "send" and request["domainId"] == domain and
              request["targetId"] == session["id"] and request["payload"] ==
              {"generation": session["generation"], "body": action["askBytes"]}, "Original User ask bytes/target differ")
        stdin = one(db, "SELECT * FROM gogoke_v37_h_stdin_journal WHERE domain_id=? AND request_id=?",
                    (domain, request["requestId"]))
        check(stdin["phase"] == "RECEIPTED" and stdin["receipt_status"] == "APPLIED" and
              bytes.fromhex(stdin["request_hex"]).decode() == sent["rawFrame"], "Original H stdin was not receipted")
        send_ack = json.loads(bytes.fromhex(stdin["receipt_hex"]))
        check(send_ack == sent["receipt"] == action["sendReceipt"] and
              send_ack["result"]["createdTurn"] is True and send_ack["result"]["turnId"] == action["turnId"],
              "Model authority lacks original H User turn authorization")
        episode = one(db, "SELECT * FROM gogoke_v37_h_process_episode WHERE domain_id=? AND session_id=? AND process_operation_id=?",
                      (domain, session["id"], stdin["process_operation_id"]))
        check(episode["seat_id"] == session["seatId"] and episode["instance_id"] == session["instanceId"] and
              str(episode["generation"]) == session["generation"] and episode["phase"] == "STOPPED" and
              episode["stop_fact_id"], "Original physical model episode/stop facts differ")
        check(stdin["session_id"] == session["id"] and stdin["generation"] == session["generation"] and
              stdin["operation"] == "send", "H stdin belongs to another generation/session")
        if session["id"] not in checked_sessions:
            pin = one(db, "SELECT i.driver_id,i.version,i.program_digest,c.binary_digest_sha256 "
                      ",c.domain_id,c.generation,c.ticket,c.custodian_nonce,c.state,c.stop_proof_hash "
                      "FROM gogoke_v37_instances i JOIN gogoke_coordination_process_custody c "
                      "ON c.operation_id=? WHERE i.instance_id=?", (stdin["process_operation_id"], session["instanceId"]))
            check(pin["driver_id"] == "codex" and pin["program_digest"] == pin["binary_digest_sha256"] and
                  pin["domain_id"] == domain and pin["generation"] == session["generation"] and
                  pin["ticket"] == stdin["ticket"] and pin["custodian_nonce"] == stdin["custodian_nonce"] and
                  pin["state"] == "STOPPED" and pin["stop_proof_hash"] == episode["stop_fact_id"],
                  "Actual Codex CLI pin differs from physical custody")
            result["sessions"].append({"sessionId": session["id"], "episode": episode, "pin": pin})
            checked_sessions.add(session["id"])
        incoming = select(db, "SELECT * FROM v37_ledger_raw_source WHERE domain_id=? AND session_id=? AND operation_id=? "
                          "AND process_ticket=? AND custodian_nonce=? AND generation=? ORDER BY rowid",
                          (domain, session["id"], stdin["process_operation_id"], stdin["ticket"], stdin["custodian_nonce"], stdin["generation"]))
        decoded = [(row, json.loads(bytes(row["raw_bytes"]))) for row in incoming]
        calls = [(row, frame) for row, frame in decoded if frame.get("method") == "item/tool/call" and
                 frame.get("params", {}).get("turnId") == action["turnId"]]
        check(len(calls) == 1, "Exactly one original model tool call is required, without retry/substitute")
        source, frame = calls[0]
        check(frame["params"]["threadId"] == session["threadId"] and frame["params"]["tool"] == "gogoke_policy" and
              frame["params"]["arguments"] == arguments and source["state"] == "NO_EVENT" and
              source["no_event_reason"] == "NATIVE_HOST_TOOL_REPLY_WRITTEN", "Original A tool/caller/arguments not the intended call")
        replies = select(db, "SELECT * FROM gogoke_v37_rpc_steps WHERE domain_id=? AND session_id=? "
                         "AND process_operation_id=? AND ticket=? AND custodian_nonce=? AND generation=?",
                         (domain, session["id"], stdin["process_operation_id"], stdin["ticket"], stdin["custodian_nonce"], stdin["generation"]))
        starts = [(step, json.loads(bytes.fromhex(step["command_hex"]))) for step in replies]
        starts = [(step, command) for step, command in starts if command.get("method") == "turn/start" and
                  command.get("params", {}).get("threadId") == session["threadId"] and
                  command.get("params", {}).get("input") == [{"type": "text", "text": action["askBytes"]}]]
        check(len(starts) == 1 and starts[0][0]["phase"] == "OBSERVED", "Actual H turn/start did not carry original Owner ask")
        start_step, start_command = starts[0]
        acknowledgements = [(row, value) for row, value in decoded if "method" not in value and "id" in value and
                            typed_id(value["id"]) == typed_id(start_command["id"])]
        check(len(acknowledgements) == 1 and acknowledgements[0][0]["state"] == "NO_EVENT" and
              acknowledgements[0][0]["no_event_reason"] == "CODEX_RPC_RESPONSE" and
              acknowledgements[0][0]["source_epoch"] == start_step["source_epoch"] and
              acknowledgements[0][0]["source_cursor"] == start_step["source_cursor"] and
              acknowledgements[0][1]["result"]["turn"]["id"] == action["turnId"] and
              acknowledgements[0][1]["result"]["turn"]["status"] == "inProgress",
              "Original CLI turn ACK does not prove this exact H User ask")
        reply_matches = []
        for reply in replies:
            written = json.loads(bytes.fromhex(reply["command_hex"]))
            if "method" not in written and "id" in written and typed_id(written["id"]) == typed_id(frame["id"]):
                reply_matches.append((reply, written))
        check(len(reply_matches) == 1, "Original typed model RPC ID has no unique H response")
        reply, written = reply_matches[0]
        check(reply["phase"] in ("WRITTEN", "OBSERVED"), "H tool reply not written for the original A source")
        content = written["result"]["contentItems"]
        check(len(content) == 1 and content[0]["type"] == "inputText", "Original H tool response shape differs")
        receipt = json.loads(content[0]["text"])
        check(content[0]["text"] == action["rawToolReceipt"] and receipt == action["receipt"] and
              reply["step_id"] == receipt["requestId"] and written["result"]["success"] == (status == "APPLIED") and
              receipt["schema"] == "gogoke.37.operations.v1" and receipt["family"] == "K-POLICY" and
              receipt["operation"] == operation and receipt["targetId"] == gate and receipt["status"] == status and
              receipt["previousRevision"] == gate_rev and receipt["revision"] == str(int(gate_rev) + (status == "APPLIED")),
              "Actual H policy receipt differs from observation/required outcome")
        tool_completions = [(row, value) for row, value in decoded if value.get("method") == "item/completed" and
                            value.get("params", {}).get("turnId") == action["turnId"] and
                            value.get("params", {}).get("item", {}).get("type") in
                            ("dynamicToolCall", "commandExecution", "fileChange", "mcpToolCall")]
        check(len(tool_completions) == 1 and tool_completions[0][1]["params"]["threadId"] == session["threadId"] and
              tool_completions[0][1]["params"]["item"]["type"] == "dynamicToolCall" and
              tool_completions[0][1]["params"]["item"]["id"] == frame["params"]["callId"] and
              tool_completions[0][1]["params"]["item"]["tool"] == "gogoke_policy" and
              tool_completions[0][1]["params"]["item"]["arguments"] == arguments,
              "Original CLI tool completion includes a substitute/additional tool")
        events = select(db, "SELECT * FROM gogoke_v37_seat_policy_events WHERE domain_id=? AND event_id=?",
                        (domain, receipt["requestId"]))
        if status == "APPLIED":
            check(len(events) == 1 and events[0]["operation"] == operation and events[0]["target_id"] == gate and
                  events[0]["state"] == state and events[0]["detail"] == reason and events[0]["policy_revision"] == policy_rev and
                  receipt["result"] == {"state": state, "reason": reason, "policyRevision": str(policy_rev)},
                  "Committed actual gate/stage event differs")
            expected_events[events[0]["event_id"]] = events[0]
        else:
            check(not events, "Denied model call committed a policy event")
        completions = [(row, value) for row, value in decoded if value.get("method") == "turn/completed" and
                       value.get("params", {}).get("threadId") == session["threadId"] and
                       value.get("params", {}).get("turn", {}).get("id") == action["turnId"]]
        check(len(completions) == 1 and completions[0][1]["params"]["turn"]["status"] == "completed" and
              completions[0][0]["state"] != "PENDING", "Original CLI completion missing/unresolved")
        result["modelCalls"].append({"caseId": key, "sessionId": session["id"],
            "originalAsk": sent["rawFrame"], "originalSendReceipt": bytes.fromhex(stdin["receipt_hex"]).decode(),
            "originalTurnStart": bytes.fromhex(start_step["command_hex"]).decode(),
            "originalTurnAck": bytes(acknowledgements[0][0]["raw_bytes"]).decode(),
            "originalAFrame": bytes(source["raw_bytes"]).decode(), "originalHReply": bytes.fromhex(reply["command_hex"]).decode(),
            "sourceEpoch": source["source_epoch"], "sourceCursor": source["source_cursor"],
            "processOperationId": stdin["process_operation_id"], "processTicket": stdin["ticket"],
            "custodianNonce": stdin["custodian_nonce"], "policyEvents": events,
            "originalToolCompletion": bytes(tool_completions[0][0]["raw_bytes"]).decode(),
            "originalCompletion": bytes(completions[0][0]["raw_bytes"]).decode()})

    current = result["policy"]
    check({row["event_id"]: row for row in current["events"]} == expected_events,
          "Unaccounted rule event or changed baseline event in exclusive domain")
    check(current["head"] == [{**head, "revision": head["revision"] + 6, "current_stage": case["toStage"]}],
          "Actual final legal stage/head revision differs")
    original_gates = {row["gate_id"]: row for row in before["policy"]["gates"]}
    for gate, state, gate_revision, count, reason in [(reject, "ESCALATION_REQUIRED", 5, 2, reasons[1]),
                                                    (passed, "ADVANCED", 4, 0, "")]:
        original_gates[gate] = {"domain_id": domain, "gate_id": gate, "submitter_seat_id": submitter["seatId"],
                              "reviewer_seat_id": reviewer["seatId"], "from_stage": case["fromStage"],
                              "to_stage": case["toStage"], "reject_cap": 2, "reject_count": count,
                              "state": state, "reason": reason, "revision": gate_revision}
    check({row["gate_id"]: row for row in current["gates"]} == original_gates, "Actual gates changed beyond the prescribed scope")
    grants = {(row["caller_seat_id"], row["target_id"], row["action"]): row for row in before["policy"]["grants"]}
    grants[(submitter["seatId"], reviewer["seatId"], "REVIEW")] = {"domain_id": domain, "caller_seat_id": submitter["seatId"],
        "target_id": reviewer["seatId"], "action": "REVIEW", "expires_at_ms": 0, "revision": head["revision"] + 5}
    check({(row["caller_seat_id"], row["target_id"], row["action"]): row for row in current["grants"]} == grants,
          "Actual final grant differs or unrelated grant changed")
    for name in ("routes", "triggers", "escalations"):
        check(current[name] == before["policy"][name], "No escalation delivery may be inferred from gate state")
    check(len(case["userBoundaries"]) == 1, "User boundary observation missing")
    boundary = case["userBoundaries"][0]
    user = operations[boundary["requestId"]]
    check(json.loads(user["rawFrame"]) == user["request"] and user["request"]["family"] == "K-POLICY" and
          user["receipt"]["status"] == "UNSUPPORTED" and user["receipt"]["revision"] == "1" and
          boundary["authority"] == "REAL_USER_INGRESS_ONLY", "User control cannot stand for model rejection")
    result["userBoundaries"].append({"rawFrame": user["rawFrame"], "receipt": user["receipt"],
                                    "persistedNativeDenialRow": False, "authority": boundary["authority"]})
    result["notRun"] = case["notRun"]
    check(len(result["notRun"]) == 5, "Unimplemented V08 caller/escalation boundaries must remain explicit")
    result["verifiedCaseId"] = case["caseId"]
    result["directCaseEvidence"] = True
    result["state"] = "IMPLEMENTED_CASES_HAVE_DIRECT_EVIDENCE_V08_INCOMPLETE"


check(len(sys.argv) == 5 and sys.argv[4] in ("before", "final"), "Expected state root, fresh output, original M2 journal, before|final")
root = Path(sys.argv[1]).resolve(strict=True)
output = Path(sys.argv[2])
check(not output.exists(), "Private output already exists; original evidence must not be overwritten")
journal = json.loads(Path(sys.argv[3]).read_text(encoding="utf-8-sig"))
check(journal.get("schema") == "gogoke.37.m2-win11-e2e.v1", "Requires the original actual-product M2 journal")
last_launch = journal.get("launches", [])[-1] if journal.get("launches") else {}
last_close = journal.get("closes", [])[-1] if journal.get("closes") else {}
check(last_launch.get("pid") and last_launch["pid"] == journal.get("currentEndpoint", {}).get("pid") ==
      last_close.get("pid") and last_close.get("exitCode") == 0 and last_close.get("forceKill") is False and
      last_launch.get("sourceCommit") == journal["sourceCommit"], "Latest actual candidate must have its normal-close receipt")
database = root / "state.sqlite"
wal = Path(str(database) + "-wal")
check(database.is_file() and (not wal.exists() or wal.stat().st_size == 0), "Normal candidate close/checkpoint required")


def files():
    return {p.name: {"length": p.stat().st_size, "sha256": digest(p.read_bytes())}
            for p in (database, wal, Path(str(database) + "-shm")) if p.exists()}


result = {"schema": "gogoke.37.private-m2-rules-readback.v1", "phase": sys.argv[4],
          "caseId": journal["caseId"], "sourceCommit": journal["sourceCommit"], "domainId": journal["domainId"],
          "databasePath": str(database), "rootIdentity": {"observer": "python-stat", "device": str(root.stat().st_dev), "inode": str(root.stat().st_ino)},
          "databaseWrites": False, "credentialReads": False, "filesBefore": files(), "acceptance": False,
          "readerSha256": digest(Path(__file__).read_bytes()),
          "normalClose": last_close,
          "directCaseEvidence": False, "ownerFrames": [], "seatCards": [], "modelCalls": [], "sessions": [], "userBoundaries": []}
try:
    with closing(sqlite3.connect(database.as_uri() + "?mode=ro&immutable=1", uri=True)) as db:
        db.row_factory = sqlite3.Row
        db.execute("PRAGMA query_only=ON")
        result["policy"] = policy(db, journal["domainId"])
        check(len(result["policy"]["head"]) == 1, "Existing real Owner-initialized policy head required")
        if sys.argv[4] == "final":
            check(len(journal.get("rulesCases", [])) == 1, "One exclusive original V08 case required")
            verify_case(db, journal, journal["rulesCases"][0], result)
        else:
            result["state"] = "BASELINE_ONLY_NOT_A_V08_RESULT"
except Exception as error:
    result["state"] = "FAILED_OR_NOT_RUN_PRESERVE_ORIGINAL"
    result["directCaseEvidence"] = False
    result["originalError"] = repr(error)
    raise
finally:
    result["filesAfter"] = files()
    result["measurementPreservedDatabaseBytes"] = result["filesBefore"] == result["filesAfter"]
    if not result["measurementPreservedDatabaseBytes"]:
        result["directCaseEvidence"] = False
        result["state"] = "FAILED_MEASUREMENT_CHANGED_DATABASE_BYTES"
    with output.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(result, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
check(result["measurementPreservedDatabaseBytes"], "Immutable measurement changed database bytes")
