"""Exercise the trusted signer's untrusted event-data boundary without any key."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/gogoke-candidate-sign.yml"
SIGNER = ROOT / "tools/ci/gogoke-candidate-sign.ps1"
RESOURCE_TOOL = ROOT / "tools/ci/gogoke_resource_pack.py"
SOURCE = "0123456789abcdef0123456789abcdef01234567"


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

    def test_actual_signer_binds_candidate_bytes_and_rejects_wrong_key(self) -> None:
        shell = shutil.which("pwsh")
        self.assertIsNotNone(shell, "trusted signer CI requires PowerShell")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            repo = root / "trusted-repo"
            tool_dir = repo / "tools/ci"
            tool_dir.mkdir(parents=True)
            shutil.copyfile(SIGNER, tool_dir / SIGNER.name)
            shutil.copyfile(RESOURCE_TOOL, tool_dir / RESOURCE_TOOL.name)
            public_path = repo / "apps/desktop/src-tauri/gogoke-candidate-public-key.txt"
            public_path.parent.mkdir(parents=True)
            frontend = root / "frontend"
            service = root / "service"
            installed = root / "installed"
            artifact = root / "artifact"
            for directory in (frontend, service, installed, artifact):
                directory.mkdir()
            (frontend / "index.html").write_bytes(b"<main>candidate</main>\n")
            (service / "bin.mjs").write_bytes(b"console.log('fixture');\n")
            installed_file = installed / "gogoke-service/node_modules/package.json"
            installed_file.parent.mkdir(parents=True)
            installed_file.write_bytes(b"{}\n")
            portable = root / "portable.exe"
            portable.write_bytes(b"shell:__TAURI_BUNDLE_TYPE_VAR_UNK:end")
            host = root / "host.exe"
            host.write_bytes(b"host fixture")
            node = root / "node.exe"
            node.write_bytes(b"node fixture")

            def resource(*args: str) -> None:
                result = subprocess.run(
                    [sys.executable, str(RESOURCE_TOOL), *args],
                    text=True, capture_output=True, check=False,
                )
                self.assertEqual(result.returncode, 0, result.stderr)

            pack = artifact / "gogoke-resources.windows.zip"
            index = artifact / "resource-index.json"
            resource("pack", "--frontend-dir", str(frontend),
                     "--service-dist-dir", str(service), "--output", str(pack))
            resource("index", "--pack", str(pack), "--portable-shell", str(portable),
                     "--native-host", str(host), "--node", str(node),
                     "--source-commit", SOURCE, "--version", "1.2.3",
                     "--installed-root", str(installed), "--output", str(index))

            key_script = """
                $key = [Security.Cryptography.ECDsa]::Create(
                    [Security.Cryptography.ECCurve]::CreateFromFriendlyName('nistP256'))
                try {
                    $parts = $key.ExportParameters($true)
                    ConvertTo-Json -InputObject @(
                        [Convert]::ToHexString($parts.D),
                        [Convert]::ToHexString($parts.Q.X),
                        [Convert]::ToHexString($parts.Q.Y)) -Compress
                } finally { $key.Dispose() }
            """
            generated = subprocess.run(
                [shell, "-NoProfile", "-NonInteractive", "-Command", key_script],
                text=True, capture_output=True, check=False,
            )
            self.assertEqual(generated.returncode, 0, generated.stderr)
            secret, x, y = json.loads(generated.stdout)
            public_path.write_text((x + y).lower() + "\n", encoding="ascii")

            environment = os.environ.copy()
            environment.update({
                "GITHUB_REF": "refs/heads/main",
                "GITHUB_REPOSITORY": "taiyun668/gogoke",
                "GOGOKE_CANDIDATE_P256_KEY": "\n".join((secret, x, y)),
                "TEST_SIGNER": str(tool_dir / SIGNER.name),
                "TEST_ARTIFACT": str(artifact),
                "TEST_RUN_JSON": json.dumps({
                    "repository": {"full_name": "taiyun668/gogoke"},
                    "head_repository": {"full_name": "taiyun668/gogoke"},
                    "head_branch": "gpt/s1-r4-r2-execution-r1",
                    "head_sha": SOURCE, "run_attempt": 1,
                    "conclusion": "success", "name": "gogoke desktop CI",
                }),
                "TEST_ARTIFACT_JSON": json.dumps({"artifacts": [{
                    "id": 19, "name": f"gogoke-windows-frozen-{SOURCE}-17",
                    "expired": False, "size_in_bytes": 1,
                }]}),
            })
            invocation = f"""
                function gh {{
                    param([string]$action, [string]$route)
                    if ($action -cne 'api') {{ throw 'unexpected gh action' }}
                    $global:LASTEXITCODE = 0
                    if ($route -ceq 'repos/taiyun668/gogoke/actions/runs/17') {{
                        return $env:TEST_RUN_JSON
                    }}
                    if ($route -ceq 'repos/taiyun668/gogoke/actions/runs/17/artifacts') {{
                        return $env:TEST_ARTIFACT_JSON
                    }}
                    throw 'unexpected gh endpoint'
                }}
                & $env:TEST_SIGNER -ArtifactDirectory $env:TEST_ARTIFACT `
                    -SourceCommit '{SOURCE}' -RunId 17 -RunAttempt 1 -ArtifactId 19
            """

            def sign() -> subprocess.CompletedProcess[str]:
                return subprocess.run(
                    [shell, "-NoProfile", "-NonInteractive", "-Command", invocation],
                    cwd=repo, env=environment, text=True, capture_output=True, check=False,
                )

            signed = sign()
            self.assertEqual(signed.returncode, 0, signed.stdout + signed.stderr)
            manifest = artifact / "CANDIDATE-RESOURCES.windows"
            signature = artifact / "CANDIDATE-RESOURCES.windows.sig"
            self.assertTrue(manifest.is_file())
            self.assertEqual(len(signature.read_text(encoding="ascii").strip()), 128)

            verify_script = """
                $parts = [Security.Cryptography.ECParameters]::new()
                $parts.Curve = [Security.Cryptography.ECCurve]::CreateFromFriendlyName('nistP256')
                $point = [Security.Cryptography.ECPoint]::new()
                $point.X = [Convert]::FromHexString($env:TEST_PUBLIC_HEX.Substring(0, 64))
                $point.Y = [Convert]::FromHexString($env:TEST_PUBLIC_HEX.Substring(64, 64))
                $parts.Q = $point
                $key = [Security.Cryptography.ECDsa]::Create($parts)
                try {
                    $manifest = [IO.File]::ReadAllBytes($env:TEST_MANIFEST)
                    $signature = [Convert]::FromHexString(([IO.File]::ReadAllText($env:TEST_SIGNATURE)).Trim())
                    $prefix = [Text.Encoding]::ASCII.GetBytes("GOGOKE-CI-CANDIDATE-RESOURCE-V1`0")
                    $payload = [byte[]]($prefix + $manifest)
                    $good = $key.VerifyData($payload, $signature, [Security.Cryptography.HashAlgorithmName]::SHA256)
                    $withoutPrefix = $key.VerifyData($manifest, $signature, [Security.Cryptography.HashAlgorithmName]::SHA256)
                    Write-Output "$good,$withoutPrefix"
                } finally { $key.Dispose() }
            """
            verification_env = {**environment, "TEST_PUBLIC_HEX": x + y,
                                "TEST_MANIFEST": str(manifest), "TEST_SIGNATURE": str(signature)}
            verified = subprocess.run(
                [shell, "-NoProfile", "-NonInteractive", "-Command", verify_script],
                env=verification_env, text=True, capture_output=True, check=False,
            )
            self.assertEqual(verified.returncode, 0, verified.stderr)
            self.assertEqual(verified.stdout.strip(), "True,False")

            duplicate = sign()
            self.assertNotEqual(duplicate.returncode, 0)
            self.assertIn("refusing overwrite", duplicate.stdout + duplicate.stderr)
            manifest.unlink()
            signature.unlink()
            public_path.write_text("0" * 128 + "\n", encoding="ascii")
            mismatch = sign()
            self.assertNotEqual(mismatch.returncode, 0)
            self.assertIn("does not match the compiled public key", mismatch.stdout + mismatch.stderr)
            self.assertFalse(manifest.exists())
            public_path.write_text((x + y).lower() + "\n", encoding="ascii")
            pack.write_bytes(pack.read_bytes() + b"tampered")
            tampered = sign()
            self.assertNotEqual(tampered.returncode, 0)
            self.assertIn("verifier rejected candidate bytes", tampered.stdout + tampered.stderr)
            self.assertFalse(manifest.exists())


if __name__ == "__main__":
    unittest.main()
