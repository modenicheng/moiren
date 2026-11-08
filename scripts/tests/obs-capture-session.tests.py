# SPDX-License-Identifier: MIT
"""Real reuse, setup and recording state machine, without OBS or networking."""
import importlib.util
import json
from pathlib import Path
import sys
import unittest

sys.dont_write_bytecode = True
SCRIPTS = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


FIXTURES = load('capture_session_fixture', SCRIPTS / 'tests/obs-acquisition.tests.py')
SOURCE = SCRIPTS / 'obs-capture-session.py'
MODULE = load('obs_capture_session_tests', SOURCE) if SOURCE.exists() else None


class CaptureSessionTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(MODULE, 'fixed-path capture-session integration is missing')
        self.fixture = FIXTURES.AcquisitionTests('test_one_open_attempt_even_after_success')
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        f = self.fixture
        self.session = MODULE.ObsCaptureSession(
            f.case, f.close_window, acquisition=f.adapter, clock=f.clock, sleep=f.clock.sleep)
        self.statuses = [True, False]
        self.start_error = self.stop_error = False
        connect = f.connect
        def wrapped_connect(password, *, timeout):
            client = connect(password, timeout=timeout)
            original = client.req
            def request(kind, data=None, *, timeout):
                result = original(kind, data, timeout=timeout)
                if kind == 'StartRecord':
                    self.assertIsNotNone(self.session.recording)
                    self.assertTrue(self.session.recording.responsible)
                    if self.start_error:
                        raise OSError('lost start reply')
                if kind == 'StopRecord' and self.stop_error:
                    raise OSError('lost stop reply')
                if kind == 'GetRecordStatus':
                    status = self.statuses.pop(0) if len(self.statuses) > 1 else self.statuses[0]
                    return {'outputActive': status}
                return result
            client.req = request
            return client
        f.adapter.connect = wrapped_connect

    def test_retained_setup_recording_and_close_order(self):
        self.session.start()
        result = self.session.finish()
        f = self.fixture
        for earlier, later in (('StartRecord', 'StopRecord'),
                               ('StopRecord', 'socket.close'), ('socket.close', 'window.close')):
            self.assertLess(f.events.index(earlier), f.events.index(later))
        self.assertTrue(result['capture_lifecycle_complete'])
        self.assertFalse(result['acquisition_accepted'])
        self.assertFalse(result['qualified'])
        self.assertEqual(result['application_copies'], 0)
        self.assertIsNone(f.reuse.lease)
        self.assertEqual(json.loads((f.run / 'obs-capture-session.json').read_text()), result)

    def test_lost_start_is_retained_and_cleaned_but_never_successful(self):
        self.start_error = True
        self.statuses = [False, True, False]
        with self.assertRaises(OSError):
            self.session.start()
        self.assertTrue(self.session.recording.responsible)
        result = self.session.finish()
        self.assertTrue(result['boundary']['boundary_complete'])
        self.assertFalse(result['capture_lifecycle_complete'])
        self.assertTrue(result['recording']['recording_inactive'])
        self.assertEqual(self.fixture.events.count('StartRecord'), 1)
        self.assertEqual(self.fixture.events.count('StopRecord'), 1)

    def test_lost_stop_reply_can_close_after_inactive_but_never_pass(self):
        self.session.start()
        self.stop_error = True
        result = self.session.finish()
        self.assertFalse(result['capture_lifecycle_complete'])
        self.assertTrue(result['boundary']['boundary_complete'])
        self.assertTrue(result['recording']['recording_inactive'])
        self.assertIsNone(self.fixture.reuse.lease)

    def test_unknown_recording_stays_owned_without_closing_socket(self):
        self.start_error = True
        self.statuses = [False]
        with self.assertRaises(OSError):
            self.session.start()
        with self.assertRaisesRegex(RuntimeError, 'recording.*unresolved'):
            self.session.finish()
        self.assertIsNotNone(self.fixture.reuse.lease)
        self.assertNotIn('window.close', self.fixture.events)
        self.assertNotIn('socket.close', self.fixture.events)
        self.assertFalse((self.fixture.run / 'obs-capture-session.json').exists())
        self.statuses = [True, False]
        result = self.session.finish()
        self.assertFalse(result['capture_lifecycle_complete'])
        self.assertEqual(self.fixture.events.count('StartRecord'), 1)

    def test_setup_failure_before_recording_never_loses_owner(self):
        self.fixture.fail_kind = 'CreateInput'
        with self.assertRaises(RuntimeError):
            self.session.start()
        self.assertIsNone(self.session.recording)
        self.assertIs(self.session.acquisition.process, self.fixture.process)
        result = self.session.finish()
        self.assertFalse(result['capture_lifecycle_complete'])
        self.assertIsNone(result['recording'])
        self.assertIsNone(self.fixture.reuse.lease)
        self.assertNotIn('StartRecord', self.fixture.events)

    def test_unknown_launcher_failure_cannot_release_or_restart(self):
        self.fixture.launch_error = True
        with self.assertRaises(RuntimeError):
            self.session.start()
        with self.assertRaises(RuntimeError):
            self.session.finish()
        with self.assertRaises(RuntimeError):
            self.session.start()
        self.assertIsNotNone(self.fixture.reuse.lease)
        self.assertEqual(len(self.fixture.case.launches), 1)

    def test_close_failure_retains_lease_and_cannot_become_success(self):
        self.session.start()
        self.fixture.join_error = True
        with self.assertRaises(RuntimeError):
            self.session.finish()
        self.fixture.process.exit_code = 0
        result = self.session.finish()
        self.assertFalse(result['capture_lifecycle_complete'])
        self.assertEqual(self.fixture.events.count('window.close'), 1)

    def test_prior_session_receipt_refuses_before_any_launch_or_config(self):
        prior = self.fixture.run / 'obs-capture-session.json'
        prior.write_bytes(b'retained evidence')
        with self.assertRaises(ValueError):
            self.session.start()
        self.assertEqual(self.fixture.case.launches, [])
        self.assertIsNone(self.fixture.reuse.lease)
        self.assertEqual(prior.read_bytes(), b'retained evidence')

    def test_second_finish_cannot_overwrite_or_repeat_close(self):
        self.session.start()
        self.session.finish()
        prior = (self.fixture.run / 'obs-capture-session.json').read_bytes()
        with self.assertRaises(RuntimeError):
            self.session.finish()
        with self.assertRaises(RuntimeError):
            self.session.start()
        self.assertEqual((self.fixture.run / 'obs-capture-session.json').read_bytes(), prior)
        self.assertEqual(self.fixture.events.count('window.close'), 1)

    def protocol_connector(self, *, lost_start=False, lost_stop=False):
        protocol = load('capture_protocol_fixture', SCRIPTS / 'tests/obs-websocket.tests.py')
        module = protocol.M
        rows = [protocol.hello(), protocol.identified()]
        kinds = ('GetVersion', 'CreateInput', 'SetInputAudioTracks',
                 'SetInputAudioMonitorType', 'SetInputAudioSyncOffset', 'SetInputVolume',
                 'StartRecord', 'GetRecordStatus', 'StopRecord', 'GetRecordStatus')
        for sequence, kind in enumerate(kinds, 1):
            body = {'outputActive': sequence == 8} if kind == 'GetRecordStatus' else {}
            rows.append(protocol.response(kind, str(sequence), responseData=body))
        owner = protocol.Socket(self.fixture.clock, rows)
        if lost_start or lost_stop:
            original_send = owner.send
            def send(value):
                result = original_send(value)
                data = json.loads(value)
                mutation = data['d'].get('requestType')
                if data['op'] == 6 and (lost_start and mutation == 'StartRecord' or
                                        lost_stop and mutation == 'StopRecord'):
                    raise OSError('recording mutation outcome unknown')
                return result
            owner.send = send
        self.fixture.adapter.connect = lambda password, *, timeout: module.connect(
            password, timeout=timeout, dial=lambda _url, *, timeout: owner,
            clock=self.fixture.clock)
        return owner

    def test_actual_json_client_through_capture_lifecycle_with_fake_wire_only(self):
        owner = self.protocol_connector()
        self.session.start()
        result = self.session.finish()
        self.assertTrue(result['capture_lifecycle_complete'])
        self.assertFalse(result['acquisition_accepted'])
        self.assertEqual(owner.close_calls, 1)
        self.assertEqual(len(owner.sent), 11)
        self.assertIsNone(self.fixture.reuse.lease)

    def test_real_client_poisoned_by_lost_start_cannot_fake_recording_cleanup(self):
        owner = self.protocol_connector(lost_start=True)
        with self.assertRaises(RuntimeError):
            self.session.start()
        with self.assertRaisesRegex(RuntimeError, 'recording.*unresolved'):
            self.session.finish()
        self.assertTrue(self.session.recording.responsible)
        self.assertFalse(self.session.recording.recording_inactive)
        self.assertFalse(self.session.client.usable)
        self.assertEqual(owner.close_calls, 0)
        requests = [json.loads(row)['d'].get('requestType') for row in owner.sent]
        self.assertEqual(requests.count('StartRecord'), 1)
        self.assertNotIn('StopRecord', requests)
        self.assertNotIn('window.close', self.fixture.events)
        self.assertIsNotNone(self.fixture.reuse.lease)
        self.assertFalse((self.fixture.run / 'obs-capture-session.json').exists())

    def test_real_client_poisoned_by_lost_stop_preserves_socket_and_lease(self):
        owner = self.protocol_connector(lost_stop=True)
        self.session.start()
        with self.assertRaisesRegex(RuntimeError, 'recording.*unresolved'):
            self.session.finish()
        with self.assertRaisesRegex(RuntimeError, 'recording.*unresolved'):
            self.session.finish()
        self.assertTrue(self.session.recording.responsible)
        self.assertTrue(self.session.recording.stop_dispatched)
        self.assertFalse(self.session.recording.recording_inactive)
        self.assertEqual(owner.close_calls, 0)
        requests = [json.loads(row)['d'].get('requestType') for row in owner.sent]
        self.assertEqual(requests.count('StartRecord'), 1)
        self.assertEqual(requests.count('StopRecord'), 1)
        self.assertNotIn('window.close', self.fixture.events)
        self.assertIsNotNone(self.fixture.reuse.lease)
        self.assertFalse((self.fixture.run / 'obs-capture-session.json').exists())

    def test_real_handshake_cleanup_failure_keeps_owner_until_explicit_finish(self):
        protocol = load('capture_failed_handshake_fixture', SCRIPTS / 'tests/obs-websocket.tests.py')
        owner = protocol.Socket(self.fixture.clock, ['malformed'])
        owner.close_error = RuntimeError('synthetic close failure')
        attempts = []
        def dial(url, *, timeout):
            attempts.append(url)
            return owner
        self.fixture.adapter.connect = lambda password, *, timeout: protocol.M.connect(
            password, timeout=timeout, dial=dial, clock=self.fixture.clock)
        with self.assertRaises(RuntimeError):
            self.session.start()
        self.assertIs(self.session.client, owner)
        self.assertEqual(len(attempts), 1)
        self.assertIsNone(self.session.recording)
        with self.assertRaises(RuntimeError):
            self.session.finish()
        self.assertNotIn('window.close', self.fixture.events)
        self.assertIsNotNone(self.fixture.reuse.lease)
        owner.close_error = None
        result = self.session.finish()
        self.assertFalse(result['capture_lifecycle_complete'])
        self.assertEqual(len(attempts), 1)
        self.assertIsNone(self.fixture.reuse.lease)
        self.assertEqual(self.fixture.events.count('window.close'), 1)


if __name__ == '__main__':
    unittest.main()
