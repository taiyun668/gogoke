"""Immutable H/F stop-gate readback after normal close.

Signed Python: STATE_ROOT OUTPUT JOURNAL stopped|final. No DB writes,
credentials, model output, Git invocation, or product launch occur here.
"""
import hashlib
import json
import sqlite3
import sys
from pathlib import Path


def check(value, message):
    if not value:
        raise RuntimeError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def rows(db, sql, args=()):
    return [dict(row) for row in db.execute(sql, args)]


def one(db, sql, args=()):
    found = rows(db, sql, args)
    check(len(found) == 1, f"One original row required, found {len(found)}")
    return found[0]


def operation(journal, request_id):
    found = [entry for entry in journal["operations"]
             if entry.get("request", {}).get("requestId") == request_id]
    check(len(found) == 1, "One original User operation required")
    entry = found[0]
    check(json.loads(entry["rawFrame"]) == entry["request"] and
          entry["receipt"] and entry["receipt"]["requestId"] == request_id and
          entry["receipt"]["operation"] == entry["request"]["operation"] and
          entry["receipt"]["targetId"] == entry["request"]["targetId"],
          "Original User request/receipt wire identity differs")
    return entry


def session_facts(db, journal, case):
    session = case["session"]
    domain, sid = case["domainId"], session["id"]
    requests = [session[key] for key in ("reserveRequestId", "commitRequestId", "openRequestId",
                                          "stopRequestId", "releaseRequestId")]
    records = [operation(journal, request_id) for request_id in requests]
    for entry in records:
        req = entry["request"]
        check(req["family"] == "K-SESSION" and req["domainId"] == domain and req["targetId"] == sid and
              req["payload"]["generation"] == session["generation"],
              "H request changed its original domain/session/generation")
        native = one(db, "SELECT raw_hex,operation,session_id,status,previous_revision,revision "
                     "FROM gogoke_v37_h_operation WHERE domain_id=? AND request_id=?", (domain, req["requestId"]))
        check(native["raw_hex"].lower() == entry["rawFrame"].encode().hex() and
              native["operation"] == req["operation"] and native["session_id"] == sid and
              native["status"] == "APPLIED" and
              str(native["revision"]) == entry["receipt"]["revision"],
              "Original H operation bytes/status/revision differ from the request journal")
    open_entry = next(item for item in records if item["request"]["requestId"] == session["openRequestId"])
    stop_entry = next(item for item in records if item["request"]["requestId"] == session["stopRequestId"])
    release_entry = next(item for item in records if item["request"]["requestId"] == session["releaseRequestId"])
    check(open_entry["request"]["operation"] == "open" and
          open_entry["request"]["payload"]["seatId"] == case["seatId"] and
          open_entry["request"]["payload"]["repositoryId"] == case["repositoryId"] and
          open_entry["request"]["payload"]["worktreeId"] == case["worktreeId"] and
          stop_entry["receipt"]["result"]["stopFact"] == session["stopFactId"] and session["stopFactId"],
          "Original open or stop receipt does not bind the exact test F tree/StopFact")
    registration = one(db, "SELECT domain_id,seat_id,purpose,COALESCE(side_id,'') AS side_id "
                       "FROM v37_ledger_session WHERE domain_id=? AND session_id=?", (domain, sid))
    check(registration["seat_id"] == case["seatId"] and registration["purpose"] == "WORK" and
          registration["side_id"] == "", "Original A session registration differs")
    claim = one(db, "SELECT generation,state,revision,process_operation_id,stop_fact_id "
                  "FROM gogoke_v37_h_claim WHERE domain_id=? AND session_id=?", (domain, sid))
    binding = one(db, "SELECT seat_id,seat_incarnation,generation FROM gogoke_v37_h_seat_binding "
                    "WHERE domain_id=? AND session_id=?", (domain, sid))
    episode = one(db, "SELECT request_id,generation,process_operation_id,phase,stop_fact_id,stop_request_id,"
                     "raw_hex,seat_id,seat_incarnation,instance_id FROM gogoke_v37_h_process_episode "
                     "WHERE domain_id=? AND session_id=?", (domain, sid))
    custody = one(db, "SELECT domain_id,generation,state,stop_proof_hash FROM gogoke_coordination_process_custody "
                    "WHERE operation_id=?", (episode["process_operation_id"],))
    check(claim["generation"] == binding["generation"] == episode["generation"] == session["generation"] and
          claim["state"] == "RELEASED" and claim["process_operation_id"] == episode["process_operation_id"] and
          claim["stop_fact_id"] == episode["stop_fact_id"] == custody["stop_proof_hash"] == session["stopFactId"] and
          binding["seat_id"] == episode["seat_id"] == case["seatId"] and
          binding["seat_incarnation"] == episode["seat_incarnation"] and
          episode["instance_id"] == case["instanceId"] and episode["phase"] == custody["state"] == "STOPPED" and
          episode["stop_request_id"] == session["stopRequestId"] and
          bytes.fromhex(episode["raw_hex"]).decode() == open_entry["rawFrame"] and
          custody["domain_id"] == domain and custody["generation"] == session["generation"],
          "Original H stop/claim/episode/seat binding/ProcessCustody StopFact chain differs")
    opens = []
    for row in rows(db, "SELECT domain_id,session_id,raw_hex FROM gogoke_v37_h_operation WHERE operation='open'"):
        request = json.loads(bytes.fromhex(row["raw_hex"]))
        if request.get("payload", {}).get("worktreeId") == case["worktreeId"]:
            opens.append((row["domain_id"], row["session_id"]))
    check(opens == [(domain, sid)], "The configured exclusive F tree has another original H open episode")
    return {"sessionId": sid, "generation": session["generation"], "seatId": case["seatId"],
            "instanceId": case["instanceId"], "processOperationId": episode["process_operation_id"],
            "stopFactId": episode["stop_fact_id"], "claimState": claim["state"],
            "episodePhase": episode["phase"], "custodyState": custody["state"],
            "hRequestIds": requests, "hOpenCountForTree": len(opens)}


