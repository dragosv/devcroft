#!/usr/bin/env python3
"""Native keeper framing/connection isolation probes; no production VM adoption."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import select
import socket
import struct
import time
import traceback

import keeper_probe as wire
from cold_prepare import require_no_active_collector
from fault_probe import control_reply
from smoke import ROOT, artifact, connect, helper_identity, line
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


def send_unless_peer_closed(sock, data):
    """Send unless the peer has already answered or closed; return why not.

    On macOS a send to a Unix socket whose peer shut down its read side blocks
    instead of failing with EPIPE (measured), so a refused connection must be
    observed by reading, never by a send error.
    """
    if select.select([sock], [], [], 0)[0]:
        return 'peer-readable-before-send'
    sock.settimeout(0.3)
    try:
        sock.sendall(data)
    except socket.timeout:
        return 'send-blocked-peer-read-side-shut'
    except (BrokenPipeError, ConnectionResetError) as error:
        return type(error).__name__
    return None


def trickle_initial_refusal(path):
    header = frame(1, b'', length=64)
    chunks = [b'{', b'"', b'c', b'"', b':']
    began = time.monotonic()
    result = {'header_hex': header.hex(), 'dribble_bytes': len(chunks),
              'dribble_interval_seconds': 0.22, 'read_timeout_seconds': 1.2}
    with wire.connect(path) as sock:
        sock.settimeout(1.2)
        sent = 0
        try:
            sock.sendall(header)
            for byte in chunks:
                time.sleep(0.22)
                if send_unless_peer_closed(sock, byte) is not None:
                    break
                sent += 1
            # The keeper's deadline is 1 s from the first byte. How quickly the host
            # client then sees the close is the transport's behaviour, recorded apart.
            sock.settimeout(8)
            if not sock.recv(1, socket.MSG_PEEK):
                result.update(refused=True, response='eof')
            else:
                tag, payload = wire.receive(sock)
                result.update(refused=tag == 3, response_tag=tag, response_payload=payload)
        except (BrokenPipeError, ConnectionResetError) as error:
            result.update(refused=True, response=type(error).__name__)
        except socket.timeout:
            result.update(refused=False, response='no-close-within-8s-after-dribble')
        result['dribble_bytes_sent_before_outcome'] = sent
    result['seconds'] = time.monotonic() - began
    result['host_close_seconds_after_keeper_deadline'] = result['seconds'] - 1.0
    return result


def connect_paced(path, timeout=1.0):
    """libkrun's host Unix acceptor uses Backlog::new(5) and accepts one socket per poll tick."""
    deadline = time.monotonic() + timeout
    while True:
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(8)
        try:
            sock.connect(str(path))
            time.sleep(0.01)
            return sock
        except ConnectionRefusedError:
            sock.close()
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.01)


