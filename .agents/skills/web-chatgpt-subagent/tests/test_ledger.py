"""Behavioral checks against the Windows command-line ledger."""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "ledger.py"


class LedgerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.env = {**os.environ, "LOCALAPPDATA": self.tmp.name}
        self.state = Path(self.tmp.name) / "gogoke" / "web-chatgpt-subagent" / "state.json"
        self.assertEqual(self.run_ledger("reconcile", "--source", "unit-test-start").returncode, 0)

    def run_ledger(self, *args):
        return subprocess.run([sys.executable, str(SCRIPT), *args], env=self.env, text=True, capture_output=True)

    def test_six_pro_is_disabled(self):
        result = self.run_ledger("reserve", "--tier", "gpt-6-pro", "--task", "t", "--seat", "trial")
        self.assertEqual(result.returncode, 2)
        self.assertIn("disabled", result.stderr)

    def test_daily_cap_blocks_121st_sol_pro_message(self):
        stamp = datetime.now(timezone.utc).astimezone().isoformat()
        self.state.parent.mkdir(parents=True, exist_ok=True)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        state["events"] = [{"tier": "gpt-5.6-sol-pro", "at": stamp} for _ in range(120)]
        self.state.write_text(json.dumps(state), encoding="utf-8")
        result = self.run_ledger("reserve", "--tier", "gpt-5.6-sol-pro", "--task", "t", "--seat", "trial")
        self.assertEqual(result.returncode, 2)
        self.assertIn("daily cap", result.stderr)

    def test_switched_reply_blocks_requested_tier(self):
        first = self.run_ledger("reserve", "--tier", "high", "--task", "t", "--seat", "trial")
        self.assertEqual(first.returncode, 0, first.stderr)
        saved = self.run_ledger("complete", first.stdout.strip(), "--actual-tier", "medium", "--switched")
        self.assertEqual(saved.returncode, 0, saved.stderr)
        finished = self.run_ledger("finish-task", "--task", "t", "--result-commit", "a" * 40)
        self.assertEqual(finished.returncode, 0, finished.stderr)
        later = self.run_ledger("reserve", "--tier", "high", "--task", "t2", "--seat", "trial")
        self.assertEqual(later.returncode, 2)
        self.assertIn("exhausted", later.stderr)


if __name__ == "__main__":
    unittest.main()
