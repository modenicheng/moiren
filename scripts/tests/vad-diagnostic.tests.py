# SPDX-License-Identifier: MIT
"""Production entry handoff tests use local files and a fake SSH boundary."""
import base64
import hashlib
import importlib.util
import io
import json
import ntpath
import os
from pathlib import Path
import re
import tempfile
import subprocess
import unittest
from unittest import mock


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


SOURCE = Path(__file__).resolve().parents[1] / 'vad-diagnostic.py'
M = load('diagnostic_handoff_tests', SOURCE) if SOURCE.exists() else None
RUN = r'D:\MoirenTest\diagnostic-handoff-fixture'
SOURCES = r'D:\MoirenTest\reviewed\vad-removal'
MANIFEST = r'D:\MoirenTest\reviewed\run-manifest.json'
NAMES = ('entry-launch.ps1', 'invoke-diagnostic.ps1', 'run-manifest.ps1',
         'run-inputs.ps1', 'finite-tool.ps1', 'closed-export.ps1', 'delivery-finalize.ps1')
REAL_POPEN = subprocess.Popen


def encoded(value):
    return json.dumps(value, separators=(',', ':'), allow_nan=False).encode('utf-8')


def pin(path, data):
    return {'Path': str(path), 'Bytes': len(data),
            'Sha256': hashlib.sha256(data).hexdigest().upper()}


class Pipe(io.BytesIO):
    def close(self):
        self.captured = self.getvalue()
        super().close()


class Process:
    def __init__(self, data, code=0):
        self.pid = 1987
        self.stdout = io.BytesIO(data)
        self.stderr = io.BytesIO()
        self.stdin = Pipe()
        self.code = code

    def poll(self):
        return self.code

    def kill(self):
        raise AssertionError('Original entry transport must not be killed')

    terminate = kill


class Availability(unittest.TestCase):
    def test_production_handoff_exists(self):
        self.assertIsNotNone(M, 'Production diagnostic handoff is not implemented')


