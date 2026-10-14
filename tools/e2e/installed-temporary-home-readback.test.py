"""Focused offline checks for the common reader's pre-launch selection boundary."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("installed-temporary-home-readback.py")
SPEC = importlib.util.spec_from_file_location("temporary_home_readback", SCRIPT)
READER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(READER)


class RootIdentity:
    @staticmethod
    def windows_file_identity(_root):
        return "root-fid"


class StartupSelection(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(":memory:")
        self.db.row_factory = sqlite3.Row
        self.db.executescript("""
            CREATE TABLE gogoke_v37_seat_secretary(singleton INTEGER,domain_id TEXT,seat_id TEXT,incarnation TEXT);
            CREATE TABLE gogoke_v37_seats(domain_id TEXT,seat_id TEXT,incarnation TEXT,instance_id TEXT,state TEXT,layer TEXT,kind TEXT,generation INTEGER);
            CREATE TABLE gogoke_v37_h_claim(domain_id TEXT,session_id TEXT,state TEXT,instance_id TEXT,home_id TEXT,binding_id TEXT,generation TEXT,revision INTEGER,stop_fact_id TEXT,process_operation_id TEXT);
            CREATE TABLE gogoke_v37_native_selection(domain_id TEXT,session_id TEXT,seat_id TEXT,seat_incarnation TEXT,seat_authorization_generation INTEGER,selected_instance_id TEXT);
            CREATE TABLE gogoke_v37_h_process_episode(domain_id TEXT,request_id TEXT,session_id TEXT,generation TEXT,old_generation TEXT,raw_hex TEXT,previous_revision INTEGER,result_revision INTEGER,process_operation_id TEXT,instance_id TEXT,home_id TEXT,binding_id TEXT,seat_id TEXT,seat_incarnation TEXT,phase TEXT,stop_fact_id TEXT,stop_request_id TEXT);
            CREATE TABLE gogoke_coordination_process_custody(operation_id TEXT,domain_id TEXT,profile_id TEXT,generation TEXT,state TEXT,stop_proof_hash TEXT);
            CREATE TABLE gogoke_v37_h_operation(domain_id TEXT,request_id TEXT,session_id TEXT,operation TEXT,raw_hex TEXT,status TEXT,previous_revision INTEGER,revision INTEGER);
        """)

    def tearDown(self):
        self.db.close()

    def test_none_requires_no_designation(self):
        self.assertEqual(READER.startup_baseline(self.db, RootIdentity(), Path("."), {}),
                         {"state": "NONE", "domainId": "global"})
        self.db.execute("INSERT INTO gogoke_v37_seat_secretary VALUES(1,'global','seat','inc')")
        self.assertEqual(READER.startup_baseline(self.db, RootIdentity(), Path("."), {})["state"],
                         "BLOCKED")

    def test_stopped_source_yields_exact_resume_and_pending_blocks(self):
        self.db.executescript("""
            INSERT INTO gogoke_v37_seat_secretary VALUES(1,'global','seat','inc');
            INSERT INTO gogoke_v37_seats VALUES('global','seat','inc','inst','BUSY','USER','LONG',24);
            INSERT INTO gogoke_v37_h_claim VALUES('global','session','STOPPED','inst','home-abc','binding-abc','24',10,'sha256:0000000000000000000000000000000000000000000000000000000000000000','process');
            INSERT INTO gogoke_v37_native_selection VALUES('global','session','seat','inc',24,'inst');
            INSERT INTO gogoke_v37_h_process_episode VALUES('global','resume-old','session','24','23','726177',3,4,'process','inst','home-abc','binding-abc','seat','inc','STOPPED','sha256:0000000000000000000000000000000000000000000000000000000000000000','stop-old');
            INSERT INTO gogoke_coordination_process_custody VALUES('process','global','inst','24','STOPPED','sha256:0000000000000000000000000000000000000000000000000000000000000000');
            INSERT INTO gogoke_v37_h_operation VALUES('global','stop-old','session','stop','726177','APPLIED',4,5);
        """)
        home = {"home_id": "home-abc", "domain_id": "global", "instance_id": "inst",
                "owner_id": "session", "generation": "24", "kind": "SESSION",
                "state": "ACTIVE", "revision": "1"}
        expected = "resident-resume-" + hashlib.sha256(
            b"secretary-resume\nroot-fid\nsession\n24\n10").hexdigest()[:40]
        result = READER.startup_baseline(self.db, RootIdentity(), Path("."),
                                         {"home-abc": home})
        self.assertEqual(result["state"], "RESUME_EXPECTED")
        self.assertEqual(result["expectedCreateRequestId"], expected)
        self.assertEqual(result["expectedGeneration"], "25")
        closed = dict(home, state="CLOSED", revision="2")
        no_create = READER.startup_baseline(self.db, RootIdentity(), Path("."),
            {"home-abc": closed})
        self.assertEqual(no_create["state"], "CLOSED_PREDECESSOR")
        self.assertEqual(no_create["expectedCreateRequestId"], expected)
        self.assertEqual(READER.startup_baseline(self.db, RootIdentity(), Path("."),
            {"home-abc": dict(home, state="UNKNOWN")})["state"], "BLOCKED")
        self.assertEqual(READER.startup_baseline(self.db, RootIdentity(), Path("."),
            {})["state"], "BLOCKED")
        self.assertEqual(READER.startup_baseline(self.db, RootIdentity(), Path("."),
            {"home-abc": dict(home, state="CLOSED", revision="1")})["state"], "BLOCKED")
        self.db.execute("INSERT INTO gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,phase) VALUES('global','pending','session','25','24','UNKNOWN')")
        self.assertEqual(READER.startup_baseline(self.db, RootIdentity(), Path("."),
            {"home-abc": closed})["state"], "BLOCKED")


class RepairSelection(unittest.TestCase):
    def test_requires_exact_active_one_and_pinned_capture(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "capture.json"
            path.write_text("{}", encoding="utf-8")
            provenance = {"path": str(path), "sha256": hashlib.sha256(b"{}").hexdigest()}
            home = {"home_id": "home-a", "instance_id": "inst", "domain_id": "global",
                    "owner_id": "session", "generation": "24", "state": "ACTIVE",
                    "revision": "1"}
            repair = {"homeId": "home-a", "instanceId": "inst", "domainId": "global",
                      "sessionId": "session", "generation": "24", "expectedRevision": "1",
                      "provenance": provenance}
            self.assertEqual(READER.declared_repairs({"authorizedRepairs": [repair]}, [home]),
                             [repair])
            changed = dict(home, state="CLOSED")
            with self.assertRaises(RuntimeError):
                READER.declared_repairs({"authorizedRepairs": [repair]}, [changed])
            path.write_text("{ }", encoding="utf-8")
            with self.assertRaises(RuntimeError):
                READER.declared_repairs({"authorizedRepairs": [repair]}, [home])

    def test_fifteen_explicit_unique_prior_homes_have_no_arbitrary_cap(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "capture.json"
            path.write_text("{}", encoding="utf-8")
            provenance = {"path": str(path), "sha256": hashlib.sha256(b"{}").hexdigest()}
            homes, repairs = [], []
            for number in range(15):
                home_id = f"home-{number}"
                homes.append({"home_id": home_id, "instance_id": "inst", "domain_id": "global",
                    "owner_id": "session", "generation": str(number + 1),
                    "state": "ACTIVE", "revision": "1"})
                repairs.append({"homeId": home_id, "instanceId": "inst", "domainId": "global",
                    "sessionId": "session", "generation": str(number + 1),
                    "expectedRevision": "1", "provenance": provenance})
            self.assertEqual(READER.declared_repairs({"authorizedRepairs": repairs}, homes), repairs)
            with self.assertRaises(RuntimeError):
                READER.declared_repairs({"authorizedRepairs": repairs + [repairs[0]]}, homes)


class RepairRefusal(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(":memory:")
        self.db.row_factory = sqlite3.Row
        self.db.execute("CREATE TABLE gogoke_v37_instance_operations(request_id TEXT,phase TEXT)")
        self.before = {"home_id": "home-24", "instance_id": "inst", "domain_id": "global",
            "owner_id": "session", "generation": "24", "directory_ref": "temp-home-original",
            "directory_identity": "volume:original/file:original", "kind": "SESSION",
            "state": "ACTIVE", "revision": "1"}
        self.request = {"schema": "gogoke.37.operations.v1", "family": "K-INSTANCE",
            "operation": "temporary-home-close", "requestId": "close-24", "domainId": "global",
            "targetId": "inst", "expectedRevision": "1",
            "payload": {"sessionId": "session", "historicalHomeId": "home-24"}}
        raw = json.dumps(self.request, separators=(",", ":"))
        self.repair = {"caseId": "repair-24", "homeId": "home-24", "instanceId": "inst",
            "domainId": "global", "sessionId": "session", "generation": "24",
            "closeDispatchState": "RECEIPT_RECORDED", "closeRequestId": "close-24",
            "closeRawFrame": raw, "closeRawSha256": hashlib.sha256(raw.encode()).hexdigest(),
            "closeReceipt": {"schema": self.request["schema"], "family": "K-INSTANCE",
                "operation": "temporary-home-close", "requestId": "close-24",
                "targetId": "inst", "status": "DENIED", "previousRevision": "1",
                "revision": "1", "result": {"reason": "original source refused"}}}

    def tearDown(self):
        self.db.close()

    def test_known_refusal_requires_unchanged_full_home_and_no_f_journal(self):
        for status in ("DENIED", "CONFLICT", "STALE"):
            repair = dict(self.repair, closeReceipt=dict(self.repair["closeReceipt"], status=status))
            result = READER.verify_repair_refusal(self.db, repair, self.before, dict(self.before))
            self.assertEqual((result["status"], result["state"], result["revision"]),
                             (status, "ACTIVE", "1"))
        for changed in (dict(self.before, state="CLOSED", revision="2"),
                        dict(self.before, directory_identity="volume:other/file:other")):
            with self.assertRaises(RuntimeError):
                READER.verify_repair_refusal(self.db, self.repair, self.before, changed)
        self.db.execute("INSERT INTO gogoke_v37_instance_operations VALUES('close-24','APPLIED')")
        with self.assertRaises(RuntimeError):
            READER.verify_repair_refusal(self.db, self.repair, self.before, self.before)

    def test_bad_tuple_unknown_and_unsent_are_not_known_refusals(self):
        for mutation in (
            {"closeReceipt": dict(self.repair["closeReceipt"], targetId="foreign")},
            {"closeReceipt": dict(self.repair["closeReceipt"], status="UNKNOWN")},
            {"closeReceipt": dict(self.repair["closeReceipt"], previousRevision="0")},
            {"closeReceipt": dict(self.repair["closeReceipt"], result={"reason": ""})},
            {"closeDispatchState": "NOT_DISPATCHED"},
        ):
            with self.subTest(mutation=mutation), self.assertRaises(RuntimeError):
                READER.verify_repair_refusal(self.db, dict(self.repair, **mutation),
                                             self.before, self.before)
        altered = dict(self.repair, closeRawFrame=self.repair["closeRawFrame"].replace(
            '"home-24"', '"home-25"'))
        with self.assertRaises(RuntimeError):
            READER.verify_repair_refusal(self.db, altered, self.before, self.before)


class NoStartupHome(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(":memory:")
        self.db.row_factory = sqlite3.Row
        self.db.executescript("""
            CREATE TABLE gogoke_v37_instance_operations(request_id TEXT);
            CREATE TABLE gogoke_v37_h_operation(domain_id TEXT,request_id TEXT);
            CREATE TABLE gogoke_v37_h_process_episode(domain_id TEXT,request_id TEXT);
        """)
        self.plan = {"state": "CLOSED_PREDECESSOR", "domainId": "global",
            "seatId": "secretary-seat", "seatIncarnation": "incarnation",
            "instanceId": "inst", "sessionId": "resident-session",
            "oldGeneration": "25", "oldRevision": "12", "expectedGeneration": "26",
            "expectedCreateRequestId": "resident-resume-original", "baselineHomeId": "home-25"}
        self.home = {"home_id": "home-25", "instance_id": "inst", "domain_id": "global",
            "owner_id": "resident-session", "generation": "25", "kind": "SESSION",
            "directory_ref": "temp-home-original", "directory_identity": "volume:original/file:original",
            "state": "CLOSED", "revision": "2"}
        self.launch = {"startup": self.plan, "homes": [self.home]}
        refusal = {"schema": "gogoke.37.operations.v1", "family": "K-SESSION",
            "operation": "resume", "requestId": self.plan["expectedCreateRequestId"],
            "targetId": self.plan["sessionId"], "status": "CONFLICT",
            "previousRevision": "12", "revision": "12", "result": {}}
        self.reply = {"schema": "gogoke.37.secretary-configuration.v1",
            "state": "DESIGNATED", "seatState": "BUSY", "seatId": "secretary-seat",
            "incarnation": "incarnation", "instanceId": "inst",
            "generation": "21", "revision": "21",  # Outer E is distinct from current H.
            "conversation": {"state": "FOUND", "sessionId": "resident-session",
                "generation": "25", "revision": "12", "claimState": "STOPPED",
                "runtimeAvailable": False, "stoppedFact": True},
            "startupReason": "Original Secretary resume result: " +
                json.dumps(refusal, separators=(",", ":"))}
        frame = {"schema": "gogoke.37.owner-configuration.v1",
                 "command": "secretary-configuration-read"}
        self.attempt = dict(self.plan, caseId="startup-no-create-0",
            sourceKind="NATIVE_STARTUP_NO_CREATE",
            bootstrap={"frame": frame, "rawFrame": json.dumps(frame, separators=(",", ":")),
                "reply": self.reply, "rawReply": json.dumps(self.reply, separators=(",", ":")),
                "startedAt": "2026-10-14T15:45:57.008Z",
                "finishedAt": "2026-10-14T15:46:00.435Z"})

    def tearDown(self):
        self.db.close()

    def verify(self, attempt=None, launch=None, current=None, home=None):
        return READER.verify_no_startup_home(self.db, attempt or self.attempt,
            launch or self.launch, current or self.plan, home or self.home)

    def test_exact_closed_predecessor_conflict_has_no_new_f_or_h_and_reports_no_create(self):
        result = self.verify()
        self.assertEqual(result, {"caseId": "startup-no-create-0", "domainId": "global",
            "sessionId": "resident-session", "instanceId": "inst", "homeId": "home-25",
            "originalCreateRequestId": "resident-resume-original", "state": "NOT_CREATED",
            "generation": "25", "revision": "12", "status": "CONFLICT"})
        create_id = "homeprep-" + hashlib.sha256(
            b"global\nresident-resume-original").hexdigest()[:40]
        self.db.execute("INSERT INTO gogoke_v37_instance_operations VALUES(?)", (create_id,))
        with self.assertRaises(RuntimeError):
            self.verify()

    def test_changed_or_missing_source_and_fake_refusal_fail_closed(self):
        for changed_home in (dict(self.home, state="ACTIVE", revision="1"),
                             dict(self.home, directory_identity="volume:other/file:other")):
            with self.subTest(home=changed_home), self.assertRaises(RuntimeError):
                self.verify(home=changed_home)
        with self.assertRaises(RuntimeError):
            self.verify(current=dict(self.plan, oldRevision="13"))
        with self.assertRaises(RuntimeError):
            self.verify(launch={"startup": dict(self.plan, state="RESUME_EXPECTED"),
                "homes": [self.home]})
        for status in ("UNKNOWN", "APPLIED"):
            changed = json.loads(json.dumps(self.attempt))
            reason = json.loads(changed["bootstrap"]["reply"]["startupReason"].split(": ", 1)[1])
            reason["status"] = status
            changed["bootstrap"]["reply"]["startupReason"] = \
                "Original Secretary resume result: " + json.dumps(reason, separators=(",", ":"))
            changed["bootstrap"]["rawReply"] = json.dumps(changed["bootstrap"]["reply"],
                separators=(",", ":"))
            with self.subTest(status=status), self.assertRaises(RuntimeError):
                self.verify(attempt=changed)
        self.db.execute("INSERT INTO gogoke_v37_h_process_episode VALUES('global','resident-resume-original')")
        with self.assertRaises(RuntimeError):
            self.verify()


if __name__ == "__main__":
    unittest.main()
