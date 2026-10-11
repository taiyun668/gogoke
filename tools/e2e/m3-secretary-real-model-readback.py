"""Immutable closed-DB readback for one original Secretary USER model turn."""
import hashlib
import json
import sqlite3
import sys
from pathlib import Path
from urllib.parse import quote


def require(value, message):
    if not value:
        raise RuntimeError(message)


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def one(db, sql, args):
    rows = db.execute(sql, args).fetchall()
    require(len(rows) == 1, f"Exact original row count differs: {len(rows)}")
    return rows[0]


def main():
    require(len(sys.argv) == 4, "STATE_ROOT OUTPUT JOURNAL required")
    root, output, journal_path = map(Path, sys.argv[1:])
    require(not output.exists(), "Refusing to overwrite immutable readback")
    journal = json.loads(journal_path.read_text(encoding="utf-8-sig"))
    require(journal.get("schema") == "gogoke.37.private-m3-v14-secretary-real-model.v1" and
            journal.get("state") == "CLOSED_PENDING_READBACK" and
            journal.get("model", {}).get("answerObserved") is True and
            journal.get("model", {}).get("newTurn") is True and
            journal.get("model", {}).get("steered") is False and
            len(journal.get("closes", [])) == 1 and
            journal["closes"][0].get("exitCode") == 0 and
            journal["closes"][0].get("forceKill") is False and
            journal["closes"][0].get("nativeExit", {}).get("code") == 0,
            "Original model evidence lacks one normal shell/native close")
    binding = journal["binding"]
    model = journal["model"]
    send = one_from_list(journal["operations"],
        lambda row: row.get("request", {}).get("requestId") == model["requestId"])
    request = send["request"]
    require(request.get("family") == "K-SESSION" and request.get("operation") == "send" and
            request.get("domainId") == "global" and request.get("targetId") == binding["sessionId"] and
            request.get("payload") == {"generation": binding["generation"], "body": journal["prompt"]} and
            send.get("rawFrame") == json.dumps(request, separators=(",", ":"), ensure_ascii=False),
            "Original USER send frame differs from the captured request")
    # JS JSON.stringify and Python dumps both preserve these ASCII-only fields and order.
    db_path = root / "state.sqlite"
    require(db_path.is_file() and not db_path.is_symlink(), "Original state.sqlite missing")
    for suffix in ("-wal", "-shm", "-journal"):
        sidecar = Path(str(db_path) + suffix)
        require(not sidecar.exists() or sidecar.stat().st_size == 0, "SQLite sidecar is not settled")
    before = sha256(db_path)
    uri = "file:" + quote(str(db_path).replace("\\", "/"), safe="/:") + "?mode=ro&immutable=1"
    db = sqlite3.connect(uri, uri=True)
    db.row_factory = sqlite3.Row
    db.execute("PRAGMA query_only=ON")
    try:
        args = ("global", binding["sessionId"], binding["generation"])
        claim = one(db,
            "SELECT state,stop_fact_id,process_operation_id,instance_id,generation "
            "FROM gogoke_v37_h_claim WHERE domain_id=? AND session_id=? AND generation=?", args)
        require(claim["state"] == "STOPPED" and claim["instance_id"] == binding["instanceId"] and
                claim["stop_fact_id"] and claim["stop_fact_id"].startswith("sha256:") and
                claim["process_operation_id"], "Original H did not reach STOPPED with StopFact")
        episode = one(db,
            "SELECT phase,stop_fact_id,stop_request_id,process_operation_id,seat_id "
            "FROM gogoke_v37_h_process_episode WHERE domain_id=? AND session_id=? AND generation=? "
            "AND process_operation_id IS NOT NULL", args)
        require(episode["phase"] == "STOPPED" and episode["stop_fact_id"] == claim["stop_fact_id"] and
                episode["process_operation_id"] == claim["process_operation_id"] and
                episode["seat_id"] == binding["seatId"] and episode["stop_request_id"],
                "Original process episode did not stop with the same fact")
        custody = one(db,
            "SELECT state,stop_proof_hash,profile_id,domain_id,generation,pid,creation_time_100ns,"
            "custodian_nonce,"
            "binary_digest_sha256 FROM gogoke_coordination_process_custody WHERE operation_id=?",
            (claim["process_operation_id"],))
        require(custody["state"] == "STOPPED" and custody["stop_proof_hash"] == claim["stop_fact_id"] and
                custody["profile_id"] == binding["instanceId"] and custody["domain_id"] == "global" and
                custody["generation"] == binding["generation"] and custody["pid"] and
                custody["creation_time_100ns"] and custody["binary_digest_sha256"],
                "Original process custody does not bind the stopped physical process")
        stop = one(db,
            "SELECT operation,status FROM gogoke_v37_h_operation WHERE domain_id=? AND session_id=? "
            "AND request_id=?", ("global", binding["sessionId"], episode["stop_request_id"]))
        require(stop["operation"] == "stop" and stop["status"] == "APPLIED",
                "Original native H stop operation is not APPLIED")
        stdin = one(db,
            "SELECT operation,request_hex,phase,receipt_status,receipt_hex,session_id,generation "
            "FROM gogoke_v37_h_stdin_journal WHERE domain_id=? AND request_id=?",
            ("global", model["requestId"]))
        require(stdin["operation"] == "send" and stdin["session_id"] == binding["sessionId"] and
                stdin["generation"] == binding["generation"] and stdin["phase"] == "RECEIPTED" and
                stdin["receipt_status"] == "APPLIED" and stdin["receipt_hex"] and
                bytes.fromhex(stdin["request_hex"]).decode("utf-8-sig").strip() == send["rawFrame"],
                "Original USER send bytes were not receipted by this H")
        native_receipt = json.loads(bytes.fromhex(stdin["receipt_hex"]).decode("utf-8-sig").strip())
        turn_id = model["startReceipt"]["receipt"]["result"]["turnId"]
        require(native_receipt.get("status") == "APPLIED" and
                native_receipt.get("requestId") == model["requestId"] and
                native_receipt.get("result", {}).get("createdTurn") is True and
                native_receipt.get("result", {}).get("turnId") == turn_id,
                "Original native receipt is not the same new Codex turn/start")
        terminal_meta = model["terminalAEvent"]["_meta"]
        require(terminal_meta.get("codexMethod") == "turn/completed" and
                terminal_meta.get("turnStatus") == "completed" and
                terminal_meta.get("turnId") == turn_id and
                terminal_meta.get("threadId") == binding["threadId"],
                "Journal terminal A event differs from original start")
        terminal = one(db,
            "SELECT raw_bytes,state FROM v37_ledger_raw_source WHERE operation_id=? "
            "AND domain_id=? AND session_id=? AND generation=? AND source_epoch=? AND source_cursor=?",
            (claim["process_operation_id"], "global", binding["sessionId"],
             binding["generation"], custody["custodian_nonce"], terminal_meta["rawSourceCursor"]))
        raw_terminal = json.loads(terminal["raw_bytes"].decode("utf-8-sig").strip())
        require(terminal["state"] == "RESOLVED" and raw_terminal.get("method") == "turn/completed" and
                raw_terminal.get("params", {}).get("threadId") == binding["threadId"] and
                raw_terminal.get("params", {}).get("turn", {}).get("id") == turn_id and
                raw_terminal.get("params", {}).get("turn", {}).get("status") == "completed",
                "Original producer raw source is not the completed Codex turn")
    finally:
        db.close()
    after = sha256(db_path)
    require(before == after, "Immutable measurement changed original database bytes")
    result = {"schema": "gogoke.37.private-m3-v14-secretary-real-model-closed.v1",
              "caseId": journal["caseId"], "sourceCommit": journal["sourceCommit"],
              "sessionId": binding["sessionId"], "generation": binding["generation"],
              "requestId": model["requestId"], "stopRequestId": episode["stop_request_id"],
              "processOperationId": claim["process_operation_id"], "stopFact": claim["stop_fact_id"],
              "terminalSourceEpoch": custody["custodian_nonce"],
              "terminalSourceCursor": terminal_meta["rawSourceCursor"],
              "process": {"pid": custody["pid"], "creationTime100ns": custody["creation_time_100ns"],
                          "binaryDigestSha256": custody["binary_digest_sha256"]},
              "databaseSha256Before": before, "databaseSha256After": after,
              "databaseBytesPreserved": True, "databaseWrites": False,
              "credentialReads": False, "acceptance": False}
    with output.open("x", encoding="utf-8") as stream:
        json.dump(result, stream, ensure_ascii=False, indent=2)
        stream.write("\n")


def one_from_list(rows, predicate):
    found = [row for row in rows if predicate(row)]
    require(len(found) == 1, "Exact original request count differs")
    return found[0]


if __name__ == "__main__":
    main()
