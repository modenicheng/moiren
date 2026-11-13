# SPDX-License-Identifier: MIT
"""Opt-in file handoff: local files, fake SFTP, harmless PowerShell only."""
import hashlib
import base64
import importlib.util
import inspect
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


M = load('file_handoff_transport_tests', ROOT / 'vad-delivery-transport.py')
REAL_POPEN = subprocess.Popen
REMOTE_PARENT = r'D:\MoirenTest\vad-removal-current'
REMOTE_FILE = REMOTE_PARENT + r'\original.zip'


class Input(io.BytesIO):
    def close(self):
        self.captured = self.getvalue()
        super().close()


class Process:
    def __init__(self, data=b'', code=0):
        self.stdin, self.stdout, self.stderr = Input(), io.BytesIO(data), io.BytesIO()
        self.pid, self.code = 18976, code

    def poll(self):
        return self.code

    def kill(self):
        raise AssertionError('Original handoff owner must never be killed')

    terminate = kill


class FileHandoff(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.helper = ROOT / 'vad-delivery-remote.ps1'
        self.guard = mock.patch.object(M.subprocess, 'Popen',
                                       side_effect=AssertionError('No unmocked target call'))
        self.guard.start()
        self.addCleanup(self.guard.stop)

    def options(self):
        self.assertTrue(self.helper.is_file(), 'Independently pinned remote file helper is missing')
        return {'HelperPath': str(self.helper),
                'ExpectedHelperSha256': hashlib.sha256(self.helper.read_bytes()).hexdigest().upper(),
                'RemoteParent': REMOTE_PARENT}

    def require_api(self):
        self.assertIn('file_handoff', inspect.signature(M.download_file).parameters,
                      'Public download has no explicit opt-in file handoff')
        self.assertTrue(callable(getattr(M, '_run_argv', None)), 'Original owner argv seam is missing')

    def test_file_download_uses_sftp_then_no_stdin_short_command(self):
        self.require_api()
        calls, processes, requests = [], [], []

        def child(argv, **kwargs):
            calls.append(argv)
            if argv[0] == 'sftp':
                batch = Path(argv[argv.index('-b') + 1]).read_text()
                self.assertTrue(batch.startswith('mkdir '), 'First SFTP operation must refuse collisions')
                self.assertEqual(batch.count('\nput '), 2)
                requests.append(json.loads(next(self.root.rglob('request.json')).read_bytes()))
                process = Process()
            elif len(calls) == 1:
                process = Process(b'{"StageParentChecked":true}')
            else:
                self.assertLess(len(argv[-1]), 8191)
                process = Process(b'original bounded bytes')
            processes.append(process)
            return process

        with mock.patch.object(M.subprocess, 'Popen', child), \
                mock.patch.object(M, '_script_frame', side_effect=AssertionError('No SSH stdin frame')):
            result = M.download_file(REMOTE_FILE, self.root / 'download.zip', cap=128,
                                     user_reboot_confirmed=True, log_prefix=self.root / 'download',
                                     file_handoff=self.options())
        self.assertEqual([call[0] for call in calls], ['ssh', 'sftp', 'ssh'])
        self.assertEqual(requests[0]['Operation'], 'Download')
        self.assertEqual(requests[0]['Arguments'], {'RemotePath': REMOTE_FILE, 'Cap': 128})
        self.assertTrue(all(process.stdin.captured == b'' for process in processes))
        self.assertEqual(result['Bytes'], len(b'original bounded bytes'))

    def test_changed_helper_pin_refuses_before_attempt_or_child(self):
        self.require_api()
        options = self.options()
        options['ExpectedHelperSha256'] = '0' * 64
        with self.assertRaises(M.DeliveryTransportError):
            M.download_file(REMOTE_FILE, self.root / 'output', cap=128,
                            user_reboot_confirmed=True, log_prefix=self.root / 'attempt',
                            file_handoff=options)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_sftp_collision_or_upload_failure_never_invokes_helper(self):
        self.require_api()
        calls = []

        def child(argv, **kwargs):
            calls.append(argv)
            return Process(b'{"StageParentChecked":true}') if len(calls) == 1 else Process(code=1)

        with mock.patch.object(M.subprocess, 'Popen', child):
            with self.assertRaises(M.DeliveryTransportError) as raised:
                M.download_file(REMOTE_FILE, self.root / 'output', cap=128,
                                user_reboot_confirmed=True, log_prefix=self.root / 'attempt',
                                file_handoff=self.options())
        self.assertEqual([call[0] for call in calls], ['ssh', 'sftp'])
        self.assertTrue(raised.exception.owner.closed)
        self.assertFalse((self.root / 'output').exists())

    def test_confirmation_and_bad_options_refuse_before_files(self):
        self.require_api()
        for value in (False, 1, 'true'):
            with self.subTest(value=value), self.assertRaises(M.DeliveryTransportError):
                M.download_file(REMOTE_FILE, self.root / 'output', cap=128,
                                user_reboot_confirmed=value, log_prefix=self.root / 'attempt',
                                file_handoff=self.options())
        for options in ({}, dict(self.options(), Extra=True),
                        dict(self.options(), RemoteParent=r'D:\MoirenTest\..\foreign')):
            with self.subTest(options=options), self.assertRaises(M.DeliveryTransportError):
                M.download_file(REMOTE_FILE, self.root / 'output', cap=128,
                                user_reboot_confirmed=True, log_prefix=self.root / 'attempt',
                                file_handoff=options)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_file_finalize_stages_exact_receipt_and_original_ack_dependencies(self):
        self.require_api()
        payload = json.dumps({'Meaning': 'HOST_ARCHIVE_DELIVERY_VERIFIED',
                              'Note': '\u4e2d\u6587' * 6000}, ensure_ascii=False).encode()
        dependencies = [{'Path': REMOTE_PARENT + '\\' + name, 'Bytes': 12, 'Sha256': 'A' * 64}
                        for name in ('closed-export.ps1', 'finite-tool.ps1')]
        calls, processes = [], []

        def child(argv, **kwargs):
            calls.append(argv)
            if argv[0] == 'sftp':
                raw = next(self.root.rglob('payload.json')).read_bytes()
                self.assertEqual(raw, payload)
                request = json.loads(next(self.root.rglob('request.json')).read_bytes())
                self.assertEqual(request['Operation'], 'Finalize')
                self.assertEqual(request['Arguments']['LocalAckSha256'], 'B' * 64)
                self.assertEqual(request['Arguments']['DependencyPins'], dependencies)
                self.assertEqual(request['PayloadPin']['Sha256'], hashlib.sha256(raw).hexdigest().upper())
                p = Process()
            else:
                p = Process(b'{"StageParentChecked":true}' if len(calls) == 1 else b'{"Path":"ack"}')
            processes.append(p)
            return p

        with mock.patch.object(M.subprocess, 'Popen', child):
            result = M.finalize_delivery(REMOTE_PARENT + r'\run', payload, 'B' * 64,
                        REMOTE_PARENT + r'\delivery-finalize.ps1', 'A' * 64,
                        dependency_pins=dependencies, user_reboot_confirmed=True,
                        log_prefix=self.root / 'finalize', file_handoff=self.options())
        self.assertEqual(result, {'Path': 'ack'})
        self.assertTrue(all(p.stdin.captured == b'' for p in processes))

    def test_pending_sftp_owner_is_drained_once_without_helper_invocation(self):
        self.require_api()

        class Pending:
            closed = False
            observations = 0

            def observe(self):
                self.observations += 1
                self.closed = self.observations == 2

        owner = Pending()
        calls = []

        def run(argv, *args, **kwargs):
            calls.append(argv)
            if argv[0] == 'sftp':
                raise M.DeliveryTransportError('Original pending upload', owner=owner)
            return {'StageParentChecked': True}

        with mock.patch.object(M, '_run_argv', run):
            with self.assertRaises(M.DeliveryTransportError):
                M.download_file(REMOTE_FILE, self.root / 'output', cap=128,
                                user_reboot_confirmed=True, log_prefix=self.root / 'attempt',
                                file_handoff=self.options())
        self.assertTrue(owner.closed)
        self.assertEqual([call[0] for call in calls], ['ssh', 'sftp'])


@unittest.skipUnless(os.name == 'nt', 'Harmless local PS5/PS7 file helper execution')
class ActualHelper(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.source = ROOT / 'vad-delivery-remote.ps1'
        self.assertTrue(self.source.is_file(), 'File helper missing')
        self.guard = mock.patch.object(M.subprocess, 'Popen',
                                       side_effect=AssertionError('No unmocked target launch'))
        self.guard.start()
        self.addCleanup(self.guard.stop)
        # Replace only fixed target/elevation/root predicates for local fixtures.
        # All real pin reads, parser, operations and locks remain production code.
        text = self.source.read_text()
        start = text.index("if($env:COMPUTERNAME")
        end = text.index('$locks=', start)
        text = text[:start] + '# TEST-ONLY local host predicate\n' + text[end:]
        text = text.replace("'D:\\MoirenTest\\'", M._ps(str(self.root) + '\\'))
        start = text.index('            if(-not([Security.Principal.WindowsPrincipal]')
        end = text.index('            Assert-HandoffPath $a.RunDirectory', start)
        text = text[:start] + '            # TEST-ONLY local elevation predicate\n' + text[end:]
        self.helper = self.root / 'vad-delivery-remote.ps1'
        self.helper.write_text(text, encoding='utf-8')

    @staticmethod
    def pin(path):
        data = path.read_bytes()
        return {'Path': str(path), 'Bytes': len(data), 'Sha256': hashlib.sha256(data).hexdigest().upper()}

    def run_helper(self, engine, request, name, *, before=None):
        request_path = self.root / (name + '.json')
        request_path.write_bytes(request if isinstance(request, bytes) else json.dumps(request).encode())
        helper_pin, request_pin = self.pin(self.helper), self.pin(request_path)
        argv = M._invoke_file_command(helper_pin, request_pin)
        command = base64.b64decode(argv[-1].split()[-1]).decode('utf-16-le')
        start = command.index('if($env:COMPUTERNAME')
        end = command.index('$pins=', start)
        command = command[:start] + '# TEST-ONLY local host predicate\n' + command[end:]
        command = command.replace("'D:\\MoirenTest\\'", M._ps(str(self.root) + '\\'))
        encoded = base64.b64encode(command.encode('utf-16-le')).decode()
        self.assertLess(len('powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ' + encoded), 8191)
        if before:
            before(request_path)
        process = REAL_POPEN([engine, '-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', encoded],
                             stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        stdout, stderr = process.communicate()
        return process.returncode, stdout, stderr

    def test_actual_binary_download_and_cap_with_empty_stdin(self):
        data = bytes(range(256)) * 15360 + b'\x00\x1a\xff\x80'
        file = self.root / 'binary.zip'
        file.write_bytes(data)
        for engine in ('powershell.exe', 'pwsh.exe'):
            for cap in (len(data), len(data) - 1):
                with self.subTest(engine=engine, cap=cap):
                    request = {'SchemaVersion': 1, 'Operation': 'Download',
                               'Arguments': {'RemotePath': str(file), 'Cap': cap}, 'PayloadPin': None}
                    code, stdout, stderr = self.run_helper(engine, request, engine + '-' + str(cap))
                    if cap == len(data):
                        self.assertEqual(code, 0, stderr.decode('utf-8', 'replace'))
                        self.assertEqual(stdout, data)
                    else:
                        self.assertNotEqual(code, 0)
                        self.assertEqual(stdout, b'', 'Cap rejection happened after output')

    def test_actual_request_or_helper_mutation_refuses_before_output(self):
        source = self.root / 'original.bin'
        source.write_bytes(b'ORIGINAL_BINARY')
        request = {'SchemaVersion': 1, 'Operation': 'Download',
                   'Arguments': {'RemotePath': str(source), 'Cap': 128}, 'PayloadPin': None}
        for engine in ('powershell.exe', 'pwsh.exe'):
            for mutation in ('request', 'helper'):
                with self.subTest(engine=engine, mutation=mutation):
                    original = self.helper.read_bytes()

                    def change(path):
                        target = path if mutation == 'request' else self.helper
                        with target.open('ab') as stream:
                            stream.write(b' ')

                    code, stdout, stderr = self.run_helper(engine, request, engine + mutation, before=change)
                    self.helper.write_bytes(original)
                    self.assertNotEqual(code, 0)
                    self.assertEqual(stdout, b'')
                    self.assertIn(b'pin differs', stderr)

    def test_actual_strict_json_and_unknown_operation_refuse(self):
        cases = [b'{"SchemaVersion":1,"schemaversion":1}', b'{"x":NaN}',
                 b'{"x":1e400}', b'\xff',
                 json.dumps({'SchemaVersion': 1, 'Operation': 'Other', 'Arguments': {}, 'PayloadPin': None}).encode()]
        for engine in ('powershell.exe', 'pwsh.exe'):
            for index, raw in enumerate(cases):
                with self.subTest(engine=engine, case=index):
                    code, stdout, stderr = self.run_helper(engine, raw, engine + '-strict-' + str(index))
                    self.assertNotEqual(code, 0)
                    self.assertEqual(stdout, b'')

    def test_actual_finalize_locks_receipt_sources_and_preserves_create_new(self):
        run = self.root / 'run'
        run.mkdir()
        payload_path = self.root / 'receipt.json'
        raw = json.dumps({'Meaning': 'HOST_ARCHIVE_DELIVERY_VERIFIED', 'Note': 'x' * 12000}).encode()
        payload_path.write_bytes(raw)
        finalizer = self.root / 'delivery-finalize.ps1'
        finalizer.write_text(r'''
function Confirm-RemovalRunDelivery {
 param($RunDirectory,$DeliveryReceiptPath,$ExpectedDeliveryReceiptSha256,$ExpectedLocalAckSha256,[switch]$UserRebootConfirmed)
 $denied=0
 foreach($path in @($PSCommandPath,(Join-Path $PSScriptRoot 'closed-export.ps1'),(Join-Path $PSScriptRoot 'finite-tool.ps1'),
                   (Join-Path $PSScriptRoot 'receipt.json'),$DeliveryReceiptPath,
                   (Join-Path $PSScriptRoot 'vad-delivery-remote.ps1'))){
  try{$w=[IO.File]::Open($path,'Open','Write','ReadWrite');$w.Dispose()}catch{$denied++}
  try{[IO.File]::Delete($path)}catch{$denied++}
 }
 if($denied-ne12){throw ('Missing original read locks: '+$denied)}
 @{Path='actual-ack';OriginalAck=$ExpectedLocalAckSha256;Locked=$denied}
}
''', encoding='ascii')
        dependencies = []
        for name in ('closed-export.ps1', 'finite-tool.ps1'):
            path = self.root / name
            path.write_bytes(b'# harmless dependency\n')
            dependencies.append(self.pin(path))
        request = {'SchemaVersion': 1, 'Operation': 'Finalize', 'Arguments': {
            'RemoteRun': str(run), 'LocalAckSha256': 'B' * 64, 'FinalizerPath': str(finalizer),
            'FinalizerSha256': self.pin(finalizer)['Sha256'], 'DependencyPins': dependencies},
            'PayloadPin': self.pin(payload_path)}
        for engine in ('powershell.exe', 'pwsh.exe'):
            with self.subTest(engine=engine):
                request['Arguments']['RemoteRun'] = str(self.root / engine)
                Path(request['Arguments']['RemoteRun']).mkdir()
                code, stdout, stderr = self.run_helper(engine, request, engine + '-finalize')
                self.assertEqual(code, 0, stderr.decode('utf-8', 'replace'))
                self.assertEqual(json.loads(stdout)['OriginalAck'], 'B' * 64)
                upload = Path(request['Arguments']['RemoteRun'] + '.delivery-host.json')
                self.assertEqual(upload.read_bytes(), raw)
                code, stdout, stderr = self.run_helper(engine, request, engine + '-collision')
                self.assertNotEqual(code, 0)
                self.assertEqual(upload.read_bytes(), raw)


if __name__ == '__main__':
    unittest.main(verbosity=2)
