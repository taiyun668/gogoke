"""Immutable readback for two original H/Codex private V11 Git turns.

The probe and vendor attempt are separate normally closed phases because H's
live normalized commandExecution update does not contain the raw exitCode.
"""
import hashlib
import json
import os
import sqlite3
import sys
from pathlib import Path


def need(value, message):
    if not value:
        raise RuntimeError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def one(db, query, args=()):
    rows = db.execute(query, args).fetchall()
    need(len(rows) == 1, "V11 expected one original row: " + query)
    return rows[0]


def same(a, b):
    # Win32 extended and ordinary spellings can identify the same registered
    # object. Compare its file identity rather than the display spelling.
    return os.path.samefile(a, b)


def same_name(a, b):
    return os.path.normcase(os.path.abspath(a)) == os.path.normcase(os.path.abspath(b))


def rpc_id(value):
    need(type(value) in (int, str), "V11 original RPC ID type changed")
    return type(value).__name__, value


def snapshot(source):
    entries = []

    def visit(folder, prefix):
        for child in sorted(folder.iterdir(), key=lambda item: item.name):
            need(all(0x20 <= ord(char) <= 0x7e for char in child.name),
                 "V11 private source snapshot requires ASCII fixture names")
            need(not child.is_symlink() and
                 same_name(str(child), str(child.resolve(strict=True))),
                 "V11 private source has a link or reparse")
            relative = prefix + "/" + child.name if prefix else child.name
            if child.is_dir():
                entries.append([relative + "/", None])
                visit(child, relative)
            else:
                need(child.is_file(), "V11 private source has a nonordinary entry")
                entries.append([relative, sha(child.read_bytes())])

    visit(source, "")
    return sha(json.dumps(entries, ensure_ascii=False, separators=(",", ":")).encode())


def installed_close(journal, record):
    pins = record.get("installedSha256", {})
    need(all(isinstance(pins.get(name), str) and len(pins[name]) == 64 and
             all(char in "0123456789abcdef" for char in pins[name])
             for name in ("gogoke.exe", "gogoke-native-host.exe", "resource-index.json")),
         "V11 installed candidate byte pins missing")
    launches, closes = journal.get("launches", []), journal.get("closes", [])
    need(launches and closes and launches[-1].get("pid") == closes[-1].get("pid") and
         closes[-1].get("exitCode") == 0 and closes[-1].get("forceKill") is False and
         launches[-1].get("sourceCommit") == record["sourceCommit"] and
         launches[-1].get("setId") == pins["resource-index.json"] and
         launches[-1].get("bootstrap", {}).get("version") == record["candidateVersion"] and
         launches[-1].get("bootstrap", {}).get("setId") == pins["resource-index.json"] and
         launches[-1].get("bootstrap", {}).get("generationId") ==
         launches[-1].get("generationId"),
         "V11 original installed candidate has no matching normal close")
    return closes[-1]


