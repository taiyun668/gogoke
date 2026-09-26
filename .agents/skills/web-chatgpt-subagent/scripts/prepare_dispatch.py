"""Verify a committed GitHub task card, reserve its tier, and emit one browser call."""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import subprocess
import sys
import uuid
from pathlib import Path


SKILL = Path(__file__).resolve().parents[1]
REPO_ROOT = SKILL.parents[2]
ROOT = Path(os.environ["LOCALAPPDATA"]) / "gogoke" / "web-chatgpt-subagent"
LEDGER = SKILL / "scripts" / "ledger.py"
TEMPLATE = SKILL / "scripts" / "dispatch_browser.js"
TIERS = {"gpt-5.6-sol-pro", "extra-high", "high", "medium"}


def command(*args: str) -> str:
    result = subprocess.run(args, cwd=REPO_ROOT, capture_output=True, text=True)
    if result.returncode:
        raise ValueError(result.stderr.strip() or f"command failed: {args[0]}")
    return result.stdout.strip()


def git_blob(path: str) -> bytes:
    result = subprocess.run(["git", "show", f"HEAD:{path}"], cwd=REPO_ROOT, capture_output=True)
    if result.returncode:
        raise ValueError("task card is not committed at HEAD")
    return result.stdout


def safe_relpath(value: str) -> str:
    path = Path(value)
    if path.is_absolute() or not value or ".." in path.parts or ":" in value:
        raise ValueError("task card paths must be repository-relative")
    return path.as_posix()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--task", required=True)
    parser.add_argument("--tier", required=True)
    parser.add_argument("--card", required=True)
    args = parser.parse_args()
    try:
        if args.tier not in TIERS:
            raise ValueError("GPT-6 Pro and unknown tiers are disabled for this trial")
        card_path = safe_relpath(args.card)
        if command("git", "status", "--porcelain"):
            raise ValueError("commit task instructions and leave checkout clean before dispatch")
        commit = command("git", "rev-parse", "HEAD")
        card_bytes = git_blob(card_path)
        card = json.loads(card_bytes)
        if card.get("task_id") != args.task or card.get("tier") != args.tier:
            raise ValueError("task ID or tier differs from committed card")
        repo = card.get("repository")
        if repo != "taiyun668/gogoke":
            raise ValueError("repository differs from the assigned gogoke route")
        branch = card.get("assigned_branch")
        if not isinstance(branch, str) or not branch.startswith("gpt/"):
            raise ValueError("assigned branch must be a dedicated gpt/ branch")
        base = card.get("base_sha")
        if not isinstance(base, str) or not re.fullmatch(r"[0-9a-fA-F]{40}", base):
            raise ValueError("base_sha must be a full commit SHA")
        result_path = safe_relpath(card.get("result_path", ""))
        writes = card.get("allowed_write_paths")
        if not isinstance(writes, list) or not writes or any(not isinstance(p, str) for p in writes):
            raise ValueError("allowed_write_paths must be a nonempty list")
        writes = [safe_relpath(p) for p in writes]
        if result_path not in writes:
            raise ValueError("result_path must be among allowed_write_paths")
        for field in ("role", "goal", "success_criteria", "stop_condition"):
            if not isinstance(card.get(field), str) or not card[field].strip():
                raise ValueError(f"{field} required")
        remote = json.loads(command("gh", "api", f"repos/{repo}/contents/{card_path}?ref={commit}"))
        if base64.b64decode(remote["content"]) != card_bytes:
            raise ValueError("remote committed task card differs from local bytes")
        starter = (
            f"{args.task}；角色：{card['role']}。仓库：{repo}。任务说明：{commit} 的 {card_path}。"
            f"基准提交：{base}。仅允许写 {branch} 的 {', '.join(writes)}。"
            f"结果：{result_path}。停止条件：{card['stop_condition']}。"
        )
        template = TEMPLATE.read_text(encoding="utf-8")
        if "__CONFIG__" not in template:
            raise ValueError("browser script template is missing its task payload marker")
        thread_file = ROOT / "controller-thread.txt"
        if not thread_file.is_file():
            raise ValueError("local controller-thread.txt is required for background result notification")
        uuid.UUID(thread_file.read_text(encoding="utf-8").strip())
        reservation = command(sys.executable, str(LEDGER), "reserve", "--tier", args.tier, "--task", args.task, "--seat", "web-channel")
        try:
            ROOT.mkdir(parents=True, exist_ok=True)
            prepared = {"task": args.task, "tier": args.tier, "reservation_id": reservation, "repository": repo, "assigned_branch": branch, "base_sha": base, "result_path": result_path, "card_commit": commit, "card_path": card_path, "status": "prepared"}
            (ROOT / f"dispatch-{args.task}.json").write_text(json.dumps(prepared, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            script = template.replace("__CONFIG__", json.dumps({"task": args.task, "tier": args.tier, "starter": starter}, ensure_ascii=False))
        except OSError:
            command(sys.executable, str(LEDGER), "release", reservation)
            raise
        print(json.dumps({"task": args.task, "tier": args.tier, "reservation_id": reservation, "browser_code": script, "status": "prepared"}, ensure_ascii=True))
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(json.dumps({"task": args.task, "status": "prepare_failed", "reason": str(error)}, ensure_ascii=True), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
