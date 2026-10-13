"""Shared, read-only evidence for an installed native H session.

Callers supply the original User journal and choose the observed binding
schema explicitly. This module never opens credential files or copies SQLite.
"""

import ctypes
import argparse
import hashlib
import json
import os
import re
import sqlite3
from contextlib import contextmanager
from pathlib import Path
from ctypes import wintypes


def require(value, message):
    if not value:
        raise RuntimeError(message)


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


_LOADED_MODULE_SHA256 = sha256(__file__)


def loaded_source():
    require(sha256(__file__) == _LOADED_MODULE_SHA256, "Loaded native instrument source changed")
    return {"path": str(Path(__file__).resolve()), "sha256": _LOADED_MODULE_SHA256}


def exact_one(db, sql, args=()):
    found = [dict(row) for row in db.execute(sql, args)]
    require(len(found) == 1, f"Expected one native row, found {len(found)}")
    return found[0]


def canonical_local_path(path, *, must_exist=True):
    """Normalize ordinary/extended drive paths without accepting UNC or links."""
    require(os.name == "nt", "Windows physical path evidence required")
    source = Path(path)
    value = str(source.resolve(strict=must_exist))
    require(re.fullmatch(r"(?:\\\\\?\\)?[A-Za-z]:\\.+", value) is not None,
            "Absolute local-drive path required")
    current = source
    while True:
        if current.exists():
            require(not current.is_symlink() and not (current.stat(follow_symlinks=False).st_file_attributes & 0x400),
                    "Reparse source rejected")
        if current == current.parent:
            break
        current = current.parent
    return Path(value if value.startswith("\\\\?\\") else "\\\\?\\" + value)


def contained_physical(root, child, *, child_must_exist=True):
    parent = canonical_local_path(root)
    target = canonical_local_path(child, must_exist=child_must_exist)
    require(target != parent and parent in target.parents,
            "Physical child outside the pinned root")
    return target


class _FileIdInfo(ctypes.Structure):
    _fields_ = [("volume", ctypes.c_ulonglong), ("file_id", ctypes.c_ubyte * 16)]


def windows_file_identity(path):
    """FileIdInfo of the opened object, with OPEN_REPARSE_POINT."""
    target = canonical_local_path(path)
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                   wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    kernel.CreateFileW.restype = wintypes.HANDLE
    kernel.GetFileInformationByHandleEx.argtypes = [wintypes.HANDLE, ctypes.c_int,
                                                     wintypes.LPVOID, wintypes.DWORD]
    kernel.GetFileInformationByHandleEx.restype = wintypes.BOOL
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.CloseHandle.restype = wintypes.BOOL
    handle = kernel.CreateFileW(str(target), 0x80, 0x7, None, 3, 0x02200000, None)
    require(handle not in (None, wintypes.HANDLE(-1).value), "Physical identity handle unavailable")
    try:
        info = _FileIdInfo()
        require(kernel.GetFileInformationByHandleEx(handle, 18, ctypes.byref(info), ctypes.sizeof(info)),
                "FileIdInfo unavailable")
        return f"volume:{info.volume:016x}/file:{bytes(info.file_id).hex()}"
    finally:
        kernel.CloseHandle(handle)


def _files(root):
    db = Path(root) / "state.sqlite"
    names = {"database": db, "wal": Path(str(db) + "-wal"),
             "shm": Path(str(db) + "-shm"), "journal": Path(str(db) + "-journal")}
    return {name: {"exists": path.exists(), "length": path.stat().st_size if path.exists() else 0,
                   "sha256": sha256(path) if path.exists() else None}
            for name, path in names.items()}


