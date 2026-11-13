#!/usr/bin/env python3
"""Native cooperative shutdown receipts versus zero-status guest panics."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import signal
import time
import traceback

from cold_prepare import require_no_active_collector
from fault_probe import control_reply, validate_pid1_fixture_pair
from smoke import ROOT, artifact, connect, helper_identity, line
from ssh_smoke import require, SOCKETS
from stop_receipt import StopReceipt, owned_identity
from workload_smoke import WorkloadInstance


def persist(path, value):
    with path.open('w') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--adapter', choices=['libkrun', 'hvi'], required=True)
    parser.add_argument('--guest', type=Path, required=True, help='separate enabled PID 1 exit fixture')
    parser.add_argument('--control-guest', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    guest, control = [json.loads(p.read_text()) for p in (args.guest, args.control_guest)]
    validate_pid1_fixture_pair(guest, control)
    require(all(type(g.get('shutdown_receipt_schema')) is int and g['shutdown_receipt_schema'] == 1
                for g in (guest, control)), 'fresh shutdown_receipt_schema=1 guests required')
    require_no_active_collector(ROOT / 'runs')
    lease = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.umask(0o077)
    args.out.mkdir(parents=True, exist_ok=False)
    helper = ROOT / ('hvi/target/debug/devcroft-hvi-spike' if args.adapter == 'hvi' else 'libkrun/target/debug/devcroft-krun-spike')
    kernel, image, seed, observer = [ROOT / name for name in (
        'artifacts/kernel-landlock/Image', 'artifacts/nix/toolchain.img', 'artifacts/nix/data.seed.img',
        'artifacts/measure-macos')]
    report = {'schema': 1, 'status': 'failed', 'platform': 'macos-arm64-native', **helper_identity(helper),
              'qualification': 'experimental cooperative PID 1 sync receipt and owned-child stop classification; not production supervisor, service-stop/unmount, guest attestation or host power-loss qualification',
              'raw_evidence': str(args.out), 'helper': artifact(helper), 'kernel': artifact(kernel),
              'image': artifact(image), 'seed': artifact(seed), 'identity_observer': artifact(observer),
              'guest': guest, 'control_guest': control, 'guest_receipts': [artifact(p) for p in (args.guest, args.control_guest)],
              'cases': [], 'instances': []}
    sources = ['stop_probe.py', 'stop_receipt.py', 'fault_probe.py', 'smoke.py', 'ssh_smoke.py',
               'ssh_transport.py', 'workload_smoke.py', 'cold_prepare.py', 'guest/init.c', 'measure_macos.c']
    for name in sources:
        target = args.out / 'sources' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes((ROOT / name).read_bytes())
    instances, query_number = [], 0

    def save():
        persist(args.out / 'report.json', report)

    def identity(vm):
        nonlocal query_number
        query_number += 1
        return owned_identity(vm.process, observer, args.out / f'identity-{query_number:02}.jsonl')

    def receipt(vm):
        live = identity(vm)
        require(live == vm.owned_identity, 'helper incarnation changed')
        return StopReceipt(live, vm.boots[-1]['boot'])

    def intent(vm, name):
        value = receipt(vm)
        packet = value.request(vm.owned_identity)
        persist(args.out / f'{name}-intent.json', value.record())
        return value, packet

    def case(name, callback):
        row = {'name': name, 'status': 'failed'}
        report['cases'].append(row)
        try:
            row['evidence'] = callback()
            row['status'] = 'passed'
        except Exception as error:
            row.update(error=f'{type(error).__name__}: {error}', traceback=traceback.format_exc())
            raise
        finally:
            save()
        print(json.dumps(row), flush=True)

    try:
        for name, fixture in [('a', guest), ('sibling', control)]:
            vm = WorkloadInstance(name, args.out, helper, kernel, fixture, image, seed)
            instances.append(vm)
            report['instances'].append({'label': name, 'private_dir': str(vm.path), 'boots': vm.boots})
            vm.prepare(validation_timeout=90)
            vm.start(); vm.ready(timeout=120)
            vm.owned_identity = identity(vm)
        a, sibling = instances
        builds, redis_ids = {}, {}
        for vm in instances:
            builds[vm.label] = vm.build()
            redis_ids[vm.label] = vm.start_redis()
            vm.write_acknowledged(); vm.verify_values()
        require(control_reply(sibling, b'TEST PID1 EXIT 86') == b'ERR command\n',
                'ordinary sibling unexpectedly enabled the exit hook')
        require(control_reply(sibling, b'TEST PID1 ARM STOP') == b'ERR command\n',
                'ordinary sibling unexpectedly enabled the shutdown exit hook')

        def sibling_ok():
            sibling.verify_values('sibling-stop-check')
            current = sibling.execute('sibling-identity', "redis-cli --raw INFO server | grep '^run_id:'").stdout.decode().strip().split(':')[1]
            require(current == redis_ids['sibling'], 'sibling Redis restarted')
            require(identity(sibling) == sibling.owned_identity, 'sibling helper identity changed')
            return {'sibling_helper_and_redis_identity_unchanged': True, 'sibling_acknowledged_keys': 32}

        def recover():
            a.start(); a.ready(timeout=120)
            a.owned_identity = identity(a)
            a.verify_build(builds['a'])
            a.start_redis(); a.verify_values('after-stop-recovery')

        def panic(name, with_intent):
            if with_intent:
                value, packet = intent(a, name)
                require(control_reply(a, b'TEST PID1 ARM STOP') == b'ARMED stop-exit\n',
                        'shutdown exit injection did not arm')
            else:
                value, packet = receipt(a), b'TEST PID1 EXIT 86\n'
            a.boots[-1]['forced_stop'] = {'expected': True, 'guest_pid': 1, 'guest_exit_code': 86}
            child = a.process
            sock, _ = connect(a.path / 'control.sock', child, time.monotonic() + 5, a.overflow)
            with sock:
                sock.sendall(packet)
                if with_intent:
                    try:
                        unexpected = line(sock, deadline=time.monotonic() + 10)
                    except (ValueError, ConnectionResetError) as error:
                        require(isinstance(error, ConnectionResetError) or 'closed before' in str(error),
                                'unexpected failure while awaiting the stop acknowledgement')
                    else:
                        raise RuntimeError(f'pre-ack panic returned a control frame: {unexpected!r}')
            code = child.wait(timeout=15)
            outcome = value.classify(a.owned_identity, code)
            a.stop()
            console = bytes(a.stdout + a.stderr)
            require(b'Kernel panic - not syncing: Attempted to kill init!' in console
                    and b'exitcode=0x00005600' in console, 'actual init-death panic missing')
            if with_intent:
                require(b'guest: test-only PID1 exit=86 during shutdown' in console,
                        'valid stop request did not reach the pre-ack panic point')
            require(outcome == 'crash' and not value.acknowledged, 'panic was misclassified as orderly')
            witness = sibling_ok()
            recover()
            return {**value.record(), 'helper_exit_code': code, 'classification': outcome,
                    'kernel_panic_observed': True, 'validated_stop_request_reached_pre_ack_panic': with_intent,
                    'existing_build_and_32_keys_recovered': True, **witness}

        case('panic-without-stop-intent', lambda: panic('no-intent', False))
        case('panic-during-valid-shutdown-before-ack', lambda: panic('pending-intent', True))

        def orderly(name):
            a.execute(name + '-redis-stop', 'redis-cli SHUTDOWN')
            value, packet = intent(a, name)
            child = a.process
            sock, greeting = connect(a.path / 'control.sock', child, time.monotonic() + 5, a.overflow)
            require(greeting.decode().strip() == a.boots[-1]['greeting'], 'stop connected to a stale boot')
            with sock:
                sock.sendall(packet)
                ack = line(sock, deadline=time.monotonic() + 10)
            require(value.accept_ack(a.owned_identity, ack), 'sync acknowledgement did not match intent')
            persist(args.out / f'{name}-ack.json', {**value.record(), 'wire_ack': ack.decode()})
            code = child.wait(timeout=15)
            outcome = value.classify(a.owned_identity, code)
            a.stop()
            require(outcome == 'orderly', 'correlated shutdown was not orderly')
            require(f'guest: shutdown synced state={value.generation}'.encode() in a.stdout + a.stderr,
                    'post-sync guest marker missing')
            return {**value.record(), 'helper_exit_code': code, 'classification': outcome,
                    'wire_ack': ack.decode(), 'post_sync_marker_observed': True, **sibling_ok()}, ack

        first_ack = []

        def first_orderly():
            evidence, ack = orderly('first-orderly')
            first_ack.append(ack)
            recover()
            return {**evidence, 'existing_build_and_32_keys_recovered': True}
        case('correlated-sync-and-clean-exit', first_orderly)

        def stale_controls():
            value, _ = intent(a, 'stale-controls')
            require(not value.accept_ack(a.owned_identity, first_ack[0]), 'old boot ack was accepted')
            requests = [f'SHUTDOWN {value.generation - 1} {value.nonce}',
                        f'SHUTDOWN {value.generation} ' + 'g' * 64,
                        f'SHUTDOWN {value.generation} {value.nonce} extra',
                        f'SHUTDOWN {value.generation} {value.nonce[:-1]}']
            for request in requests:
                require(control_reply(a, request.encode()) == b'ERR shutdown\n', 'invalid stop packet was accepted')
            pong = control_reply(a, b'PING still-live')
            require(pong == b'PONG still-live\n' and not value.accept_ack(a.owned_identity, pong),
                    'unrelated control reply was accepted as sync ack')
            require(identity(a) == a.owned_identity, 'rejected stop request changed helper identity')
            return {**value.record(), 'actual_previous_boot_ack_rejected': True,
                    'wrong_generation_and_malformed_packets_refused': len(requests),
                    'unrelated_pong_rejected': True, 'current_helper_remains_live': True,
                    'diagnostic_scope': 'fresh intent prepared; valid shutdown packet deliberately withheld', **sibling_ok()}
        case('stale-ack-generation-and-malformed-controls', stale_controls)

        def forced_stop():
            value, _ = intent(a, 'forced-stop')
            a.boots[-1]['forced_stop'] = {'expected': True, 'signal': 'SIGTERM'}
            child = a.process
            child.terminate()
            code = child.wait(timeout=10)
            outcome = value.classify(a.owned_identity, code, forced=True)
            a.stop()
            require(code == -signal.SIGTERM and outcome == 'forced-stop', 'forced stop classification failed')
            witness = sibling_ok()
            recover()
            return {**value.record(), 'helper_exit_code': code, 'classification': outcome,
                    'existing_build_and_32_keys_recovered': True, **witness}
        case('persisted-intent-and-host-forced-stop', forced_stop)
        case('new-boot-correlated-sync-and-clean-exit', lambda: orderly('final-orderly')[0])
        sibling.clean_shutdown()
        report['status'] = 'passed'
    except Exception as error:
        report.update(error=f'{type(error).__name__}: {error}', traceback=traceback.format_exc())
    finally:
        for vm in instances:
            vm.stop()
        report['cleanup'] = {'helpers_reaped': all(vm.process is None for vm in instances),
                             'endpoints_removed': all(not (vm.path / name).exists() for vm in instances for name in SOCKETS)}
        if not all(report['cleanup'].values()):
            report.update(status='failed', error='owned resources survived cleanup')
        save()
    print(json.dumps({'status': report['status'], 'error': report.get('error')}))
    raise SystemExit(report['status'] != 'passed')


if __name__ == '__main__':
    main()
