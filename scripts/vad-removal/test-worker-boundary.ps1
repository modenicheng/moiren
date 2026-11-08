#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$library=Join-Path $PSScriptRoot 'worker-boundary.ps1'
if(Test-Path -LiteralPath $library){. $library}
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
function Reject([scriptblock]$Action){$no=$false;try{&$Action|Out-Null}catch{$no=$true};Check $no 'Expected refusal'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
$root=Join-Path $PSScriptRoot ('task-fixtures\worker-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
function Fixture {
 $run=Join-Path $root ([Guid]::NewGuid().ToString('N'));$null=[IO.Directory]::CreateDirectory($run)
 $exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
 $s=New-RemovalProbeSpec -Executable $exe -ExpectedSha256 (Get-FileHash -LiteralPath $exe).Hash -RunDirectory $run -Nonce ([Guid]::NewGuid().ToString('N')) `
  -SessionId 2 -Sid 'S-1-5-21-1-2-3-1001' -ControllerPid 1234 -ControllerCreationUtcTicks 638981280000000000L -WorkerPid 5678 -WorkerCreationUtcTicks 638981280000000001L -ProbeArguments @('live-vad','--seconds','8')
 $o=New-RemovalWorkerProbeOwner -Spec $s
 $script:p=[pscustomobject]@{StartInfo=$null;Id=9876;Handle=[IntPtr]73;HasExited=$false;ExitCode=1;Calls=0;Throws=$false
  StartTime=[DateTime]::UtcNow;StandardOutput=[pscustomobject]@{BaseStream=[IO.MemoryStream]::new([Text.Encoding]::UTF8.GetBytes('out'))}
  StandardError=[pscustomobject]@{BaseStream=[IO.MemoryStream]::new([Text.Encoding]::UTF8.GetBytes('err'))}}
 $script:p|Add-Member ScriptMethod Start {$this.Calls++;if($this.Throws){throw 'ambiguous launch'};return $true}
 $script:p|Add-Member ScriptMethod WaitForExit {param($ms)return $this.HasExited}
 $script:p|Add-Member ScriptMethod Dispose {}
 return @($s,$o)
}
Case 'worker-retains-original-child-before-ambiguous-start-and-cannot-relaunch' {
 $s,$o=Fixture;$script:p.Throws=$true
 Start-RemovalWorkerProbe -Owner $o -ProcessFactory {$script:p}
 Check ($o.LaunchAttempted-and$o.LaunchUnknown-and$o.Errors.Count-gt0-and[object]::ReferenceEquals($o.Process,$script:p)) 'Unknown child ownership lost'
 Reject {Start-RemovalWorkerProbe -Owner $o -ProcessFactory {$script:p}}
 Check ($script:p.Calls-eq1) 'Unknown launch retried'
 $script:p.HasExited=$true;$r=Observe-RemovalWorkerProbe $o -ObservationMilliseconds 1000
 Check ($r.ExitKnown-and-not$r.NeedsObservation-and$o.IdentityConfirmed-and$script:p.Calls-eq1) 'Original ambiguous-start Process could not be observed'
 Close-RemovalWorkerProbeOwner $o
}
Case 'worker-pipes-share-a-bounded-cap-and-preserve-failed-natural-exit' {
 $s,$o=Fixture;Start-RemovalWorkerProbe -Owner $o -ProcessFactory {$script:p}
 Check ($o.IdentityConfirmed-and$o.ProcessId-eq9876-and$o.Copies.Count-eq2) 'Actual child/pipe owners missing'
 $live=Observe-RemovalWorkerProbe $o
 Check (-not$live.ExitKnown-and$live.NeedsObservation) 'Live worker child accepted'
 $script:p.HasExited=$true;$r=Observe-RemovalWorkerProbe $o -ObservationMilliseconds 1000
 Check ($r.ExitKnown-and$r.ExitCode-eq1-and$r.OutputStable-and-not$r.NeedsObservation-and-not$r.NativePassed) 'Failed natural exit/drain not preserved'
 Close-RemovalWorkerProbeOwner $o
 Check (([IO.File]::ReadAllText($o.StdoutPath))-ceq'out') 'Output not persisted'
}
Case 'worker-120-second-deadline-retains-live-owner-without-dispose-or-kill' {
 $s,$o=Fixture;Start-RemovalWorkerProbe -Owner $o -ProcessFactory {$script:p};$o.DeadlineMilliseconds=0
 $r=Observe-RemovalWorkerProbe $o
 Check ($r.TimedOut-and$r.NeedsObservation) 'Lifetime limit hidden'
 Reject {Close-RemovalWorkerProbeOwner $o}
 $script:p.HasExited=$true;$r=Observe-RemovalWorkerProbe $o -ObservationMilliseconds 1000
 Check ($r.TimedOut-and$r.ExitKnown-and-not$r.NeedsObservation) 'Observed natural exit erased sticky timeout'
 Close-RemovalWorkerProbeOwner $o
}
Case 'prelaunch-file-error-is-retained-and-cannot-overwrite-or-start' {
 $s,$o=Fixture;[IO.File]::WriteAllText($o.StderrPath,'existing')
 Start-RemovalWorkerProbe -Owner $o -ProcessFactory {$script:p}
 Check (-not$o.LaunchAttempted-and$o.Errors.Count-gt0-and$script:p.Calls-eq0) 'Child started after log collision'
 Reject {Start-RemovalWorkerProbe -Owner $o -ProcessFactory {$script:p}}
 Close-RemovalWorkerProbeOwner $o
 Check (([IO.File]::ReadAllText($o.StderrPath))-ceq'existing') 'Existing log replaced'
}
Case 'shared-output-cap-does-not-hide-overflow-or-leave-pipes-unobserved' {
 $s,$o=Fixture;$o=New-RemovalWorkerProbeOwner -Spec $s -MaxLogBytes 4
 Start-RemovalWorkerProbe -Owner $o -ProcessFactory {$script:p};$script:p.HasExited=$true
 $r=Observe-RemovalWorkerProbe $o -ObservationMilliseconds 1000
 Check ($r.ExitKnown-and$r.LogLimitExceeded-and-not$r.OutputStable-and-not$r.NeedsObservation) 'Overflow accepted or pipe ownership hidden'
 Close-RemovalWorkerProbeOwner $o
 Check (((Get-Item -LiteralPath $o.StdoutPath).Length+(Get-Item -LiteralPath $o.StderrPath).Length)-eq4) 'Combined log cap exceeded'
}
Case 'pipe-completion-between-snapshots-cannot-skip-output-observation' {
 $s,$o=Fixture;Start-RemovalWorkerProbe -Owner $o -ProcessFactory {$script:p};$script:p.HasExited=$true
 $copy=[pscustomobject]@{Reads=0}
 $copy|Add-Member ScriptProperty IsCompleted {$this.Reads++;return $this.Reads-gt1}
 $copy|Add-Member ScriptMethod Wait {param($ms)return $false}
 $o.Copies=@($copy,[Threading.Tasks.Task]::FromResult(0));$o.Flushes=@([Threading.Tasks.Task]::FromResult(0),[Threading.Tasks.Task]::FromResult(0))
 $r=Observe-RemovalWorkerProbe $o
 Check ($r.NeedsObservation-and-not$r.OutputStable) 'Pipe race skipped completed drain/flush observation'
 $o.Copies=@([Threading.Tasks.Task]::FromResult(0),[Threading.Tasks.Task]::FromResult(0));$null=Observe-RemovalWorkerProbe $o
 Close-RemovalWorkerProbeOwner $o
}
Case 'worker-entry-publishes-before-launch-and-retains-launch-result-before-return' {
 $s,$o=Fixture
 function Assert-RemovalWorkerContext {param($Spec,$TaskPath,$WorkerPath,$WorkerSha256) return @{Nonce=$Spec.Nonce;Pid=$Spec.WorkerPid;CreationUtcTicks=$Spec.WorkerCreationUtcTicks}}
 function Start-RemovalWorkerProbe {param($Owner,$ProcessFactory)
  Check (Test-Path -LiteralPath (Join-Path $Owner.Spec.RunDirectory 'worker-start.json')) 'Worker launched before identity receipt'
  $Owner.IdentityConfirmed=$true;$Owner.ProcessId=9876;$Owner.CreationUtcTicks=638981280000000002L;$Owner.ExitKnown=$true;$Owner.ExitCode=1;$Owner.OutputStable=$true}
 function Observe-RemovalWorkerProbe {param($Owner,$ObservationMilliseconds) return @{ExitKnown=$true;ExitCode=1;OutputStable=$true;NeedsObservation=$false;TimedOut=$false;LogLimitExceeded=$false;Errors=@()}}
 function Close-RemovalWorkerProbeOwner {param($Owner)$Owner.Closed=$true}
 $result=Invoke-RemovalWorker -Spec $s -TaskPath ('\Moiren-Removal613-'+$s.Nonce) -WorkerPath 'fixture.ps1' -WorkerSha256 ('A'*64)
 $saved=Read-RemovalJsonBounded (Join-Path $s.RunDirectory 'worker-result.json')
 Check ($result.ProbePid-eq9876-and$result.ProbeExitKnown-and$result.WorkerExitCode-eq1-and$saved.ProbeCreationUtcTicks-eq638981280000000002L) 'Worker discarded exact failed result'
 Check ($result.Owners.Count-eq0-and$result.OutputStable-and-not$result.NativePassed) 'Worker claimed acceptance or left resolved owner'
}
Case 'worker-does-not-abandon-a-live-probe-at-observation-deadline' {
 $s,$o=Fixture;$script:observations=0
 function Assert-RemovalWorkerContext {param($Spec,$TaskPath,$WorkerPath,$WorkerSha256) return @{Nonce=$Spec.Nonce;Pid=$Spec.WorkerPid;CreationUtcTicks=$Spec.WorkerCreationUtcTicks}}
 function Start-RemovalWorkerProbe {param($Owner,$ProcessFactory)$Owner.IdentityConfirmed=$true;$Owner.ProcessId=9876;$Owner.CreationUtcTicks=638981280000000002L}
 function Observe-RemovalWorkerProbe {param($Owner,$ObservationMilliseconds)
  $script:observations++
  if($script:observations-eq1){return @{ExitKnown=$false;ExitCode=$null;OutputStable=$false;NeedsObservation=$true;TimedOut=$true;LogLimitExceeded=$false;Errors=@()}}
  Check (Test-Path -LiteralPath (Join-Path $Owner.Spec.RunDirectory 'worker-pending.json')) 'No bounded pending receipt'
  Check (-not(Test-Path -LiteralPath (Join-Path $Owner.Spec.RunDirectory 'worker-result.json'))) 'Unresolved owner published terminal result'
  return @{ExitKnown=$true;ExitCode=1;OutputStable=$true;NeedsObservation=$false;TimedOut=$true;LogLimitExceeded=$false;Errors=@()}
 }
 function Close-RemovalWorkerProbeOwner {param($Owner)$Owner.Closed=$true}
 $result=Invoke-RemovalWorker -Spec $s -TaskPath ('\Moiren-Removal613-'+$s.Nonce) -WorkerPath 'fixture.ps1' -WorkerSha256 ('A'*64)
 Check ($script:observations-eq2-and$result.ProbeExitKnown-and$result.TimedOut) 'Worker abandoned live probe or erased timeout'
}
Case 'failed-receipt-publication-keeps-observing-the-original-child' {
 $s,$o=Fixture;$script:observations=0
 function Assert-RemovalWorkerContext {param($Spec,$TaskPath,$WorkerPath,$WorkerSha256) return @{Nonce=$Spec.Nonce;Pid=$Spec.WorkerPid;CreationUtcTicks=$Spec.WorkerCreationUtcTicks}}
 $writer=${function:Write-RemovalJsonOnce}
 function Write-RemovalJsonOnce {param($Path,$Value)
  if([IO.Path]::GetFileName($Path)-ceq'worker-probe-start.json'){throw 'disk publication failed'}
  &$writer $Path $Value}
 function Start-RemovalWorkerProbe {param($Owner,$ProcessFactory)$Owner.IdentityConfirmed=$true;$Owner.ProcessId=9876;$Owner.CreationUtcTicks=638981280000000002L}
 function Observe-RemovalWorkerProbe {param($Owner,$ObservationMilliseconds)$script:observations++;return @{ExitKnown=$true;ExitCode=1;OutputStable=$true;NeedsObservation=$false;TimedOut=$false;LogLimitExceeded=$false;Errors=@()}}
 function Close-RemovalWorkerProbeOwner {param($Owner)$Owner.Closed=$true}
 $result=Invoke-RemovalWorker -Spec $s -TaskPath ('\Moiren-Removal613-'+$s.Nonce) -WorkerPath 'fixture.ps1' -WorkerSha256 ('A'*64)
 Check ($script:observations-eq1-and$result.ProbeExitKnown-and$result.WorkerExitCode-eq1) 'Receipt failure abandoned child'
}
Case 'ambiguous-start-is-observed-on-the-same-process-without-identity-reacquisition' {
 $s,$o=Fixture;$script:observations=0
 function Assert-RemovalWorkerContext {param($Spec,$TaskPath,$WorkerPath,$WorkerSha256) return @{Nonce=$Spec.Nonce;Pid=$Spec.WorkerPid;CreationUtcTicks=$Spec.WorkerCreationUtcTicks}}
 function Start-RemovalWorkerProbe {param($Owner,$ProcessFactory)$Owner.LaunchAttempted=$true;$Owner.LaunchUnknown=$true;$Owner.IdentityConfirmed=$false;$Owner.Process=[pscustomobject]@{fixture='original'}}
 function Observe-RemovalWorkerProbe {param($Owner,$ObservationMilliseconds)
  $script:observations++
  Check ($Owner.Process.fixture-ceq'original') 'Original process replaced'
  if($script:observations-eq1){return @{ExitKnown=$false;ExitCode=$null;OutputStable=$false;NeedsObservation=$true;TimedOut=$false;LogLimitExceeded=$false;Errors=@()}}
  return @{ExitKnown=$true;ExitCode=1;OutputStable=$false;NeedsObservation=$false;TimedOut=$true;LogLimitExceeded=$false;Errors=@()}}
 function Close-RemovalWorkerProbeOwner {param($Owner)$Owner.Closed=$true}
 $result=Invoke-RemovalWorker -Spec $s -TaskPath ('\Moiren-Removal613-'+$s.Nonce) -WorkerPath 'fixture.ps1' -WorkerSha256 ('A'*64)
 Check ($script:observations-eq2-and$result.ProbeExitKnown) 'Unknown dispatch dropped original owner'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' NATIVE_EXECUTED=false')
if($failed){exit 1}
