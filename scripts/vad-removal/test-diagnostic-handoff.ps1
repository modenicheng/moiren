#requires -Version 5.1
# Real Finish/seal/export -> retained entry Process -> run_diagnostic ->
# deliver_run -> target finalizer. Baseline/native owners and config admission
# remain explicit existing fixtures. The local launch adapter emits an ORIGINAL
# protected production export envelope; it is not the native diagnostic entry,
# fresh BEFORE admission, SSH transport or a release/qualification test.
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName-ceq'LAPTOP-NP49RG6H'){throw 'Diagnostic handoff composition is development-host only.'}
$template=Join-Path $PSScriptRoot 'test-delivery-composition.ps1'
$tokens=$null;$parseErrors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($template,[ref]$tokens,[ref]$parseErrors)
if($parseErrors.Count){throw 'Existing delivery composition fixture did not parse.'}
$loops=@($ast.EndBlock.Statements|Where-Object{$_-is[Management.Automation.Language.ForEachStatementAst]-and
    $_.Variable.VariablePath.UserPath-ceq'mode'})
if($loops.Count-ne1){throw 'Unique existing delivery composition case boundary required.'}
# Reuse setup/native fixtures and the exact local byte-transfer/finalizer
# adapters, without executing any case from the original suites.
$prefix=$ast.Extent.Text.Substring(0,$loops[0].Extent.StartOffset)
$variables=@($ast.FindAll({param($node) $node-is[Management.Automation.Language.VariableExpressionAst]-and
    $node.VariablePath.UserPath-ceq'PSScriptRoot'-and$node.Extent.StartOffset-lt$prefix.Length},$true)|
    Sort-Object {$_.Extent.StartOffset} -Descending)
