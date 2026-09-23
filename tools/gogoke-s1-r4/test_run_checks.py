#!/usr/bin/env python3
"""Adversarial self-tests for the R4 evidence runner."""

from __future__ import annotations

import copy
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.dont_write_bytecode = True


ROOT = Path(__file__).resolve().parents[2]
TOOL_ROOT = ROOT / "tools/gogoke-s1-r4"
PLAN_ROOT = ROOT / "docs/design/gogoke-s1-r4-plan-v1"
RUNNER_PATH = TOOL_ROOT / "run_checks.py"


def load_runner():
    spec = importlib.util.spec_from_file_location("gogoke_s1_r4_runner_tests", RUNNER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("runner module unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class RunnerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.runner = load_runner()
        cls.plan = cls.runner.load_fixed_plan(PLAN_ROOT)

    def test_exact_obligation_counts_and_ids(self):
        result = self.runner.verify_obligations(self.plan)
        self.assertTrue(result["ok"], result["errors"])
        observed = result["observed"]
        self.assertEqual(19, observed["legacy_tasks"])
        self.assertEqual(68, observed["legacy_master_tests"])
        self.assertEqual(157, observed["legacy_deadlines"])
        self.assertEqual(33, observed["legacy_due"])
        self.assertEqual(84, observed["capability_behaviors"])
        self.assertEqual(32, observed["tasks"])
        self.assertEqual(59, observed["all_due"])
        self.assertEqual(33, len(result["ids"]["legacy_due_ids"]))
        self.assertEqual(26, len(result["ids"]["new_due_ids"]))

    def test_registry_is_multiple_command_and_every_check_is_tagged(self):
        registry = self.runner.read_registry()
        result = self.runner.validate_registry(self.plan, registry)
        self.assertTrue(result["ok"], result["errors"])
        self.assertEqual(self.runner.GROUPS, tuple(registry))
        self.assertGreaterEqual(result["command_count"], 23)
        self.assertGreater(len(registry["codec"]), 1)
        self.assertGreater(len(registry["sealing"]), 1)

    def test_metadata_overlay_cannot_replace_a_conflicting_inline_fact(self):
        with self.assertRaises(self.runner.RunnerError):
            self.runner.merge_command_metadata(
                {"id": "command-one", "target": "trusted.test.ts"},
                {"target": "substituted.test.ts"},
                "fixture/command-one",
            )

    def test_one_group_command_cannot_mask_other_due_checks(self):
        registry = self.runner.read_registry()
        mutated = copy.deepcopy(registry)
        mutated["host"][0]["check_ids"] = ["T21.L"]
        mutated["host"][0]["planned_test_tags"] = {"T21.L": "gogoke-s1-r4/T21.L"}
        result = self.runner.validate_registry(self.plan, mutated)
        self.assertFalse(result["ok"])
        self.assertTrue(any("host: due checks have no exact registry command" in error for error in result["errors"]))

    def test_repeated_check_cannot_borrow_case_from_another_command(self):
        commands = [
            {"id": "first", "status": "PASS", "check_ids": ["T05.L"], "observed_cases": {"T05.L": ["marker-a"]}, "observed_names": ["marker-a"]},
            {"id": "second", "status": "PASS", "check_ids": ["T05.L"], "observed_cases": {"T05.L": ["marker-b"]}, "observed_names": ["marker-a"]},
        ]
        errors = self.runner.validate_command_case_bindings(commands)
        self.assertTrue(any("second/T05.L" in error for error in errors))

    def test_blocked_command_does_not_require_case_marker(self):
        commands = [{"id": "external-blocked", "status": "BLOCKED", "check_ids": ["T05.L"], "observed_cases": {"T05.L": ["marker"]}, "observed_names": []}]
        self.assertEqual([], self.runner.validate_command_case_bindings(commands))

    def test_build_report_blocked_first_plus_pass_second_uses_each_command_marker(self):
        registry = copy.deepcopy(self.runner.read_registry())
        registry["codec"][0]["observed_cases"] = {"T05.L": ["blocked-marker"]}
        registry["codec"][1]["observed_cases"] = {"T05.L": ["pass-marker"]}
        clean = self.runner.git_identity()
        clean["dirty"] = False
        clean["dirty_paths"] = []
        def fake_command(entry, _candidate, _temp):
            blocked = entry["id"] == registry["codec"][0]["id"]
            return {"id": entry["id"], "group": "codec", "check_ids": ["T05.L"], "planned_test_tags": {"T05.L": "gogoke-s1-r4/T05.L"}, "observed_cases": entry["observed_cases"], "selector": entry["selector"], "status": "BLOCKED" if blocked else "PASS", "reason": "simulated", "observed_names": [] if blocked else ["pass-marker"]}
        with mock.patch.object(self.runner, "git_identity", return_value=clean), mock.patch.object(self.runner, "bind_registry", return_value={"errors": [], "ok": True}), mock.patch.object(self.runner, "read_registry", return_value=registry), mock.patch.object(self.runner, "run_command", side_effect=fake_command), mock.patch.object(self.runner, "auth_and_plan_identity", return_value=({"authorization": {"fixture": True}}, [])):
            report = self.runner.build_report(self.plan, "codec", registry, invoked_argv=["runner"])
        self.assertEqual("BLOCKED", report["status"])
        self.assertFalse(any("case marker" in error for error in report["instrument_errors"]))

    def test_substituted_target_is_rejected(self):
        registry = self.runner.read_registry()
        mutated = copy.deepcopy(registry)
        mutated["codec"][0]["target"] = "README.md"
        result = self.runner.validate_registry(self.plan, mutated)
        self.assertFalse(result["ok"])
        self.assertTrue(any("substituted/mismatched" in error for error in result["errors"]))

    def test_readme_python_c_substitution_is_rejected(self):
        registry = self.runner.read_registry()
        mutated = copy.deepcopy(registry)
        command = mutated["boundary"][0]
        command["target"] = "README.md"
        command["argv"] = ["python", "-c", "print('Ran 1 test')"]
        result = self.runner.validate_registry(self.plan, mutated)
        self.assertFalse(result["ok"])
        self.assertTrue(any("exact selector is not in argv" in error for error in result["errors"]))

    def test_coherent_readme_python_c_registry_is_rejected_by_candidate_binding(self):
        registry = self.runner.read_registry()
        mutated = copy.deepcopy(registry)
        command = mutated["boundary"][0]
        command["selector"] = "README.md"
        command["target"] = "README.md"
        command["argv"] = ["python", "-c", "print('test_T05_L_runner_obligation (fixture) ... ok\\nRan 1 test\\nOK')", "README.md"]
        self.assertTrue(self.runner.validate_registry(self.plan, mutated)["ok"])
        clean_candidate = self.runner.git_identity()
        clean_candidate["dirty"] = False
        clean_candidate["dirty_paths"] = []
        with mock.patch.object(self.runner, "git_identity", return_value=clean_candidate):
            report = self.runner.build_report(self.plan, "boundary", mutated, invoked_argv=["runner"])
        self.assertEqual("FAIL_INSTRUMENT", report["status"])
        self.assertTrue(any("in-memory registry" in error or "registry bytes" in error for error in report["instrument_errors"]))
        self.assertTrue(all(command.get("reason") == "preflight failed; command not executed" for command in report["commands"]))

    def test_missing_tag_is_nonaccepting(self):
        registry = self.runner.read_registry()
        mutated = copy.deepcopy(registry)
        del mutated["codec"][0]["planned_test_tags"]["T05.L"]
        result = self.runner.validate_registry(self.plan, mutated)
        self.assertFalse(result["ok"])
        self.assertTrue(any("planned tag mismatch" in error for error in result["errors"]))

    def test_fake_prose_cannot_create_framework_counts(self):
        python_counts, python_error = self.runner.parse_python_unittest("Ran 1 test\nOK\n")
        self.assertIsNone(python_counts)
        self.assertIn("enumerable", python_error or "")
        with tempfile.TemporaryDirectory() as temporary:
            tap = Path(temporary) / "fake.tap"
            tap.write_text("TAP version 13\n# process exited 0\n", encoding="utf-8")
            tap_counts, tap_error = self.runner.parse_node_tap(tap)
            self.assertIsNone(tap_counts)
            self.assertIn("complete", tap_error or "")
        rust_counts, rust_error = self.runner.parse_rust_libtest("test result: ok. 1 passed; 0 failed; 0 ignored\n")
        self.assertIsNone(rust_counts)
        self.assertIn("zero", rust_error or "")

    def test_framework_count_contradictions_are_red(self):
        with tempfile.TemporaryDirectory() as temporary:
            vitest = Path(temporary) / "contradiction.json"
            vitest.write_text(json.dumps({"numTotalTests": 1, "numPassedTests": 1, "numFailedTests": 0, "numPendingTests": 0, "testResults": [{"assertionResults": [{"fullName": "case", "status": "failed"}]}]}), encoding="utf-8")
            counts, error = self.runner.parse_vitest_json(vitest)
            self.assertIsNone(counts)
            self.assertIn("contradict", error or "")
            tap = Path(temporary) / "contradiction.tap"
            tap.write_text("TAP version 13\nok 1 - case\n1..1\n# tests 1\n# pass 0\n# fail 1\n# skipped 0\n# todo 0\n", encoding="utf-8")
            counts, error = self.runner.parse_node_tap(tap)
            self.assertIsNone(counts)
            self.assertIn("invariant", error or "")

    def test_python_ok_skipped_nine_cross_checks_case_lines(self):
        lines = [f"test_case_{index} (fixture.Case) ... skipped" for index in range(9)]
        lines.append("Ran 9 tests")
        lines.append("OK (skipped=9)")
        counts, error = self.runner.parse_python_unittest("\n".join(lines))
        self.assertIsNotNone(counts)
        self.assertIsNone(error)
        self.assertEqual(9, counts["skipped"])
        self.assertEqual(0, counts["executed"])

    def test_environment_sanitizer_removes_credential_and_package_config_classes(self):
        injected = {
            "OPENAI_KEY": "secret",
            "AWS_ACCESS_KEY_ID": "secret",
            "NPM_CONFIG_USERCONFIG": "C:/tmp/secret/.npmrc",
            "HTTPS_PROXY": "http://proxy.invalid",
            "NORMAL_R4_TEST": "removed-by-fresh-allowlist",
        }
        with mock.patch.dict("os.environ", injected, clear=False):
            env, removed = self.runner.sanitized_environment()
        for key in injected:
            self.assertNotIn(key, env)
            self.assertIn(key, removed)
        self.assertNotIn("HOME", env)
        self.assertNotIn("USERPROFILE", env)
        self.assertNotIn("NODE_OPTIONS", env)
        self.assertIn("PATH", env)

    def test_node_options_preload_cannot_inject_or_replace_machine_output(self):
        node = self.runner.resolve_tool("node")
        if not node:
            self.skipTest("node is unavailable on this host")
        with tempfile.TemporaryDirectory() as temporary:
            preload = Path(temporary) / "preload.cjs"
            preload.write_text("console.log('PRELOADED')\n", encoding="utf-8")
            with mock.patch.dict("os.environ", {"NODE_OPTIONS": f"--require={preload}"}, clear=False):
                env, removed = self.runner.sanitized_environment(node)
            result = __import__("subprocess").run([node, "-e", "console.log('RUNNER_OUTPUT')"], env=env, capture_output=True, text=True, check=False)
            self.assertIn("NODE_OPTIONS", removed)
            self.assertNotIn("PRELOADED", result.stdout)
            self.assertIn("RUNNER_OUTPUT", result.stdout)

    def test_zero_and_skip_machine_results_fail_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            zero = Path(temporary) / "zero.json"
            zero.write_text(json.dumps({"numTotalTests": 0, "numPassedTests": 0, "numFailedTests": 0, "numPendingTests": 0, "testResults": []}), encoding="utf-8")
            counts, error = self.runner.parse_vitest_json(zero)
            self.assertIsNotNone(counts)
            status, _ = self.runner.command_status(counts, exit_code=0, target_committed=True, tool="node", error=error, timed_out=False)
            self.assertEqual("FAIL_INSTRUMENT", status)
            skipped = Path(temporary) / "skip.json"
            skipped.write_text(json.dumps({"numTotalTests": 1, "numPassedTests": 0, "numFailedTests": 0, "numPendingTests": 1, "testResults": [{"assertionResults": [{"fullName": "skip", "status": "pending"}]}]}), encoding="utf-8")
            counts, error = self.runner.parse_vitest_json(skipped)
            status, _ = self.runner.command_status(counts, exit_code=0, target_committed=True, tool="node", error=error, timed_out=False)
            self.assertEqual("FAIL_EXECUTION", status)

    def public_auth_fixture(self):
        binding = {"authorization_receipt_schema": "gogoke.s1-r4.public-authorization.v1"}
        auth = {
            "schema": binding["authorization_receipt_schema"],
            "owner_instruction": self.runner.AUTH_OWNER_INSTRUCTION,
            "validity": {
                "path": self.runner.AUTH_REL,
                "introduced_by": "a merge commit on taiyun668/gogoke main whose GitHub merged_by is taiyun668",
                "missing_stale_or_inconsistent": "FAIL_CLOSED",
            },
            "repository": "taiyun668/gogoke",
            "plan": {
                "provenance_plan_commit": self.runner.PLAN_COMMIT,
                "public_plan_path": f"{self.runner.PLAN_REL}/",
                "public_plan_manifest_blob": "a" * 40,
                "stale_when": "the public plan MANIFEST.json blob differs from the value above",
            },
            "carryover": {
                "source_archive_repository": "taiyun668/gogo-party",
                "source_archive_head": "b976e8f29f8d41adffa9ee60d3fe464a2fc3505e",
                "public_content_import_commit": "f3136b0a84086d7b5f77abb1f87e854648cd54e3",
                "public_carryover_checkpoint": "MC-001",
            },
            "authorized_gates": ["G0", "G1", "G2", "G3", "G4", "G5"],
            "continuous_after_gate_pass": True,
            "completion_claim": False,
        }
        return auth, binding

    def test_public_authorization_draft_rejects_extra_fields_and_stale_plan(self):
        auth, binding = self.public_auth_fixture()
        self.assertEqual([], self.runner.validate_auth_value(auth, binding, "a" * 40))
        forged = copy.deepcopy(auth)
        forged["live_budget"] = 1
        self.assertTrue(self.runner.validate_auth_value(forged, binding, "a" * 40))
        forged = copy.deepcopy(auth)
        forged["repository"] = "attacker/repo"
        self.assertTrue(self.runner.validate_auth_value(forged, binding, "a" * 40))
        forged = copy.deepcopy(auth)
        forged["owner_instruction"] = "arbitrary-string"
        self.assertTrue(self.runner.validate_auth_value(forged, binding, "a" * 40))
        forged = copy.deepcopy(auth)
        forged["plan"]["public_plan_manifest_blob"] = "b" * 40
        self.assertTrue(self.runner.validate_auth_value(forged, binding, "a" * 40))
        forged = copy.deepcopy(auth)
        forged["completion_claim"] = True
        self.assertTrue(self.runner.validate_auth_value(forged, binding, "a" * 40))

    def test_public_authorization_missing_fails_closed(self):
        _auth, binding = self.public_auth_fixture()
        with mock.patch.object(self.runner, "public_binding", return_value=(binding, "a" * 40, [])), mock.patch.object(self.runner, "git_oid", return_value=(None, None)):
            value, errors = self.runner.auth_from_candidate({"commit": "a" * 40})
        self.assertIsNone(value)
        self.assertTrue(any("not a committed blob" in error for error in errors))

    def test_plan_loaded_from_different_public_candidate_fails_closed(self):
        candidate = self.runner.git_identity()
        stale = copy.deepcopy(self.plan)
        stale["public_commit"] = "a" * 40
        _identity, errors = self.runner.auth_and_plan_identity(stale, candidate)
        self.assertTrue(any("different candidate HEAD" in error for error in errors))

    def test_github_owner_merge_lookup_fails_closed_on_non_owner_and_api_error(self):
        merge = "a" * 40
        associated = [{"merge_commit_sha": merge, "base": {"ref": "main", "repo": {"full_name": "taiyun668/gogoke"}}, "number": 17}]
        detail = {"merge_commit_sha": merge, "merged_at": "2026-09-22T00:00:00Z", "merged_by": {"login": "other"}, "base": {"ref": "main", "repo": {"full_name": "taiyun668/gogoke"}}}
        responses = [io.BytesIO(json.dumps(item).encode()) for item in (associated, detail)]
        with mock.patch.dict("os.environ", {"GITHUB_TOKEN": "fixture-only"}), mock.patch.object(self.runner.urllib.request, "urlopen", side_effect=responses):
            accepted, error = self.runner.owner_merged_public_authorization(merge)
        self.assertFalse(accepted)
        self.assertIn("no matching", error or "")
        with mock.patch.dict("os.environ", {"GITHUB_TOKEN": "fixture-only"}), mock.patch.object(self.runner.urllib.request, "urlopen", side_effect=OSError("offline")):
            accepted, error = self.runner.owner_merged_public_authorization(merge)
        self.assertFalse(accepted)
        self.assertIn("failed closed", error or "")

    def test_github_owner_merge_lookup_accepts_only_matching_main_merge(self):
        merge = "a" * 40
        associated = [{"merge_commit_sha": merge, "base": {"ref": "main", "repo": {"full_name": "taiyun668/gogoke"}}, "number": 17}]
        detail = {"merge_commit_sha": merge, "merged_at": "2026-09-22T00:00:00Z", "merged_by": {"login": "taiyun668"}, "base": {"ref": "main", "repo": {"full_name": "taiyun668/gogoke"}}}
        responses = [io.BytesIO(json.dumps(item).encode()) for item in (associated, detail)]
        with mock.patch.dict("os.environ", {"GITHUB_TOKEN": "fixture-only"}), mock.patch.object(self.runner.urllib.request, "urlopen", side_effect=responses):
            accepted, error = self.runner.owner_merged_public_authorization(merge)
        self.assertTrue(accepted)
        self.assertIsNone(error)

    def test_fixed_git_plan_ignores_worktree_crlf_drift(self):
        self.assertFalse(self.plan["errors"], self.plan["errors"])
        self.assertEqual(self.runner.PLAN_COMMIT, self.plan["commit"])
        self.assertEqual(self.plan["files"]["inputs/CAPABILITIES.tsv"]["sha256"], self.plan["manifest"]["sha256"]["inputs/CAPABILITIES.tsv"].upper())

    def test_dirty_candidate_cannot_produce_evidence_pass(self):
        dirty = self.runner.git_identity()
        dirty["dirty"] = True
        dirty["dirty_paths"] = ["untracked-for-negative-control.txt"]
        registry = self.runner.read_registry()
        with mock.patch.object(self.runner, "git_identity", return_value=dirty):
            report = self.runner.build_report(self.plan, "codec", registry, invoked_argv=["runner"])
        self.assertEqual("FAIL_INSTRUMENT", report["status"])
        self.assertTrue(any("dirty/untracked" in error for error in report["instrument_errors"]))
        self.assertTrue(all(command.get("reason") == "preflight failed; command not executed" for command in report["commands"]))

    def test_git_status_128_is_instrument_failure(self):
        broken = self.runner.git_identity()
        broken["dirty"] = False
        broken["dirty_paths"] = []
        broken["status_exit"] = 128
        broken["errors"] = ["fatal: simulated status failure"]
        with mock.patch.object(self.runner, "git_identity", return_value=broken):
            report = self.runner.build_report(self.plan, "codec", self.runner.read_registry(), invoked_argv=["runner"])
        self.assertEqual("FAIL_INSTRUMENT", report["status"])
        self.assertTrue(any("git status/identity failed" in error for error in report["instrument_errors"]))

    def test_output_rejects_repository_and_tracked_paths(self):
        with self.assertRaises(self.runner.RunnerError):
            self.runner.output_path(str(ROOT / "tools/gogoke-s1-r4/registry.json"))
        with self.assertRaises(self.runner.RunnerError):
            self.runner.output_path(str(ROOT / self.runner.AUTH_REL))
        with self.assertRaises(self.runner.RunnerError):
            self.runner.output_path("artifacts/s1-r4/runner.json")
        with tempfile.TemporaryDirectory() as temporary:
            accepted = self.runner.output_path(str(Path(temporary) / "evidence.json"))
            self.assertTrue(str(accepted).lower().endswith("evidence.json"))

    def test_missing_target_and_missing_tool_are_not_pass(self):
        entry = {
            "id": "missing-target",
            "group": "codec",
            "framework": "python_unittest",
            "cwd": ".",
            "argv": ["python", "missing-s1-r4-target.py"],
            "selector": "missing-s1-r4-target.py",
            "target": "missing-s1-r4-target.py",
            "check_ids": ["T05.L"],
            "planned_test_tags": {"T05.L": "gogoke-s1-r4/T05.L"},
        }
        clean_candidate = self.runner.git_identity()
        clean_candidate["dirty"] = False
        clean_candidate["dirty_paths"] = []
        with tempfile.TemporaryDirectory() as temporary:
            record = self.runner.run_command(entry, clean_candidate, Path(temporary))
        self.assertEqual("FAIL_INSTRUMENT", record["status"])
        missing_tool = copy.deepcopy(entry)
        missing_tool["target"] = "README.md"
        missing_tool["argv"] = ["no-such-tool", "README.md"]
        with tempfile.TemporaryDirectory() as temporary:
            with mock.patch.object(self.runner, "bind_worktree", return_value=([], [])):
                record = self.runner.run_command(missing_tool, clean_candidate, Path(temporary))
        self.assertEqual("BLOCKED", record["status"])

    def test_explicit_nonready_target_blocks_before_external_probe(self):
        entry = copy.deepcopy(self.runner.read_registry()["store"][0])
        entry["group"] = "store"
        candidate = self.runner.git_identity()
        candidate["dirty"] = False
        candidate["dirty_paths"] = []
        with tempfile.TemporaryDirectory() as temporary:
            with mock.patch.object(self.runner, "tool_identity", side_effect=AssertionError("non-ready target must not probe tools")):
                record = self.runner.run_command(entry, candidate, Path(temporary))
        self.assertEqual("BLOCKED", record["status"])
        self.assertEqual("NEEDS_CURRENT_CANDIDATE", record["readiness"])
        self.assertEqual("QUALIFICATION_FIXTURE", record["evidence_layer"])
        self.assertIn("readiness NEEDS_CURRENT_CANDIDATE", record["reason"])
        self.assertFalse(record["target_candidate"]["committed"])

    def test_source_fake_metadata_is_carried_without_native_claim(self):
        entry = copy.deepcopy(self.runner.read_registry()["sealing"][0])
        self.assertEqual("SOURCE_DIAGNOSTIC_FAKE", entry["evidence_layer"])
        self.assertEqual("source_diagnostic_only", entry["qualification_scope"])
        self.assertEqual("host", entry["platform_requirement"])

    def test_external_frameworks_block_without_controller_receipt(self):
        entry = copy.deepcopy(self.runner.read_registry()["codec"][0])
        entry["group"] = "codec"
        clean_candidate = self.runner.git_identity()
        clean_candidate["dirty"] = False
        clean_candidate["dirty_paths"] = []
        with tempfile.TemporaryDirectory() as temporary:
            with (
                mock.patch.object(self.runner, "bind_worktree", return_value=([], [])),
                mock.patch.object(
                    self.runner,
                    "tool_identity",
                    side_effect=AssertionError("blocked external tool must not be probed"),
                ),
            ):
                record = self.runner.run_command(entry, clean_candidate, Path(temporary))
        self.assertEqual("BLOCKED", record["status"])
        self.assertIn("Controller-qualified toolchain receipt", record["reason"])
        self.assertEqual("external_requires_controller_receipt", record["dependency_identity"]["trust_class"])
        self.assertEqual("NOT_PROBED_UNQUALIFIED", record["tool_identity"]["status"])

    def test_schema_and_candidate_only_receipt_is_not_trusted(self):
        entry = copy.deepcopy(self.runner.read_registry()["codec"][0])
        candidate = self.runner.git_identity()
        malformed = {"schema": self.runner.TOOLCHAIN_RECEIPT_SCHEMA, "candidate_commit": candidate["commit"], "entries": {}}
        errors = self.runner.validate_toolchain_receipt(malformed, entry, candidate, {entry["id"]})
        self.assertTrue(errors)
        self.assertTrue(any("missing" in error or "entry" in error for error in errors))

    def test_all_receipt_entries_are_validated_not_only_current(self):
        registry = self.runner.read_registry()
        entry = registry["codec"][0]
        candidate = self.runner.git_identity()
        entries = {command["id"]: None for group in registry.values() for command in group}
        receipt = {"schema": self.runner.TOOLCHAIN_RECEIPT_SCHEMA, "qualified_base_commit": candidate["commit"], "entries": entries}
        errors = self.runner.validate_all_toolchain_receipt(receipt, candidate, registry)
        self.assertTrue(errors)
        self.assertTrue(any("command entry is missing" in error for error in errors))

    def test_qualified_base_containing_old_receipt_is_rejected(self):
        candidate = self.runner.git_identity()
        value = {"qualified_base_commit": candidate["commit"]}
        with mock.patch.object(self.runner, "git_is_ancestor", return_value=(True, None)), mock.patch.object(self.runner, "git_diff_paths", return_value=([self.runner.TOOLCHAIN_RECEIPT_REL], None)), mock.patch.object(self.runner, "git_oid", return_value=("old-receipt-oid", None)):
            errors = self.runner.validate_qualified_base(value, candidate)
        self.assertTrue(any("already contains" in error for error in errors))

    def test_malformed_all_receipt_entries_return_errors_without_throwing(self):
        entry = copy.deepcopy(self.runner.read_registry()["codec"][0])
        candidate = self.runner.git_identity()
        malformed = {"schema": self.runner.TOOLCHAIN_RECEIPT_SCHEMA, "qualified_base_commit": candidate["commit"], "entries": {entry["id"]: None}}
        errors = self.runner.validate_toolchain_receipt(malformed, entry, candidate, {entry["id"]})
        self.assertTrue(errors)
        observed, materialization_errors = self.runner.verify_toolchain_materialization({"primary_executable": None}, entry, candidate, "before")
        self.assertTrue(materialization_errors)
        self.assertIsInstance(observed, dict)

    def test_reachable_valid_receipt_structure_simulation(self):
        entry = copy.deepcopy(self.runner.read_registry()["sealing"][0])
        candidate = self.runner.git_identity()
        node = self.runner.resolve_tool("node")
        self.assertIsNotNone(node)
        bound_hashes = {relative: self.runner.candidate_blob_record(candidate["commit"], relative)["sha256"] for relative in self.runner.bound_paths(entry, candidate["commit"])}
        lock_hashes = {relative: self.runner.candidate_blob_record(candidate["commit"], relative)["sha256"] for relative in self.runner.expected_lockfiles(entry)}
        item = {
            "state": "QUALIFIED",
            "command_id": entry["id"],
            "framework": entry["framework"],
            "argv_sha256": self.runner.digest_json(entry["argv"]),
            "primary_executable": {"path": node, "sha256": self.runner.sha256_bytes(Path(node).read_bytes())},
            "runtime_executables": [],
            "wrapper_entry_files": [],
            "dependency_closure_roots": [],
            "lockfile_sha256": {},
            "bound_path_sha256": bound_hashes,
        }
        receipt = {"schema": self.runner.TOOLCHAIN_RECEIPT_SCHEMA, "qualified_base_commit": candidate["commit"], "entries": {entry["id"]: item}}
        self.assertEqual([], self.runner.validate_toolchain_receipt(receipt, entry, candidate, {entry["id"]}))

    def test_node_tap_empty_closure_is_reachable_but_rust_empty_closure_is_blocked(self):
        candidate = self.runner.git_identity()
        node_entry = copy.deepcopy(self.runner.read_registry()["sealing"][0])
        node = self.runner.resolve_tool("node")
        self.assertIsNotNone(node)
        bound = {
            relative: self.runner.candidate_blob_record(candidate["commit"], relative)["sha256"]
            for relative in self.runner.bound_paths(node_entry, candidate["commit"])
        }
        item = {"state": "QUALIFIED", "command_id": node_entry["id"], "framework": node_entry["framework"], "argv_sha256": self.runner.digest_json(node_entry["argv"]), "primary_executable": {"path": node, "sha256": self.runner.sha256_bytes(Path(node).read_bytes())}, "runtime_executables": [], "wrapper_entry_files": [], "dependency_closure_roots": [], "lockfile_sha256": {}, "bound_path_sha256": bound}
        receipt = {"schema": self.runner.TOOLCHAIN_RECEIPT_SCHEMA, "qualified_base_commit": candidate["commit"], "entries": {node_entry["id"]: item}}
        self.assertEqual([], self.runner.validate_toolchain_receipt(receipt, node_entry, candidate, {node_entry["id"]}))
        rust_entry = copy.deepcopy(self.runner.read_registry()["sealing"][2])
        rust_item = copy.deepcopy(item)
        rust_item["state"] = "BLOCKED_UNQUALIFIED"
        rust_item.pop("argv_sha256", None)
        rust_item.pop("primary_executable", None)
        rust_item.pop("runtime_executables", None)
        rust_item.pop("wrapper_entry_files", None)
        rust_item.pop("dependency_closure_roots", None)
        rust_item.pop("lockfile_sha256", None)
        rust_item.pop("bound_path_sha256", None)
        rust_item["reason_code"] = "RUST_CLOSURE_NOT_QUALIFIED"
        rust_item["command_id"] = rust_entry["id"]
        rust_item["framework"] = rust_entry["framework"]
        rust_receipt = {"schema": self.runner.TOOLCHAIN_RECEIPT_SCHEMA, "qualified_base_commit": candidate["commit"], "entries": {rust_entry["id"]: rust_item}}
        self.assertEqual([], self.runner.validate_toolchain_receipt(rust_receipt, rust_entry, candidate, {rust_entry["id"]}))

    def test_full_registry_node_qualified_rust_blocked_union(self):
        registry = self.runner.read_registry()
        candidate = self.runner.git_identity()
        node_entry = copy.deepcopy(registry["sealing"][0])
        node = self.runner.resolve_tool("node")
        bound = {
            relative: self.runner.candidate_blob_record(candidate["commit"], relative)["sha256"]
            for relative in self.runner.bound_paths(node_entry, candidate["commit"])
        }
        qualified = {"state": "QUALIFIED", "command_id": node_entry["id"], "framework": node_entry["framework"], "argv_sha256": self.runner.digest_json(node_entry["argv"]), "primary_executable": {"path": node, "sha256": self.runner.sha256_bytes(Path(node).read_bytes())}, "runtime_executables": [], "wrapper_entry_files": [], "dependency_closure_roots": [], "lockfile_sha256": {}, "bound_path_sha256": bound}
        entries = {}
        for group in registry.values():
            for command in group:
                if command["id"] == node_entry["id"]:
                    entries[command["id"]] = qualified
                elif command["framework"] == "rust_libtest":
                    entries[command["id"]] = {"state": "BLOCKED_UNQUALIFIED", "command_id": command["id"], "framework": command["framework"], "reason_code": "RUST_CLOSURE_NOT_QUALIFIED"}
                else:
                    entries[command["id"]] = {"state": "BLOCKED_UNQUALIFIED", "command_id": command["id"], "framework": command["framework"], "reason_code": "TOOLCHAIN_NOT_QUALIFIED"}
        receipt = {"schema": self.runner.TOOLCHAIN_RECEIPT_SCHEMA, "qualified_base_commit": candidate["commit"], "entries": entries}
        self.assertEqual([], self.runner.validate_all_toolchain_receipt(receipt, candidate, registry))
        with tempfile.TemporaryDirectory() as temporary:
            materialized_root = Path(temporary)
            for relative in bound:
                path = materialized_root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(self.runner.git_bytes(candidate["commit"], relative))

            def current_blob(_commit, relative):
                expected = bound.get(relative)
                return {
                    "path": relative,
                    "committed": expected is not None,
                    "sha256": expected,
                }

            with (
                mock.patch.object(self.runner, "ROOT", materialized_root),
                mock.patch.object(self.runner, "candidate_blob_record", side_effect=current_blob),
            ):
                activation = self.runner.activate_toolchain_materialization(
                    qualified, node_entry, candidate, "before"
                )
        self.assertTrue(activation["proven"], activation["errors"])
        rust_entry = next(command for command in registry["sealing"] if command["framework"] == "rust_libtest")
        self.assertEqual("BLOCKED_UNQUALIFIED", entries[rust_entry["id"]]["state"])

    def test_real_temp_closure_activation_uses_full_tree(self):
        entry = {"id": "simulated", "framework": "vitest_json", "cwd": "third_party/t3code", "argv": ["pnpm", "exec"], "target": "README.md", "bind_paths": []}
        candidate = {"commit": "simulated"}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            closure = root / "node_modules"
            (closure / "excluded").mkdir(parents=True)
            (closure / "keep.txt").write_text("keep", encoding="utf-8")
            (closure / "excluded" / "cache.txt").write_text("cache-v1", encoding="utf-8")
            lock = root / "lock.json"
            lock.write_text("lock", encoding="utf-8")
            bound = root / "bound.txt"
            bound.write_text("bound", encoding="utf-8")
            executable = Path(sys.executable)
            def fake_blob(_commit, relative):
                path = root / relative
                if not path.is_file():
                    return {"path": relative, "committed": False, "sha256": None}
                return {"path": relative, "committed": True, "sha256": self.runner.sha256_bytes(path.read_bytes())}
            item = {
                "primary_executable": {"path": str(executable), "sha256": self.runner.sha256_bytes(executable.read_bytes())},
                "runtime_executables": [],
                "wrapper_entry_files": [],
                "dependency_closure_roots": [{"path": str(closure), "tree_sha256": self.runner.deterministic_tree_digest(closure, [])[0], "exclusions": []}],
                "lockfile_sha256": {"lock.json": self.runner.sha256_bytes(lock.read_bytes())},
                "bound_path_sha256": {"bound.txt": self.runner.sha256_bytes(bound.read_bytes())},
            }
            with mock.patch.object(self.runner, "ROOT", root), mock.patch.object(self.runner, "candidate_blob_record", side_effect=fake_blob):
                activation = self.runner.activate_toolchain_materialization(item, entry, candidate, "before")
                self.assertTrue(activation["proven"], activation["errors"])
                self.assertEqual([], activation["errors"])
                first_digest = activation["observed"]["dependency_closure_roots"][0]["tree_sha256"]
                (closure / "excluded" / "cache.txt").write_text("cache-v2", encoding="utf-8")
                second_digest = self.runner.deterministic_tree_digest(closure, [])[0]
                self.assertNotEqual(first_digest, second_digest)
                (closure / "keep.txt").write_text("changed", encoding="utf-8")
                self.assertNotEqual(second_digest, self.runner.deterministic_tree_digest(closure, [])[0])

    def test_bound_directory_uses_candidate_tree_not_sparse_worktree(self):
        entry = copy.deepcopy(self.runner.read_registry()["codec"][1])
        candidate = self.runner.git_identity()
        full = self.runner.bound_paths(entry, candidate["commit"])
        with mock.patch.object(Path, "is_dir", side_effect=AssertionError("bound enumeration must not inspect sparse worktree directories")):
            sparse_independent = self.runner.bound_paths(entry, candidate["commit"])
        self.assertEqual(full, sparse_independent)
        self.assertTrue(any(path.startswith("apps/desktop/contracts/s1/fixtures/") for path in full))


if __name__ == "__main__":
    unittest.main(verbosity=2)
