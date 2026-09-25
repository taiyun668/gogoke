"""Windows CI behavior test for the embedded uninstall finalizer.

This test deliberately creates an isolated fake install and an HKCU candidate
registration. It must run only on an ephemeral GitHub Windows runner.
"""

import base64
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import uuid

if os.name == "nt":
    import msvcrt
    import winreg


SCRIPT = Path(__file__).resolve().parents[2] / "apps/desktop/src-tauri/update/gogoke-uninstall-finalizer.ps1"
RUST_PARENT = Path(__file__).resolve().parents[2] / "apps/desktop/src-tauri/src/gogoke_uninstall.rs"
REGISTRY = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke-candidate"
GENERIC_READ = 0x80000000
GENERIC_WRITE = 0x40000000
FILE_READ_ATTRIBUTES = 0x80
OPEN_EXISTING = 3
OPEN_ALWAYS = 4
BACKUP_SEMANTICS = 0x02000000
INVALID_HANDLE = ctypes.c_void_p(-1).value
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True) if os.name == "nt" else None
if kernel32:
    kernel32.CreateFileW.argtypes = (
        ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p,
        ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p,
    )
    kernel32.CreateFileW.restype = ctypes.c_void_p
    kernel32.GetFileInformationByHandleEx.argtypes = (
        ctypes.c_void_p, ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32,
    )
    kernel32.GetFileInformationByHandleEx.restype = ctypes.c_int
    kernel32.CloseHandle.argtypes = (ctypes.c_void_p,)


def identity(path: Path) -> dict[str, str]:
    handle = kernel32.CreateFileW(
        str(path), FILE_READ_ATTRIBUTES, 7, None, OPEN_EXISTING,
        BACKUP_SEMANTICS if path.is_dir() else 0, None,
    )
    if handle in (None, INVALID_HANDLE):
        raise OSError(ctypes.get_last_error(), str(path))
    try:
        result = (ctypes.c_ubyte * 24)()
        if not kernel32.GetFileInformationByHandleEx(handle, 18, result, 24):
            raise OSError(ctypes.get_last_error(), str(path))
        raw = bytes(result)
        return {
            "volumeSerialNumber": str(int.from_bytes(raw[:8], "little")),
            "fileId": raw[8:].hex(),
        }
    finally:
        kernel32.CloseHandle(handle)


