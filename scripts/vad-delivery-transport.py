# SPDX-License-Identifier: MIT
"""Single-attempt SSH delivery transport; import performs no I/O.

Timeouts are sticky observation failures, NOT permission to kill or detach.
Pumps keep draining the original child until natural exit, so a broken remote
can keep a call pending indefinitely. An observation exception retains the exact
owner on DeliveryTransportError; owner.observe() never launches another child.
Archive/member validation and delivery acknowledgment interpretation belong to
the caller. Neither transport success nor user reboot confirmation is a native
audio/driver qualification or permission to reboot.
"""
import base64
import datetime
import hashlib
import json
import math
import ntpath
import os
from pathlib import Path
import re
import stat
import subprocess
import threading
import time

TIMEOUT_SECONDS = 120.0
MAX_DOWNLOAD = 68157440
MAX_JSON = 1048576
STDERR_CAP = 65536
TARGET = 'moiren-lan'
HOST = 'LAPTOP-NP49RG6H'
SID = 'S-1-5-21-3080296863-1242338503-1346362208-1001'


class DeliveryTransportError(RuntimeError):
    def __init__(self, message, *, owner=None):
        super().__init__(message)
        self.owner = owner


def _reject(message):
    raise DeliveryTransportError(message)


def _confirmation(value):
    if value is not True:
        _reject('Explicit user reboot confirmation required before transport/filesystem operations')


def _remote_path(value):
    if (not isinstance(value, str) or len(value) > 240 or
            not value.startswith('D:\\MoirenTest\\') or
            re.search(r'[\x00-\x1f\x7f"*?<>|/]', value) or ':' in value[2:] or
            ntpath.normpath(value) != value):
        _reject('Canonical literal path under D:\\MoirenTest required')
    for part in value[3:].split('\\'):
        if (not part or part in ('.', '..') or part[-1] in ' .' or
                part.split('.')[0].upper() in {'CON', 'PRN', 'AUX', 'NUL',
                *('COM' + str(i) for i in range(1, 10)),
                *('LPT' + str(i) for i in range(1, 10))}):
            _reject('Noncanonical or reserved remote path component')
    return value


def _sha(value):
    if not isinstance(value, str) or re.fullmatch('[A-Fa-f0-9]{64}', value) is None:
        _reject('Independent SHA256 pin required')
    return value.upper()


def _dependencies(finalizer, pins):
    expected = {ntpath.join(ntpath.dirname(finalizer), name)
                for name in ('closed-export.ps1', 'finite-tool.ps1')}
    if not isinstance(pins, list) or len(pins) != 2:
        _reject('Two exact same-directory finalizer dependency pins required')
    copied = []
    for pin in pins:
        if not isinstance(pin, dict) or set(pin) != {'Path', 'Bytes', 'Sha256'}:
            _reject('Dependency pin requires exact Path/Bytes/Sha256 fields')
        path = _remote_path(pin['Path'])
        if path not in expected or type(pin['Bytes']) is not int or not 0 < pin['Bytes'] <= MAX_JSON:
            _reject('Finalizer dependency path/size differs')
        expected.remove(path)
        copied.append({'Path': path, 'Bytes': pin['Bytes'], 'Sha256': _sha(pin['Sha256'])})
    return copied


def _json_object(data):
    def pairs(rows):
        value = {}
        for key, item in rows:
            if key in value:
                raise ValueError('Duplicate JSON key')
            value[key] = item
        return value

    def floating(text):
        value = float(text)
        if not math.isfinite(value):
            raise ValueError('Nonfinite JSON number')
        return value

    def constant(_):
        raise ValueError('Nonfinite JSON token')

    if not isinstance(data, bytes) or not 0 < len(data) <= MAX_JSON:
        _reject('Bounded nonempty UTF8 JSON object required')
    try:
        value = json.loads(data.decode('utf-8'), object_pairs_hook=pairs,
                           parse_float=floating, parse_constant=constant)
        if not isinstance(value, dict):
            raise ValueError('JSON object required')
        return value
    except (ValueError, UnicodeError, RecursionError):
        _reject('Invalid/duplicate/nonfinite UTF8 JSON object')


