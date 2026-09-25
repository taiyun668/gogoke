#!/usr/bin/env python3
"""Stage and compare frozen R2-06 Windows build artifacts without executing them."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import stat
import sys
import zipfile
from pathlib import Path
from typing import Any

from gogoke_resource_pack import (
    INSTALL_TOKEN,
    INSTALLED_TOKEN,
    ResourcePackError,
    ZIP_EPOCH,
    verify_pack,
)


SCHEMA = "gogoke.r2-06.frozen-windows-build.v1"
COMPARISON_SCHEMA = "gogoke.r2-06.windows-reproducibility.v1"
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
LANES = {"frozen", "repro"}
COMPARED_FILES = (
    "gogoke-portable.exe",
    "gogoke-installed-shell.nsis.exe",
    "gogoke-native-host.exe",
    "node.exe",
    "gogoke-resources.windows.zip",
    "resource-index.json",
)
FIXED_FROZEN_FILES = set(COMPARED_FILES) | {"gogoke-package-hashes.json"}


class FrozenArtifactError(ValueError):
    pass


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise FrozenArtifactError(f"duplicate frozen metadata key: {key}")
        value[key] = item
    return value


def _bytes(path: Path) -> bytes:
    if not path.is_file() or path.is_symlink():
        raise FrozenArtifactError(f"required physical file missing: {path}")
    return path.read_bytes()


def _record(path: Path) -> dict[str, Any]:
    data = _bytes(path)
    return {"length": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def _load_index(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise FrozenArtifactError(f"cannot read resource index: {exc}") from exc
    if not isinstance(value, dict):
        raise FrozenArtifactError("resource index root is not an object")
    return value


def _expect_record(index: dict[str, Any], key: str, path: Path) -> None:
    value = index.get("executables", {}).get(key)
    if value != _record(path):
        raise FrozenArtifactError(f"resource index executable identity mismatch: {key}")


def _load_metadata(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=_unique_object)
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise FrozenArtifactError(f"cannot read frozen build metadata: {exc}") from exc
    if not isinstance(value, dict):
        raise FrozenArtifactError("frozen build metadata root is not an object")
    return value


def verify_frozen_directory(
    directory: Path,
    source_commit: str,
    run_id: int,
    run_attempt: int,
    lane: str,
) -> None:
    if not SHA_RE.fullmatch(source_commit) or run_id <= 0 or run_attempt <= 0 or lane not in LANES:
        raise FrozenArtifactError("invalid frozen build identity")
    if not directory.is_dir() or directory.is_symlink():
        raise FrozenArtifactError("frozen artifact directory is unavailable")

    metadata_path = directory / "frozen-build.json"
    metadata = _load_metadata(metadata_path)
    expected_metadata_keys = {
        "schema", "repository", "sourceCommit", "runId", "runAttempt", "lane", "files"
    }
    if set(metadata) != expected_metadata_keys:
        raise FrozenArtifactError("frozen build metadata has unexpected fields")
    if (
        metadata["schema"] != SCHEMA
        or metadata["repository"] != "taiyun668/gogoke"
        or metadata["sourceCommit"] != source_commit
        or metadata["runId"] != run_id
        or isinstance(metadata["runId"], bool)
        or metadata["runAttempt"] != run_attempt
        or isinstance(metadata["runAttempt"], bool)
        or metadata["lane"] != lane
        or not isinstance(metadata["files"], dict)
    ):
        raise FrozenArtifactError("frozen build metadata identity mismatch")

    index_path = directory / "resource-index.json"
    pack_path = directory / "gogoke-resources.windows.zip"
    try:
        verify_pack(pack_path, index_path)
    except ResourcePackError as exc:
        raise FrozenArtifactError(str(exc)) from exc
    index = _load_index(index_path)
    if index.get("sourceCommit") != source_commit:
        raise FrozenArtifactError("resource index source commit mismatch")
    version = index.get("version")
    if not isinstance(version, str):
        raise FrozenArtifactError("resource index version is unavailable")
    expected_files = FIXED_FROZEN_FILES | {f"gogoke-{version}-windows-x64-unsigned-setup.exe"}
    if set(metadata["files"]) != expected_files:
        raise FrozenArtifactError("frozen build metadata file inventory is not exact")

    physical_names: set[str] = set()
    for path in directory.iterdir():
        if path.name == metadata_path.name:
            _bytes(path)
            continue
        _bytes(path)
        physical_names.add(path.name)
    if physical_names != expected_files:
        raise FrozenArtifactError("frozen artifact directory contains missing or unexpected files")
    for name in sorted(expected_files):
        record = metadata["files"].get(name)
        if not isinstance(record, dict) or set(record) != {"length", "sha256"}:
            raise FrozenArtifactError(f"invalid frozen file record: {name}")
        if record != _record(directory / name):
            raise FrozenArtifactError(f"frozen file identity mismatch: {name}")

    _expect_record(index, "portableShell", directory / "gogoke-portable.exe")
    _expect_record(index, "installedShell", directory / "gogoke-installed-shell.nsis.exe")
    _expect_record(index, "nativeHost", directory / "gogoke-native-host.exe")
    _expect_record(index, "node", directory / "node.exe")


def _copy(source: Path, destination: Path) -> None:
    data = _bytes(source)
    destination.write_bytes(data)
    if _bytes(destination) != data:
        raise FrozenArtifactError(f"copy changed bytes: {destination.name}")


def stage(args: argparse.Namespace) -> None:
    if not SHA_RE.fullmatch(args.source_commit) or args.run_id <= 0 or args.run_attempt <= 0:
        raise FrozenArtifactError("invalid source run identity")
    if args.lane not in LANES:
        raise FrozenArtifactError("invalid build lane")
    if args.output.exists() and any(args.output.iterdir()):
        raise FrozenArtifactError("frozen artifact output must be empty")
    args.output.mkdir(parents=True, exist_ok=True)

    try:
        verify_pack(args.pack, args.index)
    except ResourcePackError as exc:
        raise FrozenArtifactError(str(exc)) from exc
    index = _load_index(args.index)
    if index.get("sourceCommit") != args.source_commit:
        raise FrozenArtifactError("resource index source commit mismatch")

    portable = _bytes(args.portable_shell)
    if portable.count(INSTALL_TOKEN) != 1:
        raise FrozenArtifactError("portable shell does not contain one exact Tauri bundle token")
    installed = portable.replace(INSTALL_TOKEN, INSTALLED_TOKEN)
    if len(installed) != len(portable):
        raise FrozenArtifactError("NSIS bundle token replacement changed shell length")

    portable_target = args.output / "gogoke-portable.exe"
    installed_target = args.output / "gogoke-installed-shell.nsis.exe"
    native_target = args.output / "gogoke-native-host.exe"
    node_target = args.output / "node.exe"
    pack_target = args.output / "gogoke-resources.windows.zip"
    index_target = args.output / "resource-index.json"
    inventory_target = args.output / "gogoke-package-hashes.json"
    installer_name = f"gogoke-{index.get('version')}-windows-x64-unsigned-setup.exe"
    installer_target = args.output / installer_name

    portable_target.write_bytes(portable)
    installed_target.write_bytes(installed)
    _copy(args.native_host, native_target)
    _copy(args.node, node_target)
    _copy(args.pack, pack_target)
    _copy(args.index, index_target)
    _copy(args.package_inventory, inventory_target)
    _copy(args.installer, installer_target)

    _expect_record(index, "portableShell", portable_target)
    _expect_record(index, "installedShell", installed_target)
    _expect_record(index, "nativeHost", native_target)
    _expect_record(index, "node", node_target)
    try:
        verify_pack(pack_target, index_target)
    except ResourcePackError as exc:
        raise FrozenArtifactError(str(exc)) from exc

    files = {
        path.name: _record(path)
        for path in sorted(args.output.iterdir(), key=lambda item: item.name)
        if path.is_file()
    }
    metadata = {
        "schema": SCHEMA,
        "repository": "taiyun668/gogoke",
        "sourceCommit": args.source_commit,
        "runId": args.run_id,
        "runAttempt": args.run_attempt,
        "lane": args.lane,
        "files": files,
    }
    (args.output / "frozen-build.json").write_text(
        json.dumps(metadata, indent=2) + "\n", encoding="utf-8", newline="\n"
    )


def compare(args: argparse.Namespace) -> None:
    verify_frozen_directory(args.frozen, args.source_commit, args.run_id, args.run_attempt, "frozen")
    verify_frozen_directory(args.repro, args.source_commit, args.run_id, args.run_attempt, "repro")
    results: dict[str, Any] = {}
    for name in COMPARED_FILES:
        first = args.frozen / name
        second = args.repro / name
        first_bytes = _bytes(first)
        second_bytes = _bytes(second)
        if first_bytes != second_bytes:
            raise FrozenArtifactError(f"independent Windows builds differ: {name}")
        results[name] = {
            "length": len(first_bytes),
            "sha256": hashlib.sha256(first_bytes).hexdigest(),
        }

    report = {
        "schema": COMPARISON_SCHEMA,
        "repository": "taiyun668/gogoke",
        "sourceCommit": args.source_commit,
        "runId": args.run_id,
        "runAttempt": args.run_attempt,
        "state": "PASS",
        "comparisons": results,
        "installedCandidateSmoke": {
            "state": "NOT_RUN",
            "reason": "trusted default-branch candidate signing workflow must exist on main before signed sidecars can be consumed",
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8", newline="\n")


def portable(args: argparse.Namespace) -> None:
    verify_frozen_directory(
        args.frozen, args.source_commit, args.run_id, args.run_attempt, "frozen"
    )
    version = _load_index(args.frozen / "resource-index.json")["version"]
    expected_name = f"gogoke-{version}-windows-x64-unsigned-portable.zip"
    if args.output.name != expected_name or args.output.exists():
        raise FrozenArtifactError("portable ZIP output must be a fresh exact versioned name")
    entries = (
        ("gogoke.exe", _bytes(args.frozen / "gogoke-portable.exe")),
        ("LICENSE", _bytes(args.license)),
        ("THIRD_PARTY_NOTICES.md", _bytes(args.notices)),
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(args.output, "w", compression=zipfile.ZIP_STORED, allowZip64=False) as archive:
        for name, data in entries:
            info = zipfile.ZipInfo(name, date_time=ZIP_EPOCH)
            info.compress_type = zipfile.ZIP_STORED
            info.create_system = 3
            info.external_attr = (stat.S_IFREG | 0o644) << 16
            archive.writestr(info, data)


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    stage_command = commands.add_parser("stage")
    stage_command.add_argument("--pack", type=Path, required=True)
    stage_command.add_argument("--index", type=Path, required=True)
    stage_command.add_argument("--portable-shell", type=Path, required=True)
    stage_command.add_argument("--native-host", type=Path, required=True)
    stage_command.add_argument("--node", type=Path, required=True)
    stage_command.add_argument("--installer", type=Path, required=True)
    stage_command.add_argument("--package-inventory", type=Path, required=True)
    stage_command.add_argument("--source-commit", required=True)
    stage_command.add_argument("--run-id", type=int, required=True)
    stage_command.add_argument("--run-attempt", type=int, required=True)
    stage_command.add_argument("--lane", required=True)
    stage_command.add_argument("--output", type=Path, required=True)

    compare_command = commands.add_parser("compare")
    compare_command.add_argument("--frozen", type=Path, required=True)
    compare_command.add_argument("--repro", type=Path, required=True)
    compare_command.add_argument("--source-commit", required=True)
    compare_command.add_argument("--run-id", type=int, required=True)
    compare_command.add_argument("--run-attempt", type=int, required=True)
    compare_command.add_argument("--output", type=Path, required=True)

    portable_command = commands.add_parser("portable")
    portable_command.add_argument("--frozen", type=Path, required=True)
    portable_command.add_argument("--license", type=Path, required=True)
    portable_command.add_argument("--notices", type=Path, required=True)
    portable_command.add_argument("--source-commit", required=True)
    portable_command.add_argument("--run-id", type=int, required=True)
    portable_command.add_argument("--run-attempt", type=int, required=True)
    portable_command.add_argument("--output", type=Path, required=True)

    verify_command = commands.add_parser("verify")
    verify_command.add_argument("--directory", type=Path, required=True)
    verify_command.add_argument("--source-commit", required=True)
    verify_command.add_argument("--run-id", type=int, required=True)
    verify_command.add_argument("--run-attempt", type=int, required=True)
    verify_command.add_argument("--lane", required=True)
    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        if args.command == "stage":
            stage(args)
        elif args.command == "compare":
            compare(args)
        elif args.command == "portable":
            portable(args)
        else:
            verify_frozen_directory(
                args.directory, args.source_commit, args.run_id, args.run_attempt, args.lane
            )
    except (FrozenArtifactError, OSError, json.JSONDecodeError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
