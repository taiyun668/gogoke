"""Original V08 A/H/policy readback, only after normal candidate close.

signed Python m2-rules-readback.py STATE_ROOT FRESH_PRIVATE_OUTPUT M2_JOURNAL before|checkpoint|final
No product launch, model invocation, credential read or database mutation.
"""
import hashlib
import json
import re
import sqlite3
import sys
from contextlib import closing
from pathlib import Path


def check(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def native_fingerprint(fields, raw):
    # Stored evidence hashes length-prefixed original bytes, not reserialized JSON.
    parts = [part.encode() for part in fields] + [raw.encode() if isinstance(raw, str) else raw]
    return "sha256:" + digest(b"".join(len(part).to_bytes(8, "big") + part for part in parts))


def original_owner_fingerprint(command, domain, raw):
    return native_fingerprint(("owner-policy", command, domain), raw)


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


def foreign_snapshot(db, domain, configuration):
    if configuration is None:
        return None
    check(isinstance(configuration, dict) and set(configuration) == {"domainId", "gateId", "ownerGate"} and
          all(isinstance(configuration.get(key), str) and
              re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}", configuration[key])
              for key in ("domainId", "gateId")) and configuration["domainId"] != domain,
          "Foreign fixture needs exact distinct atomic domain/gate selections")
    foreign_domain, gate_id = configuration["domainId"], configuration["gateId"]
    owner = configuration["ownerGate"]
    check(isinstance(owner, dict) and set(owner) == {"rawFrame", "rawReceipt"} and
          all(isinstance(owner[key], str) for key in owner), "Original NativeUser Owner gate bytes required")
    request, receipt = json.loads(owner["rawFrame"]), json.loads(owner["rawReceipt"])
    check(set(request) == {"schema", "command", "domainId", "requestId", "gateId", "submitterSeatId",
                           "reviewerSeatId", "fromStage", "toStage", "rejectCap", "expectedRevision"} and
          request["schema"] == receipt["schema"] == "gogoke.37.owner-configuration.v1" and
          request["command"] == receipt["command"] == "policy-gate" and
          request["domainId"] == foreign_domain and request["gateId"] == gate_id and
          receipt["requestId"] == request["requestId"] and receipt["status"] == "APPLIED" and
          re.fullmatch(r"[1-9][0-9]*", request["expectedRevision"]) and
          receipt["revision"] == str(int(request["expectedRevision"]) + 1),
          "Foreign gate lacks its original NativeUser Owner CAS request/receipt")
    current = policy(db, foreign_domain)
    gates = [row for row in current["gates"] if row["gate_id"] == gate_id]
    events = [row for row in current["events"] if row["event_id"] == request["requestId"]]
    check(len(current["head"]) == len(gates) == len(events) == 1 and
          not select(db, "SELECT gate_id FROM gogoke_v37_seat_policy_gates WHERE domain_id=? AND gate_id=?",
                     (domain, gate_id)), "Actual foreign gate/head/event must exist only in B")
    gate, event = gates[0], events[0]
    check(gate == {"domain_id": foreign_domain, "gate_id": gate_id,
                   "submitter_seat_id": request["submitterSeatId"], "reviewer_seat_id": request["reviewerSeatId"],
                   "from_stage": request["fromStage"], "to_stage": request["toStage"],
                   "reject_cap": request["rejectCap"], "reject_count": 0, "state": "READY", "reason": "", "revision": 1} and
          current["head"][0]["revision"] >= int(receipt["revision"]) and
          event["domain_id"] == event["target_id"] == foreign_domain and
          event["operation"] == "policy-gate" and event["state"] == "APPLIED" and event["detail"] == "" and
          event["policy_revision"] == int(receipt["revision"]) and
          event["fingerprint"] == original_owner_fingerprint("policy-gate", foreign_domain, owner["rawFrame"]),
          "B gate is not the original NativeUser-created READY object with its bound Owner event")
    return {"configuration": configuration, "gate": gate, "ownerEvent": event, "policy": current}


def typed_id(value):
    check(type(value) in (int, str), "Original RPC ID must retain number/string type")
    return type(value).__name__, value


def turn_activity(decoded, thread, turn, allow_question=False, expected_input=None):
    items, calls, questions = [], [], []
    input_echoes = []
    ordinary = {"item/agentMessage/delta", "item/reasoning/summaryTextDelta",
                "item/reasoning/textDelta", "turn/started", "turn/completed",
                "thread/tokenUsage/updated", "thread/status/changed"}
    for row, frame in decoded:
        params = frame.get("params")
        if not isinstance(params, dict) or params.get("turnId") != turn:
            continue
        check(params.get("threadId") == thread, "Original turn has another thread identity")
        method = frame.get("method")
        if method in ("item/started", "item/completed"):
            item = params.get("item")
            if isinstance(item, dict) and item.get("type") == "userMessage":
                check(expected_input is not None and
                      item.get("content") == [{"type": "text", "text": expected_input}],
                      "Original User echo is not the exact authorized ask")
                input_echoes.append((method, typed_id(item.get("id"))))
                continue
            check(isinstance(item, dict) and item.get("type") in
                  ("agentMessage", "reasoning", "contextCompaction", "dynamicToolCall"),
                  "Original turn has unexplained item/tool activity")
            if item["type"] == "dynamicToolCall":
                items.append((row, frame))
        elif method == "item/tool/call":
            calls.append((row, frame))
        elif method == "item/tool/requestUserInput" and allow_question:
            questions.append((row, frame))
        else:
            check(method in ordinary, "Original turn has unexplained item/tool activity")
    if expected_input is not None:
        check(len(input_echoes) == 2 and input_echoes[0][0] == "item/started" and
              input_echoes[1][0] == "item/completed" and input_echoes[0][1] == input_echoes[1][1],
              "Original User ask lacks its unique matched started/completed echo")
    return items, calls, questions


def inbox_rows(db, domain):
    return {name: select(db, f"SELECT * FROM gogoke_v37_inbox_{name} WHERE domain_id=? ORDER BY rowid", (domain,))
            for name in ("messages", "operations")}