@unittest.skipUnless(M, 'Not implemented yet')
class Handoff(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.manifest = self.root / 'target-manifest.json'
        self.source = [{'Name': name, **pin(ntpath.join(SOURCES, name), name.encode())}
                       for name in NAMES]
        # Complete required production membership is taken from the real source;
        # tiny bytes stand in for reviewed target files, never executable claims.
        for name in M.REQUIRED_SOURCES:
            if name not in NAMES:
                self.source.append({'Name': name,
                                    **pin(ntpath.join(SOURCES, name), name.encode())})
        self.raw = encoded({'SchemaVersion': 1, 'SourcesRevision': 'c' * 40,
                            'Sources': self.source})
        self.manifest.write_bytes(self.raw)
        self.manifest_hash = pin('', self.raw)['Sha256']
        self.powershell = pin(r'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe', b'original shell')
        self.manifest_pin = pin(MANIFEST, self.raw)
        self.entry_pin = {key: value for key, value in self.source[0].items() if key != 'Name'}
        self.ack_pin = pin(RUN + '.archive-local.json', b'original independent acknowledgment')
        self.entry_stdout = encoded({'LocalAcknowledgement': self.ack_pin,
                                     'Qualified': False})
        self.terminal = {'SchemaVersion': 1, 'Meaning': 'ORIGINAL_DIAGNOSTIC_ENTRY_TERMINAL',
                         'Target': 'moiren-lan', 'RunDirectory': RUN,
                         'ManifestPin': self.manifest_pin, 'EntrySourcePin': self.entry_pin,
                         'ChildSourcePin': {key: value for key, value in self.source[1].items() if key != 'Name'},
                         'Process': {'Pid': 987, 'CreationUtcTicks': 123456789,
                                     'ExitedConfirmed': True},
                         'ExitCode': 1, 'TimedOut': False, 'OutputStable': True, 'Errors': [],
                         'StdoutPin': pin(RUN + '.entry-stdout.log', self.entry_stdout),
                         'StderrPin': pin(RUN + '.entry-stderr.log', b'original diagnostic failure'),
                         'LocalAcknowledgementPin': self.ack_pin,
                         'NativePassed': False, 'Qualified': False}
        self.calls = []
        self.guard = mock.patch.object(M.TRANSPORT.subprocess, 'Popen',
                                       side_effect=AssertionError('Unmocked remote launch refused'))
        self.guard.start()
        self.addCleanup(self.guard.stop)

    def options(self, **overrides):
        value = dict(manifest_copy=self.manifest, expected_manifest_sha256=self.manifest_hash,
                     remote_manifest=MANIFEST, remote_run=RUN, powershell_pin=self.powershell,
                     output_directory=self.root / 'host', user_reboot_confirmed=True,
                     worker_session_id=0)
        value.update(overrides)
        return value

    def launch(self, config, terminal_path, *, user_reboot_confirmed, log_prefix):
        self.calls.append(('launch', config))
        self.assertTrue(user_reboot_confirmed)
        self.assertEqual(config['RunDirectory'], RUN)
        self.assertEqual(config['ManifestPin'], self.manifest_pin)
        self.assertEqual(config['PowerShellPin'], self.powershell)
        return pin(terminal_path, encoded(self.terminal))

    def download(self, remote, destination, **options):
        self.calls.append(('download', remote))
        data = {RUN + '.entry-terminal.json': encoded(self.terminal),
                RUN + '.entry-stdout.log': self.entry_stdout,
                RUN + '.entry-stderr.log': b'original diagnostic failure'}
        self.assertIn(remote, data)
        Path(destination).write_bytes(data[remote])

    def delivery(self, run, ack_hash, output, finalizer, finalizer_hash, **options):
        self.calls.append(('deliver', ack_hash))
        self.assertEqual(run, RUN)
        self.assertEqual(ack_hash, self.ack_pin['Sha256'])
        self.assertEqual(finalizer, ntpath.join(SOURCES, 'delivery-finalize.ps1'))
        self.assertEqual(finalizer_hash, pin('', b'delivery-finalize.ps1')['Sha256'])
        self.assertEqual({item['Path'] for item in options['dependency_pins']},
                         {ntpath.join(SOURCES, name) for name in ('finite-tool.ps1', 'closed-export.ps1')})
        return {'Meaning': 'CLOSED_RUN_DELIVERY_COMPLETE', 'Transferred': True,
                'NativePassed': False, 'Qualified': False, 'CleanupAuthorized': False,
                'RunDirectory': run, 'SourceLocalAckPin': self.ack_pin}

    def execute(self, **overrides):
        return M.run_diagnostic(**self.options(**overrides), launch=self.launch,
                                download=self.download, deliver=self.delivery)

    def test_confirmation_refuses_before_outputs_or_any_dispatch(self):
        for confirmation in (False, None, 1, 'true'):
            with self.subTest(confirmation=confirmation), self.assertRaises(ValueError):
                self.execute(user_reboot_confirmed=confirmation)
        self.assertEqual(self.calls, [])
        self.assertFalse((self.root / 'host').exists())

    def test_closed_exit_one_delivers_original_failure_evidence(self):
        result = self.execute()
        self.assertEqual([item[0] for item in self.calls], ['launch', 'download', 'download', 'download', 'deliver'])
        self.assertEqual(result['EntryExitCode'], 1)
        self.assertTrue(result['Transferred'])
        self.assertFalse(result['NativePassed'])
        self.assertFalse(result['Qualified'])
        self.assertFalse(result['CleanupAuthorized'])
        self.assertEqual(json.loads((self.root / 'host' / 'handoff-complete.json').read_bytes()), result)

    def test_exit_zero_without_ack_is_retained_but_not_delivered(self):
        self.terminal.update(ExitCode=0, LocalAcknowledgementPin=None)
        with self.assertRaises(ValueError):
            self.execute()
        self.assertEqual([item[0] for item in self.calls], ['launch', 'download'])
        self.assertTrue((self.root / 'host' / 'entry-terminal.json').is_file())
        self.assertFalse((self.root / 'host' / 'handoff-complete.json').exists())

    def test_original_stdout_ack_hash_cannot_be_replaced_by_receipt_current_ack(self):
        self.terminal['LocalAcknowledgementPin'] = pin(RUN + '.archive-local.json', b'changed current ack')
        with self.assertRaises(ValueError):
            self.execute()
        self.assertFalse(any(call[0] == 'deliver' for call in self.calls))
        self.assertEqual((self.root / 'host' / 'entry-stdout.log').read_bytes(), self.entry_stdout)

    def test_wrong_manifest_hash_duplicate_source_or_missing_dependency_never_launches(self):
        for mode in ('hash', 'duplicate', 'missing', 'case', 'path'):
            data = json.loads(self.raw)
            if mode == 'duplicate':
                data['Sources'].append(data['Sources'][0])
            elif mode == 'missing':
                data['Sources'] = [item for item in data['Sources'] if item['Name'] != 'finite-tool.ps1']
            elif mode == 'case':
                data['Sources'][0]['Name'] = 'ENTRY-LAUNCH.ps1'
            elif mode == 'path':
                data['Sources'][0]['Path'] = r'D:\outside\entry-launch.ps1'
            raw = encoded(data)
            self.manifest.write_bytes(raw)
            expected = '0' * 64 if mode == 'hash' else pin('', raw)['Sha256']
            with self.subTest(mode=mode), self.assertRaises(ValueError):
                self.execute(expected_manifest_sha256=expected)
        self.assertEqual(self.calls, [])

    def test_terminal_identity_and_original_ack_path_are_required(self):
        for field, value in (('RunDirectory', RUN + '-other'), ('ExitCode', True),
                             ('TimedOut', True), ('OutputStable', False),
                             ('Errors', ['cap']), ('NativePassed', True), ('Qualified', True),
                             ('ManifestPin', pin(MANIFEST, b'foreign')),
                             ('LocalAcknowledgementPin', pin(RUN + '-other.archive-local.json', b'x'))):
            with self.subTest(field=field):
                original = self.terminal[field]
                self.terminal[field] = value
                try:
                    with self.assertRaises(ValueError):
                        self.execute(output_directory=self.root / ('case-' + field))
                    self.assertNotEqual(self.calls[-1][0], 'deliver')
                finally:
                    self.terminal[field] = original

    def test_changed_terminal_bytes_or_foreign_returned_pin_refuses(self):
        def foreign(config, terminal_path, **options):
            return pin(RUN + '-foreign.entry-terminal.json', encoded(self.terminal))
        with self.assertRaises(ValueError):
            M.run_diagnostic(**self.options(), launch=foreign,
                             download=self.download, deliver=self.delivery)
        self.assertEqual(self.calls, [])

    def test_current_source_membership_matches_the_production_consumer(self):
        import re
        text = (SOURCE.with_name('vad-removal') / 'run-inputs.ps1').read_text()
        body = text.split('function Get-RemovalRunSourceNames {', 1)[1].split('\n}', 1)[0]
        self.assertEqual(set(re.findall(r"'([a-z][a-z0-9-]*\.ps1)'", body)), M.REQUIRED_SOURCES)

    def test_actual_cli_unconfirmed_reboot_never_creates_outputs(self):
        output = self.root / 'cli-not-started'
        with mock.patch.object(M.TRANSPORT.subprocess, 'Popen', REAL_POPEN):
            result = subprocess.run([os.sys.executable, '-B', str(SOURCE),
                                     '--manifest-copy', str(self.manifest),
                                     '--expected-manifest-sha256', self.manifest_hash,
                                     '--remote-manifest', MANIFEST, '--run-directory', RUN,
                                     '--powershell-pin', str(self.root / 'absent-pin.json'),
                                     '--expected-powershell-pin-sha256', '0' * 64,
                                     '--output-directory', str(output)], capture_output=True, timeout=15)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b'user reboot confirmation required', result.stderr)
        self.assertFalse(output.exists())

    def test_handoff_validates_downloaded_entry_log_bytes_before_delivery(self):
        def changed(remote, destination, **options):
            self.download(remote, destination, **options)
            if remote.endswith('.entry-stdout.log'):
                with Path(destination).open('ab') as stream:
                    stream.write(b' ')
        with self.assertRaises(ValueError):
            M.run_diagnostic(**self.options(), launch=self.launch,
                             download=changed, deliver=self.delivery)
        self.assertFalse(any(call[0] == 'deliver' for call in self.calls))

    def test_pending_original_launch_is_drained_never_relaunched(self):
        class Owner:
            closed = False
            observations = 0

            def observe(self):
                self.observations += 1
                self.closed = self.observations == 3
                if self.observations == 1:
                    raise OSError('transient original observation failure')

        owner = Owner()
        error = RuntimeError('original refused launch observation')
        error.owner = owner

        def pending(*args, **kwargs):
            self.calls.append(('launch', None))
            raise error

        with self.assertRaises(RuntimeError) as raised:
            M.run_diagnostic(**self.options(), launch=pending,
                             download=self.download, deliver=self.delivery)
        self.assertIs(raised.exception, error)
        self.assertTrue(owner.closed)
        self.assertEqual(self.calls, [('launch', None)])

    def test_real_retained_transport_bootstrap_holds_all_sources_before_import(self):
        config = M.prepare_config(**{key: value for key, value in self.options().items()
                                    if key not in ('output_directory', 'user_reboot_confirmed')})
        response = pin(RUN + '.entry-terminal.json', encoded(self.terminal))
        process = Process(encoded(response))

        def popen(argv, **options):
            self.calls.append((argv, options))
            return process

        with mock.patch.object(M.TRANSPORT.subprocess, 'Popen', popen):
            result = M.launch_entry(config, RUN + '.entry-terminal.json',
                                    user_reboot_confirmed=True, log_prefix=self.root / 'bootstrap')
        self.assertEqual(result, response)
        self.assertEqual(len(self.calls), 1)
        argv = self.calls[0][0]
        self.assertEqual(argv[-2], 'moiren-lan')
        self.assertLess(len(argv[-1]), 8191)
        loader = base64.b64decode(argv[-1].split()[-1]).decode('utf-16-le')
        length = int(re.search(r'\$transportLength=(\d+)', loader)[1])
        script = process.stdin.captured[:length].decode('utf-8')
        self.assertLess(script.index('foreach($pin in $config.Sources)'), script.index('. $entry'))
        self.assertIn('FileShare]::Read', script)
        self.assertIn('Administrator', script)
        self.assertNotIn('RunAs', script)
        self.assertNotIn('Restart-Computer', script)
        payload = json.loads(process.stdin.captured[length:])
        self.assertEqual(payload['Sources'], self.source)

    @unittest.skipUnless(os.name == 'nt', 'Actual bootstrap requires Windows PowerShell')
    def test_actual_bootstrap_refuses_mutated_dependency_before_import(self):
        local = self.root / 'reviewed'
        local.mkdir()
        marker = self.root / 'imported.txt'
        entry = local / 'entry-launch.ps1'
        entry.write_text("[IO.File]::WriteAllText('" + str(marker).replace("'", "''") + "','imported')",
                         encoding='utf-8')
        helper = local / 'finite-tool.ps1'
        helper.write_bytes(b'# original helper')
        sources = [{'Name': path.name, **pin(path, path.read_bytes())} for path in (entry, helper)]
        manifest = self.root / 'bootstrap-manifest.json'
        manifest.write_bytes(encoded({'SourcesRevision': 'c' * 40, 'Sources': sources}))
        exe = Path(os.environ['SystemRoot']) / 'System32/WindowsPowerShell/v1.0/powershell.exe'
        config = {'RunDirectory': str(self.root / 'new-run'), 'ManifestPin': pin(manifest, manifest.read_bytes()),
                  'SourceDirectory': str(local), 'SourcesRevision': 'c' * 40, 'Sources': sources,
                  'PowerShellPin': pin(exe, exe.read_bytes()), 'WorkerSessionId': 0}
        payload = encoded(config)
        script = M._BOOTSTRAP
        start = script.index("if([Environment]::MachineName")
        end = script.index('function Assert-LiteralPath', start)
        script = script[:start] + script[end:]
        start = script.index('if(-not([Security.Principal.WindowsPrincipal]')
        end = script.index('$payload=', start)
        script = script[:start] + script[end:]
        script = script.replace("-not$Path.StartsWith('D:\\MoirenTest\\',[StringComparison]::Ordinal)",
                                '-not$Path.StartsWith(' + M.TRANSPORT._ps(str(self.root) + '\\')
                                + ',[StringComparison]::Ordinal)')
        script = script.replace('__LENGTH__', str(len(payload))).replace('__CONFIG_SHA__', pin('', payload)['Sha256'])
        helper.write_bytes(b'# changed after independent pin')
        command = base64.b64encode(script.encode('utf-16-le')).decode('ascii')
        # This is the same generated bootstrap. Only exact target/elevation/root
        # guards are replaced for a harmless development-host refusal fixture.
        with mock.patch.object(M.TRANSPORT.subprocess, 'Popen', REAL_POPEN):
            result = subprocess.run([str(exe), '-NoLogo', '-NoProfile', '-NonInteractive',
                                     '-EncodedCommand', command], input=payload, capture_output=True, timeout=30)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b'Original file', result.stderr)
        self.assertFalse(marker.exists())
        self.assertFalse((self.root / 'new-run').exists())


if __name__ == '__main__':
    unittest.main()
