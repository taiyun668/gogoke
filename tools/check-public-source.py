#!/usr/bin/env python3
"""Reject machine-local paths and credential material in committed public files."""

from __future__ import annotations

import argparse
import base64
import binascii
import re
import subprocess
import sys
import tempfile
from pathlib import Path


DRIVE_PATH = re.compile(r"(?i)(?<![A-Za-z0-9])(?:[A-Z]:[\\/][^\s\"'<>|?*]+|\\\\[A-Za-z0-9._-]+\\[A-Za-z0-9$._-]+(?:\\[^\s\"'<>|?*]*)?)")
UNIX_HOME_PATH = re.compile(r"(?<![A-Za-z0-9./])/(?:home|Users|root)/[^/\s\"'<>]+(?:/[^\s\"'<>]*)?")
UNIX_LOCAL_PATH = re.compile(r"(?<![A-Za-z0-9./])/(?:mnt/data|tmp|private|var/tmp)/[^\s\"'<>]+")
TOKEN = re.compile(r"(?i)(?:github_pat_[A-Za-z0-9_]{20,}|gh[pousr]_[A-Za-z0-9]{30,}|sk-[A-Za-z0-9]{20,}|AKIA[0-9A-Z]{16}|\bBearer\s+[A-Za-z0-9._~+/-]{24,}={0,2})")
PRIVATE_KEY_MARKER = re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----|-----BEGIN PGP PRIVATE KEY BLOCK-----")
PRIVATE_KEY_BLOCK = re.compile(
    r"-----BEGIN (RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----\s*"
    r"([A-Za-z0-9+/=\s]{64,}?)\s*-----END \1?PRIVATE KEY-----",
    re.MULTILINE,
)
SYNTHETIC_USER_NAMES = {
    "a", "ada", "alice", "alpha", "beta", "bill", "browser-user", "dara", "demo",
    "dev", "example", "expo", "fred", "gogo-party", "jane", "josh", "julius",
    "kelchm", "local", "maria", "marlow", "matt", "me", "mike", "other",
    "runner", "runneradmin", "runner~1", "samgoodwin", "shawn", "showcase",
    "someone", "sotiriskaniras", "sprite", "tester", "testuser", "test",
    "tests", "theo", "user", "user1", "username", "vlad", "vscode", "x",
}

def _example_user(name: str) -> bool:
    return name in SYNTHETIC_USER_NAMES or name.startswith(".") or any(ch in name for ch in ("$", "{", "}", "`"))

def _path_classification(value: str, source_path: str) -> tuple[str, str]:
    raw = value.replace("\\", "/")
    normalized = (raw if raw.startswith("//./") else re.sub(r"/+", "/", raw)).lower()
    source = source_path.replace("\\", "/").lower()
    if normalized.startswith("//./pipe/") or normalized.startswith("//./nul"):
        return "EXPLAINED", "windows-device-namespace"
    if normalized.startswith("/mnt/data/") or any(p in normalized for p in ("/gogo patty/", "/sandglass/", "/cm-zh/", "/gogoke-construction/")):
        return "LEAK", "project-local-path"
    if "/users/" in normalized:
        user = normalized.split("/users/", 1)[1].split("/", 1)[0]
        if not user:
            return "EXPLAINED", "profile-placeholder"
        if user == Path.home().name.lower():
            return "LEAK", "current-user-profile-path"
        if _example_user(user):
            return "EXPLAINED", "synthetic-user-path"
        return "LEAK", "user-profile-path"
    if normalized.startswith("c:/windows") or any(marker in normalized for marker in ("/program", "/windows/", ":/tmp/", "/tmp/", "/home/fred/", "/users/julius/", "/root/runner/", ":/repo/")):
        return "EXPLAINED", "system-or-synthetic-path"
    is_test_source = source.endswith("/tests.rs") or any(x in source for x in ("/test/", "/tests/", "/fixture/", "/fixtures/", ".test."))
    if is_test_source:
        return "EXPLAINED", "synthetic-test-path"
    try:
        if Path(value).is_absolute() and Path(value).exists():
            return "LEAK", "existing-local-path"
    except OSError:
        pass
    if normalized.startswith("/home/") or normalized.startswith("/root/"):
        name = normalized.split("/", 3)[2]
        if _example_user(name):
            return "EXPLAINED", "synthetic-user-path"
        if name == Path.home().name.lower():
            return "LEAK", "current-user-home-path"
        return "LEAK", "user-home-path"
    if source.startswith("third_party/t3code/") or source.startswith("apps/desktop/native-host/vendor/"):
        return "EXPLAINED", "public-vendor-path-example"
    return "LEAK", "absolute-path"