def host_snapshot(db, domain, host):
    cause = one(db, "SELECT * FROM gogoke_v37_seat_policy_events WHERE domain_id=? AND event_id=?",
                (domain, host["causeEventId"]))
    check(cause["operation"] == "gate-decide" and cause["target_id"] == host["gateId"] and
          cause["state"] == "ESCALATION_REQUIRED" and cause["detail"] == host["reason"] and
          cause["policy_revision"] == int(host["policyRevision"]), "Original native Host cause differs")
    matched = []
    for operation in inbox_rows(db, domain)["operations"]:
        raw = bytes.fromhex(operation["request_hex"])
        frame = json.loads(raw)
        if frame.get("schema") == "gogoke.37.host-rule-inbox.v1" and frame.get("causeEventId") == cause["event_id"]:
            matched.append((operation, frame))
    enqueues = [(op, frame) for op, frame in matched if frame["operation"] == "enqueue"]
    check(len(enqueues) == 1, "One original Host enqueue is required per actual native cause")
    enqueue, frame = enqueues[0]
    check(set(frame) == {"schema", "actor", "operation", "domainId", "messageId", "escalationRequestId",
          "triggerId", "causeEventId", "sourceSeatId", "destinationSeatId", "policyRevision", "routeRevision", "body"} and
          frame["actor"] == "HOST_RULE" and frame["domainId"] == domain and
          frame["sourceSeatId"] == host["sourceSeatId"] and frame["destinationSeatId"] == host["destination"]["seatId"] and
          frame["policyRevision"] == host["policyRevision"] and frame["routeRevision"] == host["routeRevision"] and
          enqueue["phase"] == "APPLIED" and enqueue["previous_revision"] == "0" and enqueue["revision"] == "1" and
          enqueue["result_state"] == "PENDING" and frame["messageId"] == enqueue["message_id"],
          "Original C actor/cause/Owner route is not this mechanical Host request")
    message = one(db, "SELECT * FROM gogoke_v37_inbox_messages WHERE domain_id=? AND message_id=?",
                  (domain, frame["messageId"]))
    check(message["sender_seat_id"] == "HOST_RULE" and message["seat_id"] == frame["destinationSeatId"] and
          message["body"] == frame["body"] and host["reason"] not in message["body"],
          "C body/destination changed or raw model reason became the Host command")
    intent = one(db, "SELECT * FROM gogoke_v37_seat_policy_escalations WHERE domain_id=? AND trigger_id=?",
                 (domain, frame["triggerId"]))
    event = one(db, "SELECT * FROM gogoke_v37_seat_policy_events WHERE domain_id=? AND event_id=?",
                (domain, frame["escalationRequestId"]))
    check(intent["request_id"] == event["event_id"] == frame["escalationRequestId"] and
          intent["from_seat_id"] == frame["sourceSeatId"] and intent["to_seat_id"] == frame["destinationSeatId"] and
          intent["reason"] == "REJECT_CAP" and event["operation"] == "escalate" and
          event["target_id"] == intent["trigger_id"] and event["policy_revision"] == int(frame["policyRevision"]) and
          event["state"] == "INTENT" and event["detail"] == cause["event_id"], "C source does not bind original E INTENT")
    source = one(db, "SELECT incarnation,layer,parent_seat_id FROM gogoke_v37_seat_operations "
                 "WHERE domain_id=? AND seat_id=? AND revision=1 AND generation=1", (domain, host["sourceSeatId"]))
    current = one(db, "SELECT incarnation,layer,parent_seat_id FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",
                  (domain, host["sourceSeatId"]))
    check(source == current and not (source["layer"] == "LEAD" and frame["destinationSeatId"] == "OWNER"),
          "Original immutable logical source identity/Owner boundary differs")
    recipient_id = enqueue["request_id"].replace("hostenqueue-", "hostrecipient-", 1)
    recipe_rows = select(db, "SELECT * FROM gogoke_v37_inbox_operations WHERE domain_id=? AND request_id=?",
                         (domain, recipient_id))
    check(len(recipe_rows) <= 1, "More than one frozen Host recipient choice")
    recipe = None
    stages = []
    if recipe_rows:
        frozen = recipe_rows[0]
        recipe = json.loads(bytes.fromhex(frozen["request_hex"]))
        check(frozen["message_id"] == message["message_id"] and frozen["phase"] == "PREPARED" and
              frozen["result_state"] == "PENDING" and frozen["previous_revision"] == frozen["revision"] == "1" and
              set(recipe) == {"schema", "actor", "domainId", "messageId", "escalationRequestId",
                              "triggerId", "causeEventId", "destinationSeatId", "routeRevision", "mode",
                              "sessionId", "seatIncarnation", "instanceId", "permissionTier",
                              "repositoryId", "worktreeId", "generation", "reserveRequestId",
                              "commitRequestId", "startRequestId"} and
              recipe["schema"] == "gogoke.37.host-recipient.v1" and recipe["actor"] == "HOST_RULE" and
              recipe["domainId"] == domain and recipe["messageId"] == message["message_id"] and
              recipe["causeEventId"] == cause["event_id"] and recipe["triggerId"] == intent["trigger_id"] and
              recipe["escalationRequestId"] == intent["request_id"] and
              recipe["destinationSeatId"] == host["destination"]["seatId"] and
              recipe["routeRevision"] == host["routeRevision"] and recipe["mode"] == "FRESH" and
              recipe["repositoryId"] == "gogokeSeatTestbed" and
              recipe["worktreeId"] == host["destination"]["worktreeId"] and
              recipe["instanceId"] == host["destination"]["instanceId"] and
              recipe["reserveRequestId"] == recipient_id + "-reserve" and
              recipe["commitRequestId"] == recipient_id + "-commit" and
              recipe["startRequestId"] == recipient_id + "-start" and
              recipe["sessionId"] == recipient_id.replace("hostrecipient-", "hostsession-", 1) and
              str(recipe["generation"]).isdecimal(),
              "Frozen C recipe is not the exact Host-selected F/H recipient")
        for stage, key in (("reserve", "reserveRequestId"), ("commit", "commitRequestId"), ("start", "startRequestId")):
            rows = select(db, "SELECT * FROM gogoke_v37_inbox_operations WHERE domain_id=? AND request_id=?",
                          (domain, recipe[key]))
            check(len(rows) <= 1, "Duplicate frozen Host stage")
            if rows:
                row = rows[0]
                request = json.loads(bytes.fromhex(row["request_hex"]))
                check(row["message_id"] == message["message_id"] and row["phase"] == "PREPARED" and
                      row["result_state"] in ("APPLIED", "REPLAYED") and not row["reason"] and
                      request["schema"] == "gogoke.37.operations.v1" and
                      request["family"] == "K-SESSION" and request["domainId"] == domain and
                      request["targetId"] == recipe["sessionId"] and request["requestId"] == recipe[key] and
                      request["operation"] == {"reserve": "admission-reserve", "commit": "admission-commit", "start": "open"}[stage] and
                      request["payload"] == ({"seatId": recipe["destinationSeatId"], "generation": recipe["generation"]}
                          if stage != "start" else {"seatId": recipe["destinationSeatId"], "generation": recipe["generation"],
                                                     "repositoryId": recipe["repositoryId"], "worktreeId": recipe["worktreeId"]}),
                      "Frozen C stage differs from the exact Host H request")
                stages.append({"stage": stage, "operation": row, "request": request})
    deliveries = [{"operation": op, "request": raw} for op, raw in matched if raw["operation"] == "deliver"]
    check(len(matched) == 1 + len(deliveries), "Unexpected Host C operation for the cause")
    sends = []
    for row in select(db, "SELECT * FROM gogoke_v37_h_stdin_journal WHERE domain_id=? AND operation='send'", (domain,)):
        request = json.loads(bytes.fromhex(row["request_hex"]))
        if request.get("payload", {}).get("body") == message["body"]:
            sends.append(row)
    commands = []
    for row in select(db, "SELECT * FROM gogoke_v37_rpc_steps WHERE domain_id=?", (domain,)):
        command = json.loads(bytes.fromhex(row["command_hex"]))
        if command.get("method") == "turn/start" and command.get("params", {}).get("input") == [
                {"type": "text", "text": message["body"]}]:
            commands.append(row)
    auto_binding = None
    recipient_terminal = None
    if recipe and len(stages) == 3:
        episode = one(db, "SELECT * FROM gogoke_v37_h_process_episode WHERE domain_id=? AND request_id=?",
                      (domain, recipe["startRequestId"]))
        claim = one(db, "SELECT * FROM gogoke_v37_h_claim WHERE domain_id=? AND session_id=?",
                    (domain, recipe["sessionId"]))
        start = one(db, "SELECT * FROM gogoke_v37_rpc_steps WHERE domain_id=? AND session_id=? AND step_id='thread-start'",
                    (domain, recipe["sessionId"]))
        ack_row = one(db, "SELECT * FROM v37_ledger_raw_source WHERE operation_id=? AND source_epoch=? AND source_cursor=?",
                      (start["process_operation_id"], start["source_epoch"], start["source_cursor"]))
        ack = json.loads(bytes(ack_row["raw_bytes"]))
        thread = ack["result"]["thread"]["id"]
        check(episode["session_id"] == claim["session_id"] == recipe["sessionId"] and
              episode["generation"] == claim["generation"] == recipe["generation"] and
              episode["instance_id"] == claim["instance_id"] == recipe["instanceId"] and
              episode["phase"] == "STOPPED" and claim["state"] in ("STOPPED", "RELEASED") and
              episode["stop_fact_id"] == claim["stop_fact_id"] and start["phase"] == "OBSERVED" and
              start["process_operation_id"] == episode["process_operation_id"] and
              ack_row["state"] == "NO_EVENT" and ack_row["no_event_reason"] == "CODEX_RPC_RESPONSE" and thread,
              "Host-created recipient has no exact stopped H/A thread identity")
        auto_binding = {"id": recipe["sessionId"], "seatId": recipe["destinationSeatId"],
                        "instanceId": recipe["instanceId"], "worktreeId": recipe["worktreeId"],
                        "generation": recipe["generation"], "revision": str(claim["revision"]), "threadId": thread}
        if message["turn_id"]:
            completed = []
            for row in select(db, "SELECT * FROM v37_ledger_raw_source WHERE domain_id=? AND session_id=? AND generation=?",
                              (domain, recipe["sessionId"], recipe["generation"])):
                value = json.loads(bytes(row["raw_bytes"]))
                if value.get("method") == "turn/completed" and value.get("params", {}).get("threadId") == thread and \
                   value["params"].get("turn", {}).get("id") == message["turn_id"]:
                    completed.append((row, value))
            check(len(completed) <= 1, "Host recipient has duplicate original CLI completion")
            if completed:
                row, value = completed[0]
                recipient_terminal = {"source": {"state": row["state"], "operationId": row["operation_id"],
                                                "sourceEpoch": row["source_epoch"], "sourceCursor": row["source_cursor"],
                                                "rawHex": bytes(row["raw_bytes"]).hex()},
                                      "status": value["params"]["turn"]["status"]}
    return {"caseId": host["caseId"], "cause": cause, "intent": intent, "intentEvent": event,
            "enqueue": enqueue, "enqueueRequest": frame, "message": message, "deliveries": deliveries,
            "sends": sends, "commands": commands, "recipientOperation": recipe_rows[0] if recipe_rows else None,
            "recipient": recipe, "recipientStages": stages, "autoBinding": auto_binding,
            "recipientTerminal": recipient_terminal}


