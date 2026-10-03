"""Immutable readback of one real, normally closed M2 candidate case.

signed Python m2-readback.py STATE_ROOT PRIVATE_OUTPUT E2E_JOURNAL capture|side-worktrees|final
No credential path is opened. Raw protocol remains in the private output.
"""
import hashlib
import json
import os
import sqlite3
import subprocess
import sys
from pathlib import Path

if len(sys.argv) != 5 or sys.argv[4] not in ("capture", "side-worktrees", "final"):
    raise RuntimeError("Expected state root, fresh private output, original journal and phase")
def local_spelling(value):
    text = str(value)
    if text.startswith("\\\\?\\UNC\\"):
        raise RuntimeError("Network or UNC candidate path is outside this Win11 case")
    return text[4:] if text.startswith("\\\\?\\") else text

def same_local_path(left, right):
    return os.path.normcase(os.path.normpath(local_spelling(left))) == \
           os.path.normcase(os.path.normpath(local_spelling(right)))

def beneath(parent, child):
    parent_text, child_text = local_spelling(parent), local_spelling(child)
    return same_local_path(os.path.commonpath((parent_text, child_text)), parent_text)

root = Path(local_spelling(sys.argv[1])).resolve(strict=True)
output = Path(sys.argv[2])
phase = sys.argv[4]
if output.exists():
    raise RuntimeError("Evidence output already exists")
journal = json.loads(Path(sys.argv[3]).read_text(encoding="utf-8-sig"))
if journal.get("schema") != "gogoke.37.m2-win11-e2e.v1":
    raise RuntimeError("Not the original M2 journal")
database = root / "state.sqlite"
if not database.is_file():
    raise RuntimeError("Actual candidate database absent")
wal = Path(str(database) + "-wal")
if wal.exists() and wal.stat().st_size:
    raise RuntimeError("Candidate must be normally closed and checkpointed; nonempty WAL")

def fingerprint(value):
    return hashlib.sha256(value).hexdigest()

def files():
    return {p.name: {"length": p.stat().st_size, "sha256": fingerprint(p.read_bytes())}
            for p in (database, wal, Path(str(database) + "-shm")) if p.exists()}

def rows(connection, sql, args=()):
    return connection.execute(sql, args).fetchall()

def one(connection, sql, args=()):
    values = rows(connection, sql, args)
    if len(values) != 1:
        raise RuntimeError(f"Expected one original row: {sql.split(' FROM ')[0]}; found {len(values)}")
    return values[0]

