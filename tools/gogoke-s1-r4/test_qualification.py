"""Real unittest target for the R4 qualification group."""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.dont_write_bytecode = True


ROOT = Path(__file__).resolve().parents[2]
PLAN_ROOT = ROOT / "docs/design/gogoke-s1-r4-plan-v1"
RUNNER = ROOT / "tools/gogoke-s1-r4/run_checks.py"
BUILDER = ROOT / "tools/gogoke-s1-r4/build_qualification_manifest.py"


def runner_module():
    spec = importlib.util.spec_from_file_location("r4_qualification_runner", RUNNER)
    if spec is None or spec.loader is None:
        raise RuntimeError("runner module unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def builder_module():
    spec = importlib.util.spec_from_file_location("r4_qualification_builder", BUILDER)
    if spec is None or spec.loader is None:
        raise RuntimeError("builder module unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class R4QualificationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.runner = runner_module()
        cls.builder = builder_module()
        cls.plan = cls.runner.load_fixed_plan(PLAN_ROOT)

    def test_fixed_plan_has_no_git_byte_drift(self):
        self.assertEqual([], self.plan["errors"])

    def test_obligations_preserve_19_68_157_33_84_and_32_59(self):
        result = self.runner.verify_obligations(self.plan)
        self.assertTrue(result["ok"], result["errors"])
        self.assertEqual(19, result["observed"]["legacy_tasks"])
        self.assertEqual(68, result["observed"]["legacy_master_tests"])
        self.assertEqual(157, result["observed"]["legacy_deadlines"])
        self.assertEqual(33, result["observed"]["legacy_due"])
        self.assertEqual(84, result["observed"]["capability_behaviors"])
        self.assertEqual(32, result["observed"]["tasks"])
        self.assertEqual(59, result["observed"]["all_due"])

    def test_registry_has_all_fixed_groups_and_explicit_command_bindings(self):
        registry = self.runner.read_registry()
        result = self.runner.validate_registry(self.plan, registry)
        self.assertTrue(result["ok"], result["errors"])
        self.assertEqual(20, len(registry))
        self.assertGreaterEqual(result["command_count"], 23)

    def test_missing_fixture_targets_have_explicit_readiness(self):
        registry = self.runner.read_registry()
        missing = [
            command
            for commands in registry.values()
            for command in commands
            if not (self.runner.ROOT / command["target"]).is_file()
        ]
        self.assertEqual(16, len(missing))
        self.assertTrue(all(command["readiness"] != self.runner.READINESS_READY for command in missing))
        self.assertTrue(all(command["readiness_reason"] for command in missing))
        self.assertTrue(all(command["candidate_required"] for command in missing))
        self.assertTrue(all(command["evidence_layer"] == "QUALIFICATION_FIXTURE" for command in missing))

    def test_command_metadata_covers_exact_registry_command_set(self):
        raw = json.loads((ROOT / "tools/gogoke-s1-r4/registry.json").read_text(encoding="utf-8"))
        command_ids = {
            command["id"]
            for group in raw["groups"]
            for command in group["commands"]
        }
        self.assertEqual(command_ids, set(raw["command_metadata"]))
        self.assertEqual(23, len(raw["command_metadata"]))

    def synthetic_formal(self, root: Path):
        member = b'{"fixture":true}\n'
        archive = root / "FORMAL_REPORTS.zip"
        with zipfile.ZipFile(archive, "w") as output:
            output.writestr("fixture.json", member)
        archive_bytes = archive.read_bytes()
        groups = [{
            "group": "fixture",
            "status": "BLOCKED",
            "commands": [{
                "status": "BLOCKED", "discovered": 0, "executed": 0,
                "passed": 0, "failed": 0, "skipped": 0,
            }],
            "raw_report": {
                "archive_member": "fixture.json",
                "bytes": len(member),
                "sha256": self.builder.sha256_bytes(member),
            },
        }]
        results = {
            "group_count": 1,
            "command_count": 1,
            "unique_check_ids_observed": 1,
            "group_status_counts": {"BLOCKED": 1},
            "command_status_counts": {"BLOCKED": 1},
            "positive_tests_executed": 0,
            "positive_tests_passed": 0,
            "tests_failed": 0,
            "tests_skipped": 0,
            "official_due_checks_accepted": 0,
            "archive": {
                "path": "FORMAL_REPORTS.zip",
                "bytes": len(archive_bytes),
                "sha256": self.builder.sha256_bytes(archive_bytes),
            },
        }
        return results, groups, root / "RUN_RESULTS.json"

    def test_formal_results_recompute_aggregates_and_verify_archive_members(self):
        with tempfile.TemporaryDirectory() as temporary:
            results, groups, path = self.synthetic_formal(Path(temporary))
            self.builder.verify_formal_aggregates(results, groups, {"T-fixture"}, 1)
            self.builder.verify_formal_archive(results, path, groups)

    def test_formal_results_reject_forged_aggregate_and_archive_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            results, groups, path = self.synthetic_formal(Path(temporary))
            forged = json.loads(json.dumps(results))
            forged["positive_tests_passed"] = 1
            with self.assertRaises(RuntimeError):
                self.builder.verify_formal_aggregates(forged, groups, {"T-fixture"}, 1)
            forged = json.loads(json.dumps(results))
            forged["archive"]["sha256"] = "0" * 64
            with self.assertRaises(RuntimeError):
                self.builder.verify_formal_archive(forged, path, groups)


if __name__ == "__main__":
    unittest.main(verbosity=2)
