"""Windows proof that required Python helpers have no console window."""

from __future__ import annotations

import json
import os
import shutil
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

if __name__ == "__main__":
    unittest.main()
