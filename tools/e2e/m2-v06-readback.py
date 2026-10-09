"""Read V06's original seat/H facts after a normal close, without opening a writer."""
import hashlib
import json
import sqlite3
import sys
from pathlib import Path


def check(ok, reason):
    if not ok:
        raise RuntimeError(reason)


def one(db, sql, args=()):
    rows = [dict(row) for row in db.execute(sql, args)]
    check(len(rows) == 1, f"One original row required, found {len(rows)}")
    return rows[0]


def rows(db, sql, args=()):
    return [dict(row) for row in db.execute(sql, args)]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def original_user(db, journal, request_id, status):
    entries = [row for row in journal["operations"]
               if row.get("request", {}).get("requestId") == request_id]
    check(len(entries) == 1, "Original User request is missing or duplicated")
    entry = entries[0]
    request, receipt = entry["request"], entry["receipt"]
    check(entry["rawFrame"] == json.dumps(request, ensure_ascii=False, separators=(",", ":")) and
          receipt["schema"] == request["schema"] and receipt["family"] == request["family"] == "K-SEAT" and
          receipt["operation"] == request["operation"] and receipt["requestId"] == request_id and
          receipt["targetId"] == request["targetId"] and
          receipt["status"] == status and json.loads(entry["rawReceipt"]) == receipt,
          "Original User K-SEAT request/receipt differs")
    durable = rows(db, "SELECT o.seat_id,o.layer,o.parent_seat_id,o.revision,s.settings_json "
                   "FROM gogoke_v37_seat_operations o LEFT JOIN gogoke_v37_seat_operation_snapshots s "
                   "USING(domain_id,request_id) WHERE o.domain_id=? AND o.request_id=?",
                   (request["domainId"], request_id))
    if request["operation"] in ("set-orchestration-bounds", "create-from-template"):
        check((len(durable) == 1) == (status == "APPLIED"),
              "Original User mutation count differs from its native receipt")
        if status == "APPLIED":
            check(durable[0]["seat_id"] == request["targetId"] and
                  durable[0]["revision"] == int(receipt["revision"]) and
                  json.loads(durable[0]["settings_json"]) == receipt["result"]["settings"],
                  "Original User seat operation snapshot differs from its receipt")
    if request["operation"] == "state-card":
        stored = one(db, "SELECT request_bytes,receipt_bytes FROM v37_ledger_receipt "
                     "WHERE family='K-SEAT' AND domain_id=? AND request_id=?",
                     (request["domainId"], request_id))
        check(bytes(stored["request_bytes"]).decode() == entry["rawFrame"] and
              bytes(stored["receipt_bytes"]).decode() == entry["rawReceipt"],
              "Original state-card ledger bytes differ")
    return entry


