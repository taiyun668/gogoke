"""Read original M2 seat-management facts after normal product close.

Signed Python: STATE_ROOT OUTPUT JOURNAL baseline|final. Opens only candidate
state.sqlite with mode=ro&immutable=1; emits IDs and hashes, never credentials.
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


def operation(journal, request_id, db):
    found = [entry for entry in journal["operations"]
             if entry.get("request", {}).get("requestId") == request_id]
    check(len(found) == 1, "One original operation required")
    entry = found[0]
    stored = rows(db, "SELECT request_bytes,receipt_bytes FROM v37_ledger_receipt "
                  "WHERE family=? AND domain_id=? AND request_id=?",
                  (entry["request"]["family"], entry["request"]["domainId"], request_id))
    if entry["request"]["family"] == "K-SESSION" or entry["request"]["operation"] == "state-card":
        check(len(stored) == 1, "Original H send/state-card ledger receipt is missing")
    if stored:
        check(len(stored) == 1 and bytes(stored[0]["request_bytes"]).decode() == entry["rawFrame"] and
              bytes(stored[0]["receipt_bytes"]).decode() == entry["rawReceipt"],
              "Original request/receipt bytes differ from the immutable ledger")
    return entry


def seat_operation(journal, request_id, db, parts):
    entry = operation(journal, request_id, db)
    preimage = bytearray()
    for part in parts:
        data = part.encode("utf-8")
        preimage.extend(len(data).to_bytes(8, "big")); preimage.extend(data)
    raw = entry["rawFrame"].encode("utf-8")
    preimage.extend(len(raw).to_bytes(8, "big")); preimage.extend(raw)
    saved = one(db, "SELECT o.fingerprint,o.seat_id,o.incarnation,o.layer,COALESCE(o.parent_seat_id,'') AS parent_seat_id,"
                "o.kind,o.state,o.revision,o.generation,COALESCE(s.template_id,'') AS template_id,"
                "COALESCE(s.settings_json,'') AS settings_json FROM gogoke_v37_seat_operations o "
                "LEFT JOIN gogoke_v37_seat_operation_snapshots s USING(domain_id,request_id) "
                "WHERE o.domain_id=? AND o.request_id=?",
                (entry["request"]["domainId"], request_id))
    check(saved["fingerprint"] == "sha256:" + hashlib.sha256(preimage).hexdigest() and
          saved["seat_id"] == entry["request"]["targetId"],
          "Original request bytes do not match the producer's durable CAS operation")
    return entry, saved


def verify_lead_seat(db, journal, lead, baseline):
    if not lead:
        return False
    before = baseline["leadSeat"]
    seat = one(db, "SELECT * FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",
               (lead["domainId"], lead["seatId"]))
    check(seat["incarnation"] == lead["incarnation"] and seat["parent_seat_id"] == lead["parentSeatId"] and
          seat["layer"] == "LEAD" and seat["state"] == "RECLAIMED" and
          seat["generation"] == int(before["generation"]) + 2 and seat["revision"] == int(before["revision"]) + 2,
          "Original producer-bound LEAD incarnation/parent/generation/revision changed unexpectedly")
    settings = one(db, "SELECT settings_json FROM gogoke_v37_seat_settings WHERE domain_id=? AND seat_id=?",
                   (lead["domainId"], lead["seatId"]))
    tune_request = next(item["request"] for item in journal["operations"]
                        if item["request"]["requestId"] == lead["tuneRequestId"])
    value_json = json.dumps(tune_request["payload"]["value"], ensure_ascii=False, separators=(",", ":"))
    tuned, tune_row = seat_operation(journal, lead["tuneRequestId"], db,
        ["tune", lead["domainId"], lead["seatId"], str(before["generation"]), str(before["revision"]),
         lead["setting"], value_json, "", "", ""])
    card = operation(journal, lead["cardRequestId"], db)
    reclaim, reclaim_row = seat_operation(journal, lead["reclaimRequestId"], db,
        ["reclaim", lead["domainId"], lead["seatId"], str(int(before["generation"]) + 1),
         str(int(before["revision"]) + 1), "RECLAIMED", "", "", ""])
    check(tuned["receipt"]["status"] == card["receipt"]["status"] == reclaim["receipt"]["status"] == "APPLIED" and
          tuned["receipt"]["result"]["settings"][lead["setting"]] == lead["value"] and
          card["receipt"]["result"]["settings"][lead["setting"]] == lead["value"] and
          json.loads(settings["settings_json"])[lead["setting"]] == lead["value"] and
          reclaim["receipt"]["result"]["state"] == "RECLAIMED" and
          tune_row["parent_seat_id"] == lead["parentSeatId"] and tune_row["layer"] == "LEAD" and
          json.loads(tune_row["settings_json"])[lead["setting"]] == lead["value"] and
          reclaim_row["incarnation"] == lead["incarnation"] and reclaim_row["state"] == "RECLAIMED" and
          json.loads(reclaim_row["settings_json"])[lead["setting"]] == lead["value"],
          "Original User tune/state-card/reclaim receipts do not bind the LEAD seat")
    return True


def verify_model_denial(db, journal, attempt, project, case):
    session_id = attempt["sessionId"]
    send = operation(journal, attempt["sendRequestId"], db)
    check(send["request"]["family"] == "K-SESSION" and send["request"]["operation"] == "send" and
          send["request"]["targetId"] == session_id and
          send["request"]["payload"]["body"] == attempt["prompt"],
          "Original H model prompt/send differs from the root callback")
    claim = one(db, "SELECT process_operation_id,thread_id,generation FROM gogoke_v37_h_claim "
                "WHERE domain_id=? AND session_id=?", (attempt["domainId"], session_id))
    check(claim["process_operation_id"] and claim["thread_id"], "Original root H/A process binding is absent")
    episode = one(db, "SELECT phase,stop_fact_id FROM gogoke_v37_h_process_episode "
                 "WHERE domain_id=? AND session_id=? AND process_operation_id=?",
                 (attempt["domainId"], session_id, claim["process_operation_id"]))
    custody = one(db, "SELECT state,stop_proof_hash FROM gogoke_coordination_process_custody WHERE operation_id=?",
                  (claim["process_operation_id"],))
    check(episode["phase"] == custody["state"] == "STOPPED" and episode["stop_fact_id"] and
          episode["stop_fact_id"] == custody["stop_proof_hash"],
          "Original model H process must be natively stopped before normal close/readback")
    incoming = rows(db, "SELECT raw_bytes FROM v37_ledger_raw_source WHERE domain_id=? AND session_id=? "
                    "AND operation_id=? ORDER BY rowid",
                    (attempt["domainId"], session_id, claim["process_operation_id"]))
    calls, completed = [], []
    expected_arguments = {"operation": "tune", "targetId": attempt["seatId"],
                          "expectedRevision": attempt["expectedRevision"],
                          "payload": {"setting": attempt["setting"], "value": attempt["value"]}}
    for source in incoming:
        frame = json.loads(bytes(source["raw_bytes"]))
        params = frame.get("params", {})
        args = params.get("arguments", {})
        if frame.get("method") == "item/tool/call" and params.get("tool") == "gogoke_seat" and \
                params.get("threadId") == claim["thread_id"] and args == expected_arguments:
            calls.append((frame, args))
        if frame.get("method") == "turn/completed" and params.get("threadId") == claim["thread_id"] and \
                params.get("turn", {}).get("id") == attempt["turnId"] and \
                params.get("turn", {}).get("status") == "completed":
            completed.append(frame)
    check(len(calls) == 1 and attempt["seatId"] == project["seatId"] and
          attempt["expectedRevision"] == project["createReceipt"]["revision"] and len(completed) == 1,
        "Original A tool call is missing or selected another seat")
    call = calls[0][0]
    replies = rows(db, "SELECT command_hex,phase,step_id FROM gogoke_v37_rpc_steps WHERE domain_id=? "
                   "AND session_id=? AND process_operation_id=?",
                   (attempt["domainId"], session_id, claim["process_operation_id"]))
    matching = []
    for row in replies:
        command = json.loads(bytes.fromhex(row["command_hex"]))
        if "method" not in command and command.get("id") == call["id"]:
            matching.append((row, command))
    check(len(matching) == 1 and matching[0][0]["phase"] in ("WRITTEN", "OBSERVED"),
          "Original H response for the A call is absent")
    content = matching[0][1].get("result", {}).get("contentItems", [])
    check(len(content) == 1 and content[0].get("type") == "inputText", "Original H tool response shape differs")
    receipt = json.loads(content[0]["text"])
    check(receipt["schema"] == "gogoke.37.operations.v1" and receipt["family"] == "K-SEAT" and
          receipt["operation"] == "tune" and receipt["requestId"] == matching[0][0].get("step_id") and
          receipt["targetId"] == project["seatId"] and receipt["status"] == "DENIED" and
          receipt["previousRevision"] == project["createReceipt"]["revision"] and
          receipt["revision"] == project["createReceipt"]["revision"] and
          matching[0][1]["result"]["success"] is False and
          not rows(db, "SELECT request_id FROM gogoke_v37_seat_operations WHERE domain_id=? AND request_id=?",
                   (attempt["domainId"], receipt["requestId"])) and
          not any(row.get("request", {}).get("requestId") == receipt["requestId"]
                  for row in journal["operations"]),
          "Original model K-SEAT denial differs from its H-written native receipt")


def main():
    check(len(sys.argv) == 5 and sys.argv[4] in ("baseline", "final"),
          "Expected candidate state root, output, journal and phase")
    root, output, journal_path, phase = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3]), sys.argv[4]
    root = root.resolve(strict=True)
    journal = json.loads(journal_path.read_text(encoding="utf-8-sig"))
    case = journal.get("seatManagement")
    check(case and case["schema"] == "gogoke.37.m2-seat-management.v1" and
          case["acceptance"] is False and case["sourceCommit"] == journal["sourceCommit"] and
          len(case["projects"]) == 2, "Original private seat-management journal required")
    projects = case["projects"]
    domains = [row["domainId"] for row in projects]
    check(len(set(domains)) == 2 and all(row["repositoryId"] == "gogokeSeatTestbed" for row in projects),
          "Two exact test domains/repository bindings required")
    baseline = None
    if phase == "final":
        reference = case.get("baseline")
        check(reference and Path(reference["file"]).name == reference["file"], "Original baseline artifact required")
        baseline_path = Path(journal["evidenceDirectory"]) / reference["file"]
        check(digest(baseline_path.read_bytes()) == reference["sha256"], "Original baseline artifact bytes changed")
        baseline = json.loads(baseline_path.read_text(encoding="utf-8-sig"))
        check(baseline["schema"] == "gogoke.37.private-m2-seat-management-readback.v1" and
              baseline["phase"] == "baseline" and baseline["caseId"] == journal["caseId"] and
              baseline["sourceCommit"] == case["sourceCommit"], "Baseline subject differs from final candidate")

    def inspect(db):
        facts = []
        for index, row in enumerate(projects):
            template = one(db, "SELECT settings_json,revision FROM gogoke_v37_seat_templates "
                           "WHERE domain_id=? AND template_id=?", (row["domainId"], row["templateId"]))
            seat_rows = rows(db, "SELECT * FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",
                             (row["domainId"], row["seatId"]))
            setting_rows = rows(db, "SELECT * FROM gogoke_v37_seat_settings WHERE domain_id=? AND seat_id=?",
                                (row["domainId"], row["seatId"]))
            if phase == "baseline":
                check(not seat_rows and not setting_rows, "Exact new target seat must be absent at baseline")
                facts.append({"domainId": row["domainId"], "templateId": row["templateId"],
                              "templateSettingsSha256": digest(template["settings_json"].encode()),
                              "templateRevision": template["revision"], "seatId": row["seatId"], "targetAbsent": True})
                continue
            check(len(seat_rows) == len(setting_rows) == 1, "Original copied seat/settings rows required")
            current, copied = seat_rows[0], setting_rows[0]
            check(current["layer"] == "USER" and current["state"] == "IDLE" and
                  copied["template_id"] == row["templateId"], "Original copy is not the exact idle USER seat")
            base = next(item for item in baseline["projects"] if item["domainId"] == row["domainId"])
            check(digest(template["settings_json"].encode()) == base["templateSettingsSha256"],
                  "Original source template changed during copy/edit")
            created = operation(journal, row["createRequestId"], db)
            expected = json.loads(json.dumps(created["receipt"]["result"]["settings"]))
            create_parts = ["create", row["domainId"], row["seatId"], row["templateId"],
                            "", "LONG", "USER", "", "", ""]
            created, create_row = seat_operation(journal, row["createRequestId"], db, create_parts)
            check(created["receipt"]["status"] == "APPLIED" and create_row["layer"] == "USER" and
                  create_row["kind"] == "LONG" and create_row["state"] == "IDLE" and
                  create_row["revision"] == create_row["generation"] == 1 and
                  create_row["template_id"] == row["templateId"] and
                  json.loads(create_row["settings_json"]) == expected and
                  digest(create_row["settings_json"].encode()) == base["templateSettingsSha256"],
                  "Original create receipt differs from durable copied-seat operation")
            if index == 0:
                expected[case["edit"]["setting"]] = case["edit"]["value"]
                tune_request = next(item["request"] for item in journal["operations"]
                                    if item["request"]["requestId"] == row["tuneRequestId"])
                value_json = json.dumps(tune_request["payload"]["value"], ensure_ascii=False, separators=(",", ":"))
                tuned, tune_row = seat_operation(journal, row["tuneRequestId"], db,
                    ["tune", row["domainId"], row["seatId"], "1", "1", case["edit"]["setting"],
                     value_json, "", "", ""])
                check(tuned["receipt"]["result"]["settings"] == expected and current["revision"] == 2,
                      "Original User tune did not persist on project A's copy")
                check(tune_row["state"] == "IDLE" and tune_row["revision"] == tune_row["generation"] == 2 and
                      json.loads(tune_row["settings_json"]) == expected,
                      "Original User tune receipt differs from durable CAS snapshot")
            else:
                check(row["tuneRequestId"] is None and current["revision"] == 1,
                      "Project B copy was unexpectedly tuned")
            check(json.loads(copied["settings_json"]) == expected, "Persisted copied settings differ from User receipt")
            card = operation(journal, row["cardRequestId"], db)
            check(card["receipt"]["result"]["settings"] == expected and
                  card["receipt"]["revision"] == str(current["revision"]),
                  "Original state-card differs from persistent copy")
            facts.append({"domainId": row["domainId"], "templateId": row["templateId"],
                          "templateSettingsSha256": digest(template["settings_json"].encode()),
                          "seatId": row["seatId"], "layer": current["layer"], "state": current["state"],
                          "revision": current["revision"], "copiedSettingsSha256": digest(copied["settings_json"].encode()),
                          "createRequestId": row["createRequestId"], "tuneRequestId": row["tuneRequestId"],
                          "stateCardRequestId": row["cardRequestId"]})
        if phase == "baseline":
            lead_facts = None
            if case.get("leadSeat"):
                lead = case["leadSeat"]
                source = one(db, "SELECT incarnation,parent_seat_id,layer,state,generation,revision "
                             "FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",
                             (lead["domainId"], lead["seatId"]))
                parent = one(db, "SELECT layer,state FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",
                             (lead["domainId"], lead["parentSeatId"]))
                check(source["incarnation"] == lead["incarnation"] and source["parent_seat_id"] == lead["parentSeatId"] and
                      source["layer"] == "LEAD" and source["state"] == "IDLE" and
                      str(source["generation"]) == str(lead["generation"]) and str(source["revision"]) == str(lead["revision"]) and
                      parent["layer"] == "USER" and parent["state"] != "RECLAIMED",
                      "Configured LEAD fixture differs from original registered IDLE producer facts")
                lead_facts = {"domainId": lead["domainId"], "seatId": lead["seatId"],
                              "parentSeatId": lead["parentSeatId"], "incarnation": source["incarnation"],
                              "generation": str(source["generation"]), "revision": str(source["revision"]),
                              "layer": source["layer"], "state": source["state"]}
                current_settings = one(db, "SELECT template_id,settings_json FROM gogoke_v37_seat_settings "
                                       "WHERE domain_id=? AND seat_id=?", (lead["domainId"], lead["seatId"]))
                check(current_settings["template_id"], "LEAD tune fixture needs a real copied settings row")
                lead_facts["templateId"] = current_settings["template_id"]
                lead_facts["settingsSha256"] = digest(current_settings["settings_json"].encode())
            return {"projects": facts, "leadSeat": lead_facts}
        if case.get("modelAttempt"):
            target = next(row for row in projects if row["domainId"] == case["modelAttempt"]["domainId"])
            verify_model_denial(db, journal, case["modelAttempt"], target, case)
        return {"projects": facts, "modelDenial": case.get("modelAttempt") is not None,
                "leadSeatVerified": verify_lead_seat(db, journal, case.get("leadSeat"), baseline)}

    database = root / "state.sqlite"
    wal = Path(str(database) + "-wal")
    check(database.is_file() and (not wal.exists() or wal.stat().st_size == 0),
          "Normal close and empty/absent WAL required")
    database_hash = digest(database.read_bytes())
    with sqlite3.connect(database.as_uri() + "?mode=ro&immutable=1", uri=True) as db:
        db.row_factory = sqlite3.Row
        facts = inspect(db)
    check(digest(database.read_bytes()) == database_hash and (not wal.exists() or wal.stat().st_size == 0),
          "Immutable readback changed database bytes or WAL")
    if phase == "final":
        check(facts["projects"][0]["templateSettingsSha256"] == facts["projects"][1]["templateSettingsSha256"],
              "Same-ID source templates differ across the two actual domains")
    proof = {"schema": "gogoke.37.private-m2-seat-management-readback.v1", "phase": phase,
             "caseId": journal["caseId"], "sourceCommit": case["sourceCommit"],
             "readerSha256": case["readerSha256"], "databaseSha256": database_hash,
             "measurementPreservedDatabaseBytes": True, "directCaseEvidence": phase == "final",
             "acceptance": False, **facts}
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"phase": phase, "databaseSha256": database_hash,
                      "directCaseEvidence": proof["directCaseEvidence"], "acceptance": False}))


if __name__ == "__main__":
    main()
