"""Focused offline checks for the common reader's pre-launch selection boundary."""
import hashlib
import importlib.util
import json
import copy
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


class OriginalResumeCandidateStop(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(":memory:")
        self.db.row_factory = sqlite3.Row
        self.db.executescript("""
            CREATE TABLE gogoke_v37_h_operation(domain_id TEXT,request_id TEXT,session_id TEXT,
                operation TEXT,raw_hex TEXT,status TEXT,previous_revision INTEGER,revision INTEGER);
            CREATE TABLE gogoke_v37_h_process_episode(domain_id TEXT,request_id TEXT,session_id TEXT,
                generation TEXT,old_generation TEXT,raw_hex TEXT,previous_revision INTEGER,
                result_revision INTEGER,process_operation_id TEXT,instance_id TEXT,home_id TEXT,
                binding_id TEXT,seat_id TEXT,seat_incarnation TEXT,phase TEXT,stop_fact_id TEXT,
                stop_request_id TEXT);
            CREATE TABLE gogoke_coordination_process_custody(operation_id TEXT,domain_id TEXT,
                profile_id TEXT,generation TEXT,state TEXT,stop_proof_hash TEXT);
            CREATE TABLE gogoke_v37_h_claim(domain_id TEXT,session_id TEXT,generation TEXT,
                state TEXT,revision INTEGER,stop_fact_id TEXT,process_operation_id TEXT);
            CREATE TABLE gogoke_v37_instance_operations(request_id TEXT,target_id TEXT,
                request_hex TEXT,phase TEXT,receipt_json TEXT,native_receipt_id TEXT);
        """)
        self.home = {"home_id": "home-" + "a" * 40, "instance_id": "instanceA",
            "domain_id": "projectA", "owner_id": "sessionA", "generation": "2",
            "kind": "SESSION", "state": "CLOSED", "revision": "2",
            "directory_identity": "dir-fid",
            "directory_ref": "temp-home-" + hashlib.sha256(("home-" + "a" * 40).encode()).hexdigest()}
        self.old_fact = "sha256:" + "a" * 64
        self.candidate_fact = "sha256:" + "b" * 64
        self.resume = {"schema": "gogoke.37.operations.v1", "family": "K-SESSION",
            "operation": "resume", "requestId": "originalResume", "domainId": "projectA",
            "targetId": "sessionA", "expectedRevision": "3", "payload": {"generation": "1"}}
        self.resume_raw = json.dumps(self.resume, separators=(",", ":")).encode()
        self.resume_receipt = {"schema": self.resume["schema"], "family": "K-SESSION",
            "operation": "resume", "requestId": "originalResume", "targetId": "sessionA",
            "previousRevision": "3", "revision": "4", "status": "UNKNOWN", "result": {}}
        self.stop = {"schema": self.resume["schema"], "family": "K-SESSION",
            "operation": "stop", "requestId": "candidateStop", "domainId": "projectA",
            "targetId": "sessionA", "expectedRevision": "4",
            "payload": {"seatId": "seatA", "generation": "1",
                "resumeRequestId": "originalResume"}}
        self.stop_raw = json.dumps(self.stop, separators=(",", ":")).encode()
        self.stop_receipt = {"schema": self.stop["schema"], "family": "K-SESSION",
            "operation": "stop", "requestId": "candidateStop", "targetId": "sessionA",
            "previousRevision": "4", "revision": "4", "status": "APPLIED",
            "result": {"stopScope": "ORIGINAL_RESUME_CANDIDATE",
                "originalResumeRequestId": "originalResume", "stoppedGeneration": "2",
                "stopFact": self.candidate_fact}}
        self.attempt = {"operation": "resume", "generation": None,
            "producerGeneration": "1", "originalCreateRequestId": "originalResume",
            "originalCreateRawFrame": self.resume_raw.decode(),
            "originalCreateRawSha256": hashlib.sha256(self.resume_raw).hexdigest(),
            "receipt": self.resume_receipt, "candidateStopDispatchState": "RECEIPT_RECORDED",
            "candidateStopRequestId": "candidateStop",
            "candidateStopRawFrame": self.stop_raw.decode(),
            "candidateStopRawSha256": hashlib.sha256(self.stop_raw).hexdigest(),
            "candidateStopReceipt": self.stop_receipt, "candidateStoppedGeneration": "2",
            "candidateStopSource": {"requestId": "candidateStop",
                "rawFrame": self.stop_raw.decode(),
                "rawSha256": hashlib.sha256(self.stop_raw).hexdigest(),
                "receipt": self.stop_receipt, "stoppedGeneration": "2"}}
        self.db.execute("INSERT INTO gogoke_v37_h_operation VALUES(?,?,?,?,?,?,?,?)",
            ("projectA", "originalResume", "sessionA", "resume", self.resume_raw.hex(),
             "UNKNOWN", 3, 4))
        self.db.execute("INSERT INTO gogoke_v37_h_operation VALUES(?,?,?,?,?,?,?,?)",
            ("projectA", "candidateStop", "sessionA", "stop-resume-candidate",
             self.stop_raw.hex(), "APPLIED", 4, 4))
        candidate_op = "h-resume-" + hashlib.sha256(self.resume_raw).hexdigest()[:40]
        self.db.execute("INSERT INTO gogoke_v37_h_process_episode VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            ("projectA", "originalResume", "sessionA", "2", "1", self.resume_raw.hex(),
             3, 4, candidate_op, "instanceA", self.home["home_id"],
             "binding-" + "a" * 40, "seatA", "incA", "UNKNOWN", None, None))
        self.db.execute("INSERT INTO gogoke_v37_h_process_episode VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            ("projectA", "oldOpen", "sessionA", "1", None, "00", 0, 1, "oldOp",
             "instanceA", "oldHome", "oldBinding", "seatA", "incA", "STOPPED",
             self.old_fact, "oldStop"))
        self.db.execute("INSERT INTO gogoke_coordination_process_custody VALUES(?,?,?,?,?,?)",
            (candidate_op, "projectA", "instanceA", "2", "STOPPED", self.candidate_fact))
        self.db.execute("INSERT INTO gogoke_coordination_process_custody VALUES(?,?,?,?,?,?)",
            ("oldOp", "projectA", "instanceA", "1", "STOPPED", self.old_fact))
        self.db.execute("INSERT INTO gogoke_v37_h_claim VALUES(?,?,?,?,?,?,?)",
            ("projectA", "sessionA", "1", "STOPPED", 4, self.old_fact, "oldOp"))
        self.create_id = "homeprep-" + hashlib.sha256(
            b"projectA\noriginalResume").hexdigest()[:40]
        self.sid = "S-1-15-2-123456789"
        framed = b"".join(len(field).to_bytes(8, "big") + field for field in
            (b"temporary-create-detached", self.resume_raw, self.home["home_id"].encode(),
             b"instanceA", b"projectA", b"SESSION", b"sessionA", b"2",
             self.sid.encode()))
        self.create_hex = framed.hex()
        self.db.execute("INSERT INTO gogoke_v37_instance_operations VALUES(?,?,?,?,?,?)",
            (self.create_id, self.home["home_id"], self.create_hex, "APPLIED", "", "dir-fid"))

    def tearDown(self):
        self.db.close()

    def source(self, attempt=None):
        return READER.original_resume_candidate_stop_source(self.db, self.home,
            attempt or self.attempt, self.resume_raw)

    def test_original_unknown_and_independent_h_c_proof_are_exact(self):
        fact, raw_sha, source = self.source()
        self.assertEqual((fact, raw_sha),
            (self.candidate_fact, hashlib.sha256(self.stop_raw).hexdigest()))
        self.assertEqual(source, self.attempt["candidateStopSource"])
        generated, episode = READER.source_attempt(self.db, self.home, self.attempt)
        self.assertEqual(episode["phase"], "UNKNOWN")
        self.assertEqual(generated["candidateStopSource"], source)
        self.assertNotIn("hStopSource", generated)
        self.assertEqual(self.db.execute("SELECT stop_fact_id FROM gogoke_v37_h_claim").fetchone()[0],
            self.old_fact)

    def test_single_source_drift_fails_closed(self):
        changes = [
            ("UPDATE gogoke_v37_h_process_episode SET raw_hex='00' WHERE request_id='originalResume'", None, RuntimeError),
            ("UPDATE gogoke_v37_h_operation SET status='UNKNOWN' WHERE request_id='candidateStop'", None, RuntimeError),
            ("UPDATE gogoke_coordination_process_custody SET stop_proof_hash=NULL WHERE generation='2'", None, RuntimeError),
            ("UPDATE gogoke_v37_h_claim SET stop_fact_id=?", self.candidate_fact, RuntimeError),
            ("UPDATE gogoke_v37_h_operation SET raw_hex='00' WHERE request_id='candidateStop'", None, ValueError),
            ("INSERT INTO gogoke_v37_h_operation VALUES('projectA','duplicateCandidate','sessionA',"
             "'stop-resume-candidate',?,'UNKNOWN',4,4)", self.stop_raw.hex(), RuntimeError),
        ]
        for statement, value, error_type in changes:
            with self.subTest(statement=statement):
                self.db.execute("SAVEPOINT candidate_negative")
                try:
                    self.db.execute(statement, (value,) if value else ())
                    with self.assertRaises(error_type):
                        self.source()
                finally:
                    self.db.execute("ROLLBACK TO candidate_negative")
                    self.db.execute("RELEASE candidate_negative")
        for mutate in [
            lambda a: a["candidateStopSource"]["receipt"]["result"].update(stopScope="ORDINARY"),
            lambda a: a["candidateStopSource"]["receipt"]["result"].update(stoppedGeneration="1"),
            lambda a: a["candidateStopSource"].update(rawSha256="0" * 64),
        ]:
            changed = copy.deepcopy(self.attempt)
            mutate(changed)
            with self.assertRaises(RuntimeError):
                self.source(changed)

    def test_candidate_proof_binds_the_original_f_close_fingerprint(self):
        def sha(value):
            return hashlib.sha256(value).hexdigest()

        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            directory = root / "v37-temporary-homes" / "instanceA" / self.home["directory_ref"]
            directory.mkdir(parents=True)
            (root / "state.sqlite").write_bytes(b"synthetic closed fixture")
            marker = ("gogoke-v37-temporary-home-detached-v1\nroot-fid\n" + "\n".join([
                "instanceA", self.home["home_id"], "projectA", "SESSION", "sessionA", "2",
                self.create_id, sha(self.create_hex.encode()), "dir-fid", self.sid]) + "\n")
            (directory / "gogoke-temporary-home.marker").write_bytes(marker.encode())

            class Physical:
                def windows_file_identity(self, value):
                    value = Path(value)
                    if value == root:
                        return "root-fid"
                    if value == root / "state.sqlite":
                        return "db-fid"
                    if value == directory:
                        return "dir-fid"
                    raise AssertionError(f"unexpected physical source: {value}")

                def contained_physical(self, _parent, value):
                    assert Path(value).is_relative_to(root)
                    return Path(value)

            close_request = {"schema": "gogoke.37.operations.v1", "family": "K-INSTANCE",
                "operation": "temporary-home-close", "requestId": "closeCandidate",
                "domainId": "projectA", "targetId": "instanceA", "expectedRevision": "1",
                "payload": {"sessionId": "sessionA", "originalCreateRequestId": "originalResume"}}
            close_raw = json.dumps(close_request, separators=(",", ":")).encode()
            close_hex = b"".join(len(field).to_bytes(8, "big") + field for field in (
                b"temporary-home-close-v2", close_raw, self.home["home_id"].encode(),
                b"1", self.sid.encode())).hex()
            proof_fingerprint = sha(("home-stop-v1\n" + "\n".join([
                self.home["home_id"], self.create_id, sha(self.resume_raw),
                sha(self.stop_raw), self.candidate_fact])).encode())
            source_fingerprint = sha(("temporary-home-close-v2\n" + "\n".join([
                "root-fid", "db-fid", self.home["home_id"], "instanceA", "projectA",
                "sessionA", "2", "dir-fid", self.sid, self.create_hex,
                sha(self.resume_raw), "ORIGINAL_STOP", proof_fingerprint])).encode())
            receipt_id = "f-home-close-v2-" + sha(("f-home-close-receipt-v2\n" + "\n".join([
                "closeCandidate", sha(close_raw), source_fingerprint, self.home["home_id"],
                "instanceA", "projectA", "sessionA", "1", "2"])).encode())
            native = {"schema": "gogoke.37.temporary-home-close.v2", "receiptId": receipt_id,
                "proofKind": "ORIGINAL_STOP", "sourceFingerprint": source_fingerprint,
                "homeId": self.home["home_id"], "originalCreateRequestId": "originalResume",
                "fCreateOperationId": self.create_id, "beforeRevision": 1, "afterRevision": 2}
            self.db.execute("INSERT INTO gogoke_v37_instance_operations VALUES(?,?,?,?,?,?)",
                ("closeCandidate", self.home["home_id"], close_hex, "APPLIED",
                 json.dumps(native, separators=(",", ":")), receipt_id))
            attempt = dict(self.attempt, caseId="candidateCase", domainId="projectA",
                sessionId="sessionA", instanceId="instanceA", closeRequestId="closeCandidate",
                closeRawFrame=close_raw.decode(), closeRawSha256=sha(close_raw),
                closeReceipt={"schema": "gogoke.37.operations.v1", "family": "K-INSTANCE",
                    "operation": "temporary-home-close", "requestId": "closeCandidate",
                    "targetId": "instanceA", "status": "APPLIED", "previousRevision": "1",
                    "revision": "2", "result": {"homeId": self.home["home_id"],
                        "nativeReceiptId": receipt_id, "proofKind": "ORIGINAL_STOP",
                        "sourceFingerprint": source_fingerprint,
                        "directoryRef": self.home["directory_ref"], "state": "CLOSED"}})
            observed = READER.verify_attempt(self.db, Physical(), root, attempt,
                {self.home["home_id"]: self.home})
            self.assertEqual((observed["state"], observed["proofKind"]),
                ("CLOSED", "ORIGINAL_STOP"))
            native["sourceFingerprint"] = "0" * 64
            self.db.execute("UPDATE gogoke_v37_instance_operations SET receipt_json=? WHERE request_id='closeCandidate'",
                (json.dumps(native, separators=(",", ":")),))
            with self.assertRaises(RuntimeError):
                READER.verify_attempt(self.db, Physical(), root, attempt,
                    {self.home["home_id"]: self.home})


if __name__ == "__main__":
    unittest.main()
