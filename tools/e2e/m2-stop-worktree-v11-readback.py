"""Read one closed original V11 outside-F write attempt; never infer ACL cause from prose."""
import hashlib
import json
import os
import sqlite3
import sys
from pathlib import Path


def require(value, reason):
    if not value:
        raise RuntimeError(reason)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.exists() else None


def one(db, sql, args=()):
    values = db.execute(sql, args).fetchall()
    require(len(values) == 1, f"V11 expected one original row, found {len(values)}")
    return values[0]


def rpc_id(value):
    require(type(value) in (int, str), "V11 original RPC ID type differs")
    return type(value).__name__, value


def disjoint(left, right):
    left, right = Path(left).resolve(strict=True), Path(right).resolve(strict=True)
    return left != right and left not in right.parents and right not in left.parents


def object_fact(path, required):
    original = Path(path)
    if not original.exists():
        require(not required, "V11 registered F/source object is missing")
        return {"path": str(original), "identity": None}
    resolved = original.resolve(strict=True)
    stat = resolved.stat()
    return {"path": str(original), "resolved": str(resolved),
            "identity": [str(stat.st_dev), str(stat.st_ino)]}


def inventory(db, root, outside):
    trees = []
    for row in db.execute("SELECT worktree_id,repository_id,domain_id,seat_id,instance_id,"
                          "worktree_path,state FROM gogoke_v37_worktrees ORDER BY worktree_id"):
        path, state = row[5], row[6]
        fact = object_fact(path, state == "REGISTERED")
        # Even an old non-registered path must not be reused as this test target.
        require(disjoint(outside, path) if fact["identity"] is not None else
                Path(outside).resolve(strict=True) != Path(path) and
                Path(path) not in Path(outside).parents and
                Path(outside) not in Path(path).parents,
                "V11 outside target overlaps an original F path")
        trees.append({"row": list(row), "object": fact})
    sources = []
    for row in db.execute("SELECT repository_id,source_path,source_identity,common_path,"
                          "common_identity,baseline_commit,git_digest FROM "
                          "gogoke_v37_worktree_sources ORDER BY repository_id"):
        source, common = object_fact(row[1], True), object_fact(row[3], True)
        require(disjoint(outside, row[1]) and disjoint(outside, row[3]),
                "V11 outside target overlaps an original F source/common object")
        sources.append({"row": list(row), "source": source, "common": common})
    require(trees and sources and disjoint(outside, root),
            "V11 F/source inventory or isolated outside target is missing")
    return {"trees": trees, "sources": sources}


def preflight():
    require(len(sys.argv) == 5,
            "usage: m2-stop-worktree-v11-readback.py preflight STATE_ROOT OUTPUT OUTSIDE_ROOT")
    root = Path(sys.argv[2]).resolve(strict=True)
    output = Path(sys.argv[3]).resolve()
    outside = Path(sys.argv[4]).resolve(strict=True)
    require(not output.exists() and not output.is_relative_to(root) and
            outside.is_dir() and not outside.is_symlink() and not list(outside.iterdir()) and
            disjoint(outside, output.parent), "V11 fresh closed preflight/output required")
    dbfile = root / "state.sqlite"
    wal, shm = root / "state.sqlite-wal", root / "state.sqlite-shm"
    require(dbfile.is_file() and (not wal.exists() or wal.stat().st_size == 0),
            "V11 preflight needs a closed database and empty/absent WAL")
    paths = (dbfile, wal, shm)
    before = [digest(path) for path in paths]
    db = sqlite3.connect(dbfile.as_uri() + "?mode=ro&immutable=1", uri=True)
    db.execute("PRAGMA query_only=ON")
    try:
        facts = inventory(db, root, outside)
    finally:
        db.close()
    require(before == [digest(path) for path in paths] and
            (not wal.exists() or wal.stat().st_size == 0),
            "V11 preflight changed original DB/WAL/SHM bytes")
    result = {"schema": "gogoke.37.private-v11-outside-preflight.v1",
              "stateRoot": str(root), "root": object_fact(root, True),
              "database": object_fact(dbfile, True),
              "outsideRoot": str(outside), "outside": object_fact(outside, True),
              "inventory": facts, "databaseSha256": before[0],
              "measurementPreservedDatabaseBytes": True, "acceptance": False}
    output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"preflight": "CLOSED_INVENTORY_DISJOINT", "acceptance": False}))


