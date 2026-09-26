"""Close one browser-send attempt in the local ledger and start GitHub watching."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path


SKILL = Path(__file__).resolve().parents[1]
REPO_ROOT = SKILL.parents[2]
ROOT = Path(os.environ["LOCALAPPDATA"]) / "gogoke" / "web-chatgpt-subagent"
LEDGER = SKILL / "scripts" / "ledger.py"
WATCHER = SKILL / "scripts" / "watch_github_result.py"


def ledger(*args: str) -> None:
    result = subprocess.run([sys.executable, str(LEDGER), *args], capture_output=True, text=True)
    if result.returncode:
        raise ValueError(result.stderr.strip() or f"ledger exited {result.returncode}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--task", required=True)
    parser.add_argument("--status", choices=("sent_confirmed", "not_sent", "uncertain_submission", "needs_login", "security_verification", "limit_no_reply"), required=True)
    parser.add_argument("--url")
    parser.add_argument("--clicked", action="store_true")
    parser.add_argument("--reason", default="browser_dispatch_result")
    args = parser.parse_args()
    receipt_path = ROOT / f"dispatch-{args.task}.json"
    try:
        prepared = json.loads(receipt_path.read_text(encoding="utf-8"))
        if prepared.get("task") != args.task or prepared.get("status") != "prepared":
            raise ValueError("dispatch attempt is missing or already finalized")
        reservation = prepared["reservation_id"]
        status = args.status
        if status == "sent_confirmed":
            if not args.clicked or not args.url:
                raise ValueError("confirmed send requires click and saved conversation URL")
            ledger("mark-sent", reservation, "--url", args.url)
        elif status == "limit_no_reply":
            if not args.clicked:
                raise ValueError("limit-no-reply terminal outcome requires a Send attempt")
            ledger("limit-hit", reservation)
        elif status == "uncertain_submission" or (status == "security_verification" and args.clicked):
            ledger("mark-uncertain", reservation, "--reason", args.reason, *( ["--url", args.url] if args.url else [] ))
        else:
            if args.clicked:
                raise ValueError("a clicked Send cannot be released as not sent")
            ledger("release", reservation)
        if status == "security_verification":
            ledger("cooldown", "--reason", "security_verification")
        prepared.update({"status": status, "clicked": args.clicked, "conversation_url": args.url, "reason": args.reason})
        receipt_path.write_text(json.dumps(prepared, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        output = {"task": args.task, "tier": prepared["tier"], "status": status, "clicked": args.clicked, "reservation_id": reservation}
        if status in ("sent_confirmed", "uncertain_submission"):
            thread_file = ROOT / "controller-thread.txt"
            if not thread_file.is_file():
                output["watcher"] = "not_started_missing_controller_thread"
            else:
                thread = thread_file.read_text(encoding="utf-8").strip()
                process = subprocess.Popen(
                    [sys.executable, str(WATCHER), "--task", args.task, "--repo", prepared["repository"], "--branch", prepared["assigned_branch"], "--path", prepared["result_path"], "--base-sha", prepared["base_sha"], "--thread", thread],
                    cwd=REPO_ROOT,
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    creationflags=subprocess.CREATE_NO_WINDOW | subprocess.DETACHED_PROCESS,
                    close_fds=True,
                )
                output["watcher"] = "started"
                output["watcher_pid"] = process.pid
        if status == "security_verification":
            output["owner_notification_required"] = True
        if status == "limit_no_reply":
            output["codex_fallback_required"] = True
        print(json.dumps(output, ensure_ascii=True))
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(json.dumps({"task": args.task, "status": "finalize_failed", "reason": str(error)}, ensure_ascii=True), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