def worktree_facts(db, journal, case, phase):
    row = one(db, "SELECT worktree_id,path_id,repository_id,domain_id,seat_id,seat_incarnation,"
                "seat_generation,instance_id,worktree_path,worktree_identity,git_pointer_hash,"
                "git_pointer_identity,common_identity,baseline_commit,state,revision "
                "FROM gogoke_v37_worktrees WHERE worktree_id=?", (case["worktreeId"],))
    check((row["repository_id"], row["domain_id"], row["seat_id"], row["instance_id"], row["state"]) ==
          (case["repositoryId"], case["domainId"], case["seatId"], case["instanceId"], "REGISTERED"),
          "Original F registered tree identity differs")
    root = Path(case["stateRoot"]).resolve(strict=True)
    worktree_path = Path(row["worktree_path"]).resolve(strict=False)
    worktree_parent = (root / "v37-worktrees").resolve(strict=True)
    check(worktree_path != worktree_parent and worktree_parent in worktree_path.parents,
          "Original F physical path is outside the product-owned worktree root")
    exists = worktree_path.is_dir() and not worktree_path.is_symlink()
    stop = session_facts(db, journal, case)
    lifecycle = rows(db, "SELECT state,revision,COALESCE(stop_fact_id,'') AS stop_fact_id "
                     "FROM gogoke_v37_worktree_lifecycle WHERE worktree_id=?", (case["worktreeId"],))
    cleanup_rows = rows(db, "SELECT request_id,request_hash,worktree_id,operation,phase "
                        "FROM gogoke_v37_worktree_lifecycle_ops WHERE worktree_id=? AND operation='CLEANUP' "
                        "ORDER BY rowid", (case["worktreeId"],))
    denied = operation(journal, case["preStopCleanupRequestId"])
    request = denied["request"]
    check(request["family"] == "K-WORKTREE" and request["operation"] == "cleanup" and
          request["targetId"] == case["worktreeId"] and request["domainId"] == case["domainId"] and
          request["expectedRevision"] == str(case["graphRevision"]) and request["payload"] == {} and
          denied["receipt"]["status"] == "DENIED" and
          denied["receipt"]["revision"] == str(case["graphRevision"]) and not cleanup_rows,
          "Original User cleanup attempt was not denied without writing F cleanup intent")
    if phase == "stopped":
        check(exists and not lifecycle, "Stopped-only readback must retain the exact tree without a cleanup lifecycle row")
        return {"worktreeId": case["worktreeId"], "repositoryId": row["repository_id"],
                "domainId": row["domain_id"], "seatId": row["seat_id"], "instanceId": row["instance_id"],
                "worktreeIdentity": row["worktree_identity"], "gitPointerHash": row["git_pointer_hash"],
                "gitPointerIdentity": row["git_pointer_identity"], "commonIdentity": row["common_identity"],
                "baselineCommit": row["baseline_commit"], "pathDevice": str(worktree_path.stat().st_dev),
                "pathInode": str(worktree_path.stat().st_ino), "exists": True,
                "lifecycleState": "REGISTERED", "lifecycleRevision": case["graphRevision"],
                "stopFactId": stop["stopFactId"], "session": stop}
    cleanup = operation(journal, case["cleanupRequestId"])
    raw = cleanup["rawFrame"].encode("utf-8")
    lifecycle_row = one(db, "SELECT state,revision,COALESCE(stop_fact_id,'') AS stop_fact_id "
                        "FROM gogoke_v37_worktree_lifecycle WHERE worktree_id=?", (case["worktreeId"],))
    applied = one(db, "SELECT request_hash,worktree_id,operation,phase FROM gogoke_v37_worktree_lifecycle_ops "
                     "WHERE request_id=?", (case["cleanupRequestId"],))
    graph = case["graphRevision"]
    check(not exists and cleanup["receipt"]["status"] == "APPLIED" and
          cleanup["receipt"]["result"].get("worktreeId") == case["worktreeId"] and
          cleanup["receipt"]["result"].get("stopFactId") == stop["stopFactId"] and
          lifecycle_row["state"] == "CLEANED" and lifecycle_row["revision"] == int(graph) + 1 and
          lifecycle_row["stop_fact_id"] == stop["stopFactId"] and
          applied == {"request_hash": digest(raw), "worktree_id": case["worktreeId"],
                      "operation": "CLEANUP", "phase": "APPLIED"} and
          cleanup["receipt"]["revision"] == str(lifecycle_row["revision"]),
          "Original F cleanup request/hash/APPLIED lifecycle/physical removal/StopFact chain differs")
    return {"worktreeId": case["worktreeId"], "repositoryId": row["repository_id"],
            "domainId": row["domain_id"], "seatId": row["seat_id"], "instanceId": row["instance_id"],
            "worktreeIdentity": row["worktree_identity"], "gitPointerHash": row["git_pointer_hash"],
            "gitPointerIdentity": row["git_pointer_identity"], "commonIdentity": row["common_identity"],
            "baselineCommit": row["baseline_commit"], "exists": False,
            "lifecycleState": lifecycle_row["state"], "lifecycleRevision": lifecycle_row["revision"],
            "stopFactId": lifecycle_row["stop_fact_id"], "cleanupRequestId": case["cleanupRequestId"],
            "session": stop}


