"""Immutable direct readback for the standalone real Win11 M2 provider E2E.

Run only after the exact installed product exited normally. This exports the
original A/H/F bytes; it never opens credential stores or writes the database.
"""
import hashlib
import json
import os
import sqlite3
import sys
from pathlib import Path

def sha(data):
    return hashlib.sha256(data).hexdigest()

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

def capture_process_identity(pid_text, expected_sha):
    if os.name != "nt" or not pid_text.isdecimal() or len(expected_sha) != 64:
        raise RuntimeError("Exact Win32 process identity requires a decimal PID and SHA-256")
    import ctypes
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.GetProcessTimes.argtypes = [wintypes.HANDLE, ctypes.c_void_p, ctypes.c_void_p,
                                         ctypes.c_void_p, ctypes.c_void_p]
    kernel32.GetProcessTimes.restype = wintypes.BOOL
    kernel32.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD,
                                                    wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
    kernel32.QueryFullProcessImageNameW.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL

    class FileTime(ctypes.Structure):
        _fields_ = [("low", wintypes.DWORD), ("high", wintypes.DWORD)]

    pid = int(pid_text)
    handle = kernel32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
    if not handle:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        creation, exit_time, kernel_time, user_time = (FileTime(), FileTime(), FileTime(), FileTime())
        if not kernel32.GetProcessTimes(handle, ctypes.byref(creation), ctypes.byref(exit_time),
                                        ctypes.byref(kernel_time), ctypes.byref(user_time)):
            raise ctypes.WinError(ctypes.get_last_error())
        image = ctypes.create_unicode_buffer(32768)
        image_size = wintypes.DWORD(len(image))
        if not kernel32.QueryFullProcessImageNameW(handle, 0, image, ctypes.byref(image_size)):
            raise ctypes.WinError(ctypes.get_last_error())
        image_path = Path(image.value)
        image_sha = sha(image_path.read_bytes())
        if image_sha != expected_sha:
            raise RuntimeError("Exact live process image SHA-256 differs from the fixed provider pin")
        creation_100ns = (int(creation.high) << 32) | int(creation.low)
        return {"processId": str(pid), "creationTime100ns": str(creation_100ns),
                "imageSha256": image_sha}
    finally:
        kernel32.CloseHandle(handle)

def native_directory_identity(value):
    if os.name != "nt":
        raise RuntimeError("Native F identity requires Win32 FileIdInfo")
    import ctypes
    from ctypes import wintypes

    class FileAttributeTagInfo(ctypes.Structure):
        _fields_ = [("file_attributes", wintypes.DWORD), ("reparse_tag", wintypes.DWORD)]

    class FileIdInfo(ctypes.Structure):
        _fields_ = [("volume_serial", ctypes.c_ulonglong), ("file_id", ctypes.c_ubyte * 16)]

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                     wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    kernel32.CreateFileW.restype = wintypes.HANDLE
    kernel32.GetFileInformationByHandleEx.argtypes = [wintypes.HANDLE, ctypes.c_int,
                                                       wintypes.LPVOID, wintypes.DWORD]
    kernel32.GetFileInformationByHandleEx.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL
    handle = kernel32.CreateFileW(str(value), 0x80, 0x1 | 0x2 | 0x4, None, 3,
                                  0x00200000 | 0x02000000, None)  # READ_ATTRIBUTES, OPEN_REPARSE_POINT | BACKUP_SEMANTICS
    invalid = ctypes.c_void_p(-1).value
    handle_value = handle if isinstance(handle, int) else getattr(handle, "value", None)
    if handle_value is None or handle_value == invalid:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        attributes = FileAttributeTagInfo()
        if not kernel32.GetFileInformationByHandleEx(handle, 9, ctypes.byref(attributes),
                                                      ctypes.sizeof(attributes)):  # FileAttributeTagInfo
            raise ctypes.WinError(ctypes.get_last_error())
        if not attributes.file_attributes & 0x10 or attributes.file_attributes & 0x400:
            raise RuntimeError("F worktree root is not a plain directory or is a reparse point")
        identity = FileIdInfo()
        if not kernel32.GetFileInformationByHandleEx(handle, 18, ctypes.byref(identity),
                                                      ctypes.sizeof(identity)):  # FileIdInfo
            raise ctypes.WinError(ctypes.get_last_error())
        return f"volume:{identity.volume_serial:016x}/file:{bytes(identity.file_id).hex()}"
    finally:
        kernel32.CloseHandle(handle)

if len(sys.argv) == 3 and sys.argv[1] == "--directory-identity":
    print(native_directory_identity(sys.argv[2]))
    raise SystemExit(0)

if len(sys.argv) == 4 and sys.argv[1] == "--process-identity":
    observed = capture_process_identity(sys.argv[2], sys.argv[3])
    print(json.dumps(observed, separators=(",", ":")))
    raise SystemExit(0)

if len(sys.argv) not in (4, 7):
    raise RuntimeError("Expected state root, fresh private output, and original provider journal")

def fail(message):
    raise RuntimeError(message)

