# SPDX-License-Identifier: MIT
"""Launch one pinned removal diagnostic and deliver its closed evidence.

The target entry and the complete source set are locked before the first import.
An exit-1 closed diagnostic remains evidence worth delivering, not a native pass.
No target operation is attempted without explicit user reboot confirmation.
"""
import argparse
from contextlib import ExitStack
from functools import partial
import importlib.util
import json
import ntpath
import os
from pathlib import Path


def _module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


DELIVERY = _module('diagnostic_delivery', 'vad-delivery.py')
TRANSPORT = _module('diagnostic_transport', 'vad-delivery-transport.py')
require = DELIVERY.require
REQUIRED_SOURCES = frozenset((
    'closed-export.ps1', 'collector-boundary.ps1', 'collector-worker.ps1',
    'controller-seal.ps1', 'delivery-finalize.ps1', 'diagnostic-controller.ps1',
    'diagnostic-lease.ps1', 'entry-launch.ps1', 'finish-session.ps1', 'finite-tool.ps1',
    'invoke-diagnostic.ps1', 'invoke-preflight.ps1', 'native-admission.ps1',
    'preflight-session.ps1', 'probe-boundary.ps1', 'readonly-preflight.ps1',
    'run-inputs.ps1', 'run-manifest.ps1', 'safety-audit.ps1', 'session-identity.ps1',
    'target-readiness.ps1', 'task-boundary.ps1', 'trace-boundary.ps1', 'trace-decode.ps1',
    'trace-native.ps1', 'trace-query.ps1', 'trace-readback.ps1', 'trace-session.ps1',
    'worker-boundary.ps1', 'worker.ps1'))


def prepare_config(*, manifest_copy, expected_manifest_sha256, remote_manifest,
                   remote_run, powershell_pin, worker_session_id=0):
    run = DELIVERY.canonical_run(remote_run)
    manifest_path = DELIVERY.canonical_run(remote_manifest)
    require(type(worker_session_id) is int and 0 <= worker_session_id <= 65535,
            'bounded original worker session id required')
    DELIVERY.pin(powershell_pin, 268435456)
    require(ntpath.basename(powershell_pin['Path']) == 'powershell.exe'
            and powershell_pin['Bytes'] > 0, 'original Windows PowerShell executable pin required')
    with ExitStack() as stack:
        stream = DELIVERY.open_input(stack, Path(manifest_copy), DELIVERY.ACK_CAP)
        length, sha, raw = DELIVERY.read_stream(stream, DELIVERY.ACK_CAP, True)
        require(sha == DELIVERY.hash_text(expected_manifest_sha256),
                'independent reviewed manifest SHA256 differs')
        manifest = DELIVERY.strict_json(raw)
    DELIVERY.schema(manifest)
    revision = manifest.get('SourcesRevision')
    require(isinstance(revision, str) and len(revision) == 40
            and all(char in '0123456789abcdef' for char in revision),
            'reviewed source revision required')
    sources = manifest.get('Sources')
    require(isinstance(sources, list) and len(REQUIRED_SOURCES) <= len(sources) <= 64,
            'complete bounded reviewed production source set required')
    names, directory, total = set(), None, 0
    for source in sources:
        DELIVERY.pin(source, DELIVERY.ACK_CAP, named=True)
        require(source['Bytes'] > 0 and source['Name'].casefold() not in names,
                'nonempty unique source pins required')
        names.add(source['Name'].casefold())
        parent = DELIVERY.canonical_run(ntpath.dirname(source['Path']))
        directory = parent if directory is None else directory
        require(parent == directory, 'all sources must use one exact reviewed directory')
        total += source['Bytes']
    require(REQUIRED_SOURCES <= {item['Name'] for item in sources}
            and total <= DELIVERY.FILE_CAP, 'required source membership/aggregate differs')
    require(manifest_path not in {item['Path'] for item in sources}
            and not run.startswith(directory + '\\') and run != directory,
            'run evidence cannot overwrite reviewed source/manifest inputs')
    return {'SchemaVersion': 1, 'RunDirectory': run,
            'ManifestPin': {'Path': manifest_path, 'Bytes': length, 'Sha256': sha},
            'SourceDirectory': directory, 'SourcesRevision': revision,
            'Sources': sources, 'PowerShellPin': powershell_pin,
            'WorkerSessionId': worker_session_id}


