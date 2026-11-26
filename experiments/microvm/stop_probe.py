#!/usr/bin/env python3
"""Native cooperative shutdown receipts versus zero-status guest panics."""
import argparse
import fcntl
import hashlib
import json
import re
import os
from pathlib import Path
import signal
import shutil
import time
import traceback

from cold_prepare import require_no_active_collector
from fault_probe import control_reply, validate_pid1_fixture_pair
from smoke import ROOT, artifact, connect, helper_identity, line
from ssh_smoke import require, SOCKETS
from stop_receipt import StopReceipt, ext4_superblock_clean, owned_identity
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
    parser.add_argument('--helper', type=Path, help='explicit immutable helper copy for a baseline')
    parser.add_argument('--guest', type=Path, required=True, help='separate enabled PID 1 exit fixture')
    parser.add_argument('--control-guest', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--discard-private', action='store_true',
                        help='remove only this run\'s private disks/controller directories after owned cleanup')
    parser.add_argument('--post-ack-only', action='store_true',
                        help='baseline diagnostic only: omit the six earlier stop cases')
    parser.add_argument('--quiescence-only', action='store_true',
                        help='baseline diagnostic only: run only the live-service/stubborn-writer unmount stop case')
    args = parser.parse_args()
    require(not (args.post_ack_only and args.quiescence_only),
            '--post-ack-only and --quiescence-only are mutually exclusive')
    guest, control = [json.loads(p.read_text()) for p in (args.guest, args.control_guest)]
    validate_pid1_fixture_pair(guest, control)
    require(type(guest.get('pid1_exit_probe_schema')) is int and guest['pid1_exit_probe_schema'] == 2,
            'fresh post-ack PID 1 fault fixture required')
    require(all(type(g.get('shutdown_receipt_schema')) is int and g['shutdown_receipt_schema'] == 1
                for g in (guest, control)), 'fresh shutdown_receipt_schema=1 guests required')
    require_no_active_collector(ROOT / 'runs')
    lease = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.umask(0o077)
    args.out.mkdir(parents=True, exist_ok=False)
    helper = args.helper.resolve() if args.helper else ROOT / (
        'hvi/target/debug/devcroft-hvi-spike' if args.adapter == 'hvi' else 'libkrun/target/debug/devcroft-krun-spike')
    expected_helper = artifact(helper)
    helper_info = helper_identity(helper)
    require(helper_info['adapter'] == args.adapter, 'helper adapter mismatch')
    provenance = helper_info['helper_provenance']
    # hvi's reviewed wrapper succeeds only for Stop::SystemOff. libkrun's
    # original HVF backend merged OFF and RESET; demand the reviewed seam.
    if args.adapter == 'hvi':
        reference = ROOT / 'fixtures/upstream.json'
        pinned = json.loads(reference.read_text())['hvi']
        expected_patches = [
            {'file': Path(item['path']).name, 'sha256': item['sha256']}
            for item in pinned['patches']
        ]
        poweroff_qualified = (
            provenance.get('adapter') == 'hvi'
            and provenance.get('revision') == pinned['revision']
            and provenance.get('patches') == expected_patches
            and all(
                hashlib.sha256((ROOT / item['path']).read_bytes()).hexdigest() == item['sha256']
                for item in pinned['patches']
            )
        )
    else:
        reference = ROOT / 'patches/libkrun-macos-reset-exit.patch'
        poweroff_qualified = (
            provenance.get('macos_stop_semantics') == 'psci-system-off=0,system-reset=1'
            and provenance.get('macos_reset_exit_patch_sha256') == hashlib.sha256(reference.read_bytes()).hexdigest())
    kernel, image, seed, observer = [ROOT / name for name in (
        'artifacts/kernel-landlock/Image', 'artifacts/nix/toolchain.img', 'artifacts/nix/data.seed.img',
        'artifacts/measure-macos')]
    report = {'schema': 1, 'status': 'failed', 'platform': 'macos-arm64-native', **helper_info,
              'poweroff_exit_qualified': poweroff_qualified,
              'exit_qualification_reference': artifact(reference),
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
        return StopReceipt(live, vm.boots[-1]['boot'], poweroff_exit_qualified=poweroff_qualified)

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
            require(artifact(helper) == expected_helper, 'helper binary changed during the run')
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
        require(control_reply(sibling, b'TEST PID1 ARM POSTACK') == b'ERR command\n',
                'ordinary sibling unexpectedly enabled the post-ack exit hook')

        def sibling_ok():
            sibling.verify_values('sibling-stop-check')
            current = sibling.execute('sibling-identity', "redis-cli --raw INFO server | grep '^run_id:'").stdout.decode().strip().split(':')[1]
            require(current == redis_ids['sibling'], 'sibling Redis restarted')
            require(identity(sibling) == sibling.owned_identity, 'sibling helper identity changed')
            return {'sibling_helper_and_redis_identity_unchanged': True, 'sibling_acknowledged_keys': 32}

        def recover():
            require(artifact(helper) == expected_helper, 'helper binary changed during the run')
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
            sb = ext4_superblock_clean(a.path / 'data.img')
            require(not sb['cleanly_unmounted'] and sb['incompat_recover'],
                    ('panic unexpectedly left ext4 superblock marked clean', sb))
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
                    'host_ext4_superblock_after_stop': sb,
                    'existing_build_and_32_keys_recovered': True, **witness}

        if not args.post_ack_only and not args.quiescence_only:
            case('panic-without-stop-intent', lambda: panic('no-intent', False))
            case('panic-during-valid-shutdown-before-ack', lambda: panic('pending-intent', True))

        def orderly(name, live_quiescence=False, recover_after=False):
            stubborn_pid = None
            if live_quiescence:
                holder = a.execute(name + '-live-holders', '''set -eu
test "$(redis-cli --raw PING)" = PONG
sh -c 'trap "" TERM; exec 3>/workspace/unmount-canary; printf "holder-pid=%s\\n" "$$" >&3; exec sleep 60' </dev/null >/dev/null 2>&1 &
holder=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do
  if [ -s /workspace/unmount-canary ] && [ -d "/proc/$holder" ]; then break; fi
  sleep 0.05
done
test -s /workspace/unmount-canary && test -d "/proc/$holder"
printf '%s\\n' "$holder"
''').stdout.decode().strip()
                require(holder.isdigit() and int(holder) > 1, ('invalid stubborn writer PID', holder))
                stubborn_pid = int(holder)
            else:
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
            a.stop()
            sb = ext4_superblock_clean(a.path / 'data.img')
            console = bytes(a.stdout + a.stderr)
            post_sync_marker = f'guest: shutdown synced state={value.generation}'.encode() in console
            # A whole line; the libkrun console tty turns LF into CRLF.
            quiesced_marker = re.search(rb'^guest: shutdown quiesced=1 umount=1\r?$', console, re.M) is not None
            outcome = value.classify(a.owned_identity, code,
                                     workspace_unmounted=sb['cleanly_unmounted'] and quiesced_marker)
            witness = sibling_ok()
            if recover_after:
                recover()
            evidence = {**value.record(), 'helper_exit_code': code, 'classification': outcome,
                        'prior_v8_predicate_classification': 'orderly' if code == 0 and poweroff_qualified else 'crash',
                        'wire_ack': ack.decode(), 'post_sync_marker_observed': post_sync_marker,
                        'quiesced_and_unmounted_marker_observed': quiesced_marker,
                        'live_redis_and_stubborn_writer_held_workspace': live_quiescence,
                        'stubborn_writer_pid': stubborn_pid,
                        'host_ext4_superblock_after_stop': sb,
                        'existing_build_and_32_keys_recovered': recover_after,
                        **witness}
            if live_quiescence:
                report['quiescence_observation'] = evidence
            require(post_sync_marker, 'post-sync guest marker missing')
            require(outcome == 'orderly' and sb['cleanly_unmounted'] and quiesced_marker,
                    f'correlated shutdown did not cleanly quiesce and unmount workspace: {outcome}, superblock={sb}, marker={quiesced_marker}')
            return evidence, ack

        first_ack = []

        def first_orderly():
            evidence, ack = orderly('first-orderly', live_quiescence=False, recover_after=True)
            first_ack.append(ack)
            return evidence
        if not args.post_ack_only and not args.quiescence_only:
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
        if not args.post_ack_only and not args.quiescence_only:
            case('stale-ack-generation-and-malformed-controls', stale_controls)

        def forced_stop():
            value, _ = intent(a, 'forced-stop')
            a.boots[-1]['forced_stop'] = {'expected': True, 'signal': 'SIGTERM'}
            child = a.process
            child.terminate()
            code = child.wait(timeout=10)
            outcome = value.classify(a.owned_identity, code, forced=True)
            a.stop()
            sb = ext4_superblock_clean(a.path / 'data.img')
            require(not sb['cleanly_unmounted'] and sb['incompat_recover'],
                    ('forced stop unexpectedly left ext4 superblock marked clean', sb))
            require(code == -signal.SIGTERM and outcome == 'forced-stop', 'forced stop classification failed')
            witness = sibling_ok()
            recover()
            return {**value.record(), 'helper_exit_code': code, 'classification': outcome,
                    'host_ext4_superblock_after_stop': sb,
                    'existing_build_and_32_keys_recovered': True, **witness}
        if not args.post_ack_only and not args.quiescence_only:
            case('persisted-intent-and-host-forced-stop', forced_stop)
        if not args.post_ack_only:
            case('new-boot-correlated-sync-and-clean-exit',
                 lambda: orderly('final-orderly', live_quiescence=True, recover_after=True)[0])

        def post_ack_panic():
            value, packet = intent(a, 'post-ack-panic')
            require(control_reply(a, b'TEST PID1 ARM POSTACK') == b'ARMED postack-exit\n',
                    'post-ack exit injection did not arm')
            a.boots[-1]['forced_stop'] = {'expected': True, 'guest_pid': 1, 'guest_exit_code': 86}
            child = a.process
            sock, greeting = connect(a.path / 'control.sock', child, time.monotonic() + 5, a.overflow)
            require(greeting.decode().strip() == a.boots[-1]['greeting'], 'post-ack probe reached a stale boot')
            with sock:
                sock.sendall(packet)
                ack = line(sock, deadline=time.monotonic() + 10)
            require(value.accept_ack(a.owned_identity, ack), 'post-ack panic lacked a real matching acknowledgement')
            persist(args.out / 'post-ack-panic-ack.json', {**value.record(), 'wire_ack': ack.decode()})
            code = child.wait(timeout=15)
            outcome = value.classify(a.owned_identity, code)
            a.stop()
            sb = ext4_superblock_clean(a.path / 'data.img')
            console = bytes(a.stdout + a.stderr)
            require(b'guest: test-only PID1 exit=86 after shutdown ack' in console
                    and b'Kernel panic - not syncing: Attempted to kill init!' in console
                    and b'exitcode=0x00005600' in console, 'actual post-ack init-death panic missing')
            witness = sibling_ok()
            recover()
            evidence = {**value.record(), 'wire_ack': ack.decode(), 'helper_exit_code': code,
                        'classification': outcome, 'kernel_panic_observed': True,
                        'prior_v6_predicate_classification': 'orderly' if code == 0 else 'crash',
                        'post_ack_delivery_window_ms': 250,
                        'host_ext4_superblock_after_stop': sb,
                        'existing_build_and_32_keys_recovered': True, **witness}
            # Retain the complete witness even when the old helper falsely
            # classifies the panic as orderly. Never turn that baseline green.
            report['post_ack_observation'] = evidence
            require(outcome == 'crash', f'post-ack panic was misclassified: {outcome}, helper exit {code}')
            return evidence

        if not args.quiescence_only:
            case('panic-after-correlated-sync-ack', post_ack_panic)
        require(artifact(helper) == expected_helper, 'helper binary changed during the run')
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
        elif args.discard_private:
            report['discarded_private_resources'] = []
            for vm in instances:
                for path in (vm.path, vm.controller_dir):
                    shutil.rmtree(path)
                    report['discarded_private_resources'].append(str(path))
            report['cleanup']['private_resources_removed'] = all(
                not Path(path).exists() for path in report['discarded_private_resources'])
        save()
    print(json.dumps({'status': report['status'], 'error': report.get('error')}))
    raise SystemExit(report['status'] != 'passed')


if __name__ == '__main__':
    main()