def main():
    check(len(sys.argv) == 5 and sys.argv[4] in ("stopped", "final"),
          "Expected state root, output, original M2 journal and stopped|final phase")
    root, output, journal_file = Path(sys.argv[1]).resolve(strict=True), Path(sys.argv[2]).resolve(), Path(sys.argv[3]).resolve(strict=True)
    check(not output.exists() and not output.is_relative_to(root),
          "Fresh private readback must stay outside candidate state")
    journal = json.loads(journal_file.read_text(encoding="utf-8-sig"))
    case = journal.get("stopWorktree")
    check(case and case["schema"] == "gogoke.37.m2-stop-worktree.v1" and
          case["acceptance"] is False and case["sourceCommit"] == journal["sourceCommit"] and
          Path(case["stateRoot"]).resolve(strict=True) == root, "Original stop-worktree case/root required")
    evidence = Path(case["evidenceDirectory"]).resolve(strict=True)
    check(output.parent == journal_file.parent == evidence and Path(journal_file.parent).resolve(strict=True) == evidence,
          "Original journal/output must remain in the case evidence directory")
    launches, closes = journal.get("launches", []), journal.get("closes", [])
    check(launches and closes, "Actual installed candidate launch and normal close are required")
    launch, close, endpoint = launches[-1], closes[-1], journal.get("currentEndpoint", {})
    bootstrap = launch.get("bootstrap", {})
    check(launch.get("pid") == close.get("pid") == endpoint.get("pid") and close.get("exitCode") == 0 and
          close.get("forceKill") is False and launch.get("sourceCommit") == case["sourceCommit"] and
          bootstrap.get("version") == case["candidateVersion"] and launch.get("setId") == bootstrap.get("setId") and
          launch.get("generationId") == bootstrap.get("generationId"),
          "Latest actual installed candidate lacks its matching normal-close receipt")
    installed = case.get("candidateInstalledSha256")
    check(isinstance(installed, dict) and installed and all(isinstance(value, str) and len(value) == 64
          and all(ch in "0123456789abcdef" for ch in value) for value in installed.values()),
          "Actual candidate installed byte pins are required")
    st, dbst = root.stat(), (root / "state.sqlite").stat()
    root_identity = {"device": str(st.st_dev), "inode": str(st.st_ino),
                     "databaseDevice": str(dbst.st_dev), "databaseInode": str(dbst.st_ino)}
    candidate = {"sourceCommit": launch["sourceCommit"], "version": bootstrap["version"],
                 "setId": launch["setId"], "generationId": launch["generationId"]}
    if sys.argv[4] == "final":
        ref = case.get("baseline")
        check(ref and Path(ref["file"]).name == ref["file"], "Original stopped-only baseline reference is required")
        base_path = evidence / ref["file"]
        check(digest(base_path.read_bytes()) == ref["sha256"], "Original stopped-only baseline bytes changed")
        baseline = json.loads(base_path.read_text(encoding="utf-8-sig"))
        check(baseline.get("phase") == "stopped" and baseline.get("caseId") == journal["caseId"] and
              baseline.get("sourceCommit") == case["sourceCommit"] and baseline.get("rootIdentity") == root_identity and
              baseline.get("candidateIdentity") == candidate and baseline.get("candidateInstalledSha256") == installed and
              baseline.get("worktree", {}).get("worktreeIdentity") is not None,
              "Final candidate/root/F identity differs from the exact stopped baseline")
    database, wal, shm = root / "state.sqlite", root / "state.sqlite-wal", root / "state.sqlite-shm"
    check(database.is_file() and (not wal.exists() or wal.stat().st_size == 0),
          "Normal close and empty/absent WAL required")
    def files():
        return {item.name: {"length": item.stat().st_size, "sha256": digest(item.read_bytes())}
                for item in (database, wal, shm) if item.exists()}
    before = files()
    with sqlite3.connect(database.as_uri() + "?mode=ro&immutable=1", uri=True) as db:
        db.row_factory = sqlite3.Row
        db.execute("PRAGMA query_only=ON")
        worktree = worktree_facts(db, journal, case, sys.argv[4])
    after = files()
    check(before == after and (not wal.exists() or wal.stat().st_size == 0),
          "Immutable measurement changed candidate database/WAL/SHM bytes")
    if sys.argv[4] == "final":
        check(worktree.get("stopFactId") == case["session"]["stopFactId"] and
              worktree.get("lifecycleState") == "CLEANED", "Final worktree is not cleaned by its original StopFact")
    proof = {"schema": "gogoke.37.private-m2-stop-worktree-readback.v1", "phase": sys.argv[4],
             "caseId": journal["caseId"], "sourceCommit": case["sourceCommit"], "domainId": case["domainId"],
             "stateRoot": str(root), "evidenceDirectory": str(evidence),
             "readerSha256": case["readerSha256"], "candidateIdentity": candidate,
             "candidateInstalledSha256": installed, "rootIdentity": root_identity,
             "launch": {"pid": launch["pid"], "sourceCommit": launch["sourceCommit"]},
             "normalClose": {"pid": close["pid"], "exitCode": close["exitCode"], "forceKill": close["forceKill"]},
             "databaseSha256": digest(database.read_bytes()), "filesBefore": before, "filesAfter": after,
             "measurementPreservedDatabaseBytes": before == after, "acceptance": False,
             "stopFactId": worktree["stopFactId"], "stopOnlyEvidence": sys.argv[4] == "stopped",
             "cleanupApplied": sys.argv[4] == "final", "directCaseEvidence": sys.argv[4] == "final",
             "worktree": worktree,
             "notRun": case["notRun"]}
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(proof, stream, ensure_ascii=False, indent=2); stream.write("\n")
    check(proof["measurementPreservedDatabaseBytes"], "Readback must preserve original state bytes")
    print(json.dumps({"phase": sys.argv[4], "directCaseEvidence": proof["directCaseEvidence"], "acceptance": False}))


if __name__ == "__main__":
    main()