_BOOTSTRAP = TRANSPORT._REMOTE_COMMON + r"""
if(-not([Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole(
 [Security.Principal.WindowsBuiltInRole]::Administrator)){throw 'Already elevated controller required; no automatic elevation'}
$payload=New-Object byte[] __LENGTH__;$inputStream=[Console]::OpenStandardInput();$offset=0
while($offset-lt$payload.Length){
 $n=$inputStream.Read($payload,$offset,$payload.Length-$offset)
 if($n-le0){throw 'Entry configuration input became short'};$offset+=$n
}
$hasher=[Security.Cryptography.SHA256]::Create()
try{$hash=[BitConverter]::ToString($hasher.ComputeHash($payload)).Replace('-','')}
finally{$hasher.Dispose()}
if($hash-cne'__CONFIG_SHA__'){throw 'Independent entry configuration hash differs'}
$config=[Text.UTF8Encoding]::new($false,$true).GetString($payload)|ConvertFrom-Json
$handles=New-Object 'Collections.Generic.List[object]'
function Hold-Original($Pin,[long]$Cap,[switch]$SystemExecutable){
 if(-not$SystemExecutable){Assert-LiteralPath $Pin.Path}
 elseif([IO.Path]::GetFullPath($Pin.Path)-cne$Pin.Path-or$Pin.Path-cnotmatch'\A[A-Z]:\\'){throw 'Exact executable path required'}
 $cursor=$Pin.Path
 while($cursor){
  if(([IO.File]::Exists($cursor)-or[IO.Directory]::Exists($cursor))-and
   (([IO.File]::GetAttributes($cursor)-band[IO.FileAttributes]::ReparsePoint)-ne0)){throw 'Reparse executable/source path refused'}
  $cursor=[IO.Path]::GetDirectoryName($cursor)
 }
 $handle=[IO.File]::Open($Pin.Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
 $handles.Add($handle)
 if($handle.Length-ne$Pin.Bytes-or$handle.Length-le0-or$handle.Length-gt$Cap){throw 'Original file length differs'}
 $hasher=[Security.Cryptography.SHA256]::Create()
 try{$actual=[BitConverter]::ToString($hasher.ComputeHash($handle)).Replace('-','')}
 finally{$hasher.Dispose()}
 if($actual-cne$Pin.Sha256){throw 'Original file SHA256 differs'}
 $handle.Position=0
}
try{
 Assert-LiteralPath $config.RunDirectory;Assert-LiteralPath $config.SourceDirectory
 if([IO.Directory]::Exists($config.RunDirectory)-or[IO.File]::Exists($config.RunDirectory)){throw 'Create-new diagnostic run required'}
 Hold-Original $config.ManifestPin 1048576
 $manifestHandle=$handles[0];$bytes=New-Object byte[] $manifestHandle.Length
 $offset=0;while($offset-lt$bytes.Length){$n=$manifestHandle.Read($bytes,$offset,$bytes.Length-$offset);if($n-le0){throw 'Short original manifest'};$offset+=$n}
 $manifest=[Text.UTF8Encoding]::new($false,$true).GetString($bytes)|ConvertFrom-Json
 if($manifest.SourcesRevision-cne$config.SourcesRevision-or$manifest.Sources.Count-ne$config.Sources.Count){throw 'Manifest/source configuration binding differs'}
 $seen=@{};$entry=$null
 foreach($pin in $config.Sources){
  if($seen.ContainsKey($pin.Name)-or$pin.Path-cne(Join-Path $config.SourceDirectory $pin.Name)){throw 'Duplicate/outside source pin'}
  $seen[$pin.Name]=$true
  $matches=@($manifest.Sources|Where-Object{$_.Name-ceq$pin.Name-and$_.Path-ceq$pin.Path-and$_.Bytes-eq$pin.Bytes-and$_.Sha256-ceq$pin.Sha256})
  if($matches.Count-ne1){throw 'Manifest original source binding differs'}
  Hold-Original $pin 1048576
  if($pin.Name-ceq'entry-launch.ps1'){$entry=$pin.Path}
 }
 $actual=@(Get-ChildItem -LiteralPath $config.SourceDirectory -Force -Filter '*.ps1'|Where-Object{$_.Name-notlike'test-*'})
 if($actual.Count-ne$config.Sources.Count){throw 'Unpinned production source membership'}
 foreach($file in $actual){if($file.PSIsContainer-or-not$seen.ContainsKey($file.Name)){throw 'Unpinned production source file'}}
 if($null-eq$entry){throw 'Original entry-launch dependency missing'}
 Hold-Original $config.PowerShellPin 268435456 -SystemExecutable
 . $entry
 $result=Invoke-RemovalDiagnosticEntry -RunDirectory $config.RunDirectory -ManifestPath $config.ManifestPin.Path `
  -ManifestSha256 $config.ManifestPin.Sha256 -SourceDirectory $config.SourceDirectory `
  -PowerShellPin $config.PowerShellPin -UserRebootConfirmed -WorkerSessionId $config.WorkerSessionId
 [Console]::Write(($result|ConvertTo-Json -Depth 12 -Compress))
}finally{foreach($handle in $handles){$handle.Dispose()}}
"""