def observe_root_identity(root):
    """Live-safe directory metadata only; does not open the database."""
    loaded_source()
    source = canonical_local_path(root)
    physical = windows_file_identity(source)
    stat = source.stat()
    identity = {"device": str(stat.st_dev), "inode": str(stat.st_ino)}
    require(windows_file_identity(source) == physical and
            {"device": str(source.stat().st_dev), "inode": str(source.stat().st_ino)} == identity,
            "Root identity changed during metadata observation")
    return {"rootIdentity": identity, "rootPhysicalIdentity": physical,
            "databaseOpened": False, "credentialReads": False, "loadedSource": loaded_source()}


@contextmanager
def closed_database(root):
    """Yield (query-only connection, provenance); check original bytes on exit."""
    loaded_source()
    source = Path(str(root)[4:] if str(root).startswith("\\\\?\\") else root).resolve(strict=True)
    canonical_local_path(source)
    before = _files(source)
    require(before["database"]["exists"] and
            all(not before[name]["exists"] for name in ("wal", "shm", "journal")),
            "Closed state.sqlite with no sidecars required")
    identity = windows_file_identity(source)
    root_stat = {"device": str(source.stat().st_dev), "inode": str(source.stat().st_ino)}
    database = source / "state.sqlite"
    report = {"rootPhysicalIdentity": identity, "filesBefore": before,
              "rootIdentity": root_stat,
              "loadedModuleSha256": _LOADED_MODULE_SHA256, "immutableMode": True,
              "databaseWrites": False, "authContentOpened": False, "copiedDatabase": False}
    db = sqlite3.connect(database.as_uri() + "?mode=ro&immutable=1", uri=True)
    try:
        db.row_factory = sqlite3.Row
        db.execute("PRAGMA query_only=ON")
        yield db, report
    finally:
        db.close()
        after = _files(source)
        require(after == before and windows_file_identity(source) == identity and
                {"device": str(source.stat().st_dev), "inode": str(source.stat().st_ino)} == root_stat,
                "Native readback changed database, sidecars, or root identity")
        report["filesAfter"] = after
        loaded_source()


def verify_binding(db, *, domain, session_id, seat_id, seat_incarnation,
                   instance_id, mode):
    """Never infer legacy from a missing V2 row or conflate auth/H generations."""
    source = loaded_source()
    args = (domain, session_id)
    if mode == "native_v2":
        columns = ("domain_id", "session_id", "seat_id", "seat_incarnation",
                   "seat_authorization_generation", "selected_instance_id")
        selection = exact_one(db, "SELECT " + ",".join(columns) +
                              " FROM gogoke_v37_native_selection WHERE domain_id=? AND session_id=?", args)
        binding = exact_one(db, "SELECT " + ",".join(columns) + ",provenance " +
                            "FROM gogoke_v37_session_binding_v2 WHERE domain_id=? AND session_id=?", args)
        require(binding["provenance"] == "NATIVE_V2" and
                all(selection[key] == binding[key] for key in columns) and
                not list(db.execute("SELECT 1 FROM gogoke_v37_h_seat_binding "
                                    "WHERE domain_id=? AND session_id=?", args)),
                "Original native selection/V2 or legacy exclusivity differs")
        result = {"binding": binding, "selection": selection, "mode": mode}
    elif mode == "legacy":
        binding = exact_one(db, "SELECT seat_id,seat_incarnation,generation "
                           "FROM gogoke_v37_h_seat_binding WHERE domain_id=? AND session_id=?", args)
        require(not list(db.execute("SELECT 1 FROM gogoke_v37_native_selection "
                                    "WHERE domain_id=? AND session_id=?", args)),
                "Original legacy H unexpectedly has native selection")
        immutable = [dict(row) for row in db.execute(
            "SELECT * FROM gogoke_v37_session_binding_v2 WHERE domain_id=? AND session_id=?", args)]
        require(len(immutable) <= 1 and all(row["provenance"] == "LEGACY_V1" and
                row["seat_id"] == seat_id and row["seat_incarnation"] == seat_incarnation and
                row["selected_instance_id"] == instance_id for row in immutable),
                "Original legacy initial relationship differs")
        result = {"binding": binding, "selection": None, "mode": mode,
                  "immutableLegacyInitialRelationship": immutable}
    else:
        raise RuntimeError("Explicit native_v2 or legacy binding mode required")
    binding = result["binding"]
    require(binding["seat_id"] == seat_id and binding["seat_incarnation"] == seat_incarnation and
            (mode == "legacy" or binding["selected_instance_id"] == instance_id),
            "Original H seat/instance binding differs")
    result["loadedSource"] = source
    return result