def original_start(db, domain, stdin, bound, body, turn=None):
    request = json.loads(bytes.fromhex(stdin["request_hex"]))
    receipt = json.loads(bytes.fromhex(stdin["receipt_hex"]))
    check(stdin["phase"] == "RECEIPTED" and stdin["receipt_status"] == "APPLIED" and
          stdin["operation"] == request["operation"] == receipt["operation"] == "send" and
          request["family"] == receipt["family"] == "K-SESSION" and
          stdin["domain_id"] == request["domainId"] == domain and
          stdin["session_id"] == request["targetId"] == receipt["targetId"] == bound["id"] and
          stdin["generation"] == bound["generation"] and request["payload"] == {"generation": bound["generation"], "body": body} and
          stdin["request_id"] == request["requestId"] == receipt["requestId"] and receipt["status"] == "APPLIED" and
          receipt["previousRevision"] == request["expectedRevision"] and
          int(receipt["revision"]) == int(request["expectedRevision"]) + 1 and
          receipt["result"]["createdTurn"] is True and (turn is None or receipt["result"]["turnId"] == turn),
          "Original H send/createdTurn is not the retained subject")
    episode = one(db, "SELECT * FROM gogoke_v37_h_process_episode WHERE domain_id=? AND session_id=? "
                  "AND process_operation_id=?", (domain, bound["id"], stdin["process_operation_id"]))
    pin = one(db, "SELECT i.driver_id,i.program_digest,c.* FROM gogoke_v37_instances i JOIN "
              "gogoke_coordination_process_custody c ON c.operation_id=? WHERE i.instance_id=?",
              (stdin["process_operation_id"], bound["instanceId"]))
    check(episode["seat_id"] == bound["seatId"] and episode["instance_id"] == bound["instanceId"] and
          episode["generation"] == bound["generation"] and episode["phase"] == "STOPPED" and episode["stop_fact_id"] and
          pin["driver_id"] == "codex" and pin["program_digest"] == pin["binary_digest_sha256"] and
          pin["domain_id"] == domain and pin["generation"] == bound["generation"] and
          pin["ticket"] == stdin["ticket"] and pin["custodian_nonce"] == stdin["custodian_nonce"] and
          pin["state"] == "STOPPED" and pin["stop_proof_hash"] == episode["stop_fact_id"],
          "Original stopped physical Codex identity/StopFact differs")
    step = one(db, "SELECT * FROM gogoke_v37_rpc_steps WHERE domain_id=? AND session_id=? AND step_id=?",
               (domain, bound["id"], "send-" + digest(bytes.fromhex(stdin["request_hex"]))[:40]))
    command = json.loads(bytes.fromhex(step["command_hex"]))
    check(step["phase"] == "OBSERVED" and step["requires_response"] == 1 and
          step["process_operation_id"] == stdin["process_operation_id"] and step["ticket"] == stdin["ticket"] and
          step["custodian_nonce"] == stdin["custodian_nonce"] and step["generation"] == bound["generation"] and
          step["open_request_id"] == episode["request_id"] and command["method"] == "turn/start" and
          command["params"]["threadId"] == bound["threadId"] and command["params"]["input"] == [{"type": "text", "text": body}],
          "Actual original RPC turn/start command/source binding differs")
    source = one(db, "SELECT * FROM v37_ledger_raw_source WHERE operation_id=? AND source_epoch=? AND source_cursor=? "
                 "AND process_ticket=? AND custodian_nonce=? AND domain_id=? AND session_id=? AND generation=?",
                 (step["process_operation_id"], step["source_epoch"], step["source_cursor"], step["ticket"],
                  step["custodian_nonce"], domain, bound["id"], bound["generation"]))
    raw = bytes(source["raw_bytes"])
    ack = json.loads(raw)
    check(source["state"] == "NO_EVENT" and source["no_event_reason"] == "CODEX_RPC_RESPONSE" and
          typed_id(ack["id"]) == typed_id(command["id"]) and ack["result"]["turn"]["id"] == receipt["result"]["turnId"] and
          ack["result"]["turn"]["status"] in ("inProgress", "completed"), "Original typed A TurnStart ACK differs")
    identity = "\n".join((digest(bytes.fromhex(stdin["request_hex"])), digest(bytes.fromhex(step["command_hex"])),
                           stdin["process_operation_id"], stdin["custodian_nonce"]))
    check(receipt["result"]["receiptId"] == "rpc-" + digest(identity.encode())[:40], "Original H receipt ID has another command/custody basis")
    return {"stdin": stdin, "episode": episode, "pin": pin, "step": step,
            "rawAck": raw.decode(), "receipt": receipt, "turnId": receipt["result"]["turnId"]}


def verify_recipient(db, domain, bound, operations):
    registration = one(db, "SELECT * FROM v37_ledger_session WHERE session_id=?", (bound["id"],))
    check(registration["domain_id"] == domain and registration["seat_id"] == bound["seatId"] and
          registration["purpose"] == "WORK" and registration["side_id"] is None, "Recipient is not the actual registered WORK seat")
    episode = one(db, "SELECT * FROM gogoke_v37_h_process_episode WHERE domain_id=? AND session_id=? AND generation=?",
                  (domain, bound["id"], bound["generation"]))
    raw = bytes.fromhex(episode["raw_hex"]).decode()
    request = json.loads(raw)
    user = operations[request["requestId"]]
    operation = one(db, "SELECT * FROM gogoke_v37_h_operation WHERE domain_id=? AND request_id=?", (domain, request["requestId"]))
    check(raw == user["rawFrame"] and request == user["request"] and request["family"] == "K-SESSION" and request["operation"] == "open" and
          request["targetId"] == bound["id"] and request["payload"]["seatId"] == bound["seatId"] and
          request["payload"]["generation"] == bound["generation"] and request["payload"]["worktreeId"] == bound["worktreeId"] and
          user["receipt"]["status"] == "APPLIED" and user["receipt"]["result"]["threadId"] == bound["threadId"] and
          operation["raw_hex"].lower() == episode["raw_hex"].lower() and operation["status"] == "APPLIED" and
          episode["seat_id"] == bound["seatId"] and episode["instance_id"] == bound["instanceId"] and
          episode["phase"] == "STOPPED" and episode["stop_fact_id"], "Recipient has no original normal H open and physical stop")
    creation = one(db, "SELECT incarnation FROM gogoke_v37_seat_operations WHERE domain_id=? AND seat_id=? AND revision=1 AND generation=1",
                   (domain, bound["seatId"]))
    pin = one(db, "SELECT i.driver_id,i.program_digest,c.* FROM gogoke_v37_instances i JOIN gogoke_coordination_process_custody c "
              "ON c.operation_id=? WHERE i.instance_id=?", (episode["process_operation_id"], bound["instanceId"]))
    check(episode["seat_incarnation"] == creation["incarnation"] and pin["driver_id"] == "codex" and
          pin["program_digest"] == pin["binary_digest_sha256"] and pin["generation"] == bound["generation"] and
          pin["domain_id"] == domain and pin["state"] == "STOPPED" and pin["stop_proof_hash"] == episode["stop_fact_id"],
          "Recipient CLI pin/custody/incarnation is another subject")
    return {"binding": bound, "originalOpen": raw, "episode": episode, "pin": pin}


def verify_auto_recipient(db, domain, snapshot, checkpoint, bound, operations):
    recipe = snapshot["recipient"]
    check(recipe and snapshot["autoBinding"] and
          all(snapshot["autoBinding"][key] == bound[key] for key in
              ("id", "seatId", "instanceId", "worktreeId", "generation", "threadId")) and
          recipe["sessionId"] == bound["id"] and recipe["generation"] == bound["generation"] and
          [row["stage"] for row in snapshot["recipientStages"]] == ["reserve", "commit", "start"],
          "Delivered recipient is not the exact C frozen Host choice")
    registration = one(db, "SELECT * FROM v37_ledger_session WHERE session_id=?", (bound["id"],))
    tree = one(db, "SELECT * FROM gogoke_v37_worktrees WHERE domain_id=? AND worktree_id=?",
               (domain, bound["worktreeId"]))
    seat = one(db, "SELECT * FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",
               (domain, bound["seatId"]))
    check(registration["domain_id"] == domain and registration["seat_id"] == bound["seatId"] and
          registration["purpose"] == "WORK" and registration["side_id"] is None and
          tree["state"] == "REGISTERED" and tree["seat_id"] == bound["seatId"] and
          tree["repository_id"] == recipe["repositoryId"] and tree["instance_id"] == bound["instanceId"] and
          tree["seat_incarnation"] == recipe["seatIncarnation"] == seat["incarnation"] and
          tree["permission_tier"] == recipe["permissionTier"] and
          seat["instance_id"] == bound["instanceId"], "Frozen Host choice lacks actual A/F/E registration")
    originals = []
    for stage in snapshot["recipientStages"]:
        request = stage["request"]
        operation = one(db, "SELECT * FROM gogoke_v37_h_operation WHERE domain_id=? AND request_id=?",
                        (domain, request["requestId"]))
        check(bytes.fromhex(operation["raw_hex"]) == bytes.fromhex(stage["operation"]["request_hex"]) and
              operation["session_id"] == bound["id"] and operation["operation"] == request["operation"] and
              operation["status"] == "APPLIED" and operation["previous_revision"] == int(request["expectedRevision"]) and
              operation["revision"] == int(request["expectedRevision"]) + 1,
              "C frozen stage was not the actual H reserve/commit/open mutation")
        originals.append(operation)
    check([row["previous_revision"] for row in originals] == [0, 1, 2],
          "Host admission stages changed revision/order")
    episode = one(db, "SELECT * FROM gogoke_v37_h_process_episode WHERE domain_id=? AND request_id=?",
                  (domain, recipe["startRequestId"]))
    claim = one(db, "SELECT * FROM gogoke_v37_h_claim WHERE domain_id=? AND session_id=?",
                (domain, bound["id"]))
    pin = one(db, "SELECT i.driver_id,i.program_digest,c.* FROM gogoke_v37_instances i JOIN "
              "gogoke_coordination_process_custody c ON c.operation_id=? WHERE i.instance_id=?",
              (episode["process_operation_id"], bound["instanceId"]))
    check(episode["session_id"] == bound["id"] and episode["generation"] == bound["generation"] and
          episode["instance_id"] == bound["instanceId"] and episode["seat_id"] == bound["seatId"] and
          episode["seat_incarnation"] == recipe["seatIncarnation"] and
          bytes.fromhex(episode["raw_hex"]) == bytes.fromhex(snapshot["recipientStages"][2]["operation"]["request_hex"]) and
          episode["phase"] == "STOPPED" and episode["stop_fact_id"] == claim["stop_fact_id"] and
          claim["state"] == "RELEASED" and claim["generation"] == bound["generation"] and
          pin["driver_id"] == "codex" and pin["program_digest"] == pin["binary_digest_sha256"] and
          pin["domain_id"] == domain and pin["generation"] == bound["generation"] and
          pin["state"] == "STOPPED" and pin["stop_proof_hash"] == episode["stop_fact_id"],
          "Original Host H open is not the physically stopped/released Codex recipient")
    releases = [entry for entry in operations.values() if entry["request"].get("family") == "K-SESSION" and
                entry["request"].get("targetId") == bound["id"] and
                entry["request"].get("operation") == "admission-release"]
    check(len(releases) == 1, "Host-created session needs one original User release after physical stop")
    release = releases[0]
    stored_release = one(db, "SELECT * FROM gogoke_v37_h_operation WHERE domain_id=? AND request_id=?",
                         (domain, release["request"]["requestId"]))
    check(bytes.fromhex(stored_release["raw_hex"]).decode() == release["rawFrame"] and
          release["request"]["payload"] == {"seatId": bound["seatId"], "generation": bound["generation"]} and
          release["request"]["expectedRevision"] == checkpoint["autoBinding"]["revision"] and
          release["receipt"]["status"] == stored_release["status"] == "APPLIED" and
          release["receipt"]["revision"] == str(claim["revision"]) and
          stored_release["revision"] == claim["revision"],
          "Original Host recipient release does not bind its checkpoint StopFact/revision")
    step = one(db, "SELECT * FROM gogoke_v37_rpc_steps WHERE domain_id=? AND session_id=? AND step_id='thread-start'",
               (domain, bound["id"]))
    command = json.loads(bytes.fromhex(step["command_hex"]))
    ack_source = one(db, "SELECT * FROM v37_ledger_raw_source WHERE operation_id=? AND source_epoch=? AND source_cursor=?",
                     (step["process_operation_id"], step["source_epoch"], step["source_cursor"]))
    ack = json.loads(bytes(ack_source["raw_bytes"]))
    check(step["phase"] == "OBSERVED" and step["process_operation_id"] == episode["process_operation_id"] and
          command["method"] == "thread/start" and typed_id(command["id"]) == typed_id(ack["id"]) and
          ack_source["state"] == "NO_EVENT" and ack_source["no_event_reason"] == "CODEX_RPC_RESPONSE" and
          ack["result"]["thread"]["id"] == bound["threadId"],
          "Original Host recipient thread ACK does not bind its A identity")
    return {"binding": bound, "recipe": snapshot["recipientOperation"], "stages": snapshot["recipientStages"],
            "episode": episode, "claim": claim, "pin": pin, "release": release["rawFrame"],
            "originalThreadAck": bytes(ack_source["raw_bytes"]).decode()}


