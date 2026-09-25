"""Exercise the production owned-file cleanup function on ephemeral Windows CI.

The fixture supplies an old-shell inventory after an actual directory rename.
Its synthetic inventory does not stand in for Owner signature verification,
version-bound readiness, or the coordinator's complete rollback path.
"""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import uuid

from test_gogoke_uninstall_finalizer_cloud import identity, kernel32, require_cloud

if os.name == "nt":
    import ctypes
    import winreg


SCRIPT = Path(__file__).resolve().parents[2] / "apps/desktop/src-tauri/update/gogoke-update-coordinator.ps1"
REGISTRY = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke"
HARNESS = r'''
param([string]$Source, [string]$Fixture, [string]$Result)
$ErrorActionPreference = 'Stop'
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($Source, [ref]$tokens, [ref]$errors)
if ($errors.Count -ne 0) { throw 'production coordinator does not parse' }
$functions = @($ast.EndBlock.Statements | Where-Object {
    $_ -is [Management.Automation.Language.FunctionDefinitionAst]
} | ForEach-Object { $_.Extent.Text })
if ($functions.Count -lt 15) { throw 'production cleanup functions unavailable' }
. ([ScriptBlock]::Create(($functions -join "`n")))
$data = Get-Content -LiteralPath $Fixture -Raw | ConvertFrom-Json
$script:targetFull = [string]$data.target
$script:backup = [string]$data.backup
$script:StateFile = [string]$data.stateFile
$script:ExpectedVersion = [string]$data.version
$script:uninstallRegistryPath = 'Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke'
$script:lifecycleLock = [IO.File]::Open([string]$data.lock,
    [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
$script:newInstallCommitted = $false
try {
    Write-UpdateState 'installed_cleanup_pending' 'new install committed; old cleanup pending'
    $script:newInstallCommitted = $true
    Invoke-OwnedBackupCleanup $data.inventory ([string]$data.instance)
    if (Test-Path -LiteralPath $script:backup) {
        Write-UpdateState 'installed_backup_retained' 'unlisted old files retained'
    } else {
        Write-UpdateState 'installed' ''
    }
    $outcome = @{ state = 'OK'; detail = '' }
} catch {
    if ($script:newInstallCommitted) {
        Write-UpdateState 'installed_backup_retained' 'old owned cleanup incomplete'
    }
    $outcome = @{ state = 'FAILED'; detail = [string]$_.Exception.Message }
} finally { $script:lifecycleLock.Dispose() }
[IO.File]::WriteAllText($Result, ($outcome | ConvertTo-Json -Compress),
    [Text.UTF8Encoding]::new($false))
'''


class BackupCleanupCloudTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_cloud()
        try:
            winreg.OpenKey(winreg.HKEY_CURRENT_USER, REGISTRY).Close()
        except FileNotFoundError:
            pass
        else:
            raise AssertionError("formal install registration already exists on cloud runner")

    def exercise(self, user_file=False, changed_id=False, lock_second=False):
        with tempfile.TemporaryDirectory(prefix="gogoke-update-cleanup-ci-") as temp:
            base = Path(temp)
            target = base / "gogoke"
            target.mkdir()
            first = target / "owned-a.bin"
            second = target / "gogoke-service" / "runtime" / "node.exe"
            second.parent.mkdir(parents=True)
            first.write_bytes(b"first signed owned byte")
            second.write_bytes(b"second signed owned byte")
            unknown = target / "user-note.txt"
            if user_file or changed_id or lock_second:
                unknown.write_bytes(b"user byte must remain unchanged")
            old_root_identity = identity(target)
            old_entries = []
            for path in (first, second):
                old_entries.append({
                    "path": str(path),
                    "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                    "identity": identity(path),
                })
            # Rust PathBuf keeps '/' in this production relative component.
            old_entries[1]["path"] = str(target / "gogoke-service") + "/runtime/node.exe"
            if changed_id:
                old_entries[0]["identity"]["fileId"] = "f" * 32
            backup = base / f"gogoke.update-backup-{uuid.uuid4().hex}"
            target.rename(backup)
            target.mkdir()
            (target / "gogoke.exe").write_bytes(b"new installed shell")
            instance = f"ci-new-instance-{uuid.uuid4()}"
            with winreg.CreateKey(winreg.HKEY_CURRENT_USER, REGISTRY) as key:
                winreg.SetValueEx(key, "InstallLocation", 0, winreg.REG_SZ, str(target))
                winreg.SetValueEx(key, "InstallInstanceId", 0, winreg.REG_SZ, instance)
                winreg.SetValueEx(key, "InstallDomain", 0, winreg.REG_SZ, "OWNER_RELEASE")
            fixture = {
                "target": str(target), "backup": str(backup),
                "lock": str(base / "gogoke-install-lifecycle.lock"),
                "stateFile": str(base / "update-state.json"),
                "version": "1.0.1",
                "instance": instance,
                "inventory": {
                    "root": str(target), "rootIdentity": old_root_identity,
                    "files": old_entries,
                },
            }
            fixture_file = base / "fixture.json"
            fixture_file.write_text(json.dumps(fixture), encoding="utf-8")
            state_file = base / "update-state.json"
            state_file.write_text(json.dumps({"schema": 1, "status": "applying",
                                              "offer": {"version": "1.0.1"},
                                              "lastError": None}), encoding="utf-8")
            harness = base / "harness.ps1"
            harness.write_text(HARNESS, encoding="utf-8")
            result_file = base / "result.json"
            locked = None
            if lock_second:
                locked = kernel32.CreateFileW(
                    str(backup / second.relative_to(target)), 0x80, 1, None, 3, 0, None,
                )
                if locked in (None, ctypes.c_void_p(-1).value):
                    raise OSError(ctypes.get_last_error(), "cannot pin second owned file")
            try:
                process = subprocess.run(
                    ["powershell.exe", "-NoLogo", "-NoProfile", "-NonInteractive",
                     "-File", str(harness), "-Source", str(SCRIPT),
                     "-Fixture", str(fixture_file), "-Result", str(result_file)],
                    capture_output=True, text=True, timeout=90, check=False,
                    creationflags=subprocess.CREATE_NO_WINDOW,
                )
                self.assertEqual(process.returncode, 0, process.stderr)
                result = json.loads(result_file.read_text(encoding="utf-8"))
                state = json.loads(state_file.read_text(encoding="utf-8"))
                self.assertEqual(identity(target)["fileId"] != old_root_identity["fileId"], True)
                with winreg.OpenKey(winreg.HKEY_CURRENT_USER, REGISTRY) as key:
                    self.assertEqual(winreg.QueryValueEx(key, "InstallInstanceId")[0], instance)
                if changed_id:
                    self.assertEqual(result["state"], "FAILED")
                    self.assertEqual(state["status"], "installed_backup_retained")
                    self.assertIn("identity changed", result["detail"])
                    self.assertTrue((backup / first.name).exists())
                    self.assertTrue((backup / second.relative_to(target)).exists())
                elif lock_second:
                    self.assertEqual(result["state"], "FAILED")
                    self.assertEqual(state["status"], "installed_backup_retained")
                    self.assertFalse((backup / first.name).exists())
                    self.assertTrue((backup / second.relative_to(target)).exists())
                else:
                    self.assertEqual(result["state"], "OK", result["detail"])
                    self.assertEqual(state["status"], "installed_backup_retained" if user_file else "installed")
                    self.assertFalse((backup / first.name).exists())
                    self.assertFalse((backup / second.relative_to(target)).exists())
                    self.assertEqual(backup.exists(), user_file)
                if unknown.exists() or user_file or changed_id or lock_second:
                    self.assertEqual((backup / unknown.name).read_bytes(),
                                     b"user byte must remain unchanged")
            finally:
                if locked not in (None, ctypes.c_void_p(-1).value):
                    kernel32.CloseHandle(locked)
                winreg.DeleteKey(winreg.HKEY_CURRENT_USER, REGISTRY)

    def test_empty_backup_removed(self):
        self.exercise()

    def test_user_file_preserved(self):
        self.exercise(user_file=True)

    def test_changed_file_id_rejected_before_deletion(self):
        self.exercise(changed_id=True)

    def test_locked_second_file_preserves_new_registration_after_partial_cleanup(self):
        self.exercise(lock_second=True)


if __name__ == "__main__":
    unittest.main()
