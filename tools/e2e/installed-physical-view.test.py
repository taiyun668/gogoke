"""Offline boundary tests for the shared installed physical view binding."""
import hashlib
import importlib.util
import json
import os
import ast
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


TOOL = Path(__file__).with_name('installed-physical-view.py')
spec = importlib.util.spec_from_file_location('installed_physical_view', TOOL)
physical = importlib.util.module_from_spec(spec)
spec.loader.exec_module(physical)


class PhysicalBindingTests(unittest.TestCase):
    def setUp(self):
        root = Path(os.environ['GOGOKE_PHYSICAL_TEST_DIR'])
        self.assertEqual(root.drive.upper(), 'D:')
        self.scratch = tempfile.TemporaryDirectory(dir=root)
        self.addCleanup(self.scratch.cleanup)
        self.directory = Path(self.scratch.name)
        self.source = self.directory / 'physical-input.json'
        self.output = self.directory / 'physical-output.json'
        self.old_source = '1' * 40
        self.new_source = '2' * 40
        self.config = {
            'schema': 'gogoke.37.installed-physical-view-input.v1',
            'state': 'ROOT_BOUND_COMPONENT_TRANSITION',
            'executionEnabled': True,
            'acceptance': False,
            'toolSourceSha256': hashlib.sha256(TOOL.read_bytes()).hexdigest(),
            'oldComponent': {'candidateNumber': 41, 'sourceCommit': self.old_source,
                             'rootLeaf': 'gogoke37-m2-41-11111111'},
            'newComponent': {'candidateNumber': 42, 'sourceCommit': self.new_source,
                             'rootLeaf': 'gogoke37-m2-42-22222222'},
        }

    def write(self):
        self.source.write_text(json.dumps(self.config), encoding='utf-8')

    def assert_rejected(self):
        self.write()
        with self.assertRaises((RuntimeError, ValueError, json.JSONDecodeError)):
            physical.binding(self.source, self.output)
        self.assertFalse(self.output.exists())

    def test_exact_source_bound_tuple_without_live_capture(self):
        self.write()
        self.assertEqual(physical.binding(self.source, self.output),
                         'gogoke37-m2-41-11111111')
        self.assertFalse(self.output.exists())

    def test_unbound_fails_before_capture_with_poisoned_user_profile(self):
        self.config['executionEnabled'] = False
        self.write()
        env = dict(os.environ, USERPROFILE=str(self.directory / 'no-user-profile'))
        run = subprocess.run([sys.executable, '-E', '-B', str(TOOL),
                              str(self.output), str(self.source)], env=env,
                             capture_output=True, text=True, check=False)
        self.assertNotEqual(run.returncode, 0)
        self.assertIn('bound non-acceptance physical view input required', run.stderr)
        self.assertFalse(self.output.exists())

    def test_malformed_and_prior_transition_fixture_rejected(self):
        self.source.write_text('{', encoding='utf-8')
        with self.assertRaises(json.JSONDecodeError):
            physical.binding(self.source, self.output)
        fixture = os.environ.get('GOGOKE_PHYSICAL_OLD_FIXTURE')
        if fixture:
            old = Path(fixture)
            with self.assertRaises(RuntimeError):
                physical.binding(old, old.with_name('unused-physical-output.json'))

    def test_source_prefix_and_transition_identity_rejected(self):
        self.config['oldComponent']['rootLeaf'] = 'gogoke37-m2-40-11111111'
        self.assert_rejected()
        self.source.unlink()
        self.config['oldComponent']['rootLeaf'] = 'gogoke37-m2-41-11111111'
        self.config['newComponent']['candidateNumber'] = 44
        self.assert_rejected()

    def test_loaded_tool_hash_conflict_rejected(self):
        self.config['toolSourceSha256'] = '0' * 64
        self.assert_rejected()

    def test_foreign_scope_and_missing_input_fail_before_read(self):
        foreign_drive = os.environ['SYSTEMDRIVE']
        with self.assertRaises(RuntimeError):
            physical.binding(Path(foreign_drive + '/foreign/physical-input.json'),
                             Path(foreign_drive + '/foreign/physical-output.json'))
        with self.assertRaises(FileNotFoundError):
            physical.binding(self.source, self.output)
        self.assertFalse(self.output.exists())

    def test_reparse_binding_file_rejected(self):
        target = self.directory / 'target.json'
        target.write_text(json.dumps(self.config), encoding='utf-8')
        try:
            self.source.symlink_to(target)
        except OSError as error:
            self.skipTest(f'symlink unavailable: {error}')
        with self.assertRaisesRegex(RuntimeError, 'reparse binding path'):
            physical.binding(self.source, self.output)
        self.assertFalse(self.output.exists())

    def test_reparse_attribute_fails_before_json_read(self):
        self.write()
        original = Path.lstat

        def flagged(path):
            if path == self.source:
                return SimpleNamespace(st_file_attributes=physical.REPARSE)
            return original(path)

        with mock.patch.object(Path, 'lstat', flagged):
            with mock.patch.object(Path, 'read_text', side_effect=AssertionError('JSON read')):
                with self.assertRaisesRegex(RuntimeError, 'reparse binding path'):
                    physical.binding(self.source, self.output)
        self.assertFalse(self.output.exists())

    def test_original_win32_and_view_blocks_remain_equivalent(self):
        original_path = os.environ.get('GOGOKE_PHYSICAL_OLD_TOOL')
        if not original_path:
            self.skipTest('original D source path not provided')
        old = Path(original_path).read_text(encoding='utf-8')
        new = TOOL.read_text(encoding='utf-8')

        def block(source, first, last):
            start = source.index(first)
            start = source.rfind('\n', 0, start) + 1
            return textwrap.dedent(source[start:source.index(last)]).strip()

        self.assertEqual(block(old, 'def obj(path):', 'home=Path('),
                         block(new, 'def obj(path):', 'def capture('))
        old_handle_setup = block(old, "k=ctypes.WinDLL('kernel32'", 'class Id')
        new_handle_setup = block(new, "k=ctypes.WinDLL('kernel32'", 'home=Path(')
        self.assertEqual([line.strip() for line in old_handle_setup.splitlines()],
                         [line.strip() for line in new_handle_setup.splitlines()])
        pairs = (
            ('home=Path(', "source=stage/'transition78-inputs.json'", 'home=Path(',
             'old_root=local/old_root_leaf'),
            ('try:\n    old_stat=old_root.lstat()', "result={'state':",
             'try:\n        old_stat=old_root.lstat()', "result={'state':"),
            ("result={'state':", "print('REAL_ORDINARY", "result={'state':",
             "print('REAL_ORDINARY"),
        )
        for old_start, old_end, new_start, new_end in pairs:
            old_ast = ast.dump(ast.parse(block(old, old_start, old_end)),
                               include_attributes=False)
            new_ast = ast.dump(ast.parse(block(new, new_start, new_end)),
                               include_attributes=False)
            self.assertEqual(old_ast, new_ast)


if __name__ == '__main__':
    unittest.main()