def verify_final_source(db, domain, initial, current, journal_operations):
    check(all(current[name] == initial[name] for name in
              ("id", "seatId", "instanceId", "worktreeId", "threadId")) and
          current["generation"].isdecimal() and current["revision"].isdecimal() and
          int(current["generation"]) > int(initial["generation"]),
          "Final source changed its original logical binding or lacks a resumed generation")
    scoped = [entry for entry in journal_operations if entry["request"].get("family") == "K-SESSION" and
              entry["request"].get("targetId") == initial["id"]]
    resumes = [index for index, entry in enumerate(scoped) if entry["request"]["operation"] == "resume"]
    check(resumes and [entry["request"]["operation"] for entry in scoped[resumes[-1]:]
          if entry["request"]["operation"] in ("resume", "send", "open", "stop", "admission-release")]
          == ["resume", "stop", "admission-release"],
          "Last actual source generation must end with one stop and release")
    resume, stop, release = scoped[resumes[-1]], scoped[-2], scoped[-1]
    check([entry["request"]["operation"] for entry in (resume, stop, release)] ==
          ["resume", "stop", "admission-release"], "Final source operation order changed")
    for entry in (resume, stop, release):
        request, receipt = entry["request"], entry["receipt"]
        operation = one(db, "SELECT * FROM gogoke_v37_h_operation WHERE domain_id=? AND request_id=?",
                        (domain, request["requestId"]))
        expected_payload = {"generation": request["payload"]["generation"]}
        if request["operation"] != "resume":
            expected_payload["seatId"] = initial["seatId"]
        check(json.loads(entry["rawFrame"]) == request and
              bytes.fromhex(operation["raw_hex"]).decode() == entry["rawFrame"] and
              operation["operation"] == request["operation"] and operation["session_id"] == initial["id"] and
              operation["status"] == receipt["status"] == "APPLIED" and
              operation["previous_revision"] == int(request["expectedRevision"]) and
              operation["revision"] == int(receipt["revision"]) and
              receipt["previousRevision"] == request["expectedRevision"] and
              receipt["revision"] == str(int(request["expectedRevision"]) + 1) and
              request["payload"] == expected_payload,
              "Final source original H mutation/receipt differs")
    check(resume["request"]["payload"]["generation"] == resume["receipt"]["result"]["oldGeneration"] and
          resume["receipt"]["result"]["newGeneration"] == current["generation"] ==
          stop["request"]["payload"]["generation"] == release["request"]["payload"]["generation"] and
          stop["receipt"]["result"]["stopFact"] and
          release["receipt"]["revision"] == current["revision"],
          "Final source journal does not bind its last H generation/stop/release")
    episode = one(db, "SELECT * FROM gogoke_v37_h_process_episode WHERE domain_id=? AND session_id=? AND generation=?",
                  (domain, initial["id"], current["generation"]))
    claim = one(db, "SELECT * FROM gogoke_v37_h_claim WHERE domain_id=? AND session_id=?",
                (domain, initial["id"]))
    binding = one(db, "SELECT * FROM gogoke_v37_h_seat_binding WHERE domain_id=? AND session_id=?",
                  (domain, initial["id"]))
    seat = one(db, "SELECT * FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",
               (domain, initial["seatId"]))
    pin = one(db, "SELECT i.driver_id,i.program_digest,c.* FROM gogoke_v37_instances i JOIN "
              "gogoke_coordination_process_custody c ON c.operation_id=? WHERE i.instance_id=?",
              (episode["process_operation_id"], initial["instanceId"]))
    check(episode["request_id"] == resume["request"]["requestId"] and
          episode["old_generation"] == resume["receipt"]["result"]["oldGeneration"] and
          bytes.fromhex(episode["raw_hex"]).decode() == resume["rawFrame"] and
          episode["seat_id"] == binding["seat_id"] == initial["seatId"] and
          episode["seat_incarnation"] == binding["seat_incarnation"] == seat["incarnation"] and
          episode["instance_id"] == claim["instance_id"] == seat["instance_id"] == initial["instanceId"] and
          episode["generation"] == claim["generation"] == binding["generation"] == current["generation"] and
          seat["generation"] == int(current["generation"]) and seat["state"] == "IDLE" and
          episode["phase"] == "STOPPED" and episode["stop_request_id"] == stop["request"]["requestId"] and
          episode["stop_fact_id"] == claim["stop_fact_id"] == stop["receipt"]["result"]["stopFact"] and
          claim["state"] == "RELEASED" and claim["revision"] == int(current["revision"]) and
          claim["process_operation_id"] == episode["process_operation_id"] and
          pin["driver_id"] == "codex" and pin["program_digest"] == pin["binary_digest_sha256"] and
          pin["domain_id"] == domain and pin["generation"] == current["generation"] and
          pin["state"] == "STOPPED" and pin["stop_proof_hash"] == episode["stop_fact_id"],
          "Final source is not the physically stopped, released original H identity")
    step = one(db, "SELECT * FROM gogoke_v37_rpc_steps WHERE domain_id=? AND session_id=? AND "
               "process_operation_id=? AND step_id=?",
               (domain, initial["id"], episode["process_operation_id"],
                episode["process_operation_id"] + "-thread-resume"))
    command = json.loads(bytes.fromhex(step["command_hex"]))
    ack_source = one(db, "SELECT * FROM v37_ledger_raw_source WHERE operation_id=? AND source_epoch=? AND source_cursor=? "
                     "AND process_ticket=? AND custodian_nonce=? AND domain_id=? AND session_id=? AND generation=?",
                     (episode["process_operation_id"], step["source_epoch"], step["source_cursor"],
                      step["ticket"], step["custodian_nonce"], domain, initial["id"], current["generation"]))
    ack = json.loads(bytes(ack_source["raw_bytes"]))
    check(step["phase"] == "OBSERVED" and step["generation"] == current["generation"] and
          step["open_request_id"] == resume["request"]["requestId"] and
          command["method"] == "thread/resume" and command["params"]["threadId"] == current["threadId"] and
          typed_id(command["id"]) == typed_id(ack["id"]) and
          ack["result"]["thread"]["id"] == current["threadId"] and
          ack_source["state"] == "NO_EVENT" and ack_source["no_event_reason"] == "CODEX_RPC_RESPONSE" and
          resume["receipt"]["result"]["receiptId"] ==
          f'{episode["process_operation_id"]}:{step["source_epoch"]}:{step["source_cursor"]}',
          "Final source resume lacks its original typed A thread ACK")
    return {"binding": current, "episode": episode, "claim": claim, "seat": seat,
            "originalResume": resume["rawFrame"], "originalThreadAck": bytes(ack_source["raw_bytes"]).decode(),
            "originalStop": stop["rawFrame"], "originalRelease": release["rawFrame"]}