def verify_stop_tuple(db, *, domain, session_id, generation, process_operation_id,
                      stop_fact_id, instance_id, seat_id, seat_incarnation,
                      stop_request_id, binding_mode, released, old_generation=None):
    """Original H physical StopFact; release changes claim state, not episode."""
    require(re.fullmatch(r"sha256:[a-f0-9]{64}", stop_fact_id or "") is not None,
            "Nonempty original StopFact required")
    binding_evidence = verify_binding(db, domain=domain, session_id=session_id,
                                      seat_id=seat_id, seat_incarnation=seat_incarnation,
                                      instance_id=instance_id, mode=binding_mode)
    if binding_mode == "legacy":
        require(str(binding_evidence["binding"]["generation"]) == str(generation),
                "Original legacy H binding generation differs")
    claim = exact_one(db, "SELECT * FROM gogoke_v37_h_claim "
                      "WHERE domain_id=? AND session_id=?", (domain, session_id))
    episode = exact_one(db, "SELECT * FROM gogoke_v37_h_process_episode "
                        "WHERE domain_id=? AND session_id=? AND process_operation_id=?",
                        (domain, session_id, process_operation_id))
    custody = exact_one(db, "SELECT * FROM gogoke_coordination_process_custody "
                        "WHERE domain_id=? AND operation_id=?", (domain, process_operation_id))
    require(claim["state"] == ("RELEASED" if released else "STOPPED") and
            claim["generation"] == episode["generation"] == custody["generation"] == str(generation) and
            claim["process_operation_id"] == episode["process_operation_id"] == custody["operation_id"] == process_operation_id and
            claim["stop_fact_id"] == episode["stop_fact_id"] == custody["stop_proof_hash"] == stop_fact_id and
            episode["phase"] == custody["state"] == "STOPPED" and
            claim["instance_id"] == episode["instance_id"] == custody["profile_id"] == instance_id and
            episode["seat_id"] == seat_id and episode["seat_incarnation"] == seat_incarnation and
            episode["stop_request_id"] == stop_request_id,
            "Original claim/episode/custody StopFact tuple differs")
    if old_generation is not None:
        require(episode["old_generation"] == str(old_generation) and
                int(generation) == int(old_generation) + 1,
                "Current Secretary E is not the original next generation")
    if not released:
        releases = [dict(row) for row in db.execute("SELECT raw_hex FROM gogoke_v37_h_operation "
                   "WHERE domain_id=? AND session_id=? AND operation='admission-release'", (domain, session_id))]
        require(not any(str(json.loads(bytes.fromhex(row["raw_hex"]))["payload"]["generation"]) ==
                        str(generation) for row in releases), "Current stopped H was released")
    return {"claim": claim, "episode": episode, "custody": custody,
            "binding": binding_evidence["binding"], "bindingMode": binding_mode,
            "bindingEvidence": binding_evidence,
            "stopFactId": stop_fact_id}


