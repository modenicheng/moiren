# SPDX-License-Identifier: MIT
"""Recording transition ownership with fake RPC, clock and retained process."""
import importlib.util
from pathlib import Path
import sys
import unittest

sys.dont_write_bytecode = True
SOURCE = Path(__file__).resolve().parents[1] / 'obs-recording.py'
SPEC = importlib.util.spec_from_file_location('obs_recording_tests', SOURCE) if SOURCE.exists() else None
MODULE = importlib.util.module_from_spec(SPEC) if SPEC else None
if SPEC:
    SPEC.loader.exec_module(MODULE)


class Clock:
    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


class Process:
    def __init__(self):
        self.exit_code = None

    def poll(self):
        return self.exit_code


class Case:
    def __init__(self):
        self.expired = False

    def check(self):
        if self.expired:
            raise RuntimeError('case deadline')


class Client:
    def __init__(self, fixture):
        self.fixture = fixture
        self.calls = []
        self.statuses = [True, False]
        self.status_error = False
        self.start_error = self.stop_error = False
        self.delay = 0.0

    def req(self, kind, data=None, *, timeout):
        self.calls.append((kind, timeout))
        self.fixture.clock.now += self.delay
        if kind == 'StartRecord':
            self.fixture.assertTrue(self.fixture.recording.responsible)
            self.fixture.assertTrue(self.fixture.recording.start_dispatched)
            if self.start_error:
                raise OSError('synthetic lost start reply with PRIVATE-DATA')
        elif kind == 'StopRecord':
            self.fixture.assertTrue(self.fixture.recording.responsible)
            self.fixture.assertTrue(self.fixture.recording.stop_dispatched)
            if self.stop_error:
                raise OSError('synthetic lost stop reply with PRIVATE-DATA')
            return {'outputPath': 'synthetic-recording.mkv'}
        elif kind == 'GetRecordStatus':
            if self.status_error:
                raise OSError('synthetic status failure with PRIVATE-DATA')
            value = self.statuses.pop(0) if len(self.statuses) > 1 else self.statuses[0]
            return value if isinstance(value, dict) else {'outputActive': value}
        return {}


class RecordingTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(MODULE, 'bounded OBS recording transitions are missing')
        self.case, self.clock, self.process = Case(), Clock(), Process()
        self.client = Client(self)
        self.recording = MODULE.ObsRecording(
            self.case, self.client, self.process, clock=self.clock, sleep=self.clock.sleep)

    def test_no_dispatched_start_is_an_inactive_witness_without_rpc(self):
        self.assertTrue(self.recording.recording_inactive)
        self.assertIsNone(self.recording.stop())
        self.assertEqual(self.client.calls, [])
        self.assertFalse(self.recording.receipt()['recording_complete'])

    def test_start_and_stop_require_observed_transitions_in_order(self):
        self.recording.start()
        self.assertTrue(self.recording.active_observed)
        self.assertTrue(self.recording.responsible)
        self.assertFalse(self.recording.recording_inactive)
        response = self.recording.stop()
        self.assertEqual(response, {'outputPath': 'synthetic-recording.mkv'})
        self.assertEqual([kind for kind, _ in self.client.calls], [
            'StartRecord', 'GetRecordStatus', 'StopRecord', 'GetRecordStatus'])
        self.assertTrue(self.recording.recording_inactive)
        self.assertFalse(self.recording.responsible)
        self.assertTrue(self.recording.receipt()['recording_complete'])
        self.assertFalse(self.recording.receipt()['acquisition_accepted'])
        self.assertFalse(self.recording.receipt()['qualified'])
        self.assertTrue(all(0 < timeout <= 5 for _, timeout in self.client.calls))

    def test_lost_start_reply_keeps_responsibility_and_waits_for_active_before_stop(self):
        self.client.start_error = True
        self.client.statuses = [False, True, False]
        with self.assertRaises(OSError):
            self.recording.start()
        self.assertTrue(self.recording.responsible)
        self.assertFalse(self.recording.recording_inactive)
        self.recording.stop()
        self.assertEqual([kind for kind, _ in self.client.calls], [
            'StartRecord', 'GetRecordStatus', 'GetRecordStatus', 'StopRecord', 'GetRecordStatus'])
        self.assertTrue(self.recording.recording_inactive)
        self.assertFalse(self.recording.receipt()['recording_complete'])
        self.assertNotIn('PRIVATE-DATA', str(self.recording.receipt()))

    def test_no_repeated_start_even_when_first_reply_is_lost(self):
        self.client.start_error = True
        with self.assertRaises(OSError):
            self.recording.start()
        with self.assertRaises(RuntimeError):
            self.recording.start()
        self.assertEqual([kind for kind, _ in self.client.calls], ['StartRecord'])

    def test_inactive_before_unknown_start_activation_does_not_release_owner(self):
        self.client.start_error = True
        self.client.statuses = [False]
        with self.assertRaises(OSError):
            self.recording.start()
        with self.assertRaisesRegex(RuntimeError, 'active.*deadline|attempt'):
            self.recording.stop()
        self.assertTrue(self.recording.responsible)
        self.assertFalse(self.recording.recording_inactive)
        self.assertNotIn('StopRecord', [kind for kind, _ in self.client.calls])
        self.assertLessEqual(self.clock.now, 10)
        self.assertLessEqual(len(self.client.calls), 201)

    def test_stop_failure_still_observes_inactive_and_remains_failed(self):
        self.recording.start()
        self.client.stop_error = True
        with self.assertRaises(OSError):
            self.recording.stop()
        self.assertTrue(self.recording.recording_inactive)
        self.assertFalse(self.recording.responsible)
        self.assertFalse(self.recording.receipt()['recording_complete'])
        self.assertEqual(sum(kind == 'StopRecord' for kind, _ in self.client.calls), 1)
        self.recording.stop()
        self.assertEqual(sum(kind == 'StopRecord' for kind, _ in self.client.calls), 1)

    def test_failed_stop_observation_does_not_allow_dispatch_retry(self):
        self.recording.start()
        self.client.status_error = True
        with self.assertRaises(OSError):
            self.recording.stop()
        self.assertTrue(self.recording.responsible)
        self.assertTrue(self.recording.stop_dispatched)
        self.client.status_error = False
        self.recording.stop()
        self.assertEqual(sum(kind == 'StopRecord' for kind, _ in self.client.calls), 1)
        self.assertTrue(self.recording.recording_inactive)
        self.assertFalse(self.recording.receipt()['recording_complete'])

    def test_start_attempt_is_consumed_before_case_deadline_check(self):
        self.case.expired = True
        with self.assertRaises(RuntimeError):
            self.recording.start()
        self.case.expired = False
        with self.assertRaises(RuntimeError):
            self.recording.start()
        self.assertTrue(self.recording.recording_inactive)
        self.assertEqual(self.client.calls, [])

    def test_exited_process_cannot_be_mistaken_for_inactive(self):
        self.recording.start()
        self.process.exit_code = 0
        with self.assertRaisesRegex(RuntimeError, 'exited'):
            self.recording.stop()
        self.assertFalse(self.recording.recording_inactive)
        self.assertTrue(self.recording.responsible)
        self.assertFalse(self.recording.stop_dispatched)

    def test_status_requires_literal_boolean(self):
        for value in (None, 0, 1, 'false', {}, {'outputActive': 1}):
            with self.subTest(value=value):
                other = MODULE.ObsRecording(self.case, self.client, self.process,
                                            clock=self.clock, sleep=self.clock.sleep)
                self.recording = other
                self.client.statuses = [value]
                with self.assertRaisesRegex(RuntimeError, 'recording status'):
                    other.start()
                self.assertTrue(other.responsible)
                self.assertFalse(other.recording_inactive)

    def test_start_deadline_checks_late_success(self):
        self.client.delay = 6
        with self.assertRaisesRegex(RuntimeError, 'deadline'):
            self.recording.start()
        self.assertFalse(self.recording.active_observed)
        self.assertFalse(self.recording.recording_inactive)

    def test_frozen_clock_still_has_finite_poll_bound(self):
        self.client.statuses = [False]
        self.recording = MODULE.ObsRecording(self.case, self.client, self.process,
                                            clock=self.clock, sleep=lambda _seconds: None)
        with self.assertRaisesRegex(RuntimeError, 'attempt'):
            self.recording.start()
        self.assertLessEqual(len(self.client.calls), 201)

    def test_stop_deadline_is_shared_by_rpc_and_inactive_observation(self):
        self.recording.start()
        self.client.delay = 6
        with self.assertRaisesRegex(RuntimeError, 'deadline'):
            self.recording.stop()
        self.assertTrue(self.recording.responsible)
        self.assertFalse(self.recording.recording_inactive)
        self.assertFalse(self.recording.receipt()['recording_complete'])

    def test_double_failure_preserves_stop_and_observation_failures(self):
        self.recording.start()
        self.client.stop_error = self.client.status_error = True
        with self.assertRaises(OSError):
            self.recording.stop()
        phases = {row['phase'] for row in self.recording.receipt()['errors']}
        self.assertIn('stop', phases)
        self.assertIn('inactive', phases)
        self.assertTrue(self.recording.responsible)
        self.assertNotIn('PRIVATE-DATA', str(self.recording.receipt()))


if __name__ == '__main__':
    unittest.main()
