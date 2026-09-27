"""Cloud-only Windows API probe for in-place reparse changes to a pinned directory.

This is a diagnostic fixture. It does not claim the installed NSIS publisher is
safe; the separate barrier test must exercise the actual installer behavior.
"""

import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import struct
import uuid


kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
kernel32.CreateFileW.argtypes = [
    wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p,
    wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE,
]
kernel32.CreateFileW.restype = wintypes.HANDLE
kernel32.DeviceIoControl.argtypes = [
    wintypes.HANDLE, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD,
    ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p,
]
kernel32.DeviceIoControl.restype = wintypes.BOOL
kernel32.GetFileAttributesW.argtypes = [wintypes.LPCWSTR]
kernel32.GetFileAttributesW.restype = wintypes.DWORD
kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
kernel32.CloseHandle.restype = wintypes.BOOL

INVALID_HANDLE = ctypes.c_void_p(-1).value
OPEN_EXISTING = 3
FILE_READ_ATTRIBUTES = 0x80
FILE_WRITE_ATTRIBUTES = 0x100
FILE_FLAG_BACKUP_SEMANTICS = 0x02000000
FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000
FILE_ATTRIBUTE_REPARSE_POINT = 0x400
FSCTL_SET_REPARSE_POINT = 0x000900A4
IO_REPARSE_TAG_MOUNT_POINT = 0xA0000003


def open_directory(path: Path, access: int, sharing: int):
    handle = kernel32.CreateFileW(
        str(path), access, sharing, None, OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, None,
    )
    if handle in (None, INVALID_HANDLE):
        return None, ctypes.get_last_error()
    return handle, 0


def mountpoint_bytes(target: Path) -> bytes:
    substitute = ("\\??\\" + str(target)).encode("utf-16le")
    display = str(target).encode("utf-16le")
    names = substitute + b"\x00\x00" + display + b"\x00\x00"
    body = struct.pack("<HHHH", 0, len(substitute), len(substitute) + 2, len(display)) + names
    return struct.pack("<IHH", IO_REPARSE_TAG_MOUNT_POINT, len(body), 0) + body


def set_mountpoint(handle, target: Path):
    payload = ctypes.create_string_buffer(mountpoint_bytes(target))
    returned = wintypes.DWORD()
    success = kernel32.DeviceIoControl(
        handle, FSCTL_SET_REPARSE_POINT, payload, len(payload) - 1,
        None, 0, ctypes.byref(returned), None,
    )
    return bool(success), 0 if success else ctypes.get_last_error()


def exercise(root: Path, label: str, pin_share):
    directory = root / label
    target = root / f"outside-{label}"
    directory.mkdir()
    target.mkdir()
    result = {"case": label, "pinShare": pin_share}
    pin = None
    attack = None
    try:
        if pin_share is not None:
            pin, result["pinOpenWin32"] = open_directory(directory, FILE_READ_ATTRIBUTES, pin_share)
            result["pinOpen"] = pin is not None
            if pin is None:
                return result
        attack, result["attributeOpenWin32"] = open_directory(directory, FILE_WRITE_ATTRIBUTES, 7)
        result["attributeOpen"] = attack is not None
        if attack is None:
            return result
        result["setReparse"], result["setReparseWin32"] = set_mountpoint(attack, target)
        attributes = kernel32.GetFileAttributesW(str(directory))
        result["reparseAttribute"] = attributes != 0xFFFFFFFF and bool(attributes & FILE_ATTRIBUTE_REPARSE_POINT)
        if result["setReparse"]:
            marker = f"probe-{label}-{uuid.uuid4().hex}.txt"
            try:
                with open(directory / marker, "xb") as output:
                    output.write(b"cloud diagnostic only\n")
                result["childCreateWin32"] = 0
                result["redirectedChild"] = (target / marker).is_file()
            except OSError as error:
                result["childCreateWin32"] = error.winerror or 0
                result["redirectedChild"] = False
        return result
    finally:
        if attack is not None:
            kernel32.CloseHandle(attack)
        if pin is not None:
            kernel32.CloseHandle(pin)


def main():
    if os.name != "nt" or os.environ.get("GITHUB_ACTIONS") != "true" or not os.environ.get("RUNNER_TEMP"):
        raise SystemExit("This fixture requires a GitHub Windows runner")
    root = Path(os.environ["RUNNER_TEMP"]) / f"gogoke-r206a-reparse-share-{uuid.uuid4().hex}"
    root.mkdir()
    results = [exercise(root, "control", None)]
    for sharing in (3, 1, 0):
        results.append(exercise(root, f"share-{sharing}", sharing))
    if not results[0].get("setReparse") or not results[0].get("redirectedChild"):
        raise SystemExit("INSTRUMENT_INVALID: unpinned positive control could not redirect a child")
    output = {"schema": "gogoke.r2-06a.reparse-share-probe.v1", "results": results}
    receipt = root / "result.json"
    receipt.write_text(json.dumps(output, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(output, sort_keys=True))
    print("RECEIPT=" + str(receipt))


if __name__ == "__main__":
    main()
