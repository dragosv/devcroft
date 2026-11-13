"""Reject false passing classifications and accidental setup fault fixtures."""
import copy
import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from adaptable.recipe import encoded, load
from build_guest import build
from setup_interrupt_probe import classify_output, validate_guests, validate_inputs

ROOT = Path(__file__).resolve().parent
RECIPE = load(ROOT / 'fixtures/adaptable-recipe-aarch64.json')
RECIPE_ID = hashlib.sha256(encoded(RECIPE)).hexdigest()


class SetupInterruptionValidation(unittest.TestCase):
    def guests(self):
        runtime = {'adaptable': True, 'arch': 'aarch64', 'setup_window': False,
                   'setup_recipe_sha256': RECIPE_ID, 'keeper_sha256': 'keeper',
                   'busybox_sha256': 'shell', 'source_sha256': 'atomic-source',
                   'runtime': {'sha256': 'runtime-disk'}, 'initramfs': {'sha256': 'runtime-init'}}
        setup = copy.deepcopy(runtime)
        setup.update(setup_window=True, initramfs={'sha256': 'setup-init'})
        fault = copy.deepcopy(setup)
        fault.update(setup_fault_probe=True, initramfs={'sha256': 'fault-init'})
        return {'runtime': runtime, 'setup': setup, 'fault': fault}

    def test_complete_baseline_signature(self):
        self.assertEqual(classify_output(b'absent /usr/include/zlib.h\n'
                         b'absent /private-base/rootfs/.recipe-applied\nzlib-dev-db 0\n', RECIPE_ID), 'baseline')

    def test_missing_observations_cannot_pass_as_baseline(self):
        for output in (b'zlib-dev-db 0\n', b'absent /usr/include/zlib.h\nzlib-dev-db 0\n'):
            with self.subTest(output=output):
                self.assertEqual(classify_output(output, RECIPE_ID), 'partial')

    def test_complete_applied_signature(self):
        output = ('present /usr/include/zlib.h\npresent /private-base/rootfs/.recipe-applied\n'
                  'zlib-dev-db 1\nmarker ' + RECIPE_ID + '\n').encode()
        self.assertEqual(classify_output(output, RECIPE_ID), 'applied')

    def test_incomplete_or_wrong_recipe_never_passes(self):
        for marker, database in [('absent', '1'), ('present', '0'), ('present', '1')]:
            output = ('present /usr/include/zlib.h\n' + marker +
                      ' /private-base/rootfs/.recipe-applied\nzlib-dev-db ' + database + '\nmarker wrong\n').encode()
            with self.subTest(marker=marker, database=database):
                self.assertEqual(classify_output(output, RECIPE_ID), 'partial')

    def test_matching_atomic_fixture_roles(self):
        validate_guests(self.guests(), RECIPE_ID, 'atomic')

    def test_fault_hook_cannot_be_used_as_ordinary_runtime_or_setup(self):
        for name in ('runtime', 'setup'):
            guests = self.guests()
            guests[name]['setup_fault_probe'] = True
            with self.subTest(name=name), self.assertRaisesRegex(AssertionError, 'roles'):
                validate_guests(guests, RECIPE_ID, 'atomic')

    def test_missing_or_nonboolean_fault_opt_in_is_rejected(self):
        for value in (False, 1, 'true'):
            guests = self.guests()
            guests['fault']['setup_fault_probe'] = value
            with self.subTest(value=value), self.assertRaisesRegex(AssertionError, 'roles'):
                validate_guests(guests, RECIPE_ID, 'atomic')

    def test_mismatched_payloads_and_sources_are_rejected(self):
        for key in ('keeper_sha256', 'busybox_sha256', 'source_sha256'):
            guests = self.guests()
            guests['fault'][key] = 'different'
            with self.subTest(key=key), self.assertRaises(AssertionError):
                validate_guests(guests, RECIPE_ID, 'atomic')
        guests = self.guests()
        guests['fault']['initramfs'] = guests['setup']['initramfs']
        with self.assertRaisesRegex(AssertionError, 'distinct'):
            validate_guests(guests, RECIPE_ID, 'atomic')

    def test_input_bundle_and_seed_must_match_pinned_build_receipt(self):
        build_receipt = {'recipe': RECIPE, 'recipe_sha256': RECIPE_ID,
                         'toolchain.img': {'sha256': 'tools'}, 'data.seed.img': {'sha256': 'seed'}}
        validate_inputs(RECIPE, build_receipt, {'sha256': 'tools'}, {'sha256': 'seed'})
        for inputs, seed in [('different', 'seed'), ('tools', 'different')]:
            with self.subTest(inputs=inputs, seed=seed), self.assertRaises(AssertionError):
                validate_inputs(RECIPE, build_receipt, {'sha256': inputs}, {'sha256': seed})
        changed = copy.deepcopy(build_receipt)
        changed['recipe_sha256'] = 'different'
        with self.assertRaisesRegex(AssertionError, 'recipe'):
            validate_inputs(RECIPE, changed, {'sha256': 'tools'}, {'sha256': 'seed'})

    def test_setup_fault_build_requires_explicit_separate_output(self):
        result = subprocess.run([sys.executable, str(ROOT / 'build_keeper_guest.py'),
                                 '--adaptable', '--setup-window', '--setup-fault-probe'],
                                capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 2)
        self.assertIn(b'requires --adaptable --setup-window --out', result.stderr)

    def test_setup_fault_build_preserves_existing_guest(self):
        with tempfile.TemporaryDirectory() as temp:
            receipt = Path(temp) / 'guest.json'
            receipt.write_text('preserve-existing-guest')
            result = subprocess.run([sys.executable, str(ROOT / 'build_keeper_guest.py'),
                                     '--adaptable', '--setup-window', '--setup-fault-probe', '--out', temp],
                                    capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 2)
            self.assertIn(b'must be new', result.stderr)
            self.assertEqual(receipt.read_text(), 'preserve-existing-guest')

    def test_generic_builder_refuses_hooks_without_setup_fixture(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp) / 'must-not-exist'
            with self.assertRaisesRegex(ValueError, 'separate setup-window fault fixture'):
                build(out, 'aarch64', 'unused-zig', adaptable=True, setup=True, setup_fault_probe=True)
            self.assertFalse(out.exists())


if __name__ == '__main__':
    unittest.main()
