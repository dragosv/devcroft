#!/usr/bin/env python3
"""Summarize frozen density evidence without turning missing rows into passes."""
import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path
import statistics

ROOT = Path(__file__).resolve().parent
MIB, GIB = 1024**2, 1024**3


def number(value):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
        raise ValueError(f'invalid nonnegative measurement: {value!r}')
    return value


def distribution(values):
    values = [number(value) for value in values]
    return {'n': len(values), 'minimum': min(values), 'median': statistics.median(values),
            'maximum': max(values)} if values else {'n': 0}


def case_id(sample):
    return tuple(sample[key] for key in ('fixture', 'adapter', 'instances_requested', 'kind', 'repetition'))


def limit_status(values, maximum, expected):
    values = [number(value) for value in values]
    if any(value > maximum for value in values):
        return 'failed'
    return 'passed' if len(values) == expected else 'not-run'


def summarize(report, thresholds):
    protocol = report['protocol']
    calibration = 'calibration' in report['qualification'].lower()
    expected = {(fixture, adapter, count, kind, repetition)
                for fixture in protocol['fixtures'] for adapter in protocol['adapters']
                for count in protocol['instances']
                for kind, total in [('launch', protocol['launch_stop_cycles']),
                                    ('build-idle', protocol['full_build_idle_repetitions'])]
                for repetition in range(total)}
    samples = report['samples']
    identities = [case_id(sample) for sample in samples]
    duplicates = [identity for identity, count in Counter(identities).items() if count != 1]
    if duplicates or set(identities) - expected:
        raise ValueError('duplicate or unexpected sample identities')
    missing = expected - set(identities)
    limits = thresholds['original_no_go_thresholds']
    rows = []
    for fixture in protocol['fixtures']:
        for count in protocol['instances']:
            for adapter in protocol['adapters']:
                selected = [s for s in samples if (s['fixture'], s['instances_requested'], s['adapter']) == (fixture, count, adapter)]
                passed = [s for s in selected if s['status'] == 'passed']
                launches = [s for s in passed if s['kind'] == 'launch']
                builds = [s for s in passed if s['kind'] == 'build-idle']
                failures = [s for s in selected if s['status'] != 'passed']
                expected_launches, expected_builds = protocol['launch_stop_cycles'], protocol['full_build_idle_repetitions']
                for sample in passed:
                    if len(sample['launch']['instances']) != count or len(sample['launch']['sessions']) != count:
                        raise ValueError('passing launch did not contain every requested instance/session')
                    if sample['kind'] == 'build-idle' and len(sample['workload']) != count:
                        raise ValueError('passing build did not contain every requested workload')
                complete = len(launches) == expected_launches and len(builds) == expected_builds and not failures
                memory, storage = [], []
                for sample in builds:
                    idle = sample['post_build_idle']
                    if idle['window_seconds'] != protocol['idle_observation_seconds']:
                        raise ValueError('idle window differs from frozen protocol')
                    if idle['samples'] < int(idle['window_seconds'] / protocol['sample_interval_seconds']) - 3:
                        raise ValueError('passing sample has insufficient observation cadence')
                    memory.append(number(idle['maximum_per_environment_footprint_bytes_with_shared_observer_cost']) / MIB)
                    cleanup = sample.get('storage_cleanup')
                    if cleanup:
                        if not cleanup['all_helpers_reaped'] or not cleanup['endpoints_removed']:
                            raise ValueError('passing sample has incomplete owned cleanup')
                        storage.append(number(cleanup['standalone_per_environment_upper_bound_allocated_bytes']) / GIB)
                controlled = []
                for sample in builds:
                    if 'controlled_idle' in sample:
                        before, after = sample['post_build_idle'], sample['controlled_idle']
                        if after['window_seconds'] != protocol['idle_observation_seconds']:
                            raise ValueError('controlled recovery window differs from protocol')
                        controlled.append({
                            'repetition': sample['repetition'],
                            'post_build_end_total_mib': number(before['end_total_footprint_bytes']) / MIB,
                            'controlled_end_total_mib': number(after['end_total_footprint_bytes']) / MIB,
                            'residual_delta_total_mib': (number(after['end_total_footprint_bytes']) - number(before['end_total_footprint_bytes'])) / MIB,
                            'residual_delta_per_environment_mib': (after['end_total_footprint_bytes'] - before['end_total_footprint_bytes']) / MIB / count,
                            'scope': 'additional stress-program compile, 256-MiB full-page touch/free and equal idle window; not real-build peak recovery',
                        })
                rows.append({
                    'fixture': fixture, 'adapter': adapter, 'instances_requested': count,
                    'functional_status': 'failed' if failures else 'passed' if complete else 'not-run',
                    'comparison_eligible': complete and not calibration,
                    'successful_launch_repetitions': len(launches), 'successful_build_repetitions': len(builds),
                    'requested_launch_repetitions': expected_launches, 'requested_build_repetitions': expected_builds,
                    'launch_cohort_ready_seconds': distribution(s['launch']['cohort_to_ready_seconds'] for s in launches),
                    'ssh_substrate_session_max_per_cohort_seconds': distribution(max(x['upper_bound_seconds'] for x in s['launch']['sessions']) for s in launches),
                    'source_import_max_per_cohort_seconds': distribution(max(x['source_import_seconds'] for x in s['workload']) for s in builds),
                    'build_verify_max_per_cohort_seconds': distribution(max(x['build_and_verification_seconds'] for x in s['workload']) for s in builds),
                    'post_build_idle_maximum_per_environment_mib': distribution(memory),
                    'post_build_idle_end_total_mib': distribution(s['post_build_idle']['end_total_footprint_bytes'] / MIB for s in builds),
                    'post_build_idle_helper_cpu_percent_one_core': distribution(s['post_build_idle']['helper_cpu_percent_one_core'] for s in builds),
                    'post_build_idle_all_owned_cpu_percent_one_core': distribution(s['post_build_idle']['all_owned_cpu_percent_one_core'] for s in builds),
                    'idle_process_sets_stable': all(s['post_build_idle']['cpu_process_set_stable'] for s in builds) if builds else None,
                    'sampled_full_run_peak_total_mib': distribution(s['sampled_peak_total_footprint_bytes'] / MIB for s in builds),
                    'controlled_recovery': controlled,
                    'runtime_storage_standalone_upper_bound_gib': distribution(storage),
                    'runtime_storage_amortized_per_environment_gib': distribution(s['storage_cleanup']['per_environment_amortized_allocated_bytes'] / GIB for s in builds if 'storage_cleanup' in s),
                    'criteria': {
                        'post_build_idle_memory': limit_status(memory, limits['post_build_idle_memory_mib'], expected_builds),
                        'deployed_runtime_storage': limit_status(storage, limits['physical_storage_per_environment_gib'], expected_builds),
                        'actual_agent_start_in_this_density_fixture': 'not-run; SSH marker is not an agent',
                        'cold_preparation': 'not-run; immutable inputs already materialized',
                    },
                    'failures': [{key: s[key] for key in ('raw', 'kind', 'repetition', 'error', 'observer_error') if key in s} for s in failures],
                })
    complete = not missing and all(row['functional_status'] == 'passed' for row in rows)
    return {
        'schema': 1, 'platform': report['platform'], 'host': report['host'],
        'qualification': 'calibration-only; excluded from comparison' if calibration else 'native matched density evidence; remaining qualification stated per criterion',
        'status': 'running' if report['status'] == 'running' else 'passed' if report['status'] == 'passed' and complete else 'failed',
        'raw_evidence': report['raw_evidence'], 'protocol': protocol, 'thresholds': report['thresholds'],
        'coverage': {'expected_cases': len(expected), 'recorded_cases': len(samples), 'missing_cases': len(missing),
                     'complete': complete, 'comparison_eligible': complete and report['status'] == 'passed' and not calibration,
                     'missing': [dict(zip(('fixture', 'adapter', 'instances_requested', 'kind', 'repetition'), identity)) for identity in sorted(missing)]},
        'rows': rows,
        'limits_of_interpretation': [
            'Minimum/median/maximum per cohort; no P95 estimate from five or ten repetitions.',
            'Physical footprint and resident memory are separate; no PSS or exact global physical attribution claim.',
            'Storage criterion covers deployed immutable runtime inputs plus the largest private environment; retained builder/download caches are separate.',
            'Actual Codex/Claude startup is separately measured in one 1024-MiB prepared VM; density guests use 512 MiB and SSH markers do not qualify their agent startup.',
            'Unavailable native Linux/Intel platform rows remain not-run; these results cannot qualify them.',
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    source = args.run / 'report.json'
    report = json.loads(source.read_text())
    threshold_file = ROOT / 'fixtures/no-go-thresholds.json'
    expected_hash = report['protocol']['thresholds']['sha256']
    if hashlib.sha256(threshold_file.read_bytes()).hexdigest() != expected_hash:
        raise ValueError('accepted threshold record changed')
    result = summarize(report, json.loads(threshold_file.read_text()))
    result['source_report_sha256'] = hashlib.sha256(source.read_bytes()).hexdigest()
    args.out.write_text(json.dumps(result, indent=2, allow_nan=False) + '\n')
    print(json.dumps({'status': result['status'], 'coverage': {k: v for k, v in result['coverage'].items() if k != 'missing'}}))


if __name__ == '__main__':
    main()
