#requires -Version 5.1
param([string]$Library)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if(-not $Library){$Library=Join-Path $PSScriptRoot 'probe-boundary.ps1'}
if(Test-Path -LiteralPath $Library){. $Library}
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
function Reject([scriptblock]$Action){$no=$false;try{&$Action|Out-Null}catch{$no=$true};Check $no 'Expected refusal'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
$root=Join-Path $PSScriptRoot ('task-fixtures\probe-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
function Spec {
 $exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
 New-RemovalProbeSpec -Executable $exe -ExpectedSha256 (Get-FileHash -LiteralPath $exe).Hash `
  -RunDirectory $root -Nonce ([Guid]::NewGuid().ToString('N')) -SessionId 2 -Sid 'S-1-5-21-1-2-3-1001' `
  -ControllerPid 1234 -ControllerCreationUtcTicks 638981280000000000L `
  -WorkerPid 5678 -WorkerCreationUtcTicks ([DateTime]::UtcNow.Ticks) -ProbeArguments @('live-vad','--seconds','8')
}
function Ready($s) {
 @{schema_version=1;nonce=$s.Nonce;probe_pid=9876;probe_creation_utc_ticks=[DateTime]::UtcNow.Ticks
  controller_pid=$s.ControllerPid;controller_creation_utc_ticks=$s.ControllerCreationUtcTicks
  worker_pid=$s.WorkerPid;worker_creation_utc_ticks=$s.WorkerCreationUtcTicks;session_id=$s.SessionId}
}
function FakeProcess($s,$r) {
 $p=[pscustomobject]@{Id=[int]$r.probe_pid;Handle=[IntPtr]53;StartTime=([DateTime]::new($r.probe_creation_utc_ticks,[DateTimeKind]::Utc)).ToLocalTime()
  SessionId=$s.SessionId;HasExited=$false;ExitCode=1;MainModule=[pscustomobject]@{FileName=$s.Executable}}
 $p|Add-Member ScriptMethod Refresh {}
 $p|Add-Member ScriptMethod WaitForExit {param($ms)return $this.HasExited}
 return $p
}
function Open-RemovalProbeProcess([int]$ProbePid){$script:opens++;return $script:process}
function Read-RemovalProbeMetadata($Process) {return $script:metadata}
function Fixture {
 $s=Spec;$r=Ready $s;$o=New-RemovalProbeOwner -Spec $s
 $script:opens=0;$script:process=FakeProcess $s $r
 $script:metadata=@{Pid=$r.probe_pid;ParentPid=$s.WorkerPid;Sid=$s.Sid;IsAdministrator=$false;IsElevated=$false
  Executable=$s.Executable;CommandLine=('"'+$s.Executable+'" '+$s.CommandLine)}
 return @($s,$r,$o)
}
Case 'receipt-integer-types-cannot-be-replaced-by-numeric-strings-or-booleans' {
 foreach($field in @('schema_version','controller_pid','controller_creation_utc_ticks','worker_pid','worker_creation_utc_ticks','session_id','probe_pid','probe_creation_utc_ticks')){
  $s,$r,$o=Fixture;$r[$field]=[string]$r[$field]
  Reject {Attach-RemovalProbeProcess $o $r}
  Check ($script:opens-eq0) 'String receipt number accepted'
 }
}
Case 'probe-spec-keeps-120-second-lifetime-separate-from-15-second-tools' {
 $s=Spec
 Check ($s.ObservationSeconds-eq120-and$s.Arguments[0]-ceq'managed-removal') 'Missing managed entry or lifecycle deadline'
 Check ($s.CommandLine.Contains('"--admission-timeout-ms" "15000"')-and$s.Arguments[-1]-ceq'8') 'Admission or original arguments changed'
 Reject {New-RemovalProbeSpec -Executable $s.Executable -ExpectedSha256 $s.ExpectedSha256 -RunDirectory $root -Nonce $s.Nonce -SessionId 2 -Sid $s.Sid -ControllerPid 1234 -ControllerCreationUtcTicks 1L -WorkerPid 5678 -WorkerCreationUtcTicks 2L -ProbeArguments @('offline')}
}
Case 'bounded-create-new-json-preserves-existing-and-refuses-reparse' {
 $path=Join-Path $root 'receipt.json';Write-RemovalJsonOnce $path @{a=1}
 Check ((Read-RemovalJsonBounded $path).a-eq1) 'Atomic receipt absent'
 Reject {Write-RemovalJsonOnce $path @{a=2}}
 Check ((Read-RemovalJsonBounded $path).a-eq1) 'Existing receipt replaced'
 [IO.File]::WriteAllText((Join-Path $root 'oversize.json'),('x'*8193))
 Reject {Read-RemovalJsonBounded (Join-Path $root 'oversize.json')}
}
Case 'live-probe-binds-exact-original-handle-parent-token-and-commandline' {
 $s,$r,$o=Fixture
 Attach-RemovalProbeProcess $o $r
 Check ($o.BindingKnown-and$o.ProcessId-eq9876-and[object]::ReferenceEquals($o.Process,$script:process)-and$script:opens-eq1) 'Original probe binding lost'
 Reject {Attach-RemovalProbeProcess $o $r}
 Check ($script:opens-eq1) 'Probe PID reacquired'
}
Case 'binding-error-retains-first-process-and-refuses-retry' {
 $s,$r,$o=Fixture;$script:metadata.ParentPid++
 Reject {Attach-RemovalProbeProcess $o $r}
 Check (-not$o.BindingKnown-and$o.BindingError-and[object]::ReferenceEquals($o.Process,$script:process)) 'Failed binding dropped process'
 Reject {Attach-RemovalProbeProcess $o $r}
 Check ($script:opens-eq1) 'Failed binding reacquired'
}
Case 'receipt-mismatch-refuses-before-pid-open' {
 foreach($field in @('nonce','controller_pid','controller_creation_utc_ticks','worker_pid','worker_creation_utc_ticks','session_id')){
  $s,$r,$o=Fixture
  if($field-ceq'nonce'){$r[$field]='wrong'}else{$r[$field]++}
  Reject {Attach-RemovalProbeProcess $o $r}
  Check ($script:opens-eq0) 'Untrusted receipt PID opened'
 }
}
Case 'binding-checks-real-metadata-not-json-privilege-or-exit-claims' {
 foreach($mode in @('creation','session','image','commandline','administrator','elevated','token-type','dead')){
  $s,$r,$o=Fixture
  switch($mode){'creation'{$script:process.StartTime=$script:process.StartTime.AddTicks(1)};'session'{$script:process.SessionId++}
   'image'{$script:process.MainModule.FileName='C:\other.exe'};'commandline'{$script:metadata.CommandLine+=' extra'}
   'administrator'{$script:metadata.IsAdministrator=$true};'elevated'{$script:metadata.IsElevated=$true}
   'token-type'{$script:metadata.IsElevated='false'};'dead'{$script:process.HasExited=$true}}
  Reject {Attach-RemovalProbeProcess $o $r};Check (-not$o.BindingKnown) 'Mismatched process bound'
 }
}
Case 'admission-requires-live-bound-child-and-creates-exact-ready-ack' {
 $s,$r,$o=Fixture;$s.RunDirectory=Join-Path $root 'admit';$null=[IO.Directory]::CreateDirectory($s.RunDirectory)
 Reject {Admit-RemovalProbe $o}
 Attach-RemovalProbeProcess $o $r
 Admit-RemovalProbe $o
 $ack=Read-RemovalJsonBounded (Join-Path $s.RunDirectory 'probe-admit.json')
 Check ($o.AdmissionWritten-and$ack.probe_pid-eq9876-and$ack.nonce-ceq$s.Nonce) 'Independent admission missing'
 Reject {Admit-RemovalProbe $o}
}
Case 'natural-exit-is-independent-from-worker-reported-probe-code' {
 $s,$r,$o=Fixture;Attach-RemovalProbeProcess $o $r
 $live=Observe-RemovalProbe $o
 Check (-not$live.ExitKnown-and$live.NeedsObservation) 'Live child called exited'
 $script:process.HasExited=$true;$done=Observe-RemovalProbe $o
 Check ($done.ExitKnown-and$done.ExitCode-eq1-and-not$done.NeedsObservation-and-not$done.NativePassed) 'Natural failed exit lost'
}
Case 'timeout-is-sticky-without-kill-dispose-or-second-launch' {
 $s,$r,$o=Fixture;Attach-RemovalProbeProcess $o $r
 $o.Clock=[Diagnostics.Stopwatch]::StartNew();$o.DeadlineMilliseconds=0
 $late=Observe-RemovalProbe $o
 Check ($late.TimedOut-and$late.NeedsObservation) 'Observation timeout hidden'
 $script:process.HasExited=$true;$done=Observe-RemovalProbe $o
 Check ($done.TimedOut-and$done.ExitKnown-and$script:opens-eq1) 'Later exit erased timeout or replaced handle'
}
Case 'full-terminal-requires-independent-owned-probe-exit-and-matching-worker-result' {
 $s,$r,$o=Fixture;Attach-RemovalProbeProcess $o $r
 $worker=@{ProbePid=$r.probe_pid;ProbeCreationUtcTicks=$r.probe_creation_utc_ticks;ProbeExitKnown=$true;ProbeExitCode=1;OutputStable=$true;Owners=@()}
 $task=[pscustomobject]@{LastObservation=@{exact='task state'};Spec=$s;WorkerPid=$s.WorkerPid;WorkerCreationUtcTicks=$s.WorkerCreationUtcTicks}
 function Test-RemovalTaskTerminal {param($Owner,$State,$WorkerResult)
  Check ([object]::ReferenceEquals($State,$Owner.LastObservation)) 'Task proof was detached'
  return @{WorkerTerminal=$true;TaskExitCode=1}}
 $pending=Complete-RemovalProbeTerminal $task $task.LastObservation $o $worker
 Check (-not$pending.Terminal-and$pending.NeedsObservation) 'Live probe accepted from worker JSON'
 $script:process.HasExited=$true;$done=Complete-RemovalProbeTerminal $task $task.LastObservation $o $worker
 Check ($done.Terminal-and-not$done.NeedsObservation-and$done.ProbeExitKnown-and$done.ProbeExitCode-eq1-and-not$done.NativePassed) 'Independent failed child exit not retained'
 $worker.ProbeExitCode=0;$mismatch=Complete-RemovalProbeTerminal $task $task.LastObservation $o $worker
 Check (-not$mismatch.Terminal) 'Contradictory child exit accepted'
}
Case 'full-terminal-cannot-use-detached-or-string-worker-proof' {
 $s,$r,$o=Fixture;Attach-RemovalProbeProcess $o $r;$script:process.HasExited=$true
 $worker=@{ProbePid=$r.probe_pid;ProbeCreationUtcTicks=$r.probe_creation_utc_ticks;ProbeExitKnown=$true;ProbeExitCode=1;OutputStable=$true;Owners=@()}
 $owner=[pscustomobject]@{LastObservation=@{exact='task state'};Spec=$s;WorkerPid=$s.WorkerPid;WorkerCreationUtcTicks=$s.WorkerCreationUtcTicks}
 function Test-RemovalTaskTerminal {param($Owner,$State,$WorkerResult)return @{WorkerTerminal='false';TaskExitCode=1}}
 $bad=Complete-RemovalProbeTerminal $owner $owner.LastObservation $o $worker
 Check (-not$bad.Terminal) 'String false task proof accepted'
}
Case 'terminal-cannot-compose-a-probe-owned-by-a-different-run-or-worker' {
 foreach($field in @('Nonce','RunDirectory','SessionId','Sid','WorkerPid','WorkerCreationUtcTicks')){
  $s,$r,$o=Fixture;Attach-RemovalProbeProcess $o $r;$script:process.HasExited=$true
  $other=[pscustomobject]@{Nonce=$s.Nonce;RunDirectory=$s.RunDirectory;SessionId=$s.SessionId;Sid=$s.Sid}
  $task=[pscustomobject]@{LastObservation=@{exact='task state'};Spec=$other;WorkerPid=$s.WorkerPid;WorkerCreationUtcTicks=$s.WorkerCreationUtcTicks}
  if($field-cin@('WorkerPid','WorkerCreationUtcTicks')){$task.$field++}
  elseif($field-ceq'SessionId'){$other.$field++}else{$other.$field+='-other'}
  function Test-RemovalTaskTerminal {param($Owner,$State,$WorkerResult)return @{WorkerTerminal=$true;TaskExitCode=1}}
  $worker=@{ProbePid=$r.probe_pid;ProbeCreationUtcTicks=$r.probe_creation_utc_ticks;ProbeExitKnown=$true;ProbeExitCode=1;OutputStable=$true;Owners=@()}
  $bad=Complete-RemovalProbeTerminal $task $task.LastObservation $o $worker
  Check (-not$bad.Terminal) ('Cross-scope probe accepted: '+$field)
 }
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' NATIVE_EXECUTED=false')
if($failed){exit 1}