def exact_one(db, sql, args=()):
    values = db.execute(sql, args).fetchall()
    if len(values) != 1:
        fail(f"Expected exactly one original database row; found {len(values)}")
    return values[0]

root = Path(local_spelling(sys.argv[1])).resolve(strict=True)
output = Path(sys.argv[2])
journal_path = Path(sys.argv[3]).resolve(strict=True)
if output.exists():
    fail("Private provider readback output already exists")
journal_bytes = journal_path.read_bytes()
journal = json.loads(journal_bytes.decode("utf-8-sig"))
supplement = len(sys.argv) == 7 and sys.argv[4] == "--original-failure-supplement"
if len(sys.argv) not in (4, 7) or (len(sys.argv) == 7 and not supplement):
    fail("Unexpected provider readback arguments")
if supplement and (sha(journal_bytes) != sys.argv[5] or
        journal.get("driverBytes", {}).get("m2-provider-capture-readback.py") != sys.argv[6] or
        journal.get("state") != "FAIL_ORIGINAL_REQUEST_RETAINED" or
        journal.get("protection") != "PASS_FORMAL_MEMORY_LEDGER_FIELDS_UNCHANGED"):
    fail("Supplement must bind the unchanged original failure and executed reader")
expected_phase = "FAIL_ORIGINAL_REQUEST_RETAINED" if supplement else "DIRECT_PROVIDER_H_RECEIPTS_A_READBACK_REQUIRED"
if journal.get("schema") != "gogoke.37.m2-provider-win11-e2e.v1" or \
        journal.get("state") != expected_phase or \
        journal.get("acceptance") is not False or journal.get("observerDatabaseWrites") is not False or \
        journal.get("hostOperationsWriteCandidateDatabase") is not True or \
        journal.get("credentialReads") is not False or not 1 <= len(journal.get("cases", [])) <= 3 or \
        len(journal.get("launches", [])) != 1 or len(journal.get("closes", [])) != 1:
    fail("Original standalone provider journal is incomplete or not at its readback phase")
launch = journal["launches"][0]
close = journal["closes"][0]
if launch.get("sourceCommit") != journal["sourceCommit"] or \
        launch.get("bootstrap", {}).get("version") != journal.get("installedVersion") or \
        not all(name in journal.get("installedSha256", {}) for name in
                ("gogoke.exe", "gogoke-native-host.exe", "resource-index.json")) or \
        close.get("exitCode") != 0 or close.get("forceKill") is not False or \
        close.get("pid") != launch.get("pid"):
    fail("Actual installed candidate identity or normal-close receipt differs")
database = root / "state.sqlite"
wal = Path(str(database) + "-wal")
if not database.is_file():
    fail("Actual installed candidate database is absent")
if wal.exists() and wal.stat().st_size:
    fail("Actual product must be normally closed and checkpointed; refuse nonempty WAL")

def file_facts():
    return {item.name: {"length": item.stat().st_size, "sha256": sha(item.read_bytes())}
            for item in (database, wal, Path(str(database) + "-shm")) if item.exists()}

before = file_facts()
result = {"schema": "gogoke.37.private-m2-readback.v1", "phase": "provider-final",
          "caseId": journal["caseId"], "sourceCommit": journal["sourceCommit"],
          "domainId": journal["domainId"], "databaseWrites": False, "credentialReads": False,
          "acceptance": False, "rootIdentity": [str(root.stat().st_dev), str(root.stat().st_ino)],
          "filesBefore": before, "frames": [], "commands": [], "unknownFrames": [],
          "sessions": [], "providerWorktrees": [], "directProviderEvidence": False}
if supplement:
    result.update({"state": "SUPPLEMENTARY_DIRECT_PROVIDER_READBACK_ORIGINAL_FAILURE_RETAINED",
                   "originalFailureRetained": True, "originalJournalSha256": sys.argv[5],
                   "originalReaderSha256": sys.argv[6], "readerSha256": sha(Path(__file__).read_bytes())})
domain = journal["domainId"]

def nested_text(value, parts):
    found = value
    for part in parts:
        if not isinstance(found, dict):
            return None
        found = found.get(part)
    return found

def claude_text(frame):
    if frame.get("type") != "assistant":
        return ""
    content = nested_text(frame, ("message", "content"))
    if not isinstance(content, list):
        return ""
    return "".join(part.get("text", "") for part in content
                    if isinstance(part, dict) and part.get("type") == "text")

def acp_text(frame):
    update = nested_text(frame, ("params", "update"))
    if frame.get("method") != "session/update" or not isinstance(update, dict) or \
            update.get("sessionUpdate") != "agent_message_chunk":
        return ""
    content = update.get("content")
    if isinstance(content, dict) and content.get("type") == "text" and isinstance(content.get("text"), str):
        return content["text"]
    return ""