def verify_checkpoint_stops(db, domain, case, host, operations, claims):
    stops = host.get("checkpointStops", [])
    target = host["autoObservation"]["sessionId"] if host["kind"] == "DELIVERED" else host["busy"]["binding"]["id"]
    check(len(stops) == 3 and [row["binding"]["id"] for row in stops] ==
          [case["submitterSession"], case["reviewerSession"], target],
          "Checkpoint needs the two original sources and one actual recipient H stop")
    evidence = []
    for row in stops:
        bound = row["binding"]
        read, stopped = operations[row["readRequestId"]], operations[row["stopRequestId"]]
        request, receipt = stopped["request"], stopped["receipt"]
        observed = one(db, "SELECT request_bytes,receipt_bytes FROM v37_ledger_receipt WHERE family='K-SESSION' "
                       "AND domain_id=? AND request_id=?", (domain, row["readRequestId"]))
        check(bytes(observed["request_bytes"]).decode() == read["rawFrame"] and
              json.loads(bytes(observed["receipt_bytes"])) == read["receipt"] and
              read["request"]["operation"] == "output-stream" and read["request"]["targetId"] == bound["id"] and
              read["request"]["payload"]["generation"] == read["receipt"]["result"]["generation"] == bound["generation"] and
              read["receipt"]["status"] == "APPLIED" and
              request["expectedRevision"] == read["receipt"]["revision"],
              "Checkpoint stop revision lacks its original live User H read")
        operation = one(db, "SELECT * FROM gogoke_v37_h_operation WHERE domain_id=? AND request_id=?",
                        (domain, row["stopRequestId"]))
        episode = one(db, "SELECT * FROM gogoke_v37_h_process_episode WHERE domain_id=? AND session_id=? AND generation=?",
                      (domain, bound["id"], bound["generation"]))
        custody = one(db, "SELECT * FROM gogoke_coordination_process_custody WHERE operation_id=?",
                      (episode["process_operation_id"],))
        matched = [claim for claim in claims if claim["session_id"] == bound["id"]]
        check(len(matched) == 1, "Exact stopped claim missing from original immutable checkpoint")
        claim = matched[0]
        check(json.loads(stopped["rawFrame"]) == request and
              bytes.fromhex(operation["raw_hex"]).decode() == stopped["rawFrame"] and
              request["family"] == receipt["family"] == "K-SESSION" and request["domainId"] == domain and
              request["operation"] == receipt["operation"] == operation["operation"] == "stop" and
              request["targetId"] == receipt["targetId"] == operation["session_id"] == bound["id"] and
              request["payload"] == {"seatId": bound["seatId"], "generation": bound["generation"]} and
              receipt["requestId"] == request["requestId"] == row["stopRequestId"] and
              receipt["status"] == operation["status"] == "APPLIED" and
              receipt["previousRevision"] == request["expectedRevision"] == str(operation["previous_revision"]) and
              receipt["revision"] == row["revision"] == str(operation["revision"]) ==
              str(int(request["expectedRevision"]) + 1) and
              episode["seat_id"] == bound["seatId"] and episode["instance_id"] == bound["instanceId"] and
              episode["phase"] == custody["state"] == claim["state"] == "STOPPED" and
              episode["stop_request_id"] == row["stopRequestId"] and
              episode["stop_fact_id"] == custody["stop_proof_hash"] == claim["stop_fact_id"] ==
              receipt["result"]["stopFact"] == row["stopFact"] and row["stopFact"] and
              custody["domain_id"] == claim["domain_id"] == domain and
              custody["generation"] == claim["generation"] == bound["generation"] and
              claim["instance_id"] == bound["instanceId"] and str(claim["revision"]) == row["revision"] and
              claim["process_operation_id"] == episode["process_operation_id"],
              "Normal close cannot substitute for the original receipted H stop and physical StopFact")
        evidence.append({"binding": bound, "originalRead": bytes(observed["receipt_bytes"]).decode(),
                         "originalStop": stopped["rawFrame"], "receipt": receipt,
                         "claim": claim, "episode": episode, "custody": custody})
    return evidence


