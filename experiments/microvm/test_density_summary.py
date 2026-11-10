import copy
import math
import unittest

from summarize_density import GIB, MIB, summarize


def evidence():
    protocol = {'fixtures': ['alpine'], 'adapters': ['libkrun'], 'instances': [8],
                'launch_stop_cycles': 1, 'full_build_idle_repetitions': 1,
                'idle_observation_seconds': 60, 'sample_interval_seconds': 0.25}
    launch = {'cohort_to_ready_seconds': 2, 'instances': [{'ready_seconds': 2}] * 8,
              'sessions': [{'upper_bound_seconds': 0.1}] * 8}
    base = {'fixture': 'alpine', 'adapter': 'libkrun', 'instances_requested': 8,
            'repetition': 0, 'status': 'passed', 'raw': 'owned-test-case', 'launch': launch}
    first, second = copy.deepcopy(base), copy.deepcopy(base)
    first['kind'], second['kind'] = 'launch', 'build-idle'
    second.update(workload=[{'source_import_seconds': 1, 'build_and_verification_seconds': 3}] * 8,
                  sampled_peak_total_footprint_bytes=8 * 600 * MIB,
                  post_build_idle={'window_seconds': 60, 'samples': 240,
                                   'maximum_per_environment_footprint_bytes_with_shared_observer_cost': 512 * MIB,
                                   'end_total_footprint_bytes': 8 * 512 * MIB,
                                   'helper_cpu_percent_one_core': 1, 'all_owned_cpu_percent_one_core': 2,
                                   'cpu_process_set_stable': True},
                  storage_cleanup={'all_helpers_reaped': True, 'endpoints_removed': True,
                                   'standalone_per_environment_upper_bound_allocated_bytes': 2 * GIB,
                                   'per_environment_amortized_allocated_bytes': 0.4 * GIB})
    report = {'protocol': protocol, 'qualification': 'native experiment', 'samples': [first, second],
              'platform': 'macos-arm64-native', 'host': ['Darwin'], 'status': 'passed',
              'raw_evidence': 'test-only', 'thresholds': {}}
    thresholds = {'original_no_go_thresholds': {'post_build_idle_memory_mib': 4096,
                                              'physical_storage_per_environment_gib': 8}}
    return report, thresholds


class SummaryQualification(unittest.TestCase):
    def test_fast_ssh_does_not_qualify_actual_agent_start(self):
        report, thresholds = evidence()
        row = summarize(report, thresholds)['rows'][0]
        self.assertTrue(row['comparison_eligible'])
        self.assertTrue(row['criteria']['actual_agent_start_in_this_density_fixture'].startswith('not-run'))

    def test_over_budget_single_environment_is_not_divided_by_cohort_size(self):
        report, thresholds = evidence()
        report['samples'][1]['post_build_idle']['maximum_per_environment_footprint_bytes_with_shared_observer_cost'] = 4096 * MIB + 1
        result = summarize(report, thresholds)
        self.assertEqual(result['rows'][0]['criteria']['post_build_idle_memory'], 'failed')

    def test_missing_and_failed_cases_never_become_complete(self):
        report, thresholds = evidence()
        report['samples'][1].update(status='failed', error='out of memory')
        result = summarize(report, thresholds)
        self.assertFalse(result['coverage']['complete'])
        self.assertEqual(result['rows'][0]['functional_status'], 'failed')
        self.assertEqual(result['rows'][0]['criteria']['post_build_idle_memory'], 'not-run')
        report['samples'].pop()
        result = summarize(report, thresholds)
        self.assertEqual(result['coverage']['missing_cases'], 1)
        self.assertFalse(result['coverage']['comparison_eligible'])

    def test_duplicate_samples_are_rejected_instead_of_counted_twice(self):
        report, thresholds = evidence()
        report['samples'].append(copy.deepcopy(report['samples'][0]))
        with self.assertRaises(ValueError):
            summarize(report, thresholds)

    def test_running_or_calibration_dataset_never_qualifies(self):
        report, thresholds = evidence()
        report['status'] = 'running'
        self.assertFalse(summarize(report, thresholds)['coverage']['comparison_eligible'])
        report.update(status='passed', qualification='calibration-only')
        result = summarize(report, thresholds)
        self.assertFalse(result['coverage']['comparison_eligible'])
        self.assertFalse(result['rows'][0]['comparison_eligible'])

    def test_nonfinite_counter_cannot_pass_a_limit(self):
        report, thresholds = evidence()
        report['samples'][1]['post_build_idle']['maximum_per_environment_footprint_bytes_with_shared_observer_cost'] = math.nan
        with self.assertRaises(ValueError):
            summarize(report, thresholds)


if __name__ == '__main__':
    unittest.main()
