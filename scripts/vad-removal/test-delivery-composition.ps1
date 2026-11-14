#requires -Version 5.1
# Real Finish/seal/export -> host receive/verify -> durable target receipt.
# Only existing native fixtures, the exact test run guard and byte transport
# are injected. No SSH, scheduler, driver, desktop or audio operation runs.
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName-ceq'LAPTOP-NP49RG6H'){throw 'Run delivery composition only on the development host.'}
$template=Join-Path $PSScriptRoot 'test-closed-finish.ps1'
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($template,[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'Original closed-Finish fixture did not parse.'}
$loops=@($ast.EndBlock.Statements|Where-Object{$_-is[Management.Automation.Language.ForEachStatementAst]-and
    $_.Variable.VariablePath.UserPath-ceq'mode'})
if($loops.Count-ne1){throw 'Unique original closed-Finish case boundary required.'}
# Import the already-reviewed fixture setup and definitions, not its cases.
$prefix=$ast.Extent.Text.Substring(0,$loops[0].Extent.StartOffset)
$variables=@($ast.FindAll({param($node) $node-is[Management.Automation.Language.VariableExpressionAst]-and
    $node.VariablePath.UserPath-ceq'PSScriptRoot'-and$node.Extent.StartOffset-lt$prefix.Length},$true)|
    Sort-Object {$_.Extent.StartOffset} -Descending)
foreach($variable in $variables){
    $start=$variable.Extent.StartOffset;$length=$variable.Extent.EndOffset-$start
    $literal="'"+$PSScriptRoot.Replace("'","''")+"'"
    $prefix=$prefix.Substring(0,$start)+$literal+$prefix.Substring($start+$length)
}
. ([scriptblock]::Create($prefix))
. (Join-Path $PSScriptRoot 'delivery-finalize.ps1')
function Assert-RemovalDeliveryTargetHost([string]$RunDirectory){
    Check ($RunDirectory-ceq$script:deliveryRun) 'Delivery target guard escaped exact local fixture.'
}
$python=(Get-Command python -CommandType Application -ErrorAction Stop|Select-Object -First 1).Source
$hostSource=Join-Path ([IO.Directory]::GetParent($PSScriptRoot).FullName) 'vad-delivery.py'
$engine=[Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
$finalizerSource=Join-Path $PSScriptRoot 'delivery-finalize.ps1'
$targetWrapper=@'
param([string]$Source,[string]$Run,[string]$Receipt,[string]$ReceiptSha,[string]$AckSha)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
. $Source
function Assert-RemovalDeliveryTargetHost([string]$RunDirectory){
    if($RunDirectory-cne$Run){throw 'Exact original host-only composition run required.'}
}
$pin=Confirm-RemovalRunDelivery -RunDirectory $Run -DeliveryReceiptPath $Receipt `
    -ExpectedDeliveryReceiptSha256 $ReceiptSha -ExpectedLocalAckSha256 $AckSha -UserRebootConfirmed
[Console]::Write(($pin|ConvertTo-Json -Compress))
'@
$receive=@'
import hashlib,importlib.util,json,pathlib,subprocess,sys
source,run,ack_hash,output,engine,wrapper,finalizer=sys.argv[1:]
spec=importlib.util.spec_from_file_location('closed_delivery_composition',source)
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
def fixture_run(value):
    if value not in (run,finalizer): raise ValueError('exact local fixture run/source required')
    return value
module.canonical_run=fixture_run
calls=[]
def local_transport(remote,destination,**options):
    expected=[run+'.archive-local.json',run+'.zip.inventory.json',run+'.zip',run+'.archive-delivered.json']
    if remote!=expected[len(calls)]: raise ValueError('duplicate/out-of-order fixture transport')
    if options['user_reboot_confirmed'] is not True: raise ValueError('confirmation lost')
    calls.append(remote)
    total=0
    with pathlib.Path(remote).open('rb') as original,pathlib.Path(destination).open('xb') as saved:
        while chunk:=original.read(65536):
            total+=len(chunk)
            if total>options['cap']: raise ValueError('fixture download cap exceeded')
            saved.write(chunk)
def local_finalize(original,raw,ack_sha,source,source_sha,**options):
    if original!=run or source!=finalizer: raise ValueError('foreign finalization request')
    if hashlib.sha256(pathlib.Path(source).read_bytes()).hexdigest().upper()!=source_sha:
        raise ValueError('original finalizer source changed')
    upload=pathlib.Path(run+'.delivery-host.json')
    with upload.open('xb') as stream: stream.write(raw)
    completed=subprocess.run([engine,'-NoLogo','-NoProfile','-NonInteractive','-File',wrapper,
        '-Source',source,'-Run',run,'-Receipt',str(upload),
        '-ReceiptSha',hashlib.sha256(raw).hexdigest().upper(),'-AckSha',ack_sha],
        stdout=subprocess.PIPE,stderr=subprocess.PIPE,check=False)
    if completed.returncode: raise ValueError(completed.stderr.decode('utf-8',errors='replace'))
    return json.loads(completed.stdout.decode('utf-8-sig'))
dependencies=[]
for name in ('closed-export.ps1','finite-tool.ps1'):
    path=pathlib.Path(finalizer).with_name(name);raw=path.read_bytes()
    dependencies.append({'Path':str(path),'Bytes':len(raw),'Sha256':hashlib.sha256(raw).hexdigest().upper()})
receipt=module.deliver_run(run,ack_hash,pathlib.Path(output),finalizer,
    hashlib.sha256(pathlib.Path(finalizer).read_bytes()).hexdigest().upper(),
    dependency_pins=dependencies,user_reboot_confirmed=True,download=local_transport,finalize=local_finalize)
if len(calls)!=4: raise ValueError('missing original archive/target-ack transfer')
print(json.dumps(receipt,separators=(',',':')))
'@
foreach($mode in @('normal','reader-refusal')){
    Case ('real-finish-export-host-delivery-target-ack-'+$mode) {
        $pair=$null
        try{
            $pair=New-ClosedFinishFixture $mode;$o=$pair.Controller;$f=$pair.Fixture
            $script:deliveryRun=$o.RunDirectory
            $null=Complete-ClosedFinishFixture $pair
            Save-RemovalControllerResult $o
            Check ($o.Completed-and$o.RetainedLease.Closed-and-not$o.Safe) 'Original Finish/controller refusal not closed.'
            $export=Export-RemovalClosedRun $o ($o.RunDirectory+'.zip')
            $local=Confirm-RemovalClosedRunExport $export ($o.RunDirectory+'.archive-local.json')
            $hostDirectory=Join-Path $f.Root 'host-received'
            $wrapperPath=Join-Path $f.Root 'target-finalizer-fixture.ps1'
            [IO.File]::WriteAllText($wrapperPath,$targetWrapper,[Text.UTF8Encoding]::new($false))
            $counts=@($f.Calls.Count,$f.ManagedSession.TaskOwner.Folder.DeleteCalls)
            $hostOutput=&$python -B -c $receive $hostSource $o.RunDirectory $local.Sha256 $hostDirectory $engine $wrapperPath $finalizerSource 2>&1
            $hostExit=$LASTEXITCODE
            [IO.File]::WriteAllText((Join-Path $f.Root 'host-receive.log'),($hostOutput-join[Environment]::NewLine))
            Check ($hostExit-eq0) ('Real host receive/verify failed: '+($hostOutput-join'; '))
            $hostPath=Join-Path $hostDirectory 'host-delivery.json'
            $hostReceipt=Read-RemovalDeliveryJson $hostPath 1048576
            $upload=$o.RunDirectory+'.delivery-host.json'
            Check ((Pin $upload).Sha256-ceq$hostReceipt.Pin.Sha256) 'Exact host receipt delivery/readback changed bytes.'
            $saved=Read-RemovalDeliveryJson ($o.RunDirectory+'.archive-delivered.json') 2097152
            $complete=Read-RemovalDeliveryJson (Join-Path $hostDirectory 'delivery-complete.json') 1048576
            Check ($complete.Value.Transferred-and-not$complete.Value.CleanupAuthorized-and
                $complete.Value.TargetAcknowledgmentPin.Sha256-ceq$saved.Pin.Sha256) 'Full host delivery completion/downloaded target ack missing.'
            $old=Read-RemovalDeliveryJson $local.Path 1048576
            Check ($old.Pin.Sha256-ceq$local.Sha256-and-not$old.Value.Transferred) 'Historical local ack was rewritten/promoted.'
            Check ($hostReceipt.Value.Transferred-and$saved.Value.Transferred-and
                $saved.Value.Meaning-ceq'TARGET_ARCHIVE_DELIVERY_ACKNOWLEDGED'-and
                $saved.Value.DeliveryReceiptPin.Sha256-ceq$hostReceipt.Pin.Sha256-and
                $saved.Value.ResultPin.Sha256-ceq$o.ResultPin.Sha256) 'Three-step original result/delivery binding differs.'
            foreach($value in @($old.Value,$hostReceipt.Value,$saved.Value)){
                Check (-not$value.NativePassed-and-not$value.Qualified) 'Delivery promoted native/physical qualification.'
            }
            Check ($f.Calls.Count-eq$counts[0]-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq$counts[1]) 'Delivery repeated native/decoder/retirement work.'
            Reject {Assert-RemovalDiagnosticRelease}
            Reject {Confirm-RemovalRunDelivery -RunDirectory $o.RunDirectory -DeliveryReceiptPath $upload `
                -ExpectedDeliveryReceiptSha256 $hostReceipt.Pin.Sha256 -ExpectedLocalAckSha256 $local.Sha256 -UserRebootConfirmed}
        }finally{Close-ClosedFinishFixture $pair}
    }
}
$checks|ConvertTo-Json -Depth 7
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' FINISH=production SEAL=production EXPORT=production HOST_VERIFY=production TARGET_FINALIZE=production TRANSPORT=local-file-copy NATIVE_OPERATIONS=0')
Write-Output ('FIXTURE_ROOT='+$script:fixtureRoot)
if($failed){exit 1}
