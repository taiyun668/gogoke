"""Exercise the trusted signer's untrusted event-data boundary without any key."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/gogoke-candidate-sign.yml"


def identity_script() -> str:
    text = WORKFLOW.read_text(encoding="utf-8")
    start = text.index("      - name: Resolve exact controlled source run and artifact identity\n")
    start = text.index("        run: |\n", start) + len("        run: |\n")
    end = text.index("      - name: Download exact frozen archive", start)
    lines = text[start:end].splitlines()
    if any(line and not line.startswith("          ") for line in lines):
        raise AssertionError("identity run block has unexpected YAML indentation")
    return "\n".join(line[10:] if line else "" for line in lines)


class CandidateSigningBoundaryTests(unittest.TestCase):
    def test_dispatch_input_is_data_and_rejected_before_any_command(self) -> None:
        shell = shutil.which("pwsh")
        self.assertIsNotNone(shell, "trusted signer CI requires PowerShell")
        script = identity_script()
        self.assertNotIn("${{ inputs.", script)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            marker = root / "injected-statement.txt"
            hostile = (
                "0123456789abcdef0123456789abcdef01234567';\n"
                f"[IO.File]::WriteAllText('{marker.as_posix()}', 'executed'); #"
            )
            event = root / "event.json"
            event.write_text(json.dumps({"inputs": {
                "source_commit": hostile,
                "run_id": "1",
                "run_attempt": "1",
                "artifact_id": "1",
            }}), encoding="utf-8")
            environment = os.environ.copy()
            environment["GITHUB_EVENT_PATH"] = str(event)
            environment["GITHUB_EVENT_NAME"] = "workflow_dispatch"
            result = subprocess.run(
                [shell, "-NoProfile", "-NonInteractive", "-Command", script],
                cwd=ROOT,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Candidate signing identity is malformed", result.stdout + result.stderr)
            self.assertFalse(marker.exists(), "untrusted source_commit executed as PowerShell")


if __name__ == "__main__":
    unittest.main()
