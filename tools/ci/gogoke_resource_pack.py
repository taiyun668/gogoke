#!/usr/bin/env python3
"""Build and verify deterministic gogoke resource packs and their byte index."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import sys
import zipfile
from pathlib import Path, PurePosixPath
from typing import Any


SCHEMA = "gogoke.resource-index.v1"
ZIP_EPOCH = (1980, 1, 1, 0, 0, 0)
INSTALL_TOKEN = b"__TAURI_BUNDLE_TYPE_VAR_UNK"
INSTALLED_TOKEN = b"__TAURI_BUNDLE_TYPE_VAR_NSS"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
SEMVER_RE = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$"
)
WINDOWS_PATH_COMPONENT_RE = re.compile(r"^[\x20-\x7e]+$")
WINDOWS_FORBIDDEN_COMPONENT_RE = re.compile(r'[\\/:*?"<>|]')
WINDOWS_RESERVED_NAMES = {"CON", "PRN", "AUX", "NUL"} | {
    f"{prefix}{number}" for prefix in ("COM", "LPT") for number in range(1, 10)
}
SEPARATELY_INDEXED = {
    "gogoke.exe",
    "gogoke-native-host.exe",
    "gogoke-service/runtime/node.exe",
}
EXTERNAL_SIDECARS = {
    "gogoke-resources.windows.zip",
    "resource-index.json",
    "SHA256SUMS.windows",
    "SHA256SUMS.windows.sig",
    "CANDIDATE-RESOURCES.windows",
    "CANDIDATE-RESOURCES.windows.sig",
}
NATIVE_EXECUTABLE_SUFFIXES = {".exe", ".dll", ".node", ".sys", ".msi", ".msix", ".com", ".scr", ".cpl", ".ocx", ".bat", ".cmd", ".ps1"}


class ResourcePackError(ValueError):
    pass


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _is_semver(value: str) -> bool:
    match = SEMVER_RE.fullmatch(value)
    if match is None:
        return False
    prerelease = match.group(4)
    if prerelease is None:
        return True
    return all(not (part.isdigit() and len(part) > 1 and part.startswith("0")) for part in prerelease.split("."))


def _is_reparse_point(info: os.stat_result) -> bool:
    # Windows junctions and other reparse points are not reported as POSIX symlinks.
    reparse_flag = getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400)
    return stat.S_ISLNK(info.st_mode) or bool(getattr(info, "st_file_attributes", 0) & reparse_flag)


def _physical_file(path: Path) -> bytes:
    try:
        info = path.lstat()
    except OSError as exc:
        raise ResourcePackError(f"cannot inspect file {path}: {exc}") from exc
    if _is_reparse_point(info) or not stat.S_ISREG(info.st_mode):
        raise ResourcePackError(f"not a regular physical file: {path}")
    try:
        with path.open("rb") as stream:
            opened = os.fstat(stream.fileno())
            if not stat.S_ISREG(opened.st_mode) or (opened.st_dev, opened.st_ino) != (info.st_dev, info.st_ino):
                raise ResourcePackError(f"file changed while opening: {path}")
            return stream.read()
    except OSError as exc:
        raise ResourcePackError(f"cannot read file {path}: {exc}") from exc


def _normalized_relative(name: str) -> str:
    if "\\" in name or "\x00" in name:
        raise ResourcePackError(f"invalid archive path: {name!r}")
    path = PurePosixPath(name)
    if path.is_absolute() or not path.parts or any(part in ("", ".", "..") for part in path.parts):
        raise ResourcePackError(f"invalid archive path: {name!r}")
    if path.as_posix() != name:
        raise ResourcePackError(f"non-canonical archive path: {name!r}")
    for component in path.parts:
        if (not WINDOWS_PATH_COMPONENT_RE.fullmatch(component)
                or WINDOWS_FORBIDDEN_COMPONENT_RE.search(component)
                or component.endswith((".", " "))):
            raise ResourcePackError(f"archive path component is not Windows-safe ASCII: {component!r}")
        if component.split(".", 1)[0].upper() in WINDOWS_RESERVED_NAMES:
            raise ResourcePackError(f"archive path component is a reserved Windows device name: {component!r}")
    return name


def _scan_tree(root: Path, prefix: str) -> list[tuple[str, bytes]]:
    try:
        root_info = root.lstat()
    except OSError as exc:
        raise ResourcePackError(f"cannot inspect directory {root}: {exc}") from exc
    if _is_reparse_point(root_info) or not stat.S_ISDIR(root_info.st_mode):
        raise ResourcePackError(f"not a physical directory: {root}")

    result: list[tuple[str, bytes]] = []
    pending = [(root, PurePosixPath())]
    while pending:
        directory, relative = pending.pop()
        try:
            entries = sorted(os.scandir(directory), key=lambda item: item.name)
        except OSError as exc:
            raise ResourcePackError(f"cannot scan directory {directory}: {exc}") from exc
        for entry in entries:
            child = Path(entry.path)
            try:
                info = entry.stat(follow_symlinks=False)
            except OSError as exc:
                raise ResourcePackError(f"cannot inspect path {child}: {exc}") from exc
            child_relative = relative / entry.name
            if _is_reparse_point(info):
                raise ResourcePackError(f"symbolic link is not allowed: {child}")
            if stat.S_ISDIR(info.st_mode):
                pending.append((child, child_relative))
                continue
            if not stat.S_ISREG(info.st_mode):
                raise ResourcePackError(f"not a regular physical file: {child}")
            archive_name = _normalized_relative(f"{prefix}/{child_relative.as_posix()}" if prefix else child_relative.as_posix())
            if prefix in ("frontend", "dist") and PurePosixPath(archive_name).suffix.lower() in NATIVE_EXECUTABLE_SUFFIXES:
                raise ResourcePackError(f"native executable is not allowed in resource pack: {archive_name}")
            result.append((archive_name, _physical_file(child)))
    return result


def _installed_files(installed_root: Path) -> list[dict[str, Any]]:
    entries = _scan_tree(installed_root, "")
    paths = [name for name, _ in entries]
    _check_casefold_unique(paths)
    if any(name in EXTERNAL_SIDECARS or name == "uninstall.exe" or name.startswith("gogoke-service/generations/") for name in paths):
        raise ResourcePackError("staged install root contains sidecar, generation, or NSIS uninstaller")
    return [
        {"path": name, "length": len(data), "sha256": sha256(data)}
        for name, data in sorted(entries)
        if name not in SEPARATELY_INDEXED
    ]


def _check_casefold_unique(paths: list[str]) -> None:
    seen: dict[str, str] = {}
    for name in paths:
        key = name.casefold()
        previous = seen.get(key)
        if previous is not None:
            raise ResourcePackError(f"case-insensitive path collision: {previous!r} and {name!r}")
        seen[key] = name


def _zip_info(name: str) -> zipfile.ZipInfo:
    info = zipfile.ZipInfo(name, date_time=ZIP_EPOCH)
    info.compress_type = zipfile.ZIP_STORED
    info.create_system = 3
    info.external_attr = (stat.S_IFREG | 0o644) << 16
    info.flag_bits = 0
    info.extra = b""
    info.comment = b""
    return info


def build_pack(frontend_dir: Path, service_dist_dir: Path, output: Path) -> None:
    entries = _scan_tree(frontend_dir, "frontend") + _scan_tree(service_dist_dir, "dist")
    entries.sort(key=lambda item: item[0])
    names = [name for name, _ in entries]
    _require_resource_entries(names)
    _check_casefold_unique(names)
    try:
        with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_STORED, allowZip64=True) as archive:
            for name, data in entries:
                archive.writestr(_zip_info(name), data)
    except OSError as exc:
        raise ResourcePackError(f"cannot write pack {output}: {exc}") from exc


def _executable_record(path: Path) -> dict[str, Any]:
    data = _physical_file(path)
    return {"length": len(data), "sha256": sha256(data)}


def create_index(
    pack_path: Path,
    portable_shell: Path,
    native_host: Path,
    node: Path,
    source_commit: str,
    version: str,
    installed_root: Path,
    output: Path,
) -> None:
    if not COMMIT_RE.fullmatch(source_commit):
        raise ResourcePackError("source commit must be a lowercase 40-character SHA")
    if not _is_semver(version):
        raise ResourcePackError("version must be a semantic version")
    pack_bytes = _physical_file(pack_path)
    try:
        with zipfile.ZipFile(pack_path, "r") as archive:
            files = _read_pack_files(archive)
    except (OSError, zipfile.BadZipFile) as exc:
        raise ResourcePackError(f"invalid resource pack {pack_path}: {exc}") from exc

    portable_data = _physical_file(portable_shell)
    token_count = portable_data.count(INSTALL_TOKEN)
    if token_count != 1:
        raise ResourcePackError(f"portable shell must contain exactly one bundle token (found {token_count})")
    installed_data = portable_data.replace(INSTALL_TOKEN, INSTALLED_TOKEN)
    if len(installed_data) != len(portable_data):
        raise ResourcePackError("installed-shell token replacement changed byte length")

    pack_hash = sha256(pack_bytes)
    index = {
        "schema": SCHEMA,
        "sourceCommit": source_commit,
        "version": version,
        "generationId": pack_hash,
        "pack": {"fileName": pack_path.name, "length": len(pack_bytes), "sha256": pack_hash},
        "files": files,
        "installedFiles": _installed_files(installed_root),
        "executables": {
            "portableShell": _record_bytes(portable_data),
            "installedShell": _record_bytes(installed_data),
            "nativeHost": _executable_record(native_host),
            "node": _executable_record(node),
        },
    }
    payload = (json.dumps(index, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    try:
        output.write_bytes(payload)
    except OSError as exc:
        raise ResourcePackError(f"cannot write index {output}: {exc}") from exc


def _record_bytes(data: bytes) -> dict[str, Any]:
    return {"length": len(data), "sha256": sha256(data)}


def _read_pack_files(archive: zipfile.ZipFile) -> list[dict[str, Any]]:
    infos = archive.infolist()
    names = [info.filename for info in infos]
    if len(names) != len(set(names)):
        raise ResourcePackError("pack contains duplicate ZIP entries")
    _check_casefold_unique(names)
    files: list[dict[str, Any]] = []
    for info in infos:
        if info.orig_filename != info.filename:
            raise ResourcePackError("resource pack entry contains a NUL in its original ZIP name")
        name = _normalized_relative(info.filename)
        if PurePosixPath(name).suffix.lower() in NATIVE_EXECUTABLE_SUFFIXES:
            raise ResourcePackError(f"native executable is not allowed in resource pack: {name}")
        if info.is_dir():
            raise ResourcePackError(f"directory ZIP entry is not allowed: {name}")
        if info.compress_type != zipfile.ZIP_STORED or info.file_size != info.compress_size:
            raise ResourcePackError(f"resource pack entry is not stored verbatim: {name}")
        mode = (info.external_attr >> 16) & 0xFFFF
        if stat.S_ISLNK(mode) or (mode and not stat.S_ISREG(mode)):
            raise ResourcePackError(f"non-regular ZIP entry is not allowed: {name}")
        if not (name.startswith("frontend/") or name.startswith("dist/")):
            raise ResourcePackError(f"unexpected ZIP entry path: {name}")
        try:
            data = archive.read(info)
        except (OSError, RuntimeError, zipfile.BadZipFile) as exc:
            raise ResourcePackError(f"cannot read ZIP entry {name}: {exc}") from exc
        files.append({"path": name, "length": len(data), "sha256": sha256(data)})
    files.sort(key=lambda item: item["path"])
    _require_resource_entries([item["path"] for item in files])
    return files


def _require_resource_entries(paths: list[str]) -> None:
    required = {"frontend/index.html", "dist/bin.mjs"}
    missing = sorted(required.difference(paths))
    if missing:
        raise ResourcePackError(f"resource pack is missing required entries: {', '.join(missing)}")


def _unique_json_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ResourcePackError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _expect_object(value: Any, keys: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != keys:
        raise ResourcePackError(f"{label} must have exactly these fields: {', '.join(sorted(keys))}")
    return value


def _validate_record(value: Any, label: str) -> dict[str, Any]:
    record = _expect_object(value, {"length", "sha256"}, label)
    if not isinstance(record["length"], int) or isinstance(record["length"], bool) or record["length"] < 0:
        raise ResourcePackError(f"{label}.length must be a non-negative integer")
    if not isinstance(record["sha256"], str) or not SHA256_RE.fullmatch(record["sha256"]):
        raise ResourcePackError(f"{label}.sha256 must be a lowercase SHA-256")
    return record


def verify_pack(pack_path: Path, index_path: Path) -> None:
    try:
        index = json.loads(index_path.read_text(encoding="utf-8"), object_pairs_hook=_unique_json_object)
    except (OSError, UnicodeError, json.JSONDecodeError, ResourcePackError) as exc:
        raise ResourcePackError(f"cannot read index {index_path}: {exc}") from exc
    index = _expect_object(
        index,
        {"schema", "sourceCommit", "version", "generationId", "pack", "files", "installedFiles", "executables"},
        "index",
    )
    if index["schema"] != SCHEMA:
        raise ResourcePackError(f"unsupported index schema: {index['schema']!r}")
    if not isinstance(index["sourceCommit"], str) or not COMMIT_RE.fullmatch(index["sourceCommit"]):
        raise ResourcePackError("index sourceCommit is invalid")
    if not isinstance(index["version"], str) or not _is_semver(index["version"]):
        raise ResourcePackError("index version is invalid")
    if not isinstance(index["generationId"], str) or not SHA256_RE.fullmatch(index["generationId"]):
        raise ResourcePackError("index generationId is invalid")
    pack_meta = _expect_object(index["pack"], {"fileName", "length", "sha256"}, "index.pack")
    if pack_meta["fileName"] != pack_path.name:
        raise ResourcePackError("pack filename does not match index")
    _validate_record({"length": pack_meta["length"], "sha256": pack_meta["sha256"]}, "index.pack")
    files = index["files"]
    if not isinstance(files, list):
        raise ResourcePackError("index.files must be an array")
    for item in files:
        record = _expect_object(item, {"path", "length", "sha256"}, "index.files item")
        if not isinstance(record["path"], str):
            raise ResourcePackError("index file path must be a string")
        _validate_record({"length": record["length"], "sha256": record["sha256"]}, "index.files item")
    installed_files = index["installedFiles"]
    if not isinstance(installed_files, list) or not installed_files:
        raise ResourcePackError("index.installedFiles must be a nonempty array")
    installed_paths: list[str] = []
    for item in installed_files:
        record = _expect_object(item, {"path", "length", "sha256"}, "index.installedFiles item")
        if not isinstance(record["path"], str):
            raise ResourcePackError("installed file path must be a string")
        installed_paths.append(_normalized_relative(record["path"]))
        _validate_record({"length": record["length"], "sha256": record["sha256"]}, "index.installedFiles item")
    _check_casefold_unique(installed_paths)
    if installed_paths != sorted(installed_paths) or any(
        name in SEPARATELY_INDEXED or name in EXTERNAL_SIDECARS or name == "uninstall.exe"
        or name.startswith("gogoke-service/generations/")
        for name in installed_paths
    ):
        raise ResourcePackError("installed file inventory is not canonical")
    executable_meta = _expect_object(index["executables"], {"portableShell", "installedShell", "nativeHost", "node"}, "index.executables")
    for key, value in executable_meta.items():
        _validate_record(value, f"index.executables.{key}")

    pack_bytes = _physical_file(pack_path)
    actual_pack_hash = sha256(pack_bytes)
    if pack_meta["length"] != len(pack_bytes) or pack_meta["sha256"] != actual_pack_hash:
        raise ResourcePackError("resource pack length or SHA-256 does not match index")
    if index["generationId"] != actual_pack_hash:
        raise ResourcePackError("generationId does not match resource pack SHA-256")
    try:
        with zipfile.ZipFile(pack_path, "r") as archive:
            actual_files = _read_pack_files(archive)
    except (OSError, zipfile.BadZipFile) as exc:
        raise ResourcePackError(f"invalid resource pack {pack_path}: {exc}") from exc
    if files != actual_files:
        raise ResourcePackError("resource pack file list or file metadata does not match index")


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    pack = subparsers.add_parser("pack", help="create a deterministic resource ZIP")
    pack.add_argument("--frontend-dir", type=Path, required=True)
    pack.add_argument("--service-dist-dir", type=Path, required=True)
    pack.add_argument("--output", type=Path, required=True)

    index = subparsers.add_parser("index", help="write a deterministic resource index")
    index.add_argument("--pack", type=Path, required=True)
    index.add_argument("--portable-shell", type=Path, required=True)
    index.add_argument("--native-host", type=Path, required=True)
    index.add_argument("--node", type=Path, required=True)
    index.add_argument("--source-commit", required=True)
    index.add_argument("--version", required=True)
    index.add_argument("--installed-root", type=Path, required=True)
    index.add_argument("--output", type=Path, required=True)

    verify = subparsers.add_parser("verify", help="verify pack bytes and indexed files")
    verify.add_argument("--pack", type=Path, required=True)
    verify.add_argument("--index", type=Path, required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        if args.command == "pack":
            build_pack(args.frontend_dir, args.service_dist_dir, args.output)
        elif args.command == "index":
            create_index(
                args.pack,
                args.portable_shell,
                args.native_host,
                args.node,
                args.source_commit,
                args.version,
                args.installed_root,
                args.output,
            )
        else:
            verify_pack(args.pack, args.index)
    except ResourcePackError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