def verify_original_h_stop(db, *, journal, domain, session_id, generation,
                           request_ids, tree_id, repository_id, seat_id,
                           seat_incarnation, instance_id, process_operation_id,
                           stop_fact_id, binding_mode):
    """Ordinary H: exact five original raw requests/CAS plus physical stop."""
    names = ("admission-reserve", "admission-commit", "open", "stop", "admission-release")
    require(set(request_ids) == set(names), "Five original H request IDs required")
    receipts = {}
    for name in names:
        request_id = request_ids[name]
        matches = [entry for entry in journal["operations"]
                   if entry.get("request", {}).get("requestId") == request_id]
        require(len(matches) == 1, "Original H request not unique")
        entry = matches[0]
        request, receipt = entry["request"], entry["receipt"]
        raw = entry["rawFrame"].encode("utf-8")
        durable = exact_one(db, "SELECT * FROM gogoke_v37_h_operation "
                            "WHERE domain_id=? AND request_id=?", (domain, request_id))
        require(json.loads(raw) == request and request["operation"] == name and
                request["family"] == "K-SESSION" and request["domainId"] == domain and
                request["targetId"] == session_id and
                request["payload"]["generation"] == str(generation) and
                receipt["requestId"] == request_id and receipt["operation"] == name and
                receipt["targetId"] == session_id and receipt["status"] == "APPLIED" and
                durable["raw_hex"] == raw.hex() and durable["operation"] == name and
                durable["session_id"] == session_id and durable["status"] == "APPLIED" and
                str(durable["previous_revision"]) == request["expectedRevision"] == receipt["previousRevision"] and
                str(durable["revision"]) == receipt["revision"],
                "Original H raw request/CAS/receipt differs")
        receipts[name] = (request, receipt)
    opened, stopped = receipts["open"], receipts["stop"]
    released = receipts["admission-release"]
    require(opened[0]["payload"]["seatId"] == seat_id and
            opened[0]["payload"]["repositoryId"] == repository_id and
            opened[0]["payload"]["worktreeId"] == tree_id and
            stopped[1]["result"]["stopFact"] == stop_fact_id and
            released[0]["expectedRevision"] == stopped[1]["revision"],
            "Original open/stop/release relation differs")
    opens = []
    for row in db.execute("SELECT domain_id,session_id,raw_hex FROM gogoke_v37_h_operation WHERE operation='open'"):
        request = json.loads(bytes.fromhex(row["raw_hex"]))
        if request.get("payload", {}).get("worktreeId") == tree_id:
            opens.append((row["domain_id"], row["session_id"]))
    require(opens == [(domain, session_id)], "Original F has another H open")
    facts = verify_stop_tuple(db, domain=domain, session_id=session_id,
                              generation=generation, process_operation_id=process_operation_id,
                              stop_fact_id=stop_fact_id, instance_id=instance_id,
                              seat_id=seat_id, seat_incarnation=seat_incarnation,
                              stop_request_id=request_ids["stop"], binding_mode=binding_mode,
                              released=True)
    require(str(facts["claim"]["revision"]) == released[1]["revision"],
            "Original final H claim CAS revision differs from release")
    require(bytes.fromhex(facts["episode"]["raw_hex"]) ==
            next(entry["rawFrame"].encode("utf-8") for entry in journal["operations"]
                 if entry.get("request", {}).get("requestId") == request_ids["open"]),
            "Original physical E open bytes differ")
    return {"sessionId": session_id, "generation": str(generation),
            "processOperationId": process_operation_id, "stopFactId": stop_fact_id,
            "claimState": facts["claim"]["state"], "episodePhase": facts["episode"]["phase"],
            "custodyState": facts["custody"]["state"], "bindingMode": binding_mode,
            "binding": facts["binding"], "bindingEvidence": facts["bindingEvidence"],
            "hRequestIds": [request_ids[name] for name in names],
            "hOpenCountForTree": len(opens)}


def _pinned_json(reference):
    require(isinstance(reference, dict) and isinstance(reference.get("path"), str) and
            re.fullmatch(r"[a-f0-9]{64}", reference.get("sha256", "")) is not None and
            sha256(reference["path"]) == reference["sha256"],
            "Original JSON source pin differs")
    return json.loads(Path(reference["path"]).read_text(encoding="utf-8-sig"))


