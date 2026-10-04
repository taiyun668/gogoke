"""Original H/A/D/F evidence after normal candidate close; no CLI or model calls.

signed Python m2-history-boundaries-readback.py STATE_ROOT NEW_OUTPUT JOURNAL before-refusal|final|peer-final
Only candidate state.sqlite and explicitly named private evidence artifacts are read.
With peerRead requested, opens only exact original test thread/start JSONL paths
after registered-home and H/A/F checks. No credentials or active WAL. No DB writes.
"""
import hashlib
import json
import os
import sqlite3
import sys
from contextlib import closing
from pathlib import Path


def check(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def rows(db, sql, args=()):
    return [dict(row) for row in db.execute(sql, args)]


def one(db, sql, args=()):
    found = rows(db, sql, args)
    check(len(found) == 1, f"One original row required, found {len(found)}: {sql}")
    return found[0]


def rpc_id(value):
    check(type(value) in (int, str), "RPC ID must preserve its original number/string type")
    return type(value).__name__, value


def decode_hex(value):
    return json.loads(bytes.fromhex(value))


def path_spelling(value):
    # Same spelling rule as LaunchEvidence::verify_observed_cwd, no file reads.
    value = value.replace("/", "\\")
    return (value[4:] if value.startswith("\\\\?\\") else value).lower()


def snapshot(db, sessions, cases):
    # Compare scoped production rows, never synthesize fixture/golden records.
    result = {}
    for table in ("gogoke_v37_h_claim", "gogoke_v37_h_operation", "gogoke_v37_h_process_episode",
                  "gogoke_v37_h_generation", "gogoke_v37_h_stdin_journal", "gogoke_v37_rpc_steps",
                  "v37_ledger_raw_source", "v37_ledger_index", "v37_ledger_session"):
        result[table] = []
        for session in sessions:
            result[table].extend(rows(db, f"SELECT * FROM {table} WHERE domain_id=? AND session_id=? ORDER BY rowid",
                                     (session["domainId"], session["id"])))
    result["seats"] = [one(db, "SELECT * FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?", (domain, seat))
                       for domain, seat in sorted({(session["domainId"], session["seatId"]) for session in sessions})]
    result["worktrees"] = [one(db, "SELECT * FROM gogoke_v37_worktrees WHERE domain_id=? AND worktree_id=?", (domain, tree))
                           for domain, tree in sorted({(session["domainId"], session["worktreeId"]) for session in sessions})]
    result["processCustody"] = [one(db, "SELECT * FROM gogoke_coordination_process_custody WHERE operation_id=?",
                                   (episode["process_operation_id"],))
                               for episode in result["gogoke_v37_h_process_episode"]]
    for table in ("gogoke_v37_side_registry", "gogoke_v37_side_sync", "gogoke_v37_side_pending"):
        result[table] = []
        for case in cases:
            result[table].extend(rows(db, f"SELECT * FROM {table} WHERE domain_id=? AND side_id=? ORDER BY rowid",
                                     (case["sideBinding"]["domainId"], case["sideId"])))
    return result


def serializable(value):
    # Exact BLOB bytes stay in private evidence; never JSON-reserialize a source.
    if isinstance(value, bytes):
        return {"blobHex": value.hex()}
    if isinstance(value, dict):
        return {key: serializable(item) for key, item in value.items()}
    if isinstance(value, list):
        return [serializable(item) for item in value]
    return value


def session_evidence(db, session, case, operations, peer=None):
    domain = session["domainId"]
    sid = session["id"]
    registration = one(db, "SELECT * FROM v37_ledger_session WHERE domain_id=? AND session_id=?", (domain, sid))
    check(registration["seat_id"] == session["seatId"] and registration["purpose"] == session["purpose"] and
          registration["side_id"] == (case["sideId"] if session["purpose"] == "SIDE_CHAT" else None),
          "Original A registration differs from actual session purpose/seat")
    episode = one(db, "SELECT * FROM gogoke_v37_h_process_episode WHERE domain_id=? AND session_id=?", (domain, sid))
    check(episode["phase"] == "STOPPED" and episode["stop_fact_id"] == session["stopFact"] and
          episode["instance_id"] == case["instanceId"] and episode["seat_id"] == session["seatId"] and
          episode["generation"] == session["generation"] and not episode["old_generation"],
          "Actual history episode is not one original fresh stopped H process")
    tree = one(db, "SELECT * FROM gogoke_v37_worktrees WHERE domain_id=? AND worktree_id=?", (domain, session["worktreeId"]))
    check(tree["repository_id"] == session["repositoryId"] and tree["seat_id"] == session["seatId"] and
          tree["instance_id"] == case["instanceId"] and tree["state"] == "REGISTERED",
          "Original F repository/seat/instance binding differs")
    custody = one(db, "SELECT * FROM gogoke_coordination_process_custody WHERE operation_id=?", (episode["process_operation_id"],))
    pin = one(db, "SELECT * FROM gogoke_v37_instances WHERE instance_id=?", (case["instanceId"],))
    check(pin["driver_id"] == case["driverId"] and pin["version"] == case["version"] and
          pin["program_digest"] == custody["binary_digest_sha256"] == "sha256:" + case["sha256"] and
          custody["domain_id"] == domain and custody["generation"] == session["generation"] and
          custody["state"] == "STOPPED" and custody["stop_proof_hash"] == episode["stop_fact_id"],
          "Actual CLI digest/generation/physical stop custody differs from fixed F pin")
    incoming = rows(db, "SELECT * FROM v37_ledger_raw_source WHERE domain_id=? AND session_id=? AND operation_id=? ORDER BY rowid",
                    (domain, sid, episode["process_operation_id"]))
    outgoing = rows(db, "SELECT * FROM gogoke_v37_rpc_steps WHERE domain_id=? AND session_id=? AND process_operation_id=? ORDER BY rowid",
                    (domain, sid, episode["process_operation_id"]))
    check(incoming and outgoing and all(row["generation"] == session["generation"] and
          row["process_ticket"] == custody["ticket"] and row["custodian_nonce"] == custody["custodian_nonce"]
          and row["state"] != "PENDING" for row in incoming), "Original A source custody is incomplete or changed")
    check(all(row["generation"] == session["generation"] and row["ticket"] == custody["ticket"] and
              row["custodian_nonce"] == custody["custodian_nonce"] and row["phase"] in ("WRITTEN", "OBSERVED")
              for row in outgoing), "Original H command custody or write evidence incomplete")
    for epoch in {row["source_epoch"] for row in incoming}:
        cursors = sorted(int(row["source_cursor"]) for row in incoming if row["source_epoch"] == epoch)
        check(cursors == list(range(1, max(cursors) + 1)), "Original raw source stream has a capture gap")
    decoded = [(row, json.loads(row["raw_bytes"])) for row in incoming]
    commands = [(row, decode_hex(row["command_hex"])) for row in outgoing]
    open_request = case["sideOpenRequest"] if session["purpose"] == "SIDE_CHAT" else operations[session["openRequestId"]]["request"]
    check(decode_hex(episode["raw_hex"]) == open_request and episode["request_id"] == open_request["requestId"] and
          open_request["domainId"] == domain and
          open_request["payload"] == {"generation": session["generation"], "seatId": session["seatId"],
              "repositoryId": session["repositoryId"], "worktreeId": session["worktreeId"],
              **({"purpose": "FORMAL_REVIEW"} if session["purpose"] == "FORMAL_REVIEW" else {})},
          "Actual original H open input is not the expected zero-lineage registration")
    native_id, native_path, effective_memory_configuration = None, None, None
    start_method = "thread/start" if case["driverId"] == "codex" else "session/new"
    if case["driverId"] != "claude":
        starts = [(row, command) for row, command in commands if command.get("method") == start_method]
        check(len(starts) == 1 and starts[0][0]["phase"] == "OBSERVED", "One fresh original native start required")
        step, command = starts[0]
        check(path_spelling(command["params"]["cwd"]) == path_spelling(tree["worktree_path"]),
              "Actual native cwd is another F worktree")
        replies = [(row, frame) for row, frame in decoded if "method" not in frame and "id" in frame and
                   rpc_id(frame["id"]) == rpc_id(command["id"])]
        check(len(replies) == 1 and replies[0][0]["source_epoch"] == step["source_epoch"] and
              replies[0][0]["source_cursor"] == step["source_cursor"] and "error" not in replies[0][1],
              "Fresh native start has no original typed RPC ACK")
        ack = replies[0][1]["result"]
        native_id = ack["thread"]["id"] if case["driverId"] == "codex" else ack["sessionId"]
        if case["driverId"] == "codex":
            check(native_id == session["threadId"], "Original Codex native thread identity differs")
            native_path = ack["thread"].get("path")  # Export source only; do not open or guess a path.
            configs = [(row, frame) for row, frame in commands if frame.get("method") == "config/read"]
            check(len(configs) == 1 and configs[0][0]["phase"] == "OBSERVED" and
                  path_spelling(configs[0][1]["params"]["cwd"]) == path_spelling(tree["worktree_path"]),
                  "Actual Codex config observation lacks its original H binding")
            config_step, config_command = configs[0]
            replies = [(row, frame) for row, frame in decoded if "method" not in frame and "id" in frame and
                       rpc_id(frame["id"]) == rpc_id(config_command["id"])]
            check(len(replies) == 1 and replies[0][0]["source_epoch"] == config_step["source_epoch"] and
                  replies[0][0]["source_cursor"] == config_step["source_cursor"] and "error" not in replies[0][1],
                  "Actual Codex config has no original typed H/A response")
            config_source, config_reply = replies[0]
            config_value = config_reply["result"]["config"]
            check(config_value["features"]["memories"] is False and
                  config_value["memories"]["generate_memories"] is False and
                  config_value["memories"]["use_memories"] is False,
                  "Actual Codex effective memory configuration is not disabled")
            effective_memory_configuration = {"source": "ORIGINAL_H_A_CONFIG_READ",
                "sourceEpoch": config_source["source_epoch"], "sourceCursor": config_source["source_cursor"],
                "featureMemories": False, "generateMemories": False, "useMemories": False}
    check(not any(command.get("method") in ("thread/resume", "thread/fork", "session/load", "session/resume")
                  for _, command in commands), "Fresh history session inherited native context")
    check(len(session["inputs"]) == 1, "Exactly one genuine marker turn per original history session" if peer is None
          else "Exactly one genuine file-read turn per independent peer session")
    input_row = session["inputs"][0]
    stdin = one(db, "SELECT * FROM gogoke_v37_h_stdin_journal WHERE domain_id=? AND session_id=? AND request_id=?",
                (domain, sid, input_row["requestId"]))
    original_send = decode_hex(stdin["request_hex"])
    original_body = original_send["payload"]["body"]
    check(stdin["phase"] == "RECEIPTED" and stdin["receipt_status"] == "APPLIED" and
          stdin["process_operation_id"] == episode["process_operation_id"] and
          stdin["generation"] == session["generation"] and stdin["ticket"] == custody["ticket"] and
          stdin["custodian_nonce"] == custody["custodian_nonce"] and original_send["operation"] == "send" and
          original_send["targetId"] == sid and original_send["domainId"] == domain,
          "Original H input does not belong to this stopped physical process")
    if input_row.get("sideComposition"):
        check(original_body.rsplit("\nExplicit user question:\n", 1)[-1] ==
              json.dumps(input_row["body"], ensure_ascii=False, separators=(",", ":")),
              "Original D assembled input lost the exact explicit side question")
        sync = one(db, "SELECT * FROM gogoke_v37_side_sync WHERE domain_id=? AND sync_id=?", (domain, input_row["requestId"]))
        check(sync["state"] == "DELIVERED" and sync["session_id"] == sid and sync["side_id"] == case["sideId"] and
              sync["mode"] == "QUESTION" and sync["origin_request_digest"] ==
              digest(json.dumps(case["sideQuestionRequest"], separators=(",", ":"), ensure_ascii=False).encode()),
              "Actual D sync lacks its original user-question byte identity")
    else:
        operation = operations[input_row["requestId"]]
        check(bytes.fromhex(stdin["request_hex"]).decode() == operation["rawFrame"] and
              original_body == input_row["body"], "Original User input bytes changed")
    sent = decode_hex(stdin["receipt_hex"])
    check(sent["status"] == "APPLIED" and sent["result"]["createdTurn"] is True,
          "Original H receipt does not prove a real native marker turn")
    if case["driverId"] == "codex":
        starts = [(row, frame) for row, frame in commands if frame.get("method") == "turn/start" and
                  frame.get("params", {}).get("input") == [{"type": "text", "text": original_body}]]
        check(len(starts) == 1 and starts[0][0]["phase"] == "OBSERVED" and
              starts[0][1]["params"]["threadId"] == native_id, "Original H turn/start input differs")
        turn = input_row["turnId"]
        start_step, start_command = starts[0]
        acknowledgements = [(row, frame) for row, frame in decoded if "method" not in frame and "id" in frame and
                            rpc_id(frame["id"]) == rpc_id(start_command["id"])]
        check(len(acknowledgements) == 1 and
              acknowledgements[0][0]["source_epoch"] == start_step["source_epoch"] and
              acknowledgements[0][0]["source_cursor"] == start_step["source_cursor"] and
              acknowledgements[0][1].get("result", {}).get("turn", {}).get("id") == turn,
              "Original typed Codex turn ACK is not bound to the exact H input")
        check(sent["result"]["turnId"] == turn and any(frame.get("method") == "turn/completed" and
              frame.get("params", {}).get("threadId") == native_id and
              frame.get("params", {}).get("turn", {}).get("id") == turn and
              frame["params"]["turn"]["status"] == "completed" for _, frame in decoded),
              "Exact Codex native marker turn has no original completion")
        check(peer is not None or not any(frame.get("method", "").startswith("item/tool/") or
              (frame.get("method") in ("item/started", "item/completed") and
               frame.get("params", {}).get("item", {}).get("type") not in
               ("agentMessage", "reasoning", "contextCompaction")) for _, frame in decoded),
              "No-tool history marker induced a tool or unrelated activity")
    elif case["driverId"] == "claude":
        check(sent["result"]["deliveryBasis"] == "CLAUDE_USER_REPLAY_AND_RESULT", "Claude original terminal basis differs")
        native_id = sent["result"]["vendorSessionId"]
        writes = [(row, frame) for row, frame in commands if frame.get("type") == "user" and
                  frame.get("message", {}).get("content") == [{"type": "text", "text": original_body}]]
        check(len(writes) == 1 and writes[0][0]["phase"] == "OBSERVED", "Original Claude H input/echo missing")
        echoes = [frame for _, frame in decoded if frame.get("type") == "user" and
                  frame.get("uuid") == writes[0][1].get("uuid") and frame.get("session_id") == native_id and
                  frame.get("message", {}).get("content") == [{"type": "text", "text": original_body}]]
        check(len(echoes) == 1, "Original Claude user echo is not exact")
    else:
        check(sent["result"]["deliveryBasis"] == "ACP_PROMPT_RESPONSE", "ACP original terminal basis differs")
        writes = [(row, frame) for row, frame in commands if frame.get("method") == "session/prompt" and
                  frame.get("params", {}).get("sessionId") == native_id and
                  frame.get("params", {}).get("prompt") == [{"type": "text", "text": original_body}]]
        check(len(writes) == 1 and writes[0][0]["phase"] == "OBSERVED", "Original ACP native prompt missing")
    if case["driverId"] != "codex":
        terminal = [(row, frame) for row, frame in decoded if row["source_epoch"] == sent["result"]["sourceEpoch"] and
                    row["source_cursor"] == sent["result"]["sourceCursor"]]
        check(len(terminal) == 1, "Original provider terminal source missing")
        source, frame = terminal[0]
        if case["driverId"] == "claude":
            check(digest(source["raw_bytes"]) == sent["result"]["rawResultSha256"] and
                  frame.get("type") == "result" and frame.get("subtype") == "success" and
                  frame.get("is_error") is False and frame.get("session_id") == native_id,
                  "Original Claude result differs from H receipt")
        else:
            check(digest(source["raw_bytes"]) == sent["result"]["rawResponseSha256"] and "error" not in frame and
                  frame.get("result", {}).get("stopReason") == "end_turn" and
                  rpc_id(frame["id"]) == rpc_id(writes[0][1]["id"]), "Original ACP terminal ACK differs")
    normalized = rows(db, "SELECT * FROM v37_ledger_index WHERE domain_id=? AND session_id=? ORDER BY cursor", (domain, sid))
    resolved = {row["resolved_event_id"]: row for row in incoming if row["state"] == "RESOLVED"}
    check(normalized and all(row["source_kind"] == "v37" and row["source_event_id"] in resolved and
              row["source_epoch"] == resolved[row["source_event_id"]]["source_epoch"] for row in normalized),
              "Normalized assistant history is not backed by original physical A sources")
    check(peer is not None or not any(json.loads(row["update_json"]).get("sessionUpdate") in ("tool_call", "tool_call_update")
                  for row in normalized), "No-tool history marker has actual tool activity")
    assistant_text = "".join(update.get("content", {}).get("text", "")
                             for update in (json.loads(row["update_json"]) for row in normalized)
                             if update.get("sessionUpdate") == "agent_message_chunk")
    check(peer is not None or input_row["marker"] in assistant_text,
              "Non-secret marker is absent from actual assistant A events; no empty history control")
    return {"domainId": domain, "sessionId": sid, "nativeSessionId": native_id, "originalCodexThreadPath": native_path,
            "episode": episode, "custody": custody, "registration": registration, "worktree": tree,
            "stdin": stdin, "rawSource": incoming, "rpcSteps": outgoing, "normalized": normalized,
            "effectiveMemoryConfiguration": effective_memory_configuration, "instance": pin,
            "originalBody": original_body, "marker": input_row.get("marker")}


def vendor_objects(root, cases):
    """Only exact thread/start paths from the four original no-tool test sessions."""
    objects, not_run = [], []
    for case in cases:
        if case["driverId"] != "codex":
            continue
        for source in case["sessions"]:
            value = source["originalCodexThreadPath"]
            identity = {"caseId": case["caseId"], "sessionId": source["sessionId"]}
            if not isinstance(value, str) or not value:
                not_run.append({**identity, "state": "NOT_RUN_ORIGINAL_THREAD_PATH_MISSING"})
                continue
            original = Path(value)
            pin = source["instance"]
            home = root / "v37-instances" / source["episode"]["instance_id"]
            try:
                # Registry resolver's existing home_ref/layout, not a guessed history path.
                # H uses GetFinalPathNameByHandleW's extended local spelling;
                # compare the same DOS spelling while opening the original path.
                local = Path(value[4:] if value.startswith("\\\\?\\") else value)
                home_value = str(home)
                local_home = Path(home_value[4:] if home_value.startswith("\\\\?\\") else home_value)
                check(os.name == "nt" and sys.version_info >= (3, 12) and
                      original.is_absolute() and original.suffix == ".jsonl" and
                      local.is_absolute() and ".." not in local.parts and
                      not any(":" in part for part in local.parts[1:]) and not local.drive.startswith("\\\\") and
                      pin["home_ref"] == "instance-home-" + pin["instance_id"] and
                      local.is_relative_to(local_home) and local != local_home,
                      "Original test thread path is outside its registered candidate home")
                for entry in (original, *original.parents):
                    check(not entry.lstat().st_file_attributes & 0x400, "Original thread path traverses a reparse point")
                    if path_spelling(str(entry)) == path_spelling(str(root)):
                        break
                stat = home.stat()
                observed_home = f"volume:{stat.st_dev:016x}/file:{stat.st_ino.to_bytes(16, 'little').hex()}"
                check(observed_home == pin["home_identity"], "Python stat cannot confirm original registered home identity")
                check(original.is_file(), "Original thread path is not a file")
                before = original.stat()
                check(before.st_nlink == 1, "Original vendor history object has another link")
                raw = original.read_bytes()
                frames = [json.loads(line) for line in raw.splitlines() if line.strip()]
                check(frames and frames[0].get("type") == "session_meta" and
                      frames[0].get("payload", {}).get("id") == source["nativeSessionId"] and
                      path_spelling(frames[0]["payload"]["cwd"]) == path_spelling(source["worktree"]["worktree_path"]),
                      "Original vendor session_meta differs from unique H/A/F identity")
                assistant = [frame["payload"] for frame in frames if frame.get("type") == "response_item" and
                             frame.get("payload", {}).get("type") == "message" and frame["payload"].get("role") == "assistant"]
                check(any(source["marker"] in content.get("text", "") for message in assistant
                          for content in message.get("content", []) if isinstance(content, dict)),
                      "Original vendor object lacks the actual non-secret assistant marker")
                after = original.stat()
                check((before.st_dev, before.st_ino, before.st_nlink, before.st_size, before.st_mtime_ns) ==
                      (after.st_dev, after.st_ino, after.st_nlink, after.st_size, after.st_mtime_ns) and digest(original.read_bytes()) == digest(raw),
                      "Original vendor object changed during normal-close readback")
                objects.append({**identity, "domainId": source["domainId"], "path": value,
                    "nativeSessionId": source["nativeSessionId"], "instanceId": pin["instance_id"],
                    "homeIdentity": pin["home_identity"], "fileIdentity": [str(before.st_dev), str(before.st_ino)],
                    "sha256": digest(raw), "marker": source["marker"], "sessionMeta": frames[0],
                    "state": "ORIGINAL_TEST_VENDOR_OBJECT_READ_BACK"})
            except (OSError, ValueError, KeyError, RuntimeError) as error:
                not_run.append({**identity, "state": "NOT_RUN_ORIGINAL_VENDOR_OBJECT_UNQUALIFIED", "originalError": repr(error)})
    return objects, not_run


def verify_peers(db, journal, boundary, result):
    peer = boundary["peerRead"]
    check(peer["state"] == "PEER_FLOW_COMPLETE_DIRECT_READBACK_REQUIRED", "Peer original flow incomplete")
    baseline_path = Path(boundary["evidenceDirectory"]) / boundary["baselineReadback"]["file"]
    check(digest(baseline_path.read_bytes()) == boundary["baselineReadback"]["sha256"], "Original source readback changed")
    baseline = json.loads(baseline_path.read_text(encoding="utf-8-sig"))
    originals = baseline["verifiedVendorObjects"]
    operations = {entry["request"]["requestId"]: entry for entry in journal["operations"] if entry.get("request", {}).get("requestId")}
    result["peerReads"] = []
    native_ids = set()
    for attempt in peer["attempts"]:
        case = next(case for case in boundary["cases"] if case["caseId"] == attempt["caseId"])
        session = next(session for session in journal["sessions"] if session["id"] == attempt["sessionId"])
        target = next(value for value in originals if value["sessionId"] == attempt["sourceSessionId"])
        current = next((value for value in result["verifiedVendorObjects"] if value["sessionId"] == target["sessionId"]), None)
        check(current == target, "Exact original vendor object or identity changed after peer read")
        check(session["id"] not in case["projectSessions"] + [case["formalSessionId"], case["sideSessionId"]] and
              session["instanceId"] == target["instanceId"] == case["instanceId"] and
              session["purpose"] == attempt["purpose"] and
              all(session[key] == (case["projectB"] if attempt["purpose"] == "WORK" else case["sideBinding"])[key]
                  for key in ("domainId", "repositoryId", "seatId", "worktreeId")), "Peer is not an independent original test H/F session")
        check(attempt["command"] == "Get-Content -LiteralPath '" + target["path"].replace("'", "''") + "' -Raw -ErrorAction Stop" and
              session["inputs"][0]["body"] == attempt["body"] and attempt["command"] in attempt["body"],
              "Peer H input does not request the ordinary exact-object read")
        observed = session_evidence(db, session, case, operations, peer=attempt)
        check(observed["nativeSessionId"] not in [original["nativeSessionId"] for original_case in baseline["cases"]
              if original_case["caseId"] == case["caseId"] for original in original_case["sessions"]], "Peer inherited original native identity")
        check(observed["nativeSessionId"] not in native_ids, "Independent peer scopes reused one native identity")
        native_ids.add(observed["nativeSessionId"])
        turn = session["inputs"][0]["turnId"]
        decoded = [(row, json.loads(row["raw_bytes"])) for row in observed["rawSource"]]
        items = [(row, frame) for row, frame in decoded if frame.get("method") == "item/completed" and
                 frame.get("params", {}).get("threadId") == observed["nativeSessionId"] and
                 frame["params"].get("turnId") == turn and frame["params"].get("item", {}).get("type") == "commandExecution"]
        exact = [(row, frame) for row, frame in items if frame["params"]["item"].get("command") == attempt["command"]]
        item_ids = {frame["params"]["item"].get("id") for _, frame in exact}
        check(not any(frame.get("method") == "item/tool/call" or
              (frame.get("method") in ("item/started", "item/completed") and frame.get("params", {}).get("turnId") == turn and
               frame["params"].get("item", {}).get("type") not in ("agentMessage", "reasoning", "commandExecution"))
              for _, frame in decoded), "Peer performed an unrelated tool action; preserve original run")
        check(len(items) <= 1, "Peer repeated or substituted an ordinary file read")
        commands = [frame["params"]["item"] for _, frame in decoded if frame.get("method") in ("item/started", "item/completed")
                    and frame.get("params", {}).get("turnId") == turn and
                    frame["params"].get("item", {}).get("type") == "commandExecution"]
        fact = {"sessionId": session["id"], "purpose": session["purpose"], "target": target,
                "evidence": observed, "originalToolItems": [{"source": row, "frame": frame} for row, frame in items],
                "state": "NOT_RUN_ORIGINAL_EXACT_TOOL_OR_REFUSAL_CODE_MISSING", "directDeniedRead": False}
        if len(exact) == 1:
            source, frame = exact[0]
            item = frame["params"]["item"]
            started = [(row, frame) for row, frame in decoded if frame.get("method") == "item/started" and
                       frame.get("params", {}).get("threadId") == observed["nativeSessionId"] and
                       frame["params"].get("turnId") == turn and frame["params"].get("item", {}).get("id") in item_ids and
                       frame["params"]["item"].get("type") == "commandExecution" and
                       frame["params"]["item"].get("command") == attempt["command"]]
            output = item.get("aggregatedOutput")
            check(not isinstance(output, str) or target["marker"] not in output,
                  "Original source marker leaked through actual peer tool output")
            if (len(started) == 1 and len(commands) == 2 and item.get("status") in ("completed", "failed") and
                type(item.get("exitCode")) is int and item["exitCode"] != 0 and
                isinstance(output, str) and "GetContentReaderUnauthorizedAccessError" in output and
                "UnauthorizedAccessException" in output and "PermissionDenied" in output and
                not any(code in output for code in ("PathNotFound", "ItemNotFoundException", "ObjectNotFound"))):
                fact.update(state="ORIGINAL_TOOL_EXACT_READ_DENIED", directDeniedRead=True,
                            originalErrorId="GetContentReaderUnauthorizedAccessError", originalExitCode=item["exitCode"])
            elif item.get("exitCode") == 0:
                fact["state"] = "FAIL_ORIGINAL_TOOL_READ_SUCCEEDED"
                result["peerReads"].append(fact)
                raise RuntimeError("Original peer ordinary read succeeded; scope denial is false")
        result["peerReads"].append(fact)
    # No missing configured case, missing tool, generic exit=1 or model denial can pass.
    expected = {case["caseId"] for case in boundary["cases"] if case["driverId"] == "codex" and
                case["state"] == "FLOW_COMPLETE_DIRECT_READBACK_REQUIRED"}
    complete = {(attempt["caseId"], attempt["purpose"]) for attempt in peer["attempts"]}
    result["directPeerReadEvidence"] = bool(expected) and complete == {(key, purpose) for key in expected
        for purpose in ("WORK", "FORMAL_REVIEW")} and len(peer["attempts"]) == 2 * len(expected) and all(
        fact["directDeniedRead"] for fact in result["peerReads"])
    result["peerState"] = "ORIGINAL_PEER_READ_DENIAL_FACTS_COMPLETE_ACCEPTANCE_FALSE" if result["directPeerReadEvidence"] else "NOT_RUN_PEER_READ_DENIAL_UNQUALIFIED"


def verify_flow(db, journal, boundary, result):
    check(boundary["readerSha256"] == digest(Path(__file__).read_bytes()) and
          boundary["driverSha256"] == digest(Path(__file__).with_name("m2-history-boundaries.mjs").read_bytes()),
          "Readback must inspect the actual loaded history driver/reader bytes")
    cases = [case for case in boundary["cases"] if case["state"] in
             ("FLOW_COMPLETE_BEFORE_REFUSALS", "FLOW_COMPLETE_DIRECT_READBACK_REQUIRED")]
    check(cases, "No completed actual history flow; NOT_RUN is not success")
    operations = {entry["request"]["requestId"]: entry for entry in journal["operations"] if entry.get("request", {}).get("requestId")}
    sessions = []
    for case in cases:
        ids = case["projectSessions"] + [case["sideSessionId"], case["formalSessionId"]]
        owned = [session for session in journal["sessions"] if session["id"] in ids]
        check(len(owned) == 4 and [session["purpose"] for session in owned] == ["WORK", "WORK", "SIDE_CHAT", "FORMAL_REVIEW"],
              "Four original H sessions with their actual purposes required")
        for session, binding in zip(owned, (case["projectA"], case["projectB"], case["sideBinding"], case["sideBinding"])):
            check(all(session[key] == binding[key] for key in ("domainId", "repositoryId", "seatId", "worktreeId")),
                  "Original session domain/F binding differs from its configured test object")
        observed = [session_evidence(db, session, case, operations) for session in owned]
        check(len({row["nativeSessionId"] for row in observed}) == 4 and all(row["nativeSessionId"] for row in observed),
              "Original native sessions reused/forked prior context identity")
        a, b, side, formal = observed
        check(a["domainId"] != b["domainId"] and
              a["worktree"]["repository_id"] == b["worktree"]["repository_id"] == journal["repositoryId"] and
              a["episode"]["instance_id"] == b["episode"]["instance_id"] == case["instanceId"],
              "Two actual project domains must share the identical pinned instance and authorized test repository")
        check(a["domainId"] == side["domainId"] == formal["domainId"] and
              owned[2]["seatId"] == owned[3]["seatId"] and owned[2]["worktreeId"] == owned[3]["worktreeId"] and
              owned[2]["seatId"] != owned[0]["seatId"] and len({session["worktreeId"] for session in owned}) == 3 and
              int(owned[2]["generation"]) < int(owned[3]["generation"]),
              "SideChat and fresh formal review must reuse the actual same seat through distinct generations")
        check(b["marker"] not in a["originalBody"] and a["marker"] not in b["originalBody"],
              "Original double-project H prompts contain another project's private marker")
        forbidden = [row["marker"] for row in observed[:3]]
        check(all(marker not in formal["originalBody"] and
                  all(marker.encode() not in bytes.fromhex(step["command_hex"]) for step in formal["rpcSteps"])
                  for marker in forbidden), "Fresh formal original H commands contain prior project/side private bytes")
        registry = one(db, "SELECT * FROM gogoke_v37_side_registry WHERE domain_id=? AND side_id=?",
                       (side["domainId"], case["sideId"]))
        check(registry["session_id"] == side["sessionId"] and registry["source_session_id"] == a["sessionId"] and
              registry["seat_id"] == owned[2]["seatId"], "Actual D registry does not bind original source/side")
        sessions.extend(owned); result["cases"].append({"caseId": case["caseId"], "driverId": case["driverId"],
            "projectDomains": [a["domainId"], b["domainId"]], "sideDomain": side["domainId"],
            "actualHInputIsolation": True, "actualSideThenFreshFormal": True, "sessions": observed,
            "V04b": "NOT_RUN_EFFECTIVE_VENDOR_MEMORY_AND_INSTRUCTION_PROVENANCE_MISSING",
            "V10": "NOT_RUN_VENDOR_HISTORY_PROVENANCE_MISSING_ACTUAL_NATIVE_FLOW_READ_BACK"})
    result["scopedSnapshot"] = serializable(snapshot(db, sessions, cases))
    result["directFlowEvidence"] = True


def main():
    check(len(sys.argv) == 5 and sys.argv[4] in ("before-refusal", "final", "peer-final"), "Expected candidate root, new output, original journal and phase")
    root = Path(sys.argv[1]).resolve(strict=True)
    output = Path(sys.argv[2]).resolve(strict=False)
    check(not output.exists() and not output.is_relative_to(root), "Fresh private evidence must stay outside candidate state")
    journal_file = Path(sys.argv[3]).resolve(strict=True)
    journal = json.loads(journal_file.read_text(encoding="utf-8-sig"))
    boundary = journal["historyBoundary"]
    check(journal["schema"] == "gogoke.37.m2-win11-e2e.v1" and journal["repositoryId"] == "gogokeSeatTestbed" and
          journal["acceptance"] is False and Path(boundary["stateRoot"]).resolve(strict=True) == root and
          output.parent == journal_file.parent == Path(boundary["evidenceDirectory"]).resolve(strict=True),
          "Original M2 testbed journal must explicitly bind the candidate state root")
    launch, close = journal["launches"][-1], journal["closes"][-1]
    check(launch["pid"] == close["pid"] == journal["currentEndpoint"]["pid"] and close["exitCode"] == 0 and
          close["forceKill"] is False and launch["sourceCommit"] == journal["sourceCommit"],
          "Latest exact installed candidate lacks an original normal-close receipt")
    database = root / "state.sqlite"
    wal, shm = Path(str(database) + "-wal"), Path(str(database) + "-shm")
    check(database.is_file() and (not wal.exists() or wal.stat().st_size == 0), "Normal close/checkpoint required; active WAL refused")
    def files():
        return {file.name: {"length": file.stat().st_size, "sha256": digest(file.read_bytes())}
                for file in (database, wal, shm) if file.exists()}
    result = {"schema": "gogoke.37.private-m2-history-readback.v1", "phase": sys.argv[4],
              "caseId": journal["caseId"], "sourceCommit": journal["sourceCommit"], "domainId": journal["domainId"],
              "readerSha256": digest(Path(__file__).read_bytes()), "normalClose": close,
              "acceptance": False, "databaseWrites": False, "credentialReads": False,
              "filesBefore": files(), "cases": [], "directFlowEvidence": False,
              "directRefusalEvidence": False, "notRun": boundary["notRun"]}
    result["directPeerReadEvidence"] = False
    try:
        with closing(sqlite3.connect(database.as_uri() + "?mode=ro&immutable=1", uri=True)) as db:
            db.row_factory = sqlite3.Row
            db.execute("PRAGMA query_only=ON")
            verify_flow(db, journal, boundary, result)
            result["verifiedVendorObjects"], result["vendorObjectNotRun"] = vendor_objects(root, result["cases"]) \
                if boundary.get("peerReadRequested") else ([], [{"state": "NOT_RUN_PEER_READ_NOT_CONFIGURED"}])
            if sys.argv[4] == "final":
                check(boundary["state"] == "FLOW_COMPLETE_DIRECT_READBACK_REQUIRED", "Actual history refusal flow incomplete")
                reference = boundary["baselineReadback"]
                check(Path(reference["file"]).name == reference["file"], "Baseline must name one private artifact")
                baseline_path = output.parent / reference["file"]
                check(digest(baseline_path.read_bytes()) == reference["sha256"], "Original baseline artifact bytes changed")
                baseline = json.loads(baseline_path.read_text(encoding="utf-8-sig"))
                check(baseline["phase"] == "before-refusal" and baseline["caseId"] == result["caseId"] and
                      baseline["sourceCommit"] == result["sourceCommit"] and baseline["directFlowEvidence"] is True and
                      baseline["measurementPreservedDatabaseBytes"] is True and
                      baseline["scopedSnapshot"] == result["scopedSnapshot"],
                      "Formal refusal produced new H/A/D/F rows or changed original test objects")
                formal_ids = {(case["sideBinding"]["domainId"], case["formalSessionId"]) for case in boundary["cases"]
                              if case["state"] == "FLOW_COMPLETE_DIRECT_READBACK_REQUIRED"}
                expected = {(domain, sid, verb) for domain, sid in formal_ids for verb in ("resume", "reconnect", "compact", "renew-session", "open")}
                check(len(boundary["refusals"]) == len(expected) and
                      {(refusal["domainId"], refusal["sessionId"], refusal["operation"]) for refusal in boundary["refusals"]} == expected,
                      "Incomplete actual formal resume/fork controls")
                operations = {entry["request"]["requestId"]: entry for entry in journal["operations"] if entry.get("request", {}).get("requestId")}
                for refusal in boundary["refusals"]:
                    entry = operations[refusal["requestId"]]
                    req, reply = entry["request"], entry["receipt"]
                    check(json.loads(entry["rawFrame"]) == req and reply == refusal["receipt"] and
                          reply["requestId"] == req["requestId"] and reply["status"] == "DENIED" and
                          req["targetId"] == reply["targetId"] == refusal["sessionId"] and
                          req["domainId"] == refusal["domainId"] and
                          req["operation"] == reply["operation"] == refusal["operation"] and
                          reply["previousRevision"] == reply["revision"] == req["expectedRevision"],
                          "Original User ingress formal refusal differs from exact sent request")
                result["directRefusalEvidence"] = True
            if sys.argv[4] == "peer-final":
                verify_peers(db, journal, boundary, result)
            result["state"] = "DIRECT_FACTS_COMPLETE_ACCEPTANCE_FALSE"
    except Exception as error:
        result["state"] = "FAIL_OR_NOT_RUN_PRESERVE_ORIGINAL"
        result["directFlowEvidence"] = False
        result["directPeerReadEvidence"] = False
        result["originalError"] = repr(error)
        raise
    finally:
        result["filesAfter"] = files()
        result["measurementPreservedDatabaseBytes"] = result["filesBefore"] == result["filesAfter"]
        if not result["measurementPreservedDatabaseBytes"]:
            result["directFlowEvidence"] = result["directRefusalEvidence"] = False
            result["directPeerReadEvidence"] = False
        with output.open("x", encoding="utf-8", newline="\n") as stream:
            json.dump(serializable(result), stream, ensure_ascii=False, indent=2)
            stream.write("\n")
    check(result["measurementPreservedDatabaseBytes"], "Immutable readback changed database bytes")


if __name__ == "__main__":
    main()
