from __future__ import annotations

import json
import hashlib
import os
import stat
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
import zipfile
from pathlib import Path

from tools.ci.gogoke_resource_pack import _is_reparse_point, _normalized_relative


TOOL = Path(__file__).with_name("gogoke_resource_pack.py")
TOKEN = b"__TAURI_BUNDLE_TYPE_VAR_UNK"


class GogokeResourcePackTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.frontend = self.root / "frontend-src"
        self.dist = self.root / "service-dist"
        self.frontend.mkdir()
        self.dist.mkdir()
        (self.frontend / "index.html").write_bytes(b"<main>ok</main>\n")
        (self.frontend / "assets").mkdir()
        (self.frontend / "assets" / "app.js").write_bytes(b"export default 1;\n")
        (self.dist / "bin.mjs").write_bytes(b"console.log('service');\n")
        self.pack = self.root / "gogoke-resources.zip"
        self.portable_shell = self.root / "gogoke-portable.exe"
        self.native_host = self.root / "gogoke-native-host.exe"
        self.node = self.root / "node.exe"
        self.portable_shell.write_bytes(b"shell:" + TOKEN + b":end")
        self.native_host.write_bytes(b"native host bytes")
        self.node.write_bytes(b"node runtime bytes")
        self.index = self.root / "resource-index.json"
        self.installed_root = self.root / "installed-static"
        (self.installed_root / "gogoke-service" / "node_modules").mkdir(parents=True)
        (self.installed_root / "gogoke-service" / "node_modules" / "package.json").write_bytes(b"{}\n")

    def tearDown(self) -> None:
        self.temp.cleanup()

    def run_tool(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(TOOL), *args],
            check=False,
            capture_output=True,
            text=True,
        )

    def build_pack(self, output: Path) -> subprocess.CompletedProcess[str]:
        return self.run_tool(
            "pack",
            "--frontend-dir", str(self.frontend),
            "--service-dist-dir", str(self.dist),
            "--output", str(output),
        )

    def build_index(self) -> subprocess.CompletedProcess[str]:
        return self.run_tool(
            "index",
            "--pack", str(self.pack),
            "--portable-shell", str(self.portable_shell),
            "--native-host", str(self.native_host),
            "--node", str(self.node),
            "--source-commit", "0123456789abcdef0123456789abcdef01234567",
            "--version", "1.2.3-rc.1+build.7",
            "--installed-root", str(self.installed_root),
            "--output", str(self.index),
        )

    def test_pack_is_reproducible_across_source_mtimes(self) -> None:
        first = self.root / "first.zip"
        second = self.root / "second.zip"
        self.assertEqual(self.build_pack(first).returncode, 0)
        for path in (self.frontend / "index.html", self.frontend / "assets" / "app.js", self.dist / "bin.mjs"):
            os.utime(path, (1_700_000_000, 1_700_000_000))
        self.assertEqual(self.build_pack(second).returncode, 0)
        self.assertEqual(first.read_bytes(), second.read_bytes())
        with zipfile.ZipFile(first) as archive:
            self.assertEqual(archive.namelist(), ["dist/bin.mjs", "frontend/assets/app.js", "frontend/index.html"])
            self.assertTrue(all(info.date_time == (1980, 1, 1, 0, 0, 0) for info in archive.infolist()))

    def test_index_and_verify_detect_pack_tampering(self) -> None:
        self.assertEqual(self.build_pack(self.pack).returncode, 0)
        self.assertEqual(self.build_index().returncode, 0)
        index_data = json.loads(self.index.read_text(encoding="utf-8"))
        self.assertEqual(index_data["schema"], "gogoke.resource-index.v1")
        self.assertEqual(index_data["generationId"], index_data["pack"]["sha256"])
        self.assertEqual(index_data["executables"]["installedShell"]["length"], len(self.portable_shell.read_bytes()))
        self.assertEqual(index_data["installedFiles"][0]["path"], "gogoke-service/node_modules/package.json")
        verified = self.run_tool("verify", "--pack", str(self.pack), "--index", str(self.index))
        self.assertEqual(verified.returncode, 0, verified.stderr)

        damaged = bytearray(self.pack.read_bytes())
        damaged[-1] ^= 0x01
        self.pack.write_bytes(damaged)
        rejected = self.run_tool("verify", "--pack", str(self.pack), "--index", str(self.index))
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn("does not match index", rejected.stderr)

    def test_index_rejects_wrong_bundle_token_count(self) -> None:
        self.assertEqual(self.build_pack(self.pack).returncode, 0)
        for content in (b"no bundle token", TOKEN + b" and " + TOKEN):
            self.portable_shell.write_bytes(content)
            result = self.build_index()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("exactly one bundle token", result.stderr)

    def test_verify_rejects_duplicate_json_keys(self) -> None:
        self.assertEqual(self.build_pack(self.pack).returncode, 0)
        self.assertEqual(self.build_index().returncode, 0)
        content = self.index.read_text(encoding="utf-8")
        content = content.replace(
            '"schema": "gogoke.resource-index.v1",',
            '"schema": "gogoke.resource-index.v1",\n  "schema": "gogoke.resource-index.v1",',
            1,
        )
        self.index.write_text(content, encoding="utf-8", newline="\n")
        rejected = self.run_tool("verify", "--pack", str(self.pack), "--index", str(self.index))
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn("duplicate JSON key: schema", rejected.stderr)

    def test_pack_requires_frontend_index_and_dist_bin(self) -> None:
        (self.frontend / "index.html").unlink()
        missing_frontend = self.build_pack(self.pack)
        self.assertNotEqual(missing_frontend.returncode, 0)
        self.assertIn("frontend/index.html", missing_frontend.stderr)
        (self.frontend / "index.html").write_bytes(b"<main>ok</main>\n")
        (self.dist / "bin.mjs").unlink()
        missing_service = self.build_pack(self.pack)
        self.assertNotEqual(missing_service.returncode, 0)
        self.assertIn("dist/bin.mjs", missing_service.stderr)
        with zipfile.ZipFile(self.pack, "w") as archive:
            for name, data in (("frontend/index.html", b"<main>ok</main>\n"), ("dist/other.mjs", b"service")):
                info = zipfile.ZipInfo(name)
                info.create_system = 3
                info.external_attr = (stat.S_IFREG | 0o644) << 16
                archive.writestr(info, data)
        rejected_index = self.build_index()
        self.assertNotEqual(rejected_index.returncode, 0)
        self.assertIn("dist/bin.mjs", rejected_index.stderr)

        pack_bytes = self.pack.read_bytes()
        pack_hash = hashlib.sha256(pack_bytes).hexdigest()
        indexed_files = []
        with zipfile.ZipFile(self.pack) as archive:
            for name in sorted(archive.namelist()):
                data = archive.read(name)
                indexed_files.append({"path": name, "length": len(data), "sha256": hashlib.sha256(data).hexdigest()})
        empty_executable = {"length": 0, "sha256": "0" * 64}
        self.index.write_text(json.dumps({
            "schema": "gogoke.resource-index.v1",
            "sourceCommit": "0123456789abcdef0123456789abcdef01234567",
            "version": "1.2.3",
            "generationId": pack_hash,
            "pack": {"fileName": self.pack.name, "length": len(pack_bytes), "sha256": pack_hash},
            "files": indexed_files,
            "installedFiles": [{"path": "gogoke-service/node_modules/package.json", "length": 3, "sha256": hashlib.sha256(b"{}\n").hexdigest()}],
            "executables": {
                "portableShell": empty_executable,
                "installedShell": empty_executable,
                "nativeHost": empty_executable,
                "node": empty_executable,
            },
        }), encoding="utf-8")
        rejected_verify = self.run_tool("verify", "--pack", str(self.pack), "--index", str(self.index))
        self.assertNotEqual(rejected_verify.returncode, 0)
        self.assertIn("dist/bin.mjs", rejected_verify.stderr)

    def test_pack_rejects_native_executable_entry(self) -> None:
        (self.dist / "helper.node").write_bytes(b"native test fixture")
        rejected = self.build_pack(self.pack)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn("native executable is not allowed", rejected.stderr)

    def test_index_rejects_compressed_entry_before_expansion(self) -> None:
        with zipfile.ZipFile(self.pack, "w") as archive:
            for name, data, compression in (
                ("frontend/index.html", b"<main>ok</main>\n", zipfile.ZIP_STORED),
                ("dist/bin.mjs", b"x" * (1024 * 1024), zipfile.ZIP_DEFLATED),
            ):
                info = zipfile.ZipInfo(name)
                info.create_system = 3
                info.external_attr = (stat.S_IFREG | 0o644) << 16
                info.compress_type = compression
                archive.writestr(info, data)
        rejected = self.build_index()
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn("not stored verbatim", rejected.stderr)

    def test_windows_reparse_attribute_is_rejected(self) -> None:
        info = SimpleNamespace(st_mode=stat.S_IFDIR, st_file_attributes=0x400)
        self.assertTrue(_is_reparse_point(info))

    def test_archive_paths_reject_non_ascii_and_device_names(self) -> None:
        for path in ("frontend/caf\u00e9.js", "dist/CON.txt", "dist/trailing."):
            with self.subTest(path=path), self.assertRaises(ValueError):
                _normalized_relative(path)
        self.assertEqual(_normalized_relative("gogoke-service/node_modules/@scope/package.json"), "gogoke-service/node_modules/@scope/package.json")


if __name__ == "__main__":
    unittest.main()
