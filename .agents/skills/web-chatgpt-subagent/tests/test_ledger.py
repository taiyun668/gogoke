"""Windows CLI checks for the single-tier, one-shot web ledger."""

import json
import os
import subprocess
import sys
import tempfile
import threading
import unittest
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta, timezone
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "ledger.py"
SHA = "a" * 40


class LedgerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.env = {**os.environ, "LOCALAPPDATA": self.tmp.name}
        self.state = Path(self.tmp.name) / "gogoke" / "web-chatgpt-subagent" / "state.json"

    def call(self, *args):
        return subprocess.run([sys.executable, str(SCRIPT), *args], env=self.env, text=True, capture_output=True)

    def reconcile_enable(self):
        self.assertEqual(self.call("reconcile", "--source", "start-now-history-unknown").returncode, 0)
        self.assertEqual(self.call("enable-6-pro", "--owner-decision", "Owner 2026-09-25 one trial").returncode, 0)

    def reserve(self, task="one", seat="web-channel"):
        return self.call("reserve", "--tier", "gpt-6-pro", "--task", task, "--seat", seat,
                         "--original-seat", "astra", "--economics", "Fixed-SHA audit would cost Astra > web overhead")

    def test_f2_new_or_lost_state_requires_reconcile(self):
        self.assertEqual(self.call("enable-6-pro", "--owner-decision", "trial").returncode, 0)
        denied = self.reserve()
        self.assertEqual(denied.returncode, 2)
        self.assertIn("unreconciled", denied.stderr)
        self.reconcile_enable()
        first = self.reserve()
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(self.call("release", first.stdout.strip()).returncode, 0)
        self.state.unlink()
        self.assertEqual(self.call("enable-6-pro", "--owner-decision", "trial").returncode, 0)
        self.assertIn("unreconciled", self.reserve("new").stderr)
        self.reconcile_enable()
        self.assertEqual(self.reserve("new").returncode, 0)

    def test_only_pro_tier_and_25_daily_cap(self):
        self.reconcile_enable()
        wrong = self.call("reserve", "--tier", "medium", "--task", "wrong", "--seat", "a", "--original-seat", "astra", "--economics", "x")
        self.assertNotEqual(wrong.returncode, 0)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        state["events"] = [{"tier": "gpt-6-pro", "at": datetime.now(timezone.utc).astimezone().isoformat()} for _ in range(25)]
        self.state.write_text(json.dumps(state), encoding="utf-8")
        denied = self.reserve()
        self.assertEqual(denied.returncode, 2)
        self.assertIn("daily cap", denied.stderr)
        status = json.loads(self.call("status").stdout)
        self.assertEqual(list(status["tiers"]), ["gpt-6-pro"])

    def test_f1_switch_flag_blocks_pro_and_preserves_observed_model(self):
        self.reconcile_enable()
        rid = self.reserve().stdout.strip()
        self.assertEqual(self.call("mark-sent", rid, "--url", "https://chatgpt.com/c/example").returncode, 0)
        missing = self.call("complete", rid, "--actual-tier", "gpt-5.6-medium")
        self.assertEqual(missing.returncode, 2)
        self.assertIn("switch flag", missing.stderr)
        self.assertEqual(self.call("complete", rid, "--actual-tier", "gpt-5.6-medium", "--switched").returncode, 0)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        self.assertEqual(state["events"][-1]["actual_tier"], "gpt-5.6-medium")
        self.assertIn("gpt-6-pro", state["blocked"])
        self.assertEqual(self.call("approve-result", "--task", "one", "--commit", SHA).returncode, 2)
        self.assertEqual(self.call("finish-task", "--task", "one", "--fallback-agent", "codex", "--reason", "downgrade").returncode, 0)
        self.assertIn("exhausted", self.reserve("two").stderr)

    def test_f1_limit_without_reply_is_terminal(self):
        self.reconcile_enable()
        rid = self.reserve().stdout.strip()
        self.assertEqual(self.call("limit-hit", rid).returncode, 0)
        self.assertEqual(self.call("finish-task", "--task", "one", "--fallback-agent", "codex", "--reason", "limit").returncode, 0)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        self.assertEqual(state["events"][-1]["outcome"], "limit_no_reply")
        self.assertIn("gpt-6-pro", state["blocked"])

    def test_serial_slot_and_f3_task_id_survive_state_loss(self):
        self.reconcile_enable()
        barrier = threading.Barrier(3)
        def attempt(seat):
            barrier.wait(timeout=10)
            return self.reserve("shared", seat)
        with ThreadPoolExecutor(max_workers=2) as pool:
            futures = [pool.submit(attempt, seat) for seat in ("a", "b")]
            barrier.wait(timeout=10)
            results = [f.result(timeout=30) for f in futures]
        self.assertCountEqual([r.returncode for r in results], [0, 2])
        rid = next(r.stdout.strip() for r in results if r.returncode == 0)
        self.assertIn("still active", self.reserve("other").stderr)
        self.assertEqual(self.call("release", rid).returncode, 0)
        self.assertIn("task ID already used", self.reserve("shared").stderr)
        self.state.unlink()
        self.reconcile_enable()
        self.assertIn("task ID already used", self.reserve("shared").stderr)

    def test_cooldown_persists_and_requires_manual_clear(self):
        self.reconcile_enable()
        self.assertEqual(self.call("cooldown", "--reason", "security_verification").returncode, 0)
        self.assertIn("route to Codex", self.reserve().stderr)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        self.assertTrue(state["cooldown"]["owner_notification_required"])
        self.assertEqual(self.call("clear-cooldown", "--observed-at", datetime.now(timezone.utc).isoformat(), "--evidence", "safe").returncode, 2)
        state["cooldown"]["until"] = (datetime.now(timezone.utc) - timedelta(minutes=1)).isoformat()
        self.state.write_text(json.dumps(state), encoding="utf-8")
        self.assertIn("manual clearance", self.reserve().stderr)
        observed = (datetime.now(timezone.utc) - timedelta(seconds=30)).isoformat()
        self.assertEqual(self.call("clear-cooldown", "--observed-at", observed, "--evidence", "safe page").returncode, 0)
        self.assertEqual(self.reserve().returncode, 0)

    def test_timeout_voids_id_and_late_result_not_success(self):
        self.reconcile_enable()
        rid = self.reserve().stdout.strip()
        self.assertEqual(self.call("mark-sent", rid, "--url", "https://chatgpt.com/c/example").returncode, 0)
        self.assertEqual(self.call("complete-unverified", rid).returncode, 0)
        self.assertEqual(self.call("finish-task", "--task", "one", "--fallback-agent", "codex", "--reason", "deadline", "--timed-out").returncode, 0)
        self.assertEqual(self.call("approve-result", "--task", "one", "--commit", SHA).returncode, 2)
        self.assertEqual(self.call("late-result", "--task", "one", "--commit", SHA).returncode, 0)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        self.assertEqual(state["one_shot_results"][-1]["success"], False)
        self.assertEqual(state["late_results"][-1]["accepted"], False)
        self.assertIn("task ID already used", self.reserve("one").stderr)

    def test_approved_result_is_one_shot_success(self):
        self.reconcile_enable()
        rid = self.reserve().stdout.strip()
        self.assertEqual(self.call("mark-sent", rid, "--url", "https://chatgpt.com/c/example").returncode, 0)
        self.assertEqual(self.call("complete-unverified", rid).returncode, 0)
        self.assertEqual(self.call("finish-task", "--task", "one", "--result-commit", SHA).returncode, 2)
        self.assertEqual(self.call("approve-result", "--task", "one", "--commit", SHA).returncode, 0)
        self.assertEqual(self.call("finish-task", "--task", "one", "--result-commit", SHA).returncode, 0)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        self.assertTrue(state["one_shot_results"][-1]["success"])
        self.assertEqual(state["one_shot_results"][-1]["served_model_status"], "unverified")
        self.assertEqual(state["reservations"], {})

    def test_sent_or_uncertain_reservation_cannot_be_released_for_resend(self):
        self.reconcile_enable()
        sent_id = self.reserve("sent").stdout.strip()
        self.assertEqual(self.call("mark-sent", sent_id, "--url", "https://chatgpt.com/c/example").returncode, 0)
        denied_sent = self.call("release", sent_id)
        self.assertEqual(denied_sent.returncode, 2)
        self.assertIn("sent or uncertain", denied_sent.stderr)
        self.assertEqual(self.call("finish-task", "--task", "sent", "--fallback-agent", "codex", "--reason", "no accepted result").returncode, 0)
        self.assertIn("task ID already used", self.reserve("sent").stderr)

        uncertain_id = self.reserve("uncertain").stdout.strip()
        self.assertEqual(self.call("mark-uncertain", uncertain_id, "--reason", "click outcome unknown", "--url", "https://chatgpt.com/c/unknown").returncode, 0)
        denied_uncertain = self.call("release", uncertain_id)
        self.assertEqual(denied_uncertain.returncode, 2)
        self.assertIn("sent or uncertain", denied_uncertain.stderr)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        self.assertIn(uncertain_id, state["reservations"])
        self.assertEqual(state["active_web_task"]["task"], "uncertain")
        self.assertIn("still active", self.reserve("other").stderr)

    def test_owner_confirmed_new_chat_corrects_uncertain_attempt_without_reusing_id(self):
        self.reconcile_enable()
        rid = self.reserve("pre-send").stdout.strip()
        self.assertEqual(self.call("mark-sent", rid, "--url", "https://chatgpt.com/").returncode, 2)
        self.assertEqual(self.call("mark-uncertain", rid, "--reason", "click no turn", "--url", "https://chatgpt.com/").returncode, 2)
        # Model the already-finished WCS-25 record written by the older CLI.
        legacy = json.loads(self.state.read_text(encoding="utf-8"))
        legacy["reservations"][rid].update({"uncertain_at": datetime.now(timezone.utc).isoformat(), "uncertain_reason": "click no turn", "url": "https://chatgpt.com/"})
        self.state.write_text(json.dumps(legacy), encoding="utf-8")
        self.assertEqual(self.call("finish-task", "--task", "pre-send", "--fallback-agent", "codex", "--reason", "click no turn").returncode, 0)
        self.assertEqual(json.loads(self.call("status").stdout)["tiers"]["gpt-6-pro"]["used_or_reserved_today"], 1)
        correction = ("correct-unsent", "--task", "pre-send", "--observed-url", "https://chatgpt.com/",
                      "--evidence", "same tab remained new chat; draft cleared and read back empty",
                      "--owner-decision", "Owner confirmed new-chat page means no Send")
        self.assertEqual(self.call(*correction).returncode, 0)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        self.assertEqual(state["events"][-1]["outcome"], "web_not_sent")
        self.assertFalse(state["one_shot_results"][-1]["eligible_for_one_shot_rate"])
        self.assertEqual(json.loads(self.call("status").stdout)["tiers"]["gpt-6-pro"]["used_or_reserved_today"], 0)
        self.assertIn("task ID already used", self.reserve("pre-send").stderr)
        self.assertEqual(self.call(*correction).returncode, 2)

        sent_id = self.reserve("actually-sent").stdout.strip()
        self.assertEqual(self.call("mark-sent", sent_id, "--url", "https://chatgpt.com/c/example").returncode, 0)
        self.assertEqual(self.call("finish-task", "--task", "actually-sent", "--fallback-agent", "codex", "--reason", "no result").returncode, 0)
        denied = self.call("correct-unsent", "--task", "actually-sent", "--observed-url", "https://chatgpt.com/",
                           "--evidence", "not true", "--owner-decision", "not true")
        self.assertEqual(denied.returncode, 2)


if __name__ == "__main__":
    unittest.main()
