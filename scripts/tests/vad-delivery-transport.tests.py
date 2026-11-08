# SPDX-License-Identifier: MIT
"""Retained SSH transport tests: fake boundaries/harmless children, no network."""
import base64
import hashlib
import importlib.util
import io
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.dont_write_bytecode = True
SOURCE = Path(__file__).resolve().parents[1] / 'vad-delivery-transport.py'
SPEC = importlib.util.spec_from_file_location('delivery_transport_tests', SOURCE) if SOURCE.exists() else None
M = importlib.util.module_from_spec(SPEC) if SPEC else None
if SPEC:
    SPEC.loader.exec_module(M)

REMOTE_RUN = r'D:\MoirenTest\20261108-removal-test'
REMOTE_FILE = REMOTE_RUN + '.zip'
FINALIZER = r'D:\MoirenTest\reviewed\delivery-finalize.ps1'
SHA = 'A' * 64
REAL_POPEN = subprocess.Popen
DEPENDENCIES = [{'Path': 'D:\\MoirenTest\\reviewed\\' + name,
                 'Bytes': 12, 'Sha256': SHA}
                for name in ('closed-export.ps1', 'finite-tool.ps1')]


class Input(io.BytesIO):
    def close(self):
        self.captured = self.getvalue()
        super().close()


class Process:
    def __init__(self, stdout=b'closed archive', stderr=b'', code=0):
        self.pid = 4312
        self.stdout, self.stderr, self.stdin = io.BytesIO(stdout), io.BytesIO(stderr), Input()
        self.code, self.polls = code, 0

    def poll(self):
        self.polls += 1
        return self.code

    def kill(self):
        raise AssertionError('Transport must never kill an owner')

    terminate = kill


class Availability(unittest.TestCase):
    def test_transport_source_exists(self):
        self.assertIsNotNone(M, 'Retained delivery transport is not implemented')


