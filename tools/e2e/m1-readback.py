"""Read existing candidate ledger after normal product close; never credentials.

Usage: signed Python m1-readback.py STATE_ROOT OUTPUT [E2E_JOURNAL]
Without a journal, supply the real ledger epoch/cursor bootstrap to the E2E.
With a journal, export only its actual sessions and check recorded CLI evidence.
All outputs stay in the private ordinary-view evidence directory.
"""
import hashlib
import json
import sqlite3
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve(strict=True)
output = Path(sys.argv[2])
if output.exists():
    raise RuntimeError("Evidence output already exists")
database = root / "state.sqlite"

def files():
    return {p.name: {"length": p.stat().st_size,
                     "sha256": hashlib.sha256(p.read_bytes()).hexdigest()}
            for p in (database, Path(str(database) + "-wal"), Path(str(database) + "-shm")) if p.exists()}

wal, shm = Path(str(database) + "-wal"), Path(str(database) + "-shm")
if wal.exists() and wal.stat().st_size:
    raise RuntimeError("Product must be closed and checkpointed; refuse a nonempty WAL")
result = {"schema": "gogoke.37.private-e2e-ledger.v1", "databaseWrites": False,
          "credentialReads": False, "rootIdentity": [root.stat().st_dev, root.stat().st_ino],
          "filesBefore": files(), "frames": [], "commands": [], "sessions": []}