def main():
    require(len(sys.argv) == 4, "usage: m2-stop-worktree-v11-readback.py STATE_ROOT OUTPUT JOURNAL")
    root = Path(sys.argv[1]).resolve(strict=True)
    output = Path(sys.argv[2]).resolve()
    journal_path = Path(sys.argv[3]).resolve(strict=True)
    require(not output.exists() and output.parent == journal_path.parent and
            not output.is_relative_to(root), "V11 fresh private output required")
    journal = json.loads(journal_path.read_text(encoding="utf-8-sig"))
    case = journal.get("v11OutsideTree")
    require(case and case.get("state") == "ORIGINAL_OUTSIDE_ATTEMPT_REQUIRES_IMMUTABLE_READBACK" and
            case.get("acceptance") is False and case.get("domainId") == journal.get("domainId") and
            case.get("repositoryId") == journal.get("repositoryId") and
            Path(journal["stateRoot"]).resolve(strict=True) == root,
            "V11 original outside case/root differs")
    launches, closes = journal.get("launches", []), journal.get("closes", [])
    require(launches and closes and launches[-1].get("pid") == closes[-1].get("pid") and
            closes[-1].get("exitCode") == 0 and closes[-1].get("forceKill") is False and
            launches[-1].get("sourceCommit") == journal.get("sourceCommit") and
            launches[-1].get("setId") == journal["candidateInstalledSha256"]["resource-index.json"] and
            launches[-1].get("bootstrap", {}).get("version") == journal.get("candidateVersion") and
            launches[-1].get("generationId") == launches[-1].get("bootstrap", {}).get("generationId"),
            "V11 actual installed candidate has no exact normal-close proof")
    dbfile = root / "state.sqlite"
    wal, shm = root / "state.sqlite-wal", root / "state.sqlite-shm"
    require(dbfile.is_file() and (not wal.exists() or wal.stat().st_size == 0),
            "V11 normally closed database/empty WAL required")

    def files():
        return {name: digest(path) for name, path in (("db", dbfile), ("wal", wal), ("shm", shm))}

    before = files()
    reference = journal.get("v11OutsidePreflight")
    require(reference and Path(reference.get("file", "")).name == reference.get("file") and
            len(reference.get("sha256", "")) == 64,
            "V11 original closed preflight reference missing")
    baseline_file = journal_path.parent / reference["file"]
    require(digest(baseline_file) == reference["sha256"],
            "V11 original closed preflight bytes changed")
    baseline = json.loads(baseline_file.read_text(encoding="utf-8-sig"))
    require(baseline.get("schema") == "gogoke.37.private-v11-outside-preflight.v1" and
            baseline.get("acceptance") is False and baseline.get("stateRoot") == str(root) and
            baseline.get("databaseSha256") and baseline.get("outsideRoot") ==
            str(Path(case["outsideRoot"]).resolve(strict=True)),
            "V11 original closed preflight does not bind this outside target")
    db = sqlite3.connect(dbfile.as_uri() + "?mode=ro&immutable=1", uri=True)
    db.row_factory = sqlite3.Row
    db.execute("PRAGMA query_only=ON")
    try:
        require(baseline["root"] == object_fact(root, True) and
                baseline["database"] == object_fact(dbfile, True) and
                baseline["outside"] == object_fact(case["outsideRoot"], True) and
                baseline["inventory"] == inventory(db, root, Path(case["outsideRoot"])),
                "V11 original F/source/outside object inventory drifted since closed preflight")
        operations = {row["request"]["requestId"]: row for row in journal["operations"]
                      if row.get("request", {}).get("requestId")}
        sessions = [row for row in journal["sessions"] if row.get("id") == case["sessionId"]]
        require(len(sessions) == 1 and sessions[0].get("generation") == case["generation"] and
                sessions[0].get("seatId") == case["seatId"] and
                sessions[0].get("instanceId") == case["instanceId"] and
                sessions[0].get("worktreeId") == case["worktreeId"],
                "V11 original outside H session journal differs")
        for name, operation in (("reserveRequestId", "admission-reserve"),
                                ("commitRequestId", "admission-commit"),
                                ("openRequestId", "open"), ("stopRequestId", "stop"),
                                ("releaseRequestId", "admission-release")):
            request_id = case[name]
            original = operations[request_id]
            stored = one(db, "SELECT raw_hex,status FROM gogoke_v37_h_operation "
                         "WHERE domain_id=? AND session_id=? AND request_id=? AND operation=?",
                         (case["domainId"], case["sessionId"], request_id, operation))
            require(bytes.fromhex(stored[0]).decode() == original["rawFrame"] and
                    original["receipt"]["status"] == stored[1] == "APPLIED" and
                    original["request"]["targetId"] == case["sessionId"],
                    "V11 original H request/receipt differs")
        episode = one(db, "SELECT request_id,generation,raw_hex,process_operation_id,instance_id,"
                      "seat_id,phase,stop_fact_id FROM gogoke_v37_h_process_episode "
                      "WHERE domain_id=? AND session_id=?", (case["domainId"], case["sessionId"]))
        require(episode[0] == case["openRequestId"] and episode[1] == case["generation"] and
                bytes.fromhex(episode[2]).decode() == operations[case["openRequestId"]]["rawFrame"] and
                operations[case["openRequestId"]]["receipt"]["result"]["threadId"] == case["threadId"] and
                episode[4:8] == (case["instanceId"], case["seatId"], "STOPPED", case["stopFact"]),
                "V11 original H episode/StopFact differs")
        custody = one(db, "SELECT ticket,custodian_nonce,state,stop_proof_hash "
                      "FROM gogoke_coordination_process_custody WHERE operation_id=?", (episode[3],))
        require(custody[2:] == ("STOPPED", case["stopFact"]),
                "V11 original process custody stop proof differs")
        claim = one(db, "SELECT state FROM gogoke_v37_h_claim WHERE domain_id=? AND session_id=?",
                    (case["domainId"], case["sessionId"]))
        require(claim[0] == "RELEASED", "V11 original H admission was not released")
        original_send = operations[case["sendRequestId"]]
        stdin = one(db, "SELECT request_hex,receipt_hex,phase,receipt_status,process_operation_id,"
                    "generation,ticket,custodian_nonce FROM gogoke_v37_h_stdin_journal "
                    "WHERE domain_id=? AND session_id=? AND request_id=?",
                    (case["domainId"], case["sessionId"], case["sendRequestId"]))
        require(bytes.fromhex(stdin[0]).decode() == original_send["rawFrame"] and
                original_send["request"]["operation"] == "send" and
                original_send["request"]["payload"] ==
                {"generation": case["generation"], "body": case["body"]} and
                stdin[2:4] == ("RECEIPTED", "APPLIED") and stdin[4] == episode[3] and
                stdin[5] == case["generation"] and stdin[6:8] == custody[:2] and
                json.loads(bytes.fromhex(stdin[1]).decode()) == case["sendReceipt"] and
                case["sendReceipt"]["result"]["turnId"] == case["turnId"],
                "V11 original H input/turn ACK or custody differs")
        seat = one(db, "SELECT s.instance_id,t.settings_json FROM gogoke_v37_seats s "
                   "JOIN gogoke_v37_seat_settings t USING(domain_id,seat_id) "
                   "WHERE s.domain_id=? AND s.seat_id=?", (case["domainId"], case["seatId"]))
        require(seat[0] == case["instanceId"] and
                json.loads(seat[1])["permissionTier"] == "NETWORKED_WRITE",
                "V11 original E tier differs")
        tree = one(db, "SELECT repository_id,domain_id,seat_id,instance_id,worktree_path,state "
                   "FROM gogoke_v37_worktrees WHERE worktree_id=?", (case["worktreeId"],))
        require(tree[:4] == (case["repositoryId"], case["domainId"],
                             case["seatId"], case["instanceId"]) and tree[5] == "REGISTERED" and
                os.path.samefile(tree[4], case["worktreePath"]),
                "V11 original registered F tree differs")
        outside = Path(case["outsideRoot"]).resolve(strict=True)
        target = Path(case["target"])
        require(outside.is_dir() and not outside.is_symlink() and
                os.path.samefile(target.parent, outside) and not list(outside.iterdir()) and
                disjoint(outside, root) and disjoint(outside, tree[4]) and
                disjoint(outside, journal["evidenceDirectory"]) and
                all(disjoint(outside, row[0]) for row in db.execute(
                    "SELECT worktree_path FROM gogoke_v37_worktrees WHERE state='REGISTERED'")),
                "V11 original outside target became a registered tree or was created")
        sources = db.execute("SELECT source_path FROM gogoke_v37_worktree_sources").fetchall()
        require(sources and all(disjoint(outside, row[0]) for row in sources),
                "V11 outside target aliases a registered repository source")
        raw = db.execute("SELECT generation,raw_bytes,state,process_ticket,custodian_nonce,"
                         "source_epoch,source_cursor FROM v37_ledger_raw_source "
                         "WHERE domain_id=? AND session_id=? AND operation_id=? ORDER BY rowid",
                         (case["domainId"], case["sessionId"], episode[3])).fetchall()
        require(raw and all(row[0] == case["generation"] and row[2] != "PENDING" and
                            row[3:5] == custody[:2] for row in raw),
                "V11 original A raw source/custody incomplete")
        for epoch in {row[5] for row in raw}:
            cursors = sorted(int(row[6]) for row in raw if row[5] == epoch)
            require(cursors == list(range(1, max(cursors) + 1)),
                    "V11 original A source has a gap")
        frames = [json.loads(bytes(row[1]).decode()) for row in raw]
        steps = db.execute("SELECT command_hex,phase,source_epoch,source_cursor,generation,ticket,"
                           "custodian_nonce FROM gogoke_v37_rpc_steps WHERE domain_id=? AND "
                           "session_id=? AND process_operation_id=? ORDER BY rowid",
                           (case["domainId"], case["sessionId"], episode[3])).fetchall()
        require(steps and all(row[1] in ("WRITTEN", "OBSERVED") and
                              row[4] == case["generation"] and row[5:7] == custody[:2]
                              for row in steps), "V11 original H RPC steps differ")
        commands = [(row, json.loads(bytes.fromhex(row[0]).decode())) for row in steps
                    if row[1] == "OBSERVED"]
        native_starts = [(row, frame) for row, frame in commands
                         if frame.get("method") == "thread/start"]
        require(len(native_starts) == 1, "V11 original Codex thread/start differs")
        native_acks = [(row, frame) for row, frame in zip(raw, frames)
                       if "method" not in frame and "id" in frame and
                       rpc_id(frame["id"]) == rpc_id(native_starts[0][1]["id"])]
        require(len(native_acks) == 1 and native_acks[0][0][5:7] == native_starts[0][0][2:4] and
                native_acks[0][1].get("result", {}).get("thread", {}).get("id") == case["threadId"],
                "V11 original Codex thread/start ACK differs")
        starts = [(row, frame) for row, frame in commands if frame.get("method") == "turn/start" and
                  frame.get("params", {}).get("threadId") == case["threadId"] and
                  frame["params"].get("input") == [{"type": "text", "text": case["body"]}]]
        require(len(starts) == 1, "V11 original outside turn/start differs")
        acks = [(row, frame) for row, frame in zip(raw, frames) if "method" not in frame and
                "id" in frame and rpc_id(frame["id"]) == rpc_id(starts[0][1]["id"])]
        require(len(acks) == 1 and acks[0][0][5:7] == starts[0][0][2:4] and
                acks[0][1].get("result", {}).get("turn", {}).get("id") == case["turnId"],
                "V11 original outside turn ACK differs")
        terminal = [frame for frame in frames if frame.get("method") == "turn/completed" and
                    frame.get("params", {}).get("threadId") == case["threadId"] and
                    frame["params"].get("turn", {}).get("id") == case["turnId"]]
        require(len(terminal) == 1 and terminal[0]["params"]["turn"].get("status") in
                ("completed", "failed"), "V11 original outside terminal differs")
        tools = [frame for frame in frames if frame.get("method") in ("item/started", "item/completed") and
                 frame.get("params", {}).get("threadId") == case["threadId"] and
                 frame["params"].get("turnId") == case["turnId"] and
                 frame["params"].get("item", {}).get("type") in
                 ("fileChange", "commandExecution", "mcpToolCall", "dynamicToolCall")]
        unrelated = [frame for frame in frames if frame.get("params", {}).get("threadId") ==
                     case["threadId"] and frame["params"].get("turnId") == case["turnId"] and
                     (frame.get("method") == "item/tool/call" or
                      (frame.get("method") in ("item/started", "item/completed") and
                       frame["params"].get("item", {}).get("type") not in
                       ("agentMessage", "reasoning", "contextCompaction", "fileChange",
                        "commandExecution", "mcpToolCall", "dynamicToolCall")))]
        started = [frame["params"]["item"] for frame in tools if frame["method"] == "item/started"]
        completed = [frame["params"]["item"] for frame in tools if frame["method"] == "item/completed"]
        marker_atom = "".join(char if char in
                              "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
                              else "_" for char in case["marker"])
        require(case["command"] == f'echo V11_OUTSIDE_{marker_atom}>"{target}"',
                "V11 original outside command/target differs")
        exact = not unrelated and len(tools) == 2 and len(started) == len(completed) == 1 and \
            started[0].get("id") == completed[0].get("id") and \
            started[0].get("type") == completed[0].get("type") == "commandExecution" and \
            started[0].get("command") == completed[0].get("command") == case["command"] and \
            ("cwd" not in started[0] or os.path.samefile(started[0]["cwd"], tree[4])) and \
            ("cwd" not in completed[0] or os.path.samefile(completed[0]["cwd"], tree[4])) and \
            completed[0].get("status") in ("completed", "failed") and \
            type(completed[0].get("exitCode")) is int and completed[0]["exitCode"] != 0
        result = {"schema": "gogoke.37.private-v11-outside-readback.v1",
                  "sourceCommit": journal["sourceCommit"], "normalClosePid": closes[-1]["pid"],
                  "sessionId": case["sessionId"], "turnId": case["turnId"],
                  "stopFact": case["stopFact"], "outsideTarget": str(target),
                  "exactFailedOriginalTool": exact,
                  "originalToolStatus": completed[0].get("status") if len(completed) == 1 else None,
                  "originalExitCode": completed[0].get("exitCode") if len(completed) == 1 else None,
                  "originalError": completed[0].get("error") if len(completed) == 1 else None,
                  "originalOutput": completed[0].get("aggregatedOutput") if len(completed) == 1 else None,
                  "directAttemptEvidence": exact, "directCaseEvidence": False,
                  "permissionCause": "UNATTRIBUTED", "acceptance": False,
                  "databaseSha256": before["db"], "measurementPreservedDatabaseBytes": True}
    finally:
        db.close()
    require(before == files() and (not wal.exists() or wal.stat().st_size == 0),
            "V11 immutable outside readback changed DB/WAL/SHM bytes")
    output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"directAttemptEvidence": result["directAttemptEvidence"],
                      "acceptance": False}))


if __name__ == "__main__":
    preflight() if len(sys.argv) > 1 and sys.argv[1] == "preflight" else main()