def verify_host(db, domain, case, host, operations, result):
    reference = host["checkpoint"]
    check(Path(reference["file"]).name == reference["file"], "Original checkpoint basename required")
    raw = (output.parent / reference["file"]).read_bytes()
    check(digest(raw) == reference["sha256"], "Original queued checkpoint bytes changed")
    queued = json.loads(raw)
    check(queued["phase"] == "checkpoint" and queued["caseId"] == result["caseId"] and
          queued["sourceCommit"] == result["sourceCommit"] and queued["databasePath"] == result["databasePath"] and
          queued["rootIdentity"] == result["rootIdentity"] and queued["measurementPreservedDatabaseBytes"] and
          queued["databaseWrites"] is False and queued["credentialReads"] is False and
          queued["readerSha256"] == case["readerSha256"] == result["readerSha256"], "Queued artifact is another subject")
    prior = [row for row in queued["hostSnapshots"] if row["caseId"] == host["caseId"]]
    check(len(prior) == 1, "Original Host checkpoint cause missing")
    check(queued["checkpointStops"] == verify_checkpoint_stops(db, domain, case, host, operations, queued["stoppedClaims"]),
          "Original checkpoint H stop evidence changed")
    if host["kind"] == "DELIVERED":
        check(prior[0]["message"]["state"] == "DELIVERED" and prior[0]["autoBinding"] and
              len(prior[0]["deliveries"]) == len(prior[0]["sends"]) == len(prior[0]["commands"]) == 1,
              "Checkpoint did not capture genuine automatic Host delivery")
    else:
        check(prior[0]["message"]["state"] == ("CANCELLED" if host["kind"] == "CANCELLED" else "PENDING") and
              prior[0]["message"]["turn_id"] == "" and
              prior[0]["message"]["generation"] == "" and not prior[0]["deliveries"] and
              not prior[0]["sends"] and not prior[0]["commands"] and prior[0]["recipient"] is None,
              "Unanswered target did not block Host recipient preparation before Owner control")
    final = host_snapshot(db, domain, host)
    original = prior[0]
    check(final["enqueue"] == original["enqueue"] and final["intentEvent"] == original["intentEvent"] and
          final["cause"] == original["cause"] and final["message"]["message_id"] == host["messageId"] == original["message"]["message_id"] and
          final["intent"]["trigger_id"] == host["triggerId"] and final["intent"]["request_id"] == host["escalationRequestId"] and
          final["enqueue"]["request_id"] == host["enqueueRequestId"], "Original E/C cause/IDs/bytes changed after checkpoint")
    expected_targets = [host["destination"]]
    bound_targets = host["targetSessions"] if host["kind"] == "DELIVERED" else [host["busy"]["binding"]]
    check(len(bound_targets) == len(expected_targets) and all(
        all(bound[name] == selected[name] for name in ("seatId", "instanceId", "worktreeId"))
        for bound, selected in zip(bound_targets, expected_targets)), "Controlled recipients were not actually opened")
    final["recipients"] = ([verify_auto_recipient(db, domain, final, original, bound_targets[0], operations)]
                           if host["kind"] == "DELIVERED" else
                           [verify_recipient(db, domain, bound, operations) for bound in bound_targets])
    if host["kind"] == "DELIVERED":
        check(len(final["deliveries"]) == len(final["sends"]) == len(final["commands"]) == len(host["targetSessions"]) == 1 and
              final["recipientOperation"] == original["recipientOperation"] and
              final["recipientStages"] == original["recipientStages"],
              "A cause has a missing or second original logical C/H/RPC send")
        delivery = final["deliveries"][0]
        target = host["targetSessions"][0]
        live = operations[host["autoObservation"]["outputReadRequestId"]]
        stored_live = one(db, "SELECT request_bytes,receipt_bytes FROM v37_ledger_receipt WHERE family='K-SESSION' "
                          "AND domain_id=? AND request_id=?", (domain, host["autoObservation"]["outputReadRequestId"]))
        native_input = host["autoObservation"]["nativeInputReceipt"]
        check(bytes(stored_live["request_bytes"]).decode() == live["rawFrame"] and
              json.loads(bytes(stored_live["receipt_bytes"])) == live["receipt"] and
              live["request"]["operation"] == "output-stream" and live["request"]["targetId"] == target["id"] and
              live["request"]["payload"]["generation"] == live["receipt"]["result"]["generation"] == target["generation"] and
              live["receipt"]["status"] == "APPLIED" and
              native_input in live["receipt"]["result"]["nativeInputReceipts"] and native_input["phase"] == "RECEIPTED" and
              native_input["receipt"] == json.loads(bytes.fromhex(final["sends"][0]["receipt_hex"])) and
              native_input["receipt"]["targetId"] == target["id"] and
              native_input["receipt"]["result"]["turnId"] == host["observedTurnId"] and
              native_input["receipt"]["result"]["generation"] == target["generation"],
              "Automatic stop subject was not observed through its original live H output/send ACK")
        observed_card = operations[host["autoObservation"]["seatCardRequestId"]]
        stored_card = one(db, "SELECT request_bytes,receipt_bytes FROM v37_ledger_receipt WHERE family='K-SEAT' "
                          "AND domain_id=? AND request_id=?",
                          (domain, host["autoObservation"]["seatCardRequestId"]))
        check(bytes(stored_card["request_bytes"]).decode() == observed_card["rawFrame"] and
              json.loads(bytes(stored_card["receipt_bytes"])) == observed_card["receipt"] and
              observed_card["request"]["operation"] == "state-card" and
              observed_card["request"]["targetId"] == target["seatId"] and
              observed_card["receipt"]["result"]["state"] == "BUSY" and
              observed_card["receipt"]["result"]["instanceId"] == target["instanceId"] and
              str(observed_card["receipt"]["result"]["generation"]) == target["generation"] and
              host["autoObservation"]["sessionId"] == target["id"] and
              host["autoObservation"]["generation"] == target["generation"] and
              host["autoObservation"]["threadId"] == target["threadId"] and
              host["autoObservation"]["turnId"] == host["observedTurnId"],
              "Live automatic recipient observation is not the checkpoint's exact H generation/turn")
        frame, operation = delivery["request"], delivery["operation"]
        check({k: v for k, v in frame.items() if k not in ("sessionId", "ticket", "generation", "hSendRequestId", "operation")} ==
              {k: v for k, v in final["enqueueRequest"].items() if k != "operation"} and frame["operation"] == "deliver" and
              frame["sessionId"] == target["id"] and frame["generation"] == target["generation"] and
              frame["hSendRequestId"] == final["sends"][0]["request_id"] and frame["ticket"] == final["sends"][0]["ticket"] and
              operation["phase"] == "APPLIED" and operation["result_state"] == "DELIVERED" and operation["previous_revision"] == "1" and
              operation["revision"] == final["message"]["revision"] == "2" and final["message"]["state"] == "DELIVERED",
              "Original C delivery reservation/result differs")
        observed = original_start(db, domain, final["sends"][0], target, final["message"]["body"], host["observedTurnId"])
        check(final["commands"][0] == observed["step"] and final["message"]["turn_id"] == observed["turnId"] and
              final["message"]["generation"] == target["generation"] and final["intent"]["state"] == "DELIVERED" and
              final["intent"]["revision"] == 2 and final["intent"]["delivery_receipt_id"] == operation["native_receipt_id"] ==
              observed["receipt"]["result"]["receiptId"], "C/E delivery lacks original H createdTurn receipt")
        completed = []
        recipient_source = select(db, "SELECT * FROM v37_ledger_raw_source WHERE operation_id=?",
                                  (observed["stdin"]["process_operation_id"],))
        for row in recipient_source:
            value = json.loads(bytes(row["raw_bytes"]))
            if value.get("method") == "turn/completed" and value["params"]["threadId"] == target["threadId"] and value["params"]["turn"]["id"] == observed["turnId"]:
                completed.append((row, value))
        items, calls, questions = turn_activity([(row, json.loads(bytes(row["raw_bytes"]))) for row in recipient_source],
                                                 target["threadId"], observed["turnId"])
        check(not items and not calls and not questions,
              "Host notice caused extra original recipient tool/question effects")
        check(len(completed) == 1 and completed[0][0]["state"] != "PENDING" and
              completed[0][1]["params"]["turn"]["status"] == "completed" and
              final["recipientTerminal"]["source"]["rawHex"] == bytes(completed[0][0]["raw_bytes"]).hex() and
              original["recipientTerminal"] == final["recipientTerminal"],
              "Separate original Host recipient CLI completion absent")
        final["nativeDelivery"] = observed
    else:
        state = "PENDING" if host["kind"] == "ROUTE_CHANGED" else "CANCELLED"
        check(final["message"]["state"] == state and not final["message"]["turn_id"] and not final["message"]["generation"] and
              not final["deliveries"] and not final["sends"] and not final["commands"] and
              final["recipient"] is None and not final["recipientStages"] and
              final["intent"] == original["intent"] and final["intent"]["state"] == "INTENT",
              "Owner route/cancel control generated a new effect or lost historical E fact")
        if state == "CANCELLED":
            user = operations[host["cancelRequestId"]]
            operation = one(db, "SELECT * FROM gogoke_v37_inbox_operations WHERE domain_id=? AND request_id=?",
                            (domain, host["cancelRequestId"]))
            check(bytes.fromhex(operation["request_hex"]).decode() == user["rawFrame"] and json.loads(user["rawFrame"]) == user["request"] and
                  user["request"]["family"] == "K-INBOX" and user["request"]["operation"] == "cancel" and
                  user["request"]["targetId"] == final["message"]["message_id"] and user["request"]["payload"] == {} and
                  user["request"]["expectedRevision"] == "1" and user["receipt"]["status"] == "APPLIED" and
                  operation["phase"] == "APPLIED" and operation["result_state"] == user["receipt"]["result"]["state"] == "CANCELLED" and
                  operation["previous_revision"] == "1" and operation["revision"] == final["message"]["revision"] == "2", "Original User cancellation differs")
            final["cancel"] = operation
    for key in host["readRequestIds"]:
        user = operations[key]
        stored = one(db, "SELECT request_bytes,receipt_bytes FROM v37_ledger_receipt WHERE family='K-INBOX' AND domain_id=? AND request_id=?", (domain, key))
        check(bytes(stored["request_bytes"]).decode() == user["rawFrame"] and json.loads(bytes(stored["receipt_bytes"])) == user["receipt"] and
              user["request"]["operation"] == "check-unknown" and user["request"]["targetId"] == host["messageId"], "Original readonly Host observation differs")
    if host["kind"] != "DELIVERED":
        busy = host["busy"]
        user = operations[busy["sendRequestId"]]
        stdin = one(db, "SELECT * FROM gogoke_v37_h_stdin_journal WHERE domain_id=? AND request_id=?", (domain, busy["sendRequestId"]))
        check(bytes.fromhex(stdin["request_hex"]).decode() == user["rawFrame"] and
              user["request"]["payload"]["body"] == busy["askBytes"], "Original busy ask is not the real User send")
        final["busyNativeTurn"] = original_start(db, domain, stdin, busy["binding"], busy["askBytes"], busy["turnId"])
        check(len(busy["readRequestIds"]) == 2, "Two original live native busy-card observations required")
        for key in busy["readRequestIds"]:
            user = operations[key]
            stored = one(db, "SELECT request_bytes,receipt_bytes FROM v37_ledger_receipt WHERE family='K-QCARD' AND domain_id=? AND request_id=?", (domain, key))
            receipt = json.loads(bytes(stored["receipt_bytes"]))
            check(bytes(stored["request_bytes"]).decode() == user["rawFrame"] and receipt == user["receipt"] and
                  user["request"]["operation"] == "recover" and user["request"]["targetId"] == busy["cardId"] and
                  receipt["status"] == "APPLIED" and receipt["result"]["state"] == "OPEN" and receipt["result"]["availableForAnswer"] is True and
                  receipt["result"]["seatId"] == busy["binding"]["seatId"] and receipt["result"]["generation"] == busy["binding"]["generation"] and
                  receipt["result"]["nativeQuestion"]["threadId"] == busy["binding"]["threadId"] and
                  receipt["result"]["nativeQuestion"]["turnId"] == busy["turnId"], "Busy condition has no real custody-bound native question")
        raised = one(db, "SELECT * FROM gogoke_v37_qcard_native_operations WHERE domain_id=? AND card_id=? AND state='RAISED'", (domain, busy["cardId"]))
        descriptor = json.loads(bytes.fromhex(raised["request_hex"]))
        source = one(db, "SELECT * FROM v37_ledger_raw_source WHERE operation_id=? AND source_epoch=? AND source_cursor=?",
                     (descriptor["operationId"], descriptor["sourceEpoch"], descriptor["sourceCursor"]))
        raw = bytes(source["raw_bytes"])
        frame = json.loads(raw)
        check(digest(raw) == descriptor["frameSha256"] and source["operation_id"] == stdin["process_operation_id"] and
              source["domain_id"] == domain and source["process_ticket"] == stdin["ticket"] and source["custodian_nonce"] == stdin["custodian_nonce"] and
              source["state"] == "NO_EVENT" and source["no_event_reason"] == "NATIVE_QUESTION_CARD" and
              frame["params"]["threadId"] == busy["binding"]["threadId"] and frame["params"]["turnId"] == busy["turnId"] and
              frame["params"]["questions"][0]["id"] == busy["question"]["questionId"] and
              any(row["label"] == busy["question"]["optionLabel"] for row in frame["params"]["questions"][0]["options"]),
              "Native busy card has no original A question source")
        check(not select(db, "SELECT 1 FROM gogoke_v37_qcard_native_operations WHERE domain_id=? AND card_id=? AND state IN ('ANSWERED','UNKNOWN')",
                         (domain, busy["cardId"])), "Busy source was answered/replayed instead of held")
        incoming = select(db, "SELECT * FROM v37_ledger_raw_source WHERE operation_id=?", (stdin["process_operation_id"],))
        items, calls, questions = turn_activity(
            [(row, json.loads(bytes(row["raw_bytes"]))) for row in incoming],
            busy["binding"]["threadId"], busy["turnId"], allow_question=True)
        check(not items and not calls and len(questions) == 1 and
              bytes(questions[0][0]["raw_bytes"]) == raw,
              "Busy question used unrequested additional tool work")
        final["busyOriginalA"] = raw.decode()
    result["hostSnapshots"].append(final)
    return final


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
          before["rootIdentity"] == result["rootIdentity"] and before["measurementPreservedDatabaseBytes"] and
          before["readerSha256"] == case["readerSha256"] ==
          journal["driverBytes"]["m2-rules-readback.py"],
          "Original normally closed baseline is for another candidate/domain")
    result["originalReaderSha256"] = case["readerSha256"]
    result["readerChangedSinceOriginalRun"] = result["readerSha256"] != case["readerSha256"]
    check(case["ownership"] == {"lifecycle": "EXCLUSIVE_V08_SUBMITTER_AND_REVIEWER",
                                "policy": "EXCLUSIVE_V08_POLICY_DOMAIN"}, "Exclusive V08 ownership missing")
    check(case["domainId"] == domain and case["sourceCommit"] == journal["sourceCommit"], "Case byte identity differs")
    check(case["driverSha256"] == digest(Path(__file__).with_name("m2-rules.mjs").read_bytes()),
          "Actual loaded V08 module bytes differ from the reader's module")
    foreign = journal.get("foreignProject")
    check(case.get("foreignProject") == foreign and before.get("foreignProject") == result["foreignProject"],
          "Foreign fixture configuration or original B policy bytes changed since baseline")
    sessions = {s["id"]: s for s in journal["sessions"]}
    submitter, reviewer = case["initialSessions"]
    check([submitter["id"], reviewer["id"]] == [case["submitterSession"], case["reviewerSession"]] and
          all(sessions[row["id"]]["seatId"] == row["seatId"] for row in (submitter, reviewer)), "Original source identities changed")
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
    if case.get("hostCases"):
        result["initialSources"] = [verify_recipient(db, domain, row, operations) for row in (submitter, reviewer)]
        result["finalSources"] = [verify_final_source(db, domain, row, sessions[row["id"]], journal["operations"])
                                  for row in (submitter, reviewer)]
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
    expected_config = [
        ("policy-call-grant", {"callerSeatId": submitter["seatId"], "targetId": reviewer["seatId"], "action": "REVIEW", "expiresAtMs": None}),
        *[("policy-gate", {"gateId": gate, "submitterSeatId": submitter["seatId"], "reviewerSeatId": reviewer["seatId"],
                          "fromStage": case["fromStage"], "toStage": case["toStage"], "rejectCap": 2}) for gate in (reject, passed)],
        ("policy-call-grant", {"callerSeatId": submitter["seatId"], "targetId": reviewer["seatId"], "action": "REVIEW", "expiresAtMs": "1"}),
        ("policy-call-grant", {"callerSeatId": submitter["seatId"], "targetId": reviewer["seatId"], "action": "REVIEW", "expiresAtMs": None}),
    ]
    host_cases = case.get("hostCases", [])
    if host_cases:
        check([row["kind"] for row in host_cases] == ["DELIVERED", "BUSY_QUEUED", "ROUTE_CHANGED", "CANCELLED"] and
              case["hostOwnership"] == "EXCLUSIVE_V08_HOST_RECIPIENTS" and
              len({row["causeEventId"] for row in host_cases}) == len(host_cases) and
              len({row["gateId"] for row in host_cases}) == len(host_cases), "Distinct original Host cases/causes required")
        destination, alternate = case["hostRecipients"]
        check(len({row["seatId"] for row in (submitter, reviewer, destination, alternate)}) == 4 and
              len({row["worktreeId"] for row in (submitter, reviewer, destination, alternate)}) == 4, "Host recipients overlap source custody")
        for host in host_cases:
            check(host["destination"] == destination and host["sourceSeatId"] == submitter["seatId"], "Host route selectors changed")
            expected_config.extend([
                ("policy-escalation-route", {"fromSeatId": submitter["seatId"], "reason": "REJECT_CAP", "toSeatId": destination["seatId"]}),
                ("policy-gate", {"gateId": host["gateId"], "submitterSeatId": submitter["seatId"], "reviewerSeatId": reviewer["seatId"],
                                 "fromStage": case["toStage"], "toStage": host["toStage"], "rejectCap": 1}),
            ])
            if host["kind"] == "ROUTE_CHANGED":
                expected_config.append(("policy-escalation-route", {"fromSeatId": submitter["seatId"], "reason": "REJECT_CAP", "toSeatId": alternate["seatId"]}))
    check(len(configurations) == len(expected_config), "Missing or extra prescribed Owner configuration operation")
    expected_routes = {(row["from_seat_id"], row["reason"]): row for row in before["policy"]["routes"]}
    host_revisions = {}
    for index, (entry, (command, fields)) in enumerate(zip(configurations, expected_config)):
        if index == 5:
            revision += 1  # The actual core legal-stage transition precedes Host configuration.
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
        if command == "policy-escalation-route":
            expected_routes[(fields["fromSeatId"], fields["reason"])] = {"domain_id": domain,
                "from_seat_id": fields["fromSeatId"], "reason": fields["reason"], "to_seat_id": fields["toSeatId"], "revision": revision}
        if command == "policy-gate":
            host_revisions[fields["gateId"]] = revision
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
        ("V08_MODEL_FORGED_SENDER", submitter, "gate-decide", passed, "2",
         {"decision": "PASS", "callerSeatId": reviewer["seatId"]}, "DENIED", None, "", initial_revision + 2),
        ("V08_MODEL_WRONG_REVIEWER", submitter, "gate-decide", passed, "2", {"decision": "PASS"}, "DENIED", None, "", initial_revision + 2),
        *([("V08_MODEL_CROSS_PROJECT", submitter, "gate-submit", foreign["gateId"],
            str(before["foreignProject"]["gate"]["revision"]), {}, "DENIED", None, "", initial_revision + 2)]
          if foreign else []),
        ("V08_MODEL_EMPTY_REJECT_REASON", reviewer, "gate-decide", passed, "2", {"decision": "REJECT", "reason": ""}, "INVALID_INPUT", None, "", initial_revision + 2),
        ("V08_APPROVE", reviewer, "gate-decide", passed, "2", {"decision": "PASS"}, "APPLIED", "PASSED", "", initial_revision + 2),
        ("V08_LEGAL_STAGE", submitter, "stage-transition", passed, "3", {}, "APPLIED", "ADVANCED", case["toStage"], initial_revision + 3),
    ]
    for host in host_cases:
        check(int(host["policyRevision"]) == host_revisions[host["gateId"]] and
              int(host["routeRevision"]) == int(host["policyRevision"]) - 1, "Host cause revision differs from original Owner gate/route CAS")
        cases.extend([
            (host["caseId"] + "_SUBMIT", submitter, "gate-submit", host["gateId"], "1", {}, "APPLIED", "SUBMITTED", "", int(host["policyRevision"])),
            (host["caseId"] + "_REJECT", reviewer, "gate-decide", host["gateId"], "2", {"decision": "REJECT", "reason": host["reason"]},
             "APPLIED", "ESCALATION_REQUIRED", host["reason"], int(host["policyRevision"])),
        ])
    check(len(case["actions"]) == len(cases), "Incomplete or additional model actions")
    checked_sessions = set()
    for action, (key, session, operation, gate, gate_rev, payload, status, state, reason, policy_rev) in zip(case["actions"], cases):
        arguments = {"operation": operation, "targetId": gate, "expectedRevision": gate_rev, "payload": payload}
        check(action["caseId"] == key and action["sessionId"] == session["id"] and action["arguments"] == arguments,
              "Model action identity/arguments differ")
        check(action["binding"]["id"] == session["id"] and action["binding"]["seatId"] == session["seatId"] and
              action["binding"]["instanceId"] == session["instanceId"] and action["binding"]["worktreeId"] == session["worktreeId"],
              "Historical action changed its logical subject")
        session = action["binding"]  # Original per-action generation, never a mutable final session snapshot.
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
        physical = (session["id"], stdin["process_operation_id"], session["generation"])
        if physical not in checked_sessions:
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
            checked_sessions.add(physical)
        incoming = select(db, "SELECT * FROM v37_ledger_raw_source WHERE domain_id=? AND session_id=? AND operation_id=? "
                          "AND process_ticket=? AND custodian_nonce=? AND generation=? ORDER BY rowid",
                          (domain, session["id"], stdin["process_operation_id"], stdin["ticket"], stdin["custodian_nonce"], stdin["generation"]))
        decoded = [(row, json.loads(bytes(row["raw_bytes"]))) for row in incoming]
        tool_items, calls, questions = turn_activity(decoded, session["threadId"], action["turnId"],
                                                     expected_input=action["askBytes"])
        check(not questions, "Model turn has an unrelated question")
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
              acknowledgements[0][1]["result"]["turn"]["status"] in ("inProgress", "completed"),
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
        if status == "INVALID_INPUT":
            original = 'Native host operation failed: Invalid("reason")'
            check(key == "V08_MODEL_EMPTY_REJECT_REASON" and content[0]["text"] ==
                  action["rawToolReceipt"] == original and written["result"]["success"] is False and
                  action["nativeRefusal"] == {"kind": "INVALID_INPUT", "original": original} and
                  "receipt" not in action,
                  "Malformed reason must retain the exact native refusal, not an invented policy receipt")
            request_id = reply["step_id"]
            receipt = None
        else:
            receipt = json.loads(content[0]["text"])
            check(content[0]["text"] == action["rawToolReceipt"] and receipt == action["receipt"] and
                  reply["step_id"] == receipt["requestId"] and written["result"]["success"] == (status == "APPLIED") and
                  receipt["schema"] == "gogoke.37.operations.v1" and receipt["family"] == "K-POLICY" and
                  receipt["operation"] == operation and receipt["targetId"] == gate and receipt["status"] == status and
                  receipt["previousRevision"] == gate_rev and receipt["revision"] == str(int(gate_rev) + (status == "APPLIED")),
                  "Actual H policy receipt differs from observation/required outcome")
            request_id = receipt["requestId"]
        tool_completions = [(row, value) for row, value in tool_items if value["method"] == "item/completed"]
        tool_starts = [(row, value) for row, value in tool_items if value["method"] == "item/started"]
        check(len(tool_completions) == 1 and tool_completions[0][1]["params"]["threadId"] == session["threadId"] and
              tool_completions[0][1]["params"]["item"]["type"] == "dynamicToolCall" and
              tool_completions[0][1]["params"]["item"]["id"] == frame["params"]["callId"] and
              tool_completions[0][1]["params"]["item"]["tool"] == "gogoke_policy" and
              tool_completions[0][1]["params"]["item"]["arguments"] == arguments,
              "Original CLI tool completion includes a substitute/additional tool")
        check(len(tool_starts) <= 1 and all(
            started["params"]["item"]["id"] == frame["params"]["callId"] and
            started["params"]["item"]["tool"] == "gogoke_policy" and
            started["params"]["item"]["arguments"] == arguments
            for _, started in tool_starts), "Original CLI tool start includes an additional tool")
        completed_item = tool_completions[0][1]["params"]["item"]
        # Retain the same original output field precedence as native codex_output.
        completed_output = next((completed_item[name] for name in
                                 ("aggregatedOutput", "result", "error", "contentItems")
                                 if completed_item.get(name) is not None), None)
        completed_content = (completed_output if isinstance(completed_output, list) else
                             completed_output.get("contentItems") if isinstance(completed_output, dict) else None)
        check(completed_item["status"] == action["cliToolStatus"] ==
              ("completed" if status == "APPLIED" else "failed") and
              completed_content == content,
              "Original CLI tool completion does not preserve the actual H result/failure")
        if status == "DENIED":
            check(receipt["result"] == {} and written["result"]["success"] is False,
                  "Denied native boundary must preserve empty result and success=false")
        events = select(db, "SELECT * FROM gogoke_v37_seat_policy_events WHERE domain_id=? AND event_id=?",
                        (domain, request_id))
        if status == "APPLIED":
            check(len(events) == 1 and events[0]["operation"] == operation and events[0]["target_id"] == gate and
                  events[0]["state"] == state and events[0]["detail"] == reason and events[0]["policy_revision"] == policy_rev and
                  receipt["result"] == {"state": state, "reason": reason, "policyRevision": str(policy_rev)},
                  "Committed actual gate/stage event differs")
            if operation == "gate-decide":
                parts = [operation, session["seatId"], gate, payload["decision"], reason, str(policy_rev), gate_rev]
            else:
                parts = [operation, session["seatId"], gate, str(policy_rev - (operation == "stage-transition")), gate_rev]
            check(events[0]["fingerprint"] == native_fingerprint(parts, bytes(source["raw_bytes"])),
                  "Original native gate/stage event is not bound to this A caller/request bytes")
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
    host_facts = [verify_host(db, domain, case, host, operations, result) for host in host_cases]
    for fact in host_facts:
        expected_events[fact["intentEvent"]["event_id"]] = fact["intentEvent"]
    check({row["event_id"]: row for row in current["events"]} == expected_events,
          "Unaccounted rule event or changed baseline event in exclusive domain")
    check(current["head"] == [{**head, "revision": head["revision"] + len(expected_config) + 1, "current_stage": case["toStage"]}],
          "Actual final legal stage/head revision differs")
    original_gates = {row["gate_id"]: row for row in before["policy"]["gates"]}
    for gate, state, gate_revision, count, reason in [(reject, "ESCALATION_REQUIRED", 5, 2, reasons[1]),
                                                    (passed, "ADVANCED", 4, 0, "")]:
        original_gates[gate] = {"domain_id": domain, "gate_id": gate, "submitter_seat_id": submitter["seatId"],
                              "reviewer_seat_id": reviewer["seatId"], "from_stage": case["fromStage"],
                              "to_stage": case["toStage"], "reject_cap": 2, "reject_count": count,
                              "state": state, "reason": reason, "revision": gate_revision}
    for host in host_cases:
        original_gates[host["gateId"]] = {"domain_id": domain, "gate_id": host["gateId"], "submitter_seat_id": submitter["seatId"],
            "reviewer_seat_id": reviewer["seatId"], "from_stage": case["toStage"], "to_stage": host["toStage"], "reject_cap": 1,
            "reject_count": 1, "state": "ESCALATION_REQUIRED", "reason": host["reason"], "revision": 3}
    check({row["gate_id"]: row for row in current["gates"]} == original_gates, "Actual gates changed beyond the prescribed scope")
    grants = {(row["caller_seat_id"], row["target_id"], row["action"]): row for row in before["policy"]["grants"]}
    grants[(submitter["seatId"], reviewer["seatId"], "REVIEW")] = {"domain_id": domain, "caller_seat_id": submitter["seatId"],
        "target_id": reviewer["seatId"], "action": "REVIEW", "expires_at_ms": 0, "revision": head["revision"] + 5}
    check({(row["caller_seat_id"], row["target_id"], row["action"]): row for row in current["grants"]} == grants,
          "Actual final grant differs or unrelated grant changed")
    check({(row["from_seat_id"], row["reason"]): row for row in current["routes"]} == expected_routes,
          "Owner routes changed beyond original case CAS operations")
    check(current["triggers"] == before["policy"]["triggers"], "Host cap must not fabricate coordinator registration")
    escalations = {row["trigger_id"]: row for row in before["policy"]["escalations"]}
    messages = {row["message_id"]: row for row in before["inbox"]["messages"]}
    inbox_operations = {row["request_id"]: row for row in before["inbox"]["operations"]}
    for fact in host_facts:
        escalations[fact["intent"]["trigger_id"]] = fact["intent"]
        messages[fact["message"]["message_id"]] = fact["message"]
        for op in [fact["enqueue"], *([fact["recipientOperation"]] if fact["recipientOperation"] else []),
                   *[row["operation"] for row in fact["recipientStages"]],
                   *[row["operation"] for row in fact["deliveries"]],
                   *([fact["cancel"]] if "cancel" in fact else [])]:
            inbox_operations[op["request_id"]] = op
    check({row["trigger_id"]: row for row in current["escalations"]} == escalations, "Extra or changed E escalation outside case causes")
    check({row["message_id"]: row for row in result["inbox"]["messages"]} == messages and
          {row["request_id"]: row for row in result["inbox"]["operations"]} == inbox_operations,
          "Original C history changed beyond prescribed Host enqueues/delivery/User cancel")
    check(len(case["userBoundaries"]) == 1, "User boundary observation missing")
    boundary = case["userBoundaries"][0]
    user = operations[boundary["requestId"]]
    check(json.loads(user["rawFrame"]) == user["request"] and user["request"]["family"] == "K-POLICY" and
          user["receipt"]["status"] == "UNSUPPORTED" and user["receipt"]["revision"] == "1" and
          boundary["authority"] == "REAL_USER_INGRESS_ONLY", "User control cannot stand for model rejection")
    result["userBoundaries"].append({"rawFrame": user["rawFrame"], "receipt": user["receipt"],
                                    "persistedNativeDenialRow": False, "authority": boundary["authority"]})
    result["notRun"] = case["notRun"]
    required_not_run = {"V08_MODEL_SUBORDINATE_OWNER", "V08_STALL_CHAIN"}
    if not foreign:
        required_not_run.add("V08_MODEL_CROSS_PROJECT")
    required_not_run |= {"V08_HOST_LATE_ACK_AFTER_ROUTE_CHANGE", "V08_HOST_BUSY_TO_IDLE_DELIVERY"} if host_cases else {"V08_REJECT_CAP_DELIVERY"}
    check({row["caseId"] for row in result["notRun"]} == required_not_run, "Unimplemented boundaries must remain explicit")
    result["verifiedCaseId"] = case["caseId"]
    result["directCaseEvidence"] = True
    result["state"] = "IMPLEMENTED_CASES_HAVE_DIRECT_EVIDENCE_V08_INCOMPLETE"