# The caller has closed the actual product. immutable disables WAL sidecar
# creation; it is never used to observe an active writer's database.
with sqlite3.connect(database.as_uri() + "?mode=ro&immutable=1", uri=True) as connection:
    connection.execute("PRAGMA query_only=ON")
    result["epoch"] = connection.execute("SELECT epoch FROM v37_ledger_meta WHERE singleton=1").fetchone()[0]
    result["cursor"] = str(connection.execute("SELECT COALESCE(MAX(cursor),0) FROM v37_ledger_index").fetchone()[0])
    if len(sys.argv) == 4:
        journal = json.loads(Path(sys.argv[3]).read_text(encoding="utf-8-sig"))
        domain = journal["domainId"]
        for session in journal["sessions"]:
            session_id = session["id"]
            episodes = connection.execute(
                "SELECT generation,process_operation_id,phase,stop_fact_id FROM gogoke_v37_h_process_episode "
                "WHERE domain_id=? AND session_id=? ORDER BY rowid", (domain, session_id)).fetchall()
            incoming = connection.execute(
                "SELECT generation,operation_id,source_epoch,source_cursor,raw_bytes,state,process_ticket,custodian_nonce FROM v37_ledger_raw_source "
                "WHERE domain_id=? AND session_id=? ORDER BY rowid", (domain, session_id)).fetchall()
            outgoing = connection.execute(
                "SELECT generation,process_operation_id,step_id,command_hex,phase,source_epoch,source_cursor,ticket,custodian_nonce "
                "FROM gogoke_v37_rpc_steps WHERE domain_id=? AND session_id=? ORDER BY rowid",
                (domain, session_id)).fetchall()
            by_episode = {}
            parsed_in = []
            for generation, operation, epoch, cursor, raw, state, ticket, nonce in incoming:
                text = bytes(raw).decode("utf-8")
                frame = json.loads(text)
                result["frames"].append({"direction": "in", "sessionId": session_id,
                    "generation": generation, "operationId": operation, "sourceEpoch": epoch,
                    "sourceCursor": cursor, "processTicket": ticket, "custodianNonce": nonce,
                    "state": state, "originalFrame": text})
                by_episode.setdefault((operation, epoch), []).append(int(cursor))
                parsed_in.append(frame)
            for generation, operation, step, command, phase, epoch, cursor, ticket, nonce in outgoing:
                # INTENT/UNKNOWN is evidence of intent/uncertainty, never a sent frame.
                result["commands"].append({"direction": "out", "sessionId": session_id,
                    "generation": generation, "operationId": operation, "stepId": step,
                    "phase": phase, "sourceEpoch": epoch, "sourceCursor": cursor,
                    "processTicket": ticket, "custodianNonce": nonce,
                    "originalFrame": bytes.fromhex(command).decode("utf-8"),
                    "confirmedWrite": phase in ("WRITTEN", "OBSERVED")})
            normalized = connection.execute(
                "SELECT i.cursor,i.source_epoch,i.source_cursor,i.update_json,r.operation_id,r.generation,r.process_ticket,r.custodian_nonce "
                "FROM v37_ledger_index i LEFT JOIN v37_ledger_raw_source r "
                "ON r.resolved_event_id=i.source_event_id AND r.domain_id=i.domain_id AND r.session_id=i.session_id "
                "WHERE i.source_kind='v37' AND i.domain_id=? AND i.session_id=? ORDER BY i.cursor",
                (domain, session_id)).fetchall()
            completed = [frame for frame in parsed_in if frame.get("method") == "turn/completed"
                         and frame.get("params", {}).get("turn", {}).get("status") == "completed"]
            compactions = [frame for frame in parsed_in if frame.get("method") == "item/completed"
                           and frame.get("params", {}).get("item", {}).get("type") == "contextCompaction"]
            command_completions = [frame["params"]["item"] for frame in parsed_in
                if frame.get("method") == "item/completed"
                and frame.get("params", {}).get("item", {}).get("type") == "commandExecution"]
            summary = {"sessionId": session_id, "episodes": episodes,
                "normalized": [{"cursor": str(cursor), "sourceEpoch": epoch,
                    "operationId": operation, "generation": generation,
                    "processTicket": ticket, "custodianNonce": nonce,
                    "ledgerSourceCursor": source, "update": json.loads(update)}
                    for cursor, epoch, source, update, operation, generation, ticket, nonce in normalized],
                "successfulTurns": len(completed), "contextCompactionCompletions": len(compactions),
                "sourceCursorsContinuous": all(values == list(range(1, max(values) + 1))
                    for values in by_episode.values()),
                "allEpisodesStopped": bool(episodes) and all(row[2] == "STOPPED" and row[3] for row in episodes),
                "unresolvedRawFrames": sum(row[5] == "PENDING" for row in incoming)}
            summary["unresolvedRawMethods"] = [json.loads(bytes(row[4]).decode("utf-8")).get("method")
                for row in incoming if row[5] == "PENDING"]
            summary["commandCompletions"] = [{"id": item.get("id"), "status": item.get("status"),
                "exitCode": item.get("exitCode")} for item in command_completions]
            summary["allObservedCommandsSucceeded"] = all(item.get("status") == "completed"
                and item.get("exitCode") == 0 for item in command_completions)
            summary["steerConsumptionRequired"] = bool(session.get("steerMarker"))
            original_turn = session.get("turns", [{}])[0] if session.get("turns") else {}
            summary["steerConsumedInOriginalCompletion"] = bool(session.get("steerMarker")) and any(
                frame.get("method") == "item/completed"
                and frame.get("params", {}).get("threadId") == original_turn.get("threadId")
                and frame.get("params", {}).get("turnId") == original_turn.get("turnId")
                and frame.get("params", {}).get("item", {}).get("type") == "agentMessage"
                and session["steerMarker"] in frame.get("params", {}).get("item", {}).get("text", "")
                for frame in parsed_in)
            summary["everyObservedTurnDurable"] = bool(session.get("turns")) and all(
                any(frame.get("params", {}).get("threadId") == turn["threadId"]
                    and frame.get("params", {}).get("turn", {}).get("id") == turn["turnId"]
                    for frame in completed)
                and any(row["update"].get("_meta", {}).get("codexMethod") == "turn/completed"
                    and row["update"].get("_meta", {}).get("threadId") == turn["threadId"]
                    and row["update"].get("_meta", {}).get("turnId") == turn["turnId"]
                    and row["update"].get("_meta", {}).get("turnStatus") == "completed"
                    for row in summary["normalized"])
                for turn in session.get("turns", []))
            result["sessions"].append(summary)
        # This observer checks direct protocol/stop evidence, not Owner acceptance.
        result["actualFlowReportedComplete"] = journal["state"] == "ACTUAL_FLOW_COMPLETE_DIRECT_LEDGER_READBACK_REQUIRED"
        result["directReadbackComplete"] = result["actualFlowReportedComplete"] and len(result["sessions"]) == 2 and all(
            row["successfulTurns"] >= (2 if index == 0 else 1) and row["everyObservedTurnDurable"]
            and row["allObservedCommandsSucceeded"]
            and row["sourceCursorsContinuous"] and row["allEpisodesStopped"]
            and (not row["steerConsumptionRequired"] or row["steerConsumedInOriginalCompletion"])
            for index, row in enumerate(result["sessions"])) and result["sessions"][0]["contextCompactionCompletions"] > 0

result["filesAfter"] = files()
result["measurementPreservedDatabaseBytes"] = result["filesBefore"] == result["filesAfter"]
output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
if not result["measurementPreservedDatabaseBytes"]:
    raise RuntimeError("Readonly measurement changed database bytes")
if len(sys.argv) == 4 and not result["directReadbackComplete"]:
    raise RuntimeError("Real E2E direct readback incomplete; preserve original frames, do not claim PASS")