def read_attempt(db, root, journal, record, attempt, command):
    domain = record["domainId"]
    sid = attempt["sessionId"]
    sessions = [row for row in journal["sessions"] if row.get("id") == sid]
    need(len(sessions) == 1, "V11 original H session missing or duplicate")
    session = sessions[0]
    need(tuple(session[key] for key in ("seatId", "instanceId", "worktreeId")) ==
         tuple(attempt[key] for key in ("seatId", "instanceId", "worktreeId")),
         "V11 H session differs from original seat/F binding")
    seat = one(db, "SELECT s.instance_id,t.settings_json FROM gogoke_v37_seats s "
               "JOIN gogoke_v37_seat_settings t USING(domain_id,seat_id) "
               "WHERE s.domain_id=? AND s.seat_id=?", (domain, attempt["seatId"]))
    need(seat[0] == attempt["instanceId"] and
         json.loads(seat[1])["permissionTier"] == "NETWORKED_WRITE",
         "V11 original E seat/tier differs")
    tree = one(db, "SELECT repository_id,domain_id,seat_id,instance_id,worktree_path,state "
               "FROM gogoke_v37_worktrees WHERE worktree_id=?", (attempt["worktreeId"],))
    need(tuple(tree[:4]) == (record["repositoryId"], domain, attempt["seatId"],
                            attempt["instanceId"]) and tree[5] == "REGISTERED" and
         same(tree[4], attempt["worktreePath"]),
         "V11 original F registered tree differs")
    tree_path = Path(tree[4]).resolve(strict=True)
    need(tree_path.is_dir() and not tree_path.is_symlink() and
         not (getattr(tree_path.lstat(), "st_file_attributes", 0) & 0x400) and
         any(same(root / "v37-worktrees", ancestor) for ancestor in tree_path.parents),
         "V11 original F physical tree is outside candidate root")
    source = one(db, "SELECT source_path FROM gogoke_v37_worktree_sources "
                 "WHERE repository_id=?", (record["repositoryId"],))[0]
    need(same(source, record["testbedSource"]), "V11 original F source differs from private testbed")
    operations = {row["request"]["requestId"]: row for row in journal["operations"]
                  if row.get("request", {}).get("requestId")}
    episode = one(db, "SELECT request_id,generation,raw_hex,process_operation_id,instance_id,"
                  "seat_id,phase,stop_fact_id FROM gogoke_v37_h_process_episode "
                  "WHERE domain_id=? AND session_id=?", (domain, sid))
    opened = operations[attempt["openRequestId"]]
    need(episode[0] == attempt["openRequestId"] and
         episode[1] == session["generation"] and
         bytes.fromhex(episode[2]).decode() == opened["rawFrame"] and
         episode[4:6] == (attempt["instanceId"], attempt["seatId"]) and
         episode[6:8] == ("STOPPED", attempt["stopFact"]),
         "V11 original H open/StopFact bytes differ")
    custody = one(db, "SELECT ticket,custodian_nonce,state,stop_proof_hash "
                  "FROM gogoke_coordination_process_custody WHERE operation_id=?", (episode[3],))
    need(custody[2:4] == ("STOPPED", attempt["stopFact"]),
         "V11 H process custody has no physical stop proof")
    for key, operation in (("stopRequestId", "stop"),
                           ("releaseRequestId", "admission-release")):
        raw = operations[attempt[key]]
        stored = one(db, "SELECT raw_hex,status FROM gogoke_v37_h_operation "
                     "WHERE domain_id=? AND request_id=? AND session_id=? AND operation=?",
                     (domain, attempt[key], sid, operation))
        need(bytes.fromhex(stored[0]).decode() == raw["rawFrame"] and
             stored[1] == raw["receipt"]["status"] == "APPLIED",
             "V11 original H stop/release receipt differs")
    send = operations[attempt["sendRequestId"]]
    stdin = one(db, "SELECT request_hex,receipt_hex,phase,receipt_status,process_operation_id,"
                "generation,ticket,custodian_nonce FROM gogoke_v37_h_stdin_journal "
                "WHERE domain_id=? AND request_id=? AND session_id=?",
                (domain, attempt["sendRequestId"], sid))
    receipt = json.loads(bytes.fromhex(stdin[1]).decode())
    need(bytes.fromhex(stdin[0]).decode() == send["rawFrame"] and
         json.loads(send["rawFrame"]) == send["request"] and
         send["request"]["domainId"] == domain and send["request"]["targetId"] == sid and
         send["request"]["operation"] == "send" and
         send["request"]["payload"] == {"generation": session["generation"],
                                          "body": attempt["body"]} and
         stdin[2:4] == ("RECEIPTED", "APPLIED") and
         stdin[4:6] == (episode[3], session["generation"]) and
         stdin[6:8] == custody[:2] and receipt == attempt["sendReceipt"] and
         receipt["status"] == "APPLIED" and receipt["result"]["createdTurn"] is True and
         receipt["result"]["turnId"] == attempt["turnId"],
         "V11 original H send/ACK/custody differs")
    sources = db.execute("SELECT generation,raw_bytes,state,process_ticket,custodian_nonce,"
                         "source_epoch,source_cursor FROM v37_ledger_raw_source "
                         "WHERE domain_id=? AND session_id=? AND operation_id=? ORDER BY rowid",
                         (domain, sid, episode[3])).fetchall()
    need(sources and all(row[0] == session["generation"] and row[2] != "PENDING" and
                         row[3:5] == custody[:2] for row in sources),
         "V11 original A raw stream/custody incomplete")
    for epoch in {row[5] for row in sources}:
        cursors = sorted(int(row[6]) for row in sources if row[5] == epoch)
        need(cursors == list(range(1, max(cursors) + 1)), "V11 original A stream has a gap")
    frames = [json.loads(bytes(row[1]).decode()) for row in sources]
    steps = db.execute("SELECT command_hex,phase,source_epoch,source_cursor,generation,"
                       "ticket,custodian_nonce FROM gogoke_v37_rpc_steps WHERE domain_id=? "
                       "AND session_id=? AND process_operation_id=? ORDER BY rowid",
                       (domain, sid, episode[3])).fetchall()
    need(steps and all(row[4] == session["generation"] and row[1] in
                       ("WRITTEN", "OBSERVED") and row[5:7] == custody[:2]
                       for row in steps), "V11 original RPC custody incomplete")
    starts = [(row, json.loads(bytes.fromhex(row[0]).decode())) for row in steps
              if row[1] == "OBSERVED"]
    starts = [(row, frame) for row, frame in starts if frame.get("method") == "turn/start" and
              frame.get("params", {}).get("threadId") == session["threadId"] and
              frame["params"].get("input") == [{"type": "text", "text": attempt["body"]}]]
    need(len(starts) == 1, "V11 original H body has no Codex turn/start")
    acks = [(row, frame) for row, frame in zip(sources, frames)
            if "method" not in frame and "id" in frame and
            rpc_id(frame["id"]) == rpc_id(starts[0][1]["id"])]
    need(len(acks) == 1 and acks[0][0][5:7] == starts[0][0][2:4] and
         acks[0][1].get("result", {}).get("turn", {}).get("id") == attempt["turnId"],
         "V11 original A ACK not bound to H turn/start")
    terminal = [frame for frame in frames if frame.get("method") == "turn/completed" and
                frame.get("params", {}).get("threadId") == session["threadId"] and
                frame["params"].get("turn", {}).get("id") == attempt["turnId"]]
    need(len(terminal) == 1 and terminal[0]["params"]["turn"].get("status") in
         ("completed", "failed"), "V11 exact original turn terminal missing")
    tool = [frame for frame in frames if frame.get("method") in
            ("item/started", "item/completed") and
            frame.get("params", {}).get("threadId") == session["threadId"] and
            frame["params"].get("turnId") == attempt["turnId"] and
            frame["params"].get("item", {}).get("type") in
            ("commandExecution", "fileChange", "mcpToolCall", "dynamicToolCall")]
    unrelated = [frame for frame in frames if frame.get("params", {}).get("threadId") ==
                 session["threadId"] and frame["params"].get("turnId") == attempt["turnId"] and
                 (frame.get("method") == "item/tool/call" or
                  (frame.get("method") in ("item/started", "item/completed") and
                   frame["params"].get("item", {}).get("type") not in
                   ("agentMessage", "reasoning", "contextCompaction",
                    "commandExecution", "fileChange", "mcpToolCall", "dynamicToolCall")))]
    begun = [frame["params"]["item"] for frame in tool if frame["method"] == "item/started"]
    ended = [frame["params"]["item"] for frame in tool if frame["method"] == "item/completed"]
    first = begun[0] if len(begun) == 1 else {}
    last = ended[0] if len(ended) == 1 else {}
    exact = (not unrelated and len(tool) == 2 and len(begun) == len(ended) == 1 and
             first.get("type") == last.get("type") == "commandExecution" and
             isinstance(last.get("id"), str) and bool(last["id"]) and first.get("id") == last["id"] and
             first.get("command") == last.get("command") == command and
             ("cwd" not in last or same(last["cwd"], tree[4])) and
             last.get("status") in ("completed", "failed") and type(last.get("exitCode")) is int)
    claim = one(db, "SELECT state FROM gogoke_v37_h_claim WHERE domain_id=? AND session_id=?",
                (domain, sid))
    need(claim[0] == "RELEASED", "V11 H admission was not released")
    return {"phase": attempt["phase"], "sessionId": sid, "threadId": session["threadId"],
            "turnId": attempt["turnId"], "stopFact": attempt["stopFact"],
            "exactOriginalCommand": bool(exact), "originalStatus": last.get("status"),
            "originalExitCode": last.get("exitCode"), "originalError": last.get("error"),
            "originalOutput": last.get("aggregatedOutput")}