def stalled_flood_admission(path, count=20):
    began = time.monotonic()
    stalled = []
    outcomes = {}
    try:
        for index in range(count):
            sock = connect_paced(path)
            stalled.append((index, sock))
            send_unless_peer_closed(sock, b'\x01\x00')
            sock.settimeout(0.05)
        deadline = time.monotonic() + 8.0
        while len(outcomes) < count and time.monotonic() < deadline:
            for index, sock in stalled:
                if index in outcomes:
                    continue
                try:
                    peek = sock.recv(1, socket.MSG_PEEK)
                    if not peek:
                        outcomes[index] = 'eof'
                    else:
                        sock.settimeout(0.2)
                        tag, payload = wire.receive(sock)
                        if tag == 3 and payload.get('message') == 'keeper connection limit exceeded':
                            outcomes[index] = 'admission-limit-spawn-err'
                        else:
                            outcomes[index] = f'unexpected-tag-{tag}'
                except ConnectionResetError:
                    outcomes[index] = 'reset'
                except socket.timeout:
                    continue
            if len(outcomes) < count:
                time.sleep(0.05)
        immediate_refusals = sum(1 for v in outcomes.values() if v == 'admission-limit-spawn-err')
        deadline_evictions = sum(1 for v in outcomes.values() if v in ('eof', 'reset'))
        evicted_all = len(outcomes) == count and (immediate_refusals + deadline_evictions) == count
        recovered_query = None
        recovered_exec = None
        recovered_exec_observed = None
        if evicted_all:
            # All 20 attacker client descriptors remain open here: prove that
            # server-side eviction freed the admission slots for fresh work.
            recovered_query = len(wire.query(path)['sessions'])
            out, err, status = wire.execute(path, 'printf admission-recovered')
            recovered_exec = (out == b'admission-recovered' and not err
                              and status == {'code': 0, 'signal': None})
            recovered_exec_observed = {'stdout': out.decode(errors='replace'),
                                       'stderr': err.decode(errors='replace'), 'status': status}
        return {
            'opened_stalled_connections': count,
            'outcomes': {str(k): v for k, v in sorted(outcomes.items())},
            'outcome_seconds': time.monotonic() - began,
            'evicted_or_refused_by_server_while_client_held_open': len(outcomes),
            'immediate_admission_refusals': immediate_refusals,
            'deadline_eof_evictions': deadline_evictions,
            'recovered_query_active_sessions_while_stalled_fds_open': recovered_query,
            'recovered_spawn_while_stalled_fds_open': recovered_exec,
            'recovered_spawn_observed': recovered_exec_observed,
            'refused': bool(evicted_all and recovered_exec),
            'seconds': time.monotonic() - began,
        }
    finally:
        for _, sock in stalled:
            sock.close()


def control_partial_line_refusal(vm):
    began = time.monotonic()
    sock, greeting = connect(vm.path / 'control.sock', vm.process, time.monotonic() + 5, vm.overflow)
    reply = None
    followup = None
    with sock:
        sock.sendall(b'PING incomplete-without-newline')
        try:
            reply = line(sock, deadline=time.monotonic() + 2.0)
        except TimeoutError:
            reply = None
    if reply == b'ERR timeout\n':
        followup = control_reply(vm, b'PING after-partial-timeout')
    return {
        'greeting': greeting.decode().strip(),
        'reply': reply.decode(errors='replace') if reply else 'timeout-awaiting-newline',
        'followup_pong': followup == b'PONG after-partial-timeout\n',
        'refused': reply == b'ERR timeout\n' and followup == b'PONG after-partial-timeout\n',
        'seconds': time.monotonic() - began,
    }