def original_model(db, journal, case, attempt):
    fixture = case["fixture"]
    sent = [row for row in journal["operations"] if row.get("request", {}).get("requestId") == attempt["sendRequestId"]]
    check(len(sent) == 1 and sent[0]["request"]["family"] == "K-SESSION" and
          sent[0]["request"]["operation"] == "send" and
          sent[0]["request"]["domainId"] == case["domainId"] and
          sent[0]["request"]["targetId"] == attempt["sessionId"] and
          sent[0]["request"]["payload"]["body"] == attempt["prompt"],
          "Original H send was not the requested model tool turn")
    stored_send = one(db, "SELECT request_bytes,receipt_bytes FROM v37_ledger_receipt "
                      "WHERE family='K-SESSION' AND domain_id=? AND request_id=?",
                      (case["domainId"], attempt["sendRequestId"]))
    check(bytes(stored_send["request_bytes"]).decode() == sent[0]["rawFrame"] and
          bytes(stored_send["receipt_bytes"]).decode() == sent[0]["rawReceipt"],
          "Original H send ledger bytes differ")
    binding = one(db, "SELECT seat_id,selected_instance_id FROM gogoke_v37_native_selection "
                  "WHERE domain_id=? AND session_id=?", (case["domainId"], attempt["sessionId"]))
    check(binding["seat_id"] == fixture["parentSeatId"] and
          binding["selected_instance_id"] == fixture["instanceId"],
          "Original H session was not bound to the exact USER parent and instance")
    claim = one(db, "SELECT process_operation_id,thread_id,generation FROM gogoke_v37_h_claim "
                "WHERE domain_id=? AND session_id=?", (case["domainId"], attempt["sessionId"]))
    stdin = one(db, "SELECT phase,receipt_status,request_hex,receipt_hex,session_id,generation,"
                "process_operation_id,operation FROM gogoke_v37_h_stdin_journal "
                "WHERE domain_id=? AND request_id=?", (case["domainId"], attempt["sendRequestId"]))
    check(stdin["phase"] == "RECEIPTED" and stdin["receipt_status"] == "APPLIED" and
          stdin["session_id"] == attempt["sessionId"] and stdin["operation"] == "send" and
          str(stdin["generation"]) == str(claim["generation"]) and
          stdin["process_operation_id"] == claim["process_operation_id"] and
          bytes.fromhex(stdin["request_hex"]).decode() == sent[0]["rawFrame"],
          "Original H stdin send was not receipted for this physical session")
    send_ack = json.loads(bytes.fromhex(stdin["receipt_hex"]))
    check(send_ack == sent[0]["receipt"] == json.loads(sent[0]["rawReceipt"]) and
          send_ack["status"] == "APPLIED" and
          send_ack["result"]["createdTurn"] is True and
          send_ack["result"]["turnId"] == attempt["turnId"],
          "Original H stdin receipt did not authorize this exact turn")
    episode = one(db, "SELECT phase,stop_fact_id FROM gogoke_v37_h_process_episode "
                  "WHERE domain_id=? AND session_id=? AND process_operation_id=?",
                  (case["domainId"], attempt["sessionId"], claim["process_operation_id"]))
    custody = one(db, "SELECT state,stop_proof_hash FROM gogoke_coordination_process_custody "
                  "WHERE operation_id=?", (claim["process_operation_id"],))
    check(episode["phase"] == custody["state"] == "STOPPED" and episode["stop_fact_id"] and
          episode["stop_fact_id"] == custody["stop_proof_hash"],
          "Original H physical episode lacks its StopFact before readback")
    source = rows(db, "SELECT raw_bytes FROM v37_ledger_raw_source WHERE domain_id=? AND session_id=? "
                  "AND operation_id=? ORDER BY rowid",
                  (case["domainId"], attempt["sessionId"], claim["process_operation_id"]))
    calls, completed = [], []
    for row in source:
        frame = json.loads(bytes(row["raw_bytes"]))
        params = frame.get("params", {})
        if (frame.get("method") == "item/tool/call" and params.get("tool") == "gogoke_seat" and
                params.get("threadId") == claim["thread_id"] and
                params.get("turnId") == attempt["turnId"] and
                params.get("arguments") == attempt["expectedArguments"]):
            calls.append(frame)
        if (frame.get("method") == "turn/completed" and params.get("threadId") == claim["thread_id"] and
                params.get("turn", {}).get("id") == attempt["turnId"] and
                params.get("turn", {}).get("status") == "completed"):
            completed.append(frame)
    check(len(calls) == len(completed) == 1, "Original A model call or completed turn is missing")
    matching = []
    for row in rows(db, "SELECT command_hex,phase,step_id FROM gogoke_v37_rpc_steps WHERE domain_id=? "
                    "AND session_id=? AND process_operation_id=?",
                    (case["domainId"], attempt["sessionId"], claim["process_operation_id"])):
        command = json.loads(bytes.fromhex(row["command_hex"]))
        if "method" not in command and command.get("id") == calls[0]["id"]:
            matching.append((row, command))
    check(len(matching) == 1 and matching[0][0]["phase"] in ("WRITTEN", "OBSERVED"),
          "Original H tool response is absent")
    row, command = matching[0]
    content = command.get("result", {}).get("contentItems", [])
    check(len(content) == 1 and content[0].get("type") == "inputText", "Original H tool response shape differs")
    receipt = json.loads(content[0]["text"])
    check(receipt["schema"] == "gogoke.37.operations.v1" and receipt["family"] == "K-SEAT" and
          receipt["operation"] == "create-from-template" and receipt["requestId"] == row["step_id"] and
          receipt["targetId"] == attempt["targetId"] and
          receipt["status"] == attempt["expectedStatus"] and
          command["result"]["success"] is (attempt["expectedStatus"] == "APPLIED"),
          "Original H-written K-SEAT model receipt differs")
    durable = rows(db, "SELECT * FROM gogoke_v37_seat_operations WHERE domain_id=? AND request_id=?",
                   (case["domainId"], row["step_id"]))
    check((len(durable) == 1) == (attempt["expectedStatus"] == "APPLIED"),
          "Native child producer operation count differs from original H receipt")
    if attempt["expectedStatus"] == "APPLIED":
        check(durable[0]["seat_id"] == attempt["targetId"] and durable[0]["layer"] == "LEAD" and
              durable[0]["parent_seat_id"] == fixture["parentSeatId"],
              "Original child producer row belongs to another seat or parent")
    return True