def _local_path(value):
    path = Path(value)
    if not path.is_absolute() or os.path.normpath(str(path)) != str(path):
        _reject('Canonical absolute local output path required')
    if any(part in ('.', '..') for part in path.parts):
        _reject('Noncanonical local path')
    if os.name == 'nt' and (str(path).startswith('\\\\') or ':' in str(path)[2:] or
            any(part[-1:] in (' ', '.') for part in path.parts[1:])):
        _reject('Noncanonical local Windows path')
    return path


def _check_path(path):
    for item in (*reversed(path.parents), path):
        try:
            info = item.lstat()
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & 0x400:
            _reject('Symlink/reparse path refused')
        if item != path and not stat.S_ISDIR(info.st_mode):
            _reject('Nondirectory output ancestor')


def _new_file(path):
    _check_path(path)
    if os.name == 'nt':
        import ctypes
        import msvcrt
        from ctypes import wintypes
        create = ctypes.WinDLL('kernel32', use_last_error=True).CreateFileW
        create.argtypes = (wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID,
                           wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE)
        create.restype = wintypes.HANDLE
        # CREATE_NEW, OPEN_REPARSE_POINT, FILE_SHARE_READ only. Retaining this
        # original handle excludes other writers and file deletion during pumps.
        handle = create(str(path), 0xC0000000, 1, None, 1, 0x00200080, None)
        if handle == ctypes.c_void_p(-1).value:
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            descriptor = msvcrt.open_osfhandle(handle, os.O_RDWR | os.O_BINARY)
        except BaseException:
            close = ctypes.WinDLL('kernel32', use_last_error=True).CloseHandle
            close.argtypes, close.restype = (wintypes.HANDLE,), wintypes.BOOL
            close(handle)
            raise
    else:
        flags = os.O_CREAT | os.O_EXCL | os.O_RDWR | getattr(os, 'O_NOFOLLOW', 0)
        descriptor = os.open(path, flags, 0o600)
    try:
        _check_path(path)
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            _reject('Regular create-new file required')
        return os.fdopen(descriptor, 'w+b', buffering=0)
    except BaseException:
        os.close(descriptor)
        raise


def _write_all(stream, data):
    view = memoryview(data)
    while view:
        count = stream.write(view)
        if not isinstance(count, int) or count <= 0 or count > len(view):
            raise OSError('Short output write')
        view = view[count:]


def _durable(stream):
    stream.flush()
    os.fsync(stream.fileno())


def _encoded(value):
    return json.dumps(value, ensure_ascii=True, allow_nan=False, separators=(',', ':')).encode('ascii')


class _Output:
    def __init__(self, stream, cap):
        self.stream, self.cap = stream, cap
        self.seen, self.written = 0, 0
        self.writable = True
        self.eof = False
        self.done = threading.Event()