with sqlite3.connect(database.as_uri() + "?mode=ro&immutable=1", uri=True) as db:
    db.execute("PRAGMA query_only=ON")
    result["epoch"] = exact_one(db, "SELECT epoch FROM v37_ledger_meta WHERE singleton=1")[0]
    result["cursor"] = str(exact_one(db, "SELECT COALESCE(MAX(cursor),0) FROM v37_ledger_index")[0])
    for case in journal["cases"]:
        if case.get("result") != "DIRECT_H_RECEIPT_A_READBACK_REQUIRED":
            fail(f"{case.get('driverId')}: not all configured provider turns reached original H receipt")
        driver = case["driverId"]
        session_id = case["sessionId"]
        send_id = case["sendRequestId"]
        prompt = case["prompt"]
        marker = case["marker"]
        operation_record = next((row for row in journal["operations"]
                                 if row.get("request", {}).get("requestId") == send_id), None)
        request = operation_record.get("request") if operation_record else None
        original_wire = operation_record.get("rawFrame") if operation_record else None
        if not isinstance(request, dict) or request.get("operation") != "send" or \
                request.get("targetId") != session_id or request.get("domainId") != domain or \
                request.get("payload", {}).get("body") != prompt or original_wire != json.dumps(
                    request, ensure_ascii=False, separators=(",", ":")):
            fail(f"{driver}: exact original H User request bytes are absent or changed")
        send = exact_one(db,
            "SELECT phase,receipt_status,request_hex,session_id,generation,ticket,process_operation_id,custodian_nonce "
            "FROM gogoke_v37_h_stdin_journal WHERE domain_id=? AND request_id=? AND operation='send'",
            (domain, send_id))
        if send[:4] != ("RECEIPTED", "APPLIED", original_wire.encode().hex(), session_id) or \
                send[4] != request["payload"]["generation"]:
            fail(f"{driver}: original H send was not durably receipted APPLIED")
        opened = journal["sessions"]
        session = next((row for row in opened if row.get("id") == session_id), None)
        if session is None or session.get("seatId") != case["seatId"] or \
                session.get("instanceId") != case["instanceId"] or session.get("worktreeId") != case["worktreeId"]:
            fail(f"{driver}: original H session differs from configured E/F identity")
        operations = [row for row in journal["operations"]
                      if row.get("request", {}).get("targetId") == session_id]
        by_action = {row["request"]["operation"]: row for row in operations
                     if row.get("request", {}).get("family") == "K-SESSION"}
        if not all(action in by_action for action in ("open", "stop", "admission-release")):
            fail(f"{driver}: original H open/stop/release requests are incomplete")
        for action in ("open", "stop", "admission-release"):
            record = by_action[action]
            stored = exact_one(db,
                "SELECT raw_hex,status FROM gogoke_v37_h_operation WHERE domain_id=? AND request_id=? "
                "AND operation=? AND session_id=?",
                (domain, record["request"]["requestId"], action, session_id))
            if stored[0].lower() != record["rawFrame"].encode().hex() or stored[1] != "APPLIED":
                fail(f"{driver}: original H {action} bytes/receipt differ")
        open_payload = by_action["open"]["request"].get("payload", {})
        if open_payload.get("seatId") != case["seatId"] or \
                open_payload.get("repositoryId") != journal["repositoryId"] or \
                open_payload.get("worktreeId") != case["worktreeId"]:
            fail(f"{driver}: original H open does not bind the configured E/F objects")
        for lifecycle in ("stop", "admission-release"):
            payload = by_action[lifecycle]["request"].get("payload", {})
            if payload.get("generation") != send[4] or payload.get("seatId") != case["seatId"]:
                fail(f"{driver}: original H {lifecycle} does not bind the sent generation and E seat")
        capability_entry = by_action.get("capability-probe")
        if not capability_entry:
            fail(f"{driver}: original H capability-probe request is absent")
        capability_bytes = exact_one(db,
            "SELECT request_bytes,receipt_bytes FROM v37_ledger_receipt "
            "WHERE family='K-SESSION' AND domain_id=? AND request_id=?",
            (domain, capability_entry["request"]["requestId"]))
        if bytes(capability_bytes[0]) != capability_entry["rawFrame"].encode():
            fail(f"{driver}: original H capability request wire bytes differ")
        capability_receipt = json.loads(capability_bytes[1])
        cap_result = capability_receipt.get("result", {})
        expected_basis = "ORIGINAL_CLAUDE_INITIALIZE_ACK" if driver == "claude" else "NATIVE_ACP_INITIALIZE_DECLARATION"
        if capability_receipt.get("status") != "APPLIED" or cap_result.get("driverId") != driver or \
                cap_result.get("version") != case["fixedVersion"] or \
                cap_result.get("binaryDigest") != "sha256:" + case["fixedSha256"] or \
                cap_result.get("evidenceBasis") != expected_basis:
            fail(f"{driver}: original H capability receipt does not match the fixed executable")
        claim = exact_one(db,
            "SELECT state,stop_fact_id,instance_id,generation FROM gogoke_v37_h_claim "
            "WHERE domain_id=? AND session_id=?", (domain, session_id))
        if claim[0] != "RELEASED" or not claim[1] or claim[1] != case.get("stopFact") or \
                claim[2] != case["instanceId"]:
            fail(f"{driver}: original H claim lacks normal STOPPED then RELEASED facts")
        seat = exact_one(db,
            "SELECT s.incarnation,s.instance_id,s.state,settings.settings_json "
            "FROM gogoke_v37_seats s JOIN gogoke_v37_seat_settings settings "
            "USING(domain_id,seat_id) WHERE s.domain_id=? AND s.seat_id=?",
            (domain, case["seatId"]))
        seat_settings = json.loads(seat[3])
        expected_settings = case.get("seatSettings", {})
        if seat[1:3] != (case["instanceId"], "IDLE") or \
                seat_settings.get("model") != expected_settings.get("model") or \
                seat_settings.get("effort") != expected_settings.get("effort"):
            fail(f"{driver}: direct E row is not normally released with its original model/effort")
        instance = exact_one(db,
            "SELECT driver_id,version,program_digest,install_state,login_state "
            "FROM gogoke_v37_instances WHERE instance_id=?", (case["instanceId"],))
        observed_instance = case.get("loggedInInstance", {})
        if instance[:2] != (driver, case["fixedVersion"]) or \
                instance[2] != "sha256:" + case["fixedSha256"] or instance[4] != "LOGGED_IN" or \
                observed_instance != {"instanceId": case["instanceId"], "driverId": driver,
                    "version": case["fixedVersion"], "state": "LOGGED_IN"}:
            fail(f"{driver}: original configured instance was not the logged-in fixed executable")
        if driver == "claude" and (cap_result.get("requestedModel") != expected_settings.get("model") or
                cap_result.get("requestedEffort") != expected_settings.get("effort")):
            fail("Claude original H capability settings differ from bound E model/effort")
        binding = exact_one(db,
            "SELECT seat_id,seat_incarnation,generation,seat_authorization_generation,selected_instance_id,provenance "
            "FROM gogoke_v37_effective_seat "
            "WHERE domain_id=? AND session_id=?", (domain, session_id))
        relation = (case["seatId"], seat[0], int(send[4]), case["instanceId"])
        selection = exact_one(db,
            "SELECT seat_id,seat_incarnation,seat_authorization_generation,selected_instance_id "
            "FROM gogoke_v37_native_selection WHERE domain_id=? AND session_id=?", (domain, session_id))
        immutable_binding = exact_one(db,
            "SELECT seat_id,seat_incarnation,seat_authorization_generation,selected_instance_id,provenance "
            "FROM gogoke_v37_session_binding_v2 WHERE domain_id=? AND session_id=?", (domain, session_id))
        if binding != (relation[0], relation[1], send[4], relation[2], relation[3], "NATIVE_V2") or \
                selection != relation or immutable_binding != (*relation, "NATIVE_V2"):
            fail(f"{driver}: original H episode is not bound to the configured E generation")
        episodes = db.execute(
            "SELECT generation,process_operation_id,phase,stop_fact_id,seat_id,seat_incarnation,instance_id,request_id "
            "FROM gogoke_v37_h_process_episode WHERE domain_id=? AND session_id=? ORDER BY rowid",
            (domain, session_id)).fetchall()
        if not episodes or not all(row[2] == "STOPPED" and row[3] == claim[1] for row in episodes) or \
                any(row[4] != case["seatId"] or row[5] != seat[0] or row[6] != case["instanceId"] for row in episodes):
            fail(f"{driver}: original physical H episode stop facts are absent")
        bound_episodes = [row for row in episodes if row[0] == send[4] and row[1] == send[6]]
        if len(bound_episodes) != 1 or \
                bound_episodes[0][7] != by_action["open"]["request"]["requestId"]:
            fail(f"{driver}: original send is not bound to exactly one physical H episode")
        episode = bound_episodes[0]
        stop_receipt = by_action["stop"].get("receipt") or {}
        if stop_receipt.get("status") != "APPLIED" or \
                stop_receipt.get("result", {}).get("stopFact") != claim[1]:
            fail(f"{driver}: original User stop receipt does not match the H claim StopFact")
        custody = exact_one(db,
            "SELECT ticket,custodian_nonce,pid,creation_time_100ns,binary_digest_sha256,profile_id,"
            "domain_id,generation,state,stop_proof_hash FROM gogoke_coordination_process_custody "
            "WHERE operation_id=?", (send[6],))
        process_identity = case.get("processIdentity", {})
        if custody[0] != send[5] or custody[1] != send[7] or \
                custody[2] != process_identity.get("processId") or \
                custody[3] != process_identity.get("creationTime100ns") or \
                custody[4] != "sha256:" + case["fixedSha256"] or \
                custody[5] != case["instanceId"] or \
                custody[6] != domain or custody[7] != send[4] or \
                custody[8] != "STOPPED" or not custody[9] or custody[9] != claim[1] or \
                process_identity.get("basis") != "ACTUAL_PRODUCT_DESCENDANT_PROCESS_COMMAND_LINE_FILTERED" or \
                process_identity.get("imageSha256") != case["fixedSha256"] or \
                process_identity.get("productRootPid") != str(journal.get("currentEndpoint", {}).get("pid")):
            fail(f"{driver}: original process PID/creation/ticket/nonce/domain/generation/STOPPED proof differs")
        pins = db.execute(
            "SELECT DISTINCT i.driver_id,i.version,i.program_digest,e.instance_id,c.binary_digest_sha256 "
            "FROM gogoke_v37_h_process_episode e JOIN gogoke_v37_instances i ON i.instance_id=e.instance_id "
            "LEFT JOIN gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id "
            "WHERE e.domain_id=? AND e.session_id=? AND e.process_operation_id IS NOT NULL",
            (domain, session_id)).fetchall()
        if len(pins) != 1 or pins[0][0] != driver or pins[0][1] != case["fixedVersion"] or \
                pins[0][2] != "sha256:" + case["fixedSha256"] or pins[0][3] != case["instanceId"] or \
                pins[0][2] != pins[0][4]:
            fail(f"{driver}: original H/F executable pin or custody digest differs")
        tree = exact_one(db,
            "SELECT w.domain_id,w.repository_id,w.seat_id,w.instance_id,w.worktree_path,w.worktree_identity,"
            "w.state,l.state,o.phase FROM gogoke_v37_worktrees w "
            "JOIN gogoke_v37_worktree_lifecycle l USING(worktree_id) "
            "JOIN gogoke_v37_worktree_operations o USING(worktree_id) WHERE w.worktree_id=?",
            (case["worktreeId"],))
        if tree[:4] != (domain, journal["repositoryId"], case["seatId"], case["instanceId"]) or \
                tree[6:] != ("REGISTERED", "REGISTERED", "REGISTERED") or not tree[5]:
            fail(f"{driver}: original F registration is not bound to the exact E/H identity")
        native_tree_path = Path(str(tree[4]))
        tree_path = Path(local_spelling(tree[4]))
        if not beneath(root, tree_path):
            fail(f"{driver}: actual F worktree escaped the private candidate state root")
        stat_before = tree_path.lstat()
        if getattr(stat_before, "st_file_attributes", 0) & 0x400:
            fail(f"{driver}: registered original F worktree root is a reparse point")
        actual_identity = native_directory_identity(native_tree_path)
        if not tree[5] or actual_identity != tree[5]:
            fail(f"{driver}: actual F FileIdInfo differs from the recorded native opaque identity")
        stat_after = tree_path.lstat()
        if getattr(stat_after, "st_file_attributes", 0) & 0x400 or \
                (stat_before.st_dev, stat_before.st_ino) != (stat_after.st_dev, stat_after.st_ino):
            fail(f"{driver}: actual F stat identity changed during no-follow readback")
        resolved_tree = tree_path.resolve(strict=True)
        if not beneath(root, resolved_tree):
            fail(f"{driver}: resolved original F worktree escaped the private candidate state root")
        result["providerWorktrees"].append({"driverId": driver, "worktreeId": case["worktreeId"],
            "domainId": domain, "repositoryId": journal["repositoryId"], "seatId": case["seatId"],
            "instanceId": case["instanceId"], "path": str(resolved_tree),
            "nativeOpaqueIdentity": tree[5], "fileIdInfoIdentity": actual_identity,
            "rootIdentity": [str(stat_after.st_dev), str(stat_after.st_ino)]})
        incoming = db.execute(
            "SELECT generation,operation_id,source_epoch,source_cursor,raw_bytes,state,process_ticket,custodian_nonce,no_event_reason "
            "FROM v37_ledger_raw_source WHERE domain_id=? AND session_id=? ORDER BY rowid",
            (domain, session_id)).fetchall()
        records = []
        for generation, operation_id, epoch, cursor, raw, state, ticket, nonce, reason in incoming:
            data = bytes(raw)
            frame = json.loads(data.decode("utf-8"))
            row = {"direction": "in", "sessionId": session_id, "generation": generation,
                "operationId": operation_id, "sourceEpoch": epoch, "sourceCursor": cursor,
                "processTicket": ticket, "custodianNonce": nonce, "state": state,
                "noEventReason": reason, "originalFrame": data.decode("utf-8")}
            result["frames"].append(row); records.append((row, frame))
            if state == "PENDING":
                classification = "CLAUDE_INITIALIZATION_METADATA" if driver == "claude" and \
                    frame.get("type") == "system" and frame.get("subtype") == "init" else \
                    "PRESERVED_UNRESOLVED_RAW_SOURCE_NO_SUCCESS_CREDIT"
                row["pendingClassification"] = classification
                result["unknownFrames"].append({"sessionId": session_id, "sourceEpoch": epoch,
                    "sourceCursor": cursor, "method": frame.get("method", frame.get("type")),
                    "state": state, "classification": classification})
        cursor_groups = {}
        for row, _ in records:
            cursor_groups.setdefault((row["operationId"], row["sourceEpoch"]), []).append(int(row["sourceCursor"]))
        if any(sorted(values) != list(range(1, max(values) + 1)) for values in cursor_groups.values() if values):
            fail(f"{driver}: original A source cursors are not continuous within the recorded process epochs")
        outgoing = db.execute(
            "SELECT generation,process_operation_id,step_id,command_hex,phase,source_epoch,source_cursor,ticket,custodian_nonce "
            "FROM gogoke_v37_rpc_steps WHERE domain_id=? AND session_id=? ORDER BY rowid",
            (domain, session_id)).fetchall()
        for generation, operation_id, step_id, command, phase, epoch, cursor, ticket, nonce in outgoing:
            result["commands"].append({"direction": "out", "sessionId": session_id,
                "generation": generation, "operationId": operation_id, "stepId": step_id,
                "phase": phase, "sourceEpoch": epoch, "sourceCursor": cursor,
                "processTicket": ticket, "custodianNonce": nonce,
                "originalFrame": bytes.fromhex(command).decode("utf-8"),
                "confirmedWrite": phase in ("WRITTEN", "OBSERVED")})
        scoped = [(row, frame) for row, frame in records
                  if row["operationId"] == send[6] and row["generation"] == send[4] and
                  row["processTicket"] == send[5] and row["custodianNonce"] == send[7]]
        if not scoped:
            fail(f"{driver}: original H operation has no A source rows")
        marker_output = False
        provider_output = ""
        vendor_end = False
        prompt_echo = False
        terminal_session = None
        claude_terminals = []
        claude_echoes = []
        acp_end_candidates = []
        prompt_commands = []
        for command_row in result["commands"]:
            if command_row["sessionId"] != session_id or not command_row["confirmedWrite"]:
                continue
            try:
                command_frame = json.loads(command_row["originalFrame"])
            except json.JSONDecodeError:
                continue
            raw_command = json.dumps(command_frame, ensure_ascii=False)
            if marker in raw_command or prompt in raw_command:
                prompt_commands.append(command_frame)
        if not prompt_commands:
            fail(f"{driver}: original H write journal does not contain the one configured prompt")
        if len(prompt_commands) != 1:
            fail(f"{driver}: original provider prompt appears in more than one confirmed A command")
        for row, frame in scoped:
            if driver == "claude":
                provider_output += claude_text(frame)
                marker_output = marker in provider_output
                if frame.get("type") == "user" and prompt in json.dumps(frame, ensure_ascii=False):
                    claude_echoes.append(frame)
                if frame.get("type") == "result" and frame.get("subtype") == "success" and \
                        frame.get("is_error") is False and isinstance(frame.get("session_id"), str):
                    if marker_output:
                        claude_terminals.append(frame)
                if any(isinstance(part, dict) and part.get("type") == "tool_use"
                       for part in nested_text(frame, ("message", "content")) or []):
                    fail("Claude produced a tool-use frame despite the no-tool prompt")
            elif driver in ("opencode", "grok"):
                provider_output += acp_text(frame)
                marker_output = marker in provider_output
                if frame.get("result", {}).get("stopReason") == "end_turn":
                    acp_end_candidates.append((frame, marker_output))
                if frame.get("method") == "session/request_permission":
                    fail(f"{driver} produced a permission request despite the no-tool prompt")
                if frame.get("method") == "session/update" and nested_text(frame,
                        ("params", "update", "sessionUpdate")) in ("tool_call", "tool_call_update"):
                    fail(f"{driver} produced a tool-call frame despite the no-tool prompt")
        if driver == "claude":
            inits = [frame for _, frame in scoped if frame.get("type") == "system" and
                     frame.get("subtype") == "init" and isinstance(frame.get("session_id"), str)]
            if len(claude_terminals) == 1:
                terminal_session = claude_terminals[0]["session_id"]
            prompt_echo = len(claude_echoes) == 1 and len(inits) == 1 and \
                inits[0]["session_id"] == terminal_session
            vendor_end = len(claude_terminals) == 1 and prompt_echo
            evidence = case.get("modelEffortEvidence", {})
            argv = evidence.get("argv", {})
            if evidence.get("basis") != "ORIGINAL_H_CLAUDE_CAPABILITY" or \
                    argv.get("basis") != "ACTUAL_PRODUCT_DESCENDANT_PROCESS_COMMAND_LINE_FILTERED" or \
                    argv.get("imageSha256") != case["fixedSha256"] or \
                    not argv.get("processId") or argv.get("modelArgCount") != 1 or argv.get("effortArgCount") != 1 or \
                    not isinstance(argv.get("commandLineSha256"), str) or len(argv["commandLineSha256"]) != 64 or \
                    argv.get("modelFlag") != "--model" or argv.get("effortFlag") != "--effort" or \
                    argv.get("model") != expected_settings.get("model") or \
                    argv.get("effort") != expected_settings.get("effort") or \
                    argv.get("productRootPid") != str(journal.get("currentEndpoint", {}).get("pid")):
                fail("Claude model/effort lacks original capability and actual pinned child argv evidence")
            if evidence.get("model") != expected_settings.get("model") or \
                    evidence.get("effort") != expected_settings.get("effort"):
                fail("Claude H capability does not match original bound E model/effort")
        else:
            outbound_prompts = []
            for command_row in result["commands"]:
                if command_row["sessionId"] != session_id or not command_row["confirmedWrite"]:
                    continue
                try:
                    command_frame = json.loads(command_row["originalFrame"])
                except json.JSONDecodeError:
                    continue
                if command_frame.get("method") == "session/prompt" and \
                        any(isinstance(item, dict) and item.get("type") == "text" and
                            prompt in item.get("text", "") for item in command_frame.get("params", {}).get("prompt", [])):
                    outbound_prompts.append(command_frame)
            terminal_ids = {json.dumps(frame.get("id"), separators=(",", ":")) for frame in outbound_prompts}
            prompt_echo = len(outbound_prompts) == 1
            matching_ends = [frame for frame, saw_marker in acp_end_candidates if saw_marker and isinstance(frame, dict) and
                frame.get("result", {}).get("stopReason") == "end_turn" and
                json.dumps(frame.get("id"), separators=(",", ":")) in terminal_ids]
            vendor_end = len(matching_ends) == 1 and marker_output
            if driver == "opencode":
                evidence = case.get("modelEffortEvidence", {})
                if evidence.get("basis") != "REQUIRES_ORIGINAL_ACP_MODEL_EFFORT_ACK_READBACK":
                    fail("OpenCode model/effort must be established by original ACP acknowledgements")
                open_request_id = episodes[0][7]
                process_operation = episodes[0][1]
                vendor_sessions = []
                for row, frame in scoped:
                    if frame.get("method") is None:
                        # session/new is a response and shares the original H/A custody scope.
                        for command_row in result["commands"]:
                            if command_row["sessionId"] != session_id or command_row["phase"] not in ("WRITTEN", "OBSERVED"):
                                continue
                            try:
                                sent_frame = json.loads(command_row["originalFrame"])
                            except json.JSONDecodeError:
                                continue
                            if sent_frame.get("method") == "session/new" and sent_frame.get("id") == frame.get("id"):
                                candidate = nested_text(frame, ("result", "sessionId"))
                                if isinstance(candidate, str): vendor_sessions.append(candidate)
                if len(set(vendor_sessions)) != 1:
                    fail("OpenCode has no unique original ACP vendor session id")
                vendor_session = vendor_sessions[0]
                acknowledged = []
                for config_id in ("model", "effort"):
                    step_id = f"{process_operation}-setting-{config_id}"
                    ack = exact_one(db,
                        "SELECT s.command_hex,r.raw_bytes,s.source_epoch,s.source_cursor "
                        "FROM gogoke_v37_rpc_steps s JOIN v37_ledger_raw_source r "
                        "ON r.operation_id=s.process_operation_id AND r.source_epoch=s.source_epoch "
                        "AND r.source_cursor=s.source_cursor AND r.process_ticket=s.ticket "
                        "AND r.custodian_nonce=s.custodian_nonce AND r.domain_id=s.domain_id "
                        "AND r.session_id=s.session_id AND r.generation=s.generation "
                        "WHERE s.domain_id=? AND s.session_id=? AND s.process_operation_id=? "
                        "AND s.generation=? AND s.open_request_id=? AND s.ticket=? AND s.custodian_nonce=? "
                        "AND s.step_id=? AND s.phase='OBSERVED' AND s.requires_response=1 "
                        "AND r.state='NO_EVENT' AND r.no_event_reason='ACP_RPC_RESPONSE'",
                        (domain, session_id, process_operation, send[4], open_request_id,
                         send[5], send[7], step_id))
                    command_frame = json.loads(bytes.fromhex(ack[0]).decode("utf-8").rstrip("\n"))
                    response_frame = json.loads(bytes(ack[1]).decode("utf-8"))
                    wanted = expected_settings.get(config_id)
                    if command_frame.get("jsonrpc") != "2.0" or command_frame.get("method") != "session/set_config_option" or \
                            command_frame.get("params", {}).get("sessionId") != vendor_session or \
                            command_frame.get("params", {}).get("configId") != config_id or \
                            command_frame.get("params", {}).get("value") != wanted or \
                            type(response_frame.get("id")) is not type(command_frame.get("id")) or \
                            response_frame.get("id") != command_frame.get("id") or "error" in response_frame:
                        fail(f"OpenCode original ACP {config_id} command/response identity differs")
                    options = nested_text(response_frame, ("result", "configOptions"))
                    matches = [value for value in options or []
                               if isinstance(value, dict) and value.get("id") == config_id]
                    if len(matches) != 1 or matches[0].get("type") != "select" or \
                            matches[0].get("currentValue") != wanted or not any(
                                isinstance(option, dict) and option.get("value") == wanted
                                for option in matches[0].get("options", [])):
                        fail(f"OpenCode original ACP {config_id} acknowledgement did not confirm the exact value")
                    acknowledged.append({"configId": config_id, "stepId": step_id,
                        "requestId": command_frame["id"], "sourceEpoch": ack[2], "sourceCursor": ack[3]})
                if acknowledged[0]["sourceEpoch"] != acknowledged[1]["sourceEpoch"] or \
                        int(acknowledged[0]["sourceCursor"]) >= int(acknowledged[1]["sourceCursor"]):
                    fail("OpenCode model/effort ACP acknowledgements are not ordered in one original epoch")
                record_model_evidence = {"basis": "ORIGINAL_ACP_MODEL_EFFORT_ACKS",
                    "vendorSessionId": vendor_session, "acknowledgements": acknowledged}
            else:
                evidence = case.get("modelEffortEvidence", {})
                argv = evidence.get("argv", {})
                if evidence.get("basis") != "REQUIRES_CAPTURED_ACTUAL_PROCESS_ARGV" or \
                        argv.get("basis") != "ACTUAL_PRODUCT_DESCENDANT_PROCESS_COMMAND_LINE_FILTERED" or \
                        argv.get("imageSha256") != case["fixedSha256"] or \
                        not argv.get("processId") or argv.get("modelArgCount") != 1 or argv.get("effortArgCount") != 1 or \
                        not isinstance(argv.get("commandLineSha256"), str) or len(argv["commandLineSha256"]) != 64 or \
                        argv.get("modelFlag") != "--model" or argv.get("effortFlag") != "--reasoning-effort" or \
                        argv.get("model") != expected_settings.get("model") or \
                        argv.get("effort") != expected_settings.get("effort") or \
                        argv.get("productRootPid") != str(journal.get("currentEndpoint", {}).get("pid")):
                    fail("Grok model/effort lacks actual pinned child argv evidence")
                record_model_evidence = {"basis": "ACTUAL_PINNED_GROK_PROCESS_ARGV",
                    "processId": argv.get("processId"), "commandLineSha256": argv.get("commandLineSha256"),
                    "model": argv.get("model"), "effort": argv.get("effort")}
        if not prompt_echo or not marker_output or not vendor_end:
            fail(f"{driver}: matching original A prompt, provider marker output, and vendor end-turn are not all present")
        normalized = db.execute(
            "SELECT i.cursor,i.source_epoch,i.source_cursor,i.update_json,r.operation_id,r.generation,"
            "r.process_ticket,r.custodian_nonce FROM v37_ledger_index i LEFT JOIN v37_ledger_raw_source r "
            "ON r.resolved_event_id=i.source_event_id AND r.domain_id=i.domain_id AND r.session_id=i.session_id "
            "WHERE i.source_kind='v37' AND i.domain_id=? AND i.session_id=? ORDER BY i.cursor",
            (domain, session_id)).fetchall()
        updates = [{"cursor": str(cursor), "sourceEpoch": epoch, "ledgerSourceCursor": source_cursor,
            "operationId": operation_id, "generation": generation, "processTicket": ticket,
            "custodianNonce": nonce, "update": json.loads(update)}
            for cursor, epoch, source_cursor, update, operation_id, generation, ticket, nonce in normalized]
        if not updates:
            fail(f"{driver}: original normalized provider output is absent")
        result["sessions"].append({"sessionId": session_id, "seatId": case["seatId"],
            "instanceId": case["instanceId"], "worktreeId": case["worktreeId"],
            "driverId": pins[0][0], "version": pins[0][1], "binarySha256": pins[0][2],
            "persistedInstallState": instance[3], "installObservationCredit": False,
            "episodes": episodes, "normalized": updates, "missingNormalized": False,
            "rawFrameCount": len(incoming), "unknownFrameCount": sum(row[5] == "PENDING" for row in incoming),
            "allEpisodesStopped": True, "claimState": claim[0], "providerPromptObserved": prompt_echo,
            "providerEndTurn": vendor_end, "markerObserved": marker_output,
            "vendorSessionId": terminal_session,
            "modelEffortEvidence": record_model_evidence if driver in ("opencode", "grok") else
                {"basis": "ACTUAL_PINNED_CLAUDE_PROCESS_ARGV", "processId": argv.get("processId"),
                 "commandLineSha256": argv.get("commandLineSha256"), "model": argv.get("model"),
                 "effort": argv.get("effort")}})

case_count = len(journal["cases"])
if len(result["providerWorktrees"]) != case_count or len(result["sessions"]) != case_count:
    fail("Every selected fixed provider F/H session must have direct readback")
if len({os.path.normcase(row["path"]) for row in result["providerWorktrees"]}) != case_count:
    fail("Provider F worktree roots overlap")
result["filesAfter"] = file_facts()
result["measurementPreservedDatabaseBytes"] = before == result["filesAfter"]
result["directProviderEvidence"] = result["measurementPreservedDatabaseBytes"] and \
    all(row["allEpisodesStopped"] and row["providerEndTurn"] and row["markerObserved"]
        for row in result["sessions"])
output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
if not result["directProviderEvidence"]:
    fail("Direct provider readback incomplete; preserve original private artifact")