def run_pinned_input(input_path, output_path):
    """One CLI entry; positive/negative originals retain their exact source refs."""
    source = Path(input_path).resolve(strict=True)
    output = Path(output_path).resolve(strict=False)
    require(not output.exists() and source != output, "Fresh readback output required")
    config = json.loads(source.read_text(encoding="utf-8-sig"))
    require(config.get("schema") == "gogoke.37.installed-native-input.v1" and
            config.get("acceptance") is False and config.get("moduleSha256") == sha256(__file__) and
            config.get("bindingMode") in ("native_v2", "legacy") and
            re.fullmatch(r"[a-f0-9]{64}", config.get("expectedDatabaseSha256", "")) is not None,
            "Installed native input/module/schema differs")
    root = Path(config["stateRoot"]).resolve(strict=True)
    canonical_local_path(root)
    require(output != root and root not in output.parents,
            "Readback output must stay outside installed state")
    if config.get("kind") == "root_identity":
        prior = _pinned_json(config["worktreeProof"])
        measured = observe_root_identity(root)
        require(measured["rootIdentity"] == prior["rootIdentity"],
                "Original physical root changed before operation")
        report = {"schema": "gogoke.37.installed-root-identity.v1", "acceptance": False,
                  "state": "ORIGINAL_ROOT_IDENTITY_MEASURED", "inputSha256": sha256(source),
                  "sourceRefs": {"worktreeProof": config["worktreeProof"]}, **measured}
        with output.open("x", encoding="utf-8", newline="\n") as stream:
            json.dump(report, stream, indent=2); stream.write("\n")
        return report
    with closed_database(root) as (db, measured):
        require(measured["filesBefore"]["database"]["sha256"] == config["expectedDatabaseSha256"],
                "Closed source database differs")
        if config.get("kind") == "ordinary":
            journal = _pinned_json(config["journal"])
            prior = _pinned_json(config["worktreeProof"])
            case = journal["stopWorktree"]
            require(case["schema"] == "gogoke.37.m2-stop-worktree.v1" and
                    case["sourceCommit"] == journal["sourceCommit"] and
                    case["stateRoot"] == config["stateRoot"] and
                    prior["schema"] == "gogoke.37.v07-original-stopped-source-native-v2.v1" and
                    prior["acceptance"] is False and
                    prior["originalJournal"]["sha256"] == config["journal"]["sha256"] and
                    prior["rootIdentity"] == measured["rootIdentity"] and
                    prior["worktree"]["worktreeId"] == case["worktreeId"] and
                    prior["worktree"]["stopFactId"] == case["session"]["stopFactId"] and
                    prior["worktree"]["session"]["sessionId"] == case["session"]["id"],
                    "Original stopped F/H source differs")
            session = case["session"]
            seat = exact_one(db, "SELECT incarnation FROM gogoke_v37_seats "
                             "WHERE domain_id=? AND seat_id=?", (case["domainId"], case["seatId"]))
            claim = exact_one(db, "SELECT process_operation_id FROM gogoke_v37_h_claim "
                              "WHERE domain_id=? AND session_id=?", (case["domainId"], session["id"]))
            request_ids = {"admission-reserve": session["reserveRequestId"],
                           "admission-commit": session["commitRequestId"],
                           "open": session["openRequestId"], "stop": session["stopRequestId"],
                           "admission-release": session["releaseRequestId"]}
            facts = verify_original_h_stop(
                db, journal=journal, domain=case["domainId"], session_id=session["id"],
                generation=session["generation"], request_ids=request_ids,
                tree_id=case["worktreeId"], repository_id=case["repositoryId"],
                seat_id=case["seatId"], seat_incarnation=seat["incarnation"],
                instance_id=case["instanceId"], process_operation_id=claim["process_operation_id"],
                stop_fact_id=session["stopFactId"], binding_mode=config["bindingMode"])
            if config["bindingMode"] == "native_v2":
                original_session = prior["worktree"]["session"]
                require(facts["bindingEvidence"]["binding"] == original_session["nativeBinding"] and
                        facts["bindingEvidence"]["selection"] == original_session["nativeSelection"],
                        "Original immutable native relationship changed since stopped source")
            tree = exact_one(db, "SELECT * FROM gogoke_v37_worktrees WHERE worktree_id=?",
                             (case["worktreeId"],))
            lifecycle = exact_one(db, "SELECT * FROM gogoke_v37_worktree_lifecycle WHERE worktree_id=?",
                                  (case["worktreeId"],))
            physical = contained_physical(root / "v37-worktrees", tree["worktree_path"])
            require(tree["state"] == lifecycle["state"] == "REGISTERED" and
                    tree["domain_id"] == case["domainId"] and
                    tree["repository_id"] == case["repositoryId"] and
                    tree["seat_id"] == case["seatId"] and
                    tree["seat_incarnation"] == seat["incarnation"] and
                    tree["instance_id"] == case["instanceId"] and
                    windows_file_identity(physical) == tree["worktree_identity"] ==
                    prior["worktree"]["worktreeIdentity"] and
                    tree["git_pointer_hash"] == prior["worktree"]["gitPointerHash"] and
                    windows_file_identity(physical / ".git") == tree["git_pointer_identity"] ==
                    prior["worktree"]["gitPointerIdentity"] and
                    tree["common_identity"] == prior["worktree"]["commonIdentity"] and
                    tree["baseline_commit"] == prior["worktree"]["baselineCommit"] and
                    "sha256:" + sha256(physical / ".git") == tree["git_pointer_hash"],
                    "Original F physical identity/pointer/registration differs")
            result = {"kind": "ordinary", "caseId": journal["caseId"], "h": facts,
                      "worktree": {"worktreeId": tree["worktree_id"],
                                   "physicalIdentity": tree["worktree_identity"],
                                   "state": lifecycle["state"]}}
        elif config.get("kind") == "secretary":
            original = _pinned_json(config["originalSource"])
            spec = config["stop"]
            original_h = original.get("secretary", {}).get("current", original)
            original_session = original_h.get("session_id", original_h.get("sessionId"))
            original_instance = original_h.get("instance_id", original_h.get("instanceId"))
            require(original_session == spec["sessionId"] and
                    original_instance == spec["instanceId"] and
                    str(original_h["generation"]) == str(spec["oldGeneration"]) and
                    config["expectedRootIdentity"] == original["rootIdentity"] == measured["rootIdentity"],
                    "Original Secretary source identity differs")
            facts = verify_stop_tuple(
                db, domain=spec["domainId"], session_id=spec["sessionId"],
                generation=spec["generation"], process_operation_id=spec["processOperationId"],
                stop_fact_id=spec["stopFactId"], instance_id=spec["instanceId"],
                seat_id=spec["seatId"], seat_incarnation=spec["seatIncarnation"],
                stop_request_id=spec["stopRequestId"], binding_mode=config["bindingMode"],
                released=False, old_generation=spec["oldGeneration"])
            result = {"kind": "secretary", "h": facts}
        else:
            raise RuntimeError("Explicit ordinary or secretary readback kind required")
    require(measured["filesAfter"]["database"]["sha256"] == config["expectedDatabaseSha256"],
            "Closed database changed during installed readback")
    report = {"schema": "gogoke.37.installed-native-report.v1",
              "state": "INSTALLED_NATIVE_READBACK_REVIEW_REQUIRED", "acceptance": False,
              "inputSha256": sha256(source), "sourceRefs": {key: config[key] for key in
              ("journal", "worktreeProof", "originalSource") if key in config},
              "bindingMode": config["bindingMode"], "native": measured, **result}
    with output.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True)
    parser.add_argument("--output", required=True)
    options = parser.parse_args()
    print(run_pinned_input(options.input, options.output)["state"])
