# SPDX-License-Identifier: MIT
"""Real reuse/lease/configuration with fake process and websocket boundaries."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
SCRIPTS = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


REUSE = load('acquisition_reuse_tests', SCRIPTS / 'obs-reuse.py')
SOURCE = SCRIPTS / 'obs-acquisition.py'
MODULE = load('obs_acquisition_tests', SOURCE) if SOURCE.exists() else None
RECORDING = load('obs_recording_acquisition_tests', SCRIPTS / 'obs-recording.py')


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


class Client:
    def __init__(self, fixture, fail=None):
        self.fixture, self.fail = fixture, fail
        self.calls = []
        self.sock = self
        self.closed = False

    def req(self, kind, data=None, *, timeout):
        self.calls.append((kind, data, timeout))
        self.fixture.events.append(kind)
        self.fixture.clock.now += self.fixture.rpc_delay
        if kind == self.fail:
            raise RuntimeError('synthetic RPC failure')
        return {}

    def close(self):
        self.fixture.events.append('socket.close')
        if self.fixture.socket_error:
            raise RuntimeError('synthetic socket close failure')
        self.closed = True


class Case:
    def __init__(self, fixture):
        self.fixture = fixture
        self.root = fixture.run
        self.launches = []

    def check(self):
        if self.fixture.case_error:
            raise RuntimeError('synthetic case deadline')

    def launch(self, exe, args, name, cwd):
        self.launches.append((exe, args, name, cwd))
        self.fixture.events.append('launch')
        self.fixture.assertIs(self.fixture.adapter.reuse, self.fixture.reuse)
        if self.fixture.launch_error:
            raise RuntimeError('synthetic uncertain spawn')
        return None if self.fixture.invalid_owner else self.fixture.process

    def join(self, proc, seconds):
        self.fixture.events.append(('join', seconds))
        self.fixture.assertIs(proc, self.fixture.process)
        if self.fixture.join_error:
            raise RuntimeError('synthetic join timeout')
        proc.exit_code = self.fixture.exit_code
        return proc.exit_code


class AcquisitionTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(MODULE, 'OBS acquisition adapter is missing')
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.install = self.root / 'existing-obs'
        self.exe = self.install / 'bin/64bit/obs64.exe'
        self.exe.parent.mkdir(parents=True)
        self.exe.write_bytes(b'synthetic fixture; never executed')
        self.sha = hashlib.sha256(self.exe.read_bytes()).hexdigest().upper()
        self.config = self.install / 'config/obs-studio'
        self.config.mkdir(parents=True)
        (self.config / 'prior.ini').write_text('retained configuration')
        self.run = self.root / 'new-run'
        self.run.mkdir()
        self.events, self.clients, self.connections = [], [], []
        self.clock = Clock()
        self.process = Process()
        self.rpc_delay = 0
        self.case_error = self.launch_error = self.invalid_owner = False
        self.socket_error = self.join_error = self.close_error = False
        self.exit_code = 0
        self.fail_kind = None
        self.reuse = REUSE.ReusedObs(self.install, self.sha, idle_check=lambda: None)
        self.addCleanup(self.release_fixture)
        self.case = Case(self)
        self.adapter = self.make_adapter()

    def release_fixture(self):
        # Fake callbacks never spawn. This closes only the test's kernel lease.
        if self.reuse.lease is not None:
            self.reuse.lease.close()
            self.reuse.lease = None

    def connect(self, password, *, timeout):
        self.assertEqual(password, self.reuse.password)
        self.assertIs(self.adapter.process, self.process)
        self.assertIsNotNone(self.reuse.lease)
        self.connections.append(timeout)
        client = Client(self, self.fail_kind)
        self.clients.append(client)
        return client

    def close_window(self, proc):
        self.assertIs(proc, self.process)
        self.events.append('window.close')
        if self.close_error:
            raise RuntimeError('synthetic normal close failure')

    def make_adapter(self, connect=None):
        return MODULE.ObsAcquisition(
            self.case, connect or self.connect, self.close_window,
            reuse=self.reuse, clock=self.clock, sleep=self.clock.sleep)

    def test_default_session_keeps_canonical_target_guard(self):
        adapter = MODULE.ObsAcquisition(self.case, self.connect, self.close_window)
        self.assertEqual(adapter.reuse.root, MODULE.REUSE.CANONICAL_ROOT.absolute())
        self.assertIs(adapter.reuse.idle_check, MODULE.REUSE.windows_idle_check)

    def test_default_guard_refuses_host_before_any_process_or_config_operation(self):
        adapter = MODULE.ObsAcquisition(self.case, self.connect, self.close_window)
        with mock.patch.dict('os.environ', {'COMPUTERNAME': 'NOT-THE-AUTHORIZED-TARGET'}), \
                mock.patch.object(MODULE.REUSE.subprocess, 'run',
                                  side_effect=AssertionError('native process query')):
            with self.assertRaisesRegex(RuntimeError, 'separate authorized target'):
                adapter.open()
        self.assertEqual(self.case.launches, [])
        self.assertEqual(list(self.run.iterdir()), [])

    def test_exact_configuration_uses_real_helper_without_application_copy(self):
        with mock.patch('shutil.copytree', side_effect=AssertionError('application copy')):
            client = self.adapter.open()
        self.assertIs(client, self.clients[0])
        self.assertEqual(self.case.launches[0][0], self.exe)
        self.assertEqual(self.case.launches[0][3], self.exe.parent)
        self.assertNotIn('--multi', self.case.launches[0][1])
        self.assertFalse((self.run / 'obs-portable').exists())
        self.assertFalse(any(p.suffix in ('.exe', '.dll') for p in self.run.rglob('*')))
        self.assertEqual([(kind, data) for kind, data, _ in client.calls], [
            ('GetVersion', None),
            ('CreateInput', {'sceneName': 'Isolated', 'inputName': 'DawnOutput',
                             'inputKind': 'wasapi_output_capture', 'inputSettings': {
                                 'device_id': '{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}',
                                 'use_device_timing': True}, 'sceneItemEnabled': True}),
            ('SetInputAudioTracks', {'inputName': 'DawnOutput', 'inputAudioTracks': {
                str(i): i == 1 for i in range(1, 7)}}),
            ('SetInputAudioMonitorType', {'inputName': 'DawnOutput',
                                        'monitorType': 'OBS_MONITORING_TYPE_NONE'}),
            ('SetInputAudioSyncOffset', {'inputName': 'DawnOutput', 'inputAudioSyncOffset': 0}),
            ('SetInputVolume', {'inputName': 'DawnOutput', 'inputVolumeMul': 1.0})])
        self.assertTrue(all(0 < timeout <= 5 for timeout in self.connections))
        self.assertTrue(all(0 < timeout <= 5 for _, _, timeout in client.calls))
        self.assertNotIn('StartRecord', self.events)
        self.assertNotIn('StopRecord', self.events)

    def test_one_open_attempt_even_after_success(self):
        self.adapter.open()
        with self.assertRaises(RuntimeError):
            self.adapter.open()
        self.assertEqual(len(self.case.launches), 1)

    def test_bad_pin_never_launches_or_retries_prepare(self):
        self.exe.write_bytes(b'wrong pin')
        with self.assertRaises(ValueError):
            self.adapter.open()
        with self.assertRaises(RuntimeError):
            self.adapter.open()
        self.assertEqual(self.case.launches, [])

    def test_prior_adapter_receipt_refuses_before_configuration_or_launch(self):
        receipt = self.run / 'obs-acquisition-result.json'
        receipt.write_bytes(b'prior immutable evidence')
        with self.assertRaisesRegex(ValueError, 'prior OBS acquisition'):
            self.adapter.open()
        self.assertEqual(self.case.launches, [])
        self.assertIsNone(self.reuse.lease)
        self.assertFalse((self.run / 'obs-config-before.zip').exists())
        self.assertEqual(receipt.read_bytes(), b'prior immutable evidence')

    def test_known_owner_and_client_survive_configuration_exception(self):
        self.fail_kind = 'SetInputAudioTracks'
        with self.assertRaises(RuntimeError):
            self.adapter.open()
        self.assertIs(self.adapter.process, self.process)
        self.assertIs(self.reuse.process, self.process)
        self.assertIs(self.adapter.client, self.clients[0])
        self.assertIsNotNone(self.reuse.lease)
        result = self.adapter.finish(recording_inactive=True)
        self.assertFalse(result['boundary_complete'])
        self.assertFalse(result['acquisition_accepted'])
        self.assertEqual(len(self.case.launches), 1)
        self.assertIsNone(self.reuse.lease)

    def test_readiness_retries_only_connections_and_closes_failed_client(self):
        def connect(password, *, timeout):
            client = self.connect(password, timeout=timeout)
            if len(self.clients) == 1:
                client.fail = 'GetVersion'
            return client
        self.adapter = self.make_adapter(connect)
        self.adapter.open()
        self.assertEqual(len(self.case.launches), 1)
        self.assertEqual(len(self.clients), 2)
        self.assertTrue(self.clients[0].closed)

    def test_readiness_has_elapsed_and_attempt_bounds(self):
        self.fail_kind = 'GetVersion'
        with self.assertRaisesRegex(RuntimeError, 'readiness'):
            self.adapter.open()
        self.assertLessEqual(len(self.connections), 140)
        self.assertLessEqual(self.clock.now, 35)
        self.assertTrue(all(client.closed for client in self.clients))
        self.assertEqual(len(self.case.launches), 1)

    def test_frozen_clock_cannot_make_readiness_loop_infinite(self):
        self.fail_kind = 'GetVersion'
        self.adapter = MODULE.ObsAcquisition(
            self.case, self.connect, self.close_window, reuse=self.reuse,
            clock=self.clock, sleep=lambda _seconds: None)
        with self.assertRaisesRegex(RuntimeError, 'readiness'):
            self.adapter.open()
        self.assertEqual(len(self.connections), 140)

    def test_late_success_is_not_readiness(self):
        def connect(password, *, timeout):
            client = self.connect(password, timeout=timeout)
            self.clock.now += 36
            return client
        self.adapter = self.make_adapter(connect)
        with self.assertRaisesRegex(RuntimeError, 'readiness'):
            self.adapter.open()
        self.assertNotIn('CreateInput', self.events)
        self.assertTrue(self.clients[0].closed)

    def test_configuration_has_shared_deadline_without_rpc_retry(self):
        self.rpc_delay = 3
        with self.assertRaisesRegex(RuntimeError, 'configuration'):
            self.adapter.open()
        self.assertEqual(self.events.count('CreateInput'), 1)
        self.assertLessEqual(len(self.clients[0].calls), 5)
        self.assertNotIn('SetInputVolume', self.events)

    def test_case_deadline_refuses_before_launch(self):
        self.case_error = True
        with self.assertRaisesRegex(RuntimeError, 'case deadline'):
            self.adapter.open()
        self.assertEqual(self.case.launches, [])

    def test_exited_owner_during_readiness_cannot_configure_or_respawn(self):
        def connect(password, *, timeout):
            client = self.connect(password, timeout=timeout)
            self.process.exit_code = 0
            return client
        self.adapter = self.make_adapter(connect)
        with self.assertRaisesRegex(RuntimeError, 'exited'):
            self.adapter.open()
        self.assertNotIn('CreateInput', self.events)
        self.assertEqual(len(self.case.launches), 1)

    def test_connection_error_retries_without_launch_retry(self):
        attempts = []
        def connect(password, *, timeout):
            attempts.append(timeout)
            if len(attempts) == 1:
                raise OSError('synthetic connect timeout')
            return self.connect(password, timeout=timeout)
        self.adapter = self.make_adapter(connect)
        self.adapter.open()
        self.assertEqual(len(attempts), 2)
        self.assertEqual(len(self.case.launches), 1)

    def test_uncertain_spawn_keeps_lease_and_never_retries(self):
        self.launch_error = True
        with self.assertRaisesRegex(RuntimeError, 'uncertain spawn'):
            self.adapter.open()
        with self.assertRaises(RuntimeError):
            self.adapter.open()
        with self.assertRaises(RuntimeError):
            self.adapter.finish(recording_inactive=True)
        self.assertEqual(len(self.case.launches), 1)
        self.assertIsNotNone(self.reuse.lease)
        self.assertFalse((self.run / 'obs-reuse-result.json').exists())

    def test_invalid_owner_keeps_lease_and_has_no_connection(self):
        self.invalid_owner = True
        with self.assertRaises(RuntimeError):
            self.adapter.open()
        with self.assertRaises(RuntimeError):
            self.adapter.finish(recording_inactive=True)
        self.assertEqual(self.connections, [])
        self.assertIsNotNone(self.reuse.lease)

    def test_graceful_order_archives_then_releases_without_acceptance(self):
        self.adapter.open()
        original_finish = self.reuse.finish
        def finish():
            self.events.append('finish')
            self.assertIsNotNone(self.reuse.lease)
            self.assertTrue((self.run / 'obs-config-before.zip').is_file())
            return original_finish()
        with mock.patch.object(self.reuse, 'finish', side_effect=finish):
            result = self.adapter.finish(recording_inactive=True)
        self.assertLess(self.events.index('socket.close'), self.events.index('window.close'))
        self.assertLess(self.events.index('window.close'), self.events.index(('join', 12)))
        self.assertLess(self.events.index(('join', 12)), self.events.index('finish'))
        self.assertTrue(result['boundary_complete'])
        self.assertFalse(result['qualified'])
        self.assertFalse(result['acquisition_accepted'])
        self.assertEqual(result['obs']['obs_exit_code'], 0)
        self.assertIsNone(self.reuse.lease)
        self.assertEqual(json.loads((self.run / 'obs-acquisition-result.json').read_text()), result)

    def test_unconfirmed_inactive_never_closes_or_releases_live_owner(self):
        self.adapter.open()
        for value in (False, None, 1, 'false'):
            with self.subTest(value=value), self.assertRaises(RuntimeError):
                self.adapter.finish(recording_inactive=value)
        self.assertNotIn('socket.close', self.events)
        self.assertNotIn('window.close', self.events)
        self.assertIsNotNone(self.reuse.lease)

    def test_socket_failure_prevents_normal_close_and_keeps_lease(self):
        self.adapter.open()
        self.socket_error = True
        with self.assertRaisesRegex(RuntimeError, 'socket close'):
            self.adapter.finish(recording_inactive=True)
        self.assertNotIn('window.close', self.events)
        self.assertIsNotNone(self.reuse.lease)

    def test_terminal_owner_still_requires_returned_socket_to_be_closed(self):
        self.adapter.open()
        client = self.adapter.client
        self.socket_error = True
        with self.assertRaises(RuntimeError):
            self.adapter.finish(recording_inactive=True)
        self.process.exit_code = 0
        with self.assertRaisesRegex(RuntimeError, 'socket close'):
            self.adapter.finish(recording_inactive=True)
        self.assertIsNotNone(self.reuse.lease)
        self.socket_error = False
        result = self.adapter.finish(recording_inactive=True)
        self.assertTrue(client.closed)
        self.assertFalse(result['boundary_complete'])
        self.assertIsNone(self.reuse.lease)

    def test_join_timeout_keeps_lease_and_does_not_reissue_normal_close(self):
        self.adapter.open()
        self.join_error = True
        with self.assertRaisesRegex(RuntimeError, 'join timeout'):
            self.adapter.finish(recording_inactive=True)
        with self.assertRaises(RuntimeError):
            self.adapter.finish(recording_inactive=True)
        self.assertEqual(self.events.count('window.close'), 1)
        self.assertEqual(self.events.count(('join', 12)), 1)
        self.assertIsNotNone(self.reuse.lease)

    def test_normal_close_failure_preserves_owner_and_no_join_or_retry(self):
        self.adapter.open()
        self.close_error = True
        with self.assertRaisesRegex(RuntimeError, 'normal close'):
            self.adapter.finish(recording_inactive=True)
        with self.assertRaises(RuntimeError):
            self.adapter.finish(recording_inactive=True)
        self.assertEqual(self.events.count('window.close'), 1)
        self.assertNotIn(('join', 12), self.events)
        self.assertIsNotNone(self.reuse.lease)

    def test_forced_cleanup_after_timeout_cannot_erase_failure(self):
        self.adapter.open()
        self.join_error = True
        with self.assertRaises(RuntimeError):
            self.adapter.finish(recording_inactive=True)
        self.process.exit_code = 0
        result = self.adapter.finish(recording_inactive=True)
        self.assertFalse(result['boundary_complete'])
        self.assertTrue(result['errors'])
        self.assertEqual(self.events.count('window.close'), 1)
        self.assertIsNone(self.reuse.lease)

    def test_already_exited_owner_is_archived_but_not_graceful_success(self):
        self.adapter.open()
        self.process.exit_code = 0
        result = self.adapter.finish(recording_inactive=True)
        self.assertFalse(result['boundary_complete'])
        self.assertNotIn('window.close', self.events)
        self.assertTrue(result['obs']['configuration_archived'])

    def test_nonzero_exit_archives_but_never_passes_boundary(self):
        self.adapter.open()
        self.exit_code = 7
        result = self.adapter.finish(recording_inactive=True)
        self.assertFalse(result['boundary_complete'])
        self.assertFalse(result['obs']['obs_exit_success'])
        self.assertIsNone(self.reuse.lease)

    def test_join_reply_without_actual_exit_cannot_release_or_pass(self):
        self.adapter.open()
        with mock.patch.object(self.case, 'join', return_value=0):
            with self.assertRaisesRegex(RuntimeError, 'join did not confirm'):
                self.adapter.finish(recording_inactive=True)
        self.assertIsNotNone(self.reuse.lease)
        self.assertFalse((self.run / 'obs-reuse-result.json').exists())

    def test_setup_failure_never_appears_successful_after_normal_close(self):
        for kind in ('CreateInput', 'SetInputAudioMonitorType',
                     'SetInputAudioSyncOffset', 'SetInputVolume'):
            with self.subTest(kind=kind):
                # Each subcase owns separate real files and a real lease.
                fixture = AcquisitionTests('test_one_open_attempt_even_after_success')
                fixture.setUp()
                try:
                    fixture.fail_kind = kind
                    with self.assertRaises(RuntimeError):
                        fixture.adapter.open()
                    result = fixture.adapter.finish(recording_inactive=True)
                    self.assertFalse(result['boundary_complete'])
                    self.assertFalse(result['acquisition_accepted'])
                    self.assertEqual(fixture.events.count(kind), 1)
                    self.assertIsNone(fixture.reuse.lease)
                finally:
                    fixture.doCleanups()

    def test_configuration_archive_failure_keeps_lease_and_failure(self):
        self.adapter.open()
        (self.config / 'forbidden.exe').write_bytes(b'not configuration')
        with self.assertRaises(ValueError):
            self.adapter.finish(recording_inactive=True)
        self.assertIsNotNone(self.reuse.lease)
        self.assertFalse((self.run / 'obs-acquisition-result.json').exists())
        (self.config / 'forbidden.exe').unlink()
        result = self.adapter.finish(recording_inactive=True)
        self.assertFalse(result['boundary_complete'])
        self.assertTrue(result['errors'])
        self.assertIsNone(self.reuse.lease)

    def test_second_route_cannot_prepare_until_first_session_finishes(self):
        self.adapter.open()
        second_root = self.root / 'second-route'
        second_root.mkdir()
        other = REUSE.ReusedObs(self.install, self.sha, idle_check=lambda: None)
        with self.assertRaises((OSError, RuntimeError)):
            other.prepare(second_root)
        self.assertIsNone(other.lease)
        self.adapter.finish(recording_inactive=True)
        other.prepare(second_root)
        other.release()

    def test_duplicate_finish_cannot_overwrite_receipt(self):
        self.adapter.open()
        self.adapter.finish(recording_inactive=True)
        before = (self.run / 'obs-acquisition-result.json').read_bytes()
        with self.assertRaises(RuntimeError):
            self.adapter.finish(recording_inactive=True)
        self.assertEqual((self.run / 'obs-acquisition-result.json').read_bytes(), before)

    def test_recording_transition_witness_drives_real_boundary_close(self):
        client = self.adapter.open()
        original = client.req
        statuses = iter((True, False))
        def request(kind, data=None, *, timeout):
            result = original(kind, data, timeout=timeout)
            return {'outputActive': next(statuses)} if kind == 'GetRecordStatus' else result
        client.req = request
        recording = RECORDING.ObsRecording(
            self.case, client, self.process, clock=self.clock, sleep=self.clock.sleep)
        recording.start()
        with self.assertRaises(RuntimeError):
            self.adapter.finish(recording_inactive=recording.recording_inactive)
        recording.stop()
        result = self.adapter.finish(recording_inactive=recording.recording_inactive)
        self.assertLess(self.events.index('StopRecord'), self.events.index('socket.close'))
        self.assertLess(self.events.index('socket.close'), self.events.index('window.close'))
        # The earlier attempted close during active recording remains a failure.
        self.assertFalse(result['boundary_complete'])
        self.assertTrue(recording.receipt()['recording_complete'])
        self.assertIsNone(self.reuse.lease)

    def test_lost_start_reply_prevents_boundary_close_until_full_observed_cycle(self):
        client = self.adapter.open()
        original = client.req
        statuses = iter((False, True, False))
        def request(kind, data=None, *, timeout):
            original(kind, data, timeout=timeout)
            if kind == 'StartRecord':
                raise OSError('lost dispatched start reply')
            return {'outputActive': next(statuses)} if kind == 'GetRecordStatus' else {}
        client.req = request
        recording = RECORDING.ObsRecording(
            self.case, client, self.process, clock=self.clock, sleep=self.clock.sleep)
        with self.assertRaises(OSError):
            recording.start()
        self.assertFalse(recording.recording_inactive)
        self.assertIsNotNone(self.reuse.lease)
        self.assertNotIn('window.close', self.events)
        recording.stop()
        result = self.adapter.finish(recording_inactive=recording.recording_inactive)
        self.assertTrue(result['boundary_complete'])
        self.assertFalse(recording.receipt()['recording_complete'])
        self.assertFalse(result['acquisition_accepted'])
        self.assertEqual(self.events.count('StartRecord'), 1)
        self.assertEqual(self.events.count('StopRecord'), 1)


if __name__ == '__main__':
    unittest.main()