class _Owner:
    def __init__(self, stdout, stderr, receipt, intent):
        self.process = None
        self.launch_attempted = False
        self.exited_confirmed = False
        self.exit_code = None
        self.timed_out = False
        self.started = None
        self.errors = set()
        self.lock = threading.Lock()
        self.stdout, self.stderr = stdout, stderr
        self.receipt, self.intent = receipt, intent
        self.threads = []
        self.channels = []
        self.pump_start_states = []
        self.setup_refused = False
        self.progress_pending = None
        self.input_done = threading.Event()
        self.result = None
        self.validate = None
        self.saved = False
        self.closed = False

    @property
    def success(self):
        return (self.exited_confirmed and self.exit_code == 0 and not self.errors and
                self.stdout.eof and self.stderr.eof and self.input_done.is_set() and self.saved)

    def fail(self, category):
        with self.lock:
            self.errors.add(category)

    def _deadline(self):
        if self.started is not None and time.monotonic() - self.started >= TIMEOUT_SECONDS:
            self.timed_out = True
            self.fail('deadline')

    def _pump(self, source, output, name):
        try:
            while True:
                try:
                    block = source.read(65536)
                    if not isinstance(block, bytes):
                        raise OSError('Binary pipe required')
                except Exception:
                    # Keep the same pipe, including after a transient read failure.
                    self.fail(name + '_read')
                    time.sleep(0.01)
                    continue
                if not block:
                    output.eof = True
                    break
                previous = output.seen
                output.seen += len(block)
                if output.seen > output.cap:
                    self.fail(name + '_cap')
                keep = block[:max(0, output.cap - previous)]
                if keep and output.writable:
                    try:
                        _write_all(output.stream, keep)
                        output.written += len(keep)
                    except Exception:
                        output.writable = False
                        self.fail(name + '_write')
                # After a sink failure/cap, discard but drain all remaining bytes.
            try:
                _durable(output.stream)
            except Exception:
                self.fail(name + '_flush')
        finally:
            output.done.set()

    def _input(self, payload):
        try:
            _write_all(self.process.stdin, payload)
        except Exception:
            self.fail('stdin_write')
        finally:
            try:
                self.process.stdin.close()
            except Exception:
                self.fail('stdin_close')
            self.input_done.set()

    def start(self, argv, payload):
        self.launch_attempted = True
        self.started = time.monotonic()
        try:
            self.process = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                            stderr=subprocess.PIPE, shell=False, bufsize=0)
        except Exception:
            self.fail('launch_unknown')
            self._save()
            self._close_files()
            return
        # Retain the original Popen before any pipe setup or process observation.
        self.channels = [(self._pump, (self.process.stdout, self.stdout, 'stdout')),
                         (self._pump, (self.process.stderr, self.stderr, 'stderr')),
                         (self._input, (payload,))]
        self.setup_refused = not self._resume_pumps()

    @staticmethod
    def _thread_state(thread):
        if thread.ident is not None or thread.is_alive() or thread._started.is_set():
            return 'started'
        # CPython records a dispatched-but-not-yet-entered thread in _limbo.
        # Without this evidence, an ident-less thread is NOT safe to restart.
        lock = getattr(threading, '_active_limbo_lock', None)
        limbo = getattr(threading, '_limbo', None)
        if lock is None or limbo is None:
            return 'unknown'
        with lock:
            if thread.ident is None and not thread._started.is_set() and thread not in limbo:
                return 'not_started'
        return 'unknown'

    def _resume_pumps(self):
        self.progress_pending = None
        while len(self.threads) < len(self.channels):
            target, arguments = self.channels[len(self.threads)]
            try:
                thread = threading.Thread(target=target, args=arguments, daemon=True)
            except Exception:
                self.fail('pump_construct')
                self.progress_pending = 'pump_construction_refused'
                return False
            # Publish each original channel before constructing the next one.
            # Re-observation completes missing slots on this same Popen only.
            self.threads.append(thread)
            self.pump_start_states.append('not_started')
        for index, thread in enumerate(self.threads):
            state = self.pump_start_states[index]
            if state == 'unknown':
                state = self._thread_state(thread)
                self.pump_start_states[index] = state
            if state == 'unknown':
                self.progress_pending = 'pump_dispatch_unknown'
                return False
            if state == 'started':
                continue
            # Retry only the very same provably undispatched Thread. Retain
            # every other original pump and Popen, including on setup failure.
            try:
                thread.start()
                self.pump_start_states[index] = 'started'
            except Exception:
                self.fail('pump_start_unknown')
                self.pump_start_states[index] = self._thread_state(thread)
                self.progress_pending = 'pump_setup_refused'
                return False
        return True

    def observe(self):
        if self.process is None or self.closed:
            return self
        if not self._resume_pumps():
            self._deadline()
            return self
        while True:
            self._deadline()
            try:
                code = self.process.poll()
            except Exception:
                self.fail('poll_unknown')
                return self
            if code is not None:
                if not isinstance(code, int) or isinstance(code, bool):
                    self.fail('exit_unknown')
                    return self
                self.exit_code, self.exited_confirmed = code, True
                if code != 0:
                    self.fail('exit_nonzero')
            if (self.exited_confirmed and self.stdout.done.is_set() and
                    self.stderr.done.is_set() and self.input_done.is_set()):
                break
            time.sleep(0.005)
        if not self.errors:
            try:
                self.result = self.validate()
            except Exception:
                self.fail('result_validation')
        self._deadline()
        self._save()
        self._close_files()
        self._deadline()
        return self

    def _save(self):
        if self.saved:
            return
        value = {'SchemaVersion': 1, 'Target': TARGET, 'Pid': getattr(self.process, 'pid', None),
                 'LaunchAttempted': self.launch_attempted, 'ExitedConfirmed': self.exited_confirmed,
                 'ExitCode': self.exit_code, 'TimedOut': self.timed_out,
                 'StdoutBytes': self.stdout.seen, 'StderrBytes': self.stderr.seen,
                 'PumpStartStates': self.pump_start_states,
                 'Errors': sorted(self.errors), 'NativePassed': False, 'Qualified': False}
        try:
            _write_all(self.receipt, _encoded(value))
            _durable(self.receipt)
            self._deadline()
            if value['TimedOut'] != self.timed_out:
                # Update only this attempt's still-retained CREATE_NEW handle;
                # a slow evidence flush cannot leave a success-looking receipt.
                value['TimedOut'], value['Errors'] = self.timed_out, sorted(self.errors)
                self.receipt.seek(0)
                self.receipt.truncate()
                _write_all(self.receipt, _encoded(value))
                _durable(self.receipt)
            self.saved = True
        except Exception:
            self.fail('receipt_write')

    def _close_files(self):
        for name, stream in (('stdout_file', self.stdout.stream), ('stderr_file', self.stderr.stream),
                             ('receipt_file', self.receipt), ('intent_file', self.intent)):
            try:
                stream.close()
            except Exception:
                self.fail(name + '_close')
        if self.exited_confirmed:
            for name in ('stdout', 'stderr', 'stdin'):
                try:
                    pipe = getattr(self.process, name)
                    if not pipe.closed:
                        pipe.close()
                except Exception:
                    self.fail(name + '_close')
        self.closed = True