check(len(sys.argv) == 5 and sys.argv[4] in ("before", "checkpoint", "final"), "Expected state root, fresh output, original M2 journal, before|checkpoint|final")
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
          "directCaseEvidence": False, "ownerFrames": [], "seatCards": [], "modelCalls": [], "sessions": [], "userBoundaries": [], "hostSnapshots": []}
try:
    with closing(sqlite3.connect(database.as_uri() + "?mode=ro&immutable=1", uri=True)) as db:
        db.row_factory = sqlite3.Row
        db.execute("PRAGMA query_only=ON")
        result["policy"] = policy(db, journal["domainId"])
        result["foreignProject"] = foreign_snapshot(db, journal["domainId"], journal.get("foreignProject"))
        result["inbox"] = inbox_rows(db, journal["domainId"])
        check(len(result["policy"]["head"]) == 1, "Existing real Owner-initialized policy head required")
        if sys.argv[4] == "final":
            check(len(journal.get("rulesCases", [])) == 1, "One exclusive original V08 case required")
            verify_case(db, journal, journal["rulesCases"][0], result)
        elif sys.argv[4] == "checkpoint":
            check(len(journal.get("rulesCases", [])) == 1, "One original exclusive V08 checkpoint case required")
            check(journal["rulesCases"][0]["readerSha256"] == result["readerSha256"] and
                  journal["rulesCases"][0]["driverSha256"] == digest(Path(__file__).with_name("m2-rules.mjs").read_bytes()),
                  "Checkpoint reader/module differs from actual loaded bytes")
            result["hostSnapshots"] = [host_snapshot(db, journal["domainId"], row)
                                       for row in journal["rulesCases"][0]["hostCases"] if row.get("causeEventId")]
            case = journal["rulesCases"][0]
            identifiers = {case["submitterSession"], case["reviewerSession"]}
            identifiers.update(row["busy"]["binding"]["id"] for row in case["hostCases"] if row.get("busy"))
            identifiers.update(row["autoObservation"]["sessionId"] for row in case["hostCases"] if row.get("autoObservation"))
            result["stoppedClaims"] = [one(db, "SELECT * FROM gogoke_v37_h_claim WHERE domain_id=? AND session_id=?",
                                          (journal["domainId"], session)) for session in sorted(identifiers)]
            operations = {row["request"].get("requestId"): row for row in journal["operations"]}
            result["checkpointStops"] = verify_checkpoint_stops(db, journal["domainId"], case, case["hostCases"][-1],
                                                               operations, result["stoppedClaims"])
            result["state"] = "ORIGINAL_HOST_SNAPSHOT_NOT_A_FINAL_V08_RESULT"
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
