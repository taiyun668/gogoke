"""Offline binding regression against an original stopped-source D artifact.

Set GOGOKE_ORIGINAL_STOPPED_SOURCE to the pinned private JSON path. An absent
source is SKIP, never evidence that the original installed case passed.
"""

import importlib.util
import json
import os
from pathlib import Path
import unittest


MODULE = Path(__file__).resolve().parents[1] / "installed_native_readback.py"
SPEC = importlib.util.spec_from_file_location("installed_native_readback", MODULE)
NATIVE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(NATIVE)


class OriginalRows:
    def __init__(self, binding, selection):
        self.binding = binding
        self.selection = selection

    def execute(self, sql, _args):
        if "FROM gogoke_v37_native_selection" in sql:
            return [self.selection]
        if "FROM gogoke_v37_session_binding_v2" in sql:
            return [self.binding]
        if "FROM gogoke_v37_h_seat_binding" in sql:
            return []
        raise AssertionError(f"Unexpected read-only query: {sql}")


@unittest.skipUnless(os.environ.get("GOGOKE_ORIGINAL_STOPPED_SOURCE"),
                     "Original stopped-source D artifact not provided; not a pass")
class ActualStoppedBinding(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        source = json.loads(Path(os.environ["GOGOKE_ORIGINAL_STOPPED_SOURCE"]).read_text(encoding="utf-8-sig"))
        cls.session = source["worktree"]["session"]
        cls.binding = cls.session["nativeBinding"]
        cls.selection = cls.session["nativeSelection"]

    def call(self, db, mode):
        binding = self.binding
        return NATIVE.verify_binding(
            db, domain=binding["domain_id"], session_id=binding["session_id"],
            seat_id=binding["seat_id"], seat_incarnation=binding["seat_incarnation"],
            instance_id=binding["selected_instance_id"], mode=mode)

    def test_original_native_v2_six_fields(self):
        result = self.call(OriginalRows(self.binding, self.selection), "native_v2")
        self.assertEqual(result["mode"], "native_v2")
        self.assertEqual(result["binding"]["provenance"], "NATIVE_V2")

    def test_original_legacy_lookup_must_not_replace_native_v2(self):
        with self.assertRaisesRegex(RuntimeError, "Expected one native row"):
            self.call(OriginalRows(self.binding, self.selection), "legacy")

    def test_selection_identity_mutation_is_rejected(self):
        changed = dict(self.selection, selected_instance_id="different-original-instance")
        with self.assertRaisesRegex(RuntimeError, "native selection/V2"):
            self.call(OriginalRows(self.binding, changed), "native_v2")


if __name__ == "__main__":
    unittest.main()
