"""Persistent serial-dispatch and cooldown checks for the Windows ledger CLI."""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "ledger.py"


class DispatchSafetyTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.env = {**os.environ, "LOCALAPPDATA": self.tmp.name}
        self.state = Path(self.tmp.name) / "gogoke" / "web-chatgpt-subagent" / "state.json"

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


if __name__ == "__main__":
    unittest.main()