def launch_entry(config, terminal_path, *, user_reboot_confirmed, log_prefix, file_handoff=None):
    require(user_reboot_confirmed is True, 'user reboot confirmation required; no target access')
    DELIVERY.pin(config['ManifestPin'], DELIVERY.ACK_CAP)
    DELIVERY.canonical_run(terminal_path)
    require(terminal_path == config['RunDirectory'] + '.entry-terminal.json',
            'original entry terminal path required')
    payload = json.dumps(config, separators=(',', ':'), allow_nan=False).encode('utf-8')
    require(len(payload) <= DELIVERY.ACK_CAP, 'bounded entry configuration required')
    script = _BOOTSTRAP.replace('__LENGTH__', str(len(payload))).replace(
        '__CONFIG_SHA__', DELIVERY.digest(payload))

    def validate(path, stream):
        TRANSPORT._pin_stream(path, stream)
        stream.seek(0)
        result = DELIVERY.strict_json(stream.read(DELIVERY.ACK_CAP + 1))
        DELIVERY.pin(result, DELIVERY.ACK_CAP, terminal_path)
        return result

    if file_handoff is not None:
        return TRANSPORT.invoke_file_handoff('Entry', config,
                    Path(str(log_prefix) + '.stdout.log'), DELIVERY.ACK_CAP, log_prefix, validate,
                    file_handoff=file_handoff, user_reboot_confirmed=True, timeout_seconds=600.0)
    return TRANSPORT._run(script, Path(str(log_prefix) + '.stdout.log'), DELIVERY.ACK_CAP,
                          log_prefix, {'Operation': 'DiagnosticEntry', 'RunDirectory': config['RunDirectory'],
                                       'ManifestPin': config['ManifestPin']}, payload, validate,
                          timeout_seconds=600.0)


def _terminal(value, config):
    DELIVERY.schema(value)
    require(value.get('Meaning') == 'ORIGINAL_DIAGNOSTIC_ENTRY_TERMINAL'
            and value.get('Target') == 'moiren-lan'
            and value.get('RunDirectory') == config['RunDirectory'], 'original entry terminal identity differs')
    require(value.get('ManifestPin') == config['ManifestPin'], 'original entry manifest binding differs')
    entry = next(item for item in config['Sources'] if item['Name'] == 'entry-launch.ps1')
    require(value.get('EntrySourcePin') == {key: entry[key] for key in ('Path', 'Bytes', 'Sha256')},
            'original entry source binding differs')
    child = next(item for item in config['Sources'] if item['Name'] == 'invoke-diagnostic.ps1')
    require(value.get('ChildSourcePin') == {key: child[key] for key in ('Path', 'Bytes', 'Sha256')},
            'original diagnostic child source binding differs')
    require(value.get('NativePassed') is False and value.get('Qualified') is False,
            'entry cannot promote native or physical qualification')
    require(type(value.get('ExitCode')) is int and value['ExitCode'] in (0, 1)
            and value.get('TimedOut') is False and value.get('OutputStable') is True
            and value.get('Errors') == [], 'entry execution failed/unknown; original evidence retained')
    process = value.get('Process')
    require(isinstance(process, dict) and type(process.get('Pid')) is int and process['Pid'] > 0
            and type(process.get('CreationUtcTicks')) is int and process['CreationUtcTicks'] > 0
            and process.get('ExitedConfirmed') is True, 'original process terminal identity required')
    run = config['RunDirectory']
    for field, suffix in (('StdoutPin', '.entry-stdout.log'), ('StderrPin', '.entry-stderr.log')):
        DELIVERY.pin(value.get(field), DELIVERY.ACK_CAP, run + suffix)
    return DELIVERY.pin(value.get('LocalAcknowledgementPin'), DELIVERY.ACK_CAP,
                        run + '.archive-local.json')