@unittest.skipUnless(M, 'Source not implemented yet')
class TransportTests(unittest.TestCase):
    def setUp(self):
        self.no_launch = mock.patch.object(M.subprocess, 'Popen',
                                          side_effect=AssertionError('Unmocked process launch refused'))
        self.no_launch.start()
        self.addCleanup(self.no_launch.stop)
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.log = self.root / 'attempt'
        self.destination = self.root / 'download.zip'
        self.calls = []

    def tearDown(self):
        self.temp.cleanup()

    def factory(self, process):
        def start(argv, **kwargs):
            self.assertTrue(Path(str(self.log) + '.intent.json').is_file(), 'Intent must precede Popen')
            self.calls.append((argv, kwargs))
            return process
        return start

    def download(self, **overrides):
        options = dict(user_reboot_confirmed=True, cap=1000, log_prefix=self.log)
        options.update(overrides)
        return M.download_file(REMOTE_FILE, self.destination, **options)

    def finalize(self, **overrides):
        options = dict(user_reboot_confirmed=True, log_prefix=self.log,
                       dependency_pins=DEPENDENCIES)
        options.update(overrides)
        return M.finalize_delivery(REMOTE_RUN, b'{"Meaning":"HOST_ARCHIVE_DELIVERY_VERIFIED"}',
                                   SHA, FINALIZER, SHA, **options)

    def script(self):
        argv = self.calls[0][0]
        self.assertEqual(argv[0:2], ['ssh', '-T'])
        self.assertEqual(argv[-2], 'moiren-lan')
        self.assertIn('StrictHostKeyChecking=yes', argv)
        self.assertIn('BatchMode=yes', argv)
        self.assertIn('ConnectTimeout=10', argv)
        encoded = argv[-1].rsplit(' ', 1)[1]
        return base64.b64decode(encoded).decode('utf-16-le')

    def test_unconfirmed_reboot_precedes_directory_and_file_creation(self):
        with mock.patch.object(M.subprocess, 'Popen', side_effect=AssertionError('No launch')):
            with self.assertRaises(M.DeliveryTransportError):
                self.download(user_reboot_confirmed=False, log_prefix=self.root / 'absent' / 'log')
            with self.assertRaises(M.DeliveryTransportError):
                self.finalize(user_reboot_confirmed=False)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_confirmation_is_exact_boolean_not_truthy(self):
        with self.assertRaises(M.DeliveryTransportError):
            self.download(user_reboot_confirmed='true')
        self.assertEqual(list(self.root.iterdir()), [])

    def test_download_retains_single_owner_and_hashes_binary_stdout(self):
        data = b'\x00\xffZIP\r\n' + '\u4e2d\u6587'.encode()
        p = Process(data)
        with mock.patch.object(M.subprocess, 'Popen', self.factory(p)):
            result = self.download()
        self.assertEqual(result, {'Path': str(self.destination), 'Bytes': len(data),
                                  'Sha256': hashlib.sha256(data).hexdigest().upper()})
        self.assertEqual(self.destination.read_bytes(), data)
        self.assertEqual(len(self.calls), 1)
        receipt = json.loads(Path(str(self.log) + '.transport.json').read_text())
        self.assertEqual(receipt['Pid'], p.pid)
        self.assertTrue(receipt['ExitedConfirmed'])
        self.assertFalse(receipt['TimedOut'])
        self.assertIn('LAPTOP-NP49RG6H', self.script())
        self.assertIn('S-1-5-21-3080296863-1242338503-1346362208-1001', self.script())
        self.assertIn('ReparsePoint', self.script())
        self.assertIn('OpenStandardOutput', self.script())
        self.assertNotIn('ReadToEnd', self.script())

    def test_stdout_cap_discards_tail_but_drains_and_preserves_failure(self):
        p = Process(b'X' * 10000)
        with mock.patch.object(M.subprocess, 'Popen', self.factory(p)):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                self.download(cap=31)
        self.assertEqual(self.destination.stat().st_size, 31)
        self.assertTrue(raised.exception.owner.exited_confirmed)
        self.assertEqual(len(self.calls), 1)

    def test_stderr_cap_is_shared_finite_drain_not_unbounded_capture(self):
        p = Process(stderr=b'E' * (128 * 1024))
        with mock.patch.object(M.subprocess, 'Popen', self.factory(p)):
            with self.assertRaises(M.DeliveryTransportError):
                self.download()
        self.assertEqual(Path(str(self.log) + '.stderr.log').stat().st_size, 65536)
        self.assertEqual(len(self.calls), 1)

    def test_nonzero_exit_is_sticky_and_partial_destination_remains(self):
        with mock.patch.object(M.subprocess, 'Popen', self.factory(Process(b'partial', code=7))):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                self.download()
        self.assertEqual(raised.exception.owner.exit_code, 7)
        self.assertEqual(self.destination.read_bytes(), b'partial')

    def test_outer_entry_deadline_is_per_owner_without_changing_delivery_default(self):
        p = Process(b'closed output')
        with mock.patch.object(M.subprocess, 'Popen', self.factory(p)):
            result = M._run('Write-Output original', self.destination, 1000,
                            self.log, {'Operation': 'DiagnosticEntry'}, b'',
                            M._pin_stream, timeout_seconds=600.0)
        self.assertEqual(result['Bytes'], len(b'closed output'))
        intent = json.loads(Path(str(self.log) + '.intent.json').read_bytes())
        self.assertEqual(intent['DeadlineSeconds'], 600.0)
        self.assertEqual(M.TIMEOUT_SECONDS, 120.0)

    def test_invalid_outer_deadlines_refuse_before_attempt_files(self):
        for seconds in (False, 0, -1, 601, math.inf, math.nan, '600'):
            with self.subTest(seconds=seconds), self.assertRaises(M.DeliveryTransportError):
                M._run('unused', self.destination, 1000, self.log,
                        {'Operation': 'DiagnosticEntry'}, b'', M._pin_stream,
                        timeout_seconds=seconds)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_owner_deadline_is_frozen_and_diagnostic_does_not_relax_native_gates(self):
        owner = M._Owner(M._Output(io.BytesIO(), 1), M._Output(io.BytesIO(), 1),
                         io.BytesIO(), io.BytesIO(), 600.0)
        owner.started = 0.0
        with mock.patch.object(M.time, 'monotonic', return_value=121.0), \
                mock.patch.object(M, 'TIMEOUT_SECONDS', 1.0):
            owner._deadline()
        self.assertFalse(owner.timed_out)
        with mock.patch.object(M.time, 'monotonic', return_value=601.0):
            owner._deadline()
        self.assertTrue(owner.timed_out)
        self.assertIn('deadline', owner.errors)
        controller = SOURCE.with_name('vad-removal') / 'diagnostic-controller.ps1'
        self.assertIn('DeadlineMilliseconds=[long]240000', controller.read_text())

    def test_transient_pump_read_failure_drains_original_pipe_and_stays_failed(self):
        class Flaky(io.BytesIO):
            def __init__(self):
                super().__init__(b'original output')
                self.first = True

            def read(self, count):
                if self.first:
                    self.first = False
                    raise OSError('transient pipe fault')
                return super().read(count)

        p = Process()
        p.stdout = Flaky()
        with mock.patch.object(M.subprocess, 'Popen', self.factory(p)):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                self.download()
        self.assertTrue(raised.exception.owner.exited_confirmed)
        self.assertTrue(raised.exception.owner.stdout.eof)
        self.assertIn('stdout_read', raised.exception.owner.errors)
        self.assertEqual(self.destination.read_bytes(), b'original output')
        self.assertEqual(len(self.calls), 1)

    def test_output_write_failure_drains_and_waits_for_same_child(self):
        p = Process(b'x' * 10000)
        write = M._write_all

        # fdopen streams expose integer names. Wrap only the selected original
        # stream so the failure targets payload writes, not intent/receipt.
        open_file = M._new_file
        payload_stream = []

        def create(path):
            stream = open_file(path)
            if path == self.destination:
                payload_stream.append(stream)
            return stream

        def fault(stream, data):
            if payload_stream and stream is payload_stream[0]:
                raise OSError('output storage failure')
            return write(stream, data)

        with mock.patch.object(M, '_new_file', create), mock.patch.object(M, '_write_all', fault), \
                mock.patch.object(M.subprocess, 'Popen', self.factory(p)):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                self.download()
        self.assertTrue(raised.exception.owner.exited_confirmed)
        self.assertTrue(raised.exception.owner.stdout.eof)
        self.assertIn('stdout_write', raised.exception.owner.errors)
        self.assertEqual(len(self.calls), 1)

    def test_stdin_write_failure_is_sticky_after_original_child_settles(self):
        class Broken(Input):
            def write(self, _):
                raise BrokenPipeError('input refused')

        p = Process(b'{}')
        p.stdin = Broken()
        with mock.patch.object(M.subprocess, 'Popen', self.factory(p)):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                self.finalize()
        self.assertTrue(raised.exception.owner.exited_confirmed)
        self.assertIn('stdin_write', raised.exception.owner.errors)

    def test_validation_finishing_after_deadline_cannot_be_success(self):
        pin = M._pin_stream

        def slow_pin(path, stream):
            time.sleep(0.06)
            return pin(path, stream)

        with mock.patch.object(M, 'TIMEOUT_SECONDS', 0.03), mock.patch.object(M, '_pin_stream', slow_pin), \
                mock.patch.object(M.subprocess, 'Popen', self.factory(Process())):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                self.download()
        self.assertTrue(raised.exception.owner.timed_out)
        self.assertTrue(raised.exception.owner.exited_confirmed)

    def test_intent_records_only_pending_launch_until_popen_is_attempted(self):
        with mock.patch.object(M.subprocess, 'Popen', self.factory(Process())):
            self.download()
        intent = json.loads(Path(str(self.log) + '.intent.json').read_bytes())
        self.assertEqual(intent['Phase'], 'BeforePopen')
        self.assertFalse(intent['LaunchAttempted'])

    def test_late_receipt_flush_cannot_promote_deadline_success(self):
        durable = M._durable
        calls = 0

        def slow_receipt(stream):
            nonlocal calls
            calls += 1
            if calls == 4:
                time.sleep(0.07)
            return durable(stream)

        with mock.patch.object(M, 'TIMEOUT_SECONDS', 0.04), mock.patch.object(M, '_durable', slow_receipt), \
                mock.patch.object(M.subprocess, 'Popen', self.factory(Process())):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                self.download()
        self.assertTrue(raised.exception.owner.timed_out)
        self.assertFalse(raised.exception.owner.success)

    def test_popen_failure_retains_unknown_attempt_without_retry(self):
        def fail(argv, **kwargs):
            self.assertTrue(Path(str(self.log) + '.intent.json').exists())
            self.calls.append(argv)
            raise OSError('ambiguous launch')
        with mock.patch.object(M.subprocess, 'Popen', fail):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                self.download()
        self.assertTrue(raised.exception.owner.launch_attempted)
        self.assertFalse(raised.exception.owner.exited_confirmed)
        self.assertEqual(len(self.calls), 1)
        self.assertTrue(self.destination.exists())

    def test_observation_error_exposes_exact_pending_process(self):
        p = Process()
        p.poll = mock.Mock(side_effect=OSError('unknown status'))
        with mock.patch.object(M.subprocess, 'Popen', self.factory(p)):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                self.download()
        owner = raised.exception.owner
        self.assertIs(owner.process, p)
        self.assertFalse(owner.exited_confirmed)
        p.poll = lambda: 0
        owner.observe()
        self.assertTrue(owner.exited_confirmed)
        self.assertFalse(owner.success)

    def test_transient_second_third_thread_start_recovers_same_threads_and_child(self):
        original_start = M.threading.Thread.start
        for failed_index in (2, 3):
            with self.subTest(failed_index=failed_index):
                self.log = self.root / ('thread-' + str(failed_index))
                self.destination = self.root / ('thread-' + str(failed_index) + '.zip')
                starts, processes = [], []

                def start_thread(thread):
                    starts.append(thread)
                    if len(starts) == failed_index:
                        raise RuntimeError('Known pre-dispatch thread setup fault')
                    return original_start(thread)

                def child(argv, **kwargs):
                    self.calls.append((argv, kwargs))
                    p = REAL_POPEN([sys.executable, '-c',
                        'import sys;sys.stderr.buffer.write(b"e"*200000);'
                        'sys.stderr.buffer.flush();sys.stdin.buffer.read();'
                        'sys.stdout.buffer.write(b"settled")'], **kwargs)
                    processes.append(p)
                    return p

                with mock.patch.object(M.threading.Thread, 'start', start_thread), \
                        mock.patch.object(M.subprocess, 'Popen', child):
                    with self.assertRaises(M.DeliveryTransportError) as raised:
                        self.download()
                owner = raised.exception.owner
                try:
                    self.assertIs(owner.process, processes[0])
                    retained = tuple(owner.threads)
                    # Re-observation is permitted to start only the original
                    # known-not-started thread, never a new process or pump.
                    owner.observe()
                    self.assertEqual(tuple(owner.threads), retained)
                    self.assertTrue(owner.exited_confirmed)
                    self.assertTrue(owner.closed)
                    self.assertFalse(owner.success)
                    self.assertEqual(owner.exit_code, 0)
                    self.assertEqual(len(processes), 1)
                    self.assertEqual(self.destination.read_bytes(), b'settled')
                finally:
                    # RED cleanup supplies only the missing harmless pumps so
                    # the very same child can see stdin EOF and exit naturally.
                    # Never terminate/kill even a deliberately failing fixture.
                    if not owner.closed:
                        active = {thread._target for thread in owner.threads
                                  if thread.ident is not None and thread.is_alive()}
                        for thread in owner.threads:
                            if thread.ident is None and not thread.is_alive():
                                original_start(thread)
                        if owner._input not in active and len(owner.threads) < 3:
                            owner._input(b'')
                        for thread in owner.threads:
                            thread.join()
                        processes[0].wait()
                        owner.exited_confirmed, owner.exit_code = True, 0
                        owner._close_files()

    def test_ambiguous_start_after_thread_dispatch_never_restarts_it(self):
        original_start = M.threading.Thread.start
        starts = []

        def start_thread(thread):
            starts.append(thread)
            original_start(thread)
            if len(starts) == 2:
                raise RuntimeError('Thread dispatched; result observation lost')

        with mock.patch.object(M.threading.Thread, 'start', start_thread), \
                mock.patch.object(M.subprocess, 'Popen', self.factory(Process())):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                self.download()
        owner = raised.exception.owner
        try:
            retained = tuple(owner.threads)
            owner.observe()
            self.assertTrue(owner.closed)
            self.assertFalse(owner.success)
            self.assertEqual(tuple(owner.threads), retained)
            self.assertEqual(len({id(thread) for thread in starts}), len(starts),
                             'An already dispatched pump was started twice')
        finally:
            if not owner.closed:
                for thread in owner.threads:
                    if thread.ident is None and not thread.is_alive():
                        original_start(thread)
                if len(owner.threads) < 3:
                    owner._input(b'')
                owner.exited_confirmed, owner.exit_code = True, 0
                owner._close_files()

    def test_transient_thread_constructor_failure_preserves_and_completes_original_channels(self):
        original_thread = M.threading.Thread
        calls, processes = [], []

        def construct(*args, **kwargs):
            calls.append(kwargs)
            if len(calls) == 2:
                raise RuntimeError('transient thread constructor failure')
            return original_thread(*args, **kwargs)

        def child(argv, **kwargs):
            p = REAL_POPEN([sys.executable, '-c',
                'import sys;sys.stderr.buffer.write(b"e"*200000);'
                'sys.stderr.buffer.flush();sys.stdin.buffer.read();'
                'sys.stdout.buffer.write(b"settled")'], **kwargs)
            processes.append(p)
            return p

        with mock.patch.object(M.threading, 'Thread', construct), \
                mock.patch.object(M.subprocess, 'Popen', child):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                self.download()
        owner = raised.exception.owner
        try:
            self.assertGreater(len(owner.threads), 0, 'Earlier thread references were lost')
            original_first = owner.threads[0]
            owner.observe()
            self.assertIs(owner.threads[0], original_first)
            self.assertTrue(owner.closed)
            self.assertTrue(owner.exited_confirmed)
            self.assertEqual(owner.exit_code, 0)
            self.assertFalse(owner.success)
            self.assertEqual(len(processes), 1)
            self.assertEqual(self.destination.read_bytes(), b'settled')
        finally:
            if not owner.closed:
                # Mechanical RED constructs but never starts the lost Thread;
                # drain its same harmless child explicitly without kill/retry.
                pumps = [original_thread(target=stream.read, daemon=True)
                         for stream in (processes[0].stdout, processes[0].stderr)]
                for thread in pumps:
                    thread.start()
                if not processes[0].stdin.closed:
                    processes[0].stdin.close()
                processes[0].wait()
                for thread in pumps:
                    thread.join()
                owner.exited_confirmed, owner.exit_code = True, 0
                owner._close_files()

    def test_actual_transient_poll_failure_keeps_owner_until_original_child_exits(self):
        processes = []

        def child(argv, **kwargs):
            p = REAL_POPEN([sys.executable, '-c',
                           'import time;time.sleep(0.12);print("original")'], **kwargs)
            poll = p.poll
            first = True

            def flaky_poll():
                nonlocal first
                if first:
                    first = False
                    raise OSError('transient process observation')
                return poll()

            p.poll = flaky_poll
            processes.append(p)
            return p

        with mock.patch.object(M.subprocess, 'Popen', child):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                self.download()
        owner = raised.exception.owner
        self.assertIs(owner.process, processes[0])
        self.assertFalse(owner.exited_confirmed)
        owner.observe()
        self.assertTrue(owner.exited_confirmed)
        self.assertTrue(owner.closed)
        self.assertEqual(processes[0].poll(), 0)
        self.assertFalse(owner.success)
        self.assertEqual(len(processes), 1)

    def test_harmless_host_executable_drains_pending_owner_before_exiting_failure(self):
        script = r'''
import importlib.util, pathlib, subprocess, sys, time
spec=importlib.util.spec_from_file_location('transport',sys.argv[1])
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
real=subprocess.Popen;children=[]
def child(argv,**kwargs):
    p=real([sys.executable,'-c','import time;time.sleep(0.2);print("natural")'],**kwargs)
    poll=p.poll;first=True
    def observe():
        nonlocal first
        if first:
            first=False;raise OSError('transient poll failure')
        return poll()
    p.poll=observe;children.append(p);return p
m.subprocess.Popen=child
root=pathlib.Path(sys.argv[2]);start=time.monotonic()
try:
    m.download_file(r'D:\MoirenTest\harmless.zip',root/'host.zip',
        user_reboot_confirmed=True,cap=1000,log_prefix=root/'host-attempt')
except m.DeliveryTransportError as error:
    owner=error.owner
    while not owner.closed:
        owner.observe()
    assert len(children)==1 and children[0].poll()==0 and owner.exited_confirmed
    assert not owner.success and time.monotonic()-start>=0.18
    print('original owner settled before sticky host failure')
    raise SystemExit(23)
raise AssertionError('sticky failure lost')
'''
        host = REAL_POPEN([sys.executable, '-B', '-c', script, str(SOURCE), str(self.root)],
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        stdout, stderr = host.communicate()
        self.assertEqual(host.returncode, 23, stderr.decode('utf-8', errors='replace'))
        self.assertIn(b'original owner settled', stdout)

    def test_existing_destination_or_attempt_logs_never_overwrite_or_launch(self):
        self.destination.write_bytes(b'original')
        with mock.patch.object(M.subprocess, 'Popen', side_effect=AssertionError('No launch')):
            with self.assertRaises(M.DeliveryTransportError):
                self.download()
        self.assertEqual(self.destination.read_bytes(), b'original')
        self.destination.unlink()
        Path(str(self.log) + '.intent.json').write_text('original')
        with mock.patch.object(M.subprocess, 'Popen', side_effect=AssertionError('No launch')):
            with self.assertRaises(M.DeliveryTransportError):
                self.download()
        self.assertEqual(Path(str(self.log) + '.intent.json').read_text(), 'original')

    @unittest.skipUnless(os.name == 'nt', 'Windows file/directory share ownership')
    def test_created_output_retains_no_delete_and_no_write_share_until_settled(self):
        denied = []

        def start(argv, **kwargs):
            for operation in (lambda: self.destination.unlink(),
                              lambda: self.destination.write_bytes(b'foreign')):
                try:
                    operation()
                except PermissionError:
                    denied.append(True)
                else:
                    denied.append(False)
            self.calls.append((argv, kwargs))
            return Process()

        with mock.patch.object(M.subprocess, 'Popen', start):
            self.download()
        self.assertEqual(denied, [True, True], 'Original output must exclude writers/deletion')

    def test_linked_output_ancestor_is_refused_before_popen(self):
        real, link = self.root / 'real', self.root / 'link'
        real.mkdir()
        if os.name == 'nt':
            # mklink /J is a harmless local directory fixture, not a target/tool.
            child = REAL_POPEN(['cmd.exe', '/d', '/c', 'mklink', '/J', str(link), str(real)],
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            child.communicate()
            self.assertEqual(child.returncode, 0)
        else:
            link.symlink_to(real, target_is_directory=True)
        self.destination = link / 'download.zip'
        with self.assertRaises(M.DeliveryTransportError):
            self.download()
        self.assertEqual(list(real.iterdir()), [])

    def test_remote_escape_ads_and_noncanonical_names_refuse_before_files(self):
        for path in (r'C:\Windows\x', r'D:\MoirenTest\..\foreign', r'D:\MoirenTest\x:ads',
                     r'D:\MoirenTest\x.\y', 'D:\\MoirenTest\\x\nwhoami', r'\\server\share\x'):
            with self.subTest(path=path), self.assertRaises(M.DeliveryTransportError):
                M.download_file(path, self.destination, user_reboot_confirmed=True, cap=1, log_prefix=self.log)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_invalid_caps_and_hashes_refuse_before_files(self):
        for cap in (0, -1, True, 1.5, 68157441):
            with self.assertRaises(M.DeliveryTransportError):
                self.download(cap=cap)
        with self.assertRaises(M.DeliveryTransportError):
            M.finalize_delivery(REMOTE_RUN, b'{}', 'invalid', FINALIZER, SHA,
                                user_reboot_confirmed=True, log_prefix=self.log,
                                dependency_pins=DEPENDENCIES)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_finalize_sends_exact_receipt_stdin_with_short_fixed_command(self):
        p = Process(b'{"Path":"target-ack","Bytes":4,"Sha256":"AA"}')
        payload = json.dumps({'Meaning': 'HOST_ARCHIVE_DELIVERY_VERIFIED', 'Note': '\u4e2d\u6587' * 5000},
                             ensure_ascii=False).encode()
        with mock.patch.object(M.subprocess, 'Popen', self.factory(p)):
            result = M.finalize_delivery(REMOTE_RUN, payload, SHA, FINALIZER, SHA,
                                         user_reboot_confirmed=True, log_prefix=self.log,
                                         dependency_pins=DEPENDENCIES)
        self.assertEqual(result['Path'], 'target-ack')
        self.assertEqual(p.stdin.captured, payload)
        script = self.script()
        self.assertIn(REMOTE_RUN + '.delivery-host.json', script)
        self.assertIn('CreateNew', script)
        self.assertIn('Confirm-RemovalRunDelivery', script)
        self.assertIn('ExpectedLocalAckSha256', script)
        self.assertIn(hashlib.sha256(payload).hexdigest().upper(), script)
        self.assertNotIn('ReadToEnd', script)
        self.assertLess(len(self.calls[0][0][-1]), 30000)
        self.assertNotIn(payload, Path(str(self.log) + '.intent.json').read_bytes())

    def test_finalize_rejects_malformed_or_nonobject_output_after_settlement(self):
        for data in (b'not-json', b'[]', b'{"x":1,"x":2}', b'{"x":NaN}', b'{"x":1e400}', b'\xff'):
            with self.subTest(data=data):
                self.log = self.root / ('invalid-' + str(len(list(self.root.iterdir()))))
                with mock.patch.object(M.subprocess, 'Popen', self.factory(Process(data))):
                    with self.assertRaises(M.DeliveryTransportError):
                        self.finalize()

    def test_receipt_must_be_bounded_utf8_json_object(self):
        for payload in (b'', b'[]', b'bad', b'\xff', b'{"a":0,"a":1}', b'X' * (1048576 + 1)):
            with self.assertRaises(M.DeliveryTransportError):
                M.finalize_delivery(REMOTE_RUN, payload, SHA, FINALIZER, SHA,
                                    user_reboot_confirmed=True, log_prefix=self.log,
                                    dependency_pins=DEPENDENCIES)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_dependency_pins_exact_membership_types_and_directory(self):
        cases = (None, [], DEPENDENCIES[:1], DEPENDENCIES + DEPENDENCIES[:1],
                 [dict(DEPENDENCIES[0], Bytes=True), DEPENDENCIES[1]],
                 [dict(DEPENDENCIES[0], Sha256='invalid'), DEPENDENCIES[1]],
                 [dict(DEPENDENCIES[0], Path=r'D:\MoirenTest\elsewhere\closed-export.ps1'),
                  DEPENDENCIES[1]])
        for pins in cases:
            with self.subTest(pins=pins), self.assertRaises(M.DeliveryTransportError):
                self.finalize(dependency_pins=pins)
        self.assertEqual(list(self.root.iterdir()), [])

    @unittest.skipUnless(os.name == 'nt', 'Local PS5/PS7 script validation')
    def test_actual_local_script_dependency_mutation_refuses_before_upload(self):
        # Generate the real remote script through the public API, but execute
        # only on an ordinary temporary directory with host/path guards replaced.
        # No SSH, network, actual finalizer, archive or driver operations.
        finalizer = self.root / 'reviewed' / 'delivery-finalize.ps1'
        finalizer.parent.mkdir()
        finalizer.write_text(r'''
. (Join-Path $PSScriptRoot 'closed-export.ps1')
. (Join-Path $PSScriptRoot 'finite-tool.ps1')
function Confirm-RemovalRunDelivery {
 $denied=0
 foreach($name in @('delivery-finalize.ps1','closed-export.ps1','finite-tool.ps1')){
  $path=Join-Path $PSScriptRoot $name
  try{$w=[IO.File]::Open($path,'Open','Write','ReadWrite');$w.Dispose()}catch{$denied++}
  try{[IO.File]::Delete($path)}catch{$denied++}
 }
 if($denied-ne6){throw 'Script handles did not exclude changes'}
 @{Path='test-ack';LockedScripts=$denied}
}
''', encoding='ascii')
        remote_paths, pins = {}, []
        for name in ('closed-export.ps1', 'finite-tool.ps1'):
            path = finalizer.parent / name
            data = b'# harmless dependency\n'
            path.write_bytes(data)
            remote = 'D:\\MoirenTest\\reviewed\\' + name
            pins.append({'Path': remote, 'Bytes': len(data),
                         'Sha256': hashlib.sha256(data).hexdigest().upper()})
            remote_paths[name] = path
        finalizer_sha = hashlib.sha256(finalizer.read_bytes()).hexdigest().upper()
        with mock.patch.object(M.subprocess, 'Popen', self.factory(Process(b'{"Path":"test-ack"}'))):
            M.finalize_delivery(REMOTE_RUN, b'{}', SHA, FINALIZER, finalizer_sha,
                                dependency_pins=pins, user_reboot_confirmed=True,
                                log_prefix=self.log)
        original = self.script()
        def local_script(run):
            script = original.replace(
                "if([Environment]::MachineName-cne'LAPTOP-NP49RG6H'-or\n"
                " [Security.Principal.WindowsIdentity]::GetCurrent().User.Value-cne\n"
                " 'S-1-5-21-3080296863-1242338503-1346362208-1001'){throw 'Wrong delivery host/SID'}",
                '# local fixture: no remote host identity read')
            script = script.replace(REMOTE_RUN, str(run))
            script = script.replace('D:\\MoirenTest\\reviewed\\', str(finalizer.parent) + '\\')
            return script.replace("'D:\\MoirenTest\\'", "'" + str(self.root) + "\\'")

        for engine in ('powershell.exe', 'pwsh.exe'):
            run = self.root / (engine + '-success')
            run.mkdir()
            encoded = base64.b64encode(local_script(run).encode('utf-16-le')).decode('ascii')
            process = REAL_POPEN([engine, '-NoLogo', '-NoProfile', '-NonInteractive',
                                 '-EncodedCommand', encoded], stdin=subprocess.PIPE,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            stdout, stderr = process.communicate(b'{}')
            self.assertEqual(process.returncode, 0, stderr.decode('utf-8', errors='replace'))
            self.assertEqual(json.loads(stdout)['LockedScripts'], 6)
            self.assertEqual(Path(str(run) + '.delivery-host.json').read_bytes(), b'{}')
            for name in ('closed-export.ps1', 'finite-tool.ps1'):
                for mutation in ('hash', 'length'):
                    with self.subTest(engine=engine, name=name, mutation=mutation):
                        dependency = remote_paths[name]
                        before = dependency.read_bytes()
                        changed = bytes([before[0] ^ 1]) + before[1:]
                        dependency.write_bytes(changed if mutation == 'hash' else before + b'x')
                        run = self.root / (engine + '-' + name + '-' + mutation)
                        run.mkdir()
                        encoded = base64.b64encode(local_script(run).encode('utf-16-le')).decode('ascii')
                        result = REAL_POPEN([engine, '-NoLogo', '-NoProfile', '-NonInteractive',
                                             '-EncodedCommand', encoded], stdin=subprocess.PIPE,
                                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                        result.communicate(b'{}')
                        dependency.write_bytes(before)
                        self.assertNotEqual(result.returncode, 0, 'Mutated dependency accepted')
                        self.assertFalse(Path(str(run) + '.delivery-host.json').exists(),
                                         'Receipt uploaded before dependency refusal')

    def test_actual_harmless_child_binary_unicode_stdout_stderr(self):
        calls = []
        popen = REAL_POPEN
        def child(argv, **kwargs):
            calls.append(argv)
            return popen([sys.executable, '-c',
                          'import sys;sys.stdout.buffer.write(bytes([0,255])+"\u4e2d\u6587".encode());'
                          'sys.stderr.buffer.write("harmless Unicode \u6587".encode())'], **kwargs)
        with mock.patch.object(M.subprocess, 'Popen', child):
            result = self.download()
        self.assertEqual(self.destination.read_bytes(), b'\x00\xff' + '\u4e2d\u6587'.encode())
        self.assertEqual(result['Bytes'], 8)
        self.assertEqual(len(calls), 1)
        self.assertIn('harmless Unicode', Path(str(self.log) + '.stderr.log').read_text(encoding='utf-8'))

    def test_actual_late_success_is_failed_and_original_child_exits_naturally(self):
        popen = REAL_POPEN
        processes = []
        def child(argv, **kwargs):
            p = popen([sys.executable, '-c', 'import time;time.sleep(0.12);print("late")'], **kwargs)
            processes.append(p)
            return p
        with mock.patch.object(M, 'TIMEOUT_SECONDS', 0.02), mock.patch.object(M.subprocess, 'Popen', child):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                self.download()
        self.assertTrue(raised.exception.owner.timed_out)
        self.assertTrue(raised.exception.owner.exited_confirmed)
        self.assertEqual(processes[0].poll(), 0)
        self.assertEqual(self.destination.read_bytes(), b'late\r\n' if sys.platform == 'win32' else b'late\n')
        self.assertEqual(len(processes), 1)

    def test_actual_harmless_child_cap_is_failed_after_full_drain(self):
        popen = REAL_POPEN
        def child(argv, **kwargs):
            return popen([sys.executable, '-c',
                          'import sys;sys.stdout.buffer.write(b"x"*200000);sys.stderr.buffer.write(b"e"*100000)'], **kwargs)
        with mock.patch.object(M.subprocess, 'Popen', child):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                self.download(cap=17)
        self.assertTrue(raised.exception.owner.exited_confirmed)
        self.assertEqual(self.destination.stat().st_size, 17)
        self.assertEqual(Path(str(self.log) + '.stderr.log').stat().st_size, 65536)


if __name__ == '__main__':
    unittest.main(verbosity=2)
