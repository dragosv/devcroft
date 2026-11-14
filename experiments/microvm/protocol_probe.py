#!/usr/bin/env python3
"""Native keeper framing/connection isolation probes; no production VM adoption."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import time
import traceback

import keeper_probe as wire
from cold_prepare import require_no_active_collector
from smoke import ROOT, artifact, helper_identity
from ssh_smoke import Instance, SOCKETS, require
from stop_receipt import owned_identity


def frame(tag, payload=b'', length=None):
    if not isinstance(payload, bytes):
        payload = json.dumps(payload).encode()
    return struct.pack('>BI', tag, len(payload) if length is None else length) + payload


def request(**extra):
    return {'cmd': '/bin/sh', 'args': ['-c', 'printf accepted-forged-selector'],
            'cwd': '/workspace', 'env': {}, 'pty': None, **extra}


class Canary:
    def __init__(self, vm):
        self.vm = vm
        self.sock, self.session_id = wire.spawn(vm.path / 'keeper.sock', '/bin/cat')

    def echo(self, data):
        wire.send(self.sock, 4, data)
        output = bytearray()
        while len(output) < len(data):
            tag, payload = wire.receive(self.sock)
            require(tag == 6, ('canary lost its session', tag, payload))
            output.extend(payload)
            require(len(output) <= len(data), 'canary returned unexpected extra bytes')
        require(output == data, 'canary data changed')
        live = wire.query(self.vm.path / 'keeper.sock')['sessions']
        require(any(s['id'] == self.session_id for s in live), 'canary disappeared from registry')

    def close(self):
        wire.send(self.sock, 5)
        try:
            output, error, status = wire.finish(self.sock)
            require((output, error, status) == (b'', b'', {'code': 0, 'signal': None}),
                    ('canary did not close cleanly', output, error, status))
        finally:
            self.sock.close()


def initial_refusal(path, packet, half_close=False):
    began = time.monotonic()
    result = {'request_sha256': hashlib.sha256(packet).hexdigest(), 'request_bytes': len(packet),
              'header_hex': packet[:5].hex(), 'half_close': half_close, 'read_timeout_seconds': 2}
    with wire.connect(path) as sock:
        sock.settimeout(2)
        sock.sendall(packet)
        if half_close:
            sock.shutdown(socket.SHUT_WR)
        try:
            if not sock.recv(1, socket.MSG_PEEK):
                result.update(refused=True, response='eof')
            else:
                tag, payload = wire.receive(sock)
                result.update(refused=tag == 3 and payload.get('message') ==
                              'expected Spawn or Query as the first frame', response_tag=tag)
                if tag == 2:
                    wire.send(sock, 5)
                    out, err, status = wire.finish(sock)
                    result.update(unexpected_spawn_output=out.decode(errors='replace'),
                                  unexpected_spawn_status=status)
        except ConnectionResetError:
            result.update(refused=True, response='reset')
        except socket.timeout:
            result.update(refused=False, response='timeout-awaiting-body-or-response')
    result['seconds'] = time.monotonic() - began
    return result


def session_refusal(path, packet):
    sock, session_id = wire.spawn(path, '/bin/cat')
    result = {'owned_session_id': session_id, 'request_sha256': hashlib.sha256(packet).hexdigest(),
              'header_hex': packet[:5].hex(), 'read_timeout_seconds': 3}
    with sock:
        sock.settimeout(3)
        sock.sendall(packet)
        try:
            out, err, status = wire.finish(sock)
            result.update(refused=not out and not err and status == {'code': None, 'signal': 15},
                          owned_session_exit=status)
        except socket.timeout:
            result.update(refused=False, response='unexpected-frame-ignored-session-remains-live')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--adapter', choices=['libkrun', 'hvi'], required=True)
    parser.add_argument('--guest', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--baseline-only', action='store_true', help='retain failures; omit cold restart')
    args = parser.parse_args()
    require_no_active_collector(ROOT / 'runs')
    lease = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.umask(0o077)
    args.out.mkdir(parents=True, exist_ok=False)
    helper = ROOT / ('libkrun/target/debug/devcroft-krun-spike' if args.adapter == 'libkrun'
                     else 'hvi/target/debug/devcroft-hvi-spike')
    expected_helper = artifact(helper)
    guest = json.loads(args.guest.read_text())
    require(not any(guest.get(key) for key in ('workload', 'pid1_exit_probe', 'remote_ssh_fixture')),
            'ordinary minimal keeper fixture required')
    report = {'schema': 1, 'status': 'failed', 'platform': 'macos-arm64-native', **helper_identity(helper),
              'qualification': 'bounded malformed-message sample and connection-scoped keeper controls; not host parser fuzzing, hostile-guest containment audit or denial-of-service qualification',
              'raw_evidence': str(args.out), 'helper': expected_helper,
              'kernel': artifact(ROOT / 'artifacts/kernel-landlock/Image'), 'guest': guest,
              'guest_receipt': artifact(args.guest), 'cases': [], 'instances': []}
    for name in ['protocol_probe.py', 'keeper_probe.py', 'ssh_smoke.py', 'smoke.py',
                 'ssh_transport.py', 'stop_receipt.py', 'guest/init.c']:
        target = args.out / 'sources' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes((ROOT / name).read_bytes())
    for name in ('protocol.rs', 'connection.rs'):
        target = args.out / 'sources/shared-keeper' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes((ROOT.parent.parent / 'src/keeper' / name).read_bytes())
    instances, canaries, identities = [], [], []

    def save():
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')

    def identity(vm, label):
        return owned_identity(vm.process, ROOT / 'artifacts/measure-macos', args.out / f'{label}-identity.jsonl')

    def witnesses(name):
        for canary in canaries:
            canary.echo(f'{name}:{canary.vm.label}\n'.encode())
        return {'both_existing_canaries_live_and_echo_exact': True,
                'canary_session_ids': [c.session_id for c in canaries]}

    def case(name, callback):
        row = {'name': name, 'status': 'failed'}
        report['cases'].append(row)
        try:
            evidence = callback()
            evidence.update(witnesses(name))
            row.update(status='passed' if evidence['refused'] else 'failed', evidence=evidence)
        finally:
            save()
        print(json.dumps(row), flush=True)

    try:
        for label in ('a', 'sibling'):
            require(artifact(helper) == expected_helper, 'helper changed before launch')
            vm = Instance(label, args.out, helper, ROOT / 'artifacts/kernel-landlock/Image', guest)
            instances.append(vm)
            report['instances'].append({'label': label, 'boots': vm.boots})
            vm.prepare(validation_timeout=30)
            vm.start(); vm.ready(timeout=90)
            identities.append(identity(vm, label + '-first'))
            canaries.append(Canary(vm))
        a, sibling = instances
        canaries[0].echo(bytes(range(256)) * 64)
        canaries[1].echo(bytes(range(256)) * 64)
        report['binary_echo_positive_control_bytes_per_vm'] = 16384
        path = a.path / 'keeper.sock'
        sibling_id = canaries[1].session_id
        checks = [
            ('unknown-tag-without-body', frame(255, length=4096), False),
            ('query-declared-payload-without-body', frame(11, length=16 * 1024 * 1024), False),
            ('query-forged-instance-payload', frame(11, {'instance_id': sibling.id}), False),
            ('spawn-forged-instance-selector', frame(1, request(instance_id=sibling.id)), False),
            ('spawn-forged-session-selector', frame(1, request(session_id=sibling_id)), False),
            ('spawn-forged-boot-selector', frame(1, request(boot_generation=0)), False),
            ('pty-forged-session-selector', frame(1, request(pty={'rows': 24, 'cols': 80,
                                                               'session_id': sibling_id})), False),
            ('oversized-header', frame(1, length=16 * 1024 * 1024 + 1), False),
            ('maximum-u32-length', frame(4, length=0xffffffff), False),
            ('invalid-json', frame(1, b'{'), False),
            ('invalid-utf8-json', frame(1, b'\xff'), False),
            ('duplicate-command-field', frame(1, b'{"cmd":"/bin/sh","cmd":"/bin/cat","args":[],"cwd":"/workspace","env":{},"pty":null}'), False),
            ('wrong-argument-type', frame(1, request(args='not-an-array')), False),
            ('truncated-header', b'\x01\x00', True),
            ('truncated-body', frame(1, b'{', length=8), True),
            ('stdin-before-spawn', frame(4, b'unexpected'), False),
            ('server-spawn-ok-before-spawn', frame(2, {'session_id': sibling_id}), False),
            ('forged-cross-session-signal', frame(9, {'session_id': sibling_id, 'signal': 'Term'}), False),
        ]
        for name, packet, half_close in checks:
            case(name, lambda packet=packet, half_close=half_close: initial_refusal(path, packet, half_close))
        for name, packet in [
            ('second-spawn-on-live-session', frame(1, request())),
            ('server-response-on-live-session', frame(2, {'session_id': sibling_id})),
            ('query-on-live-session', frame(11)),
            ('nonempty-stdin-close-on-live-session', frame(5, b'forged')),
        ]:
            case(name, lambda packet=packet: session_refusal(path, packet))

        signal_sock, _ = wire.spawn(path, '/bin/sleep', ['30'])
        with signal_sock:
            wire.send(signal_sock, 9, 'Term')
            out, err, status = wire.finish(signal_sock)
            require((out, err, status) == (b'', b'', {'code': None, 'signal': 15}),
                    'valid connection-owned signal failed')
        report['valid_signal_positive_control'] = witnesses('valid-signal')
        require(identity(sibling, 'sibling-after-inputs') == identities[1], 'sibling helper changed')
        require(identity(a, 'a-after-inputs') == identities[0], 'helper changed during malformed inputs')

        if not args.baseline_only:
            canaries[0].close()
            a.shutdown()
            require(artifact(helper) == expected_helper, 'helper changed before cold restart')
            a.start(); a.ready(timeout=90)
            report['cold_restart_new_helper_identity'] = identity(a, 'a-after-restart') != identities[0]
            require(report['cold_restart_new_helper_identity'], 'cold restart reused the old helper identity')
            canaries[0] = Canary(a)
            for name, packet in [('after-restart-forged-instance', checks[3][1]),
                                 ('after-restart-query-payload', checks[2][1])]:
                case(name, lambda packet=packet: initial_refusal(path, packet))
            require(identity(sibling, 'sibling-after-restart') == identities[1], 'sibling restarted')
        require(artifact(helper) == expected_helper, 'helper changed during run')
        for canary in canaries:
            canary.close()
        canaries.clear()
        for vm in instances:
            vm.shutdown()
        report['status'] = 'passed' if all(c['status'] == 'passed' for c in report['cases']) else 'failed'
    except Exception as error:
        report.update(error=f'{type(error).__name__}: {error}', traceback=traceback.format_exc())
    finally:
        for canary in canaries:
            canary.sock.close()
        for vm in instances:
            vm.stop()
        report['cleanup'] = {'helpers_reaped': all(vm.process is None for vm in instances),
                             'endpoints_removed': all(not (vm.path / name).exists()
                                                      for vm in instances for name in SOCKETS)}
        if not all(report['cleanup'].values()):
            report.update(status='failed', error='owned helper or endpoint survived cleanup')
        else:
            report['discarded_private_resources'] = []
            for vm in instances:
                for directory in (vm.path, vm.controller_dir):
                    shutil.rmtree(directory)
                    report['discarded_private_resources'].append(str(directory))
            report['cleanup']['private_resources_removed'] = True
        save()
    print(json.dumps({'status': report['status'], 'error': report.get('error')}), flush=True)
    raise SystemExit(report['status'] != 'passed')


if __name__ == '__main__':
    main()
