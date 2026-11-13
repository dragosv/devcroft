"""Keep the destructive PID 1 command confined to an explicit test fixture."""
import copy
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from build_guest import build
from fault_probe import validate_pid1_fixture_pair

ROOT = Path(__file__).resolve().parent


class Pid1FaultOptIn(unittest.TestCase):
    def pair(self):
        control = {'arch': 'aarch64', 'nix': True, 'source_sha256': 'same-init-source',
                   'keeper_sha256': 'same-keeper', 'busybox_sha256': 'same-busybox',
                   'runtime': {'sha256': 'same-runtime'}, 'initramfs': {'sha256': 'ordinary-init'}}
        guest = copy.deepcopy(control)
        guest.update(pid1_exit_probe=True, initramfs={'sha256': 'fault-enabled-init'})
        return guest, control

    def test_matching_enabled_and_disabled_guests_are_accepted(self):
        validate_pid1_fixture_pair(*self.pair())

    def test_normal_guest_cannot_be_the_destructive_target(self):
        _, control = self.pair()
        with self.assertRaisesRegex(ValueError, 'enabled fault fixture'):
            validate_pid1_fixture_pair(control, control)

    def test_control_guest_must_have_no_exit_hook(self):
        guest, control = self.pair()
        for value in (True, 1, 'false'):
            with self.subTest(value=value):
                control['pid1_exit_probe'] = value
                with self.assertRaises(ValueError):
                    validate_pid1_fixture_pair(guest, control)

    def test_mismatched_payloads_cannot_be_a_negative_control(self):
        for key in ('arch', 'source_sha256', 'keeper_sha256', 'busybox_sha256', 'nix'):
            with self.subTest(key=key):
                guest, control = self.pair()
                control[key] = 'different'
                with self.assertRaisesRegex(ValueError, 'mismatch'):
                    validate_pid1_fixture_pair(guest, control)
        guest, control = self.pair()
        control['runtime']['sha256'] = 'different-runtime'
        with self.assertRaisesRegex(ValueError, 'runtime mismatch'):
            validate_pid1_fixture_pair(guest, control)

    def test_identical_initramfs_is_not_an_enabled_disabled_pair(self):
        guest, control = self.pair()
        guest['initramfs'] = control['initramfs']
        with self.assertRaisesRegex(ValueError, 'must differ'):
            validate_pid1_fixture_pair(guest, control)

    def test_fault_build_requires_explicit_nix_and_output(self):
        result = subprocess.run([sys.executable, str(ROOT / 'build_keeper_guest.py'),
                                 '--pid1-exit-probe', '--nix'], capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 2)
        self.assertIn(b'requires --nix --out', result.stderr)

    def test_fault_build_cannot_overwrite_an_existing_guest(self):
        with tempfile.TemporaryDirectory() as temp:
            receipt = Path(temp) / 'guest.json'
            receipt.write_text('keep-existing-guest')
            result = subprocess.run([sys.executable, str(ROOT / 'build_keeper_guest.py'),
                                     '--pid1-exit-probe', '--nix', '--out', temp],
                                    capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 2)
            self.assertIn(b'must be new', result.stderr)
            self.assertEqual(receipt.read_text(), 'keep-existing-guest')

    def test_generic_builder_refuses_hooks_in_other_guest_types(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp) / 'should-not-exist'
            with self.assertRaisesRegex(ValueError, 'separate Nix keeper fault fixture'):
                build(out, 'aarch64', 'unused-zig', pid1_exit_probe=True)
            self.assertFalse(out.exists())


if __name__ == '__main__':
    unittest.main()
