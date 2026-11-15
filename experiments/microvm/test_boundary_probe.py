"""Do not mistake a missing target, timeout or absent protocol for policy denial."""
import copy
from pathlib import Path
import tempfile
import unittest

from boundary_probe import (CACHE_DENIAL, LINUX_ERRNO, NO_ROUTE, POSITIVE, RAW_DENIAL,
                            TCP_DENIAL, assess_child, assess_raw_cache, validate_pair)
from build_guest import build


class BoundaryValidation(unittest.TestCase):
    def data(self):
        rows = [{'name': name, 'rc': 0, 'errno': 0} for name in sorted(POSITIVE)]
        rows += [{'name': name, 'rc': -1, 'errno': 13} for name in sorted(TCP_DENIAL | RAW_DENIAL)]
        rows += [{'name': name, 'rc': -1, 'errno': 101} for name in sorted(NO_ROUTE)]
        rows += [{'name': name, 'rc': -1, 'errno': 30} for name in sorted(CACHE_DENIAL)]
        return {'schema': 1, 'errno_constants': dict(LINUX_ERRNO), 'ipv4_interfaces': ['lo'],
                'ipv6_socket_errno': 97, 'http_instance_id': 'expected', 'checks': rows}

    def test_layered_results_preserve_udp_positive_and_route_distinction(self):
        rows = assess_child(self.data(), 'expected')
        self.assertEqual(len(rows), 20)
        self.assertTrue(all(row['status'] == 'passed' for row in rows))
        layers = {row['name']: row['enforcement_layer'] for row in rows}
        self.assertEqual(layers['udp_unlisted_loopback_delivery'], 'positive-control')
        self.assertEqual(layers['udp_external_send'], 'guest-no-route')

    def test_missing_cache_refused_connection_and_timeout_do_not_pass(self):
        for name, error in [('cache_append', 2), ('cache_append', 18), ('tcp_live_unlisted_port', 111),
                            ('tcp_external_allowed_port', 110), ('udp_external_send', 13)]:
            data = self.data()
            next(row for row in data['checks'] if row['name'] == name)['errno'] = error
            row = next(row for row in assess_child(data, 'expected') if row['name'] == name)
            self.assertEqual(row['status'], 'failed')

    def test_failed_positive_and_duplicate_check_never_become_complete(self):
        data = self.data()
        data['checks'][0].update(rc=-1, errno=13)
        self.assertTrue(any(r['status'] == 'failed' for r in assess_child(data, 'expected')))
        data = self.data()
        data['checks'][-1] = data['checks'][0]
        with self.assertRaises(ValueError):
            assess_child(data, 'expected')

    def test_wrong_identity_extra_interface_and_ipv6_support_are_not_denial_passes(self):
        for field, value in [('http_instance_id', 'other'), ('ipv4_interfaces', ['lo', 'eth0']),
                             ('ipv6_socket_errno', 0)]:
            data = self.data(); data[field] = value
            with self.assertRaises(ValueError):
                assess_child(data, 'expected')

    def pair(self):
        ordinary = {'arch': 'aarch64', 'workload': True, 'source_sha256': 'same-source',
                    'keeper_sha256': 'same-keeper', 'busybox_sha256': 'same-busybox',
                    'runtime': {'sha256': 'same-runtime'}, 'initramfs': {'sha256': 'ordinary'}}
        challenge = copy.deepcopy(ordinary)
        challenge.update(cache_boundary_probe_schema=1, initramfs={'sha256': 'challenge'})
        return challenge, ordinary

    def test_challenge_is_explicit_and_ordinary_control_must_differ(self):
        validate_pair(*self.pair())
        challenge, ordinary = self.pair()
        for mutation in ('ordinary-enabled', 'same-image', 'other-keeper', 'nix', 'boolean-schema'):
            a, b = copy.deepcopy(challenge), copy.deepcopy(ordinary)
            if mutation == 'ordinary-enabled': b['cache_boundary_probe_schema'] = 1
            if mutation == 'same-image': b['initramfs'] = a['initramfs']
            if mutation == 'other-keeper': b['keeper_sha256'] = 'different'
            if mutation == 'nix': a['nix'] = b['nix'] = True
            if mutation == 'boolean-schema': a['cache_boundary_probe_schema'] = True
            with self.assertRaises(ValueError): validate_pair(a, b)

    def test_hook_rejects_missing_keeper_or_nonordinary_image_before_writing(self):
        for extra in ({}, {'keeper': 'unused', 'workload': True, 'nix': True},
                      {'keeper': 'unused', 'workload': True, 'pid1_exit_probe': True},
                      {'keeper': 'unused', 'workload': True, 'remote_ssh': True}):
            with tempfile.TemporaryDirectory() as tmp:
                out = Path(tmp) / 'must-not-exist'
                with self.assertRaises(ValueError):
                    build(out, 'aarch64', 'unused-zig', cache_boundary_probe=True, **extra)
                self.assertFalse(out.exists())

    def test_raw_driver_refusal_does_not_qualify_a_backend_bypass(self):
        value = {'schema': 1, 'phase': 'before-mount-and-keeper', 'clear_ro_rc': 0,
                 'inspect_ro_rc': 0, 'guest_ro_after_clear': 1, 'cache_open_write_rc': 0,
                 'cache_write_rc': -1, 'cache_write_errno': 1, 'cache_sector_unchanged': True,
                 'private_write_read_restore': True}
        result = assess_raw_cache(value)
        self.assertEqual(result['status'], 'passed')
        self.assertEqual(result['enforcement_layer'], 'guest-block-readonly')
        self.assertFalse(result['direct_vmm_backend_bypass_qualified'])
        for field, changed in [('cache_open_write_rc', -1), ('private_write_read_restore', False),
                               ('cache_write_rc', 512), ('cache_sector_unchanged', False),
                               ('cache_write_errno', 2)]:
            bad = dict(value); bad[field] = changed
            self.assertEqual(assess_raw_cache(bad)['status'], 'failed')


if __name__ == '__main__':
    unittest.main()