def _pin_stream(path, stream):
    stream.seek(0)
    digest, count = hashlib.sha256(), 0
    while True:
        block = stream.read(65536)
        if not block:
            break
        count += len(block)
        digest.update(block)
    _check_path(path)
    if os.stat(path, follow_symlinks=False).st_ino != os.fstat(stream.fileno()).st_ino:
        _reject('Output pathname no longer identifies original file')
    return {'Path': str(path), 'Bytes': count, 'Sha256': digest.hexdigest().upper()}


def _run(script, output, cap, log_prefix, intent, payload, validator):
    output, prefix = _local_path(output), _local_path(log_prefix)
    paths = [output, Path(str(prefix) + '.stderr.log'), Path(str(prefix) + '.intent.json'),
             Path(str(prefix) + '.transport.json')]
    if len({os.path.normcase(str(path)) for path in paths}) != len(paths):
        _reject('Distinct create-new output and transport evidence paths required')
    streams = []
    owner = None
    try:
        for path in paths:
            _check_path(path)
            if os.path.lexists(path):
                _reject('Existing output/attempt evidence refused; no overwrite/retry')
        for path in paths:
            path.parent.mkdir(parents=True, exist_ok=True)
            _check_path(path)
            streams.append(_new_file(path))
        stdout, stderr, intent_file, receipt = streams
        owner = _Owner(_Output(stdout, cap), _Output(stderr, STDERR_CAP), receipt, intent_file)
        owner.validate = lambda: validator(output, stdout)
        _write_all(intent_file, _encoded(dict(intent, SchemaVersion=1, Target=TARGET,
                   Host=HOST, Sid=SID, Phase='BeforePopen', LaunchAttempted=False, StdoutCap=cap,
                   StderrCap=STDERR_CAP, DeadlineSeconds=TIMEOUT_SECONDS,
                   StartedUtc=datetime.datetime.now(datetime.timezone.utc).isoformat())))
        _durable(intent_file)
        encoded = base64.b64encode(script.encode('utf-16-le')).decode('ascii')
        command = 'powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ' + encoded
        if len(command) >= 30000:
            _reject('Encoded SSH command exceeds reviewed Windows command bound')
        argv = ['ssh', '-T', '-o', 'BatchMode=yes', '-o', 'StrictHostKeyChecking=yes',
                '-o', 'ConnectTimeout=10', '-o', 'ServerAliveInterval=5',
                '-o', 'ServerAliveCountMax=2', TARGET, command]
        owner.start(argv, payload)
        if not owner.setup_refused:
            owner.observe()
        if not owner.success:
            raise DeliveryTransportError('Delivery transport failed/unknown; retain original owner, no retry', owner=owner)
        return owner.result
    except DeliveryTransportError:
        if owner is None or not owner.launch_attempted:
            for stream in streams:
                stream.close()
        raise
    except Exception:
        if owner is None or not owner.launch_attempted:
            for stream in streams:
                stream.close()
        else:
            owner.fail('transport_observation')
        raise DeliveryTransportError('Delivery transport setup/observation failed', owner=owner) from None


