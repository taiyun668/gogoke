"""Real unittest target for R4 boundary/obligation evidence."""

from __future__ import annotations

import importlib.util
import copy
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.dont_write_bytecode = True


ROOT = Path(__file__).resolve().parents[2]
PLAN_ROOT = ROOT / "docs/design/gogoke-s1-r4-plan-v1"
RUNNER = ROOT / "tools/gogoke-s1-r4/run_checks.py"


def runner_module():
    spec = importlib.util.spec_from_file_location("r4_boundary_runner", RUNNER)
    if spec is None or spec.loader is None:
        raise RuntimeError("runner module unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class R4BoundaryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.runner = runner_module()
        cls.plan = cls.runner.load_fixed_plan(PLAN_ROOT)

    def test_framework_prose_cannot_be_a_python_result(self):
        parsed, error = self.runner.parse_python_unittest("Ran 1 test\nOK\n")
        self.assertIsNone(parsed)
        self.assertIn("enumerable", error or "")

    def test_registry_tags_join_every_due_check(self):
        result = self.runner.validate_registry(self.plan, self.runner.read_registry())
        self.assertTrue(result["ok"], result["errors"])
        for group, check_ids in result["expected_by_group"].items():
            if group != "qualification":
                self.assertTrue(set(check_ids).issubset(set(result["observed_by_group"][group])))

    def test_tap_requires_machine_counts(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "fake.tap"
            path.write_text("TAP version 13\n# process exited 0\n", encoding="utf-8")
            parsed, error = self.runner.parse_node_tap(path)
            self.assertIsNone(parsed)
            self.assertIn("complete", error or "")

    def test_T05_L_runner_obligation(self):
        result = self.runner.verify_obligations(self.plan)
        self.assertTrue(result["ok"], result["errors"])
        registry = self.runner.validate_registry(self.plan, self.runner.read_registry())
        self.assertIn("T05.L", registry["observed_by_group"]["codec"])

    def test_T60_L_source_selector_binding(self):
        registry = self.runner.read_registry()
        command = next(item for item in registry["boundary"] if item["id"] == "boundary-unittest")
        self.assertEqual("tools/gogoke-s1-r4/test_boundary.py", command["target"])
        self.assertEqual("gogoke-s1-r4/T60.L", command["planned_test_tags"]["T60.L"])

    def test_T61_L_negative_runner_instrument(self):
        parsed, error = self.runner.parse_python_unittest("Ran 1 test\nOK\n")
        self.assertIsNone(parsed)
        self.assertIn("enumerable", error or "")

    def test_T68_D_authorization_anchor(self):
        oid, _ = self.runner.git_oid(self.runner.git_identity()["commit"], self.runner.AUTH_REL)
        with mock.patch.dict("os.environ", {"GITHUB_TOKEN": ""}):
            auth, errors = self.runner.auth_from_candidate(self.runner.git_identity())
        if oid is None:
            self.assertIsNone(auth)
            self.assertTrue(errors)
        else:
            self.assertTrue(any("GITHUB_TOKEN unavailable" in error for error in errors))
            self.assertEqual("taiyun668/gogoke", auth["repository"])
            self.assertIsNone(auth["owner_merge_commit"])

    def test_source_diagnostic_fake_cannot_claim_native_or_owner_evidence(self):
        registry = copy.deepcopy(self.runner.read_registry())
        node = registry["sealing"][0]
        self.assertEqual("SOURCE_DIAGNOSTIC_FAKE", node["evidence_layer"])
        self.assertEqual("source_diagnostic_only", node["qualification_scope"])
        node["platform_requirement"] = "owner_machine"
        result = self.runner.validate_registry(self.plan, registry)
        self.assertFalse(result["ok"])
        self.assertTrue(any("source diagnostic fake" in error for error in result["errors"]))

    def test_missing_readiness_metadata_is_a_registry_error(self):
        registry = copy.deepcopy(self.runner.read_registry())
        missing = registry["store"][0]
        missing.pop("readiness", None)
        missing.pop("readiness_reason", None)
        result = self.runner.validate_registry(self.plan, registry)
        self.assertFalse(result["ok"])
        self.assertTrue(any("readiness" in error for error in result["errors"]))


if __name__ == "__main__":
    unittest.main(verbosity=2)