def run_diagnostic(*, manifest_copy, expected_manifest_sha256, remote_manifest,
                   remote_run, powershell_pin, output_directory, user_reboot_confirmed,
                   worker_session_id=0, launch=None, download=None, deliver=None, file_handoff=None):
    require(user_reboot_confirmed is True, 'user reboot confirmation required; no target access')
    config = prepare_config(manifest_copy=manifest_copy,
                            expected_manifest_sha256=expected_manifest_sha256,
                            remote_manifest=remote_manifest, remote_run=remote_run,
                            powershell_pin=powershell_pin, worker_session_id=worker_session_id)
    output = DELIVERY.safe_local(output_directory)
    output.mkdir(exist_ok=False)
    if file_handoff is not None:
        file_download = partial(TRANSPORT.download_file, file_handoff=file_handoff)
        launch = partial(launch_entry, file_handoff=file_handoff) if launch is None else launch
        download = file_download if download is None else download
        if deliver is None:
            deliver = partial(DELIVERY.deliver_run, download=file_download,
                              finalize=partial(TRANSPORT.finalize_delivery, file_handoff=file_handoff))
    else:
        launch = launch_entry if launch is None else launch
        download = TRANSPORT.download_file if download is None else download
        deliver = DELIVERY.deliver_run if deliver is None else deliver
    terminal_path = remote_run + '.entry-terminal.json'
    terminal_pin = DELIVERY.retained_call(launch, config, terminal_path, user_reboot_confirmed=True,
                                          log_prefix=output / 'launch-entry')
    DELIVERY.pin(terminal_pin, DELIVERY.ACK_CAP, terminal_path)
    local_terminal = output / 'entry-terminal.json'
    DELIVERY.retained_call(download, terminal_path, local_terminal, user_reboot_confirmed=True,
                           cap=DELIVERY.ACK_CAP, log_prefix=output / 'download-entry-terminal')
    with ExitStack() as stack:
        stream = DELIVERY.open_input(stack, local_terminal, DELIVERY.ACK_CAP)
        _, _, raw = DELIVERY.read_stream(stream, DELIVERY.ACK_CAP, True)
        DELIVERY.matches_bytes(terminal_pin, raw)
        terminal = DELIVERY.strict_json(raw)
        acknowledgment = _terminal(terminal, config)
        for field, name in (('StdoutPin', 'entry-stdout.log'), ('StderrPin', 'entry-stderr.log')):
            expected = terminal[field]
            path = output / name
            DELIVERY.retained_call(download, expected['Path'], path, user_reboot_confirmed=True,
                                   cap=DELIVERY.ACK_CAP, log_prefix=output / ('download-' + name))
            log = DELIVERY.open_input(stack, path, DELIVERY.ACK_CAP)
            _, _, data = DELIVERY.read_stream(log, DELIVERY.ACK_CAP, True)
            DELIVERY.matches_bytes(expected, data)
            if field == 'StdoutPin':
                original = DELIVERY.strict_json(data)
                require(original.get('Qualified') is False
                        and original.get('LocalAcknowledgement') == acknowledgment,
                        'original child stdout acknowledgment binding differs; no current-file adoption')
        sources = {item['Name']: item for item in config['Sources']}
        finalizer = sources['delivery-finalize.ps1']
        dependencies = [{key: sources[name][key] for key in ('Path', 'Bytes', 'Sha256')}
                        for name in ('closed-export.ps1', 'finite-tool.ps1')]
        complete = DELIVERY.retained_call(deliver, remote_run, acknowledgment['Sha256'], output / 'delivery',
                                          finalizer['Path'], finalizer['Sha256'], dependency_pins=dependencies,
                                          user_reboot_confirmed=True)
        require(complete.get('Meaning') == 'CLOSED_RUN_DELIVERY_COMPLETE'
                and complete.get('RunDirectory') == remote_run
                and complete.get('SourceLocalAckPin') == acknowledgment
                and complete.get('Transferred') is True
                and complete.get('NativePassed') is False and complete.get('Qualified') is False
                and complete.get('CleanupAuthorized') is False, 'original closed-run delivery binding differs')
        require(DELIVERY.content_pin(local_terminal, stream, DELIVERY.ACK_CAP)['Sha256']
                == terminal_pin['Sha256'], 'original entry terminal changed during delivery')
        receipt = {'SchemaVersion': 1, 'Meaning': 'DIAGNOSTIC_ENTRY_DELIVERY_COMPLETE',
                   'Target': 'moiren-lan', 'RunDirectory': remote_run, 'ManifestPin': config['ManifestPin'],
                   'EntryTerminalPin': terminal_pin, 'EntryExitCode': terminal['ExitCode'],
                   'SourceLocalAckPin': acknowledgment, 'Delivery': complete, 'Transferred': True,
                   'NativePassed': False, 'Qualified': False, 'CleanupAuthorized': False}
        data = json.dumps(receipt, separators=(',', ':'), allow_nan=False).encode('utf-8')
        path = DELIVERY.safe_local(output / 'handoff-complete.json')
        with path.open('xb') as target:
            target.write(data)
            target.flush()
            os.fsync(target.fileno())
        require(path.read_bytes() == data, 'handoff receipt readback differs')
    return receipt


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest-copy', type=Path, required=True)
    parser.add_argument('--expected-manifest-sha256', required=True)
    parser.add_argument('--remote-manifest', required=True)
    parser.add_argument('--run-directory', required=True)
    parser.add_argument('--powershell-pin', type=Path, required=True)
    parser.add_argument('--expected-powershell-pin-sha256', required=True)
    parser.add_argument('--output-directory', type=Path, required=True)
    parser.add_argument('--worker-session-id', type=int, default=0)
    parser.add_argument('--user-reboot-confirmed', action='store_true')
    parser.add_argument('--file-handoff-helper', type=Path)
    parser.add_argument('--expected-file-handoff-helper-sha256')
    parser.add_argument('--file-handoff-parent')
    args = parser.parse_args(argv)
    require(args.user_reboot_confirmed is True, 'user reboot confirmation required; no target access')
    handoff_args = (args.file_handoff_helper, args.expected_file_handoff_helper_sha256, args.file_handoff_parent)
    require(not any(value is not None for value in handoff_args) or all(value is not None for value in handoff_args),
            'file handoff requires helper path, independent SHA256 and staging parent together')
    file_handoff = None if args.file_handoff_helper is None else {
        'HelperPath': str(args.file_handoff_helper),
        'ExpectedHelperSha256': args.expected_file_handoff_helper_sha256,
        'RemoteParent': args.file_handoff_parent}
    with ExitStack() as stack:
        stream = DELIVERY.open_input(stack, args.powershell_pin, DELIVERY.ACK_CAP)
        _, sha, raw = DELIVERY.read_stream(stream, DELIVERY.ACK_CAP, True)
        require(sha == DELIVERY.hash_text(args.expected_powershell_pin_sha256),
                'independent PowerShell pin SHA256 differs')
        powershell = DELIVERY.strict_json(raw)
        receipt = run_diagnostic(manifest_copy=args.manifest_copy,
                                 expected_manifest_sha256=args.expected_manifest_sha256,
                                 remote_manifest=args.remote_manifest, remote_run=args.run_directory,
                                 powershell_pin=powershell, output_directory=args.output_directory,
                                 user_reboot_confirmed=True, worker_session_id=args.worker_session_id,
                                 file_handoff=file_handoff)
    print(json.dumps(receipt, separators=(',', ':'), allow_nan=False))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
