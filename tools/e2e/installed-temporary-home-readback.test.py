"""Focused offline checks for the common reader's pre-launch selection boundary."""
import hashlib
import importlib.util
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


if __name__ == "__main__":
    unittest.main()