def parent_helper(payload_path: Path) -> int:
    payload = json.loads(payload_path.read_text(encoding="utf-8"))
    payload["parentPid"] = os.getpid()
    parent_source = RUST_PARENT.read_text(encoding="utf-8")
    bootstrap_match = re.search(
        r'const FINALIZER_BOOTSTRAP: &str = r#"(.*?)"#;', parent_source, re.DOTALL,
    )
    if not bootstrap_match:
        raise AssertionError("Rust finalizer bootstrap is missing")
    encoded_bootstrap = base64.b64encode(
        bootstrap_match.group(1).encode("utf-16-le"),
    ).decode("ascii")
    if len(encoded_bootstrap) >= 32767:
        raise AssertionError("finalizer bootstrap exceeds CreateProcess argument limit")
    embedded = base64.b64encode(SCRIPT.read_bytes())
    lock = kernel32.CreateFileW(
        payload["lockPath"], GENERIC_READ | GENERIC_WRITE, 0, None,
        OPEN_ALWAYS, 0x00200000, None,  # no-follow final component
    )
    if lock in (None, INVALID_HANDLE):
        raise OSError(ctypes.get_last_error(), payload["lockPath"])
    descriptor = msvcrt.open_osfhandle(lock, os.O_RDWR)
    with os.fdopen(descriptor, "r+b", buffering=0) as lock_file:
        child = subprocess.Popen(
            ["powershell.exe", "-NoLogo", "-NoProfile", "-NonInteractive",
             "-EncodedCommand", encoded_bootstrap],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=lock_file,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        child.stdin.write(embedded + b"\n")
        child.stdin.write((json.dumps(payload, separators=(",", ":")) + "\n").encode())
        child.stdin.close()
        lines: list[bytes] = []
        reader = threading.Thread(target=lambda: lines.append(child.stdout.readline()), daemon=True)
        reader.start()
        reader.join(30)
        if reader.is_alive():
            child.kill()
            return 3
        if lines != [f"READY:{payload['nonce']}\r\n".encode()]:
            child.kill()
            return 4
        lock_file.seek(0)
        if lock_file.read(len(payload["nonce"]) + 6) != f"LOCK:{payload['nonce']}\n".encode():
            child.kill()
            return 5
        # The finalizer now owns the inherited lock handle and waits for this
        # exact process to exit. The supervising test checks its receipt.
        return 0


class CloudFinalizerTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if (os.name != "nt" or os.environ.get("GITHUB_ACTIONS") != "true"
                or not os.environ.get("GITHUB_RUN_ID") or not os.environ.get("RUNNER_TEMP")):
            raise RuntimeError("Windows GitHub Actions only; never run deletion locally")
        try:
            winreg.OpenKey(winreg.HKEY_CURRENT_USER, REGISTRY).Close()
        except FileNotFoundError:
            pass
        else:
            raise AssertionError("candidate registration already exists on cloud runner")

    def test_owned_files_only_and_stale_identity(self):
        with tempfile.TemporaryDirectory(prefix="gogoke-finalizer-ci-") as temporary:
            base = Path(temporary)
            root = base / "gogoke-candidate"
            root.mkdir()
            exe = root / "gogoke.exe"
            owned = root / "owned.bin"
            unknown = root / "user-note.txt"
            exe.write_bytes(b"fixture shell")
            owned.write_bytes(b"fixture owned file")
            unknown.write_bytes(b"leave this byte-for-byte intact")
            unknown_bytes = unknown.read_bytes()
            instance = f"ci-fixture-{uuid.uuid4()}"
            with winreg.CreateKey(winreg.HKEY_CURRENT_USER, REGISTRY) as key:
                winreg.SetValueEx(key, "InstallLocation", 0, winreg.REG_SZ, str(root))
                winreg.SetValueEx(key, "InstallInstanceId", 0, winreg.REG_SZ, instance)
                winreg.SetValueEx(key, "InstallDomain", 0, winreg.REG_SZ, "CI_CANDIDATE_RESOURCE")
                winreg.SetValueEx(key, "UninstallString", 0, winreg.REG_SZ,
                                  f'"{exe}" --uninstall')
            try:
                for negative in (True, False):
                    nonce = str(uuid.uuid4())
                    tag = hashlib.sha256(instance.encode()).hexdigest()[:16]
                    payload = {
                        "root": str(root), "rootIdentity": identity(root),
                        "lockPath": str(base / "gogoke-install-lifecycle.lock"),
                        "registryKey": "gogoke-candidate", "instance": instance,
                        "domain": "CI_CANDIDATE_RESOURCE", "parentPid": 0,
                        "nonce": nonce,
                        "receipt": str(base / f"gogoke-uninstall-{tag}-{nonce}.json"),
                        "files": [
                            {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                             "identity": identity(path)}
                            for path in (exe, owned)
                        ],
                    }
                    if negative:
                        payload["files"][1]["identity"]["fileId"] = "f" * 32
                    payload_path = base / f"payload-{nonce}.json"
                    payload_path.write_text(json.dumps(payload), encoding="utf-8")
                    result = subprocess.run(
                        [sys.executable, __file__, "--parent", str(payload_path)],
                        timeout=45, check=False,
                    )
                    self.assertEqual(result.returncode, 0)
                    receipt = Path(payload["receipt"])
                    deadline = time.monotonic() + 90
                    terminal = None
                    while time.monotonic() < deadline:
                        try:
                            observed = json.loads(receipt.read_text(encoding="utf-8"))
                            if observed.get("state") in ("FAILED", "DELETED"):
                                terminal = observed
                                break
                        except (OSError, json.JSONDecodeError):
                            pass
                        time.sleep(0.2)
                    self.assertIsNotNone(terminal, "finalizer did not finish its bounded receipt")
                    if negative:
                        self.assertEqual(terminal["state"], "FAILED")
                        self.assertTrue(exe.exists())
                        self.assertTrue(owned.exists())
                        continue
                    self.assertEqual(terminal["state"], "DELETED")
                    self.assertFalse(exe.exists())
                    self.assertFalse(owned.exists())
                    self.assertEqual(unknown.read_bytes(), unknown_bytes)
                    with self.assertRaises(FileNotFoundError):
                        winreg.OpenKey(winreg.HKEY_CURRENT_USER, REGISTRY)
            finally:
                try:
                    winreg.DeleteKey(winreg.HKEY_CURRENT_USER, REGISTRY)
                except FileNotFoundError:
                    pass


if __name__ == "__main__":
    if len(sys.argv) >= 3 and sys.argv[1] == "--parent":
        sys.exit(parent_helper(Path(sys.argv[2])))
    unittest.main()