def findings(text: str, source_path: str = "") -> list[tuple[int, str, str, str]]:
    """Return leak and explained-candidate records without exposing values."""
    found: set[tuple[int, str, str, str]] = set()
    source = source_path.replace("\\", "/").lower()
    lines = text.splitlines()
    rust_test_region = False
    for line_number, line in enumerate(lines, start=1):
        if "#[cfg(test)]" in line:
            rust_test_region = True
        for pattern in (DRIVE_PATH, UNIX_HOME_PATH, UNIX_LOCAL_PATH):
            for match in pattern.finditer(line):
                classification, reason = _path_classification(match.group(0), source_path)
                if rust_test_region and classification == "LEAK" and reason not in {"current-user-profile-path", "user-profile-path", "current-user-home-path", "user-home-path", "project-local-path"}:
                    classification, reason = "EXPLAINED", "synthetic-rust-test-path"
                found.add((line_number, "machine-path", classification, reason))
        for match in TOKEN.finditer(line):
            value = match.group(0).lower()
            synthetic = any(label in value for label in ("example", "fake", "dummy", "test", "abcdef"))
            reason = "synthetic-token" if synthetic else "credential-format"
            found.add((line_number, "credential-token", "EXPLAINED" if synthetic else "LEAK", reason))
        for _ in PRIVATE_KEY_MARKER.finditer(line):
            found.add((line_number, "private-key-marker", "EXPLAINED", "marker-without-validated-block"))
    for match in PRIVATE_KEY_BLOCK.finditer(text):
        body = re.sub(r"\s+", "", match.group(2))
        try:
            decoded = base64.b64decode(body, validate=True)
        except (binascii.Error, ValueError):
            continue
        if len(decoded) >= 64:
            line_number = text.count("\n", 0, match.start()) + 1
            found.add((line_number, "private-key-block", "LEAK", "complete-base64-private-key-block"))
    return sorted(found)


