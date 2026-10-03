"""Readonly post-close observer for real provider capability preflight.

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
    return Path(text[4:] if text.startswith("\\\\?\\") else text).resolve(strict=True)

def within(parent, child):
    parent_text, child_text = os.path.normcase(str(parent)), os.path.normcase(str(child))
    return os.path.commonpath((parent_text, child_text)) == parent_text

result = {"schema": "gogoke.37.private-m2-provider-boundaries.v1", "databaseWrites": False,
          "credentialReads": False, "acceptance": False, "filesBefore": file_facts(),
          "rootIdentity": [str(root.stat().st_dev), str(root.stat().st_ino)],
          "cases": [], "frames": [], "commands": [], "unknownFrames": [],
          "checks": {"V03b": "NOT_RUN", "V04b": "NOT_RUN", "V10": "NOT_RUN"}}
with sqlite3.connect(db_file.as_uri() + "?mode=ro&immutable=1", uri=True) as db:
    db.execute("PRAGMA query_only=ON")
    for case in cases:
        driver = case["driverId"]
        if driver == "antigravity" or case["state"] == "NOT_RUN_NOT_LOGGED_IN":
            result["cases"].append({"caseId": case["caseId"], "driverId": driver,
                "state": case["state"], "checks": case["checks"], "source": "ORIGINAL_STATUS_ONLY_NO_SESSION"})
            continue
        if driver not in ("claude", "opencode", "grok") or \
                case["state"] != "REAL_FIXED_CAPABILITY_PREFLIGHT_ONLY":
            raise RuntimeError("Provider case did not finish its original preflight")
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
        if report.get("driverId") != driver or report.get("version") != instance[1] or \
                report.get("binaryDigest") != instance[2] or \
                report.get("capabilities", {}).get("nativeQuestionCard") != "UNSUPPORTED_REPLY_ENCODER" or \
                report.get("capabilities", {}).get("memoryOffLaunch") != "NOT_RUN":
            raise RuntimeError("Current provider capability boundary differs; update the case contract first")
        sends = db.execute("SELECT request_id FROM gogoke_v37_h_stdin_journal "
                           "WHERE domain_id=? AND session_id=? AND operation='send'",
                           (domain, session_id)).fetchall()
        if sends:
            raise RuntimeError("Preflight unexpectedly sent a model prompt")
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
        result["cases"].append({"caseId": case["caseId"], "driverId": driver,
            "sessionId": session_id, "state": "DIRECT_FIXED_CAPABILITY_PREFLIGHT_ONLY",
            "actualVersion": instance[1], "actualDigest": instance[2],
            "worktreePath": str(worktree_path), "worktreeIdentity": worktree[5],
            "H": {"episode": episodes[0], "custody": custody, "claim": claim},
            "capability": report, "rawFrameCount": len(raw_rows),
            "normalized": [{"cursor": str(cursor), "sourceEpoch": epoch,
                "ledgerSourceCursor": source_cursor, "operationId": operation,
                "generation": generation, "processTicket": ticket,
                "custodianNonce": nonce, "update": json.loads(update)}
                for cursor, epoch, source_cursor, update, operation, generation, ticket, nonce in normalized],
            "missingNormalized": len(normalized) == 0,
            "checks": case["checks"], "unknownFrameCount": len(unknown)})

result["memoryObserver"] = {"basis": "NOT_OBSERVED_AFTER_NORMAL_CLOSE", "V04b": "NOT_RUN"}
memory_refs = journal.get("snapshots", {})
if all(name in memory_refs for name in ("memory-before", "memory-after")):
    measured = {}
    for phase in ("before", "after"):
        reference = memory_refs[f"memory-{phase}"]
        if Path(reference["file"]).name != reference["file"]:
            raise RuntimeError("Private memory snapshot reference is not a basename")
        source = journal_file.parent / reference["file"]
        raw = source.read_bytes()
        if hash_bytes(raw) != reference["sha256"]:
            raise RuntimeError("Original read-only memory snapshot bytes changed")
        value = json.loads(raw)
        measured[phase] = {"file": reference["file"], "sha256": reference["sha256"],
                           "memoryDataUnchangedByRead": value.get("memoryDataUnchangedByRead"),
                           "stage1OutputCount": value.get("stage1OutputCount"),
                           "memoryJobCount": value.get("memoryJobCount")}
    result["memoryObserver"] = {"basis": "EXISTING_READONLY_SNAPSHOT_NOT_VENDOR_MEMORY_PROOF",
                                 "snapshots": measured, "V04b": "NOT_RUN"}

result["filesAfter"] = file_facts()
result["measurementPreservedDatabaseBytes"] = result["filesBefore"] == result["filesAfter"]
output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
if not result["measurementPreservedDatabaseBytes"]:
    raise RuntimeError("Readonly provider measurement changed candidate database bytes")
