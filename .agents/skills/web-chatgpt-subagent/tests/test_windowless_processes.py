"""Windows proof that required Python helpers have no console window."""

from __future__ import annotations

import json
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import time
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
            "--task", "WINDOW-TEST", "--tier", "gpt-6-pro", "--repo", "example/example", "--branch", "gpt/example",
            "--path", "result.md", "--base-sha", "a" * 40,
            "--thread", "00000000-0000-0000-0000-000000000000", "--max-hours", "0",
        )
        self.assertEqual(code, 3)
        started = json.loads((self.root / "watch-WINDOW-TEST-process.json").read_text(encoding="utf-8"))
        self.assertFalse(started["console_window_present"])
        ended = json.loads((self.root / "watch-WINDOW-TEST.json").read_text(encoding="utf-8"))
        self.assertEqual(ended["status"], "watch_deadline_exceeded")

    def test_watcher_uses_slow_pro_interval(self):
        spec = importlib.util.spec_from_file_location("watch_github_result", SCRIPTS / "watch_github_result.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.assertEqual(module.poll_seconds("gpt-6-pro", 0), 90)
        self.assertEqual(module.poll_seconds("gpt-6-pro", 1201), 300)
        with self.assertRaises(ValueError):
            module.poll_seconds("medium", 0)

    def test_powershell_ledger_wrapper_propagates_failure_and_has_no_console(self):
        pwsh = shutil.which("pwsh")
        self.assertIsNotNone(pwsh)
        wrapper = SCRIPTS / "Invoke-LedgerHidden.ps1"
        denied = subprocess.run(
            [pwsh, "-NoProfile", "-File", str(wrapper), "reserve", "--tier", "gpt-6-pro", "--task", "before-reconcile", "--seat", "test", "--original-seat", "astra", "--economics", "test"],
            env=self.env, text=True, capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW,
        )
        self.assertNotEqual(denied.returncode, 0)
        self.assertIn("Ledger command failed", denied.stderr)
        reconciled = subprocess.run(
            [pwsh, "-NoProfile", "-File", str(wrapper), "reconcile", "--source", "windowless-wrapper-test"],
            env=self.env, text=True, capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW,
        )
        self.assertEqual(reconciled.returncode, 0, reconciled.stderr)
        receipt = json.loads(reconciled.stdout)
        self.assertEqual(receipt["exit_code"], 0)
        self.assertFalse(receipt["console_window_present"])

    def test_powershell_watcher_wrapper_starts_windowless_process(self):
        pwsh = shutil.which("pwsh")
        self.assertIsNotNone(pwsh)
        wrapper = SCRIPTS / "Start-WatcherHidden.ps1"
        started = subprocess.run(
            [pwsh, "-NoProfile", "-File", str(wrapper), "-Task", "WINDOW-PS", "-Tier", "gpt-6-pro", "-Repo", "example/example", "-Branch", "gpt/example", "-Path", "result.md", "-BaseSha", "a" * 40, "-Thread", "00000000-0000-0000-0000-000000000000", "-MaxHours", "0"],
            env=self.env, text=True, capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW,
        )
        self.assertEqual(started.returncode, 0, started.stderr)
        self.assertEqual(json.loads(started.stdout)["console"], "pythonw")
        receipt = self.root / "watch-WINDOW-PS-process.json"
        for _ in range(100):
            if receipt.is_file():
                break
            time.sleep(0.05)
        self.assertTrue(receipt.is_file())
        self.assertFalse(json.loads(receipt.read_text(encoding="utf-8"))["console_window_present"])

if __name__ == "__main__":
    unittest.main()
