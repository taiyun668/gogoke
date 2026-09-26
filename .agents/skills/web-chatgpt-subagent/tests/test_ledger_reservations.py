"""Focused reservation-capacity checks for the Windows ledger CLI."""

import json
import os
import subprocess
import sys
import tempfile
import threading
import unittest
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "ledger.py"


class ReservationCapacityTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.env = {**os.environ, "LOCALAPPDATA": self.tmp.name}
        self.state = Path(self.tmp.name) / "gogoke" / "web-chatgpt-subagent" / "state.json"
        self.assertEqual(self.run_ledger("reconcile", "--source", "unit-test-start").returncode, 0)

    def run_ledger(self, *args):
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args],
            env=self.env,
            text=True,
            capture_output=True,
        )

    def seed_high_with_one_slot_left(self):
        stamp = datetime.now(timezone.utc).astimezone().isoformat()
        self.state.parent.mkdir(parents=True, exist_ok=True)
        state = json.loads(self.state.read_text(encoding="utf-8"))
        state["events"] = [{"tier": "high", "at": stamp} for _ in range(19)]
        self.state.write_text(json.dumps(state), encoding="utf-8")

    def test_release_of_unsent_reservation_restores_capacity(self):
        self.seed_high_with_one_slot_left()

        held = self.run_ledger(
            "reserve", "--tier", "high", "--task", "held", "--seat", "seat-a"
        )
        self.assertEqual(held.returncode, 0, held.stderr)

        blocked = self.run_ledger(
            "reserve", "--tier", "high", "--task", "blocked", "--seat", "seat-b"
        )
        self.assertEqual(blocked.returncode, 2)
        self.assertIn("still active", blocked.stderr)

        released = self.run_ledger("release", held.stdout.strip())
        self.assertEqual(released.returncode, 0, released.stderr)
        self.assertIn("released", released.stdout)

        recovered = self.run_ledger(
            "reserve", "--tier", "high", "--task", "recovered", "--seat", "seat-b"
        )
        self.assertEqual(recovered.returncode, 0, recovered.stderr)

        data = json.loads(self.state.read_text(encoding="utf-8"))
        self.assertEqual(len(data["events"]), 19)
        self.assertEqual(len(data["reservations"]), 1)
        self.assertEqual(next(iter(data["reservations"].values()))["seat"], "seat-b")

    def test_two_seats_cannot_both_take_the_last_slot(self):
        self.seed_high_with_one_slot_left()
        start = threading.Barrier(3)

        def reserve(seat):
            start.wait(timeout=10)
            return self.run_ledger(
                "reserve", "--tier", "high", "--task", f"task-{seat}", "--seat", seat
            )

        with ThreadPoolExecutor(max_workers=2) as pool:
            futures = [pool.submit(reserve, seat) for seat in ("seat-a", "seat-b")]
            start.wait(timeout=10)
            results = [future.result(timeout=30) for future in futures]

        self.assertCountEqual([result.returncode for result in results], [0, 2])
        failed = next(result for result in results if result.returncode == 2)
        self.assertIn("still active", failed.stderr)

        data = json.loads(self.state.read_text(encoding="utf-8"))
        self.assertEqual(len(data["events"]), 19)
        self.assertEqual(len(data["reservations"]), 1)
        winner = next(iter(data["reservations"].values()))
        self.assertIn(winner["seat"], {"seat-a", "seat-b"})
        self.assertEqual(winner["task"], f"task-{winner['seat']}")


if __name__ == "__main__":
    unittest.main()
