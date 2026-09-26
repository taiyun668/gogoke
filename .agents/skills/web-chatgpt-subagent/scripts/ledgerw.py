"""Run ledger.py with pythonw.exe and write a private, machine-readable receipt."""

from __future__ import annotations

import contextlib
import ctypes
import io
import json
import os
import sys
import tempfile
from pathlib import Path

import ledger


ROOT = Path(os.environ["LOCALAPPDATA"]) / "gogoke" / "web-chatgpt-subagent"


def main() -> int:
    if len(sys.argv) < 4 or sys.argv[1] != "--receipt" or sys.argv[3] != "--":
        return 64
    receipt = Path(sys.argv[2]).resolve()
    if receipt.parent != (ROOT / "receipts").resolve():
        return 65
    ROOT.joinpath("receipts").mkdir(parents=True, exist_ok=True)
    output = io.StringIO()
    errors = io.StringIO()
    old_argv = sys.argv
    try:
        sys.argv = [str(Path(ledger.__file__).resolve()), *old_argv[4:]]
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
            try:
                exit_code = ledger.main()
            except SystemExit as error:
                exit_code = int(error.code) if isinstance(error.code, int) else 64
            except Exception as error:
                exit_code = 70
                print(f"{type(error).__name__}: {error}", file=sys.stderr)
    finally:
        sys.argv = old_argv
    result = {
        "exit_code": exit_code,
        "stdout": output.getvalue().strip(),
        "stderr": errors.getvalue().strip(),
        "console_window_present": bool(ctypes.windll.kernel32.GetConsoleWindow()),
    }
    fd, temp_name = tempfile.mkstemp(prefix="ledgerw-", suffix=".json", dir=receipt.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(result, handle, ensure_ascii=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temp_name, receipt)
    finally:
        if os.path.exists(temp_name):
            os.unlink(temp_name)
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
