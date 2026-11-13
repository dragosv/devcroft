"""Verify storage units, checkpoint identity and actual sparse-file accounting."""
import os
from pathlib import Path
import tempfile
import unittest

from setup_storage import HostAllocationSampler, checkpoint_space, filesystem_space, runtime_space


class SetupStorageValidation(unittest.TestCase):
    def test_guest_block_units_and_reserved_space_are_distinct(self):
        r = filesystem_space(4096, 100, 40, 30)
        self.assertEqual(r['capacity_bytes'], 409600)
        self.assertEqual(r['used_bytes'], 245760)
        self.assertEqual(r['free_bytes'], 163840)
        self.assertEqual(r['available_bytes'], 122880)

    def test_impossible_counters_cannot_be_reported_as_measurements(self):
        for args in [(0, 100, 40, 30), (4096, 0, 0, 0), (4096, 100, 101, 30),
                     (4096, 100, 40, 41), (4096, 100, -1, 0)]:
            with self.subTest(args=args), self.assertRaises(ValueError):
                filesystem_space(*args)

    def test_checkpoint_accepts_crlf_and_matches_only_the_requested_point(self):
        data = (b'guest: other log\n'
                b'guest: test-only setup storage point=recipe-staged block_size=4096 blocks=100 free=40 available=30\r\n')
        self.assertEqual(checkpoint_space(data, 'recipe-staged')['used_bytes'], 245760)
        with self.assertRaises(ValueError):
            checkpoint_space(data, 'recipe-installed')
        with self.assertRaises(ValueError):
            checkpoint_space(data + data, 'recipe-staged')

    def test_runtime_observation_has_the_same_units_and_rejects_extra_data(self):
        self.assertEqual(runtime_space(b'setup-space 4096 100 40 30\n'), filesystem_space(4096, 100, 40, 30))
        for value in [b'', b'4096 100 40 30', b'setup-space 4096 100 40 30 extra', b'setup-space no 100 40 30']:
            with self.subTest(value=value), self.assertRaises(ValueError):
                runtime_space(value)

    def test_host_sampler_separates_sparse_capacity_from_allocated_storage(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'private.img'
            with path.open('wb') as file:
                file.truncate(1024 * 1024)
            sampler = HostAllocationSampler([Path(temp)])
            sampler.start()
            before = sampler.capture('sparse-control')
            with path.open('r+b') as file:
                file.write(os.urandom(65536))
                file.flush()
                os.fsync(file.fileno())
            written = sampler.capture('written-control')
            result = sampler.finish()
            self.assertEqual(before['logical_bytes'], 1024 * 1024)
            self.assertLess(before['allocated_bytes'], before['logical_bytes'])
            self.assertGreater(written['allocated_bytes'], before['allocated_bytes'])
            self.assertGreaterEqual(result['observed_peak']['allocated_bytes'], written['allocated_bytes'])
            self.assertEqual(result['errors'], [])
            self.assertGreaterEqual(result['sample_count'], 4)

    def test_hardlinked_files_count_once_and_symlinks_are_not_followed(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            original = root / 'original'
            original.write_bytes(os.urandom(4096))
            os.link(original, root / 'hardlink')
            (root / 'symlink').symlink_to(original)
            record = HostAllocationSampler([root]).capture('identity-control')
            self.assertEqual(record['logical_bytes'], 4096)
            self.assertEqual(len(record['files']), 1)

    def test_unbounded_sampling_intervals_are_refused(self):
        for interval in (0, -1, 0.001, 2):
            with self.subTest(interval=interval), self.assertRaises(ValueError):
                HostAllocationSampler([], interval)


if __name__ == '__main__':
    unittest.main()