def control_trickle_and_hold_refusal(vm):
    began = time.monotonic()
    sock, _ = connect(vm.path / 'control.sock', vm.process, time.monotonic() + 5, vm.overflow)
    trickle_reply = None
    with sock:
        try:
            for byte in (b'P', b'I', b'N', b'G', b' '):
                time.sleep(0.22)
                sock.sendall(byte)
        except (BrokenPipeError, ConnectionResetError):
            pass
        try:
            trickle_reply = line(sock, deadline=time.monotonic() + 1.2)
        except (TimeoutError, BrokenPipeError, ConnectionResetError):
            trickle_reply = None
    holder, _ = connect(vm.path / 'control.sock', vm.process, time.monotonic() + 5, vm.overflow)
    second = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    second_pong = None
    try:
        holder.sendall(b'PING hold-first\n')
        first_pong = line(holder, deadline=time.monotonic() + 2.0)
        require(first_pong == b'PONG hold-first\n', ('unexpected first control reply', first_pong))
        try:
            second.settimeout(1.8)
            second.connect(str(vm.path / 'control.sock'))
            greeting = line(second, deadline=time.monotonic() + 1.8)
            if greeting.startswith(b'READY 1 '):
                second.sendall(b'PING hold-second\n')
                second_pong = line(second, deadline=time.monotonic() + 1.8)
        except (OSError, ValueError):
            second_pong = None
    finally:
        second.close()
        holder.close()
    return {
        'trickle_reply': trickle_reply.decode(errors='replace') if trickle_reply else 'timeout-or-reset',
        'second_client_pong_while_first_held_open': second_pong == b'PONG hold-second\n',
        'refused': trickle_reply == b'ERR timeout\n' and second_pong == b'PONG hold-second\n',
        'seconds': time.monotonic() - began,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--adapter', choices=['libkrun', 'hvi'], required=True)
    parser.add_argument('--guest', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--baseline-only', action='store_true', help='retain failures; omit cold restart')
    parser.add_argument('--admission-only', action='store_true',
                        help='baseline diagnostic only: run only the 12 deadline/allocation/admission/control-stall cases')
    args = parser.parse_args()
    require(not (args.baseline_only and args.admission_only),
            '--baseline-only and --admission-only are mutually exclusive')
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
    for name in ('protocol.rs', 'connection.rs', 'mod.rs'):
        target = args.out / 'sources/shared-keeper' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes((ROOT.parent.parent / 'src/keeper' / name).read_bytes())
    target = args.out / 'sources/guest-keeper-linux.rs'
    target.write_bytes((ROOT / 'guest-keeper/src/linux.rs').read_bytes())
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
        if not args.admission_only:
            for name, packet, half_close in checks:
                case(name, lambda packet=packet, half_close=half_close: initial_refusal(path, packet, half_close))
            for name, packet in [
                ('second-spawn-on-live-session', frame(1, request())),
                ('server-response-on-live-session', frame(2, {'session_id': sibling_id})),
                ('query-on-live-session', frame(11)),
                ('nonempty-stdin-close-on-live-session', frame(5, b'forged')),
            ]:
                case(name, lambda packet=packet: session_refusal(path, packet))

        for name, packet in [
            ('spawn-oversized-control-header', frame(1, length=4 * 1024 * 1024 + 1)),
            ('resize-oversized-control-header', frame(8, length=64 * 1024 + 1)),
            ('signal-oversized-control-header', frame(9, length=64 * 1024 + 1)),
            ('idle-connection-without-frame', b''),
            ('truncated-header-without-eof', b'\x01\x00'),
            ('truncated-body-without-eof', frame(1, b'{', length=64)),
        ]:
            case(name, lambda packet=packet: initial_refusal(path, packet, False))
        case('trickle-spawn-slowloris', lambda: trickle_initial_refusal(path))
        for name, packet in [
            ('mid-session-truncated-header-without-eof', b'\x04\x00'),
            ('mid-session-incomplete-large-stdin-without-eof', frame(4, b'partial', length=16 * 1024 * 1024)),
        ]:
            case(name, lambda packet=packet: session_refusal(path, packet))
        case('stalled-connection-flood-and-admission-recovery', lambda: stalled_flood_admission(path))
        case('pid1-control-partial-line-without-eof', lambda: control_partial_line_refusal(a))
        case('pid1-control-trickle-and-post-reply-hold', lambda: control_trickle_and_hold_refusal(a))

        signal_sock, _ = wire.spawn(path, '/bin/sleep', ['30'])
        with signal_sock:
            wire.send(signal_sock, 9, 'Term')
            out, err, status = wire.finish(signal_sock)
            require((out, err, status) == (b'', b'', {'code': None, 'signal': 15}),
                    'valid connection-owned signal failed')
        report['valid_signal_positive_control'] = witnesses('valid-signal')
        require(identity(sibling, 'sibling-after-inputs') == identities[1], 'sibling helper changed')
        require(identity(a, 'a-after-inputs') == identities[0], 'helper changed during malformed inputs')

        if not args.baseline_only and not args.admission_only:
            canaries[0].close()
            a.shutdown()
            require(artifact(helper) == expected_helper, 'helper changed before cold restart')
            a.start(); a.ready(timeout=90)
            report['cold_restart_new_helper_identity'] = identity(a, 'a-after-restart') != identities[0]
            require(report['cold_restart_new_helper_identity'], 'cold restart reused the old helper identity')
            canaries[0] = Canary(a)
            for name, packet in [('after-restart-forged-instance', checks[3][1]),
                                 ('after-restart-query-payload', checks[2][1]),
                                 ('after-restart-spawn-oversized-control-header', frame(1, length=4 * 1024 * 1024 + 1)),
                                 ('after-restart-truncated-header-without-eof', b'\x01\x00')]:
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