def main():
    need(len(sys.argv) == 5 and sys.argv[4] in ("probe", "final"),
         "usage: m2-v11-git-readback.py STATE_ROOT OUTPUT JOURNAL probe|final")
    root = Path(sys.argv[1]).resolve(strict=True)
    output = Path(sys.argv[2]).resolve()
    journal_path = Path(sys.argv[3]).resolve(strict=True)
    need(not output.exists() and output.parent == journal_path.parent and
         not output.is_relative_to(root), "V11 fresh private output outside product root required")
    journal = json.loads(journal_path.read_text(encoding="utf-8-sig"))
    record = journal.get("v11GitBoundary")
    phase = sys.argv[4]
    expected_state = ("PROBE_REQUIRES_NORMAL_CLOSE_IMMUTABLE_READBACK" if phase == "probe" else
                      "VENDOR_ATTEMPT_REQUIRES_NORMAL_CLOSE_IMMUTABLE_READBACK")
    need(record and record.get("acceptance") is False and record.get("state") == expected_state and
         record.get("sourceCommit") == journal.get("sourceCommit") and
         record.get("repositoryId") == "gogokeSeatTestbed" and
         len(record.get("attempts", [])) == (1 if phase == "probe" else 2),
         "V11 original Git phase/candidate/journal differs")
    close = installed_close(journal, record)
    dbfile, wal, shm = root / "state.sqlite", root / "state.sqlite-wal", root / "state.sqlite-shm"
    need(dbfile.is_file() and (not wal.exists() or wal.stat().st_size == 0),
         "V11 normally closed H/A database or empty WAL required")

    def physical_files():
        return {key: sha(file.read_bytes()) if file.exists() else None for key, file in
                (("db", dbfile), ("wal", wal), ("shm", shm))}

    before = physical_files()
    db = sqlite3.connect(dbfile.as_uri() + "?mode=ro&immutable=1", uri=True)
    db.row_factory = sqlite3.Row
    db.execute("PRAGMA query_only=ON")
    try:
        source = one(db, "SELECT source_path,git_digest,git_version FROM "
                     "gogoke_v37_worktree_sources WHERE repository_id=?",
                     (record["repositoryId"],))
        program = one(db, "SELECT git_path FROM gogoke_v37_worktree_programs "
                      "WHERE repository_id=?", (record["repositoryId"],))[0]
        need(same(source[0], record["testbedSource"]) and
             same(record["privateTestbedRoot"], Path(source[0]).parent) and
             Path(source[0]).is_dir(), "V11 F source/private root differs")
        pin = record.get("fixedGitPath")
        fixed_pin = (bool(pin) and same_name(pin, program) and same(pin, program) and
                     source[1] == "sha256:" + sha(Path(program).read_bytes()))
        probe_command = '"' + pin + '" --version' if pin else "git --version"
        need(record["probeCommand"] == probe_command and
             record["attempts"][0]["phase"] == "GIT_VERSION" and
             record["attempts"][0]["command"] == probe_command,
             "V11 original Git probe command changed")
        probe = read_attempt(db, root, journal, record, record["attempts"][0], probe_command)
        probe_zero = probe["exactOriginalCommand"] and probe["originalExitCode"] == 0
        fixed_reachable = bool(probe_zero and fixed_pin)
        vendor = None
        if phase == "final":
            need(fixed_reachable, "V11 vendor attempt lacks original reachable fixed Git")
            proof_path = Path(record["probeReadbackPath"]).resolve(strict=True)
            proof_bytes = proof_path.read_bytes()
            proof = json.loads(proof_bytes)
            need(sha(proof_bytes) == record["probeReadbackSha256"] and
                 proof["schema"] == "gogoke.37.private-v11-git-readback.v1" and
                 proof["phase"] == "probe" and proof["fixedGitReachable"] is True and
                 proof["sourceCommit"] == record["sourceCommit"] and
                 any(row.get("pid") == proof["normalClosePid"] and
                     row.get("exitCode") == 0 and row.get("forceKill") is False
                     for row in journal["closes"][:-1]) and
                 proof["probeTurnId"] == probe["turnId"] and
                 proof["probeStopFact"] == probe["stopFact"] and
                 proof["probeCommand"] == probe_command,
                 "V11 prior normal-close probe reader changed")
            target = Path(record["vendorTarget"])
            need(record.get("vendorTargetAbsentBefore") is True and
                 same(target.parent, record["privateTestbedRoot"]) and
                 target.name.startswith("v11-unregistered-vendor-tree-"),
                 "V11 vendor target is outside the private sibling root")
            command = ('"' + pin + '" -C "' + record["testbedSource"] +
                       '" worktree add --detach "' + str(target) + '" HEAD')
            need(record["vendorCommand"] == command and
                 record["attempts"][1]["phase"] == "VENDOR_WORKTREE_ADD" and
                 record["attempts"][1]["command"] == command,
                 "V11 original vendor command changed")
            original = read_attempt(db, root, journal, record, record["attempts"][1], command)
            current_source = snapshot(Path(source[0]))
            unchanged = (record["beforeSourceSha256"] ==
                         record["afterSourceSha256"] == current_source)
            target_absent = not target.exists()
            vendor = {**original, "target": str(target),
                      "targetAbsent": target_absent,
                      "sourceUnchanged": unchanged,
                      "state": ("ORIGINAL_COMMAND_NONZERO_TARGET_ABSENT_SOURCE_UNCHANGED_CAUSE_UNATTRIBUTED"
                                if original["exactOriginalCommand"] and
                                original["originalExitCode"] != 0 and target_absent and unchanged
                                else "UNEXPECTED_EFFECT_OR_ORIGINAL_COMMAND_UNQUALIFIED")}
    finally:
        db.close()
    need(before == physical_files(), "V11 immutable reader changed DB/WAL/SHM bytes")
    result = {"schema": "gogoke.37.private-v11-git-readback.v1", "phase": phase,
              "sourceCommit": record["sourceCommit"], "normalClosePid": close["pid"],
              "databaseSha256": before["db"], "probeTurnId": probe["turnId"],
              "probeStopFact": probe["stopFact"], "probeCommand": probe_command,
              "fixedGitPath": pin, "registeredGitDigest": source[1],
              "registeredGitVersion": source[2],
              "originalGitProbe": probe, "gitProbeExitZero": bool(probe_zero),
              "fixedGitReachable": fixed_reachable, "vendorAttempt": vendor,
              "vendorNativeWorktreeBoundary": vendor["state"] if vendor else "NOT_RUN",
              "acceptance": False, "measurementPreservedDatabaseBytes": True}
    output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"phase": phase, "fixedGitReachable": fixed_reachable,
                      "vendorNativeWorktreeBoundary": result["vendorNativeWorktreeBoundary"],
                      "acceptance": False}))


if __name__ == "__main__":
    main()
