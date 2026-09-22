import unittest
import importlib.util
import os
import subprocess
import tempfile
from pathlib import Path

SCANNER_PATH = Path(__file__).resolve().parents[1] / "check-public-source.py"
SPEC = importlib.util.spec_from_file_location("check_public_source", SCANNER_PATH)
assert SPEC is not None and SPEC.loader is not None
SCANNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SCANNER)
findings = SCANNER.findings


class PublicSourceScannerTests(unittest.TestCase):
    def test_current_user_path_is_a_leak_even_inside_a_test_file(self):
        owner_path = "C:" + chr(92) + "Users" + chr(92) + Path.home().name + chr(92) + "Documents"
        results = findings("path: " + owner_path, "apps/desktop/src/leak.test.ts")
        self.assertIn((1, "machine-path", "LEAK", "current-user-profile-path"), results)

    def test_unknown_user_path_still_leaks_inside_rust_test_region(self):
        value = "C:" + chr(92) + "Users" + chr(92) + "gogoke-private-owner" + chr(92) + "data"
        results = findings("#[cfg(test)]\nlet path = \"" + value + "\";", "apps/desktop/native-host/src/example.rs")
        self.assertIn((2, "machine-path", "LEAK", "user-profile-path"), results)

    def test_unknown_user_path_still_leaks_inside_donor_test(self):
        value = "C:" + chr(92) + "Users" + chr(92) + "gogoke-private-owner" + chr(92) + "data"
        results = findings("path: " + value, "third_party/t3code/apps/server/src/example.test.ts")
        self.assertIn((1, "machine-path", "LEAK", "user-profile-path"), results)

    def test_detects_machine_paths_tokens_and_private_keys(self):
        import base64

        key_body = base64.b64encode(b"synthetic-private-key-material-that-is-not-secret-" * 3).decode("ascii")
        sample = "\n".join(
            (
                "path: " + "C:" + "\\Users\\gogoke-control-user\\private.txt",
                "token: " + "gh" + "p_" + "A" * 36,
                "-----BEGIN PRIVATE KEY-----",
                key_body,
                "-----END PRIVATE KEY-----",
            )
        )
        detected = {(rule, status) for _, rule, status, _ in findings(sample)}
        self.assertEqual(
            detected,
            {
                ("machine-path", "LEAK"),
                ("credential-token", "LEAK"),
                ("private-key-marker", "EXPLAINED"),
                ("private-key-block", "LEAK"),
            },
        )

    def test_reports_synthetic_paths_and_tokens_as_explained(self):
        sample = "runner: C:/Program Files/LLVM/bin\ntemp: C:/tmp/one\nhome: /home/fred/project\nwindows: C:\\\\Windows\ntoken: AKIAIOSFODNN7EXAMPLE"
        detected = {(rule, status, reason) for _, rule, status, reason in findings(sample)}
        self.assertEqual(
            detected,
            {
                ("machine-path", "EXPLAINED", "system-or-synthetic-path"),
                ("machine-path", "EXPLAINED", "system-or-synthetic-path"),
                ("machine-path", "EXPLAINED", "system-or-synthetic-path"),
                ("credential-token", "EXPLAINED", "synthetic-token"),
            },
        )

    def test_marker_without_valid_pem_body_is_explained_only(self):
        sample = "-----BEGIN " + "PRIVATE KEY----- ... -----END " + "PRIVATE KEY-----"
        detected = {(rule, status) for _, rule, status, _ in findings(sample)}
        self.assertEqual(detected, {("private-key-marker", "EXPLAINED")})

    def test_token_and_full_key_block_still_fail_in_test_sources(self):
        import base64

        key_body = base64.b64encode(b"test-private-key-material-long-enough-for-validation-" * 3).decode("ascii")
        sample = "\n".join(
            (
                "token: " + "gh" + "p_" + "B" * 36,
                "-----BEGIN PRIVATE KEY-----",
                key_body,
                "-----END PRIVATE KEY-----",
            )
        )
        detected = {(rule, status) for _, rule, status, _ in findings(sample, "src/credential.test.ts")}
        self.assertIn(("credential-token", "LEAK"), detected)
        self.assertIn(("private-key-block", "LEAK"), detected)

    def test_complete_key_block_in_donor_fixture_is_rejected(self):
        import base64

        key_body = base64.b64encode(b"test-private-key-material-long-enough-for-validation-" * 3).decode("ascii")
        sample = "\n".join(("-----BEGIN PRIVATE KEY-----", key_body, "-----END PRIVATE KEY-----"))
        path = "third_party/t3code/.repos/alchemy-effect/packages/alchemy/test/AWS/ACM/fixtures/import-cert.ts"
        detected = {(rule, status, reason) for _, rule, status, reason in findings(sample, path)}
        self.assertIn(("private-key-block", "LEAK", "complete-base64-private-key-block"), detected)

    def test_allows_documented_environment_placeholders(self):
        sample = "Use %USERPROFILE% and %LOCALAPPDATA% for user-specific locations."
        self.assertEqual(findings(sample), [])

    def test_committed_scan_reads_blob_not_changed_worktree(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get("GOGOKE_TEST_TEMP_ROOT")) as temporary:
            root = Path(temporary)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.name", "Scanner Fixture"], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.email", "scanner-fixture@example.invalid"], check=True)
            source = root / "record.txt"
            source.write_text("committed-only-value", encoding="utf-8")
            subprocess.run(["git", "-C", str(root), "add", "--", "record.txt"], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "-q", "-m", "fixture"], check=True)
            source.write_text("changed-worktree-value", encoding="utf-8")
            observed = list(SCANNER.committed_blobs(root))
            self.assertEqual(observed, [(source, b"committed-only-value")])


if __name__ == "__main__":
    unittest.main()
