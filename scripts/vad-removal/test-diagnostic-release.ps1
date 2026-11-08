#requires -Version 5.1
# Real constructors, local pins/locks/result persistence. Target metadata and
# immutable binary constants are fixtures; no scheduler, ETW or audio dispatch.
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName-ceq'LAPTOP-NP49RG6H'){throw 'Run release tests only on the development host.'}
. (Join-Path $PSScriptRoot 'diagnostic-controller.ps1')
. (Join-Path $PSScriptRoot 'run-manifest.ps1')
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$failed=$false;try{&$Action|Out-Null}catch{$failed=$true};Check $failed 'Expected original-owner release refusal.'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message;at=$_.ScriptStackTrace})}}
function Import-ReleaseFixture([string]$File,[string[]]$Names){
    $tokens=$null;$errors=$null
    $ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot $File),[ref]$tokens,[ref]$errors)
    Check ($errors.Count-eq0) 'Fixture parse failed.'
    foreach($name in $Names){
        $nodes=@($ast.EndBlock.Statements|Where-Object{$_-is[Management.Automation.Language.FunctionDefinitionAst]-and$_.Name-ceq$name})
        Check ($nodes.Count-eq1) ('Unique fixture definition required: '+$name)
        $source=$nodes[0].Extent.Text-replace'\Afunction\s+([^\s(]+)','function script:$1'
        . ([scriptblock]::Create($source))
    }
}
Import-ReleaseFixture 'test-trace-decode-fixtures.ps1' @('Pin','Write-Text')
Import-ReleaseFixture 'test-run-inputs.ps1' @('Save-InputManifest','New-InputFixture')
Import-ReleaseFixture 'test-diagnostic-lifetime.ps1' @('New-LifetimeSnapshot')
$root=Join-Path $env:TEMP ('moiren-diagnostic-release-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$script:ps1Root=$PSScriptRoot
$script:RealReleaseLease=${function:New-RemovalDiagnosticLease}
function Assert-RemovalTaskHost {} # TEST ONLY host metadata; native APIs never invoked.
function Get-RemovalRunFixedPins {return $script:inputFixture.Fixed} # Tiny inert file fixtures, not release binaries.
function New-RemovalDiagnosticLease {& $script:RealReleaseLease -Path (Join-Path $script:releaseDirectory 'writer.lock')}
function Assert-RemovalTargetReadiness($Owner){
    # Keep real snapshot boot/freshness/schema validation. No target collector
    # or Explorer/token lookup runs on this host.
    Check ([Object]::ReferenceEquals($Owner,$script:releaseFixture.Controller.RetainedBefore)) 'Foreign readiness.'
    Assert-RemovalPreflightSnapshot $Owner.RetainedPreflight.RetainedOwner.Shared.Snapshot ([DateTimeOffset]::UtcNow) `
        ([Diagnostics.Process]::GetCurrentProcess().SessionId)
}
function New-Fixture([switch]$LeaveTraceRunning,[uint32]$StopStatus=0){
    $dir=Join-Path $root ([Guid]::NewGuid().ToString('N'));$null=[IO.Directory]::CreateDirectory($dir)
    $tmf=Join-Path $dir 'tmf';$null=[IO.Directory]::CreateDirectory($tmf);$pins=@{}
    foreach($entry in @(@('Tracefmt','tracefmt.exe'),@('Python','python.exe'),@('Reader','vad-removal-stages.py'),@('DriverPackage','package-manifest.json'))){
        $path=Join-Path $dir $entry[1];Write-Text $path ('inert fixture '+$entry[1]);$pins[$entry[0]]=Pin $path
    }
    $probe=Join-Path $dir 'e2e_latency_probe.exe';Write-Text $probe 'inert fixture, never executable'
    $build=Join-Path $dir 'probe-build.json';Write-Text $build '{}'
    $symbol=Join-Path $tmf 'reviewed.tmf';Write-Text $symbol 'inert reviewed TMF'
    $pins.Probe=@{File=(Pin $probe);BuildManifest=(Pin $build)};$pins.Symbols=@{Directory=$tmf;Files=@((Pin $symbol))}
    @{Root=$dir;Pins=$pins;Calls=(New-Object Collections.Generic.List[object])}
}
function New-ReleaseFixture([string]$ManifestMode,[switch]$Prepared) {
    $f=New-InputFixture
    # The actual production source set is independently frozen before creating
    # inputs/root. Never modify these shared files in a regression test.
    $f.SourceDirectory=$script:ps1Root;$f.Manifest.Sources=@()
    foreach($item in @(Get-ChildItem -LiteralPath $script:ps1Root -Filter '*.ps1' -File|Where-Object{$_.Name-notlike'test-*'})){
        $p=Pin $item.FullName;$p.Name=$item.Name;$f.Manifest.Sources+=,$p
    }
    $f.Fixed.FiniteSha256=(@($f.Manifest.Sources|Where-Object{$_.Name-ceq'finite-tool.ps1'}))[0].Sha256
    $bundle=[ordered]@{SchemaVersion=1;SourceKind='ReviewedBundle';BaseCommit=$f.Manifest.SourcesRevision
        BundleIdentity='independently-pinned-release-test';Sources=@($f.Manifest.Sources|ForEach-Object{
            [ordered]@{Name=$_.Name;Bytes=$_.Bytes;Sha256=$_.Sha256}})}
    $f.BundlePath=Join-Path $f.Root 'source-review.json';Write-Text $f.BundlePath ($bundle|ConvertTo-Json -Depth 8)
    $f.Manifest.SourceProvenance=[pscustomobject]@{SourceKind='ReviewedBundle';BaseCommit=$f.Manifest.SourcesRevision
        BundleIdentity=$bundle.BundleIdentity;SourcesRevisionMeaning='ReviewedBundleBaseCommit';BundleManifestPin=([pscustomobject](Pin $f.BundlePath))}
    if($ManifestMode-ceq'wrong-bundle-row'){
        $bundle.Sources[0].Sha256='0'*64
        Write-Text $f.BundlePath ($bundle|ConvertTo-Json -Depth 8)
        $f.Manifest.SourceProvenance.BundleManifestPin=[pscustomobject](Pin $f.BundlePath)
    }
    if($ManifestMode-ceq'missing-bundle'){$f.Manifest.Remove('SourceProvenance')}
    Save-InputManifest $f
    if($ManifestMode-ceq'duplicate-json'){
        $text=[IO.File]::ReadAllText($f.ManifestPath)
        $text=[regex]::Replace($text,'"SchemaVersion"\s*:\s*1','"SchemaVersion":1,"SchemaVersion":1')
        Write-Text $f.ManifestPath $text
        $f.ManifestHash=(Pin $f.ManifestPath).Sha256
    }
    $f.Inputs=New-RemovalRunInputs -ManifestPath $f.ManifestPath -ExpectedSha256 $f.ManifestHash -SourceDirectory $f.SourceDirectory
    $script:releaseDirectory=$f.Root
    $f.Controller=New-RemovalDiagnosticController -RunDirectory (Join-Path $f.Root 'run') -Inputs $f.Inputs -UserRebootConfirmed -WorkerSessionId 7
    $script:releaseFixture=$f
    if($Prepared){return $f}
    $o=$f.Controller;$o.StartAttempted=$true
    Acquire-RemovalDiagnosticLease $o.RetainedLease;Bind-RemovalControllerLifetime $o 'Writer'
    $null=[IO.Directory]::CreateDirectory($o.RunDirectory)
    $f.Snapshot=New-LifetimeSnapshot
    # No invocation occurred. This metadata seam is not native approval and
    # cannot establish real readiness outside this explicitly synthetic suite.
    $collector=[pscustomobject]@{Shared=@{Snapshot=$f.Snapshot};KnownNoDispatch=$true;Completed=$true;Closed=$true
        EndInvoked=$false;LaunchAttempted=$false;Invocation=$null;ToolOwners=@()}
    $o.RetainedBefore.RetainedPreflight.RetainedOwner=$collector
    $o.RetainedBefore.RetainedPreflight.CollectorOwner=$collector
    Bind-RemovalControllerLifetime $o 'Before'
    $receipt=Join-Path $f.Root 'synthetic-before.json';Write-Text $receipt '{"fixture":true}'
    $o.RetainedBefore.ReceiptPin=[pscustomobject](Pin $receipt)
    $o.RetainedBefore.RetainedPreflight.ReceiptPin=[pscustomobject](Pin $receipt)
    return $f
}
function Close-ReleaseFixture($f){
    $o=$f.Controller;$c=Get-RemovalControllerLifetime $o
    # Fixture finalizer never disposes an owned native process. All handles here
    # belong to test-only synchronous local reads or an unused writer.
    $handles=Get-RemovalSealProperty $c 'ReleaseHandles'
    if($null-ne$handles){foreach($h in $handles){$h.Dispose()}}
    if($o.RetainedLease.Owned){Release-RemovalDiagnosticLease $o.RetainedLease}
}
function Set-ReleaseFixtureExpiredOrigin($Owner){
    # TEST ONLY deterministic expiry of the actual private constructor lifetime.
    # The old implementation has no origin and ignores this added fixture field.
    $c=Get-RemovalControllerLifetime $Owner
    $origin=[Diagnostics.Stopwatch]::GetTimestamp()-([long]241*[Diagnostics.Stopwatch]::Frequency)
    if($null-eq$c.PSObject.Properties['TimestampOrigin']){$c|Add-Member NoteProperty TimestampOrigin $origin}
    else{$c.TimestampOrigin=$origin}
}
function Set-ReleaseFixtureCollectorRepin($f){
    $p=$f.Controller.RetainedBefore.RetainedPreflight
    $foreign=Join-Path $f.Root 'foreign-collector.ps1';Write-Text $foreign '# inert fixture; never executed'
    $p.CollectorPath=$foreign;$p.CollectorSha256=(Pin $foreign).Sha256
    $p.SourceIdentityJson=ConvertTo-Json -InputObject ([ordered]@{FiniteToolPath=$p.FiniteToolPath;FiniteToolSha256=$p.FiniteToolSha256
        CollectorPath=$p.CollectorPath;CollectorSha256=$p.CollectorSha256;ExecutionMode=$p.ExecutionMode
        ObservationMode=$p.ObservationMode;ReadToolPinsJson=$p.ReadToolPinsJson}) -Compress
    Check ($p.ExecutionMode-ceq'RepositoryCollector') 'Fixture did not retain the misleading repository label.'
    Assert-RemovalPreflightSessionIdentity $p
}
Case 'collector-repin-on-original-preflight-refuses-before-readiness-dispatch' {
    $f=New-ReleaseFixture -Prepared
    $start=${function:Start-RemovalTargetReadiness};$script:ReleaseReadinessDispatches=0
    try{
        Set-ReleaseFixtureCollectorRepin $f
        function Start-RemovalTargetReadiness($Owner){
            $script:ReleaseReadinessDispatches++
            throw 'TEST ONLY intercepted readiness dispatch; no collector executed.'
        }
        $failure=$null;try{Start-RemovalDiagnosticController $f.Controller}catch{$failure=$_.Exception.Message}
        Check ($script:ReleaseReadinessDispatches-eq0) 'Repinned original collector reached readiness dispatch.'
        Check ($failure-match'collector.*(identity|source|pin|changed)') ('Unexpected predispatch failure: '+$failure)
        Check (-not$f.Controller.OriginalNativeOwner.StartAttempted) 'Collector refusal dispatched native work.'
    }finally{Set-Item Function:Start-RemovalTargetReadiness $start;Close-ReleaseFixture $f}
}
Case 'collector-repin-on-original-preflight-cannot-pass-admission' {
    $f=New-ReleaseFixture
    try{
        Set-ReleaseFixtureCollectorRepin $f
        $failure=$null;try{Assert-RemovalDiagnosticAdmission $f.Controller}catch{$failure=$_.Exception.Message}
        Check ($null-ne$failure) 'Repinned original collector passed production admission.'
        Check ($failure-match'collector.*(identity|source|pin|changed)') ('Unexpected collector admission failure: '+$failure)
    }finally{Close-ReleaseFixture $f}
}
foreach($mode in @('Stop','Restart')){
    Case ('public-clock-'+$mode+'-cannot-renew-private-release-deadline') {
        $f=New-ReleaseFixture
        try{
            Set-ReleaseFixtureExpiredOrigin $f.Controller
            if($mode-ceq'Stop'){$f.Controller.Clock.Stop()}else{$f.Controller.Clock.Restart()}
            Check ($f.Controller.Clock.ElapsedMilliseconds-lt240000) 'Public clock fixture was not renewed.'
            $failure=$null;try{Assert-RemovalDiagnosticRelease $f.Controller}catch{$failure=$_.Exception.Message}
            Check ($null-ne$failure) ('Original expired lifetime released after public Clock.'+$mode)
            Check ($failure-match'expired|finite|deadline') ('Unexpected expiry failure: '+$failure)
            Check (-not$f.Controller.OriginalNativeOwner.StartAttempted) 'Expired release dispatched native work.'
        }finally{Close-ReleaseFixture $f}
    }
}
Case 'private-origin-expiry-latches-controller-timeout-despite-public-restart' {
    $f=New-ReleaseFixture
    try{
        Set-ReleaseFixtureExpiredOrigin $f.Controller;$f.Controller.Clock.Restart()
        $f.Controller.BeforeSettled=$true
        $f.Controller.Errors.Add('TEST ONLY prior birth refusal; isolate original timeout observation.')
        $r=Observe-RemovalDiagnosticController $f.Controller -ObservationMilliseconds 1
        Check ($f.Controller.TimedOut-and$r.TimedOut) 'Public Clock.Restart suppressed original controller timeout.'
        Check (-not$f.Controller.OriginalNativeOwner.StartAttempted) 'Timeout observation dispatched native work.'
    }finally{Close-ReleaseFixture $f}
}
Case 'bare-clone-and-both-reference-replacement-have-no-constructor-authority' {
    Reject {Assert-RemovalDiagnosticRelease}
    $f=New-ReleaseFixture
    try{
        Reject {Assert-RemovalDiagnosticRelease $f.Controller.PSObject.Copy()}
        $original=$f.Controller.RetainedInputs
        $f.Controller.Inputs=$original.PSObject.Copy();$f.Controller.RetainedInputs=$f.Controller.Inputs
        Reject {Assert-RemovalDiagnosticAdmission $f.Controller}
        Reject {Assert-RemovalControllerSealedIdentity $f.Controller}
        $f.Controller.Inputs=$original;$f.Controller.RetainedInputs=$original
    }finally{Close-ReleaseFixture $f}
}
Case 'original-constructor-releases-once-and-retains-independent-full-source-read-locks' {
    $f=New-ReleaseFixture
    try{
        Assert-RemovalDiagnosticRelease $f.Controller
        $c=Get-RemovalControllerLifetime $f.Controller
        Check ($c.ReleaseHandles.Count-eq($f.Manifest.Sources.Count+2)) 'Manifest, bundle or complete source handles missing.'
        foreach($path in @($f.ManifestPath,$f.BundlePath,(Join-Path $script:ps1Root 'native-admission.ps1'))){
            Reject {$writer=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::ReadWrite);$writer.Dispose()}
        }
        foreach($path in @($f.ManifestPath,$f.BundlePath)){
            Reject {[IO.File]::Move($path,$path+'.not-moved')}
        }
        Reject {Assert-RemovalDiagnosticRelease $f.Controller}
        Check (-not$f.Controller.OriginalNativeOwner.StartAttempted-and-not$f.Controller.RetainedTrace.StartAttempted) 'Release validation dispatched native work.'
    }finally{Close-ReleaseFixture $f}
}
Case 'constructor-input-json-prevents-inplace-repin-even-with-fresh-public-config' {
    $f=New-ReleaseFixture
    try{
        $f.Inputs.Schedule.Sha256='0'*64
        $f.Inputs.PinsJson=ConvertTo-Json -InputObject $f.Inputs.Pins -Depth 14 -Compress
        $f.Inputs.ConfigJson=Get-RemovalRunConfigJson $f.Inputs
        Reject {Assert-RemovalDiagnosticRelease $f.Controller}
    }finally{Close-ReleaseFixture $f}
}
Case 'independent-manifest-bundle-and-source-content-are-never-current-hash-adopted' {
    foreach($mode in @('manifest','bundle','source-row','missing-source','duplicate-json')){
        $f=New-ReleaseFixture
        try{
            switch($mode){
                'manifest'{[IO.File]::AppendAllText($f.ManifestPath,' ')}
                'bundle'{[IO.File]::AppendAllText($f.BundlePath,' ')}
                'source-row'{$f.Manifest.SourceProvenance.BundleManifestPin.Sha256='0'*64;Save-InputManifest $f}
                'missing-source'{$f.Manifest.Sources=@($f.Manifest.Sources|Where-Object{$_.Name-cne'worker.ps1'});Save-InputManifest $f}
                'duplicate-json'{Write-Text $f.ManifestPath '{"SchemaVersion":1,"SchemaVersion":1}'}
            }
            Reject {Assert-RemovalDiagnosticRelease $f.Controller}
        }finally{Close-ReleaseFixture $f}
    }
}
Case 'injected-query-collector-stale-preinstall-deadline-and-closed-writer-refuse' {
    foreach($mode in @('query','collector','stale','preinstall','deadline','closed-writer')){
        $f=New-ReleaseFixture
        try{
            switch($mode){
                'query'{$f.Controller.RetainedTrace.QueryBoundary=@{Query={throw 'never invoked'}}}
                'collector'{$f.Controller.RetainedBefore.RetainedPreflight.ExecutionMode='InjectedCollectorSource'}
                'stale'{$f.Snapshot.ObservedUtc=[DateTimeOffset]::UtcNow.AddMinutes(-3).ToString('o')}
                'preinstall'{$f.Snapshot.BootUtc='2026-11-06T00:00:00Z'}
                'deadline'{$f.Controller.DeadlineMilliseconds=0}
                'closed-writer'{Release-RemovalDiagnosticLease $f.Controller.RetainedLease}
            }
            Reject {Assert-RemovalDiagnosticRelease $f.Controller}
        }finally{Close-ReleaseFixture $f}
    }
}
Case 'strict-original-json-and-independent-bundle-rows-refuse-despite-valid-manifest-pin' {
    foreach($mode in @('duplicate-json','missing-bundle','wrong-bundle-row')){
        $f=New-ReleaseFixture -ManifestMode $mode
        try{
            Check ((Pin $f.ManifestPath).Sha256-ceq$f.Inputs.ExpectedSha256) 'Negative fixture lacks independent original hash.'
            Reject {Assert-RemovalDiagnosticRelease $f.Controller}
            Check (-not$f.Controller.OriginalNativeOwner.StartAttempted) 'Malformed original authority dispatched native work.'
        }finally{Close-ReleaseFixture $f}
    }
}
Case 'result-publication-failure-retains-every-original-lock-and-forbids-retry' {
    $f=New-ReleaseFixture
    try{
        Assert-RemovalDiagnosticRelease $f.Controller
        $c=Get-RemovalControllerLifetime $f.Controller
        $result=Join-Path $f.Controller.RunDirectory 'controller-result.json'
        Write-Text $result '{"existing":"do-not-overwrite"}'
        $f.Controller.BeforeSettled=$true
        Reject {Save-RemovalControllerResult $f.Controller}
        Check ($f.Controller.ResultAttempted-and-not$f.Controller.Completed-and$f.Controller.RetainedLease.Owned-and
            -not$c.ReleaseHandlesClosed) 'Persistence failure abandoned ownership or invented terminal result.'
        Check ([IO.File]::ReadAllText($result)-ceq'{"existing":"do-not-overwrite"}') 'Original result path overwritten.'
        Reject {Close-RemovalDiagnosticReleaseHandles $f.Controller}
        Reject {Save-RemovalControllerResult $f.Controller}
        foreach($h in $c.ReleaseHandles){Check $h.CanRead 'Persistence failure closed an original source handle.'}
        Reject {$w=[IO.File]::Open($f.ManifestPath,[IO.FileMode]::Open,[IO.FileAccess]::Write,[IO.FileShare]::ReadWrite);$w.Dispose()}
    }finally{Close-ReleaseFixture $f}
}
Case 'actual-initialize-release-and-staged-managed-bind-existing-original-context-without-dispatch' {
    $f=New-ReleaseFixture
    try{
        $o=$f.Controller;$path=Join-Path $o.RunDirectory 'managed'
        $o.RetainedBefore.RetainedUserSession.Snapshot=[pscustomobject]@{SessionId=7;Sid='S-1-5-21-3080296863-1242338503-1346362208-1001'}
        $arguments=@(Get-RemovalDiagnosticArguments $f.Inputs -ArtifactDirectory (Join-Path $path 'artifacts'))
        $managed=New-RemovalManagedSession -RunDirectory $path -SourceDirectory $f.SourceDirectory `
            -Executable $f.Inputs.Pins.Probe.File.Path -ExpectedSha256 $f.Inputs.Pins.Probe.File.Sha256 `
            -ProbeArguments $arguments -SessionId 7 -Sid $o.RetainedBefore.RetainedUserSession.Snapshot.Sid -Nonce $o.Nonce -TraceSession $o.RetainedTrace
        $o.ManagedSession=$managed;$o.RetainedManaged=$managed
        Bind-RemovalControllerLifetime $o 'Managed' $managed
        Initialize-RemovalNativeContext $o
        $context=Get-RemovalOperationContext $o.OriginalNativeOwner
        Check ([Object]::ReferenceEquals($context.Root,$o)-and[Object]::ReferenceEquals($context.Managed,$managed)-and
            [Object]::ReferenceEquals($context,(Get-RemovalOperationContext $managed.TaskOwner))-and
            [Object]::ReferenceEquals($context,(Get-RemovalOperationContext $managed))) 'Original resource CWT association missing.'
        Check (-not$o.OriginalNativeOwner.StartAttempted-and-not$o.RetainedTrace.StartAttempted-and
            -not$managed.TaskOwner.RegistrationAttempted-and-not$managed.TaskOwner.DispatchAttempted-and
            $null-eq$managed.TaskOwner.Scheduler-and$null-eq$managed.ProbeOwner) 'Constructor composition dispatched native work.'
        Reject {Initialize-RemovalNativeContext $o}
        Set-ReleaseFixtureExpiredOrigin $o;$o.Clock.Restart()
        $o.RetainedBefore.RetainedPreflight.ExecutionMode='InjectedCollectorSource'
        $o.RetainedInputs.Schedule.Sha256='0'*64
        Assert-RemovalOperationLifetime $context
        Reject {Assert-RemovalOperationBirth $context}
    }finally{Close-ReleaseFixture $f}
}
Case 'failed-source-close-cannot-export-by-forging-public-completion' {
    $f=New-ReleaseFixture;$close=${function:Close-RemovalDiagnosticReleaseHandles}
    try{
        Assert-RemovalDiagnosticRelease $f.Controller
        function Close-RemovalDiagnosticReleaseHandles {throw 'TEST ONLY original source disposal failure'}
        $f.Controller.BeforeSettled=$true
        Reject {Save-RemovalControllerResult $f.Controller}
        $c=Get-RemovalControllerLifetime $f.Controller
        Check ($null-ne$c.SealJson-and-not$c.ReleaseHandlesClosed-and$f.Controller.RetainedLease.Closed) 'Original interrupted close seam not exercised.'
        $f.Controller.Completed=$true;$f.Controller.Phase='Complete'
        Reject {Get-RemovalClosedControllerSeal $f.Controller}
        $r=Observe-RemovalDiagnosticController $f.Controller -ObservationMilliseconds 1
        Check ($r.NeedsObservation-and-not$r.Completed) 'Public terminal flags abandoned open original source handles.'
    }finally{Set-Item Function:Close-RemovalDiagnosticReleaseHandles $close;Close-ReleaseFixture $f}
}
Case 'original-read-locks-stay-pending-until-persisted-result-and-original-writer-close' {
    $f=New-ReleaseFixture
    try{
        Assert-RemovalDiagnosticRelease $f.Controller
        $c=Get-RemovalControllerLifetime $f.Controller
        $safeHandles=@($c.ReleaseHandles|ForEach-Object{$_.SafeFileHandle})
        Reject {Close-RemovalDiagnosticReleaseHandles $f.Controller}
        $f.Controller.BeforeSettled=$true
        Save-RemovalControllerResult $f.Controller
        Check ($f.Controller.Completed-and$f.Controller.RetainedLease.Closed-and$c.ReleaseHandlesClosed) 'Settled persisted root did not close original release handles.'
        foreach($h in $safeHandles){Check $h.IsClosed 'Original source read lock remained open after persisted terminal.'}
        $writer=[IO.File]::Open($f.ManifestPath,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::Read);$writer.Dispose()
    }finally{Close-ReleaseFixture $f}
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' NATIVE_CALLS=0 TARGET_CALLS=0')
if($failed){exit 1}
