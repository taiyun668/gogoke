"""Local, shared accounting for the optional ChatGPT Chat task route (Windows)."""

from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
import uuid
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path

import msvcrt


TIERS = ("gpt-6-pro", "gpt-5.6-sol-pro", "extra-high", "high", "medium")
CAPS = dict(zip(TIERS, (25, 120, 12, 20, 30)))
ROOT = Path(os.environ["LOCALAPPDATA"]) / "gogoke" / "web-chatgpt-subagent"
STATE = ROOT / "state.json"
LOCK = ROOT / "state.lock"


def now() -> datetime:
    return datetime.now(timezone.utc).astimezone()


def parse_time(value: str) -> datetime:
    result = datetime.fromisoformat(value)
    if result.tzinfo is None:
        raise ValueError("time must include a timezone offset")
    return result


def fresh_state() -> dict:
    return {"version": 1, "gpt_6_pro_enabled": False, "events": [], "reservations": {}, "blocked": {}, "resets": {}}


@contextmanager
def locked_state():
    ROOT.mkdir(parents=True, exist_ok=True)
    with LOCK.open("a+b") as handle:
        handle.seek(0)
        if handle.tell() == 0:
            handle.write(b"0")
            handle.flush()
        handle.seek(0)
        msvcrt.locking(handle.fileno(), msvcrt.LK_LOCK, 1)
        try:
            data = json.loads(STATE.read_text(encoding="utf-8")) if STATE.exists() else fresh_state()
            if data.get("version") != 1:
                raise ValueError("unsupported ledger version")
            yield data
            fd, name = tempfile.mkstemp(prefix="state-", suffix=".json", dir=ROOT)
            try:
                with os.fdopen(fd, "w", encoding="utf-8") as output:
                    json.dump(data, output, ensure_ascii=False, indent=2, sort_keys=True)
                    output.write("\n")
                    output.flush()
                    os.fsync(output.fileno())
                os.replace(name, STATE)
            finally:
                if os.path.exists(name):
                    os.unlink(name)
        finally:
            handle.seek(0)
            msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)


def active_events(data: dict, tier: str) -> list[dict]:
    reset = data["resets"].get(tier)
    return [e for e in data["events"] if e["tier"] == tier and (not reset or parse_time(e["at"]) >= parse_time(reset))]


def count_day(data: dict, tier: str, instant: datetime) -> int:
    local_day = instant.date()
    sent = sum(parse_time(e["at"]).astimezone().date() == local_day for e in active_events(data, tier))
    pending = sum(r["tier"] == tier and parse_time(r["at"]).astimezone().date() == local_day for r in data["reservations"].values())
    return sent + pending


def count_week(data: dict, tier: str, instant: datetime) -> int:
    local_week = instant.isocalendar()[:2]
    sent = sum(parse_time(e["at"]).astimezone().isocalendar()[:2] == local_week for e in active_events(data, tier))
    pending = sum(r["tier"] == tier and parse_time(r["at"]).astimezone().isocalendar()[:2] == local_week for r in data["reservations"].values())
    return sent + pending


def available(data: dict, tier: str, instant: datetime) -> tuple[bool, str]:
    if tier == "gpt-6-pro" and not data["gpt_6_pro_enabled"]:
        return False, "GPT-6 Pro disabled"
    block = data["blocked"].get(tier)
    if block:
        return False, f"tier exhausted; observed reset: {block.get('reset_at') or 'unknown'}"
    if count_day(data, tier, instant) >= CAPS[tier]:
        return False, "daily cap reached"
    if tier in TIERS[:2] and sum(count_day(data, t, instant) for t in TIERS[:2]) >= 200:
        return False, "combined Pro daily cap reached"
    if tier == "gpt-6-pro" and len(active_events(data, tier)) + sum(r["tier"] == tier for r in data["reservations"].values()) >= 200:
        return False, "weekly vendor allowance cannot be assumed reset; record observed reset"
    return True, "available in local ledger; reconcile other Chat seats separately"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    commands.add_parser("status")
    reserve = commands.add_parser("reserve")
    reserve.add_argument("--tier", choices=TIERS, required=True)
    reserve.add_argument("--task", required=True)
    reserve.add_argument("--seat", required=True)
    release = commands.add_parser("release")
    release.add_argument("id")
    complete = commands.add_parser("complete")
    complete.add_argument("id")
    complete.add_argument("--actual-tier", choices=TIERS, required=True)
    complete.add_argument("--url")
    complete.add_argument("--seconds", type=int)
    complete.add_argument("--switched", action="store_true")
    complete.add_argument("--reset-at")
    reset = commands.add_parser("reset-tier")
    reset.add_argument("--tier", choices=TIERS, required=True)
    reset.add_argument("--observed-at", required=True)
    reset.add_argument("--reason", required=True)
    enable = commands.add_parser("enable-6-pro")
    enable.add_argument("--owner-decision", required=True)
    args = parser.parse_args()
    try:
        with locked_state() as data:
            instant = now()
            if args.action == "status":
                print(json.dumps({"path": str(STATE), "gpt_6_pro_enabled": data["gpt_6_pro_enabled"], "tiers": {t: {"cap_per_local_day": CAPS[t], "used_or_reserved_today": count_day(data, t, instant), "used_or_reserved_this_calendar_week": count_week(data, t, instant), "available": available(data, t, instant)[0], "reason": available(data, t, instant)[1], "blocked": data["blocked"].get(t)} for t in TIERS}}, ensure_ascii=False, indent=2))
            elif args.action == "reserve":
                ok, reason = available(data, args.tier, instant)
                if not ok:
                    raise ValueError(reason)
                identifier = str(uuid.uuid4())
                data["reservations"][identifier] = {"tier": args.tier, "task": args.task, "seat": args.seat, "at": instant.isoformat()}
                print(identifier)
            elif args.action == "release":
                if args.id not in data["reservations"]:
                    raise ValueError("unknown reservation")
                del data["reservations"][args.id]
                print("released")
            elif args.action == "complete":
                reservation = data["reservations"].pop(args.id, None)
                if reservation is None:
                    raise ValueError("unknown reservation")
                if args.reset_at:
                    parse_time(args.reset_at)
                if args.seconds is not None and args.seconds < 0:
                    raise ValueError("seconds must be nonnegative")
                if args.switched and args.actual_tier == reservation["tier"]:
                    raise ValueError("switched reply must have a different actual tier")
                event = {**reservation, "id": args.id, "actual_tier": args.actual_tier, "completed_at": instant.isoformat(), "switched": args.switched, "seconds": args.seconds, "url": args.url}
                data["events"].append(event)
                if args.switched:
                    data["blocked"][reservation["tier"]] = {"at": instant.isoformat(), "reset_at": args.reset_at}
                print("recorded")
            elif args.action == "reset-tier":
                parse_time(args.observed_at)
                data["resets"][args.tier] = args.observed_at
                data["blocked"].pop(args.tier, None)
                data.setdefault("reset_evidence", []).append({"tier": args.tier, "at": args.observed_at, "reason": args.reason})
                print("reset recorded")
            elif args.action == "enable-6-pro":
                data["gpt_6_pro_enabled"] = True
                data["gpt_6_pro_owner_decision"] = args.owner_decision
                print("GPT-6 Pro enabled")
    except (ValueError, OSError) as error:
        print(str(error), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