def _ps(value):
    return "'" + value.replace("'", "''") + "'"


_REMOTE_COMMON = r"""
$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
if([Environment]::MachineName-cne'LAPTOP-NP49RG6H'-or
 [Security.Principal.WindowsIdentity]::GetCurrent().User.Value-cne
 'S-1-5-21-3080296863-1242338503-1346362208-1001'){throw 'Wrong delivery host/SID'}
function Assert-LiteralPath([string]$Path){
 if(-not$Path.StartsWith('D:\MoirenTest\',[StringComparison]::Ordinal)-or
 [IO.Path]::GetFullPath($Path)-cne$Path){throw 'Noncanonical delivery path'}
 $part=$Path
 while($part){
  if([IO.File]::Exists($part)-or[IO.Directory]::Exists($part)){
   if(([IO.File]::GetAttributes($part)-band[IO.FileAttributes]::ReparsePoint)-ne0){throw 'ReparsePoint refused'}
  }
  $part=[IO.Path]::GetDirectoryName($part)
 }
}
function Read-Hash([string]$Path){
 Assert-LiteralPath $Path
 $s=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
 $h=[Security.Cryptography.SHA256]::Create()
 try{[BitConverter]::ToString($h.ComputeHash($s)).Replace('-','')}
 finally{$h.Dispose();$s.Dispose()}
}
"""


def download_file(remote_path: str, destination: Path, *, user_reboot_confirmed: bool,
                  cap: int, log_prefix: Path):
    """Download one exact file to a create-new destination; never overwrite/retry."""
    _confirmation(user_reboot_confirmed)
    remote_path = _remote_path(remote_path)
    if type(cap) is not int or not 1 <= cap <= MAX_DOWNLOAD:
        _reject('Download cap must be an integer in 1..68157440')
    script = _REMOTE_COMMON + r"""
$path=__PATH__;Assert-LiteralPath $path
$inputFile=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
try{
 $length=$inputFile.Length;if($length-gt__CAP__){throw 'Remote file exceeds cap'}
 $out=[Console]::OpenStandardOutput();$buffer=New-Object byte[] 65536;$total=[long]0
 while(($n=$inputFile.Read($buffer,0,$buffer.Length))-gt0){
  $total+=$n;if($total-gt__CAP__-or$total-gt$length){throw 'Remote file changed/exceeded cap'}
  $out.Write($buffer,0,$n)
 }
 if($total-ne$length-or$inputFile.Length-ne$length){throw 'Remote file became short/changed'}
 $out.Flush()
}finally{$inputFile.Dispose()}
"""
    script = script.replace('__PATH__', _ps(remote_path)).replace('__CAP__', str(cap))
    return _run(script, destination, cap, log_prefix, {'Operation': 'Download', 'RemotePath': remote_path},
                b'', _pin_stream)