foreach($variable in $variables){
    $start=$variable.Extent.StartOffset;$length=$variable.Extent.EndOffset-$start
    $prefix=$prefix.Substring(0,$start)+("'"+$PSScriptRoot.Replace("'","''")+"'")+$prefix.Substring($start+$length)
}
. ([scriptblock]::Create($prefix))
$hostDiagnostic=Join-Path ([IO.Directory]::GetParent($PSScriptRoot).FullName) 'vad-diagnostic.py'
$entrySource=Join-Path $PSScriptRoot 'entry-launch.ps1'
$shellPath=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$shellPin=Pin $shellPath
$script:handoffRoot=Join-Path $PSScriptRoot ('logs\diagnostic-handoff\'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($script:handoffRoot)
$script:fixtureRoot=$script:handoffRoot

$childEmitter=@'
param([string]$RunDirectory,[string]$ManifestPath,[string]$ManifestSha256,[switch]$UserRebootConfirmed,[int]$WorkerSessionId)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$config=[IO.File]::ReadAllText('__CONFIG__')|ConvertFrom-Json
if(-not$UserRebootConfirmed-or$RunDirectory-cne$config.RunDirectory-or$ManifestPath-cne$config.ManifestPin.Path-or
 $ManifestSha256-cne$config.ManifestPin.Sha256-or$WorkerSessionId-ne$config.WorkerSessionId){throw 'Foreign harmless-child invocation'}
$inputFile=[IO.File]::Open('__ENVELOPE__',[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
try{
 if($inputFile.Length-gt1048576){throw 'Original export envelope cap'}
 $sha=[Security.Cryptography.SHA256]::Create()
 try{$hash=[BitConverter]::ToString($sha.ComputeHash($inputFile)).Replace('-','')}finally{$sha.Dispose()}
 if($hash-cne'__ENVELOPE_SHA__'){throw 'Original export envelope changed'}
 $inputFile.Position=0;$inputFile.CopyTo([Console]::OpenStandardOutput())
}finally{$inputFile.Dispose()}
[Console]::Error.Write('Harmless local emitter: sealed controller evidence remains unsafe; no native diagnostic ran.')
exit 1
'@
$localLauncher=@'
param([string]$Source,[string]$ConfigPath,[string]$ChildSource,[string]$Envelope)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$config=[IO.File]::ReadAllText($ConfigPath)|ConvertFrom-Json
$handles=New-Object 'Collections.Generic.List[object]'
function Hold($Pin){
 $handle=[IO.File]::Open($Pin.Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read);$handles.Add($handle)
 if($handle.Length-ne$Pin.Bytes){throw 'Fixture independent length pin differs'}
 $sha=[Security.Cryptography.SHA256]::Create()
 try{$hash=[BitConverter]::ToString($sha.ComputeHash($handle)).Replace('-','')}finally{$sha.Dispose()}
 if($hash-cne$Pin.Sha256){throw 'Fixture independent SHA256 differs'}
}
try{
 Hold $config.ManifestPin;foreach($pin in $config.Sources){Hold $pin};Hold $config.PowerShellPin
 $handles.Add([IO.File]::Open($Envelope,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read))
 if($Source-cne(Join-Path $config.SourceDirectory 'entry-launch.ps1')){throw 'Foreign original wrapper source'}
 . $Source
 function Assert-RemovalDiagnosticEntryTarget([string]$RunDirectory){
  if($RunDirectory-cne$config.RunDirectory){throw 'Only this already-closed synthetic run may launch'}
 }
 function Get-RemovalDiagnosticEntryScript([string]$SourceDirectory){
  if($SourceDirectory-cne$config.SourceDirectory){throw 'Foreign source directory'}
  return $ChildSource
 }
 $pin=Invoke-RemovalDiagnosticEntry -RunDirectory $config.RunDirectory -ManifestPath $config.ManifestPin.Path `
  -ManifestSha256 $config.ManifestPin.Sha256 -SourceDirectory $config.SourceDirectory -PowerShellPin $config.PowerShellPin `
  -UserRebootConfirmed -WorkerSessionId $config.WorkerSessionId
 [Console]::Write(($pin|ConvertTo-Json -Depth 8 -Compress))
}finally{foreach($handle in $handles){$handle.Dispose()}}
'@

# Keep the already-tested delivery adapters byte-for-byte; only the outer
# launch/config boundaries below are new host-only fixture adapters.
$adapterStart=$receive.IndexOf('calls=[]',[StringComparison]::Ordinal)
$adapterEnd=$receive.IndexOf('dependencies=[]',[StringComparison]::Ordinal)
if($adapterStart-lt0-or$adapterEnd-le$adapterStart-or$adapterStart-ne$receive.LastIndexOf('calls=[]',[StringComparison]::Ordinal)-or
    $adapterEnd-ne$receive.LastIndexOf('dependencies=[]',[StringComparison]::Ordinal)){throw 'Exact reusable local delivery adapter boundaries changed.'}
$deliveryAdapters=$receive.Substring($adapterStart,$adapterEnd-$adapterStart)
$handoffPython=@'
import copy,hashlib,importlib.util,json,os,pathlib,subprocess,sys
source,config_path,ack_hash,output,engine,launcher,wrapper,finalizer,child,envelope=sys.argv[1:]
spec=importlib.util.spec_from_file_location('actual_diagnostic_handoff',source)
handoff=importlib.util.module_from_spec(spec);spec.loader.exec_module(handoff)
module=handoff.DELIVERY
config=module.strict_json(pathlib.Path(config_path).read_bytes())
run=config['RunDirectory']
original_popen=subprocess.Popen
def local_process_only(argv,*args,**kwargs):
    if not isinstance(argv,list) or argv[0]!=engine:
        raise AssertionError('Remote/unexpected executable prohibited in composition')
    return original_popen(argv,*args,**kwargs)
subprocess.Popen=local_process_only
def fixture_run(value):
    if value not in (run,finalizer): raise ValueError('Exact local fixture run/finalizer required')
    return value
module.canonical_run=fixture_run
def prepare_fixture(**options):
    if options!={'manifest_copy':pathlib.Path(config['ManifestPin']['Path']),
       'expected_manifest_sha256':config['ManifestPin']['Sha256'],
       'remote_manifest':config['ManifestPin']['Path'],'remote_run':run,
       'powershell_pin':config['PowerShellPin'],'worker_session_id':config['WorkerSessionId']}:
        raise ValueError('Original independently pinned fixture config changed')
    module.schema(config)
    module.matches_bytes(config['ManifestPin'],pathlib.Path(config['ManifestPin']['Path']).read_bytes())
    names=set()
    for pin in config['Sources']:
        module.pin(pin,module.ACK_CAP,named=True)
        if pin['Name'] in names: raise ValueError('Duplicate fixture source pin')
        names.add(pin['Name']);module.matches_bytes(pin,pathlib.Path(pin['Path']).read_bytes())
    if names!=handoff.REQUIRED_SOURCES: raise ValueError('Complete actual source membership required')
    module.matches_bytes(config['PowerShellPin'],pathlib.Path(config['PowerShellPin']['Path']).read_bytes())
    return copy.deepcopy(config)
# Deliberate fixture-only admission: the production seal's original inputs are
# untouched, and the harmless child lives outside the production source tree.
# Do not claim production prepare_config or the remote bootstrap executed.
handoff.prepare_config=prepare_fixture
entry_calls=[];entry_downloads=[];delivery_calls=[]
def launch(original,terminal_path,**options):
    if original!=config or entry_calls or terminal_path!=run+'.entry-terminal.json' or options['user_reboot_confirmed'] is not True:
        raise ValueError('Changed/duplicate original entry launch')
    entry_calls.append(terminal_path)
    completed=subprocess.run([engine,'-NoLogo','-NoProfile','-NonInteractive','-File',launcher,
        '-Source',next(p['Path'] for p in config['Sources'] if p['Name']=='entry-launch.ps1'),
        '-ConfigPath',config_path,'-ChildSource',child,'-Envelope',envelope],
        stdout=subprocess.PIPE,stderr=subprocess.PIPE,check=False)
    if completed.returncode: raise ValueError(completed.stderr.decode('utf-8',errors='replace'))
    return module.strict_json(completed.stdout.decode('utf-8-sig').encode('utf-8'))
def entry_download(remote,destination,**options):
    expected=[run+'.entry-terminal.json',run+'.entry-stdout.log',run+'.entry-stderr.log']
    if len(entry_downloads)>=len(expected) or remote!=expected[len(entry_downloads)] or options['user_reboot_confirmed'] is not True:
        raise ValueError('Foreign/out-of-order entry evidence download')
    raw=pathlib.Path(remote).read_bytes()
    if len(raw)>options['cap']: raise ValueError('Entry download cap exceeded')
    with pathlib.Path(destination).open('xb') as stream:
        stream.write(raw);stream.flush();os.fsync(stream.fileno())
    entry_downloads.append(remote)
__DELIVERY_ADAPTERS__
def deliver(original,original_ack,output_directory,source,source_sha,**options):
    if delivery_calls or original!=run or original_ack!=ack_hash:
        raise ValueError('Host did not retain the original independent local acknowledgment')
    delivery_calls.append(original_ack)
    return module.deliver_run(original,original_ack,output_directory,source,source_sha,
        dependency_pins=options['dependency_pins'],user_reboot_confirmed=options['user_reboot_confirmed'],
        download=local_transport,finalize=local_finalize)
result=handoff.run_diagnostic(manifest_copy=pathlib.Path(config['ManifestPin']['Path']),
    expected_manifest_sha256=config['ManifestPin']['Sha256'],remote_manifest=config['ManifestPin']['Path'],
    remote_run=run,powershell_pin=config['PowerShellPin'],output_directory=pathlib.Path(output),
    user_reboot_confirmed=True,worker_session_id=config['WorkerSessionId'],
    launch=launch,download=entry_download,deliver=deliver)
if len(entry_calls)!=1 or len(entry_downloads)!=3 or len(delivery_calls)!=1 or len(calls)!=4:
    raise ValueError('Original entry/evidence/delivery operation count differs')
if result['EntryExitCode']!=1 or result['SourceLocalAckPin']['Sha256']!=ack_hash:
    raise ValueError('Closed failed evidence was relabeled or current acknowledgment adopted')
print(json.dumps(result,separators=(',',':')))
'@
$handoffPython=$handoffPython.Replace('__DELIVERY_ADAPTERS__',$deliveryAdapters)

foreach($mode in @('normal','reader-refusal')){
    Case ('real-entry-host-handoff-and-delivery-finalizer-'+$mode) {
        $pair=$null;$protected=$null
        try{
            $pair=New-ClosedFinishFixture $mode;$o=$pair.Controller;$f=$pair.Fixture
            $o|Add-Member NoteProperty Refused $false
            $null=Complete-ClosedFinishFixture $pair;Save-RemovalControllerResult $o
            Check ($o.Completed-and$o.RetainedLease.Closed-and-not$o.Safe) 'Original failed controller did not close.'
            $export=Export-RemovalClosedRun $o ($o.RunDirectory+'.zip')
            $local=Confirm-RemovalClosedRunExport $export ($o.RunDirectory+'.archive-local.json')
            $envelope=Join-Path $f.Root 'original-entry-envelope.json'
            $payload=[ordered]@{ControllerResult=(Get-RemovalControllerResult $o);Archive=$export;LocalAcknowledgement=$local;Qualified=$false}
            Write-Text $envelope (ConvertTo-Json -InputObject $payload -Depth 10 -Compress)
            $envelopePin=Pin $envelope
            $protected=[IO.File]::Open($envelope,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
            $configPath=Join-Path $f.Root 'handoff-fixture-config.json'
            $child=Join-Path $f.Root 'invoke-diagnostic.ps1'
            $body=$childEmitter.Replace('__CONFIG__',$configPath.Replace("'","''")).Replace('__ENVELOPE__',$envelope.Replace("'","''")).Replace('__ENVELOPE_SHA__',$envelopePin.Sha256)
            Write-Text $child $body
            $sources=@(foreach($name in (Get-RemovalRunSourceNames)){
                $path=if($name-ceq'invoke-diagnostic.ps1'){$child}else{Join-Path $PSScriptRoot $name}
                $pin=Pin $path;[ordered]@{Name=$name;Path=$pin.Path;Bytes=[long]$pin.Bytes;Sha256=$pin.Sha256}
            })
            $config=[ordered]@{SchemaVersion=1;RunDirectory=$o.RunDirectory;ManifestPin=$o.RetainedInputs.ManifestPin
                SourceDirectory=$PSScriptRoot;SourcesRevision=('f'*40);Sources=$sources;PowerShellPin=$shellPin;WorkerSessionId=2}
            Write-Text $configPath (ConvertTo-Json -InputObject $config -Depth 12 -Compress)
            $launcher=Join-Path $f.Root 'entry-launch-fixture.ps1';Write-Text $launcher $localLauncher
            $target=Join-Path $f.Root 'target-finalizer-fixture.ps1';Write-Text $target $targetWrapper
            $hostDirectory=Join-Path $f.Root 'host-handoff'
            $counts=@($f.Calls.Count,$f.ManagedSession.TaskOwner.Folder.DeleteCalls)
            $hostOutput=&$python -B -c $handoffPython $hostDiagnostic $configPath $local.Sha256 $hostDirectory $engine $launcher $target $finalizerSource $child $envelope 2>&1
            $hostExit=$LASTEXITCODE
            Write-Text (Join-Path $f.Root 'host-handoff.log') ($hostOutput-join[Environment]::NewLine)
            Check ($hostExit-eq0) ('Actual diagnostic handoff failed: '+($hostOutput-join'; '))
            $terminal=Read-RemovalDeliveryJson ($o.RunDirectory+'.entry-terminal.json') 1048576
            $handoff=Read-RemovalDeliveryJson (Join-Path $hostDirectory 'handoff-complete.json') 1048576
            $deliveryDirectory=Join-Path $hostDirectory 'delivery'
            $hostReceipt=Read-RemovalDeliveryJson (Join-Path $deliveryDirectory 'host-delivery.json') 1048576
            $delivered=Read-RemovalDeliveryJson ($o.RunDirectory+'.archive-delivered.json') 2097152
            $completed=Read-RemovalDeliveryJson (Join-Path $deliveryDirectory 'delivery-complete.json') 1048576
            Check ($terminal.Value.ExitCode-eq1-and$terminal.Value.OutputStable-and$terminal.Value.Errors.Count-eq0-and
                $terminal.Value.Process.ExitedConfirmed-and$terminal.Value.LocalAcknowledgementPin.Sha256-ceq$local.Sha256) 'Original child identity/stdout acknowledgment binding lost.'
            Check ($terminal.Value.EntrySourcePin.Sha256-ceq(Pin $entrySource).Sha256-and
                $terminal.Value.ChildSourcePin.Sha256-ceq(Pin $child).Sha256) 'Actual wrapper/harmless child identity differs.'
            Check ($handoff.Value.Meaning-ceq'DIAGNOSTIC_ENTRY_DELIVERY_COMPLETE'-and$handoff.Value.EntryExitCode-eq1-and
                $handoff.Value.EntryTerminalPin.Sha256-ceq$terminal.Pin.Sha256-and$handoff.Value.SourceLocalAckPin.Sha256-ceq$local.Sha256) 'Host handoff lost independently retained original pins.'
            Check ($hostReceipt.Value.Transferred-and$delivered.Value.DeliveryReceiptPin.Sha256-ceq$hostReceipt.Pin.Sha256-and
                $completed.Value.TargetAcknowledgmentPin.Sha256-ceq$delivered.Pin.Sha256-and
                $delivered.Value.ResultPin.Sha256-ceq$o.ResultPin.Sha256) 'Actual target delivery receipt/result chain differs.'
            foreach($value in @($terminal.Value,$handoff.Value,$hostReceipt.Value,$delivered.Value,$completed.Value)){
                Check (-not$value.NativePassed-and-not$value.Qualified) 'Composition promoted native qualification.'
            }
            Check (-not$handoff.Value.CleanupAuthorized-and-not$completed.Value.CleanupAuthorized) 'Composition authorized cleanup.'
            Check ((Pin $local.Path).Sha256-ceq$local.Sha256-and(Pin $envelope).Sha256-ceq$envelopePin.Sha256) 'Original acknowledgment/envelope changed.'
            Check ($f.Calls.Count-eq$counts[0]-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq$counts[1]) 'Handoff repeated trace/decoder/task retirement.'
            Reject {Assert-RemovalDiagnosticRelease}
        }finally{if($null-ne$protected){$protected.Dispose()};Close-ClosedFinishFixture $pair}
    }
}
$checks|ConvertTo-Json -Depth 7
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ENTRY_WRAPPER=production HOST_HANDOFF=production DELIVERY=production TARGET_FINALIZE=production CONFIG=explicit-fixture READINESS=InjectedCollectorSource TRANSPORT=local-files NATIVE_OPERATIONS=0')
Write-Output ('FIXTURE_ROOT='+$script:fixtureRoot)
if($failed){exit 1}
