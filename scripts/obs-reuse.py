# SPDX-License-Identifier: MIT
"""Reuse one dedicated OBS installation; archive config, never application trees.

The CLI is a target-only read-only check. Remote acquisition harnesses use
ReusedObs.prepare/launch/finish and retain their own recording/normal-close
protocol. This helper never starts recording, closes windows or kills processes.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import secrets
import stat
import subprocess
import sys
import zipfile

CANONICAL_ROOT = Path(r'D:\MoirenTest\20261101-clean-comb-probe-1320\obs-portable')
EXE_SHA256 = '30FB8CCF7700B6747FBF349845EC0EE0DDCBCD1189AA67607E1FF6ADC7EC9E80'
MAX_CONFIG_BYTES = 16 * 1024 * 1024
MAX_CONFIG_FILES = 1024


def no_links(path):
    path = Path(path).absolute()
    if any(part in ('.', '..') for part in path.parts):
        raise ValueError('noncanonical path components refused')
    for item in (path, *path.parents):
        if item.exists() or item.is_symlink():
            info = item.lstat()
            if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & 0x400:
                raise ValueError('reparse/symlink path refused')
            if stat.S_ISREG(info.st_mode) and info.st_nlink != 1:
                raise ValueError('hardlinked file path refused')
    return path


def digest(path):
    value = hashlib.sha256()
    with Path(path).open('rb') as stream:
        remaining = 32 * 1024 * 1024
        while remaining:
            block = stream.read(min(65536, remaining))
            if not block:
                return value.hexdigest().upper()
            value.update(block)
            remaining -= len(block)
        if stream.read(1):
            raise ValueError('file hash cap exceeded')
    return value.hexdigest().upper()


def save(path, value):
    text = json.dumps(value, indent=2, ensure_ascii=True, allow_nan=False)
    if len(text) > 1024 * 1024:
        raise ValueError('OBS evidence output cap exceeded')
    with Path(path).open('x', encoding='ascii') as stream:
        stream.write(text)


def windows_idle_check(environment=None):
    environment = os.environ if environment is None else environment
    if environment.get('COMPUTERNAME') != 'LAPTOP-NP49RG6H' or os.name != 'nt':
        raise RuntimeError('OBS operation requires the separate authorized target')
    script = r"""
$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
if([Security.Principal.WindowsIdentity]::GetCurrent().User.Value-cne
 'S-1-5-21-3080296863-1242338503-1346362208-1001'){throw 'Wrong OBS target identity'}
$owners=@(Get-CimInstance Win32_Process -OperationTimeoutSec 5 |
 Where-Object {$_.Name-match'^obs.*\.exe$'} |
 Select-Object ProcessId,Name)
$listeners=@([Net.NetworkInformation.IPGlobalProperties]::GetIPGlobalProperties().GetActiveTcpListeners() |
 Where-Object {$_.Port-eq4471} | ForEach-Object {$_.ToString()})
