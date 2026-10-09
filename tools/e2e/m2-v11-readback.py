"""Read one normally closed V11 two-project F graph from the actual native store.

This proves the graph/physical layout case only. It makes no H or Model
write-refusal claim and never opens a credential or writes the database.
"""
import hashlib
import json
import os
import sqlite3
import sys
from pathlib import Path


def require(value, message):
    if not value:
        raise RuntimeError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def exactly(db, query, args=()):
    rows = db.execute(query, args).fetchall()
    require(len(rows) == 1, f"V11 expected one original row, got {len(rows)}")
    return rows[0]


def same_path(left, right):
    return os.path.normcase(str(Path(left).resolve(strict=True))) == os.path.normcase(
        str(Path(right).resolve(strict=True)))


def ordinary(path):
    return path.is_file() and not path.is_symlink() and not (
        getattr(path.lstat(), "st_file_attributes", 0) & 0x400)


def candidate_close(journal, case):
    launches, closes = journal.get("launches", []), journal.get("closes", [])
    pins = case.get("installedSha256", {})
    require(all(isinstance(pins.get(name), str) and len(pins[name]) == 64 and
                all(char in "0123456789abcdef" for char in pins[name])
                for name in ("gogoke.exe", "gogoke-native-host.exe", "resource-index.json")),
            "V11 exact installed product byte pins absent")
    require(launches and closes and launches[-1].get("pid") == closes[-1].get("pid") and
            closes[-1].get("exitCode") == 0 and closes[-1].get("forceKill") is False and
            launches[-1].get("sourceCommit") == case["sourceCommit"] and
            launches[-1].get("setId") == pins["resource-index.json"] and
            launches[-1].get("bootstrap", {}).get("version") == case.get("candidateVersion") and
            launches[-1].get("bootstrap", {}).get("setId") == pins["resource-index.json"] and
            launches[-1].get("bootstrap", {}).get("generationId") ==
            launches[-1].get("generationId"),
            "V11 actual candidate byte identity has no matching normal close")
    return closes[-1]