def git(program, cwd, *args):
    observed = subprocess.run([str(program), "--no-optional-locks", "-C", str(cwd), *args],
                              capture_output=True, text=True, timeout=30, check=False,
                              stdin=subprocess.DEVNULL,
                              creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    if observed.returncode:
        raise RuntimeError(f"Read-only Git {args[0]} exit={observed.returncode}: {observed.stderr[-2048:]}")
    return observed.stdout.strip()

def git_bytes(program, cwd, *args):
    observed = subprocess.run([str(program), "--no-optional-locks", "-C", str(cwd), *args],
                              capture_output=True, timeout=30, check=False,
                              stdin=subprocess.DEVNULL,
                              creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    if observed.returncode:
        raise RuntimeError(f"Read-only Git {args[0]} exit={observed.returncode}: {observed.stderr[-2048:]!r}")
    return observed.stdout

result = {"schema": "gogoke.37.private-m2-readback.v1", "phase": phase,
          "caseId": journal["caseId"], "sourceCommit": journal["sourceCommit"],
          "databaseWrites": False, "credentialReads": False,
          "rootIdentity": [root.stat().st_dev, root.stat().st_ino],
          "filesBefore": files(), "frames": [], "commands": [], "unknownFrames": [],
          "sessions": [], "worktree": None, "directCaseEvidence": False}
domain = journal["domainId"]
lead = journal["sessions"][0]
if lead["seatId"] != journal["leadSeatId"]:
    raise RuntimeError("Original lead session binding changed")

with sqlite3.connect(database.as_uri() + "?mode=ro&immutable=1", uri=True) as db:
    db.execute("PRAGMA query_only=ON")
    result["epoch"] = one(db, "SELECT epoch FROM v37_ledger_meta WHERE singleton=1")[0]
    result["cursor"] = str(one(db, "SELECT COALESCE(MAX(cursor),0) FROM v37_ledger_index")[0])
    result["userRequests"] = []
    for operation in journal["operations"]:
        request = operation["request"]
        raw = operation.get("rawFrame")
        if not isinstance(raw, str) or json.loads(raw) != request:
            raise RuntimeError("Original User request ID or exact wire frame absent")
        if request.get("schema") != "gogoke.37.operations.v1":
            result["userRequests"].append({"kind": operation.get("kind", "COMPOSITION"),
                                           "rawSha256": fingerprint(raw.encode()),
                                           "nestedRequestIds": [json.loads(value)["requestId"]
                                               for key, value in request.items() if key.endswith("Request")
                                               and isinstance(value, str)]})
            continue
        if request.get("requestId") is None:
            raise RuntimeError("Original User request ID absent")
        family, action, request_id = request["family"], request["operation"], request["requestId"]
        if family == "K-SESSION" and action in (
                "admission-reserve", "admission-commit", "open", "stop", "admission-release", "resume"):
            stored = one(db, "SELECT raw_hex FROM gogoke_v37_h_operation WHERE domain_id=? AND request_id=?",
                         (request["domainId"], request_id))[0]
            if stored.lower() != raw.encode().hex():
                raise RuntimeError("Original User H admission/control bytes differ")
        elif family == "K-SESSION" and action in ("send", "append-without-turn"):
            stored = one(db, "SELECT request_hex FROM gogoke_v37_h_stdin_journal WHERE domain_id=? AND request_id=?",
                         (request["domainId"], request_id))[0]
            if stored.lower() != raw.encode().hex():
                raise RuntimeError("Original User H stdin bytes differ")
        elif family == "K-QCARD" and action == "answer":
            stored = one(db,
                "SELECT request_hex FROM gogoke_v37_qcard_native_operations WHERE domain_id=? AND request_id=?",
                (request["domainId"], request_id))[0]
            if stored.lower() != raw.encode().hex():
                raise RuntimeError("Original User native answer bytes differ")
        elif family == "K-WORKTREE" and action == "create":
            stored = one(db,
                "SELECT request_hash FROM gogoke_v37_worktree_operations WHERE request_id=?",
                (request_id,))[0]
            if stored != fingerprint(raw.encode()):
                raise RuntimeError("Original User F create bytes differ")
        elif family == "K-WORKTREE" and action == "register":
            stored = one(db,
                "SELECT request_hash FROM gogoke_v37_worktree_lifecycle_ops "
                "WHERE request_id=? AND operation='REGISTER'", (request_id,))[0]
            if stored != fingerprint(raw.encode()):
                raise RuntimeError("Original User F register bytes differ")
        elif family == "K-SIDE" and action in ("create", "resume", "archive", "restore", "delete"):
            stored = one(db,
                "SELECT request_hex FROM gogoke_v37_side_operations WHERE domain_id=? AND request_id=?",
                (request["domainId"], request_id))[0]
            if stored.lower() != raw.encode().hex():
                raise RuntimeError("Original User D lifecycle bytes differ")
        result["userRequests"].append({"requestId": request_id, "family": family,
                                       "operation": action, "rawSha256": fingerprint(raw.encode())})
    originals = {}
    for session in journal["sessions"]:
        session_id = session["id"]
        episodes = rows(db,
            "SELECT generation,process_operation_id,phase,stop_fact_id,seat_id,seat_incarnation,instance_id "
            "FROM gogoke_v37_h_process_episode WHERE domain_id=? AND session_id=? ORDER BY rowid",
            (domain, session_id))
        originals[session_id] = []
        incoming = rows(db,
            "SELECT generation,operation_id,source_epoch,source_cursor,raw_bytes,state,process_ticket,custodian_nonce,no_event_reason "
            "FROM v37_ledger_raw_source WHERE domain_id=? AND session_id=? "
            "ORDER BY rowid", (domain, session_id))
        for generation, operation, epoch, cursor, raw, state, ticket, nonce, reason in incoming:
            data = bytes(raw)
            frame = json.loads(data.decode("utf-8"))
            record = {"direction": "in", "sessionId": session_id, "generation": generation,
                      "operationId": operation, "sourceEpoch": epoch,
                      "sourceCursor": cursor, "processTicket": ticket,
                      "custodianNonce": nonce, "state": state, "noEventReason": reason,
                      "originalFrame": data.decode("utf-8")}
            result["frames"].append(record)
            originals[session_id].append((record, frame, data))
            if state == "PENDING":
                result["unknownFrames"].append({"sessionId": session_id,
                    "sourceEpoch": epoch, "sourceCursor": cursor,
                    "method": frame.get("method"), "state": state})
        for generation, operation, step, command, step_phase, epoch, cursor, ticket, nonce in rows(db,
            "SELECT generation,process_operation_id,step_id,command_hex,phase,source_epoch,source_cursor,ticket,custodian_nonce "
            "FROM gogoke_v37_rpc_steps WHERE domain_id=? AND session_id=? ORDER BY rowid",
            (domain, session_id)):
            result["commands"].append({"direction": "out", "sessionId": session_id, "generation": generation,
                "operationId": operation, "stepId": step, "phase": step_phase,
                "sourceEpoch": epoch, "sourceCursor": cursor,
                "processTicket": ticket, "custodianNonce": nonce,
                "originalFrame": bytes.fromhex(command).decode("utf-8"),
                "confirmedWrite": step_phase in ("WRITTEN", "OBSERVED")})
        normalized = rows(db,
            "SELECT i.cursor,i.source_epoch,i.source_cursor,i.update_json,r.operation_id,r.generation,"
            "r.process_ticket,r.custodian_nonce "
            "FROM v37_ledger_index i LEFT JOIN v37_ledger_raw_source r "
            "ON r.resolved_event_id=i.source_event_id AND r.domain_id=i.domain_id "
            "AND r.session_id=i.session_id "
            "WHERE i.source_kind='v37' AND i.domain_id=? AND i.session_id=? ORDER BY i.cursor",
            (domain, session_id))
        actual_pins = rows(db,
            "SELECT DISTINCT i.driver_id,i.version,i.program_digest,e.instance_id,c.binary_digest_sha256 "
            "FROM gogoke_v37_h_process_episode e "
            "JOIN gogoke_v37_instances i ON i.instance_id=e.instance_id "
            "LEFT JOIN gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id "
            "WHERE e.domain_id=? AND e.session_id=? AND e.process_operation_id IS NOT NULL",
            (domain, session_id))
        if len(actual_pins) != 1 or actual_pins[0][3] != session["instanceId"] or \
                actual_pins[0][2] != actual_pins[0][4]:
            raise RuntimeError("Actual F/H program pin absent, changed across generations or mismatched custody")
        driver_id, version, digest, instance_id, _ = actual_pins[0]
        result["sessions"].append({"sessionId": session_id, "seatId": session["seatId"],
            "instanceId": session["instanceId"], "episodes": episodes,
            "driverId": driver_id, "version": version, "binarySha256": digest,
            "normalized": [{"cursor": str(cursor), "sourceEpoch": epoch,
                "ledgerSourceCursor": source_cursor, "operationId": operation,
                "generation": generation, "processTicket": ticket,
                "custodianNonce": nonce, "update": json.loads(update)}
                for cursor, epoch, source_cursor, update, operation, generation, ticket, nonce in normalized],
            "missingNormalized": len(normalized) == 0,
            "allEpisodesStopped": bool(episodes) and all(row[2] == "STOPPED" and row[3] for row in episodes),
            "rawFrameCount": len(incoming), "unknownFrameCount": sum(row[5] == "PENDING" for row in incoming)})

    matching = []
    for record, frame, data in originals[lead["id"]]:
        if frame.get("method") != "item/tool/call":
            continue
        params = frame.get("params", {})
        args = params.get("arguments", {})
        if params.get("tool") == "gogoke_seat" and args.get("operation") == "dispatch" \
                and args.get("targetId") == journal["childSeatId"]:
            matching.append((record, frame, data))
    if len(matching) != 1:
        raise RuntimeError(f"Expected one original model child dispatch, found {len(matching)}")
    call_record, call, original_bytes = matching[0]
    typed = call["id"]
    if type(typed) is int:
        typed_id = f"n:{typed}"
    elif isinstance(typed, str):
        typed_id = f"s:{typed}"
    else:
        raise RuntimeError("Original model RPC ID has unsupported type")
    host_id = "model-" + fingerprint((call_record["operationId"] + "\n" +
        call_record["processTicket"] + "\n" + call_record["custodianNonce"] +
        "\n" + typed_id).encode())[:40]
    worktree_op = one(db,
        "SELECT request_id,request_hash,repository_id,domain_id,seat_id,worktree_id,phase,seat_incarnation,seat_generation,instance_id "
        "FROM gogoke_v37_worktree_operations WHERE request_id=?", (host_id + "-worktree",))
    if worktree_op[1] != fingerprint(original_bytes) or worktree_op[2:5] != (
        journal["repositoryId"], domain, journal["childSeatId"]) or worktree_op[6] != "REGISTERED" \
            or worktree_op[9] != journal["childInstanceId"]:
        raise RuntimeError("F worktree does not bind original A call and child")
    worktree_id = worktree_op[5]
    reserve = one(db,
        "SELECT session_id,status,raw_hex FROM gogoke_v37_h_operation WHERE domain_id=? AND request_id=? AND operation='admission-reserve'",
        (domain, host_id))
    if reserve[1] != "APPLIED" or reserve[0] != journal.get("childSessionId"):
        raise RuntimeError("Original H child reservation absent or changed")
    child_session = reserve[0]
    child_episodes = rows(db,
        "SELECT generation,phase,stop_fact_id,seat_id,seat_incarnation,instance_id "
        "FROM gogoke_v37_h_process_episode WHERE domain_id=? AND session_id=? ORDER BY rowid",
        (domain, child_session))
    claim = one(db,
        "SELECT a.state,a.generation,s.seat_id FROM gogoke_v37_h_claim a "
        "JOIN gogoke_v37_h_seat_binding s ON s.domain_id=a.domain_id AND s.session_id=a.session_id "
        "AND s.generation=a.generation WHERE a.domain_id=? AND a.session_id=?", (domain, child_session))
    child_seat = one(db,
        "SELECT layer,parent_seat_id,instance_id,state,incarnation FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",
        (domain, journal["childSeatId"]))
    if not child_episodes or not all(row[1] == "STOPPED" and row[2] for row in child_episodes) \
            or claim[0] != "RELEASED" or claim[2] != journal["childSeatId"] \
            or child_seat[:4] != ("LEAD", journal["leadSeatId"], journal["childInstanceId"], "IDLE") \
            or any(row[3:6] != (journal["childSeatId"], child_seat[4], journal["childInstanceId"])
                   for row in child_episodes):
        raise RuntimeError("Original child E/H stop and release facts missing")
    stdin = one(db,
        "SELECT phase,receipt_status,request_hex,session_id FROM gogoke_v37_h_stdin_journal "
        "WHERE domain_id=? AND request_id=? AND operation='send'", (domain, host_id + "-send"))
    if stdin[:2] != ("RECEIPTED", "APPLIED") or stdin[3] != child_session:
        raise RuntimeError("Original H child send was not receipted")
    reply = one(db,
        "SELECT phase,command_hex FROM gogoke_v37_rpc_steps "
        "WHERE domain_id=? AND session_id=? AND step_id=?", (domain, lead["id"], host_id))
    if reply[0] not in ("WRITTEN", "OBSERVED") or not bytes.fromhex(reply[1]).endswith(b"\n"):
        raise RuntimeError("Original model tool reply was not written through H")
    written_reply = json.loads(bytes.fromhex(reply[1]).decode("utf-8"))
    content = written_reply.get("result", {}).get("contentItems", [])
    if len(content) != 1 or content[0].get("type") != "inputText":
        raise RuntimeError("Original H model response lacks its single native text item")
    dispatch_ack = json.loads(content[0]["text"])
    selected = journal.get("childLocator", {})
    if dispatch_ack.get("family") != "K-SESSION" or dispatch_ack.get("operation") != "send" \
            or dispatch_ack.get("status") not in ("APPLIED", "REPLAYED") or \
            dispatch_ack.get("targetId") != child_session or \
            dispatch_ack.get("revision") != selected.get("revision") or \
            dispatch_ack.get("result", {}).get("worktreeId") != worktree_id or \
            dispatch_ack.get("result", {}).get("seatId") != journal["childSeatId"] or \
            dispatch_ack.get("result", {}).get("generation") != claim[1] or \
            selected.get("source") != "ORIGINAL_NATIVE_TOOL_ACK" or \
            selected.get("sessionId") != child_session or \
            selected.get("worktreeId") != worktree_id:
        raise RuntimeError("Original H tool ACK selectors differ from F/A and live observer")
    created = [(record, frame, data) for record, frame, data in originals[lead["id"]]
               if frame.get("method") == "item/tool/call" and
               frame.get("params", {}).get("tool") == "gogoke_seat" and
               frame.get("params", {}).get("arguments", {}).get("operation") == "create-from-template" and
               frame.get("params", {}).get("arguments", {}).get("targetId") == journal["childSeatId"]]
    if len(created) != 1:
        raise RuntimeError("Expected exactly one original model child create call")
    create_id_value = created[0][1]["id"]
    create_typed = (f"n:{create_id_value}" if type(create_id_value) is int
                    else f"s:{create_id_value}" if isinstance(create_id_value, str) else None)
    if create_typed is None:
        raise RuntimeError("Original child create RPC ID invalid")
    create_record = created[0][0]
    create_id = "model-" + fingerprint((create_record["operationId"] + "\n" +
        create_record["processTicket"] + "\n" + create_record["custodianNonce"] +
        "\n" + create_typed).encode())[:40]
    seat_creation = one(db,
        "SELECT layer,parent_seat_id,instance_id FROM gogoke_v37_seat_operations "
        "WHERE domain_id=? AND request_id=? AND seat_id=?", (domain, create_id, journal["childSeatId"]))
    if seat_creation != ("LEAD", journal["leadSeatId"], journal["childInstanceId"]):
        raise RuntimeError("E child create fact does not bind original model call")
    takeover = one(db,
        "SELECT basis,source_ref,instance_id FROM gogoke_v37_seat_takeover_answers "
        "WHERE domain_id=? AND seat_id=? AND question_id=?",
        (domain, journal["leadSeatId"], journal["takeoverQuestionId"]))
    if takeover[0] != "CITED" or takeover[2] != lead["instanceId"] or len(journal["nativeCards"]) != 1 \
            or not takeover[1].startswith("C-QCARD:" + journal["nativeCards"][0]["cardId"] +
                                              ":" + journal["nativeCards"][0]["answerRequestId"] + ":"):
        raise RuntimeError("Original native takeover answer is not C-cited")
    tree = one(db,
        "SELECT w.worktree_path,w.state,w.repository_id,w.domain_id,w.seat_id,w.seat_incarnation,w.instance_id,"
        "l.state,l.revision,l.merge_target_commit,l.merge_reason,s.source_path,p.git_path,s.git_digest "
        "FROM gogoke_v37_worktrees w JOIN gogoke_v37_worktree_lifecycle l USING(worktree_id) "
        "JOIN gogoke_v37_worktree_sources s USING(repository_id) "
        "JOIN gogoke_v37_worktree_programs p USING(repository_id) WHERE w.worktree_id=?",
        (worktree_id,))
    if tree[1:7] != ("REGISTERED", journal["repositoryId"], domain,
        journal["childSeatId"], child_seat[4], journal["childInstanceId"]) \
            or not same_local_path(Path(local_spelling(tree[11])).resolve(strict=True),
                                   Path(local_spelling(journal["testbedSource"])).resolve(strict=True)):
        raise RuntimeError("Original F registration or testbed source changed")
    tree_path = Path(local_spelling(tree[0])).resolve(strict=True)
    if not beneath(root, tree_path):
        raise RuntimeError("Registered worktree is outside candidate state root")
    marker_file = journal["markerFile"]
    if Path(marker_file).name != marker_file or not marker_file.endswith(".json"):
        raise RuntimeError("Invalid original marker filename")
    marker_path = tree_path / marker_file
    if marker_path.is_symlink() or not marker_path.is_file() or marker_path.stat().st_nlink != 1:
        raise RuntimeError("Original tool marker is not an ordinary worktree file")
    marker_bytes = marker_path.read_bytes()
    if json.loads(marker_bytes)["marker"] != journal["marker"]:
        raise RuntimeError("Actual worktree marker content differs")
    marker_hash = fingerprint(marker_bytes)
    git_program = Path(local_spelling(tree[12])).resolve(strict=True)
    if "sha256:" + fingerprint(git_program.read_bytes()) != tree[13]:
        raise RuntimeError("Readback Git executable bytes differ from F registered pin")
    tree_head = git(git_program, tree_path, "rev-parse", "--verify", "HEAD^{commit}")
    if len(tree_head) != 40 or git(git_program, tree_path, "status", "--porcelain=v1", "--untracked-files=all"):
        raise RuntimeError("Child did not leave a clean committed worktree")
    committed_bytes = git_bytes(git_program, tree_path, "show", f"HEAD:{marker_file}")
    if fingerprint(committed_bytes) != marker_hash:
        raise RuntimeError("Actual Git commit does not contain marker bytes")
    result["worktree"] = {"id": worktree_id, "childSessionId": child_session,
        "requestId": worktree_op[0], "requestHash": worktree_op[1],
        "path": str(tree_path), "revision": tree[8], "state": tree[7],
        "markerFile": marker_file, "markerSha256": marker_hash, "childCommit": tree_head,
        "mergeTargetCommit": tree[9]}
    result["providerSessions"] = []
    for case in journal["providerCases"]:
        if case["result"] == "NOT_RUN_NOT_LOGGED_IN" or case["driverId"] == "antigravity":
            result["providerSessions"].append({"driverId": case["driverId"],
                                               "result": case["result"]})
            continue
        if case["result"] != "ORIGINAL_INPUT_RECEIPTED_RAW_READBACK_REQUIRED":
            raise RuntimeError("Provider case is not a completed original protocol capture")
        original = one(db,
            "SELECT phase,receipt_status,request_hex,session_id FROM gogoke_v37_h_stdin_journal "
            "WHERE domain_id=? AND request_id=? AND operation='send'",
            (domain, case["sendRequestId"]))
        provider_session = next((item for item in result["sessions"]
                                 if item["sessionId"] == case["sessionId"]), None)
        if original[:2] != ("RECEIPTED", "APPLIED") or original[3] != case["sessionId"] \
                or provider_session is None or not provider_session["allEpisodesStopped"] \
                or provider_session["rawFrameCount"] == 0 or \
                provider_session["driverId"] != case["driverId"] or \
                provider_session["instanceId"] != case["instanceId"] or \
                provider_session["version"] != case["fixedVersion"] or \
                provider_session["binarySha256"] != "sha256:" + case["fixedSha256"] or \
                case["capability"] != {"version": case["fixedVersion"],
                                       "binaryDigest": provider_session["binarySha256"]}:
            raise RuntimeError("Original provider H/A terminal or stop evidence missing")
        result["providerSessions"].append({"driverId": case["driverId"],
            "sessionId": case["sessionId"], "sendRequestId": case["sendRequestId"],
            "originalFrameCount": provider_session["rawFrameCount"],
            "normalizedEventCount": len(provider_session["normalized"]),
            "result": "UNKNOWN_MISSING_NORMALIZED_NOT_GOLDEN" if provider_session["missingNormalized"]
                else "DIRECT_ORIGINAL_PROTOCOL_EXPORTED_NOT_OWNER_ACCEPTANCE"})
    result["directCaseEvidence"] = True
    if phase == "capture":
        if tree[7] != "REGISTERED" or tree[8] < 2 or tree[9] is not None:
            raise RuntimeError("Capture stage did not retain registered unmerged worktree")
    else:
        merges = rows(db,
            "SELECT request_id,request_hash,phase,result_commit FROM gogoke_v37_worktree_lifecycle_ops "
            "WHERE worktree_id=? AND operation='MERGE'", (worktree_id,))
        merge_calls = [(record, frame, data) for record, frame, data in originals[lead["id"]]
                       if frame.get("method") == "item/tool/call" and
                       frame.get("params", {}).get("tool") == "gogoke_worktree" and
                       frame.get("params", {}).get("arguments", {}).get("operation") == "merge" and
                       frame.get("params", {}).get("arguments", {}).get("targetId") == worktree_id]
        if tree[7] != "MERGED" or not tree[9] or len(merges) != 1 or len(merge_calls) != 1 \
                or merges[0][2] != "APPLIED" or merges[0][3] != tree[9] \
                or merges[0][1] != fingerprint(merge_calls[0][2]):
            raise RuntimeError("Original model merge A/F facts do not match")
        merge_record, merge_frame, _ = merge_calls[0]
        merge_rpc = merge_frame["id"]
        merge_typed = (f"n:{merge_rpc}" if type(merge_rpc) is int
                       else f"s:{merge_rpc}" if isinstance(merge_rpc, str) else None)
        if merge_typed is None:
            raise RuntimeError("Original merge RPC ID invalid")
        merge_id = "model-" + fingerprint((merge_record["operationId"] + "\n" +
            merge_record["processTicket"] + "\n" + merge_record["custodianNonce"] +
            "\n" + merge_typed).encode())[:40]
        if merges[0][0] != merge_id:
            raise RuntimeError("F merge request ID differs from original A/H model call")
        source = Path(local_spelling(tree[11])).resolve(strict=True)
        source_marker = source / marker_file
        if source_marker.is_symlink() or not source_marker.is_file() \
                or fingerprint(source_marker.read_bytes()) != marker_hash:
            raise RuntimeError("Merged source marker bytes differ")
        source_head = git(git_program, source, "rev-parse", "--verify", "HEAD^{commit}")
        parents = git(git_program, source, "show", "-s", "--format=%P", "HEAD").split()
        message = git(git_program, source, "show", "-s", "--format=%B", "HEAD")
        if source_head != tree[9] or len(parents) != 2 or parents[1] != tree_head \
                or any(label not in message for label in (
                    f"Gogoke-Project: {domain}", f"Gogoke-Seat: {journal['childSeatId']}",
                    f"Gogoke-Instance: {journal['childInstanceId']}",
                    f"Gogoke-Turn: {merge_calls[0][1]['params']['turnId']}")):
            raise RuntimeError("Actual Git merge ancestry or provenance differs")
        result["worktree"]["sourceHead"] = source_head
        result["worktree"]["mergeRequestId"] = merges[0][0]
        result["worktree"]["mergeParents"] = parents

    result["worktrees"] = []
    plan = journal.get("sideChatPlan")
    if plan:
        for prefix in ("source", "side"):
            logical_id = plan[prefix + "WorktreeId"]
            expected = (domain, journal["repositoryId"], plan[prefix + "SeatId"],
                        plan[prefix + "InstanceId"])
            side_tree = one(db,
                "SELECT w.domain_id,w.repository_id,w.seat_id,w.instance_id,w.worktree_path,"
                "w.worktree_identity,w.state,l.state,o.phase "
                "FROM gogoke_v37_worktrees w JOIN gogoke_v37_worktree_lifecycle l USING(worktree_id) "
                "JOIN gogoke_v37_worktree_operations o USING(worktree_id) WHERE w.worktree_id=?",
                (logical_id,))
            if side_tree[:4] != expected or side_tree[6:] != ("REGISTERED", "REGISTERED", "REGISTERED"):
                raise RuntimeError("V12 worktree was not registered for this original case")
            original_path = Path(local_spelling(side_tree[4]))
            if original_path.is_symlink():
                raise RuntimeError("V12 registered worktree path is a link")
            observed_path = original_path.resolve(strict=True)
            if not beneath(root, observed_path):
                raise RuntimeError("V12 worktree path escaped candidate state root")
            stat = observed_path.stat()
            result["worktrees"].append({"worktreeId": logical_id,
                "domainId": side_tree[0], "repositoryId": side_tree[1],
                "seatId": side_tree[2], "instanceId": side_tree[3],
                "path": str(observed_path), "nativeOpaqueIdentity": side_tree[5],
                "rootIdentity": {"observer": "python-stat", "device": str(stat.st_dev),
                                 "inode": str(stat.st_ino)}})
        if len({row["path"] for row in result["worktrees"]}) != 2:
            raise RuntimeError("V12 source and side physical worktrees overlap")
    if phase == "side-worktrees" and len(result["worktrees"]) != 2:
        raise RuntimeError("V12 worktree observation requires both original F rows")
    result["sideChatCases"] = []
    if phase == "final":
        for case in journal.get("sideChatCases", []):
            reference = case["worktreeReadback"]
            if Path(reference["file"]).name != reference["file"]:
                raise RuntimeError("V12 readback reference is not a private basename")
            original_file = Path(sys.argv[3]).parent / reference["file"]
            original_bytes = original_file.read_bytes()
            if fingerprint(original_bytes) != reference["sha256"]:
                raise RuntimeError("Original closed-product V12 F artifact hash changed")
            original_artifact = json.loads(original_bytes)
            if original_artifact.get("phase") != "side-worktrees" or \
                    original_artifact.get("caseId") != journal["caseId"] or \
                    original_artifact.get("worktrees") != case["worktrees"] or \
                    original_artifact.get("worktrees") != result["worktrees"] or \
                    not original_artifact.get("measurementPreservedDatabaseBytes"):
                raise RuntimeError("V12 F artifact and current Python physical identity differ")
            sessions = [case["sourceSession"]["id"], case["sideSession"]["id"]]
            selected = [item for item in result["sessions"] if item["sessionId"] in sessions]
            if len(selected) != 2 or not all(item["allEpisodesStopped"] for item in selected):
                raise RuntimeError("Original V12 H episodes are not stopped")
            requirements = case["readbackRequirements"]
            source_id, side_id = sessions
            source_frames = [frame for _, frame, _ in originals[source_id]]
            side_records = originals[side_id]
            side_frames = [frame for _, frame, _ in side_records]
            source_text = "".join(frame.get("params", {}).get("delta", "")
                                  for frame in source_frames
                                  if frame.get("method") == "item/agentMessage/delta")
            if not all(value in source_text for value in requirements["sourceRawContains"]):
                raise RuntimeError("Original V12 source A output lacks untrusted reference")
            source_turn = requirements["sourceTurn"]
            side_turn = requirements["sideTurn"]
            def completed_turn(frames, turn):
                return any(frame.get("method") == "turn/completed" and
                           frame.get("params", {}).get("threadId") == turn["threadId"] and
                           frame.get("params", {}).get("turn", {}).get("id") == turn["turnId"] and
                           frame.get("params", {}).get("turn", {}).get("status") == "completed"
                           for frame in frames)
            if not completed_turn(source_frames, source_turn) or not completed_turn(side_frames, side_turn):
                raise RuntimeError("Original V12 source or side turn was not completed in A")
            def tool(frame):
                if frame.get("method") == "item/tool/call":
                    return True
                item = frame.get("params", {}).get("item", {})
                return frame.get("method") in ("item/started", "item/completed") and \
                    item.get("type") in ("commandExecution", "fileChange", "mcpToolCall", "dynamicToolCall")
            if any(tool(frame) for frame in source_frames):
                raise RuntimeError("V12 source reference turn executed a tool")
            passive_highwater = int(case["passiveBoundary"]["sideOutput"]["rawHighwater"])
            if any(tool(frame) or frame.get("method") == "turn/started"
                   for record, frame, _ in side_records
                   if int(record["sourceCursor"]) <= passive_highwater):
                raise RuntimeError("V12 side performed a turn/tool during passive synchronization")
            if sum(frame.get("method") == "turn/started" for frame in side_frames) != 1:
                raise RuntimeError("V12 side has an unrequested extra model turn")
            successful = [frame for frame in side_frames
                          if frame.get("method") == "item/completed" and
                          frame.get("params", {}).get("turnId") == side_turn["turnId"] and
                          frame.get("params", {}).get("threadId") == side_turn["threadId"] and
                          frame.get("params", {}).get("item", {}).get("status") == "completed" and
                          tool(frame)]
            target = case["target"]
            if target["file"] != requirements["successfulToolTarget"] or \
                    target["authorizedMarker"] != requirements["successfulToolMarker"] or \
                    not any(target["file"] in json.dumps(frame, ensure_ascii=False) and
                            target["authorizedMarker"] in json.dumps(frame, ensure_ascii=False)
                            for frame in successful):
                raise RuntimeError("V12 original successful model tool lacks exact authorized file and marker")
            side_root = next(row for row in result["worktrees"]
                             if row["worktreeId"] == case["sideSession"]["worktreeId"])
            target_file = Path(side_root["path"]) / target["file"]
            if target_file.is_symlink() or not target_file.is_file() or \
                    fingerprint(target_file.read_bytes()) != target["finalSha256"] or \
                    json.loads(target_file.read_bytes())["marker"] != target["authorizedMarker"] or \
                    target["initialSha256"] != target["passiveSha256"] or \
                    target["initialSha256"] == target["finalSha256"]:
                raise RuntimeError("V12 actual file hash does not match authorized model tool result")
            source_send = one(db,
                "SELECT phase,receipt_status FROM gogoke_v37_h_stdin_journal WHERE domain_id=? "
                "AND request_id=? AND session_id=? AND operation='send'",
                (domain, case["sourceQuestionRequestId"], source_id))
            side_request = case["questionRequest"]
            side_send = one(db,
                "SELECT phase,receipt_status,request_hex FROM gogoke_v37_h_stdin_journal "
                "WHERE domain_id=? AND request_id=? AND session_id=? AND operation='send'",
                (domain, side_request["requestId"], side_id))
            if source_send != ("RECEIPTED", "APPLIED") or side_send[:2] != ("RECEIPTED", "APPLIED") \
                    or side_send[2].lower() != json.dumps(side_request, ensure_ascii=False,
                        separators=(",", ":")).encode().hex() or \
                    rows(db, "SELECT request_id FROM gogoke_v37_h_stdin_journal "
                        "WHERE domain_id=? AND session_id=? AND operation='append-without-turn'",
                        (domain, side_id)):
                raise RuntimeError("V12 original H source/side input receipts or no-append boundary differ")
            side_open = case["openRequest"]
            original_open = one(db,
                "SELECT raw_hex,status FROM gogoke_v37_h_operation WHERE domain_id=? "
                "AND request_id=? AND operation='open' AND session_id=?",
                (domain, side_open["requestId"], side_id))
            if original_open[1] != "APPLIED" or original_open[0].lower() != \
                    json.dumps(side_open, ensure_ascii=False, separators=(",", ":")).encode().hex():
                raise RuntimeError("V12 nested side open is not the original H request")
            side_create = case["createRequest"]
            original_create = one(db,
                "SELECT request_hex FROM gogoke_v37_side_operations WHERE domain_id=? AND request_id=?",
                (domain, side_create["requestId"]))[0]
            if original_create.lower() != json.dumps(side_create, ensure_ascii=False,
                    separators=(",", ":")).encode().hex():
                raise RuntimeError("V12 nested D create is not the original User request")
            side_state = one(db,
                "SELECT state,source_session_id,session_id,source_epoch FROM gogoke_v37_side_registry "
                "WHERE domain_id=? AND side_id=?", (domain, case["sideId"]))
            if side_state != ("DELETED", source_id, side_id, case["passiveBoundary"]["sideOutput"].get("sourceEpoch", result["epoch"])):
                raise RuntimeError("V12 D final source/side registry state differs")
            side_commands = [row["originalFrame"] for row in result["commands"]
                             if row["sessionId"] == side_id and row["confirmedWrite"]]
            if not any(all(value in raw for value in requirements["questionRawContains"])
                       for raw in side_commands):
                raise RuntimeError("V12 original H side prompt omitted pending reference or explicit request")
            if case["sourceLedgerBeforeDelete"]["sha256"] != case["sourceLedgerAfterDelete"]["sha256"] \
                    or case["state"] != "FLOW_COMPLETE_DIRECT_LEDGER_READBACK_REQUIRED":
                raise RuntimeError("V12 retained source or module completion state differs")
            result["sideChatCases"].append({"caseId": case["caseId"],
                "sessionIds": sessions, "originalFrameCounts": [item["rawFrameCount"] for item in selected],
                "worktreeReadbackSha256": reference["sha256"],
                "successfulOriginalToolCount": len(successful),
                "result": "DIRECT_A_H_F_AND_AUTHORIZED_TOOL_EXPORTED_REQUIRES_V12_REVIEW"})

result["filesAfter"] = files()
result["measurementPreservedDatabaseBytes"] = result["filesBefore"] == result["filesAfter"]
output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
if not result["measurementPreservedDatabaseBytes"] or not result["directCaseEvidence"]:
    raise RuntimeError("Actual M2 direct evidence incomplete; preserve original private readback")
