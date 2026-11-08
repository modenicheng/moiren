#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$library=Join-Path $PSScriptRoot 'collector-boundary.ps1'
if(Test-Path -LiteralPath $library){. $library}
$actualTraceAdmission=${function:Assert-RemovalManagedTraceAdmission}
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
function Reject([scriptblock]$Action){$no=$false;try{&$Action|Out-Null}catch{$no=$true};Check $no 'Expected refusal'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
function Observe-RemovalRetainedWorker {param($Owner)return @{ExitKnown=$false;NeedsObservation=$true}}
$root=Join-Path $PSScriptRoot ('task-fixtures\collector-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
Case 'managed-stage-copies-only-reviewed-scripts-never-executables-and-pins-config' {
 $exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
 $run=Join-Path $root 'stage'
 $o=New-RemovalManagedSession -RunDirectory $run -SourceDirectory $PSScriptRoot -Executable $exe -ExpectedSha256 (Get-FileHash -LiteralPath $exe).Hash `
  -ProbeArguments @('live-vad','--seconds','8') -SessionId 2 -Sid 'S-1-5-21-3080296863-1242338503-1346362208-1001' -Nonce ([Guid]::NewGuid().ToString('N'))
 $spec=Read-RemovalJsonBounded (Join-Path $run 'run-spec.json')
 Check ($spec.Libraries.Count-eq4-and$spec.Executable-ceq$exe-and$o.TaskOwner.Spec.Arguments.Contains('"-RunSpecSha256"')) 'Source/config pins missing'
 Check (@(Get-ChildItem -LiteralPath $run -File -Filter '*.exe').Count-eq0) 'Executable copied into per-run directory'
 Check ($null-eq$o.ProbeOwner-and-not$o.TaskOwner.DispatchAttempted) 'Stage dispatched native work'
 Reject {New-RemovalManagedSession -RunDirectory $run -SourceDirectory $PSScriptRoot -Executable $exe -ExpectedSha256 (Get-FileHash -LiteralPath $exe).Hash -ProbeArguments @('live-vad') -SessionId 2 -Sid 'S-1-5-21-1-2-3-1001' -Nonce ([Guid]::NewGuid().ToString('N'))}
}
Case 'staging-cannot-substitute-the-pinned-limited-task-principal' {
 $exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe';$run=Join-Path $root 'wrong-sid'
 Reject {New-RemovalManagedSession -RunDirectory $run -SourceDirectory $PSScriptRoot -Executable $exe -ExpectedSha256 (Get-FileHash -LiteralPath $exe).Hash -ProbeArguments @('live-vad') -SessionId 2 -Sid 'S-1-5-21-1-2-3-1001' -Nonce ([Guid]::NewGuid().ToString('N'))}
 Check (-not(Test-Path -LiteralPath $run)) 'Invalid principal staged files'
}
Case 'collector-never-admits-probe-before-worker-bound-and-retains-single-owner' {
 $script:events=New-Object Collections.Generic.List[string]
 $run=Join-Path $root 'observe';$null=[IO.Directory]::CreateDirectory($run)
 $session=[pscustomobject]@{RunDirectory=$run;ProbeOwner=$null;Error=$null;Terminal=$null;LastResult=$null
  ControllerPid=$PID;ControllerCreationUtcTicks=[Diagnostics.Process]::GetCurrentProcess().StartTime.ToUniversalTime().Ticks
  TaskOwner=[pscustomobject]@{WorkerBindingKnown=$false;WorkerBindingError=$null;WorkerProcess=$null;WorkerPid=5678;WorkerCreationUtcTicks=638981280000000001L;Spec=[pscustomobject]@{Nonce='a'*32;SessionId=2;Sid='S-1-5-21-1-2-3-1001'}}
  Config=@{Executable='C:\fixture.exe';ExpectedSha256='A'*64;ProbeArguments=@('live-vad')}
  TraceReadback=[pscustomobject]@{fixture='exact-trace'}}
 foreach($name in @('worker-start.json','probe-ready.json','worker-result.json')){Write-RemovalJsonOnce (Join-Path $run $name) @{fixture=$name}}
 function Attach-RemovalWorkerProcess {param($Owner,$Receipt)$script:events.Add('worker');$Owner.WorkerBindingKnown=$true}
 function New-RemovalProbeSpec {param($Executable,$ExpectedSha256,$RunDirectory,$Nonce,$SessionId,$Sid,$ControllerPid,$ControllerCreationUtcTicks,$WorkerPid,$WorkerCreationUtcTicks)return @{fixture='spec'}}
 function New-RemovalProbeOwner {param($Spec)return [pscustomobject]@{BindingKnown=$false;AdmissionAttempted=$false;Process=$null;Spec=$Spec}}
 function Attach-RemovalProbeProcess {param($Owner,$Receipt)$script:events.Add('probe');$Owner.BindingKnown=$true}
 function Admit-RemovalProbe {param($Owner)$script:events.Add('admit');$Owner.AdmissionAttempted=$true}
 function Assert-RemovalManagedTraceAdmission {param($Session)$script:events.Add('trace')}
 function Observe-RemovalProbe {param($Owner)return @{ExitKnown=$true;NeedsObservation=$false}}
 function Get-RemovalTaskObservation {param($Owner)return @{fixture='actual-observation'}}
 function Test-RemovalTaskTerminal {param($Owner,$State,$WorkerResult)return @{WorkerTerminal=$true;TaskExitCode=1}}
 function Complete-RemovalProbeTerminal {param($TaskOwner,$TaskState,$ProbeOwner,$WorkerResult)return @{Terminal=$true;NeedsObservation=$false;NativePassed=$false}}
 $result=Observe-RemovalManagedSession $session
 Check (($script:events-join',')-ceq'worker,probe,trace,admit'-and$result.Terminal-and-not$result.NativePassed) 'Ownership/admission order wrong'
 $saved=$session.ProbeOwner;$null=Observe-RemovalManagedSession $session
 Check ([object]::ReferenceEquals($saved,$session.ProbeOwner)-and$script:events.Count-eq4) 'Reacquired process or readmitted'
}
Case 'collector-without-trace-readback-refuses-before-probe-admission' {
 $run=Join-Path $root 'trace-missing';$null=[IO.Directory]::CreateDirectory($run);$script:admissions=0
 $session=[pscustomobject]@{RunDirectory=$run;ProbeOwner=[pscustomobject]@{BindingKnown=$true;AdmissionAttempted=$false}
  Error=$null;Terminal=$null;LastResult=$null;TraceReadback=$null
  ControllerPid=$PID;ControllerCreationUtcTicks=[Diagnostics.Process]::GetCurrentProcess().StartTime.ToUniversalTime().Ticks
  TaskOwner=[pscustomobject]@{WorkerBindingKnown=$true};Config=@{}}
 function Assert-RemovalManagedTraceAdmission {param($Session)throw 'No live trace readback'}
 function Admit-RemovalProbe {param($Owner)$script:admissions++;$Owner.AdmissionAttempted=$true}
 function Observe-RemovalProbe {param($Owner)return @{ExitKnown=$false;NeedsObservation=$true}}
 function Get-RemovalTaskObservation {param($Owner)return @{fixture='task'}}
 $r=Observe-RemovalManagedSession $session
 Check ($script:admissions-eq0-and$session.Error-and$r.NeedsObservation-and-not$session.ProbeOwner.AdmissionAttempted) 'Probe admitted without exact trace readback'
}
Case 'admission-failure-does-not-prevent-later-observation-of-original-probe' {
 $run=Join-Path $root 'admission-failure';$null=[IO.Directory]::CreateDirectory($run)
 $session=[pscustomobject]@{RunDirectory=$run;ProbeOwner=[pscustomobject]@{BindingKnown=$true;AdmissionAttempted=$false};Error=$null;Terminal=$null;LastResult=$null
  ControllerPid=$PID;ControllerCreationUtcTicks=[Diagnostics.Process]::GetCurrentProcess().StartTime.ToUniversalTime().Ticks
  TaskOwner=[pscustomobject]@{WorkerBindingKnown=$true};Config=@{}}
 function Assert-RemovalManagedTraceAdmission {param($Session)}
 function Admit-RemovalProbe {param($Owner)$Owner.AdmissionAttempted=$true;throw 'unknown admission publication'}
 function Observe-RemovalProbe {param($Owner)return @{ExitKnown=$true;NeedsObservation=$false}}
 $failed=Observe-RemovalManagedSession $session
 Check ($failed.Error-and-not$failed.Terminal) 'Unknown admission hidden'
 Write-RemovalJsonOnce (Join-Path $run 'worker-result.json') @{fixture='natural failed exit'}
 function Get-RemovalTaskObservation {param($Owner)return @{fixture='actual-observation'}}
 function Test-RemovalTaskTerminal {param($Owner,$State,$WorkerResult)return @{WorkerTerminal=$true;TaskExitCode=1}}
 function Complete-RemovalProbeTerminal {param($TaskOwner,$TaskState,$ProbeOwner,$WorkerResult)return @{Terminal=$true;NeedsObservation=$false;NativePassed=$false}}
 $done=Observe-RemovalManagedSession $session
 Check ($done.Terminal-and$session.ProbeOwner.AdmissionAttempted-and-not$done.NativePassed) 'Original owner could not be observed after admission error'
}
Case 'missing-worker-result-still-samples-original-worker-and-probe' {
 $run=Join-Path $root 'missing-result';$null=[IO.Directory]::CreateDirectory($run);$script:workerReads=0;$script:probeReads=0
 $session=[pscustomobject]@{RunDirectory=$run;ProbeOwner=[pscustomobject]@{BindingKnown=$true;AdmissionAttempted=$true};Error=$null;Terminal=$null;LastResult=$null
  ControllerPid=$PID;ControllerCreationUtcTicks=[Diagnostics.Process]::GetCurrentProcess().StartTime.ToUniversalTime().Ticks
  TaskOwner=[pscustomobject]@{WorkerBindingKnown=$true};Config=@{}}
 function Get-RemovalTaskObservation {param($Owner)$script:workerReads++;return @{fixture='actual-observation'}}
 function Observe-RemovalProbe {param($Owner)$script:probeReads++;return @{ExitKnown=$false;NeedsObservation=$true}}
 $r=Observe-RemovalManagedSession $session
 Check ($script:workerReads-eq1-and$script:probeReads-eq1-and$r.NeedsObservation-and-not$r.Terminal) 'Retained owners not sampled without receipt'
}
Case 'real-final-admission-guard-refreshes-exact-session-and-rejects-stale-provider' {
 . (Join-Path $PSScriptRoot 'trace-readback.ps1')
 $script:refreshes=0;$script:queryChanged=$false
 function Refresh-RemovalTraceSession {param($Session)
  $script:refreshes++
  if($script:queryChanged){$Session.Readback.Invalid=$true;throw 'Actual provider disabled or session replaced'}
 }
 $trace=[pscustomobject]@{TraceOwned=$true;TraceStopped=$false;SessionReadbackConfirmed=$true;Failed=$false}
 $readback=[pscustomobject]@{Invalid=$false;Initial=@{verified='initial'};Trace=$trace}
 $traceSession=[pscustomobject]@{Readback=$readback}
 $session=[pscustomobject]@{TraceSession=$traceSession;TraceReadback=$readback}
 &$actualTraceAdmission $session
 Check ($script:refreshes-eq1) 'Final guard did not perform a fresh query'
 $script:queryChanged=$true
 Reject {&$actualTraceAdmission $session}
 Check ($script:refreshes-eq2-and$readback.Invalid) 'Cached initial proof admitted changed session'
 $session.TraceReadback=[pscustomobject]@{Invalid=$false;Initial=@{};Trace=$trace}
 Reject {&$actualTraceAdmission $session}
 Check ($script:refreshes-eq2) 'Detached readback was substituted before refresh'
}
Case 'root-refusal-still-binds-late-original-owners-without-admitting-audio' {
 $run=Join-Path $root 'late-root-refusal';$null=[IO.Directory]::CreateDirectory($run)
 $script:events=New-Object Collections.Generic.List[string]
 $session=[pscustomobject]@{RunDirectory=$run;ProbeOwner=$null;Error='Root input provenance changed';Terminal=$null;LastResult=$null
  ControllerPid=$PID;ControllerCreationUtcTicks=[Diagnostics.Process]::GetCurrentProcess().StartTime.ToUniversalTime().Ticks
  TaskOwner=[pscustomobject]@{WorkerBindingKnown=$false;WorkerBindingError=$null;WorkerProcess=$null;WorkerPid=5678;WorkerCreationUtcTicks=638981280000000001L
   Spec=[pscustomobject]@{Nonce='a'*32;SessionId=2;Sid='S-1-5-21-1-2-3-1001'}}
  Config=@{Executable='C:\fixture.exe';ExpectedSha256='A'*64;ProbeArguments=@('live-vad')}}
 foreach($name in @('worker-start.json','probe-ready.json')){Write-RemovalJsonOnce (Join-Path $run $name) @{fixture=$name}}
 function Attach-RemovalWorkerProcess {param($Owner,$Receipt)$script:events.Add('worker');$Owner.WorkerProcess=[pscustomobject]@{fixture='retained'};$Owner.WorkerBindingKnown=$true}
 function New-RemovalProbeSpec {param($Executable,$ExpectedSha256,$RunDirectory,$Nonce,$SessionId,$Sid,$ControllerPid,$ControllerCreationUtcTicks,$WorkerPid,$WorkerCreationUtcTicks)return @{fixture='original-spec'}}
 function New-RemovalProbeOwner {param($Spec)return [pscustomobject]@{BindingKnown=$false;AdmissionAttempted=$false;Process=$null;Spec=$Spec}}
 function Attach-RemovalProbeProcess {param($Owner,$Receipt)$script:events.Add('probe');$Owner.BindingKnown=$true}
 function Admit-RemovalProbe {param($Owner)$script:events.Add('UNSAFE-ADMIT');$Owner.AdmissionAttempted=$true}
 function Observe-RemovalProbe {param($Owner)return @{ExitKnown=$false;NeedsObservation=$true}}
 function Observe-RemovalRetainedWorker {param($Owner)return @{NeedsObservation=$true}}
 function Get-RemovalTaskObservation {param($Owner)return @{fixture='original-task'}}
 $r=Observe-RemovalManagedSession $session
 Check (($script:events-join',')-ceq'worker,probe'-and$r.NeedsObservation-and-not$session.ProbeOwner.AdmissionAttempted) 'Root failure lost arriving original process or admitted audio.'
 $original=$session.ProbeOwner;$null=Observe-RemovalManagedSession $session
 Check ([Object]::ReferenceEquals($original,$session.ProbeOwner)-and$script:events.Count-eq2) 'Drain-only observation reacquired owners.'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' NATIVE_EXECUTED=false')
if($failed){exit 1}
