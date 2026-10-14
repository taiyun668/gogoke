"""One closed, immutable F HOME reader shared by testcase callers.

This observes the actual F writer journal. It never closes a HOME, reads auth
content, supplies a StopFact, or replaces the shared native/close instruments.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
CORE_SHA = "80ff35a0cadcf4e3bcf9cc6b2838fe941d34ee131a58684c300402153b5ef174"
INPUT_SCHEMA = "gogoke.37.temporary-home-readback-input.v1"
REPORT_SCHEMA = "gogoke.37.temporary-home-readback.v1"
HOME_SQL = """SELECT home_id,instance_id,domain_id,kind,owner_id,generation,
 COALESCE(directory_ref,'') AS directory_ref,
 COALESCE(directory_identity,'') AS directory_identity,state,
 CAST(revision AS TEXT) AS revision FROM gogoke_v37_instance_homes ORDER BY home_id"""
OP_SQL = """SELECT request_id,request_hex,target_id,phase,receipt_json,native_receipt_id
 FROM gogoke_v37_instance_operations WHERE request_id=?"""
STOP_SQL = """SELECT e.seat_id,e.seat_incarnation,e.binding_id,e.generation,e.phase,e.stop_fact_id,e.stop_request_id,
 e.result_revision,c.domain_id AS c_domain,c.profile_id,c.generation AS c_generation,
 c.state AS c_state,c.stop_proof_hash,o.raw_hex,o.status,o.previous_revision,o.revision
 FROM gogoke_v37_h_process_episode e
 JOIN gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id
 JOIN gogoke_v37_h_operation o ON o.domain_id=e.domain_id
  AND o.request_id=e.stop_request_id AND o.session_id=e.session_id AND o.operation='stop'
 WHERE e.home_id=? AND e.instance_id=? AND e.domain_id=? AND e.session_id=? AND e.generation=?"""
DESIGNATION_SQL = """SELECT d.seat_id,d.incarnation,s.instance_id,s.state,s.layer,s.kind,
 CAST(s.generation AS TEXT) AS seat_generation
 FROM gogoke_v37_seat_secretary d LEFT JOIN gogoke_v37_seats s
 ON s.domain_id=d.domain_id AND s.seat_id=d.seat_id
 WHERE d.singleton=1 AND d.domain_id='global'"""
CURRENT_SQL = """SELECT c.session_id,c.state,c.instance_id,c.home_id,c.binding_id,
 c.generation,CAST(c.revision AS TEXT) AS revision,c.stop_fact_id,c.process_operation_id,
 CAST(n.seat_authorization_generation AS TEXT) AS authorization_generation,
 n.selected_instance_id
 FROM gogoke_v37_h_claim c JOIN gogoke_v37_native_selection n
 ON n.domain_id=c.domain_id AND n.session_id=c.session_id
 WHERE c.domain_id='global' AND n.seat_id=? AND n.seat_incarnation=? AND c.state<>'RELEASED'"""
EPISODE_SQL = """SELECT e.request_id,e.raw_hex,e.generation,e.old_generation,e.phase,
 e.process_operation_id,e.binding_id,e.seat_id,e.seat_incarnation,
 e.home_id,e.instance_id,e.session_id,e.domain_id,e.stop_fact_id
 FROM gogoke_v37_h_process_episode e WHERE e.home_id=? AND e.instance_id=?
 AND e.domain_id=? AND e.session_id=? AND e.generation=?"""


def need(ok, reason):
    if not ok:
        raise RuntimeError(reason)


def sha(value):
    return hashlib.sha256(value).hexdigest()


def file_sha(path):
    return sha(Path(path).read_bytes())


def read(path):
    return json.loads(Path(path).read_text(encoding="utf-8-sig"))


def pinned(ref):
    need(isinstance(ref, dict) and isinstance(ref.get("path"), str) and
         re.fullmatch(r"[a-f0-9]{64}", ref.get("sha256", "")), "Source pin missing")
    target = Path(ref["path"]).resolve(strict=True)
    need(not Path(ref["path"]).is_symlink() and target.is_file() and
         file_sha(target) == ref["sha256"], "Pinned source bytes differ")
    return target


def unframe(encoded):
    need(isinstance(encoded, str) and re.fullmatch(r"(?:[a-f0-9]{2})+", encoded),
         "Native length-framed fingerprint is not exact lower hex")
    raw = bytes.fromhex(encoded)
    cursor, result = 0, []
    while cursor < len(raw):
        need(cursor + 8 <= len(raw), "Native fingerprint length truncated")
        size = int.from_bytes(raw[cursor:cursor + 8], "big")
        cursor += 8
        need(size <= len(raw) - cursor, "Native fingerprint field truncated")
        result.append(raw[cursor:cursor + size])
        cursor += size
    return result


def one_operation(db, request_id):
    found = [dict(row) for row in db.execute(OP_SQL, (request_id,))]
    need(len(found) == 1, "Exact original F operation missing or ambiguous")
    return found[0]


def startup_baseline(db, core, root, home_by_id):
    designated = [dict(row) for row in db.execute(DESIGNATION_SQL)]
    need(len(designated) <= 1, "Secretary designation ambiguous")
    if not designated:
        return {"state": "NONE", "domainId": "global"}
    seat = designated[0]
    common = {"domainId": "global", "seatId": seat["seat_id"],
              "seatIncarnation": seat["incarnation"], "instanceId": seat["instance_id"]}
    def blocked(reason):
        return dict(common, state="BLOCKED", reason=reason)
    if seat["layer"] != "USER" or seat["kind"] != "LONG" or seat["state"] != "BUSY":
        return blocked("Designated Secretary seat is not the current BUSY USER/LONG seat")
    claims = [dict(row) for row in db.execute(CURRENT_SQL,
        (seat["seat_id"], seat["incarnation"]))]
    if len(claims) != 1:
        return blocked("Secretary has no unique non-RELEASED selected claim")
    claim = claims[0]
    if (claim["state"] != "STOPPED" or claim["instance_id"] != seat["instance_id"] or
            claim["selected_instance_id"] != seat["instance_id"] or
            claim["authorization_generation"] != seat["seat_generation"] or
            claim["binding_id"] != "binding-" + claim["home_id"][5:]):
        return blocked("Secretary selected claim is not the exact STOPPED current seat")
    old_home = home_by_id.get(claim["home_id"])
    if (not old_home or old_home["domain_id"] != "global" or
            old_home["instance_id"] != claim["instance_id"] or
            old_home["owner_id"] != claim["session_id"] or
            old_home["generation"] != claim["generation"]):
        return blocked("Secretary stopped claim does not bind an existing original HOME")
    episodes = [dict(row) for row in db.execute(EPISODE_SQL,
        (claim["home_id"], claim["instance_id"], "global", claim["session_id"],
         claim["generation"]))]
    if len(episodes) != 1:
        return blocked("Secretary old generation has no unique original H episode")
    episode = episodes[0]
    stopped = [dict(row) for row in db.execute(STOP_SQL,
        (claim["home_id"], claim["instance_id"], "global", claim["session_id"],
         claim["generation"]))]
    if (len(stopped) != 1 or episode["phase"] != "STOPPED" or
            episode["process_operation_id"] != claim["process_operation_id"] or
            episode["binding_id"] != claim["binding_id"] or
            episode["seat_id"] != seat["seat_id"] or
            episode["seat_incarnation"] != seat["incarnation"] or
            episode["stop_fact_id"] != claim["stop_fact_id"] or
            stopped[0]["c_state"] != "STOPPED" or
            stopped[0]["stop_proof_hash"] != claim["stop_fact_id"] or
            not re.fullmatch(r"sha256:[a-f0-9]{64}", claim["stop_fact_id"] or "")):
        return blocked("Secretary old H episode/C custody does not prove original STOPPED")
    pending = [dict(row) for row in db.execute("""SELECT request_id,phase FROM gogoke_v37_h_process_episode
        WHERE domain_id='global' AND session_id=? AND old_generation IS NOT NULL
        AND phase IN ('INTENT','PREPARED','UNKNOWN')""", (claim["session_id"],))]
    if pending:
        return blocked("Secretary resume has an unresolved original pending episode")
    try:
        old = int(claim["generation"])
        need(str(old) == claim["generation"] and old >= 1, "Old generation invalid")
        expected = str(old + 1)
    except (TypeError, ValueError):
        return blocked("Secretary old generation is invalid")
    identity = "secretary-resume\n" + "\n".join([
        core.windows_file_identity(root), claim["session_id"],
        claim["generation"], claim["revision"]])
    request_id = "resident-resume-" + sha(identity.encode())[:40]
    return dict(common, state="RESUME_EXPECTED", sessionId=claim["session_id"],
        oldGeneration=claim["generation"], oldRevision=claim["revision"],
        expectedGeneration=expected, expectedCreateRequestId=request_id,
        baselineHomeId=claim["home_id"])


def declared_repairs(config, homes):
    repairs = config.get("authorizedRepairs", [])
    need(isinstance(repairs, list) and len(repairs) <= 4,
         "Authorized repairs must be an explicitly bounded list")
    by_id = {row["home_id"]: row for row in homes}
    seen = set()
    result = []
    for repair in repairs:
        need(isinstance(repair, dict), "Authorized repair selector missing")
        home_id = repair.get("homeId")
        need(isinstance(home_id, str) and home_id not in seen,
             "Authorized repair HOME duplicate or invalid")
        seen.add(home_id)
        home = by_id.get(home_id)
        need(home is not None and all((home[key] == repair.get(selector) for key, selector in
            (("instance_id", "instanceId"), ("domain_id", "domainId"),
             ("owner_id", "sessionId"), ("generation", "generation")))) and
             home["state"] == "ACTIVE" and home["revision"] == repair.get("expectedRevision") == "1",
             "Authorized repair does not select an exact prior ACTIVE(1) HOME")
        pinned(repair["provenance"])
        result.append({key: repair[key] for key in ("homeId", "instanceId", "domainId",
            "sessionId", "generation", "expectedRevision", "provenance")})
    return result


def original_stop_source(db, core, home, attempt):
    row = core.exact_one(db, STOP_SQL, (home["home_id"], home["instance_id"],
        home["domain_id"], home["owner_id"], home["generation"]))
    stop = attempt["hStopSource"]
    raw = stop["rawFrame"].encode()
    request = json.loads(raw)
    need(row["phase"] == row["c_state"] == "STOPPED" and row["status"] == "APPLIED" and
         isinstance(row["seat_id"], str) and bool(row["seat_id"]) and
         isinstance(row["seat_incarnation"], str) and bool(row["seat_incarnation"]) and
         row["binding_id"] == "binding-" + home["home_id"][5:] and
         row["generation"] == row["c_generation"] == home["generation"] and
         row["c_domain"] == home["domain_id"] and row["profile_id"] == home["instance_id"] and
         row["stop_fact_id"] == row["stop_proof_hash"] == stop["stopFact"] and
         re.fullmatch(r"sha256:[a-f0-9]{64}", stop["stopFact"] or "") and
         row["stop_request_id"] == stop["requestId"] and row["raw_hex"] == raw.hex() and
         stop["rawSha256"] == sha(raw), "Original H/C stopped source differs")
    need(request == {"schema": "gogoke.37.operations.v1", "family": "K-SESSION", "operation": "stop",
        "requestId": row["stop_request_id"], "domainId": home["domain_id"], "targetId": home["owner_id"],
        "expectedRevision": str(row["previous_revision"]),
        "payload": {"seatId": row["seat_id"], "generation": home["generation"]}} and
        row["revision"] == row["previous_revision"] + 1 and
        row["result_revision"] is not None and row["result_revision"] <= row["previous_revision"],
        "Original H stop USER raw/CAS/seat/generation differs")
    return stop["stopFact"], sha(raw)


def verify_attempt(db, core, root, attempt, home_by_id, historical=False):
    create_raw = attempt["originalCreateRawFrame"].encode("utf-8")
    original = json.loads(create_raw)
    need(sha(create_raw) == attempt["originalCreateRawSha256"] and
         original["schema"] == "gogoke.37.operations.v1" and
         original["family"] == "K-SESSION" and
         original["operation"] in ("admission-reserve", "open", "resume") and
         original["requestId"] == attempt["originalCreateRequestId"] and
         original["domainId"] == attempt["domainId"] and
         original["targetId"] == attempt["sessionId"], "Original own producer raw differs")
    create_id = "homeprep-" + sha((attempt["domainId"] + "\n" + original["requestId"]).encode())[:40]
    create = one_operation(db, create_id)
    need(create["phase"] == "APPLIED", "Original F creation is not APPLIED")
    fields = unframe(create["request_hex"])
    need(len(fields) == 9 and fields[0] == b"temporary-create-detached" and
         fields[1] == create_raw, "F creation is not the exact original USER raw")
    home_id = create["target_id"]
    home = home_by_id.get(home_id)
    need(home is not None and home["home_id"] == home_id and
         (not attempt.get("homeId") or attempt["homeId"] == home_id) and
         home["instance_id"] == attempt["instanceId"] and
         home["domain_id"] == attempt["domainId"] and
         home["owner_id"] == attempt["sessionId"] and home["kind"] == "SESSION" and
         home["state"] == "CLOSED" and home["revision"] == "2", "Own original HOME is not CLOSED(2)")
    need(fields[2:8] == [home_id.encode(), home["instance_id"].encode(),
         home["domain_id"].encode(), b"SESSION", home["owner_id"].encode(),
         home["generation"].encode()] and fields[8].startswith(b"S-1-15-2-"),
         "Original F creation metadata/SID differs")
    close_raw = attempt["closeRawFrame"].encode("utf-8")
    close_request = json.loads(close_raw)
    selector = ({"historicalHomeId": home_id} if historical else
                {"originalCreateRequestId": attempt["originalCreateRequestId"]})
    need(sha(close_raw) == attempt["closeRawSha256"] and
         close_request == {"schema": "gogoke.37.operations.v1", "family": "K-INSTANCE",
            "operation": "temporary-home-close", "requestId": attempt["closeRequestId"],
            "domainId": attempt["domainId"], "targetId": attempt["instanceId"],
            "expectedRevision": "1", "payload": {"sessionId": attempt["sessionId"],
                **selector}},
         "Original close raw is not the exact own request")
    close = one_operation(db, attempt["closeRequestId"])
    need(close["phase"] == "APPLIED" and close["target_id"] == home_id,
         "Original independent F close is not APPLIED")
    need(unframe(close["request_hex"]) == [b"temporary-home-close-v2", close_raw,
         home_id.encode(), b"1", fields[8]], "Native close fingerprint differs")
    native = json.loads(close["receipt_json"])
    user = attempt["closeReceipt"]
    need(user["schema"] == "gogoke.37.operations.v1" and
         user["requestId"] == attempt["closeRequestId"] and
         user["family"] == "K-INSTANCE" and user["operation"] == "temporary-home-close" and
         user["targetId"] == attempt["instanceId"] and user["status"] in ("APPLIED", "REPLAYED") and
         user["previousRevision"] == "1" and user["revision"] == "2",
         "Original USER close receipt identity/CAS differs")
    result = user["result"]
    need(native["schema"] == "gogoke.37.temporary-home-close.v2" and
         native["homeId"] == result["homeId"] == home_id and
         native["originalCreateRequestId"] == attempt["originalCreateRequestId"] and
         native["fCreateOperationId"] == create_id and
         native["beforeRevision"] == 1 and native["afterRevision"] == 2 and
         native["proofKind"] == result["proofKind"] == "ORIGINAL_STOP" and
         re.fullmatch(r"[a-f0-9]{64}", native["sourceFingerprint"]) and
         native["sourceFingerprint"] == result["sourceFingerprint"] and
         native["receiptId"] == close["native_receipt_id"] == result["nativeReceiptId"] and
         re.fullmatch(r"f-home-close-v2-[a-f0-9]{64}", close["native_receipt_id"] or "") and
         result["directoryRef"] == home["directory_ref"] and result["state"] == "CLOSED",
         "USER receipt differs from the actual independent native F journal")
    stop_fact, stop_raw_sha = original_stop_source(db, core, home, attempt)
    proof_fingerprint = sha(("home-stop-v1\n" + "\n".join([home_id, create_id,
        sha(create_raw), stop_raw_sha, stop_fact])).encode())
    source_fingerprint = sha(("temporary-home-close-v2\n" + "\n".join([
        core.windows_file_identity(root), core.windows_file_identity(Path(root) / "state.sqlite"),
        home_id, home["instance_id"], home["domain_id"], home["owner_id"], home["generation"],
        home["directory_identity"], fields[8].decode(), create["request_hex"], sha(create_raw),
        "ORIGINAL_STOP", proof_fingerprint])).encode())
    receipt_id = "f-home-close-v2-" + sha(("f-home-close-receipt-v2\n" + "\n".join([
        attempt["closeRequestId"], sha(close_raw), source_fingerprint, home_id,
        home["instance_id"], home["domain_id"], home["owner_id"], "1", "2"])).encode())
    need(native["sourceFingerprint"] == source_fingerprint and native["receiptId"] == receipt_id,
         "Native receipt source or independent receipt ID does not match the original facts")
    expected_directory = "temp-home-" + sha(home_id.encode())
    need(home["directory_ref"] == expected_directory, "Own HOME directory reference differs")
    directory = core.contained_physical(root, Path(root) / "v37-temporary-homes" /
        home["instance_id"] / expected_directory)
    need(core.windows_file_identity(directory) == home["directory_identity"] ==
         create["native_receipt_id"], "Original HOME FileID changed")
    marker = core.contained_physical(directory, directory / "gogoke-temporary-home.marker")
    expected_marker = ("gogoke-v37-temporary-home-detached-v1\n" +
        core.windows_file_identity(root) + "\n" + "\n".join([home["instance_id"], home_id,
        home["domain_id"], "SESSION", home["owner_id"], home["generation"], create_id,
        sha(create["request_hex"].encode()), home["directory_identity"], fields[8].decode()]) + "\n")
    need(marker.read_bytes() == expected_marker.encode(), "Original native HOME marker changed")
    return {"caseId": attempt.get("caseId"), "homeId": home_id,
        "originalCreateRequestId": attempt["originalCreateRequestId"],
        "domainId": home["domain_id"], "sessionId": home["owner_id"],
        "instanceId": home["instance_id"], "state": "CLOSED", "revision": "2",
        "closeRequestId": attempt["closeRequestId"], "nativeReceiptId": close["native_receipt_id"],
        "proofKind": native["proofKind"], "sourceFingerprint": native["sourceFingerprint"]}


def source_attempt(db, home, close_fields, historical=False):
    candidates = []
    for row in db.execute("SELECT request_id,request_hex,phase FROM gogoke_v37_instance_operations WHERE target_id=?",
                          (home["home_id"],)):
        item = dict(row)
        fields = unframe(item["request_hex"])
        if fields and fields[0] == b"temporary-create-detached":
            candidates.append((item, fields))
    need(len(candidates) == 1 and candidates[0][0]["phase"] == "APPLIED",
         "Exact original F create source missing or ambiguous")
    raw = candidates[0][1][1]
    original = json.loads(raw)
    need(original.get("operation") in ("admission-reserve", "open", "resume") and
         original.get("domainId") == home["domain_id"] and
         original.get("targetId") == home["owner_id"], "Original native create raw is not H producer")
    episodes = [dict(row) for row in db.execute(EPISODE_SQL,
        (home["home_id"], home["instance_id"], home["domain_id"], home["owner_id"],
         home["generation"])) if row["request_id"] == original["requestId"]]
    if original["operation"] in ("open", "resume"):
        need(len(episodes) == 1 and episodes[0]["request_id"] == original["requestId"] and
             episodes[0]["raw_hex"] == raw.hex() and
             episodes[0]["binding_id"] == "binding-" + home["home_id"][5:] and
             episodes[0]["phase"] == "STOPPED" and
             episodes[0]["process_operation_id"] == "h-" + original["operation"] + "-" + sha(raw)[:40],
             "Original H process producer does not bind F HOME")
    else:
        need(not episodes, "Admission reserve HOME has unexpected H process episode")
        rows = [dict(row) for row in db.execute("""SELECT raw_hex,status FROM gogoke_v37_h_operation
            WHERE domain_id=? AND request_id=? AND session_id=? AND operation='admission-reserve'""",
            (home["domain_id"], original["requestId"], home["owner_id"]))]
        need(len(rows) == 1 and rows[0] == {"raw_hex": raw.hex(), "status": "APPLIED"},
             "Original H admission source differs")
    stop_rows = [dict(row) for row in db.execute(STOP_SQL,
        (home["home_id"], home["instance_id"], home["domain_id"], home["owner_id"],
         home["generation"]))]
    need(len(stop_rows) == 1 and stop_rows[0]["raw_hex"] is not None,
         "Original H stop source missing or ambiguous")
    stop = stop_rows[0]
    stop_raw = bytes.fromhex(stop["raw_hex"])
    generated = dict(close_fields, originalCreateRequestId=original["requestId"],
        originalCreateRawFrame=raw.decode("utf-8"), originalCreateRawSha256=sha(raw),
        hStopSource={"requestId": stop["stop_request_id"],
            "rawFrame": stop_raw.decode("utf-8"), "rawSha256": sha(stop_raw),
            "stopFact": stop["stop_fact_id"]})
    if historical:
        generated["homeId"] = home["home_id"]
    return generated, (episodes[0] if episodes else None)


def run(input_path, output_path):
    source = Path(__file__).resolve(strict=True)
    config = read(input_path)
    mode = config.get("mode")
    phase = config.get("phase", mode)
    need(config.get("schema") == INPUT_SCHEMA and mode in ("baseline", "verify", "startup") and
         ((isinstance(phase, str) and re.fullmatch(r"startup-\d+", phase)) if mode == "startup"
          else phase == mode) and config.get("readerSha256") == file_sha(source),
         "Reader input/source phase pin differs")
    core_path = pinned(config["core"])
    need(config["core"]["sha256"] == CORE_SHA, "Shared native core changed")
    spec = importlib.util.spec_from_file_location("shared_installed_native", core_path)
    core = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(core)
    manifest_path = pinned(config["manifest"])
    manifest_origin = Path(config["manifestOrigin"])
    need(manifest_origin.is_absolute() and manifest_origin.parent.resolve(strict=True) == manifest_path.parent
         and manifest_path.name == f"temporary-home-readback-{phase}-manifest.json",
         "Readback must retain its exact phase snapshot beside the original manifest")
    manifest = read(manifest_path)
    need(manifest.get("schema") == "gogoke.37.temporary-home-attempts.v1" and
         isinstance(manifest.get("attempts"), list) and
         isinstance(manifest.get("startupAttempts", []), list) and
         isinstance(manifest.get("authorizedRepairs", []), list),
         "Own creation manifest differs")
    producers = manifest.get("producers")
    cases = manifest.get("cases")
    need(isinstance(producers, list) and isinstance(cases, list) and
         len({row["caseId"] for row in cases}) == len(cases) and
         len({row["sessionId"] for row in producers}) == len(producers),
         "Exact own-case producer collection required")
    for attempt in manifest["attempts"]:
        matched = [row for row in producers if row["sessionId"] == attempt["sessionId"] and
            row["instanceId"] == attempt["instanceId"] and row["caseId"] == attempt["caseId"] and
            row["domainId"] == attempt["domainId"] == manifest["domainId"]]
        case_rows = [row for row in cases if row["caseId"] == attempt["caseId"] and
            row.get("attempts", []).count(attempt["originalCreateRequestId"]) == 1]
        need(len(matched) == len(case_rows) == 1, "Attempt has no exact original case/domain/producer anchor")
    root = Path(config["stateRoot"]).resolve(strict=True)
    output = Path(output_path).resolve(strict=False)
    need(not output.exists() and output != root and root not in output.parents,
         "Fresh observer output outside installed state required")
    report = {"schema": REPORT_SCHEMA, "mode": mode, "phase": phase, "acceptance": False,
        "caseId": manifest["caseId"], "domainId": manifest["domainId"],
        "source": {"reader": {"path": str(source), "sha256": file_sha(source)},
            "core": config["core"], "manifest": config["manifest"],
            "manifestOrigin": str(manifest_origin),
            "inputSha256": file_sha(input_path)}, "credentialsRead": False}
    with core.closed_database(root) as (db, measured):
        db_identity = core.windows_file_identity(root / "state.sqlite")
        homes = [dict(row) for row in db.execute(HOME_SQL)]
        report.update({"measurement": measured, "databasePhysicalIdentity": db_identity})
        if mode in ("baseline", "startup"):
            need((mode != "baseline" or not manifest["attempts"] and
                 not manifest.get("startupAttempts", []) and config.get("baseline") is None),
                 "Baseline must precede all native creation attempts")
            repairs = declared_repairs(config, homes) if mode == "baseline" else None
            report.update(state="BASELINE_MEASURED" if mode == "baseline" else
                          "STARTUP_SOURCE_MEASURED", homes=homes,
                          startup=startup_baseline(db, core, root,
                              {row["home_id"]: row for row in homes}))
            if repairs is not None:
                report["authorizedRepairs"] = [dict(repair, beforeHome=next(row for row in homes
                    if row["home_id"] == repair["homeId"])) for repair in repairs]
        else:
            baseline_path = pinned(config["baseline"])
            baseline = read(baseline_path)
            need(baseline.get("schema") == REPORT_SCHEMA and baseline.get("state") == "BASELINE_MEASURED"
                 and baseline["source"]["reader"] == report["source"]["reader"] and
                 baseline["source"]["core"] == config["core"] and
                 baseline["caseId"] == manifest["caseId"] and baseline["domainId"] == manifest["domainId"] and
                 baseline["source"]["manifestOrigin"] == str(manifest_origin) and
                 baseline["measurement"]["rootPhysicalIdentity"] == measured["rootPhysicalIdentity"] and
                 baseline["measurement"]["rootIdentity"] == measured["rootIdentity"] and
                 baseline["databasePhysicalIdentity"] == db_identity, "Original observer/root/DB identity changed")
            home_by_id = {row["home_id"]: row for row in homes}
            before = {row["home_id"]: row for row in baseline["homes"]}
            errors, own, startup_homes, repairs = [], [], [], []
            for attempt in manifest["attempts"]:
                try:
                    observed = verify_attempt(db, core, root, attempt, home_by_id)
                    original, _ = source_attempt(db, home_by_id[observed["homeId"]], attempt)
                    need(all(original[key] == attempt[key] for key in (
                        "originalCreateRequestId", "originalCreateRawFrame",
                        "originalCreateRawSha256", "hStopSource")),
                        "Normal HOME original H/F producer differs from caller source")
                    own.append(observed)
                except Exception as error:
                    errors.append({"source": "temporary", "caseId": attempt.get("caseId"),
                                   "error": repr(error)})
            startup_ids = set()
            launch_sources = []
            for attempt in manifest.get("startupAttempts", []):
                try:
                    need(attempt.get("sourceKind") == "NATIVE_STARTUP" and
                         attempt.get("domainId") == "global" and
                         attempt.get("caseId") not in startup_ids and
                         len([row for row in cases if row["caseId"] == attempt["caseId"]]) == 1,
                         "Startup case/source identity differs")
                    startup_ids.add(attempt["caseId"])
                    launch_path = pinned(attempt["launchSource"])
                    launch_sources.append(attempt["launchSource"])
                    launch = read(launch_path)
                    source_start = launch["startup"]
                    need(launch.get("schema") == REPORT_SCHEMA and
                         launch.get("state") == "STARTUP_SOURCE_MEASURED" and
                         launch.get("caseId") == manifest["caseId"] and
                         launch["source"]["reader"] == report["source"]["reader"] and
                         launch["source"]["core"] == config["core"] and
                         launch["source"]["manifestOrigin"] == str(manifest_origin) and
                         launch["measurement"]["rootPhysicalIdentity"] ==
                             measured["rootPhysicalIdentity"] and
                         launch["databasePhysicalIdentity"] == db_identity and
                         source_start["state"] == "RESUME_EXPECTED" and
                         all((source_start[key] == attempt[value] for key, value in (
                             ("domainId", "domainId"), ("seatId", "seatId"),
                             ("instanceId", "instanceId"), ("sessionId", "sessionId"),
                             ("oldGeneration", "producerGeneration"),
                             ("oldRevision", "oldRevision"),
                             ("expectedGeneration", "generation"),
                             ("expectedCreateRequestId", "originalCreateRequestId")))) and
                         not any(row["home_id"] == attempt.get("homeId") for row in launch["homes"]),
                         "Startup launch source is not exact pre-spawn RESUME_EXPECTED")
                    bootstrap = attempt["bootstrap"]
                    frame = json.loads(bootstrap["rawFrame"])
                    reply = json.loads(bootstrap["rawReply"])
                    need(frame == bootstrap["frame"] == {
                        "schema": "gogoke.37.owner-configuration.v1",
                        "command": "secretary-configuration-read"} and
                        reply == bootstrap["reply"] and
                        reply["schema"] == "gogoke.37.secretary-configuration.v1" and
                        reply["state"] == "DESIGNATED" and
                        reply["seatId"] == attempt["seatId"] and
                        reply["incarnation"] == source_start["seatIncarnation"] and
                        reply["instanceId"] == attempt["instanceId"] and
                        reply["conversation"]["state"] == "FOUND" and
                        reply["conversation"]["sessionId"] == attempt["sessionId"] and
                        reply["conversation"]["generation"] == attempt["generation"] and
                        reply["conversation"]["claimState"] == "COMMITTED" and
                        reply["conversation"]["runtimeAvailable"] is True and
                        bootstrap["startedAt"] <= bootstrap["finishedAt"],
                        "Original USER bootstrap read does not name actual resumed Secretary")
                    create_id = "homeprep-" + sha(("global\n" +
                        attempt["originalCreateRequestId"]).encode())[:40]
                    create = one_operation(db, create_id)
                    home = home_by_id.get(create["target_id"])
                    need(home is not None and home["home_id"] not in
                         {row["home_id"] for row in launch["homes"]},
                         "Startup F HOME was present before its launch")
                    sourced, episode = source_attempt(db, home, attempt)
                    original = json.loads(sourced["originalCreateRawFrame"])
                    need(episode is not None and
                         sourced["originalCreateRequestId"] == attempt["originalCreateRequestId"] and
                         sourced["hStopSource"] == attempt["hStopSource"] and
                         episode["old_generation"] == source_start["oldGeneration"] and
                         episode["generation"] == attempt["generation"] and
                         episode["seat_id"] == attempt["seatId"] and
                         episode["seat_incarnation"] == source_start["seatIncarnation"] and
                         original == {"schema": "gogoke.37.operations.v1",
                            "family": "K-SESSION", "operation": "resume",
                            "requestId": attempt["originalCreateRequestId"],
                            "domainId": "global", "targetId": attempt["sessionId"],
                            "expectedRevision": source_start["oldRevision"],
                            "payload": {"generation": source_start["oldGeneration"]}},
                         "Original startup H/F resume source differs from pre-spawn plan")
                    startup_homes.append(verify_attempt(db, core, root, sourced,
                        home_by_id))
                except Exception as error:
                    errors.append({"source": "startup", "caseId": attempt.get("caseId"),
                                   "error": repr(error)})
            declared = baseline.get("authorizedRepairs", [])
            wanted = {row["homeId"]: row for row in declared}
            seen_repairs = set()
            for repair in manifest.get("authorizedRepairs", []):
                try:
                    home_id = repair["homeId"]
                    need(home_id not in seen_repairs and home_id in wanted,
                         "Repair is not an exact unique baseline authorization")
                    seen_repairs.add(home_id)
                    authorized = wanted[home_id]
                    need(all(repair.get(key) == authorized[key] for key in (
                        "homeId", "instanceId", "domainId", "sessionId", "generation",
                        "expectedRevision", "provenance")) and
                        before[home_id] == authorized["beforeHome"],
                        "Repair selector/provenance differs from baseline intent")
                    pinned(repair["provenance"])
                    home = home_by_id[home_id]
                    close = one_operation(db, repair["closeRequestId"])
                    native = json.loads(close["receipt_json"])
                    sourced, _ = source_attempt(db, home, dict(repair,
                        originalCreateRequestId=native["originalCreateRequestId"]), True)
                    need(sourced["originalCreateRequestId"] == native["originalCreateRequestId"],
                         "Repair native receipt original create selector differs")
                    repairs.append(verify_attempt(db, core, root, sourced, home_by_id, True))
                except Exception as error:
                    errors.append({"source": "authorizedRepair", "homeId": repair.get("homeId"),
                                   "error": repr(error)})
            own_ids = [row["homeId"] for row in own + startup_homes]
            if len(set(own_ids)) != len(own_ids) or any(value in before for value in own_ids):
                errors.append({"source": "inventory", "error": "Own HOME duplicated or pre-existing"})
            repaired_ids = {row["homeId"] for row in repairs}
            if repaired_ids != seen_repairs or len(seen_repairs) != len(wanted):
                errors.append({"source": "authorizedRepair",
                    "error": "Declared baseline repairs lack exact final proof"})
            after = {row["home_id"]: row for row in homes}
            changed, missing, extra = [], sorted(set(before) - set(after)), []
            for home_id, prior in before.items():
                if home_id not in after:
                    continue
                expected = dict(prior)
                if home_id in repaired_ids:
                    expected.update(state="CLOSED", revision="2")
                if after[home_id] != expected:
                    changed.append({"homeId": home_id, "before": prior, "after": after[home_id]})
            extra = sorted(set(after) - set(before) - set(own_ids))
            exact = not errors and not (changed or missing or extra)
            if changed or missing or extra:
                errors.append({"source": "inventory", "error": "HOME inventory differs from declared scope"})
            report.update(state=("EXACT_OWN_HOMES_CLOSED" if own_ids else
                "NO_HOME_CREATE_DISPATCHED") if exact else "READBACK_FAILED",
                baseline=config["baseline"], facts={"temporaryHomes": own,
                    "startupHomes": startup_homes, "authorizedRepairs": repairs,
                    "homeInventoryMatchesDeclaredScope": exact,
                    "homeInventoryUnchangedExceptOwnClosed": exact and not repairs},
                differences={"changed": changed, "missing": missing, "extra": extra},
                proofFailures=errors)
            for source_ref in launch_sources:
                need(file_sha(source_ref["path"]) == source_ref["sha256"],
                     "Startup source changed during final readback")
        need(core.windows_file_identity(root / "state.sqlite") == db_identity,
             "Original database object changed while reading")
    need(file_sha(source) == config["readerSha256"] and file_sha(core_path) == CORE_SHA and
         file_sha(manifest_path) == config["manifest"]["sha256"], "Loaded sources changed during readback")
    with output.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2, ensure_ascii=True)
        stream.write("\n")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    try:
        result = run(args.input, args.output)
        print(json.dumps({"state": result["state"], "acceptance": False}))
        if result["state"] == "READBACK_FAILED":
            print(json.dumps({"state": result["state"],
                "differences": result.get("differences"),
                "proofFailures": result.get("proofFailures"), "acceptance": False}), file=sys.stderr)
            raise SystemExit(1)
    except Exception as error:
        diagnostic = {"schema": REPORT_SCHEMA, "state": "READBACK_FAILED",
            "error": repr(error), "acceptance": False}
        try:
            target = Path(args.output).resolve(strict=False)
            requested = read(args.input)
            state_root = Path(requested["stateRoot"]).resolve(strict=True)
            if not target.exists() and target != state_root and state_root not in target.parents:
                with target.open("x", encoding="utf-8", newline="\n") as stream:
                    json.dump(diagnostic, stream, indent=2)
                    stream.write("\n")
        except (OSError, ValueError, KeyError):
            pass
        print(json.dumps(diagnostic), file=sys.stderr)
        raise SystemExit(1)