def finalize_delivery(remote_run: str, host_receipt: bytes, local_ack_sha256: str,
                      finalizer_path: str, finalizer_sha256: str, *,
                      dependency_pins, user_reboot_confirmed: bool, log_prefix: Path):
    """Upload one immutable receipt and invoke only the independently pinned finalizer.

    stdin is a known finite byte count, not ReadToEnd. A failed upload/finalize
    is retained, never removed/retried. Returning JSON is not native acceptance.
    """
    _confirmation(user_reboot_confirmed)
    remote_run, finalizer_path = _remote_path(remote_run), _remote_path(finalizer_path)
    local_ack_sha256, finalizer_sha256 = _sha(local_ack_sha256), _sha(finalizer_sha256)
    dependencies = _dependencies(finalizer_path, dependency_pins)
    _json_object(host_receipt)
    receipt_sha = hashlib.sha256(host_receipt).hexdigest().upper()
    upload = _remote_path(remote_run + '.delivery-host.json')
    script = _REMOTE_COMMON + r"""
$run=__RUN__;$upload=__UPLOAD__;$finalizer=__FINALIZER__
Assert-LiteralPath $run;Assert-LiteralPath $upload;Assert-LiteralPath $finalizer
if(-not[IO.Directory]::Exists($run)){throw 'Original run directory missing'}
$scripts=@(@{Path=$finalizer;Bytes=$null;Sha256='__FINALIZER_SHA__'},__DEPENDENCIES__)
$scriptHandles=New-Object 'Collections.Generic.List[object]'
try{
foreach($pin in $scripts){
 Assert-LiteralPath $pin.Path
 $handle=[IO.File]::Open($pin.Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
 $scriptHandles.Add($handle)
 if($handle.Length-le0-or$handle.Length-gt1048576-or
  ($null-ne$pin.Bytes-and$handle.Length-ne$pin.Bytes)){throw 'Script length pin differs'}
 $hasher=[Security.Cryptography.SHA256]::Create()
 try{$actual=[BitConverter]::ToString($hasher.ComputeHash($handle)).Replace('-','')}
 finally{$hasher.Dispose()}
 if($actual-cne$pin.Sha256){throw 'Script hash pin differs'}
}
$s=[IO.File]::Open($upload,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
try{
 $inputStream=[Console]::OpenStandardInput();$buffer=New-Object byte[] 65536;$remaining=[long]__LENGTH__
 while($remaining-gt0){
  $n=$inputStream.Read($buffer,0,[int][Math]::Min($remaining,$buffer.Length))
  if($n-le0){throw 'Receipt input became short'}
  $s.Write($buffer,0,$n);$remaining-=$n
 }
 $s.Flush($true)
}finally{$s.Dispose()}
if((Get-Item -LiteralPath $upload -Force).Length-ne__LENGTH__-or
 (Read-Hash $upload)-cne'__RECEIPT_SHA__'){throw 'Receipt readback differs'}
. $finalizer
$result=Confirm-RemovalRunDelivery -RunDirectory $run -DeliveryReceiptPath $upload `
 -ExpectedDeliveryReceiptSha256 '__RECEIPT_SHA__' -ExpectedLocalAckSha256 '__ACK__' -UserRebootConfirmed
[Console]::Write(($result|ConvertTo-Json -Depth 16 -Compress))
}finally{foreach($handle in $scriptHandles){$handle.Dispose()}}
"""
    dependency_script = ','.join('@{Path=' + _ps(pin['Path']) + ';Bytes=[long]' +
                                 str(pin['Bytes']) + ";Sha256='" + pin['Sha256'] + "'}"
                                 for pin in dependencies)
    for key, value in {'__RUN__': _ps(remote_run), '__UPLOAD__': _ps(upload),
                       '__FINALIZER__': _ps(finalizer_path), '__FINALIZER_SHA__': finalizer_sha256,
                       '__LENGTH__': str(len(host_receipt)), '__RECEIPT_SHA__': receipt_sha,
                       '__ACK__': local_ack_sha256, '__DEPENDENCIES__': dependency_script}.items():
        script = script.replace(key, value)

    def validate(path, stream):
        _pin_stream(path, stream)
        stream.seek(0)
        return _json_object(stream.read(MAX_JSON + 1))

    return _run(script, Path(str(log_prefix) + '.stdout.log'), MAX_JSON, log_prefix,
                {'Operation': 'Finalize', 'RemoteRun': remote_run, 'Upload': upload,
                 'ReceiptBytes': len(host_receipt), 'ReceiptSha256': receipt_sha,
                 'FinalizerPath': finalizer_path, 'FinalizerSha256': finalizer_sha256,
                 'DependencyPins': dependencies,
                 'LocalAckSha256': local_ack_sha256}, host_receipt, validate)
