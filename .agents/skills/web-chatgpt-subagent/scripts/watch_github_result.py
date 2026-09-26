"""Poll one assigned GitHub result in the background and queue one Codex notice."""

from __future__ import annotations

import argparse
import base64
import ctypes
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(os.environ["LOCALAPPDATA"]) / "gogoke" / "web-chatgpt-subagent"
SLOW_TIERS = {"gpt-6-pro"}


def poll_seconds(tier: str, elapsed_seconds: float) -> int:
    if tier in SLOW_TIERS:
        return 90 if elapsed_seconds < 20 * 60 else 300
    raise ValueError("watcher tier must be GPT-6 Pro")


def gh_json(endpoint: str) -> dict | None:
    result = subprocess.run(["gh", "api", endpoint], capture_output=True, text=True, creationflags=subprocess.CREATE_NO_WINDOW)
    if result.returncode:
        if "HTTP 404" in result.stderr or '"status":"404"' in result.stdout:
            return None
        raise RuntimeError(result.stderr.strip() or f"gh api exited {result.returncode}")
    return json.loads(result.stdout)


def save_receipt(path: Path, data: dict) -> None:
    ROOT.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix="watch-", suffix=".json", dir=ROOT)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as output:
            json.dump(data, output, ensure_ascii=False, indent=2, sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def candidate(repo: str, branch: str, path: str, base_sha: str, task: str) -> dict | None:
    branch_info = gh_json(f"repos/{repo}/branches/{branch}")
    if not branch_info:
        return None
    head = branch_info["commit"]["sha"]
    if head == base_sha:
        return None
    file_info = gh_json(f"repos/{repo}/contents/{path}?ref={branch}")
    if not file_info or file_info.get("type") != "file":
        return None
    content = base64.b64decode(file_info["content"]).decode("utf-8")
    if task not in content:
        return None
    return {"task": task, "repository": repo, "branch": branch, "path": path, "head_sha": head, "file_blob_sha": file_info["sha"], "detected_at": datetime.now(timezone.utc).isoformat(), "status": "candidate_requires_controller_verification"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--task", required=True)
    parser.add_argument("--tier", choices=sorted(SLOW_TIERS), required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--branch", required=True)
    parser.add_argument("--path", required=True)
    parser.add_argument("--base-sha", required=True)
    parser.add_argument("--thread", default=os.environ.get("CODEX_THREAD_ID"))
    parser.add_argument("--max-hours", type=float, default=6)
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_-]+", args.task):
        parser.error("task must be a simple identifier")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repo):
        parser.error("repo must be owner/name")
    if not re.fullmatch(r"[0-9a-fA-F]{40}", args.base_sha):
        parser.error("base-sha must be a full Git SHA")
    if not args.thread:
        parser.error("--thread or CODEX_THREAD_ID is required")
    save_receipt(ROOT / f"watch-{args.task}-process.json", {"task": args.task, "console_window_present": bool(ctypes.windll.kernel32.GetConsoleWindow()), "started_at": datetime.now(timezone.utc).isoformat()})
    receipt = ROOT / f"watch-{args.task}.json"
    if receipt.exists():
        existing = json.loads(receipt.read_text(encoding="utf-8"))
        if existing.get("status") == "candidate_requires_controller_verification":
            return 0
    start = time.monotonic()
    while time.monotonic() - start < args.max_hours * 3600:
        try:
            found = candidate(args.repo, args.branch, args.path, args.base_sha, args.task)
        except (RuntimeError, ValueError, KeyError, OSError) as error:
            found = None
            ROOT.mkdir(parents=True, exist_ok=True)
            (ROOT / f"watch-{args.task}.log").open("a", encoding="utf-8").write(f"{datetime.now(timezone.utc).isoformat()} {error}\n")
        if found:
            save_receipt(receipt, found)
            message = f"{args.task}: GitHub result candidate at {args.repo} {args.branch}@{found['head_sha']} {args.path}. Verify commit, scope, and CI before acceptance."
            notice = subprocess.run(["codex", "queue", "--thread", args.thread, "--message", message], capture_output=True, text=True, creationflags=subprocess.CREATE_NO_WINDOW)
            if notice.returncode:
                found["notification_error"] = notice.stderr.strip() or f"codex queue exited {notice.returncode}"
                save_receipt(receipt, found)
                return 2
            return 0
        elapsed = time.monotonic() - start
        time.sleep(poll_seconds(args.tier, elapsed))
    save_receipt(receipt, {"task": args.task, "status": "watch_deadline_exceeded", "at": datetime.now(timezone.utc).isoformat()})
    return 3


if __name__ == "__main__":
    raise SystemExit(main())
