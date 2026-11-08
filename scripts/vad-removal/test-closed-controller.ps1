#requires -Version 5.1
# Real controller and writer; target metadata, collectors and trace APIs are synthetic.
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){throw 'Run closed-controller tests only on the development host.'}
. (Join-Path $PSScriptRoot 'diagnostic-controller.ps1')
. (Join-Path $PSScriptRoot 'closed-export.ps1')
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$refused=$false;try{&$Action|Out-Null}catch{$refused=$true};Check $refused 'Expected closed-export refusal.'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message;at=$_.ScriptStackTrace})}}
Case 'production-closed-controller-seal-api-is-present' {
    Check ($null-ne(Get-Command Get-RemovalClosedControllerSeal -ErrorAction SilentlyContinue)) 'Retained terminal controller seal is missing.'
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_OPERATIONS=0';exit 1}
$script:RealLifetimeLease=${function:New-RemovalDiagnosticLease}
$script:RealLifetimeTarget=${function:New-RemovalTargetReadiness}
$script:RealLifetimeTargetParameters=(Get-Command New-RemovalTargetReadiness).Parameters
$script:RealLifetimeAudit=${function:New-RemovalSafetyAudit}
$script:RealLifetimeTrace=${function:New-RemovalTraceSession}
$template=Join-Path $PSScriptRoot 'test-diagnostic-lifetime.ps1'
$tokens=$null;$parseErrors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($template,[ref]$tokens,[ref]$parseErrors)
if($parseErrors.Count){throw 'Shared lifetime fixture did not parse.'}
$helpers=@('Event','Assert-RemovalTaskHost','Get-RemovalSessionExplorerPath','Get-RemovalSessionProcessIds',
    'Open-RemovalSessionProcess','Read-RemovalWorkerToken','Assert-RemovalRunInputs','Assert-RemovalDiagnosticAdmission',
    'New-RemovalDiagnosticLease','New-RemovalTargetReadiness','New-RemovalSafetyAudit','Write-LifetimeTraceBuffer',
    'New-RemovalTraceSession','Assert-RemovalTaskNativeApproval','Assert-RemovalTraceNativeOwnerApproval',
    'New-LifetimeSnapshot','Write-LifetimeCollector','New-LifetimeFixture','Get-LifetimePin','Assert-LifetimeTrace','Close-LifetimeFixture')
$definitions=@($ast.EndBlock.Statements|Where-Object{$_-is[Management.Automation.Language.FunctionDefinitionAst]})
foreach($name in $helpers){
    $found=@($definitions|Where-Object{$_.Name-ceq$name});if($found.Count-ne1){throw ('Unique fixture helper required: '+$name)}
    . ([scriptblock]::Create($found[0].Extent.Text.Replace('$PSScriptRoot',("'"+$PSScriptRoot.Replace("'","''")+"'"))))
}
$root=Join-Path $PSScriptRoot ('logs\closed-controller-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
Initialize-RemovalTraceNative
function Settle-ClosedFixture($f){
    Start-RemovalDiagnosticController $f.Controller
    for($i=0;$i-lt150;$i++){$r=Observe-RemovalDiagnosticController $f.Controller -ObservationMilliseconds 50;if(-not$r.NeedsObservation){return $r}}
    throw ('Original closed-controller fixture did not settle: '+($f.Controller.Errors -join '; '))
}
Case 'prepared-or-forged-public-completion-cannot-create-a-terminal-seal' {
    $f=New-LifetimeFixture;$o=$f.Controller
    try{
        Reject {Get-RemovalClosedControllerSeal $o}
        $o.Completed=$true;$o.ResultPin=Get-LifetimePin $f.Inputs.ManifestPin.Path
        Reject {Get-RemovalClosedControllerSeal $o}
        $clone=$o.PSObject.Copy();Reject {Get-RemovalClosedControllerSeal $clone}
    }finally{Close-LifetimeFixture $f}
}
Case 'failed-enable-real-after-result-and-closed-writer-produce-an-unqualified-seal' {
    $f=New-LifetimeFixture;$o=$f.Controller
    try{
        $r=Settle-ClosedFixture $f
        Check ($r.Completed-and-not$r.Safe) ('Original refusal did not complete: '+($o.Errors-join'; '))
        Assert-LifetimeTrace $f
        $s=Get-RemovalClosedControllerSeal $o
        Check ($s.Nonce-ceq$o.Nonce-and$s.Directory-ceq$o.OriginalDirectory-and
            $s.ResultPin.Sha256-ceq$o.ResultPin.Sha256-and$s.CollectorProvenance-ceq'InjectedCollectorSource') 'Sealed identity/result/provenance changed.'
        $saved=$s.ResultJson|ConvertFrom-Json
        Check ($saved.ControllerComplete-and-not$saved.Safe-and-not$saved.NativePassed-and-not$saved.Qualified) 'Seal promoted safety or qualification.'
        Check ($s.TerminalPins.Count-ge5-and$s.StagedScriptPins.Count-eq5-and$s.InputJson) 'Original evidence or staged source pins missing.'
        $s.ResultPin.Sha256='0'*64
        Check ((Get-RemovalClosedControllerSeal $o).ResultPin.Sha256-ceq$o.ResultPin.Sha256) 'Returned seal aliases retained private authority.'
        [IO.File]::AppendAllText($f.Inputs.Schedule.Path,' changed after closure')
        $null=Get-RemovalClosedControllerSeal $o
        $export=Export-RemovalClosedRun -Controller $o -ArchivePath ($o.OriginalDirectory+'.zip')
        $ack=Confirm-RemovalClosedRunExport -Export $export -AcknowledgementPath ($o.OriginalDirectory+'.archive-local.json')
        $ackJson=[IO.File]::ReadAllText($ack.Path)|ConvertFrom-Json
        Check ($export.LocalVerified-and$export.Nonce-ceq$o.Nonce-and$export.ResultPin.Sha256-ceq$o.ResultPin.Sha256-and
            $ackJson.ArchivePin.Sha256-ceq$export.ArchivePin.Sha256-and-not$ackJson.Transferred-and-not$ackJson.Qualified-and
            $ackJson.Meaning-ceq'LOCAL_ARCHIVE_VERIFICATION_ONLY') 'Real root archive/local acknowledgment lost original refusal or identity.'
        $stream=[IO.File]::OpenRead($export.ArchivePin.Path)
        $zip=[IO.Compression.ZipArchive]::new($stream,[IO.Compression.ZipArchiveMode]::Read,$false)
        try{
            $entry=$zip.GetEntry('export/provenance.json');Check ($null-ne$entry) 'Real closed archive provenance omitted.'
            $reader=[IO.StreamReader]::new($entry.Open())
            try{$provenance=$reader.ReadToEnd()|ConvertFrom-Json}finally{$reader.Dispose()}
            Check ($provenance.InputJson-ceq$s.InputJson-and$provenance.ResultJson-ceq$s.ResultJson-and
                $provenance.CollectorProvenance-ceq'InjectedCollectorSource') 'Real root archive repinned old inputs or relabelled collector.'
        }finally{$zip.Dispose();$stream.Dispose()}
        $pin=$o.ResultPin;$o.ResultPin=[pscustomobject]@{Path=$pin.Path;Bytes=$pin.Bytes;Sha256=('0'*64)}
        Reject {Get-RemovalClosedControllerSeal $o};$o.ResultPin=$pin
        $before=$o.RetainedBefore;$o.BeforeReadiness=[pscustomobject]@{fixture='foreign'}
        Reject {Get-RemovalClosedControllerSeal $o};$o.BeforeReadiness=$before
        [IO.File]::AppendAllText($pin.Path,' ')
        Reject {Get-RemovalClosedControllerSeal $o}
    }finally{Close-LifetimeFixture $f}
}
Case 'pending-original-after-child-cannot-export-despite-mutated-completion-flags' {
    $f=New-LifetimeFixture -PendingChild;$o=$f.Controller
    try{
        Start-RemovalDiagnosticController $o
        for($i=0;$i-lt150;$i++){
            $r=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 50
            if($o.AfterStartAttempted-and$null-ne$o.RetainedAfter.RetainedAfterPreflight.RetainedOwner-and
                $o.RetainedAfter.RetainedAfterPreflight.RetainedOwner.Completed){break}
        }
        Check ($r.NeedsObservation-and$o.RetainedLease.Owned) 'Fixture did not retain pending original child.'
        $o.Completed=$true;$o.ResultPin=Get-LifetimePin $f.Inputs.ManifestPin.Path
        Reject {Get-RemovalClosedControllerSeal $o}
        Reject {Export-RemovalClosedRun $o ($o.OriginalDirectory+'.zip')}
        Check (-not(Test-Path -LiteralPath ($o.OriginalDirectory+'.zip'))) 'Pending original child created a ZIP.'
        foreach($mode in @('complete','refuse','both')){
            $o.Completed=$mode-cne'refuse';$o.Refused=$mode-cne'complete'
            $forged=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 1
            Check ($forged.NeedsObservation-and-not$forged.Completed-and-not$forged.Refused) 'Forged public terminal flags abandoned an otherwise unchanged live controller.'
        }
        $o.Refused=$false
        $o.Completed=$false;$o.ResultPin=$null
    }finally{Close-LifetimeFixture $f}
}
Case 'capped-after-tool-exits-and-drains-into-an-exportable-unsafe-result' {
    $f=New-LifetimeFixture;$o=$f.Controller
    try{
        $body=@'
function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){
    $exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $child=Start-FiniteTool -Executable $exe -ExpectedSha256 (Get-FileHash $exe).Hash -OutputDirectory $EvidenceDirectory `
        -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-Command','[Console]::Out.Write(("x"*8192));exit 0') -MaxLogBytes 1024
    $ToolOwners.Add($child);$null=Observe-FiniteTool $child
    throw 'Expected capped AFTER tool refuses safety, not physical settlement'
}
'@
        [IO.File]::WriteAllText($f.AfterCollector,$body,[Text.UTF8Encoding]::new($false))
        $r=Settle-ClosedFixture $f
        $child=$o.RetainedAfter.RetainedAfterPreflight.RetainedOwner.ToolOwners[0]
        Check ($child.Closed-and$child.ExitedConfirmed-and$child.Budget.Exceeded-and-not$child.OutputStable) 'Real capped-output refusal not exercised.'
        Check ($r.Completed-and-not$r.Safe-and$o.RetainedLease.Closed) 'Drained capped AFTER tool stranded original writer or became safe.'
        $s=Get-RemovalClosedControllerSeal $o
        Check ($s.ResultPin.Sha256-ceq$o.ResultPin.Sha256-and-not($s.ResultJson|ConvertFrom-Json).Safe) 'Capped failure result not exportable or qualified.'
    }finally{Close-LifetimeFixture $f}
}
Case 'known-before-worker-predispatch-refusal-closes-without-inventing-an-invocation' {
    $f=New-LifetimeFixture;$o=$f.Controller
    try{
        $null=[IO.Directory]::CreateDirectory($o.RetainedBefore.RetainedPreflight.EvidenceDirectory)
        Reject {Start-RemovalDiagnosticController $o}
        for($i=0;$i-lt10;$i++){$r=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 1;if(-not$r.NeedsObservation){break}}
        Check ($r.Completed-and-not$r.Safe-and$o.RetainedLease.Closed) 'Known worker validation refusal stranded writer/completion.'
        Check ($null-eq$o.RetainedBefore.RetainedPreflight.RetainedOwner-and-not$o.OriginalNativeOwner.StartAttempted) 'Predispatch refusal invented native or collector ownership.'
        $s=Get-RemovalClosedControllerSeal $o
        Check ($s.ResultPin.Sha256-ceq$o.ResultPin.Sha256) 'Known no-dispatch result not sealed.'
    }finally{Close-LifetimeFixture $f}
}
Case 'known-created-worker-setup-failure-closes-without-fabricated-EndInvoke' {
    $f=New-LifetimeFixture;$o=$f.Controller;$drain=${function:New-CollectorDiagnosticDrain}
    try{
        function New-CollectorDiagnosticDrain($PowerShell){throw 'Expected pre-BeginInvoke setup refusal'}
        Start-RemovalDiagnosticController $o
        for($i=0;$i-lt10;$i++){$r=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 1;if(-not$r.NeedsObservation){break}}
        $collector=$o.RetainedBefore.RetainedPreflight.RetainedOwner
        Check ($null-ne$collector-and$collector.Closed-and$collector.Completed-and-not$collector.LaunchAttempted-and
            $null-eq$collector.Invocation-and-not$collector.EndInvoked) 'Known synchronous setup refusal invented an invocation or stranded the PowerShell owner.'
        Check ($r.Completed-and-not$r.Safe-and$o.RetainedLease.Closed) 'Known no-call setup failure stranded the result writer.'
        $null=Get-RemovalClosedControllerSeal $o
    }finally{Set-Item Function:New-CollectorDiagnosticDrain $drain;Close-LifetimeFixture $f}
}
Case 'first-observation-cannot-adopt-an-injected-both-reference-managed-owner' {
    $f=New-LifetimeFixture;$o=$f.Controller
    try{
        $m=[pscustomobject]@{RunDirectory=(Join-Path $o.RunDirectory 'managed');TaskOwner=[pscustomobject]@{};ProbeOwner=$null}
        $null=[IO.Directory]::CreateDirectory($m.RunDirectory)
        foreach($name in @('finite-tool.ps1','task-boundary.ps1','probe-boundary.ps1','worker-boundary.ps1','worker.ps1')){
            [IO.File]::WriteAllText((Join-Path $m.RunDirectory $name),'# foreign uncreated owner')
        }
        $o.ManagedSession=$m;$o.RetainedManaged=$m
        Bind-RemovalControllerLifetime $o
        $c=Get-RemovalControllerLifetime $o
        Check ($null-eq$c.Managed-and$c.Error) 'Generic observation adopted an injected first managed owner.'
        $o.ManagedSession=$null;$o.RetainedManaged=$null
    }finally{Close-LifetimeFixture $f}
}
Case 'changed-managed-aliases-drain-original-processes-without-new-authority' {
    foreach($mode in @('public','both')){
        $f=New-LifetimeFixture -PendingChild;$o=$f.Controller;$tools=@();$managed=$null
        $realObserve=${function:Observe-RemovalManagedSession};$realFinish=${function:New-RemovalManagedFinish}
        $realProbeObserve=${function:Observe-RemovalProbe};$script:drainEvents=New-Object Collections.Generic.List[string]
        try{
            Start-RemovalDiagnosticController $o
            for($i=0;$i-lt150-and$null-eq$o.RetainedManaged;$i++){
                $null=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 50
            }
            $managed=$o.RetainedManaged
            Check ($null-ne$managed-and-not$o.Completed) 'Original staged managed fixture was not retained.'
            $exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
            $code='while(-not [IO.File]::Exists('+ ("'"+$f.Release+"'") +')){Start-Sleep -Milliseconds 20};exit 0'
            foreach($name in @('worker-drain','probe-drain')){
                $tools+=Start-FiniteTool -Executable $exe -ExpectedSha256 (Get-FileHash $exe).Hash `
                    -OutputDirectory (Join-Path $f.Root $name) -TimeoutSeconds 15 `
                    -ArgumentList @('-NoProfile','-NonInteractive','-Command',$code)
            }
            $managed.TaskOwner.DispatchAttempted=$true;$managed.TaskOwner.WorkerProcess=$tools[0].Process
            $probe=[pscustomobject]@{Process=$tools[1].Process;Clock=[Diagnostics.Stopwatch]::StartNew()
                DeadlineMilliseconds=[long]30000;Closed=$false;ExitKnown=$false;ExitCode=$null;TimedOut=$false
                AdmissionAttempted=$false;BindingKnown=$false;BindingError=$null}
            $managed.ProbeOwner=$probe;Bind-RemovalControllerLifetime $o 'Probe' $probe
            function Observe-RemovalManagedSession($Session){$script:drainEvents.Add('managed');throw 'No receipt binding/admission in drain-only mode.'}
            function New-RemovalManagedFinish($Session){$script:drainEvents.Add('finish');throw 'No finish birth in drain-only mode.'}
            function Observe-RemovalProbe($Owner,[int]$ObservationMilliseconds=0){
                $script:drainEvents.Add('probe');&$realProbeObserve $Owner -ObservationMilliseconds $ObservationMilliseconds
            }
            $o.ManagedSession=[pscustomobject]@{Foreign=$true}
            if($mode-ceq'both'){$o.RetainedManaged=$o.ManagedSession;$o.BeforeSettled=$false}
            $o.Completed=$true;$o.Refused=$true
            $r=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 1
            Check ($r.NeedsObservation-and-not$r.Completed-and$o.RetainedLease.Owned) 'Alias refusal released writer or claimed completion.'
            [IO.File]::WriteAllText($f.Release,'natural exit')
            Check ($tools[0].Process.WaitForExit(10000)-and$tools[1].Process.WaitForExit(10000)) 'Harmless processes did not naturally exit.'
            $r=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 1
            Check ($probe.ExitKnown-and$probe.ExitCode-eq0-and$script:drainEvents.Contains('probe')) 'Alias refusal stranded original retained probe observation.'
            $lifetime=Get-RemovalControllerLifetime $o
            Check ($lifetime.WorkerDrain.ExitKnown-and$lifetime.WorkerDrain.ExitCode-eq0-and
                $lifetime.ProbeDrain.ExitKnown-and$lifetime.ProbeDrain.ExitCode-eq0) 'Original retained process exits were not independently observed.'
            Check ($managed.Error-and-not$script:drainEvents.Contains('managed')-and-not$script:drainEvents.Contains('finish')-and
                $null-eq$o.RetainedFinish-and-not$probe.AdmissionAttempted-and-not$managed.TaskOwner.RetirementAttempted) 'Drain gained binding, admission, finish or retirement authority.'
            Check ($r.NeedsObservation-and-not$r.Completed-and$o.RetainedLease.Owned) 'Drain-only exits became terminal controller authority.'
            Reject {Get-RemovalClosedControllerSeal $o}
            $o.ManagedSession=$managed;$o.RetainedManaged=$managed
            $null=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 1
            Check ($lifetime.DrainOnly-and$null-eq$o.RetainedFinish-and-not$probe.AdmissionAttempted) 'Restoring aliases reopened admission or Finish birth.'
        }finally{
            Set-Item Function:Observe-RemovalManagedSession $realObserve
            Set-Item Function:New-RemovalManagedFinish $realFinish
            Set-Item Function:Observe-RemovalProbe $realProbeObserve
            $o.Completed=$false;$o.Refused=$false;$o.BeforeSettled=$true
            if($null-ne$managed){$o.ManagedSession=$managed;$o.RetainedManaged=$managed}
            [IO.File]::WriteAllText($f.Release,'natural exit')
            foreach($tool in $tools){
                for($i=0;$i-lt100-and$null-eq$tool.CompletionObservedMilliseconds;$i++){$null=Observe-FiniteTool $tool -ObservationMilliseconds 100}
                Close-FiniteToolOwner $tool
                Check $tool.Closed 'Harmless drain fixture child remains unresolved.'
            }
            Close-LifetimeFixture $f
        }
    }
}
Case 'replaced-nested-collector-aliases-never-strand-original-child-or-observe-foreign' {
    $f=New-LifetimeFixture -PendingChild;$o=$f.Controller;$collector=$null;$preflight=$null
    try{
        Start-RemovalDiagnosticController $o
        for($i=0;$i-lt150;$i++){
            $null=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 50
            if($o.AfterStartAttempted){
                $preflight=$o.RetainedAfter.RetainedAfterPreflight;$collector=$preflight.RetainedOwner
                if($collector.Completed-and$collector.ToolOwners.Count-eq1){break}
            }
        }
        Check ($null-ne$collector-and$collector.Completed-and-not$collector.Closed) 'Original pending collector fixture absent.'
        $child=$collector.ToolOwners[0];$invocation=$collector.Invocation
        $foreign=[pscustomobject]@{Foreign=$true}
        $preflight.RetainedOwner=$foreign;$preflight.CollectorOwner=$foreign
        $r=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 1
        Check ($r.NeedsObservation-and-not$r.Completed) 'Foreign collector alias became terminal.'
        [IO.File]::WriteAllText($f.Release,'natural exit')
        for($i=0;$i-lt150-and-not$collector.Closed;$i++){
            $null=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 100
        }
        Check ($collector.Closed-and$collector.EndInvoked-and$child.Closed-and$child.ExitedConfirmed-and
            $child.ExitCode-eq0-and[Object]::ReferenceEquals($invocation,$collector.Invocation)) 'Nested alias refusal stranded original collector/child.'
        Check (@($foreign.PSObject.Properties).Count-eq1-and$null-eq$foreign.PSObject.Properties['Closed']) 'Foreign collector observed or mutated.'
        Reject {Get-RemovalClosedControllerSeal $o}
    }finally{
        if($null-ne$preflight){$preflight.RetainedOwner=$collector;$preflight.CollectorOwner=$collector}
        Close-LifetimeFixture $f
    }
}
Case 'known-lease-acquisition-refusal-is-terminal-without-export-authority' {
    $f=New-LifetimeFixture;$o=$f.Controller;$blocker=$null
    try{
        $blocker=[IO.File]::Open($o.RetainedLease.Path,[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
        Reject {Start-RemovalDiagnosticController $o}
        $r=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 1
        Check ($r.Refused-and-not$r.NeedsObservation-and-not$r.Completed-and-not$o.RetainedBefore.StartAttempted-and
            -not$o.OriginalNativeOwner.StartAttempted) 'Known lease refusal became endless pending or launched collection.'
        Reject {Get-RemovalClosedControllerSeal $o}
    }finally{if($null-ne$blocker){$blocker.Dispose()};Close-LifetimeFixture $f}
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' CONTROLLER=production WRITER=FileStream COLLECTORS=synthetic TRACE_APIS=injected NATIVE_OPERATIONS=0')
Write-Output ('FIXTURE_ROOT='+$root)
if($failed){exit 1}
