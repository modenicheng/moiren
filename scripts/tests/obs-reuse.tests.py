# SPDX-License-Identifier: MIT
"""File-only reuse checks; never launch OBS or touch an audio device."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import zipfile

sys.dont_write_bytecode = True
SOURCE = Path(__file__).resolve().parents[1] / 'obs-reuse.py'
SPEC = importlib.util.spec_from_file_location('obs_reuse', SOURCE) if SOURCE.exists() else None
MODULE = importlib.util.module_from_spec(SPEC) if SPEC else None
if SPEC:
    SPEC.loader.exec_module(MODULE)


class Process:
    def __init__(self, exit_code=None):
        self.exit_code = exit_code

    def poll(self):
        return self.exit_code


class ReuseTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(MODULE, 'fixed-path OBS reuse helper is missing')
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.install = self.root / 'existing-obs'
        self.exe = self.install / 'bin/64bit/obs64.exe'
        self.exe.parent.mkdir(parents=True)
        self.exe.write_bytes(b'unchanged synthetic executable; never run')
        self.sha = hashlib.sha256(self.exe.read_bytes()).hexdigest().upper()
        self.config = self.install / 'config/obs-studio'
        self.config.mkdir(parents=True)
        (self.config / 'prior.ini').write_bytes(b'[original]\nvalue=1\n')
        self.run = self.root / 'new-run'
        self.run.mkdir()
        self.idle_checks = 0
        self.sessions = []
        self.addCleanup(self.release_sessions)

    def release_sessions(self):
        for session in self.sessions:
            if session.process is not None:
                session.process.exit_code = 0
            if getattr(session, 'launch_attempted', False) and session.process is None:
                # These callbacks never launch a process. Close only the
                # synthetic fixture handle after testing the refusal itself.
                if session.lease is not None:
                    session.lease.close()
                    session.lease = None
                continue
            session.release()

    def idle(self):
        self.idle_checks += 1

    def session(self, idle=None, **kwargs):
        session = MODULE.ReusedObs(self.install, self.sha,
                                   idle_check=self.idle if idle is None else idle, **kwargs)
        self.sessions.append(session)
        return session

    def test_prepare_reuses_exact_executable_and_archives_only_config(self):
        session = self.session()
        plan = session.prepare(self.run)
        self.assertEqual(Path(plan['executable']), self.exe)
        self.assertEqual(plan['executable_sha256'], self.sha)
        self.assertFalse((self.run / 'obs-portable').exists())
        self.assertFalse(any(p.suffix == '.exe' for p in self.run.rglob('*')))
        self.assertEqual(hashlib.sha256(self.exe.read_bytes()).hexdigest().upper(), self.sha)
        with zipfile.ZipFile(self.run / 'obs-config-before.zip') as archive:
            self.assertEqual(archive.namelist(), ['prior.ini'])
            self.assertEqual(archive.read('prior.ini'), b'[original]\nvalue=1\n')
        self.assertNotIn(session.password, (self.run / 'obs-reuse-plan.json').read_text())

    def test_existing_ordinary_config_outside_dedicated_install_is_untouched(self):
        ordinary = self.root / 'user-appdata/obs-studio/global.ini'
        ordinary.parent.mkdir(parents=True)
        ordinary.write_bytes(b'user configuration')
        self.session().prepare(self.run)
        self.assertEqual(ordinary.read_bytes(), b'user configuration')

    def test_recording_directory_is_per_run_and_profile_explicit(self):
        plan = self.session().prepare(self.run)
        ini = (self.config / 'basic/profiles/Isolated/basic.ini').read_text()
        self.assertIn('RecFilePath=' + (self.run / 'records').as_posix(), ini)
        self.assertIn('RecAudioEncoder=ffmpeg_pcm_s24le', ini)
        self.assertIn('--profile', plan['arguments'])
        self.assertIn('--collection', plan['arguments'])
        self.assertIn('Isolated', plan['arguments'])
        self.assertNotIn('--multi', plan['arguments'])

    def test_hash_mismatch_refuses_before_config_or_archive_changes(self):
        self.exe.write_bytes(b'changed')
        with self.assertRaises(ValueError):
            self.session().prepare(self.run)
        self.assertFalse((self.run / 'obs-config-before.zip').exists())
        self.assertEqual(sorted(p.name for p in self.config.iterdir()), ['prior.ini'])

    def test_active_external_obs_refuses_and_does_not_terminate(self):
        def occupied():
            raise RuntimeError('external OBS is running')
        with self.assertRaises(RuntimeError):
            self.session(idle=occupied).prepare(self.run)
        self.assertFalse((self.run / 'obs-config-before.zip').exists())
        self.assertEqual(sorted(p.name for p in self.config.iterdir()), ['prior.ini'])

    def test_lock_serializes_instances_and_releases_after_close(self):
        first = self.session()
        first.prepare(self.run)
        run2 = self.root / 'second-run'
        run2.mkdir()
        second = self.session()
        with self.assertRaises((OSError, RuntimeError)):
            second.prepare(run2)
        first.finish()
        self.assertEqual(second.prepare(run2)['executable'], str(self.exe))

    def test_lock_does_not_expire_when_only_a_stale_receipt_is_present(self):
        first = self.session()
        first.prepare(self.run)
        (self.config.parent / '.moiren-obs-reuse.lock').touch()
        run2 = self.root / 'second-run'
        run2.mkdir()
        with self.assertRaises((OSError, RuntimeError)):
            self.session().prepare(run2)

    def test_lock_initialization_does_not_append_to_existing_storage(self):
        lock = self.config.parent / '.moiren-obs-reuse.lock'
        lock.write_bytes(b'\0\0')
        session = self.session()
        session.acquire()
        session.release()
        self.assertEqual(lock.read_bytes(), b'\0\0')

    def test_live_other_process_holds_lock_until_actual_exit(self):
        code = '''import importlib.util, pathlib, sys
spec=importlib.util.spec_from_file_location("reuse",sys.argv[1])
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
session=module.ReusedObs(pathlib.Path(sys.argv[2]),sys.argv[3],idle_check=lambda:None)
session.acquire();print("locked",flush=True);sys.stdin.readline();session.release()
'''
        process = subprocess.Popen([sys.executable, '-B', '-c', code, str(SOURCE),
                                    str(self.install), self.sha], stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            self.assertEqual(process.stdout.readline().strip(), b'locked')
            self.assertIsNone(process.poll())
            contender = self.session()
            with self.assertRaises((OSError, RuntimeError)):
                contender.acquire()
            process.communicate(b'release\n', timeout=10)
            self.assertEqual(process.returncode, 0)
            contender.acquire()
            contender.release()
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate(timeout=10)

    def test_launch_uses_existing_path_and_refuses_second_process(self):
        session = self.session()
        session.prepare(self.run)
        calls = []
        process = Process()
        def launch(exe, args, name, cwd):
            calls.append((exe, args, name, cwd))
            return process
        self.assertIs(session.launch(launch), process)
        self.assertEqual(calls[0][0], self.exe)
        self.assertEqual(calls[0][3], self.exe.parent)
        with self.assertRaises(RuntimeError):
            session.launch(launch)
        self.assertEqual(len(calls), 1)

    def test_rechecks_external_owner_and_executable_at_launch(self):
        for changed in ('owner', 'hash'):
            with self.subTest(changed=changed):
                session = self.session()
                run = self.root / changed
                run.mkdir()
                session.prepare(run)
                if changed == 'owner':
                    def occupied():
                        raise RuntimeError('external owner appeared')
                    session.idle_check = occupied
                else:
                    self.exe.write_bytes(b'changed after prepare')
                with self.assertRaises((ValueError, RuntimeError)):
                    session.launch(lambda *_args, **_kwargs: self.fail('must not launch'))
                session.release()

    def test_uncertain_or_invalid_launch_is_not_retried_or_released(self):
        for kind in ('raises', 'none'):
            with self.subTest(kind=kind):
                run = self.root / kind
                run.mkdir()
                session = self.session()
                session.prepare(run)
                def uncertain(*_args):
                    if kind == 'raises':
                        raise RuntimeError('dispatch failed after possible spawn')
                    return None
                with self.assertRaises(RuntimeError):
                    session.launch(uncertain)
                with self.assertRaises(RuntimeError):
                    session.launch(lambda *_args: self.fail('uncertain launch retried'))
                with self.assertRaises(RuntimeError):
                    session.release()
                with self.assertRaises(RuntimeError):
                    session.finish()
                self.assertIsNotNone(session.lease)
                session.lease.close()
                session.lease = None

    def test_live_owned_obs_keeps_lease_and_refuses_finish(self):
        session = self.session()
        session.prepare(self.run)
        process = Process()
        session.launch(lambda *_args, **_kwargs: process)
        with self.assertRaises(RuntimeError):
            session.finish()
        self.assertFalse((self.run / 'obs-reuse-result.json').exists())
        self.assertIsNotNone(session.lease)
        process.exit_code = 0
        receipt = session.finish()
        self.assertEqual(receipt['obs_exit_code'], 0)
        self.assertTrue(receipt['configuration_archived'])
        self.assertIsNone(session.lease)

    def test_failed_obs_exit_is_retained_without_quality_claim(self):
        session = self.session()
        session.prepare(self.run)
        session.launch(lambda *_args, **_kwargs: Process(7))
        receipt = session.finish()
        self.assertEqual(receipt['obs_exit_code'], 7)
        self.assertFalse(receipt['qualified'])
        self.assertFalse(receipt['obs_exit_success'])

    def test_config_byte_and_file_caps_refuse_before_archive_creation(self):
        for kwargs in ({'max_config_bytes': 2}, {'max_config_files': 0}):
            with self.subTest(kwargs=kwargs):
                with self.assertRaises(ValueError):
                    self.session(**kwargs).prepare(self.run)
                self.assertFalse((self.run / 'obs-config-before.zip').exists())

    def test_config_file_count_and_application_binary_refuse_before_archive(self):
        for name, kwargs in (('second.ini', {'max_config_files': 1}), ('copy.exe', {})):
            with self.subTest(name=name):
                path = self.config / name
                path.write_bytes(b'not a configuration')
                try:
                    with self.assertRaises(ValueError):
                        self.session(**kwargs).prepare(self.run)
                    self.assertFalse((self.run / 'obs-config-before.zip').exists())
                finally:
                    path.unlink()

    def test_deep_configuration_tree_refuses_before_archive(self):
        directory = self.config
        for _index in range(33):
            directory = directory / 'd'
            directory.mkdir()
        with self.assertRaises(ValueError):
            self.session().prepare(self.run)
        self.assertFalse((self.run / 'obs-config-before.zip').exists())

    def test_config_link_is_refused_without_following_it(self):
        foreign = self.root / 'foreign.ini'
        foreign.write_text('external settings')
        link = self.config / 'linked.ini'
        try:
            link.symlink_to(foreign)
        except OSError:
            self.skipTest('symlink privilege unavailable')
        with self.assertRaises(ValueError):
            self.session().prepare(self.run)
        self.assertEqual(foreign.read_text(), 'external settings')

    def test_configuration_directory_link_is_refused_before_descent(self):
        foreign = self.root / 'foreign'
        foreign.mkdir()
        (foreign / 'settings.ini').write_bytes(b'external configuration')
        link = self.config / 'directory-link'
        try:
            link.symlink_to(foreign, target_is_directory=True)
        except OSError:
            self.skipTest('symlink privilege unavailable')
        with self.assertRaises(ValueError):
            self.session().prepare(self.run)
        self.assertFalse((self.run / 'obs-config-before.zip').exists())
        self.assertEqual((foreign / 'settings.ini').read_bytes(), b'external configuration')

    def test_hardlinked_configuration_cannot_modify_external_settings(self):
        foreign = self.root / 'external-user-settings.ini'
        foreign.write_bytes(b'ordinary OBS settings')
        path = self.config / 'global.ini'
        try:
            os.link(foreign, path)
        except OSError:
            self.skipTest('hard links unavailable')
        with self.assertRaises(ValueError):
            self.session().prepare(self.run)
        self.assertEqual(foreign.read_bytes(), b'ordinary OBS settings')

    def test_run_directory_cannot_be_inside_obs_or_reuse_prior_evidence(self):
        with self.assertRaises(ValueError):
            self.session().prepare(self.config)
        (self.run / 'obs-reuse-plan.json').write_text('retained evidence')
        with self.assertRaises((ValueError, FileExistsError)):
            self.session().prepare(self.run)
        self.assertFalse((self.run / 'obs-config-before.zip').exists())

    def test_dotdot_alias_cannot_put_run_inside_installation(self):
        alias = self.run / '..' / self.install.name / 'config/obs-studio'
        with self.assertRaises(ValueError):
            self.session().prepare(alias)
        self.assertFalse((self.config / 'obs-config-before.zip').exists())

    def test_existing_recordings_directory_is_not_reused(self):
        records = self.run / 'records'
        records.mkdir()
        media = records / 'user-recording.mkv'
        media.write_bytes(b'preserved media')
        with self.assertRaises(FileExistsError):
            self.session().prepare(self.run)
        self.assertEqual(media.read_bytes(), b'preserved media')
        self.assertFalse((self.config / 'basic/profiles/Isolated/basic.ini').exists())

    def test_cli_has_only_readonly_fixed_path_check(self):
        result = subprocess.run([sys.executable, '-B', str(SOURCE), '--help'],
                                capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0)
        text = result.stdout.decode()
        self.assertIn('--check', text)
        self.assertNotIn('--install-root', text)
        self.assertNotIn('--launch', text)

    def test_target_guard_refuses_host_before_windows_process_query(self):
        with self.assertRaises(RuntimeError):
            MODULE.windows_idle_check(environment={'COMPUTERNAME': 'development-host'})

    @unittest.skipUnless(os.name == 'nt', 'mock Windows boundary only')
    def test_native_owner_reply_requires_idle_obs_and_free_port(self):
        for value, refused in (({'owners': [], 'listeners': []}, False),
                               ({'owners': [{'ProcessId': 123}], 'listeners': []}, True),
                               ({'owners': [], 'listeners': ['0.0.0.0:4471']}, True),
                               ({'owners': []}, True)):
            with self.subTest(value=value):
                completed = subprocess.CompletedProcess([], 0, json.dumps(value).encode(), b'')
                with mock.patch.object(MODULE.subprocess, 'run', return_value=completed) as run:
                    if refused:
                        with self.assertRaises(RuntimeError):
                            MODULE.windows_idle_check(environment={'COMPUTERNAME': 'LAPTOP-NP49RG6H'})
                    else:
                        MODULE.windows_idle_check(environment={'COMPUTERNAME': 'LAPTOP-NP49RG6H'})
                    self.assertEqual(run.call_count, 1)


if __name__ == '__main__':
    unittest.main()