def file_boundaries(root, output, journal_file):
    journal = json.loads(journal_file.read_text(encoding="utf-8-sig"))
    case = journal.get("v11FileBoundaries")
    require(case and case.get("acceptance") is False and
            case.get("state") == "ORIGINAL_ATTEMPTS_REQUIRE_NORMAL_CLOSE_IMMUTABLE_READER" and
            case.get("sourceCommit") == journal.get("sourceCommit") and
            len(case.get("cases", [])) == 2,
            "V11 two original H boundary cases required")
    close = candidate_close(journal, case)
    dbfile, wal, shm = root / "state.sqlite", root / "state.sqlite-wal", root / "state.sqlite-shm"
    require(dbfile.is_file() and (not wal.exists() or wal.stat().st_size == 0),
            "V11 closed H/A checkpoint or empty WAL required")

    def files():
        return {name: digest(p.read_bytes()) if p.exists() else None for name, p in
                (("db", dbfile), ("wal", wal), ("shm", shm))}

    before = files()
    db = sqlite3.connect(f"file:{dbfile.as_posix()}?mode=ro&immutable=1", uri=True)
    db.row_factory = sqlite3.Row
    db.execute("PRAGMA query_only=ON")
    details = []
    try:
        operations = {row["request"]["requestId"]: row for row in journal["operations"]
                      if row.get("request", {}).get("requestId")}
        for item in case["cases"]:
            session = next((row for row in journal["sessions"] if row["id"] == item["sessionId"]), None)
            require(session and session["seatId"] == item["seatId"] and
                    session["instanceId"] == item["instanceId"] and
                    session["worktreeId"] == item["worktreeId"],
                    "V11 original H session differs from selected E/F binding")
            seat = exactly(db, "SELECT s.instance_id,t.settings_json FROM gogoke_v37_seats s "
                           "JOIN gogoke_v37_seat_settings t USING(domain_id,seat_id) "
                           "WHERE s.domain_id=? AND s.seat_id=?", (case["domainId"], item["seatId"]))
            require(seat[0] == item["instanceId"] and
                    json.loads(seat[1])["permissionTier"] == item["expectedTier"],
                    "V11 actual E permission tier differs")
            tree = exactly(db, "SELECT repository_id,domain_id,seat_id,instance_id,worktree_path,state "
                           "FROM gogoke_v37_worktrees WHERE worktree_id=?", (item["worktreeId"],))
            require(tuple(tree[:4]) == (case["repositoryId"], case["domainId"],
                                        item["seatId"], item["instanceId"]) and tree[5] == "REGISTERED",
                    "V11 original F tree differs")
            tree_path = Path(tree[4]).resolve(strict=True)
            require(tree_path.is_dir() and not tree_path.is_symlink() and
                    (root / "v37-worktrees").resolve(strict=True) in tree_path.parents,
                    "V11 original F physical tree is outside candidate root")
            source = exactly(db, "SELECT source_path FROM gogoke_v37_worktree_sources "
                             "WHERE repository_id=?", (case["repositoryId"],))[0]
            target = Path(item["target"])
            expected_parent = Path(source) if item["name"] == "MAIN_WRITE" else tree_path
            require(same_path(target.parent, expected_parent) and not target.exists(),
                    "V11 attempted private marker appeared or target parent changed")
            episode = exactly(db, "SELECT request_id,generation,raw_hex,process_operation_id,instance_id,"
                              "seat_id,phase,stop_fact_id FROM gogoke_v37_h_process_episode "
                              "WHERE domain_id=? AND session_id=?", (case["domainId"], item["sessionId"]))
            open_op = operations[item["openRequestId"]]
            require(episode[0] == item["openRequestId"] and episode[1] == session["generation"] and
                    bytes.fromhex(episode[2]).decode() == open_op["rawFrame"] and
                    episode[4] == item["instanceId"] and episode[5] == item["seatId"] and
                    episode[6] == "STOPPED" and episode[7] == item["stopFact"],
                    "V11 original H open/StopFact bytes differ")
            custody = exactly(db, "SELECT ticket,custodian_nonce,state,stop_proof_hash "
                              "FROM gogoke_coordination_process_custody WHERE operation_id=?",
                              (episode[3],))
            require(custody[2] == "STOPPED" and custody[3] == item["stopFact"],
                    "V11 physical H custody has no original stop proof")
            stdin = exactly(db, "SELECT request_hex,receipt_hex,phase,receipt_status,process_operation_id,"
                            "generation,ticket,custodian_nonce FROM gogoke_v37_h_stdin_journal "
                            "WHERE domain_id=? AND request_id=? AND session_id=?",
                            (case["domainId"], item["sendRequestId"], item["sessionId"]))
            original_send = operations[item["sendRequestId"]]
            sent = json.loads(bytes.fromhex(stdin[1]).decode())
            require(bytes.fromhex(stdin[0]).decode() == original_send["rawFrame"] and
                    stdin[2:4] == ("RECEIPTED", "APPLIED") and stdin[4] == episode[3] and
                    stdin[5] == session["generation"] and stdin[6:8] == custody[:2] and
                    sent["result"]["createdTurn"] is True and
                    sent["result"]["turnId"] == item["turnId"],
                    "V11 original H send/ACK or physical custody differs")
            sources = db.execute("SELECT generation,raw_bytes,state,process_ticket,custodian_nonce "
                                 "FROM v37_ledger_raw_source WHERE domain_id=? AND session_id=? "
                                 "AND operation_id=? ORDER BY rowid",
                                 (case["domainId"], item["sessionId"], episode[3])).fetchall()
            require(sources and all(row[0] == session["generation"] and row[2] != "PENDING" and
                                    row[3:5] == custody[:2] for row in sources),
                    "V11 original A source or custody is incomplete")
            frames = [json.loads(bytes(row[1]).decode()) for row in sources]
            terminal = [frame for frame in frames if frame.get("method") == "turn/completed" and
                        frame.get("params", {}).get("threadId") == session["threadId"] and
                        frame.get("params", {}).get("turn", {}).get("id") == item["turnId"]]
            require(len(terminal) == 1, "V11 original exact turn terminal missing")
            mutating = [frame for frame in frames if frame.get("method") == "item/completed" and
                        frame.get("params", {}).get("turnId") == item["turnId"] and
                        frame.get("params", {}).get("item", {}).get("type") in
                        ("fileChange", "commandExecution", "mcpToolCall", "dynamicToolCall")]
            original_tool = mutating[0]["params"]["item"] if len(mutating) == 1 else {}
            failure_source = original_tool.get("error") or original_tool.get("result") or \
                original_tool.get("aggregatedOutput")
            failure_text = json.dumps(failure_source, ensure_ascii=False) if failure_source is not None else ""
            exact_failed = original_tool.get("type") == "fileChange" and \
                original_tool.get("status") == "failed" and any(
                    isinstance(change, dict) and change.get("path") == str(target)
                    for change in original_tool.get("changes", [])) and any(
                    phrase in failure_text for phrase in
                    ("Access is denied", "Permission denied", "os error 5", "EACCES", "EPERM"))
            claim = exactly(db, "SELECT state FROM gogoke_v37_h_claim WHERE domain_id=? AND session_id=?",
                            (case["domainId"], item["sessionId"]))
            require(claim[0] == "RELEASED", "V11 original H admission was not released")
            details.append({"name": item["name"], "sessionId": item["sessionId"],
                            "turnId": item["turnId"], "stopFact": item["stopFact"],
                            "exactFailedFileChange": exact_failed,
                            "state": "DIRECT_ORIGINAL_FILE_CHANGE_PERMISSION_REFUSAL" if exact_failed else
                                     "NOT_RUN_NO_EXACT_ORIGINAL_FILE_CHANGE_REFUSAL"})
    finally:
        db.close()
    after = files()
    require(before == after, "V11 H/A readback changed DB/WAL/SHM bytes")
    result = {"schema": "gogoke.37.private-v11-file-boundaries-readback.v1",
              "sourceCommit": case["sourceCommit"], "normalClosePid": close["pid"],
              "databaseSha256": before["db"], "cases": details,
              "directCaseEvidence": all(row["exactFailedFileChange"] for row in details),
              "mainTreeFileWriteRefusal": next(row["state"] for row in details if row["name"] == "MAIN_WRITE"),
              "readOnlyFileWriteRefusal": next(row["state"] for row in details if row["name"] == "READ_ONLY_WRITE"),
              "vendorNativeWorktreeEscape": "NOT_RUN", "noNetworkBoundary": "NOT_RUN",
              "acceptance": False, "measurementPreservedDatabaseBytes": True}
    output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"directCaseEvidence": result["directCaseEvidence"], "acceptance": False}))


