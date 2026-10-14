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
                "owner_id": "session", "generation": "24"}
        expected = "resident-resume-" + hashlib.sha256(
            b"secretary-resume\nroot-fid\nsession\n24\n10").hexdigest()[:40]
        result = READER.startup_baseline(self.db, RootIdentity(), Path("."),
                                         {"home-abc": home})
        self.assertEqual(result["state"], "RESUME_EXPECTED")
        self.assertEqual(result["expectedCreateRequestId"], expected)
        self.assertEqual(result["expectedGeneration"], "25")
        self.db.execute("INSERT INTO gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,phase) VALUES('global','pending','session','25','24','UNKNOWN')")
        self.assertEqual(READER.startup_baseline(self.db, RootIdentity(), Path("."),
            {"home-abc": home})["state"], "BLOCKED")


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


if __name__ == "__main__":
    unittest.main()