def main():
    check(len(sys.argv) == 5 and sys.argv[4] in ("baseline", "final"),
          "Expected STATE_ROOT OUTPUT JOURNAL baseline|final")
    root = Path(sys.argv[1]).resolve(strict=True)
    output = Path(sys.argv[2]).resolve(strict=False)
    journal_path = Path(sys.argv[3]).resolve(strict=True)
    phase = sys.argv[4]
    check(not output.exists() and not output.is_relative_to(root), "Fresh private output must stay outside state root")
    journal = json.loads(journal_path.read_text(encoding="utf-8-sig"))
    case = journal.get("v06NativeUser")
    check(case and case["schema"] == "gogoke.37.m2-v06-native-user.v1" and case["acceptance"] is False and
          case["sourceCommit"] == journal["sourceCommit"] and
          digest(Path(__file__).read_bytes()) == case["readerSha256"] and
          root == Path(case["stateRoot"]).resolve(strict=True) and
          output.parent == journal_path.parent == Path(case["evidenceDirectory"]).resolve(strict=True),
          "Original V06 case, reader bytes or private paths differ")
    launch, close = journal["launches"][-1], journal["closes"][-1]
    check(launch["pid"] == close["pid"] and close["exitCode"] == 0 and close["forceKill"] is False and
          launch["sourceCommit"] == case["sourceCommit"] and
          launch["bootstrap"]["version"] == case["candidateVersion"],
          "Actual installed candidate lacks a matched normal-close receipt")
    check(all(isinstance(value, str) and len(value) == 64 and
              all(char in "0123456789abcdef" for char in value)
              for value in case["candidateInstalledSha256"].values()),
          "Original installed candidate byte pins differ")
    database = root / "state.sqlite"
    wal, shm = Path(str(database) + "-wal"), Path(str(database) + "-shm")
    check(database.is_file() and (not wal.exists() or wal.stat().st_size == 0),
          "Normal close requires an empty or absent WAL")
    files = lambda: {p.name: {"size": p.stat().st_size, "sha256": digest(p.read_bytes())}
                     for p in (database, wal, shm) if p.exists()}
    before = files()
    fixture = case["fixture"]
    with sqlite3.connect(database.as_uri() + "?mode=ro&immutable=1", uri=True) as db:
        db.row_factory = sqlite3.Row
        db.execute("PRAGMA query_only=ON")
        parent = one(db, "SELECT * FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",
                     (case["domainId"], fixture["parentSeatId"]))
        settings = one(db, "SELECT settings_json FROM gogoke_v37_seat_settings WHERE domain_id=? AND seat_id=?",
                       (case["domainId"], fixture["parentSeatId"]))
        template = one(db, "SELECT settings_json,revision FROM gogoke_v37_seat_templates "
                       "WHERE domain_id=? AND template_id=?", (case["domainId"], fixture["templateId"]))
        child_settings = json.loads(template["settings_json"])
        template_matches = (child_settings.get("model") == fixture["model"] and
                            child_settings.get("effort", child_settings.get("reasoningEffort")) == fixture["effort"] and
                            child_settings.get("permissionTier") == fixture["permissionTier"])
        project_cap = one(db, "SELECT parallel_cap FROM gogoke_v37_seat_project_caps WHERE domain_id=?",
                          (case["domainId"],))["parallel_cap"]
        machine = one(db, "SELECT source,observed_parallelism,machine_limit FROM gogoke_v37_seat_host_resources "
                      "WHERE singleton=1")
        check(machine["source"] == "STD_AVAILABLE_PARALLELISM" and
              machine["observed_parallelism"] == machine["machine_limit"] > 0,
              "Original host machine limit is not source-backed")
        children = rows(db, "SELECT * FROM gogoke_v37_seats WHERE domain_id=? AND parent_seat_id=? "
                        "AND layer='LEAD' AND state!='RECLAIMED'",
                        (case["domainId"], fixture["parentSeatId"]))
        targets = [rows(db, "SELECT * FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",
                        (case["domainId"], fixture[key]))
                   for key in ("directSeatId", "childSeatId", "deniedChildSeatId")]
        if phase == "baseline":
            check(parent["layer"] == "USER" and parent["state"] == "IDLE" and
                  parent["instance_id"] == fixture["instanceId"] and not children and
                  not any(targets) and template_matches and project_cap > 0,
                  "Baseline exact parent/template/capacity prerequisites are missing")
            result = {"targetsAbsent": True, "templateMatchesScope": True,
                      "deniedTargetAbsent": True, "originalModelReceipts": False,
                      "originalUserReceipts": False}
        else:
            base_ref = case["baseline"]
            base_path = Path(case["evidenceDirectory"]) / base_ref["file"]
            check(digest(base_path.read_bytes()) == base_ref["sha256"], "Original baseline reference changed")
            baseline = json.loads(base_path.read_text(encoding="utf-8-sig"))
            scope = {"instanceIds": [fixture["instanceId"]], "models": [fixture["model"]],
                     "reasoningEfforts": [fixture["effort"]], "maxPermissionTier": fixture["permissionTier"],
                     "maxConcurrent": 1}
            check(baseline["projectCap"] == project_cap and baseline["templateSha256"] ==
                  digest(template["settings_json"].encode()) and
                  json.loads(settings["settings_json"]).get("orchestrationScope") == scope and
                  len(children) == 1 and children[0]["seat_id"] == fixture["childSeatId"] and
                  children[0]["layer"] == "LEAD" and children[0]["parent_seat_id"] == fixture["parentSeatId"] and
                  len(targets[0]) == len(targets[1]) == 1 and not targets[2] and
                  targets[0][0]["layer"] == "USER" and targets[0][0]["parent_seat_id"] is None and
                  targets[1][0]["layer"] == "LEAD",
                  "Original V06 direct/child count or Owner project cap changed")
            copied_template = dict(child_settings)
            copied_template.pop("reasoningEffort", None)
            copied_template["effort"] = fixture["effort"]
            for key in ("directSeatId", "childSeatId"):
                copied = one(db, "SELECT template_id,settings_json FROM gogoke_v37_seat_settings "
                             "WHERE domain_id=? AND seat_id=?", (case["domainId"], fixture[key]))
                check(copied["template_id"] == fixture["templateId"] and
                      json.loads(copied["settings_json"]) == copied_template,
                      "Original template copy settings differ for direct or child seat")
            bounds = case["operations"].get("set-orchestration-bounds", [])
            direct = case["operations"].get("create-from-template", [])
            check(len(bounds) == 2 and len(direct) == 1, "Original User operation count differs")
            original_user(db, journal, bounds[0], "DENIED")
            original_user(db, journal, bounds[1], "APPLIED")
            original_user(db, journal, direct[0], "APPLIED")
            for request_id in case["operations"].get("state-card", []):
                entry = next((row for row in journal["operations"]
                              if row.get("request", {}).get("requestId") == request_id), None)
                check(entry is not None and entry["receipt"]["status"] in ("APPLIED", "STALE"),
                      "Original state-card request is missing")
                if entry["receipt"]["status"] == "APPLIED":
                    original_user(db, journal, request_id, "APPLIED")
            check(len(case["modelAttempts"]) == 2 and
                  [item["expectedStatus"] for item in case["modelAttempts"]] == ["APPLIED", "DENIED"] and
                  all(original_model(db, journal, case, item) for item in case["modelAttempts"]),
                  "Original model child capacity receipts are incomplete")
            result = {"targetsAbsent": False, "templateMatchesScope": template_matches,
                      "deniedTargetAbsent": True, "originalModelReceipts": True,
                      "originalUserReceipts": True}
        proof = {"schema": "gogoke.37.private-m2-v06-readback.v1", "phase": phase,
                 "caseId": journal["caseId"], "sourceCommit": case["sourceCommit"],
                 "readerSha256": case["readerSha256"], "acceptance": False,
                 "stateRoot": str(root), "databaseSha256": digest(database.read_bytes()),
                 "rootIdentity": {"device": str(root.stat().st_dev), "inode": str(root.stat().st_ino),
                                  "databaseDevice": str(database.stat().st_dev),
                                  "databaseInode": str(database.stat().st_ino)},
                 "candidateIdentity": {"setId": launch["setId"],
                                       "generationId": launch["generationId"],
                                       "sourceCommit": launch["sourceCommit"],
                                       "version": launch["bootstrap"]["version"]},
                 "candidateInstalledSha256": case["candidateInstalledSha256"],
                 "measurementPreservedDatabaseBytes": before == files(),
                 "normalClose": {"pid": close["pid"], "exitCode": close["exitCode"],
                                 "forceKill": close["forceKill"]},
                 "parentRevision": str(parent["revision"]), "parentGeneration": str(parent["generation"]),
                 "projectCap": project_cap, "hostLimit": machine["machine_limit"],
                 "childCount": len(children), "templateSha256": digest(template["settings_json"].encode()),
                 "templateRevision": template["revision"], **result}
    check(before == files() and (not wal.exists() or wal.stat().st_size == 0),
          "Readback changed the original database or WAL")
    output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"phase": phase, "childCount": proof["childCount"], "acceptance": False}))


if __name__ == "__main__":
    main()
