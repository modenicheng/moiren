# SPDX-License-Identifier: MIT
"""Offline removal evidence never turns callback markers into acceptance."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1] / 'vad-removal-stages.py'
if SOURCE.exists():
    SPEC = importlib.util.spec_from_file_location('removal_stages', SOURCE)
    MODULE = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(MODULE)
else:
    MODULE = None


def report(state='Removing'):
    return {'native': {
        'error': None if state == 'Removed' else 'VAD cleanup did not reach Removed',
        'vad_playback': {'worker_joined': True, 'native_stop_succeeded': True, 'failure': None},
        'engine_cleanup': {'host': 'Closed', 'fault': 'None', 'cleanup_error': 'None'},
        'vad_cleanup': {'diagnostic_provenance': 'Fresh', 'error': None,
                        'snapshot': {'stamp': [21, 99, 21, 6, 8], 'state': state,
                                     'stream_state': 'Closed', 'gate_open': False}}}}


def trace(stage=0, sequence=1, status='STATUS_SUCCESS', has_status=0, native_call=0,
          endpoint=21, session=99, count=1, saturated=0, kind=0):
    return (f'[0]0004.1234::11/08/2026-00:00:00.000 [vad]Removal stage endpoint={endpoint} '
            f'session={session} stage={stage} count={count} sequence={sequence} '
            f'frequency=10000000 saturated={saturated}\n'
            f'[0]0004.1234::11/08/2026-00:00:00.000 [vad]Removal observation endpoint={endpoint} '
            f'session={session} stage={stage} kind={kind} sequence={sequence} qpc={100 + sequence} '
            f'has_status={has_status} status={status} native_call={native_call}\n')


SUMMARY = ('Total Events Processed 24\nTotal Events Lost 0\n'
           'Total Format Errors 0\nTotal Formats Unknown 0\n')


class RemovalStages(unittest.TestCase):
    def analyze(self, text=None, summary=SUMMARY, native=None):
        self.assertIsNotNone(MODULE, 'bounded removal-stage reader is not implemented')
        return MODULE.analyze(trace() if text is None else text, summary,
                              report() if native is None else native)

    def test_partial_stages_are_useful_but_never_cleanup_pass(self):
        value = self.analyze()
        self.assertTrue(value['trace_association_available'])
        self.assertFalse(value['direct_removed_observed'])
        self.assertFalse(value['qualified'])
        self.assertEqual(value['observed_stages'], ['remove_enter'])
        self.assertIn('release_enter', value['unobserved_stages'])
        self.assertEqual(value['native_error'], 'VAD cleanup did not reach Removed')

    def test_destroy_entry_does_not_replace_removed_query(self):
        value = self.analyze(trace(stage=5))
        self.assertFalse(value['direct_removed_observed'])
        self.assertEqual(value['observed_stages'], ['destroy_entry'])

    def test_actual_fresh_removed_and_source_join_are_separate_fields(self):
        value = self.analyze(native=report('Removed'))
        self.assertTrue(value['direct_removed_observed'])
        self.assertTrue(value['source_stop_join_observed'])
        self.assertTrue(value['engine_closed_observed'])
        self.assertFalse(value['qualified'])

    def test_cached_removed_is_not_direct_query_evidence(self):
        native = report('Removed')
        native['native']['vad_cleanup']['diagnostic_provenance'] = 'CachedAfterQueryFailure'
        self.assertFalse(self.analyze(native=native)['direct_removed_observed'])

    def test_missing_error_fields_do_not_become_positive_lifecycle_evidence(self):
        native = report('Removed')
        del native['native']['vad_cleanup']['error']
        self.assertFalse(self.analyze(native=native)['direct_removed_observed'])
        native = report('Removed')
        del native['native']['vad_playback']['failure']
        self.assertFalse(self.analyze(native=native)['source_stop_join_observed'])

    def test_native_status_bits_and_idempotent_no_call_are_preserved(self):
        value = self.analyze(trace(stage=1, status='0xC0000001', has_status=1))
        observation = value['observations'][0]
        self.assertEqual(observation['status'], '0xC0000001')
        self.assertTrue(observation['has_status'])
        self.assertFalse(observation['native_call'])

    def test_symbolic_status_is_retained_not_guessed(self):
        value = self.analyze(trace(stage=3, status='STATUS_UNSUCCESSFUL', has_status=1, native_call=1))
        self.assertEqual(value['observations'][0]['status'], 'STATUS_UNSUCCESSFUL')

    def test_first_failure_requires_status_and_failure_bits(self):
        for status, has_status in (('STATUS_SUCCESS', 0), ('STATUS_UNSUCCESSFUL', 0),
                                   ('STATUS_SUCCESS', 1), ('0x00000000', 1),
                                   ('0x40000001', 1)):
            with self.subTest(status=status, has_status=has_status):
                value = self.analyze(trace(stage=3, kind=2, status=status, has_status=has_status))
                self.assertFalse(value['trace_association_available'])
        for status in ('STATUS_UNSUCCESSFUL', '0xC0000001', '0x80000005',
                       'STATUS_UNRECOGNIZED_SYMBOL'):
            with self.subTest(status=status):
                value = self.analyze(trace(stage=3, kind=2, status=status, has_status=1))
                self.assertTrue(value['trace_association_available'])
                self.assertEqual(value['observations'][0]['status'], status)

    def test_foreign_runtime_never_supplies_target_stage(self):
        value = self.analyze(trace() + trace(stage=5, endpoint=22))
        self.assertEqual(value['foreign_fragments'], 2)
        self.assertEqual(value['observed_stages'], ['remove_enter'])

    def test_only_foreign_runtime_refused(self):
        self.assertFalse(self.analyze(trace(endpoint=22))['trace_association_available'])

    def test_overwrite_notice_refuses_complete_association(self):
        text = trace() + ('[vad]Removal overwritten endpoint=21 session=99 count=1 '
                          'old_endpoint=20 old_session=98 old_sequence=6 old_pending=63 saturated=0\n')
        value = self.analyze(text)
        self.assertFalse(value['trace_association_available'])
        self.assertEqual(value['overwrites'][0]['old_endpoint'], 20)

    def test_new_runtime_overwrite_of_target_is_not_ignored_as_foreign(self):
        text = trace() + ('[vad]Removal overwritten endpoint=22 session=100 count=1 '
                          'old_endpoint=21 old_session=99 old_sequence=6 old_pending=63 saturated=0\n')
        value = self.analyze(text)
        self.assertFalse(value['trace_association_available'])
        self.assertEqual(value['overwrites'][0]['old_endpoint'], 21)

    def test_saturation_refused(self):
        self.assertFalse(self.analyze(trace(saturated=1))['trace_association_available'])

    def test_lost_format_or_unknown_events_refused(self):
        for field in ('Total Events Lost', 'Total Format Errors', 'Total Formats Unknown'):
            with self.subTest(field=field):
                summary = SUMMARY.replace(field + ' 0', field + ' 1')
                self.assertFalse(self.analyze(summary=summary)['trace_association_available'])

    def test_summary_missing_duplicate_or_nonnumeric_refused(self):
        for summary in ('', SUMMARY + 'Total Events Lost 0\n', SUMMARY.replace('Lost 0', 'Lost unknown'),
                        SUMMARY + 'Total Events Lost unknown\n'):
            with self.subTest(summary=summary):
                self.assertFalse(self.analyze(summary=summary)['trace_association_available'])

    def test_malformed_target_fragment_refused(self):
        for text in (trace().replace('stage=0', 'stage=6'),
                     trace().replace('count=1', 'count=0'),
                     trace().replace('frequency=10000000', 'frequency=0'),
                     trace().replace('native_call=0', 'native_call=2'),
                     trace().replace('qpc=101', 'qpc=-1'),
                     trace().replace('sequence=1', 'sequence=18446744073709551616')):
            with self.subTest(text=text):
                self.assertFalse(self.analyze(text)['trace_association_available'])

    def test_observation_requires_matching_stage_header(self):
        self.assertFalse(self.analyze(trace().split('\n', 1)[1])['trace_association_available'])

    def test_repeated_first_last_observation_deduplicated_without_inventing_attempts(self):
        value = self.analyze(trace() + trace(kind=1))
        self.assertTrue(value['trace_association_available'])
        self.assertEqual(len(value['observations']), 1)
        self.assertEqual(value['observations'][0]['kinds'], ['first', 'last'])

    def test_same_sequence_different_status_refused(self):
        self.assertFalse(self.analyze(trace() + trace(status='STATUS_UNSUCCESSFUL'))['trace_association_available'])

    def test_qpc_regression_refused_but_equal_qpc_allowed(self):
        second = trace(stage=1, sequence=2)
        self.assertFalse(self.analyze(trace() + second.replace('qpc=102', 'qpc=100'))['trace_association_available'])
        self.assertTrue(self.analyze(trace() + second.replace('qpc=102', 'qpc=101'))['trace_association_available'])

    def test_frequency_change_refused(self):
        self.assertFalse(self.analyze(trace() + trace(stage=1, sequence=2).replace(
            'frequency=10000000', 'frequency=9999999'))['trace_association_available'])

    def test_stage_count_watermark_cannot_regress_or_exceed_sequence(self):
        for text in (trace(count=2), trace(sequence=2, count=2) + trace(sequence=1, count=1)):
            with self.subTest(text=text):
                self.assertFalse(self.analyze(text)['trace_association_available'])

    def test_stage_count_growth_requires_at_least_equal_sequence_growth(self):
        for text in (trace(sequence=2, count=1) + trace(sequence=2, count=2),
                     trace(sequence=2, count=1) + trace(sequence=3, count=3)):
            with self.subTest(text=text):
                self.assertFalse(self.analyze(text)['trace_association_available'])
        value = self.analyze(trace(sequence=2, count=1) + trace(sequence=4, count=2))
        self.assertTrue(value['trace_association_available'])

    def test_real_production_export_text_patterns_remain_compatible(self):
        self.assertIsNotNone(MODULE, 'bounded removal-stage reader is not implemented')
        root = SOURCE.parents[1]
        control = (root / 'driver/vad/src/Control.cpp').read_text(encoding='utf-8')
        header = (root / 'driver/vad/src/RemovalDiagnostic.h').read_text(encoding='utf-8')
        self.assertIn('RemoveEnter, RemoveReturn, ReleaseEnter, ReleaseReturn, Cleanup, Destroy, Count', header)
        import re
        formats = re.findall(r'"Removal (stage|observation|overwritten) ([^"\n]+)"', control)
        self.assertEqual(len(formats), 3)
        for kind, pattern in formats:
            fields = [part.split('=')[0] for part in pattern.split()]
            self.assertEqual(fields, MODULE.FIELDS[kind])

    def test_report_stamp_must_be_exact_typed_runtime_and_session(self):
        for stamp in ([True, 99, 21, 6, 8], ['21', 99, 21, 6, 8], [21, 99], [21, 99, 22, 6, 8]):
            with self.subTest(stamp=stamp):
                native = report()
                native['native']['vad_cleanup']['snapshot']['stamp'] = stamp
                self.assertFalse(self.analyze(native=native)['trace_association_available'])

    def test_different_session_of_same_runtime_is_not_adopted(self):
        self.assertFalse(self.analyze(trace() + trace(session=100))['trace_association_available'])

    def test_missing_or_failed_stop_and_engine_not_hidden(self):
        native = report()
        native['native']['vad_playback']['worker_joined'] = False
        native['native']['engine_cleanup']['cleanup_error'] = 'failed'
        value = self.analyze(native=native)
        self.assertFalse(value['source_stop_join_observed'])
        self.assertFalse(value['engine_closed_observed'])

    def test_input_and_output_limits_are_enforced_before_write(self):
        self.assertIsNotNone(MODULE, 'bounded removal-stage reader is not implemented')
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for name, data in [('trace', trace()), ('summary', SUMMARY), ('report', '{}')]:
                (root / name).write_text(data, encoding='ascii')
            output = root / 'out.json'
            command = [sys.executable, '-B', str(SOURCE), '--trace', str(root / 'trace'),
                       '--summary', str(root / 'summary'), '--report', str(root / 'report'),
                       '--max-input-bytes', '1', '--output', str(output)]
            failed = subprocess.run(command, capture_output=True, timeout=10)
            self.assertEqual(failed.returncode, 2)
            self.assertFalse(output.exists())
            output.write_text('sentinel', encoding='ascii')
            command[command.index('--max-input-bytes') + 1] = '100000'
            subprocess.run(command, capture_output=True, timeout=10)
            self.assertEqual(output.read_text(), 'sentinel')

    def test_ambiguous_or_nonfinite_report_json_cannot_supply_removed(self):
        self.assertIsNotNone(MODULE, 'bounded removal-stage reader is not implemented')
        raw = json.dumps(report())
        malformed = (raw.replace('"state": "Removing"', '"state": "Removing", "state": "Removed"'),
                     raw[:-1] + ', "untrusted": NaN}')
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'trace').write_text(trace(), encoding='ascii')
            (root / 'summary').write_text(SUMMARY, encoding='ascii')
            for index, data in enumerate(malformed):
                with self.subTest(index=index):
                    (root / 'report').write_text(data, encoding='ascii')
                    output = root / ('out' + str(index) + '.json')
                    failed = subprocess.run(
                        [sys.executable, '-B', str(SOURCE), '--trace', str(root / 'trace'),
                         '--summary', str(root / 'summary'), '--report', str(root / 'report'),
                         '--max-input-bytes', '100000', '--output', str(output)],
                        capture_output=True, timeout=10)
                    self.assertEqual(failed.returncode, 2)
                    self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
