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
PATROL_SECONDS = 5 * 60
RESULT_REMINDER_SECONDS = 30


def poll_seconds(tier: str, elapsed_seconds: float) -> int:
    if tier in SLOW_TIERS:
        return 90 if elapsed_seconds < 20 * 60 else 300
    raise ValueError("watcher tier must be GPT-6 Pro")


def next_sleep_seconds(tier: str, elapsed_seconds: float, next_patrol_at: float) -> float:
    return min(poll_seconds(tier, elapsed_seconds), max(0, next_patrol_at - elapsed_seconds))


def queue_notice(thread: str, message: str) -> str | None:
    result = subprocess.run(
        ["codex", "queue", "--thread", thread, "--message", message],
        capture_output=True, text=True, creationflags=subprocess.CREATE_NO_WINDOW,
    )
    return (result.stderr.strip() or f"codex queue exited {result.returncode}") if result.returncode else None


def notify_result_until_ack(found: dict, thread: str, receipt: Path, ack: Path) -> None:
    """Repeat a found-result wake every 30 s until Controller acknowledges that SHA."""
    message = (
        f"{found['task']}: GitHub result candidate at {found['repository']} "
        f"{found['branch']}@{found['head_sha']} {found['path']}. "
        "Controller: acknowledge this result wake, then verify commit, scope, and CI."
    )
    while True:
        if ack.is_file() and ack.read_text(encoding="utf-8").strip() == found["head_sha"]:
            found["wake_acknowledged_at"] = datetime.now(timezone.utc).isoformat()
            save_receipt(receipt, found)
            return
        error = queue_notice(thread, message)
        if error:
            found["notification_error"] = error
        else:
            found["last_result_notice_at"] = datetime.now(timezone.utc).isoformat()
            found["result_notice_count"] = found.get("result_notice_count", 0) + 1
        save_receipt(receipt, found)
        time.sleep(RESULT_REMINDER_SECONDS)


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
    parser.add_argument("--patrol-seconds", type=int, default=PATROL_SECONDS)
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_-]+", args.task):
        parser.error("task must be a simple identifier")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repo):
        parser.error("repo must be owner/name")
    if not re.fullmatch(r"[0-9a-fA-F]{40}", args.base_sha):
        parser.error("base-sha must be a full Git SHA")
    if not args.thread:
        parser.error("--thread or CODEX_THREAD_ID is required")
    if args.patrol_seconds <= 0:
        parser.error("--patrol-seconds must be positive")
    save_receipt(ROOT / f"watch-{args.task}-process.json", {"task": args.task, "console_window_present": bool(ctypes.windll.kernel32.GetConsoleWindow()), "started_at": datetime.now(timezone.utc).isoformat()})
    receipt = ROOT / f"watch-{args.task}.json"
    ack = ROOT / f"watch-{args.task}-ack.txt"
    if receipt.exists():
        existing = json.loads(receipt.read_text(encoding="utf-8"))
        if existing.get("status") == "candidate_requires_controller_verification":
            notify_result_until_ack(existing, args.thread, receipt, ack)
            return 0
    start = time.monotonic()
    next_patrol_at = float(args.patrol_seconds)
    while time.monotonic() - start < args.max_hours * 3600:
        try:
            found = candidate(args.repo, args.branch, args.path, args.base_sha, args.task)
        except (RuntimeError, ValueError, KeyError, OSError) as error:
            found = None
            ROOT.mkdir(parents=True, exist_ok=True)
            (ROOT / f"watch-{args.task}.log").open("a", encoding="utf-8").write(f"{datetime.now(timezone.utc).isoformat()} {error}\n")
        if found:
            save_receipt(receipt, found)
            notify_result_until_ack(found, args.thread, receipt, ack)
            return 0
        elapsed = time.monotonic() - start
        if elapsed >= next_patrol_at:
            message = f"{args.task}: five-minute read-only ChatGPT page patrol is due. Inspect the existing tab once, record an actual stop or safety prompt if present, then end this Controller turn. Do not click Retry, Continue, or Send. GitHub result is still watched in the background."
            error = queue_notice(args.thread, message)
            if error:
                with (ROOT / f"watch-{args.task}.log").open("a", encoding="utf-8") as log:
                    log.write(f"{datetime.now(timezone.utc).isoformat()} patrol notification: {error}\n")
            else:
                save_receipt(ROOT / f"watch-{args.task}-patrol.json", {"task": args.task, "notified_at": datetime.now(timezone.utc).isoformat(), "elapsed_seconds": elapsed})
            while next_patrol_at <= elapsed:
                next_patrol_at += args.patrol_seconds
            continue
        time.sleep(next_sleep_seconds(args.tier, elapsed, next_patrol_at))
    save_receipt(receipt, {"task": args.task, "status": "watch_deadline_exceeded", "at": datetime.now(timezone.utc).isoformat()})
    if args.max_hours > 0:
        queue_notice(args.thread, f"{args.task}: GitHub watcher deadline exceeded. Check the ledger and assigned branch; void the task ID before Codex fallback if no accepted result exists.")
    return 3


if __name__ == "__main__":
    raise SystemExit(main())
