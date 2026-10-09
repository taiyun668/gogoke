"""Read-only post-uninstall proof for one exact Gogoke candidate root.

The owned inventory is either the original
``gogoke.update-owned-inventory.v1`` artifact or the candidate finalizer's
saved owned payload from before uninstall.  The reader derives only its
root-internal parent directories, probes each with Win32 FileIdInfo, and
records immediate children without deleting, moving, or following them.

Run this after the official candidate uninstall with a private output path.
Public stdout/stderr contains only a status code; the JSON result is private.
"""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
import re
import stat
import sys
from pathlib import Path
from typing import Any


SCHEMA = "gogoke.m2.uninstall-directory-readback.v1"
OWNED_SCHEMA = "gogoke.update-owned-inventory.v1"
RESOURCE_SCHEMA = "gogoke.resource-index.v1"
REPARSE_POINT = 0x400
FILE_READ_ATTRIBUTES = 0x80
FILE_SHARE_READ = 0x1
FILE_SHARE_WRITE = 0x2
FILE_SHARE_DELETE = 0x4
OPEN_EXISTING = 3
FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000
FILE_FLAG_BACKUP_SEMANTICS = 0x02000000
FILE_ATTRIBUTE_TAG_INFO = 9
FILE_ID_INFO = 18
ERROR_FILE_NOT_FOUND = 2
ERROR_PATH_NOT_FOUND = 3
ERROR_DIR_NOT_EMPTY = 145
UUID_RE = re.compile(r"^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
FILE_ID_RE = re.compile(r"^[0-9a-f]{32}$")
VOLUME_RE = re.compile(r"^[0-9]+$")
DRIVE_ABSOLUTE_RE = re.compile(r"^[A-Za-z]:\\")


class ReadbackError(RuntimeError):
    def __init__(self, code: str, win32: int | None = None):
        super().__init__(code)
        self.code = code
        self.win32 = win32


if os.name == "nt":
    from ctypes import wintypes

    class _FileAttributeTagInfo(ctypes.Structure):
        _fields_ = [("file_attributes", wintypes.DWORD), ("reparse_tag", wintypes.DWORD)]

    class _FileIdInfo(ctypes.Structure):
        _fields_ = [("volume_serial", ctypes.c_ulonglong), ("file_id", ctypes.c_ubyte * 16)]

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _kernel32.CreateFileW.argtypes = [
        wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID,
        wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE,
    ]
    _kernel32.CreateFileW.restype = wintypes.HANDLE
    _kernel32.GetFileInformationByHandleEx.argtypes = [
        wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD,
    ]
    _kernel32.GetFileInformationByHandleEx.restype = wintypes.BOOL
    _kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    _kernel32.CloseHandle.restype = wintypes.BOOL
else:
    _kernel32 = None


def _fail(code: str, win32: int | None = None) -> None:
    raise ReadbackError(code, win32)


def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _local_spelling(value: str) -> str:
    text = str(value)
    if text.startswith("\\\\?\\UNC\\") or text.startswith("\\\\"):
        _fail("PATH_UNSUPPORTED")
    if text.startswith("\\\\?\\"):
        text = text[4:]
    return text


def _same_path(left: str | Path, right: str | Path) -> bool:
    return os.path.normcase(os.path.normpath(_local_spelling(str(left)))) == \
        os.path.normcase(os.path.normpath(_local_spelling(str(right))))


def _beneath(parent: str | Path, child: str | Path) -> bool:
    parent_text = _local_spelling(str(parent))
    child_text = _local_spelling(str(child))
    try:
        common = os.path.commonpath((parent_text, child_text))
    except ValueError:
        return False
    return _same_path(common, parent_text)


def _validate_absolute_path(value: Any, code: str) -> str:
    if not isinstance(value, str):
        _fail(code)
    text = _local_spelling(value)
    if not DRIVE_ABSOLUTE_RE.match(text) or "/" in text or any(ord(c) < 0x20 for c in text):
        _fail(code)
    parts = text[3:].split("\\")
    if any(part in (".", "..") or part.endswith((".", " ")) for part in parts if part):
        _fail(code)
    if text.endswith("\\") and len(text) > 3:
        _fail(code)
    return text


def _validate_relative(value: Any, code: str) -> str:
    if not isinstance(value, str) or not value or "\\" in value or "\x00" in value:
        _fail(code)
    parts = value.split("/")
    if any(not part or part in (".", "..") or part.endswith((".", " ")) for part in parts):
        _fail(code)
    return value


def _relative(root: Path, path: Path) -> str:
    if not _beneath(root, path) or _same_path(root, path):
        _fail("OWNED_PATH_OUTSIDE_ROOT")
    value = os.path.relpath(str(path), str(root)).replace(os.sep, "/")
    return _validate_relative(value, "OWNED_PATH_INVALID")


def _ordinary_info(path: Path, expect_directory: bool | None) -> os.stat_result:
    try:
        info = path.lstat()
    except OSError as error:
        _fail("INPUT_STAT_FAILED", getattr(error, "winerror", None))
    if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & REPARSE_POINT:
        _fail("INPUT_REPARSE_POINT")
    if info.st_nlink != 1:
        _fail("INPUT_HARDLINK")
    if expect_directory is not None and stat.S_ISDIR(info.st_mode) != expect_directory:
        _fail("INPUT_TYPE_MISMATCH")
    return info


def _read_private_file(path: Path, maximum: int) -> tuple[bytes, str]:
    before = _ordinary_info(path, False)
    try:
        with path.open("rb") as stream:
            opened = os.fstat(stream.fileno())
            if (opened.st_dev, opened.st_ino, opened.st_nlink) != \
                    (before.st_dev, before.st_ino, before.st_nlink):
                _fail("INPUT_CHANGED_WHILE_OPEN")
            data = stream.read(maximum + 1)
    except OSError as error:
        _fail("INPUT_READ_FAILED", getattr(error, "winerror", None))
    if len(data) > maximum:
        _fail("INPUT_TOO_LARGE")
    after = _ordinary_info(path, False)
    if (after.st_dev, after.st_ino, after.st_nlink) != \
            (before.st_dev, before.st_ino, before.st_nlink):
        _fail("INPUT_CHANGED_AFTER_READ")
    return data, _sha256(data)


def _identity(value: Any) -> dict[str, str]:
    if not isinstance(value, dict):
        _fail("IDENTITY_INVALID")
    volume = value.get("volumeSerialNumber")
    file_id = value.get("fileId")
    if not isinstance(volume, str) or not VOLUME_RE.fullmatch(volume) or \
            not isinstance(file_id, str) or not FILE_ID_RE.fullmatch(file_id.lower()):
        _fail("IDENTITY_INVALID")
    return {"volumeSerialNumber": volume, "fileId": file_id.lower()}


def _probe_object(path: Path, directory: bool) -> dict[str, Any]:
    if _kernel32 is None:
        _fail("WINDOWS_ONLY")
    flags = FILE_FLAG_OPEN_REPARSE_POINT
    if directory:
        flags |= FILE_FLAG_BACKUP_SEMANTICS
    handle = _kernel32.CreateFileW(
        str(path), FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        None, OPEN_EXISTING, flags, None,
    )
    invalid = ctypes.c_void_p(-1).value
    value = handle if isinstance(handle, int) else getattr(handle, "value", None)
    if value is None or value == invalid:
        error = int(ctypes.get_last_error())
        if error in (ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND):
            return {"state": "ABSENT", "win32": error}
        _fail("OPEN_OBJECT_FAILED", error)
    try:
        attributes = _FileAttributeTagInfo()
        if not _kernel32.GetFileInformationByHandleEx(
                handle, FILE_ATTRIBUTE_TAG_INFO, ctypes.byref(attributes), ctypes.sizeof(attributes)):
            error = int(ctypes.get_last_error())
            _fail("DIRECTORY_ATTRIBUTES_FAILED", error)
        is_directory = bool(attributes.file_attributes & 0x10)
        if is_directory != directory or attributes.file_attributes & REPARSE_POINT:
            _fail("OBJECT_REPARSE_OR_TYPE")
        identity = _FileIdInfo()
        if not _kernel32.GetFileInformationByHandleEx(
                handle, FILE_ID_INFO, ctypes.byref(identity), ctypes.sizeof(identity)):
            error = int(ctypes.get_last_error())
            _fail("DIRECTORY_ID_FAILED", error)
        file_id = bytes(identity.file_id).hex()
        if file_id == "0" * 32:
            _fail("DIRECTORY_ID_ZERO")
        return {
            "state": "PRESENT",
            "identity": {
                "volumeSerialNumber": str(int(identity.volume_serial)),
                "fileId": file_id,
            },
        }
    finally:
        if not _kernel32.CloseHandle(handle):
            error = int(ctypes.get_last_error())
            _fail("CLOSE_OBJECT_FAILED", error)


def _parse_owned_index(path: Path, root: Path, instance: str, nonce: str) -> tuple[dict[str, Any], str]:
    data_bytes, digest = _read_private_file(path, 16 * 1024 * 1024)
    try:
        data = json.loads(data_bytes.decode("utf-8-sig"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        _fail("OWNED_INDEX_JSON_INVALID")
    if not isinstance(data, dict):
        _fail("OWNED_INDEX_SCHEMA_INVALID")
    schema = data.get("schema")
    candidate_payload = schema is None
    if schema != OWNED_SCHEMA and not candidate_payload:
        _fail("OWNED_INDEX_SCHEMA_INVALID")
    if candidate_payload and data.get("domain") != "CI_CANDIDATE_RESOURCE":
        _fail("OWNED_INDEX_DOMAIN_INVALID")
    if data.get("registryKey") not in (None, "gogoke-candidate"):
        _fail("OWNED_INDEX_REGISTRY_INVALID")
    if data.get("nonce") not in (None, nonce):
        _fail("OWNED_INDEX_NONCE_MISMATCH")
    indexed_root = _validate_absolute_path(data.get("root"), "OWNED_INDEX_ROOT_INVALID")
    if not _same_path(indexed_root, root):
        _fail("OWNED_INDEX_ROOT_MISMATCH")
    if data.get("instance") != instance:
        _fail("OWNED_INDEX_INSTANCE_MISMATCH")
    files = data.get("files")
    if not isinstance(files, list) or not 1 <= len(files) <= 100000:
        _fail("OWNED_INDEX_FILES_INVALID")
    seen: set[str] = set()
    normalized_files: list[dict[str, Any]] = []
    for entry in files:
        if not isinstance(entry, dict):
            _fail("OWNED_INDEX_FILE_INVALID")
        absolute = _validate_absolute_path(entry.get("path"), "OWNED_INDEX_PATH_INVALID")
        relative = _relative(root, Path(absolute))
        folded = os.path.normcase(relative)
        if folded in seen:
            _fail("OWNED_INDEX_DUPLICATE_PATH")
        seen.add(folded)
        sha = entry.get("sha256")
        if not isinstance(sha, str) or not SHA256_RE.fullmatch(sha):
            _fail("OWNED_INDEX_HASH_INVALID")
        normalized_files.append({
            "path": Path(absolute), "relative": relative, "folded": folded,
            "sha256": sha, "identity": _identity(entry.get("identity")),
        })
    return {
        "schema": schema or "gogoke.uninstall-owned-payload.v1", "version": data.get("version"),
        "root": indexed_root, "rootIdentity": _identity(data.get("rootIdentity")),
        "instance": instance, "files": normalized_files,
    }, digest


def _parse_resource_index(path: Path, version: Any, owned_files: set[str]) -> dict[str, Any]:
    data_bytes, digest = _read_private_file(path, 4 * 1024 * 1024)
    try:
        data = json.loads(data_bytes.decode("utf-8-sig"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        _fail("RESOURCE_INDEX_JSON_INVALID")
    if not isinstance(data, dict) or data.get("schema") != RESOURCE_SCHEMA:
        _fail("RESOURCE_INDEX_SCHEMA_INVALID")
    if version is not None and data.get("version") != version:
        _fail("RESOURCE_INDEX_VERSION_MISMATCH")
    if not isinstance(data.get("generationId"), str) or not data["generationId"]:
        _fail("RESOURCE_INDEX_GENERATION_INVALID")
    installed = data.get("installedFiles")
    generation = data.get("files")
    if not isinstance(installed, list) or not isinstance(generation, list):
        _fail("RESOURCE_INDEX_FILES_INVALID")
    expected_paths = {"gogoke.exe", "gogoke-native-host.exe", "gogoke-service/runtime/node.exe"}
    for entry in installed:
        if not isinstance(entry, dict):
            _fail("RESOURCE_INDEX_FILES_INVALID")
        expected_paths.add(_validate_relative(entry.get("path"), "RESOURCE_INDEX_PATH_INVALID"))
    for entry in generation:
        if not isinstance(entry, dict):
            _fail("RESOURCE_INDEX_FILES_INVALID")
        relative = _validate_relative(entry.get("path"), "RESOURCE_INDEX_PATH_INVALID")
        expected_paths.add(f"gogoke-service/generations/{data['generationId']}/{relative}")
    if any(os.path.normcase(path) not in owned_files for path in expected_paths):
        _fail("RESOURCE_INDEX_OWNERSHIP_MISMATCH")
    return {"schema": data["schema"], "version": data.get("version"),
            "sourceCommit": data.get("sourceCommit"),
            "generationId": data["generationId"], "sha256": digest,
            "boundFileCount": len(expected_paths)}


def _derive_parents(root: Path, files: list[dict[str, Any]]) -> list[dict[str, Any]]:
    parents: dict[str, dict[str, Any]] = {}
    for file in files:
        current = file["path"].parent
        while not _same_path(current, root):
            relative = _relative(root, current)
            key = os.path.normcase(relative)
            parents.setdefault(key, {"path": current, "relative": relative, "folded": key})
            current = current.parent
    return sorted(parents.values(), key=lambda item: (
        len(item["relative"].split("/")), len(item["relative"])), reverse=True)


def _scan_children(root: Path, directory: Path, owned_files: set[str], owned_parents: set[str]) -> list[dict[str, Any]]:
    try:
        entries = list(os.scandir(directory))
    except OSError as error:
        _fail("DIRECTORY_SCAN_FAILED", getattr(error, "winerror", None))
    observed: list[dict[str, Any]] = []
    for entry in entries:
        child = Path(entry.path)
        relative = _relative(root, child)
        folded = os.path.normcase(relative)
        try:
            info = entry.stat(follow_symlinks=False)
        except OSError as error:
            _fail("CHILD_STAT_FAILED", getattr(error, "winerror", None))
        attributes = getattr(info, "st_file_attributes", 0)
        if folded in owned_files:
            classification = "owned-file"
        elif folded in owned_parents:
            classification = "owned-parent"
        else:
            classification = "unknown"
        observed.append({
            "relative": relative, "classification": classification,
            "directory": stat.S_ISDIR(info.st_mode),
            "reparse": bool(attributes & REPARSE_POINT) or stat.S_ISLNK(info.st_mode),
        })
    return observed


def _receipt(root: Path, instance: str, nonce: str) -> tuple[dict[str, Any], str]:
    tag = _sha256(instance.encode("utf-8"))[:16]
    name = f"gogoke-uninstall-{tag}-{nonce}.json"
    path = root.parent / name
    data_bytes, digest = _read_private_file(path, 2048)
    try:
        data = json.loads(data_bytes.decode("utf-8-sig"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        _fail("RECEIPT_JSON_INVALID")
    if not isinstance(data, dict) or data.get("schema") != "gogoke.uninstall-result.v1":
        _fail("RECEIPT_SCHEMA_INVALID")
    if data.get("domain") != "CI_CANDIDATE_RESOURCE":
        _fail("RECEIPT_DOMAIN_INVALID")
    return {"fileName": name, "sha256": digest, "state": data.get("state"),
            "detail": data.get("detail")}, name


def _run(args: argparse.Namespace) -> dict[str, Any]:
    if os.name != "nt":
        _fail("WINDOWS_ONLY")
    if not isinstance(args.instance, str) or not args.instance or len(args.instance) > 256 or \
            any(char in args.instance for char in "\r\n"):
        _fail("INSTANCE_INVALID")
    if not isinstance(args.nonce, str) or not UUID_RE.fullmatch(args.nonce):
        _fail("NONCE_INVALID")
    root_text = _validate_absolute_path(args.root, "ROOT_INVALID")
    root = Path(root_text)
    root_resolved = root.resolve(strict=True)
    if not _same_path(root, root_resolved):
        _fail("ROOT_REPARSE_OR_ALIAS")
    output = Path(args.output)
    if output.exists() or not output.parent.is_dir() or _beneath(root, output):
        _fail("OUTPUT_SCOPE_INVALID")
    owned_path = Path(args.owned_index).resolve(strict=True)
    if _beneath(root, owned_path):
        _fail("OWNED_INDEX_SCOPE_INVALID")
    owned, owned_digest = _parse_owned_index(owned_path, root, args.instance, args.nonce)
    owned_file_keys = {entry["folded"] for entry in owned["files"]}
    resource = None
    if args.resource_index:
        resource_path = Path(args.resource_index).resolve(strict=True)
        if _beneath(root, resource_path):
            _fail("RESOURCE_INDEX_SCOPE_INVALID")
        resource = _parse_resource_index(resource_path, owned["version"], owned_file_keys)

    root_before = _probe_object(root, True)
    if root_before.get("state") != "PRESENT" or root_before.get("identity") != owned["rootIdentity"]:
        _fail("ROOT_IDENTITY_MISMATCH")
    receipt, _ = _receipt(root, args.instance, args.nonce)
    if receipt.get("state") != "DELETED":
        _fail("RECEIPT_NOT_DELETED")

    owned_files = {entry["folded"] for entry in owned["files"]}
    parents = _derive_parents(root, owned["files"])
    parent_keys = {entry["folded"] for entry in parents}
    rows: list[dict[str, Any]] = []
    failures: list[str] = []
    present_nonempty = 0
    for parent in parents:
        probe = _probe_object(parent["path"], True)
        if probe.get("state") == "ABSENT":
            rows.append({"relative": parent["relative"], "state": "ABSENT",
                         "win32": probe["win32"]})
            continue
        entries = _scan_children(root, parent["path"], owned_files, parent_keys)
        after = _probe_object(parent["path"], True)
        if after.get("state") != "PRESENT" or after.get("identity") != probe.get("identity"):
            _fail("DIRECTORY_CHANGED_DURING_READBACK")
        if entries:
            present_nonempty += 1
            rows.append({"relative": parent["relative"], "state": "PRESENT_NONEMPTY",
                         "directoryIdentity": probe["identity"],
                         "win32": ERROR_DIR_NOT_EMPTY, "entries": entries})
        else:
            failures.append("OWNED_PARENT_EMPTY")
            rows.append({"relative": parent["relative"], "state": "PRESENT_EMPTY",
                         "directoryIdentity": probe["identity"], "win32": None})

    residual_files: list[dict[str, Any]] = []
    for entry in owned["files"]:
        probe = _probe_object(entry["path"], False)
        if probe.get("state") == "PRESENT":
            residual_files.append({"relative": entry["relative"], "identity": probe["identity"]})
    if residual_files:
        failures.append("OWNED_FILE_REMAINS")

    detail = receipt.get("detail") if isinstance(receipt.get("detail"), str) else ""
    count_match = re.search(r"retained ([0-9]+) owned parent directories after Win32 145", detail)
    receipt_count = int(count_match.group(1)) if count_match else 0
    if present_nonempty and ("Win32 145" not in detail or receipt_count != present_nonempty):
        failures.append("RECEIPT_145_MISMATCH")
    if not present_nonempty and "Win32 145" in detail:
        failures.append("RECEIPT_145_WITHOUT_RETAINED_PARENT")

    root_after = _probe_object(root, True)
    if root_after.get("state") != "PRESENT" or root_after.get("identity") != owned["rootIdentity"]:
        failures.append("ROOT_NOT_PRESERVED")
    result = {
        "schema": SCHEMA, "phase": "post-uninstall", "acceptance": False,
        "readOnly": True, "productWrites": False,
        "instance": args.instance, "nonce": args.nonce,
        "rootIdentity": owned["rootIdentity"],
        "ownedIndex": {"schema": owned["schema"], "sha256": owned_digest,
                        "version": owned["version"], "fileCount": len(owned["files"])},
        "resourceIndex": resource,
        "receipt": receipt,
        "parents": rows, "ownedFileResiduals": residual_files,
        "summary": {"derivedParentCount": len(parents),
                    "absentParentCount": sum(row["state"] == "ABSENT" for row in rows),
                    "retainedNonemptyParentCount": present_nonempty,
                    "emptyParentCount": sum(row["state"] == "PRESENT_EMPTY" for row in rows),
                    "ownedFileResidualCount": len(residual_files),
                    "rootPreserved": not any(code == "ROOT_NOT_PRESERVED" for code in failures)},
        "failureCodes": sorted(set(failures)),
    }
    result["status"] = "PASS" if not failures else "FAIL"
    return result


def _write_private(path: Path, result: dict[str, Any]) -> None:
    path.write_text(json.dumps(result, ensure_ascii=False, sort_keys=True, indent=2) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(add_help=True)
    parser.add_argument("--root", required=True)
    parser.add_argument("--instance", required=True)
    parser.add_argument("--nonce", required=True)
    parser.add_argument("--owned-index", required=True)
    parser.add_argument("--resource-index")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    try:
        args.output = str(Path(args.output).resolve(strict=False))
        output = Path(args.output)
        result = _run(args)
        _write_private(output, result)
    except ReadbackError as error:
        if not output.exists() and output.parent.is_dir():
            try:
                _write_private(output, {
                    "schema": SCHEMA, "phase": "post-uninstall", "acceptance": False,
                    "readOnly": True, "productWrites": False,
                    "status": "FAIL", "failureCodes": [error.code],
                    "win32": error.win32,
                })
            except OSError:
                pass
        print(f"M2_UNINSTALL_DIRECTORY_READBACK: FAIL:{error.code}", file=sys.stderr)
        return 1
    except (OSError, ValueError, TypeError):
        print("M2_UNINSTALL_DIRECTORY_READBACK: FAIL:READER_ERROR", file=sys.stderr)
        return 1
    if result["status"] == "PASS":
        print("M2_UNINSTALL_DIRECTORY_READBACK: PASS")
        return 0
    print("M2_UNINSTALL_DIRECTORY_READBACK: FAIL", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