def main():
    require(len(sys.argv) in (4, 5), "usage: m2-v11-readback.py STATE_ROOT OUTPUT JOURNAL [file]")
    root = Path(sys.argv[1]).resolve(strict=True)
    output = Path(sys.argv[2]).resolve()
    journal_file = Path(sys.argv[3]).resolve(strict=True)
    require(not output.exists() and output.parent == journal_file.parent and
            not output.is_relative_to(root), "V11 fresh private output outside product root required")
    if len(sys.argv) == 5:
        require(sys.argv[4] == "file", "V11 unknown readback phase")
        return file_boundaries(root, output, journal_file)
    journal = json.loads(journal_file.read_text(encoding="utf-8-sig"))
    case = journal.get("v11Graph")
    require(case and case.get("acceptance") is False and
            case.get("state") == "ORIGINAL_USER_F_GRAPHS_RECORDED_PHYSICAL_READBACK_REQUIRED" and
            case.get("sourceCommit") == journal.get("sourceCommit") and
            same_path(case["stateRoot"], root) and len(case.get("graphs", [])) == 6,
            "V11 six original F graph facts and same candidate root required")
    close = candidate_close(journal, case)
    dbfile = root / "state.sqlite"
    wal, shm = root / "state.sqlite-wal", root / "state.sqlite-shm"
    require(dbfile.is_file() and (not wal.exists() or wal.stat().st_size == 0),
            "V11 closed checkpoint or empty WAL required")

    def files():
        return {name: digest(p.read_bytes()) if p.exists() else None for name, p in
                (("db", dbfile), ("wal", wal), ("shm", shm))}

    before = files()
    db = sqlite3.connect(f"file:{dbfile.as_posix()}?mode=ro&immutable=1", uri=True)
    db.row_factory = sqlite3.Row
    db.execute("PRAGMA query_only=ON")
    try:
        original_ops = {entry["request"]["requestId"]: entry for entry in journal["operations"]
                        if entry.get("kind") == "V11_ORIGINAL_USER_GRAPH"}
        require(len(original_ops) == len(case["operations"]) and
                set(original_ops) == set(case["operations"]), "V11 original User request set changed")
        for entry in original_ops.values():
            require(json.loads(entry["rawFrame"]) == entry["request"] and
                    entry["receipt"]["requestId"] == entry["request"]["requestId"],
                    "V11 original User frame/receipt identity changed")
        paths, groups = set(), {}
        for graph in case["graphs"]:
            domain, tree_id, repository, seat = (graph[key] for key in
                                                 ("domainId", "worktreeId", "repositoryId", "seatId"))
            graph_entry = original_ops.get(graph["graphRequestId"])
            require(graph_entry and graph_entry["request"]["domainId"] == domain and
                    graph_entry["request"]["targetId"] == tree_id and
                    graph_entry["request"]["operation"] == "graph-query" and
                    graph_entry["receipt"]["status"] == "APPLIED" and
                    graph_entry["receipt"]["result"] == graph["graph"],
                    "V11 original graph User receipt differs from recorded case")
            create = [entry for entry in original_ops.values() if entry["request"].get("domainId") == domain
                      and entry["request"].get("targetId") == tree_id
                      and entry["request"].get("operation") == "create"]
            register = [entry for entry in original_ops.values() if entry["request"].get("domainId") == domain
                        and entry["request"].get("targetId") == tree_id
                        and entry["request"].get("operation") == "register"]
            require(len(create) == len(register) == 1 and
                    create[0]["receipt"]["status"] == register[0]["receipt"]["status"] == "APPLIED",
                    "V11 original F create/register receipts missing")
            request = create[0]["request"]
            require(request["payload"] == {"repositoryId": repository, "seatId": seat,
                                           "layout": graph["layout"]},
                    "V11 F create layout or source differs")
            op = exactly(db, "SELECT request_hash,repository_id,domain_id,seat_id,worktree_id,phase "
                            "FROM gogoke_v37_worktree_operations WHERE request_id=?",
                         (request["requestId"],))
            require(tuple(op) == (digest(create[0]["rawFrame"].encode()), repository, domain,
                                  seat, tree_id, "REGISTERED"),
                    "V11 persisted F create request/identity differs")
            registration = exactly(db, "SELECT request_hash,worktree_id,operation,phase "
                                   "FROM gogoke_v37_worktree_lifecycle_ops WHERE request_id=?",
                                   (register[0]["request"]["requestId"],))
            require(tuple(registration) == (digest(register[0]["rawFrame"].encode()), tree_id,
                                            "REGISTER", "APPLIED"),
                    "V11 persisted F registration differs")
            row = exactly(db, "SELECT repository_id,domain_id,seat_id,instance_id,worktree_path,"
                          "git_pointer_hash,baseline_commit,state FROM gogoke_v37_worktrees WHERE worktree_id=?",
                          (tree_id,))
            require(tuple(row[:4]) == (repository, domain, seat, graph["instanceId"]) and
                    row[7] == "REGISTERED" and len(row[6]) == 40,
                    "V11 native F tree binding differs")
            source = exactly(db, "SELECT source_path,git_digest,git_version FROM "
                             "gogoke_v37_worktree_sources WHERE repository_id=?", (repository,))
            require(same_path(source[0], case["sources"][repository]) and
                    source[1].startswith("sha256:") and source[2],
                    "V11 original registered physical source/Git pin differs")
            path = Path(row[4])
            physical = path.resolve(strict=True)
            layout_root = root / "v37-worktrees" / graph["layout"]
            require(same_path(path, physical) and layout_root.resolve(strict=True) in physical.parents and
                    physical.is_dir() and not physical.is_symlink() and str(physical) not in paths,
                    "V11 registered F tree is replaced, duplicated or outside its native layout")
            paths.add(str(physical))
            pointer = physical / ".git"
            require(ordinary(pointer) and row[5] == "sha256:" + digest(pointer.read_bytes()),
                    "V11 original F Git pointer differs")
            members = db.execute("SELECT space_id FROM gogoke_v37_worktree_members WHERE worktree_id=?",
                                 (tree_id,)).fetchall()
            if graph["layout"] == "mixed":
                require(len(members) == 1 and members[0][0] == graph["spaceId"],
                        "V11 mixed native space membership differs")
                space = exactly(db, "SELECT classification,state,path_id FROM gogoke_v37_worktree_spaces "
                                "WHERE space_id=?", (graph["spaceId"],))
                require(space[0] == "MIXED" and space[1] == "ACTIVE" and
                        physical.parent == layout_root / space[2],
                        "V11 mixed physical parent differs from original F space")
                groups.setdefault(domain, []).append((graph["spaceId"], physical.parent, repository))
            else:
                require(not members and graph["spaceId"] == tree_id,
                        "V11 single tree entered a mixed native space")
        require(len(groups) == 2 and all(len(values) == 2 and
                values[0][:2] == values[1][:2] for values in groups.values()) and
                len({values[0][0] for values in groups.values()}) == 2 and
                len({values[0][1] for values in groups.values()}) == 2,
                "V11 MIXED spaces or physical parents cross the two projects")
    finally:
        db.close()
    after = files()
    require(before == after, "V11 immutable readback changed DB/WAL/SHM bytes")
    result = {"schema": "gogoke.37.private-v11-graph-readback.v1",
              "sourceCommit": case["sourceCommit"], "caseState": case["state"],
              "normalClosePid": close["pid"], "databaseSha256": before["db"],
              "directGraphEvidence": True, "modelTreeRefusal": "NOT_RUN",
              "permissionTierRefusal": "NOT_RUN", "acceptance": False,
              "measurementPreservedDatabaseBytes": True}
    output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"directGraphEvidence": True, "acceptance": False}))


if __name__ == "__main__":
    main()