def committed_blobs(root: Path):
    """Yield bytes from HEAD's blobs, independent of the checkout contents."""
    result = subprocess.run(
        ["git", "-C", str(root), "ls-tree", "-r", "-z", "HEAD"],
        check=True,
        stdout=subprocess.PIPE,
    )
    entries = []
    for item in result.stdout.split(b"\0"):
        if not item:
            continue
        metadata, name = item.split(b"\t", 1)
        _, kind, oid = metadata.split()
        if kind != b"blob":
            raise ValueError("committed tree contains a non-blob entry")
        entries.append((root / name.decode("utf-8", errors="surrogateescape"), oid))
    process = subprocess.Popen(
        ["git", "-C", str(root), "cat-file", "--batch"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    assert process.stdin is not None and process.stdout is not None
    try:
        for path, oid in entries:
            process.stdin.write(oid + b"\n")
            process.stdin.flush()
            header = process.stdout.readline().split()
            if len(header) != 3 or header[0] != oid or header[1] != b"blob":
                raise ValueError("committed blob identity mismatch")
            size = int(header[2])
            data = process.stdout.read(size)
            if len(data) != size or process.stdout.read(1) != b"\n":
                raise ValueError("committed blob frame is incomplete")
            yield path, data
    finally:
        process.stdin.close()
        status = process.wait()
        process.stdout.close()
        if status != 0:
            raise ValueError("git cat-file failed")


def worktree_files(root: Path) -> list[Path]:
    normal = subprocess.run(
        ["git", "-C", str(root), "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        check=True,
        stdout=subprocess.PIPE,
    )
    ignored = subprocess.run(
        ["git", "-C", str(root), "ls-files", "--others", "--ignored", "--exclude-standard", "-z"],
        check=True,
        stdout=subprocess.PIPE,
    )
    names = {
        name
        for output in (normal.stdout, ignored.stdout)
        for name in output.decode("utf-8", errors="surrogateescape").split("\0")
        if name
    }
    return [root / name for name in sorted(names) if (root / name).is_file()]


def scan_bytes(path: Path, data: bytes, root: Path, worktree_mode: bool = False) -> list[tuple[Path, int, str, str, str]]:
    results: list[tuple[Path, int, str, str, str]] = []
    control_bytes = sum(byte < 32 and byte not in (9, 10, 13) for byte in data)
    is_binary = b"\0" in data or (data and control_bytes / len(data) > 0.01)
    if is_binary:
        # Inspect printable runs so embedded ASCII credentials are visible,
        # without interpreting arbitrary image/compressed bytes as paths.
        text = "\n".join(run.decode("ascii") for run in re.findall(rb"[ -~]{5,}", data))
    else:
        text = data.decode("utf-8", errors="replace")
    source_path = path.relative_to(root).as_posix() if path.is_relative_to(root) else path.name
    for line_number, rule_name, classification, reason in findings(text, source_path):
        if worktree_mode and path.suffix == ".pyc" and "__pycache__" in path.parts:
            classification, reason = "EXPLAINED", "ignored-generated-bytecode"
        results.append((path, line_number, rule_name, classification, reason))
    return results


def scan_files(paths: list[Path], root: Path, worktree_mode: bool = False) -> list[tuple[Path, int, str, str, str]]:
    results: list[tuple[Path, int, str, str, str]] = []
    for path in paths:
        try:
            data = path.read_bytes()
        except OSError as exc:
            print(f"SCAN_ERROR file={path.name} error={type(exc).__name__}", file=sys.stderr)
            results.append((path, 0, "unreadable-file", "LEAK", "unreadable-file"))
            continue
        results.extend(scan_bytes(path, data, root, worktree_mode))
    return results


def positive_control() -> bool:
    # Build each marker from fragments so the control itself does not trigger the
    # committed-source scan. The sample is temporary and never enters Git.
    key_body = base64.b64encode(b"synthetic-private-key-material-that-is-not-secret-" * 3).decode("ascii")
    sample = "\n".join(
        (
            "owner path: " + "C:" + "\\Users\\gogoke-control-user\\private.txt",
            "token: " + "gh" + "p_" + "A" * 36,
            "-----BEGIN PRIVATE KEY-----",
            key_body,
            "-----END PRIVATE KEY-----",
        )
    )
    with tempfile.TemporaryDirectory(prefix="gogoke-public-source-control-") as temp_dir:
        fixture = Path(temp_dir) / "known-sensitive-sample.txt"
        fixture.write_text(sample, encoding="utf-8")
        process = subprocess.run(
            [sys.executable, str(Path(__file__).resolve()), "--file", str(fixture)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
    categories = sorted({
        line.split(" rule=", 1)[1].split(" ", 1)[0]
        for line in process.stdout.splitlines() if " rule=" in line
    })
    expected = {"machine-path", "credential-token", "private-key-block"}
    # This line is the auditable positive-control result requested for the export report.
    print(
        "POSITIVE_CONTROL sample=known-sensitive-sample "
        f"expected_exit=1 actual_exit={process.returncode} "
        f"findings={','.join(categories) or 'none'}"
    )
    return process.returncode == 1 and expected.issubset(categories)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path.cwd(), help="repository root")
    parser.add_argument(
        "--file",
        type=Path,
        help=argparse.SUPPRESS,
    )
    parser.add_argument("--worktree", action="store_true", help="scan tracked, untracked, and ignored working-tree files")
    parser.add_argument("--report", type=Path, help="write the complete machine report outside the repository")
    parser.add_argument("--quiet", action="store_true", help="print only the control and summary; preserve detailed findings with --report")
    parser.add_argument("--control-only", action="store_true", help="run the positive control without scanning the repository")
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run a known-sensitive positive control before scanning committed files",
    )
    args = parser.parse_args(argv)
    root = args.root.resolve()

    if args.control_only and not args.self_test:
        parser.error("--control-only requires --self-test")

    if args.self_test and not positive_control():
        print("POSITIVE_CONTROL_RESULT=FAIL", file=sys.stderr)
        return 2
    if args.self_test:
        print("POSITIVE_CONTROL_RESULT=PASS (sample was rejected with scanner exit 1)")
    if args.control_only:
        return 0

    try:
        if args.file:
            paths = [args.file.resolve()]
        elif args.worktree:
            paths = worktree_files(root)
        else:
            paths = []
    except (OSError, subprocess.CalledProcessError, ValueError) as exc:
        print(f"SCAN_ERROR file_list_read={type(exc).__name__}", file=sys.stderr)
        return 2
    try:
        if args.file or args.worktree:
            results = scan_files(paths, root, args.worktree)
            file_count = len(paths)
        else:
            results = []
            file_count = 0
            for path, data in committed_blobs(root):
                results.extend(scan_bytes(path, data, root))
                file_count += 1
    except (OSError, subprocess.CalledProcessError, ValueError) as exc:
        print(f"SCAN_ERROR committed_blob_read={type(exc).__name__}", file=sys.stderr)
        return 2
    report_lines: list[str] = []
    for path, line_number, category, classification, reason in results:
        relative = path.relative_to(root).as_posix() if path.is_relative_to(root) else path.name
        report_lines.append(f"{classification} file={relative} line={line_number} rule={category} reason={reason}")
    if not args.quiet:
        for report_line in report_lines:
            print(report_line)
    leaks = sum(1 for _, _, _, classification, _ in results if classification == "LEAK")
    explained = len(results) - leaks
    summary = f"PUBLIC_SOURCE_SCAN files={file_count} leaks={leaks} explained={explained} candidates={len(results)}"
    print(summary)
    if args.report:
        args.report.write_text("\n".join(report_lines + [summary]) + "\n", encoding="utf-8")
        print(f"MACHINE_REPORT={args.report.resolve()}")
    return 1 if leaks else 0


if __name__ == "__main__":
    raise SystemExit(main())