@{owners=$owners;listeners=$listeners}|ConvertTo-Json -Depth 3 -Compress
"""
    encoded = base64.b64encode(script.encode('utf-16-le')).decode('ascii')
    result = subprocess.run(
        ['powershell.exe', '-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', encoded],
        capture_output=True, check=True, timeout=15, creationflags=subprocess.CREATE_NO_WINDOW)
    value = json.loads(result.stdout.decode('utf-8-sig'))
    if (not isinstance(value, dict) or not isinstance(value.get('owners'), list) or
            not isinstance(value.get('listeners'), list)):
        raise RuntimeError('malformed OBS ownership query')
    if value['owners']:
        raise RuntimeError('preserve existing OBS processes; dedicated instance is unavailable')
    if value['listeners']:
        raise RuntimeError('preserve existing listener on the dedicated OBS websocket port')
    return value


class ReusedObs:
    def __init__(self, root=CANONICAL_ROOT, expected_sha256=EXE_SHA256, *,
                 idle_check=windows_idle_check, max_config_bytes=MAX_CONFIG_BYTES,
                 max_config_files=MAX_CONFIG_FILES):
        if not 1 <= max_config_bytes <= MAX_CONFIG_BYTES or not 1 <= max_config_files <= MAX_CONFIG_FILES:
            raise ValueError('configuration cap out of range')
        self.root = Path(root).absolute()
        self.executable = self.root / 'bin/64bit/obs64.exe'
        self.config = self.root / 'config/obs-studio'
        self.expected_sha256 = expected_sha256
        self.idle_check = idle_check
        self.max_config_bytes = max_config_bytes
        self.max_config_files = max_config_files
        self.lease = None
        self.process = None
        self.launch_attempted = False
        self.run = None
        self.password = None
        self.plan = None

    def check(self):
        self.idle_check()
        no_links(self.executable)
        no_links(self.config)
        if not self.executable.is_file() or digest(self.executable) != self.expected_sha256:
            raise ValueError('dedicated OBS executable pin mismatch')
        return dict(executable=str(self.executable), executable_sha256=self.expected_sha256,
                    configuration=str(self.config), application_copies=0)

    def acquire(self):
        if self.lease is not None:
            raise RuntimeError('OBS lease already held')
        lock = no_links(self.root / 'config/.moiren-obs-reuse.lock')
        lock.parent.mkdir(parents=True, exist_ok=True)
        # Lock byte zero even on an empty file. Windows permits a byte-range
        # lock past EOF; no unlocked append/init race can poison the file.
        stream = lock.open('a+b')
        try:
            stream.seek(0)
            # The live kernel lock serializes harnesses. A receipt, PID or
            # timeout never authorizes breaking another owner's lease.
            if os.name == 'nt':
                import msvcrt
                msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BaseException:
            stream.close()
            raise
        self.lease = stream

    def release(self):
        if self.launch_attempted and self.process is None:
            raise RuntimeError('uncertain OBS launch; inspect ownership before releasing lease')
        if self.process is not None and self.process.poll() is None:
            raise RuntimeError('owned OBS still running; preserve its lease')
        if self.lease is not None:
            stream, self.lease = self.lease, None
            try:
                if os.name == 'nt':
                    import msvcrt
                    stream.seek(0)
                    msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
                else:
                    import fcntl
                    fcntl.flock(stream.fileno(), fcntl.LOCK_UN)
            finally:
                stream.close()

    def archive_config(self, destination):
        entries, total, visited = [], 0, 0
        if self.config.exists():
            pending = [(self.config, 0)]
            while pending:
                directory, depth = pending.pop()
                no_links(directory)
                if depth > 32:
                    raise ValueError('configuration directory depth cap exceeded')
                # Check every directory before descent: rglob+sorted would
                # enumerate an unbounded tree before checking a junction/cap.
                with os.scandir(directory) as children:
                    for entry in children:
                        visited += 1
                        if visited > 2 * MAX_CONFIG_FILES:
                            raise ValueError('configuration entry cap exceeded')
                        path = no_links(entry.path)
                        if len(path.relative_to(self.config).as_posix()) > 512:
                            raise ValueError('configuration relative path cap exceeded')
                        if entry.is_dir(follow_symlinks=False):
                            pending.append((path, depth + 1))
                            continue
                        if not entry.is_file(follow_symlinks=False):
                            raise ValueError('non-regular configuration entry')
                        size = entry.stat(follow_symlinks=False).st_size
                        total += size
                        if (len(entries) >= self.max_config_files or total > self.max_config_bytes or
                                path.suffix.lower() in ('.exe', '.dll', '.pdb', '.sys', '.mkv', '.wav')):
                            raise ValueError('configuration archive cap/type refused')
                        entries.append((path, size))
        # Preflight the whole bounded inventory before creating any archive.
        members = []
        with zipfile.ZipFile(destination, 'x', compression=zipfile.ZIP_DEFLATED) as archive:
            for path, size in sorted(entries):
                no_links(path)
                with path.open('rb') as stream:
                    data = stream.read(size + 1)
                if len(data) != size:
                    raise ValueError('configuration changed during readback')
                name = path.relative_to(self.config).as_posix()
                archive.writestr(name, data)
                members.append(dict(path=name, bytes=size,
                                    sha256=hashlib.sha256(data).hexdigest().upper()))
        return dict(bytes=total, members=members, sha256=digest(destination))

    def prepare(self, run):
        if self.run is not None:
            raise RuntimeError('OBS session already prepared')
        run = no_links(run)
        if not run.is_dir() or run == self.root or self.root in run.parents or run in self.root.parents:
            raise ValueError('existing, separate run directory required')
        for name in ('obs-config-before.zip', 'obs-config-after.zip', 'obs-reuse-plan.json',
                     'obs-reuse-result.json'):
            if (run / name).exists() or (run / name).is_symlink():
                raise ValueError('prior OBS evidence must not be overwritten')
        self.check()
        self.acquire()
        try:
            self.check()
            before = self.archive_config(run / 'obs-config-before.zip')
            password = secrets.token_hex(24)
            records = no_links(run / 'records')
            records.mkdir(exist_ok=False)
            profile = (
                '[General]\nName=Isolated\n[Output]\nMode=Advanced\n[AdvOut]\nRecType=Standard\n'
                f'RecFilePath={records.as_posix()}\nRecFormat2=mkv\nRecEncoder=obs_x264\n'
                'RecAudioEncoder=ffmpeg_pcm_s24le\nRecTracks=1\nRecSplitFile=false\n'
                '[Video]\nBaseCX=320\nBaseCY=180\nOutputCX=320\nOutputCY=180\nFPSCommon=10\n'
                '[Audio]\nSampleRate=48000\nChannelSetup=Stereo\n')
            global_ini = (
                '[General]\nFirstRun=false\nLastVersion=520093699\nEnableAutoUpdates=false\n'
                '[Basic]\nProfile=Isolated\nProfileDir=Isolated\nSceneCollection=Isolated\n'
                'SceneCollectionFile=Isolated\n[BasicWindow]\nPreviewEnabled=false\n'
                'ConfirmOnExit=false\nWarnBeforeStoppingRecord=false\n')
            scene = dict(name='Isolated', current_scene='Isolated',
                         current_program_scene='Isolated', scene_order=[dict(name='Isolated')],
                         sources=[dict(name='Isolated', id='scene', versioned_id='scene',
                                       settings=dict(items=[]))])
            websocket = dict(server_enabled=True, server_port=4471, auth_required=True,
                             server_password=password, alerts_enabled=False, first_load=False)
            updates = {
                'basic/profiles/Isolated/basic.ini': profile,
                'user.ini': global_ini, 'global.ini': global_ini,
                'basic/scenes/Isolated.json': json.dumps(scene),
                'plugin_config/obs-websocket/config.json': json.dumps(websocket)}
            for name in updates:
                no_links(self.config / name)
            self.idle_check()
            for name, text in updates.items():
                path = no_links(self.config / name)
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text, encoding='utf-8')
            plan = dict(
                executable=str(self.executable), executable_sha256=self.expected_sha256,
                cwd=str(self.executable.parent), application_copies=0,
                arguments=['--portable', '--profile', 'Isolated', '--collection', 'Isolated',
                           '--disable-shutdown-check', '--disable-updater', '--minimize-to-tray',
                           '--websocket_ipv4_only'],
                before=before, qualified=False)
            save(run / 'obs-reuse-plan.json', plan)
            self.password, self.plan, self.run = password, plan, run
            return plan
        except BaseException:
            self.release()
            raise

    def launch(self, launcher):
        if self.lease is None or self.plan is None or self.launch_attempted:
            raise RuntimeError('one prepared, leased OBS launch required')
        self.check()
        self.launch_attempted = True
        process = launcher(self.executable, self.plan['arguments'], 'obs', self.executable.parent)
        if process is None or not callable(getattr(process, 'poll', None)):
            raise RuntimeError('OBS launch did not retain an observable process owner')
        self.process = process
        return self.process

    def finish(self):
        if self.lease is None or self.run is None:
            raise RuntimeError('OBS session is not prepared')
        if self.launch_attempted and self.process is None:
            raise RuntimeError('uncertain OBS launch cannot become successful cleanup')
        if self.process is not None and self.process.poll() is None:
            raise RuntimeError('confirm recording inactive and join owned OBS before finish')
        self.idle_check()
        after = self.archive_config(self.run / 'obs-config-after.zip')
        exit_code = self.process.poll() if self.process is not None else None
        result = dict(configuration_archived=True, after=after, application_copies=0,
                      obs_exit_code=exit_code, obs_exit_success=exit_code == 0, qualified=False)
        save(self.run / 'obs-reuse-result.json', result)
        self.release()
        return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', required=True,
                        help='read-only identity, executable and existing-owner check')
    args = parser.parse_args(argv)
    try:
        value = ReusedObs().check()
        print(json.dumps(value, ensure_ascii=True))
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
