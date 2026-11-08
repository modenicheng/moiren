#requires -Version 5.1
# Production lifetime composition; only host metadata, input pins and API delegates are synthetic.
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){throw 'Run synthetic diagnostic lifetime tests on a non-target host.'}
. (Join-Path $PSScriptRoot 'diagnostic-controller.ps1')
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Case([string]$Name,[scriptblock]$Action){
    try{&$Action;$checks.Add(@{name=$Name;passed=$true})}
    catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message;at=$_.ScriptStackTrace})}
}
$root=Join-Path $PSScriptRoot ('logs\diagnostic-lifetime-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$script:RealLifetimeLease=${function:New-RemovalDiagnosticLease}
$script:RealLifetimeTarget=${function:New-RemovalTargetReadiness}
$script:RealLifetimeTargetParameters=(Get-Command New-RemovalTargetReadiness).Parameters
$script:RealLifetimeAudit=${function:New-RemovalSafetyAudit}
$script:RealLifetimeTrace=${function:New-RemovalTraceSession}
Initialize-RemovalTraceNative

function Event([string]$Name){$script:fixture.Events.Add($Name)}
function Assert-RemovalTaskHost {}
function Get-RemovalSessionExplorerPath {'C:\Windows\explorer.exe'}
function Get-RemovalSessionProcessIds {return @([int]41)}
function Open-RemovalSessionProcess([int]$ProcessId){return $script:fixture.Process}
function Read-RemovalWorkerToken($Process){return $Process.Token}
function Assert-RemovalRunInputs($Owner){
    Event 'inputs'
    Check ([Object]::ReferenceEquals($Owner,$script:fixture.Inputs)) 'Original synthetic inputs replaced.'
    Check ((ConvertTo-Json -InputObject $Owner -Depth 10 -Compress)-ceq$script:fixture.InputsJson) 'Original input configuration changed.'
    foreach($pin in @($Owner.ManifestPin,$Owner.Schedule,$Owner.Pins.Probe.File)){
        $actual=Copy-RemovalDecodePin $pin 268435456
        Check ($actual.Bytes-eq$pin.Bytes-and$actual.Sha256-ceq$pin.Sha256) 'Original fixture file pin changed.'
    }
}
function Assert-RemovalDiagnosticAdmission($Owner){
    # TEST ONLY: production never admits InjectedCollectorSource. Keep its true
    # provenance, and retain every real identity/lease/readiness assertion.
    Assert-RemovalControllerIdentity $Owner
    if($Owner.Errors.Count-or$Owner.TimedOut-or$Owner.Completed){throw 'Diagnostic admission is closed.'}
    Assert-RemovalDiagnosticLease $Owner.RetainedLease
    Assert-RemovalRunInputs $Owner.RetainedInputs
    Assert-RemovalTargetReadiness $Owner.RetainedBefore
    Check ($Owner.RetainedBefore.RetainedPreflight.ExecutionMode-ceq'InjectedCollectorSource') 'Synthetic collector was relabelled.'
    Event 'admission'
}
function New-RemovalDiagnosticLease {
    Event 'lease-create'
    & $script:RealLifetimeLease -Path (Join-Path $script:fixture.Root 'controller.lock')
}
function New-RemovalTargetReadiness {
    param($EvidenceDirectory,[switch]$UserRebootConfirmed,$WorkerSessionId,$ReadToolPins)
    Event 'before-create'
    $args=@{EvidenceDirectory=$EvidenceDirectory;UserRebootConfirmed=$UserRebootConfirmed;WorkerSessionId=$WorkerSessionId
        CollectorPath=$script:fixture.BeforeCollector;CollectorSha256=(Get-FileHash $script:fixture.BeforeCollector).Hash}
    if($script:RealLifetimeTargetParameters.ContainsKey('ReadToolPins')){$args.ReadToolPins=$ReadToolPins}
    & $script:RealLifetimeTarget @args
}
function New-RemovalSafetyAudit {
    param($BeforeReadiness,$EvidenceDirectory,[switch]$UserRebootConfirmed)
    Event 'after-create'
    Check ($BeforeReadiness.RetainedPreflight.RetainedOwner.Closed) 'Audit bound before actual baseline closure.'
    $audit=& $script:RealLifetimeAudit -BeforeReadiness $BeforeReadiness -EvidenceDirectory $EvidenceDirectory `
        -UserRebootConfirmed:$UserRebootConfirmed -CollectorPath $script:fixture.AfterCollector `
        -CollectorSha256 (Get-FileHash $script:fixture.AfterCollector).Hash
    Check ($BeforeReadiness.RetainedPreflight.ReadToolPinsJson-ceq'null'-and
        $audit.RetainedAfterPreflight.ReadToolPinsJson-ceq$BeforeReadiness.RetainedPreflight.ReadToolPinsJson) 'After audit lost exact original inherited read-tool pins.'
    return $audit
}
function Write-LifetimeTraceBuffer([IntPtr]$Pointer){
    $v=[Runtime.InteropServices.Marshal]::PtrToStructure($Pointer,[type][Moiren.RemovalTraceQueryV1.Properties])
    $w=$v.Wnode;$w.HistoricalContext=[uint64]701;$w.Guid=$script:fixture.Guid;$w.ClientContext=1;$v.Wnode=$w
    $v.LogFileMode=1;$v.MaximumFileSize=16
    [Runtime.InteropServices.Marshal]::StructureToPtr($v,$Pointer,$false)
    foreach($entry in @(@{Offset=$v.LoggerNameOffset;Text=$script:fixture.TraceName},@{Offset=$v.LogFileNameOffset;Text=$script:fixture.EtlPath})){
        $bytes=[Text.Encoding]::Unicode.GetBytes($entry.Text+[char]0)
        [Runtime.InteropServices.Marshal]::Copy($bytes,0,[IntPtr]::Add($Pointer,[int]$entry.Offset),$bytes.Length)
    }
}
function New-RemovalTraceSession {
    param($Session,$EtlPath)
    Event 'trace-create';$script:fixture.TraceName=$Session;$script:fixture.EtlPath=$EtlPath
    $native=@{
        Start={param([IntPtr]$Handle,[string]$Name,[IntPtr]$Properties)
            Event 'trace-start'
            Check ($script:fixture.Controller.RetainedLease.RetainedFile-is[IO.FileStream]) 'Trace did not retain a real lease.'
            Assert-RemovalDiagnosticLease $script:fixture.Controller.RetainedLease
            Check (Test-Path -LiteralPath (Join-Path $script:fixture.Controller.RetainedManaged.RunDirectory 'run-spec.json')) 'Trace started before actual managed staging.'
            $v=[Runtime.InteropServices.Marshal]::PtrToStructure($Properties,[type][Moiren.RemovalTraceQueryV1.Properties])
            $script:fixture.Guid=$v.Wnode.Guid
            [Runtime.InteropServices.Marshal]::WriteInt64($Handle,701);return [uint32]0}
        Enumerate={param([IntPtr]$Pointers,[uint32]$Capacity,[IntPtr]$Returned)
            Event 'provider-idle'
            $row=New-Object Moiren.RemovalTraceQueryV1.GuidProperties
            $row.Guid=[Guid]'7dc75ba9-8274-4eae-a42d-b4c74e0209d2';$row.IsEnable=[byte]0
            [Runtime.InteropServices.Marshal]::StructureToPtr($row,[Runtime.InteropServices.Marshal]::ReadIntPtr($Pointers),$false)
            [Runtime.InteropServices.Marshal]::WriteInt32($Returned,1);return [uint32]0}
        Enable={param($Handle,$Provider,$Code,$Level,$Any,$All,$Timeout)
            Event 'trace-enable-fail';return [uint32]5}
        Query={param([uint64]$Handle,[IntPtr]$Properties)
            Event 'trace-stop-query';Check ($Handle-eq701) 'Pre-STOP query lost original handle.'
            Write-LifetimeTraceBuffer $Properties;return [uint32]0}
        Stop={param([uint64]$Handle,[IntPtr]$Properties)
            Event 'trace-stop';Check ($Handle-eq701) 'STOP did not use the retained handle.'
            Write-LifetimeTraceBuffer $Properties;return [uint32]0}
    }
    $query=@{Query={throw 'Failed enable must not query capture admission.'};Enumerate={throw 'Failed enable must not query capture admission.'}}
    & $script:RealLifetimeTrace -Session $Session -EtlPath $EtlPath -NativeBoundary $native -QueryBoundary $query
}
function Assert-RemovalTaskNativeApproval {Event 'UNEXPECTED-scheduler';throw 'Host-only composition cannot access scheduler.'}
function Assert-RemovalTraceNativeOwnerApproval {Event 'UNEXPECTED-native-etw';throw 'Host-only composition cannot invoke ETW.'}

function New-LifetimeSnapshot {
    $now=[DateTimeOffset]::UtcNow
    @{schema_version=1;host='LAPTOP-NP49RG6H';sid='S-1-5-21-3080296863-1242338503-1346362208-1001'
        session=[Diagnostics.Process]::GetCurrentProcess().SessionId
        CollectorStartedUtc=$now.ToString('o');BootUtc=$now.AddMinutes(-10).ToString('o');ObservedUtc=$now.ToString('o')
        UptimeMilliseconds=[uint64]600000;BootAfterDeployment=$true;SignaturesVerified=$true;CatalogMembersVerified=$true
        NativeOwners=@();RunningTasks=@();PresentRawChildren=@()
        ExternalOwners=@(@{Pid=42;Name='QQMusic.exe';Session=7;CreationUnixMs=[long]1794000000000;ExecutablePath='D:\QQMusic\QQMusic.exe'})
        ExternalOwnersProvenance='ObservedComplete';ExternalOwnersObservationStartedUtc=$now.ToString('o');ExternalOwnersObservedUtc=$now.ToString('o')
        Defaults=@(foreach($flow in @('capture','render')){foreach($role in @('communications','console','multimedia')){
            @{flow=$flow;role=$role;id='endpoint-'+$flow}}})
        Render=@(@{id='render-one';volume_scalar=[double]0.5;muted=$false})
        Security=@{SecureBoot=$false;Vbs=2;CodeIntegrity=2;UserCodeIntegrity=0;Configured=@(1,2);Running=@(1,2);Bcd=('a'*64)}
        Driver=@{Version='3.1.1.613';Inf='oem207.inf';State='Running';InstanceId='ROOT\MEDIA\0001';PnpStatus='OK';ProblemCode=0
            SysPath='C:\Windows\System32\DriverStore\FileRepository\reviewed\MoirenVad.sys'
            SysSha256='0A1DFF698923B2DF39D0625979D3CD1AEE486228396C1BF601EADEA565EB3B76'
            InfSha256='24574ED3BAE942EF7ED91601513F8F0B181272BF1F15D2067F0F6EE2014F827C'}
        NativeTestStarted=$false;RebootRequested=$false;Qualified=$false}
}
function Write-LifetimeCollector([string]$Path,$Snapshot){
    $data=[Management.Automation.PSSerializer]::Serialize($Snapshot,16).Replace("'","''")
    $body='$begin=[DateTimeOffset]::UtcNow;$s=[Management.Automation.PSSerializer]::Deserialize(' + "'"+$data+"');"+@'
foreach($key in @('NativeOwners','RunningTasks','PresentRawChildren','ExternalOwners','Defaults','Render')){
    $s[$key]=[object[]]$s[$key]
}
foreach($key in @('Configured','Running')){$s.Security[$key]=[object[]]$s.Security[$key]}
$now=[DateTimeOffset]::UtcNow
$s.CollectorStartedUtc=$begin.ToString('o');$s.ObservedUtc=$now.ToString('o')
$s.ExternalOwnersObservationStartedUtc=$now.ToString('o');$s.ExternalOwnersObservedUtc=$now.ToString('o')
$s.UptimeMilliseconds=[uint64]($now-[DateTimeOffset]$s.BootUtc).TotalMilliseconds
return $s
'@
    [IO.File]::WriteAllText($Path,('function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){'+$body+'}'),[Text.UTF8Encoding]::new($false))
}
function New-LifetimeFixture([switch]$PendingChild){
    $dir=Join-Path $root ([Guid]::NewGuid().ToString('N'));$null=[IO.Directory]::CreateDirectory($dir)
    $process=[pscustomobject]@{Id=41;SessionId=7;StartTime=[DateTime]'2026-11-08T01:00:00Z';HasExited=$false;Handle=[IntPtr]123
        MainModule=[pscustomobject]@{FileName='C:\Windows\explorer.exe'}
        Token=@{Sid='S-1-5-21-3080296863-1242338503-1346362208-1001';IsAdministrator=$false;IsElevated=$false}}
    $process|Add-Member ScriptMethod Refresh {}
    $script:fixture=[pscustomobject]@{Root=$dir;Process=$process;Events=(New-Object Collections.Generic.List[string])
        BeforeCollector=(Join-Path $dir 'before-collector.ps1');AfterCollector=(Join-Path $dir 'after-collector.ps1')
        Release=(Join-Path $dir 'release-inner');Inputs=$null;InputsJson=$null;Controller=$null
        Guid=[Guid]::Empty;TraceName=$null;EtlPath=$null}
    $snapshot=New-LifetimeSnapshot
    Write-LifetimeCollector $fixture.BeforeCollector $snapshot
    if($PendingChild){
        $body=@'
function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){
    $exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $release=Join-Path ([IO.Path]::GetDirectoryName([IO.Path]::GetDirectoryName($EvidenceDirectory))) 'release-inner'
    $code='while(-not [IO.File]::Exists('+ ("'"+$release+"'") +')){Start-Sleep -Milliseconds 20};[Console]::Out.WriteLine("natural exit");[Console]::Error.WriteLine("drained stderr");exit 0'
    $child=Start-FiniteTool -Executable $exe -ExpectedSha256 (Get-FileHash $exe).Hash -OutputDirectory $EvidenceDirectory `
        -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-Command',$code) -TimeoutSeconds 0.02
    $ToolOwners.Add($child);$null=Observe-FiniteTool $child
    throw 'Original after child refused after timeout'
}
'@
        [IO.File]::WriteAllText($fixture.AfterCollector,$body,[Text.UTF8Encoding]::new($false))
    }else{Write-LifetimeCollector $fixture.AfterCollector $snapshot}
    $manifest=Join-Path $dir 'fixture-inputs.json';[IO.File]::WriteAllText($manifest,'{"fixture":true,"NativePassed":false,"Qualified":false}')
    $schedule=Join-Path $dir 'fixture-schedule.json';[IO.File]::WriteAllText($schedule,'[]')
    $exe=[Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
    $fixture.Inputs=[pscustomobject]@{SourceDirectory=$PSScriptRoot
        ManifestPin=(Get-LifetimePin $manifest);Schedule=(Get-LifetimePin $schedule)
        Pins=@{Probe=@{File=(Get-LifetimePin $exe)};Tools=$null}}
    $fixture.InputsJson=ConvertTo-Json -InputObject $fixture.Inputs -Depth 10 -Compress
    $fixture.Controller=New-RemovalDiagnosticController -RunDirectory (Join-Path $dir 'run') -Inputs $fixture.Inputs `
        -UserRebootConfirmed -WorkerSessionId 7
    return $fixture
}
function Get-LifetimePin([string]$Path){
    [pscustomobject]@{Path=$Path;Bytes=(Get-Item -LiteralPath $Path).Length;Sha256=(Get-FileHash -LiteralPath $Path).Hash}
}
function Assert-LifetimeTrace($f){
    $o=$f.Controller;$n=$o.OriginalNativeOwner
    Check ($n-is[Moiren.RemovalTraceNativeV1.Owner]-and$n.StartStatus-eq[uint32]0-and$n.EnableStatus-eq[uint32]5) 'Actual C# Start0/Enable5 owner not exercised.'
    foreach($name in @('trace-start','provider-idle','trace-enable-fail','trace-stop-query','trace-stop')){
        Check (@($f.Events|Where-Object{$_-ceq$name}).Count-eq1) ('Native boundary missing/retried: '+$name)
    }
    Check ($n.TraceOwned-and$n.TraceStopped-and$n.StopReceipt.Status-eq0-and$n.StopReceipt.StoppedConfirmed) 'Original trace STOP not confirmed.'
    Check ([Object]::ReferenceEquals($n,$n.StopReceipt.Owner)-and[Object]::ReferenceEquals($n.StopReceipt,$o.RetainedTrace.StoppedReceipt)) 'Exact original STOP receipt not retained.'
    Check ($null-eq$o.OriginalReadback.Initial-and-not$o.OriginalReadback.FinalCountersKnown-and-not$n.Qualified-and-not$n.NativeApproved) 'Failed enable gained capture/qualification.'
    Check (-not$o.RegistrationRequested-and-not$o.DispatchRequested-and-not$o.RetainedManaged.TaskOwner.RegistrationAttempted-and
        -not$o.RetainedManaged.TaskOwner.DispatchAttempted-and$null-eq$o.RetainedManaged.TaskOwner.Scheduler) 'Staged task reached scheduler.'
    Check (@($f.Events|Where-Object{$_-like'UNEXPECTED-*'}).Count-eq0) 'A native fallback was reached.'
}
function Close-LifetimeFixture($f){
    # Release only the harmless child. Never Stop/kill a process or unfinished
    # runspace. The real original owners drain before fixture lease disposal.
    [IO.File]::WriteAllText($f.Release,'natural exit')
    $o=$f.Controller
    if($null-eq$o){return}
    $clock=[Diagnostics.Stopwatch]::StartNew()
    while($clock.ElapsedMilliseconds-lt15000){
        $before=$o.RetainedBefore.RetainedPreflight.RetainedOwner
        $after=if($null-ne$o.RetainedAfter){$o.RetainedAfter.RetainedAfterPreflight.RetainedOwner}else{$null}
        if(($null-eq$before-or$before.Closed)-and($null-eq$after-or$after.Closed)){break}
        $null=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 100
    }
    # Failure cleanup still owns the original child/runspace. This fallback is
    # never success evidence: the case asserts root-driven closure beforehand.
    if($null-ne$after-and-not$after.Closed){
        for($i=0;$i-lt150-and-not$after.Closed;$i++){
            $null=Observe-RemovalSafetyAudit $o.RetainedAfter -ObservationMilliseconds 100
        }
    }
    foreach($collector in @($before,$after)){
        if($null-ne$collector){Check $collector.Closed 'Original synthetic collector remains unresolved; retained.'}
    }
    if(-not$o.RetainedLease.Closed){Release-RemovalDiagnosticLease $o.RetainedLease}
}

Case 'actual-before-stage-failed-enable-exact-stop-after-receipts-and-lease-release' {
    $f=New-LifetimeFixture;$o=$f.Controller
    try{
        Start-RemovalDiagnosticController $o
        $file=$o.RetainedLease.RetainedFile;$handle=$o.RetainedLease.RetainedHandle
        Check ($file-is[IO.FileStream]-and$handle-is[Microsoft.Win32.SafeHandles.SafeFileHandle]-and-not$handle.IsClosed) 'No actual exclusive lease.'
        $blocked=$false
        try{$other=[IO.File]::Open($o.RetainedLease.Path,[IO.FileMode]::Open,[IO.FileAccess]::Write,[IO.FileShare]::Read);$other.Dispose()}catch [IO.IOException]{$blocked=$true}
        Check $blocked 'A second writer entered the real lease.'
        for($i=0;$i-lt150;$i++){
            $r=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 50
            if(-not$r.NeedsObservation){break}
        }
        Check ($r.Completed-and-not$r.Safe-and-not$r.NativePassed-and-not$r.Qualified) ('Actual root did not persist failed completion: '+($o.Errors-join'; '))
        Assert-LifetimeTrace $f
        $before=$o.RetainedBefore.RetainedPreflight;$after=$o.RetainedAfter.RetainedAfterPreflight
        Check ($before.RetainedOwner.Completed-and$before.RetainedOwner.Closed-and$after.RetainedOwner.Completed-and$after.RetainedOwner.Closed) 'Actual before/after collectors not closed.'
        Check (-not[Object]::ReferenceEquals($before.RetainedOwner,$after.RetainedOwner)-and
            $before.ExecutionMode-ceq'InjectedCollectorSource'-and$after.ExecutionMode-ceq'InjectedCollectorSource') 'Collector provenance/ownership replaced.'
        Check ($before.ReadToolPinsJson-ceq'null'-and$after.ReadToolPinsJson-ceq$before.ReadToolPinsJson-and
            $before.SourceIdentityJson.Contains('ReadToolPinsJson')-and$after.SourceIdentityJson.Contains('ReadToolPinsJson')) 'Original inherited read-tool pin JSON missing from source identity.'
        Check ($o.RetainedAfter.Complete-and$o.RetainedAfter.Safe-and$after.ObservationAccepted-and-not$after.Ready) 'Unchanged after audit missing or granted admission.'
        Assert-RemovalSafetyReceipt $o.RetainedAfter
        foreach($pin in @($before.ReceiptPin,$o.RetainedBefore.ReceiptPin,$after.ReceiptPin,$o.RetainedAfter.ReceiptPin,$o.ResultPin)){
            $null=Copy-RemovalDecodePin $pin 2097152
        }
        $saved=[IO.File]::ReadAllText($o.ResultPin.Path)|ConvertFrom-Json
        $audit=[IO.File]::ReadAllText($o.RetainedAfter.ReceiptPin.Path)|ConvertFrom-Json
        Check ($saved.ControllerComplete-and-not$saved.Safe-and-not$saved.NativePassed-and-not$saved.Qualified-and$saved.Errors.Count-gt0) 'Root result lost refusal or promoted qualification.'
        Check ($saved.Before.Sha256-ceq$o.RetainedBefore.ReceiptPin.Sha256-and$saved.After.Sha256-ceq$o.RetainedAfter.ReceiptPin.Sha256-and
            $audit.BeforeReadinessReceipt.Sha256-ceq$o.RetainedBefore.ReceiptPin.Sha256) 'Root/after receipt chain lost originals.'
        Check ($o.RetainedLease.Closed-and-not$o.RetainedLease.Owned-and$handle.IsClosed-and-not$file.CanWrite) 'Persisted root did not release original lease.'
        $count=$f.Events.Count;$null=Observe-RemovalDiagnosticController $o
        Check ($count-eq$f.Events.Count) 'Completed root repeated work.'
    }finally{Close-LifetimeFixture $f}
}
Case 'root-timeout-and-public-trace-mutation-still-drain-original-after-child' {
    $f=New-LifetimeFixture -PendingChild;$o=$f.Controller;$child=$null;$collector=$null;$invocation=$null;$process=$null
    try{
        try{
            Start-RemovalDiagnosticController $o
            for($i=0;$i-lt150;$i++){
                $r=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 50
                if($o.AfterStartAttempted){
                    $collector=$o.RetainedAfter.RetainedAfterPreflight.RetainedOwner
                    if($collector.Completed-and$collector.ToolOwners.Count-eq1){break}
                }
            }
            Check ($null-ne$collector-and$collector.Completed-and-not$collector.Closed-and$collector.ToolOwners.Count-eq1) ('After fixture did not retain pending child: '+($o.Errors-join'; '))
            $child=$collector.ToolOwners[0];$process=$child.Process;$invocation=$collector.Invocation;$leaseFile=$o.RetainedLease.RetainedFile
            $leaseHandle=$o.RetainedLease.RetainedHandle
            Check ($r.NeedsObservation-and$child.Process-is[Diagnostics.Process]-and$child.IdentityConfirmed-and$child.TimedOut-and-not$child.Closed-and-not$child.ExitedConfirmed) 'Actual .02-second timed-out child not retained.'
            $o.DeadlineMilliseconds=[long]0
            $r=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 1
            Check ($o.TimedOut-and$r.NeedsObservation-and-not$r.Completed) 'Root timeout invented terminal ownership.'
            $o.TraceSession=[pscustomobject]@{fixture='foreign-public-trace'}
            $o.RetainedTrace.Readback=[pscustomobject]@{fixture='foreign-public-readback'}
            for($i=0;$i-lt3;$i++){
                $r=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 10
                Check ($r.NeedsObservation-and-not$r.Completed-and-not$r.NativePassed-and-not$r.Qualified-and$o.RetainedLease.Owned-and-not$leaseFile.SafeFileHandle.IsClosed) 'Outer refusal released pending lease or claimed completion.'
                Check ([Object]::ReferenceEquals($collector,$o.RetainedAfter.RetainedAfterPreflight.RetainedOwner)-and
                    [Object]::ReferenceEquals($invocation,$collector.Invocation)-and[Object]::ReferenceEquals($child,$collector.ToolOwners[0])-and
                    [Object]::ReferenceEquals($process,$child.Process)-and-not$child.Closed) 'Root refusal replaced original child/runspace.'
            }
        }finally{[IO.File]::WriteAllText($f.Release,'natural exit')}
        # Only root observation may satisfy these assertions; the outer finally
        # is failure cleanup, not evidence that the controller drained ownership.
        for($i=0;$i-lt150-and-not$collector.Closed;$i++){
            $r=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 100
        }
        Check ($null-ne$child-and$child.Closed-and$child.ExitedConfirmed-and$child.ExitCode-eq0-and$child.TimedOut-and$child.OutputStable) 'Original child did not naturally exit/drain/close.'
        Check ($child.Copies.Count-eq2-and$child.Flushes.Count-eq2-and@($child.Copies|Where-Object{-not$_.IsCompleted}).Count-eq0-and
            @($child.Flushes|Where-Object{-not$_.IsCompleted}).Count-eq0) 'Original output copies/flushes not settled.'
        Check ([IO.File]::ReadAllText($child.StdoutPath).Contains('natural exit')-and[IO.File]::ReadAllText($child.StderrPath).Contains('drained stderr')) 'Natural child output not retained.'
        Check ($collector.Closed-and$collector.EndInvoked-and[Object]::ReferenceEquals($invocation,$collector.Invocation)-and
            [Object]::ReferenceEquals($process,$child.Process)) 'Original collector closure lost retained invocation/process.'
        Check ($o.RetainedAfter.Complete-and$o.RetainedAfter.Unsafe-and$null-ne$o.RetainedAfter.ReceiptPin) 'Failed after collector lost terminal unsafe evidence.'
        Assert-RemovalSafetyReceipt $o.RetainedAfter
        Assert-LifetimeTrace $f
        Check (-not$o.Completed-and$null-eq$o.ResultPin-and-not$o.ResultAttempted-and$o.Errors.Count-gt0-and
            -not$o.NativePassed-and-not$o.Qualified) 'Changed root composition falsely completed.'
        Check ($o.RetainedLease.Owned-and-not$leaseHandle.IsClosed) 'Root released lease despite deliberately pending composition.'
    }finally{Close-LifetimeFixture $f}
    Check ($o.RetainedLease.Closed-and$leaseHandle.IsClosed) 'Fixture failed to dispose original lease after draining.'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ROOT=production LEASE=FileStream COLLECTORS=synthetic-runspace TRACE_OWNER=production-csharp API_DELEGATES=injected NATIVE_AUDIO=false ETW_CALLS=0 SCHEDULER_CALLS=0')
Write-Output ('FIXTURE_ROOT='+$root)
if($failed){exit 1}
