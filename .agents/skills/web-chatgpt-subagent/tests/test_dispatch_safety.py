"""Persistent serial-dispatch and cooldown checks for the Windows ledger CLI."""

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


class DispatchSafetyTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.env = {**os.environ, "LOCALAPPDATA": self.tmp.name}
        self.state = Path(self.tmp.name) / "gogoke" / "web-chatgpt-subagent" / "state.json"
        self.assertEqual(self.run_ledger("reconcile", "--source", "unit-test-start").returncode, 0)

    def run_ledger(self, *args):
        return subprocess.run([sys.executable, str(SCRIPT), *args], env=self.env, text=True, capture_output=True)

    def test_one_active_web_task_even_after_reply_until_github_result(self):
        first = self.run_ledger("reserve", "--tier", "high", "--task", "one", "--seat", "a")
        self.assertEqual(first.returncode, 0, first.stderr)
        second = self.run_ledger("reserve", "--tier", "medium", "--task", "two", "--seat", "b")
        self.assertEqual(second.returncode, 2)
        self.assertIn("still active", second.stderr)
        reply = self.run_ledger("complete", first.stdout.strip(), "--actual-tier", "high")
        self.assertEqual(reply.returncode, 0, reply.stderr)
        still_active = self.run_ledger("reserve", "--tier", "medium", "--task", "two", "--seat", "b")
        self.assertEqual(still_active.returncode, 2)
        self.assertIn("still active", still_active.stderr)
        finish = self.run_ledger("finish-task", "--task", "one", "--result-commit", "a" * 40)
        self.assertEqual(finish.returncode, 0, finish.stderr)
        later = self.run_ledger("reserve", "--tier", "medium", "--task", "two", "--seat", "b")
        self.assertEqual(later.returncode, 0, later.stderr)
        released = self.run_ledger("release", later.stdout.strip())
        self.assertEqual(released.returncode, 0, released.stderr)
        duplicate = self.run_ledger("reserve", "--tier", "medium", "--task", "one", "--seat", "b")
        self.assertEqual(duplicate.returncode, 2)
        self.assertIn("task ID already used", duplicate.stderr)

    def test_cooldown_persists_across_processes_and_does_not_auto_resume(self):
        started = self.run_ledger("cooldown", "--reason", "trial_simulation")
        self.assertEqual(started.returncode, 0, started.stderr)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        self.assertEqual(state["cooldown"]["reason"], "trial_simulation")
        self.assertGreaterEqual(datetime.fromisoformat(state["cooldown"]["until"]) - datetime.fromisoformat(state["cooldown"]["started_at"]), timedelta(hours=24))
        denied = self.run_ledger("reserve", "--tier", "high", "--task", "blocked", "--seat", "a")
        self.assertEqual(denied.returncode, 2)
        self.assertIn("route to Codex", denied.stderr)
        too_early = self.run_ledger("clear-cooldown", "--observed-at", datetime.now(timezone.utc).isoformat(), "--evidence", "test")
        self.assertEqual(too_early.returncode, 2)
        state["cooldown"]["until"] = (datetime.now(timezone.utc) - timedelta(minutes=1)).isoformat()
        self.state.write_text(json.dumps(state), encoding="utf-8")
        still_denied = self.run_ledger("reserve", "--tier", "high", "--task", "blocked", "--seat", "a")
        self.assertEqual(still_denied.returncode, 2)
        self.assertIn("manual clearance after 24 hours required", still_denied.stderr)
        observed = (datetime.now(timezone.utc) - timedelta(seconds=30)).isoformat()
        cleared = self.run_ledger("clear-cooldown", "--observed-at", observed, "--evidence", "safe page after simulated expiry")
        self.assertEqual(cleared.returncode, 0, cleared.stderr)
        resumed = self.run_ledger("reserve", "--tier", "high", "--task", "after-clear", "--seat", "a")
        self.assertEqual(resumed.returncode, 0, resumed.stderr)

    def test_security_verification_requires_owner_notification(self):
        activated = self.run_ledger("cooldown", "--reason", "security_verification")
        self.assertEqual(activated.returncode, 0, activated.stderr)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        self.assertTrue(state["cooldown"]["owner_notification_required"])

    def test_f1_switched_reply_charges_actual_tier_and_requires_flag(self):
        held = self.run_ledger("reserve", "--tier", "high", "--task", "f1-switch", "--seat", "a")
        self.assertEqual(held.returncode, 0, held.stderr)
        missing_flag = self.run_ledger("complete", held.stdout.strip(), "--actual-tier", "medium")
        self.assertEqual(missing_flag.returncode, 2)
        self.assertIn("switch flag", missing_flag.stderr)
        switched = self.run_ledger("complete", held.stdout.strip(), "--actual-tier", "medium", "--switched")
        self.assertEqual(switched.returncode, 0, switched.stderr)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        self.assertEqual(len(state["reservations"]), 0)
        self.assertEqual(state["events"][-1]["actual_tier"], "medium")
        self.assertIn("high", state["blocked"])
        status = json.loads(self.run_ledger("status").stdout)
        self.assertEqual(status["tiers"]["high"]["used_or_reserved_today"], 1)
        self.assertEqual(status["tiers"]["medium"]["used_or_reserved_today"], 1)

    def test_f1_limit_without_reply_is_terminal_and_blocks_tier(self):
        held = self.run_ledger("reserve", "--tier", "extra-high", "--task", "f1-limit", "--seat", "a")
        self.assertEqual(held.returncode, 0, held.stderr)
        limit = self.run_ledger("limit-hit", held.stdout.strip())
        self.assertEqual(limit.returncode, 0, limit.stderr)
        fallback = self.run_ledger("finish-task", "--task", "f1-limit", "--fallback-agent", "codex-fallback")
        self.assertEqual(fallback.returncode, 0, fallback.stderr)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        self.assertEqual(state["events"][-1]["outcome"], "limit_no_reply")
        denied = self.run_ledger("reserve", "--tier", "extra-high", "--task", "next", "--seat", "a")
        self.assertEqual(denied.returncode, 2)
        self.assertIn("exhausted", denied.stderr)

    def test_f2_new_or_lost_ledger_requires_controller_reconcile(self):
        self.state.unlink()
        denied = self.run_ledger("reserve", "--tier", "high", "--task", "new", "--seat", "a")
        self.assertEqual(denied.returncode, 2)
        self.assertIn("unreconciled", denied.stderr)
        reconciled = self.run_ledger("reconcile", "--source", "controller starts ledger now; prior unknown")
        self.assertEqual(reconciled.returncode, 0, reconciled.stderr)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        self.assertEqual(state["reconciliation"]["prior_usage"], "unknown")
        self.assertEqual(state["reconciliation"]["scope"], "skill_and_web_construction_seat_only")
        allowed = self.run_ledger("reserve", "--tier", "high", "--task", "new", "--seat", "a")
        self.assertEqual(allowed.returncode, 0, allowed.stderr)
        self.state.unlink()
        lost = self.run_ledger("reserve", "--tier", "medium", "--task", "after-loss", "--seat", "b")
        self.assertEqual(lost.returncode, 2)
        self.assertIn("unreconciled", lost.stderr)

    def test_f3_same_task_id_cannot_be_reserved_twice_across_seats(self):
        start = threading.Barrier(3)

        def reserve(seat):
            start.wait(timeout=10)
            return self.run_ledger("reserve", "--tier", "high", "--task", "same-id", "--seat", seat)

        with ThreadPoolExecutor(max_workers=2) as pool:
            futures = [pool.submit(reserve, seat) for seat in ("seat-a", "seat-b")]
            start.wait(timeout=10)
            results = [f.result(timeout=30) for f in futures]
        self.assertCountEqual([r.returncode for r in results], [0, 2])
        winner = next(r for r in results if r.returncode == 0)
        self.assertEqual(self.run_ledger("complete", winner.stdout.strip(), "--actual-tier", "high").returncode, 0)
        self.assertEqual(self.run_ledger("finish-task", "--task", "same-id", "--result-commit", "a" * 40).returncode, 0)
        again = self.run_ledger("reserve", "--tier", "high", "--task", "same-id", "--seat", "seat-b")
        self.assertEqual(again.returncode, 2)
        self.assertIn("task ID already used", again.stderr)

    def test_f3_released_task_id_is_not_reused_by_another_seat(self):
        first = self.run_ledger("reserve", "--tier", "medium", "--task", "released-id", "--seat", "a")
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(self.run_ledger("release", first.stdout.strip()).returncode, 0)
        again = self.run_ledger("reserve", "--tier", "medium", "--task", "released-id", "--seat", "b")
        self.assertEqual(again.returncode, 2)
        self.assertIn("task ID already used", again.stderr)

    def test_confirmed_or_uncertain_send_cannot_be_released_for_retry(self):
        sent = self.run_ledger("reserve", "--tier", "high", "--task", "sent", "--seat", "a")
        self.assertEqual(sent.returncode, 0, sent.stderr)
        self.assertEqual(self.run_ledger("mark-sent", sent.stdout.strip(), "--url", "https://chatgpt.com/c/example").returncode, 0)
        release_sent = self.run_ledger("release", sent.stdout.strip())
        self.assertEqual(release_sent.returncode, 2)
        self.assertIn("cannot be released", release_sent.stderr)
        self.assertEqual(self.run_ledger("complete-unverified", sent.stdout.strip()).returncode, 0)
        self.assertEqual(self.run_ledger("finish-task", "--task", "sent", "--result-commit", "a" * 40).returncode, 0)
        uncertain = self.run_ledger("reserve", "--tier", "high", "--task", "uncertain", "--seat", "b")
        self.assertEqual(uncertain.returncode, 0, uncertain.stderr)
        self.assertEqual(self.run_ledger("mark-uncertain", uncertain.stdout.strip(), "--reason", "click_outcome_unknown").returncode, 0)
        release_uncertain = self.run_ledger("release", uncertain.stdout.strip())
        self.assertEqual(release_uncertain.returncode, 2)
        self.assertIn("cannot be released", release_uncertain.stderr)


if __name__ == "__main__":
    unittest.main()
