from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PACK_TOOL = ROOT / "tools" / "ci" / "gogoke_resource_pack.py"
FREEZE_TOOL = ROOT / "tools" / "ci" / "gogoke_ci_frozen_artifact.py"
SOURCE = "0123456789abcdef0123456789abcdef01234567"


class FrozenArtifactTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.frontend = self.root / "frontend"
        self.service = self.root / "service"
        self.frontend.mkdir()
        self.service.mkdir()
        (self.frontend / "index.html").write_text("<main>fixture</main>\n", encoding="utf-8")
        (self.service / "bin.mjs").write_text("export default 1;\n", encoding="utf-8")
        self.shell = self.root / "gogoke.exe"
        self.shell.write_bytes(b"head__TAURI_BUNDLE_TYPE_VAR_UNKtail")
        self.native = self.root / "gogoke-native-host.exe"
        self.native.write_bytes(b"native")
        self.node = self.root / "node.exe"
        self.node.write_bytes(b"node")
        self.installer = self.root / "setup.exe"
        self.installer.write_bytes(b"installer")
        self.inventory = self.root / "inventory.json"
        self.inventory.write_text("{}\n", encoding="utf-8")
        self.installed_root = self.root / "installed-static"
        (self.installed_root / "gogoke-service" / "node_modules").mkdir(parents=True)
        (self.installed_root / "gogoke-service" / "node_modules" / "package.json").write_text(
            "{}\n", encoding="utf-8"
        )
        self.pack = self.root / "gogoke-resources.windows.zip"
        self.index = self.root / "resource-index.json"
        self.run_tool(PACK_TOOL, "pack", "--frontend-dir", self.frontend,
                      "--service-dist-dir", self.service, "--output", self.pack)
        self.run_tool(PACK_TOOL, "index", "--pack", self.pack,
                      "--portable-shell", self.shell, "--native-host", self.native,
                      "--node", self.node, "--source-commit", SOURCE,
                      "--version", "1.2.3", "--installed-root", self.installed_root,
                      "--output", self.index)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def run_tool(self, tool: Path, *args: object, check: bool = True) -> subprocess.CompletedProcess[str]:
        result = subprocess.run(
            [sys.executable, str(tool), *(str(value) for value in args)],
            check=False,
            capture_output=True,
            text=True,
        )
        if check and result.returncode != 0:
            self.fail(result.stderr)
        return result

    def stage(self, lane: str, destination: Path) -> None:
        self.run_tool(
            FREEZE_TOOL, "stage", "--pack", self.pack, "--index", self.index,
            "--portable-shell", self.shell, "--native-host", self.native,
            "--node", self.node, "--installer", self.installer,
            "--package-inventory", self.inventory, "--source-commit", SOURCE,
            "--run-id", 42, "--run-attempt", 3, "--lane", lane,
            "--output", destination,
        )

    def test_stage_and_compare_bind_exact_independent_bytes(self) -> None:
        frozen = self.root / "frozen"
        repro = self.root / "repro"
        self.stage("frozen", frozen)
        self.stage("repro", repro)
        report = self.root / "comparison.json"
        self.run_tool(
            FREEZE_TOOL, "compare", "--frozen", frozen, "--repro", repro,
            "--source-commit", SOURCE, "--run-id", 42, "--run-attempt", 3,
            "--output", report,
        )
        value = json.loads(report.read_text(encoding="utf-8"))
        self.assertEqual(value["state"], "PASS")
        self.assertEqual(value["installedCandidateSmoke"]["state"], "NOT_RUN")
        self.assertEqual(
            (frozen / "gogoke-installed-shell.nsis.exe").read_bytes(),
            b"head__TAURI_BUNDLE_TYPE_VAR_NSStail",
        )

    def test_compare_rejects_one_byte_native_difference(self) -> None:
        frozen = self.root / "frozen"
        repro = self.root / "repro"
        self.stage("frozen", frozen)
        self.stage("repro", repro)
        (repro / "gogoke-native-host.exe").write_bytes(b"changed")
        result = self.run_tool(
            FREEZE_TOOL, "compare", "--frozen", frozen, "--repro", repro,
            "--source-commit", SOURCE, "--run-id", 42, "--run-attempt", 3,
            "--output", self.root / "comparison.json", check=False,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("frozen file identity mismatch", result.stderr)

    def test_verify_rejects_unlisted_artifact_file(self) -> None:
        frozen = self.root / "frozen"
        self.stage("frozen", frozen)
        (frozen / "untrusted.ps1").write_text("throw 'must not run'\n", encoding="utf-8")
        result = self.run_tool(
            FREEZE_TOOL, "verify", "--directory", frozen,
            "--source-commit", SOURCE, "--run-id", 42, "--run-attempt", 3,
            "--lane", "frozen", check=False,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("missing or unexpected files", result.stderr)

    def test_verify_rejects_duplicate_metadata_keys(self) -> None:
        frozen = self.root / "frozen"
        self.stage("frozen", frozen)
        metadata = frozen / "frozen-build.json"
        metadata.write_text(
            '{"schema":"forged",' + metadata.read_text(encoding="utf-8").lstrip()[1:],
            encoding="utf-8",
        )
        result = self.run_tool(
            FREEZE_TOOL, "verify", "--directory", frozen,
            "--source-commit", SOURCE, "--run-id", 42, "--run-attempt", 3,
            "--lane", "frozen", check=False,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("duplicate frozen metadata key", result.stderr)


if __name__ == "__main__":
    unittest.main()
