"""Read the original secretary E selection after a normal installed-product close.

Signed Python: STATE_ROOT OUTPUT JOURNAL before-cold|after-cold. Reads only the
candidate's closed state.sqlite; no credential, model, or project content table.
"""
import hashlib
import json
import sqlite3
import sys
from pathlib import Path
from urllib.parse import quote


def require(value, message):
    if not value:
        raise RuntimeError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def one(cursor, sql, values=()):
    rows = cursor.execute(sql, values).fetchall()
    require(len(rows) == 1, f"Original secretary row count differs: {len(rows)}")
    return rows[0]


def stable(reply):
    if reply["state"] == "UNSET":
        return {"state": "UNSET"}
    keys = ("state", "seatId", "incarnation", "generation", "revision",
            "instanceId", "model", "effort", "permissionTier", "seatState")
    return {key: reply[key] for key in keys}


def main():
    require(len(sys.argv) == 5, "STATE_ROOT OUTPUT JOURNAL PHASE required")
    root, output, journal_path = map(lambda value: Path(value).resolve(), sys.argv[1:4])
    phase = sys.argv[4]
    require(phase in ("before-cold", "after-cold") and root.is_dir() and
            output.parent == journal_path.parent and output != journal_path and
            not output.exists() and not output.is_relative_to(root),
            "Closed original readback path or phase invalid")
    journal = json.loads(journal_path.read_text(encoding="utf-8-sig"))
    require(journal["schema"] == "gogoke.37.private-m3-v14-secretary-read.v1" and
            journal["state"] == "RUNNING" and journal["stateRoot"] == str(root) and
            journal["evidenceDirectory"] == str(output.parent) and
            journal["acceptance"] is False and len(journal["launches"]) == len(journal["closes"]) and
            len(journal["launches"]) == (1 if phase == "before-cold" else 2),
            "Original installed candidate was not normally closed before immutable read")
    close = journal["closes"][-1]
    launch = journal["launches"][-1]
    require(close["pid"] == launch["pid"] and close["exitCode"] == 0 and
            close["forceKill"] is False and launch["sourceCommit"] == journal["sourceCommit"],
            "Normal close does not bind the actual candidate launch")
    operation = journal["operations"][-1]
    require(operation["phase"] == ("warm" if phase == "before-cold" else "cold") and
            operation["rawFrame"] == '{"schema":"gogoke.37.owner-configuration.v1","command":"secretary-configuration-read"}' and
            json.loads(operation["rawReply"]) == operation["reply"] and
            operation["reply"]["schema"] == "gogoke.37.secretary-configuration.v1",
            "Original USER frame/reply is not retained exactly")
    database = root / "state.sqlite"
    require(database.is_file() and not database.is_symlink(), "Original state.sqlite is unavailable")
    for suffix in ("-wal", "-shm"):
        sidecar = Path(str(database) + suffix)
        require(not sidecar.exists() or sidecar.stat().st_size == 0,
                "SQLite sidecar is not settled after normal close")
    before = digest(database)
    uri = "file:" + quote(str(database).replace("\\", "/"), safe="/:" ) + "?mode=ro&immutable=1"
    connection = sqlite3.connect(uri, uri=True)
    connection.row_factory = sqlite3.Row
    try:
        connection.execute("PRAGMA query_only=ON")
        designation = connection.execute(
            "SELECT seat_id,incarnation FROM gogoke_v37_seat_secretary WHERE singleton=1").fetchall()
        reply = operation["reply"]
        require(reply["state"] in ("UNSET", "DESIGNATED"), "This slice does not cover revoked secretary")
        if reply["state"] == "UNSET":
            require(len(designation) == 0 and reply["conversation"] == {"state": "NONE"},
                    "UNSET USER read disagrees with original global singleton")
        else:
            require(len(designation) == 1, "Expected one original global designation")
            selected = designation[0]
            seat = one(connection,
                "SELECT incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision "
                "FROM gogoke_v37_seats WHERE domain_id='global' AND seat_id=?",
                (selected["seat_id"],))
            settings_row = one(connection,
                "SELECT settings_json FROM gogoke_v37_seat_settings "
                "WHERE domain_id='global' AND seat_id=?", (selected["seat_id"],))
            settings = json.loads(settings_row["settings_json"])
            require(selected["seat_id"] == reply["seatId"] and
                    selected["incarnation"] == reply["incarnation"] == seat["incarnation"] and
                    seat["layer"] == "USER" and seat["parent_seat_id"] is None and
                    seat["kind"] == "LONG" and seat["state"] == reply["seatState"] and
                    str(seat["generation"]) == reply["generation"] and
                    str(seat["revision"]) == reply["revision"] and
                    (seat["instance_id"] or None) == reply["instanceId"] and
                    settings.get("model") == reply["model"] and
                    (settings.get("effort") or settings.get("reasoningEffort")) == reply["effort"] and
                    settings.get("permissionTier") == reply["permissionTier"],
                    "DESIGNATED USER read differs from original E singleton/seat/settings")
            instance = one(connection,
                "SELECT i.install_state,i.login_state,p.enabled,p.tombstoned,"
                "e.available_models_json,e.models_source,e.models_observed_at "
                "FROM gogoke_v37_instances i JOIN gogoke_v37_instance_profiles p "
                "ON p.instance_id=i.instance_id JOIN gogoke_v37_instance_evidence e "
                "ON e.instance_id=i.instance_id WHERE i.instance_id=?",
                (reply["instanceId"],))
            models = json.loads(instance["available_models_json"] or "null")
            require(instance["install_state"] == "INSTALLED" and
                    instance["login_state"] == "LOGGED_IN" and
                    instance["enabled"] == 1 and instance["tombstoned"] == 0 and
                    isinstance(models, list) and reply["model"] in models and
                    instance["models_source"] and instance["models_observed_at"] and
                    reply["permissionTier"] in
                    ("READ_ONLY", "NO_NETWORK", "ISOLATED_WRITE", "NETWORKED_WRITE"),
                    "Configured model/tier lacks the original enabled F source")
    finally:
        connection.close()
    require(digest(database) == before, "Measurement changed original SQLite bytes")
    proof = {"schema": "gogoke.37.private-m3-v14-secretary-sqlite.v1", "phase": phase,
             "caseId": journal["caseId"], "sourceCommit": journal["sourceCommit"],
             "candidateInstalledSha256": journal["candidateInstalledSha256"],
             "normalClose": close, "configuration": stable(operation["reply"]),
             "databaseSha256": before, "readerSha256": digest(Path(__file__).resolve()),
             "measurementPreservedDatabaseBytes": True, "acceptance": False}
    output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
