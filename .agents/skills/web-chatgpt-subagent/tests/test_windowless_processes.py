"""Windows proof that required Python helpers have no console window."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"


@unittest.skipUnless(sys.platform == "win32", "Windows-only window check")
class WindowlessProcessTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.env = {**os.environ, "LOCALAPPDATA": self.tmp.name}
        self.root = Path(self.tmp.name) / "gogoke" / "web-chatgpt-subagent"
        self.pythonw = Path(sys.executable).with_name("pythonw.exe")
        self.assertTrue(self.pythonw.is_file())

    def run_hidden(self, *args: str) -> int:
        process = subprocess.Popen(
            [str(self.pythonw), *args],
            env=self.env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        return process.wait(timeout=15)

    def test_ledger_receipt_reports_no_console_window(self):
        receipt = self.root / "receipts" / "ledger-test.json"
        code = self.run_hidden(str(SCRIPTS / "ledgerw.py"), "--receipt", str(receipt), "--", "reconcile", "--source", "windowless-test")
        self.assertEqual(code, 0)
        result = json.loads(receipt.read_text(encoding="utf-8"))
        self.assertEqual(result["exit_code"], 0)
        self.assertFalse(result["console_window_present"])

    def test_github_watcher_starts_without_console_window(self):
        code = self.run_hidden(
            str(SCRIPTS / "watch_github_result.py"),
            "--task", "WINDOW-TEST", "--repo", "example/example", "--branch", "gpt/example",
            "--path", "result.md", "--base-sha", "a" * 40,
            "--thread", "00000000-0000-0000-0000-000000000000", "--max-hours", "0",
        )
        self.assertEqual(code, 3)
        started = json.loads((self.root / "watch-WINDOW-TEST-process.json").read_text(encoding="utf-8"))
        self.assertFalse(started["console_window_present"])
        ended = json.loads((self.root / "watch-WINDOW-TEST.json").read_text(encoding="utf-8"))
        self.assertEqual(ended["status"], "watch_deadline_exceeded")


if __name__ == "__main__":
    unittest.main()
