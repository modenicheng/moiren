#requires -Version 5.1
param([string]$Library)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if(-not $Library){$Library=Join-Path $PSScriptRoot 'task-boundary.ps1'}
if(Test-Path -LiteralPath $Library){. $Library}
$nativeApprovalGuard=${function:Assert-RemovalTaskNativeApproval}
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
function Reject([scriptblock]$Action){$no=$false;try{&$Action|Out-Null}catch{$no=$true};Check $no 'Expected refusal'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
$root=Join-Path $PSScriptRoot ('task-fixtures\'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$worker=Join-Path $root 'worker.ps1';[IO.File]::WriteAllText($worker,'# mock entry, never executed')
function Spec {New-RemovalTaskSpec -WorkerPath $worker -WorkerSha256 (Get-FileHash -LiteralPath $worker).Hash -RunDirectory $root -SessionId 2 -Nonce ([Guid]::NewGuid().ToString('N'))}
function Assert-RemovalTaskHost {}
function Assert-RemovalTaskNativeApproval {}
function FakeTask($s){
 $t=[pscustomobject]@{Path=$s.Path;Name=$s.Name;Enabled=$true;State=3;RunCalls=0;RunThrows=$false;Instances=0;LastTaskResult=267011;LastRunTime=[DateTime]'1899-12-30';Xml='<exact-created-task/>'
  Definition=[pscustomobject]@{Principal=[pscustomobject]@{UserId=$s.Sid;LogonType=3;RunLevel=0};Triggers=[pscustomobject]@{Count=0};Actions=[pscustomobject]@{Count=1;Action=[pscustomobject]@{Type=0;Path=$s.Executable;Arguments=$s.Arguments;WorkingDirectory=$s.RunDirectory}};Settings=[pscustomobject]@{AllowDemandStart=$true;Enabled=$true;MultipleInstances=2;RestartCount=0;AllowHardTerminate=$false;ExecutionTimeLimit='PT0S';StopIfGoingOnBatteries=$false;DisallowStartIfOnBatteries=$false}}}
 $t.Definition.Actions|Add-Member ScriptMethod Item {param($i)if($i-ne1){throw 'Unexpected action index'};return $this.Action}
 $t|Add-Member ScriptMethod GetInstances {param($flags)return [pscustomobject]@{Count=$this.Instances}}
 $t|Add-Member ScriptMethod RunEx {param($parameters,$flags,$session,$user)$this.RunCalls++;if($this.RunThrows){throw 'ambiguous scheduler dispatch'};if($flags-ne4-or$session-ne2-or$null-ne$user){throw 'Wrong RunEx arguments'};return [pscustomobject]@{InstanceGuid='{d5d7acdf-e49d-4134-ac20-3d965accb18c}';Path=$this.Path;EnginePID=800}}
 return $t
}
function Owner($s,$t){New-RemovalTaskOwner -Spec $s -RegisteredTask $t}
function FakeScheduler($s,[string]$Mode) {
 $task=FakeTask $s
 $task.Definition|Add-Member NoteProperty RegistrationInfo ([pscustomobject]@{Description=''})
 $task.Definition.Actions|Add-Member ScriptMethod Create {param($type)return $this.Action}
 $folder=[pscustomobject]@{Task=$task;Mode=$Mode;Calls=0}
 $folder|Add-Member ScriptMethod RegisterTaskDefinition {
  param($name,$definition,$flags,$sid,$password,$logon,$sddl)
  $this.Calls++
  if($flags-ne2-or$logon-ne3-or$null-ne$password){throw 'Wrong registration arguments'}
  if($this.Mode-ceq'throw'){throw 'registration may already have committed'}
  if($this.Mode-ceq'wrong'){$this.Task.Name='unexpected'}
  return $this.Task
 }
 $scheduler=[pscustomobject]@{Folder=$folder;Task=$task;ConnectCalls=0}
 $scheduler|Add-Member ScriptMethod Connect {$this.ConnectCalls++}
 $scheduler|Add-Member ScriptMethod GetFolder {param($path)return $this.Folder}
 $scheduler|Add-Member ScriptMethod NewTask {param($flags)return $this.Task.Definition}
 return $scheduler
}
function New-RemovalTaskScheduler {return $script:mockScheduler}
Case 'real-native-approval-guard-refuses-unreviewed-preparation' {
 Reject {&$nativeApprovalGuard}
 $s=Spec;$s.ReviewedForNative=$true
 Reject {&$nativeApprovalGuard $s}
}
Case 'noncanonical-and-reparse-task-paths-refuse-before-dispatch' {
 foreach($path in @('D:\work\..\worker.ps1','D:\work\','D:\work\worker.ps1:stream','\\server\share\worker.ps1')){
  Reject {Assert-RemovalTaskPath $path}
 }
 function Get-Item {param($LiteralPath,[switch]$Force,$ErrorAction)
  [pscustomobject]@{Attributes=[IO.FileAttributes]::ReparsePoint;PSIsContainer=$true}}
 Reject {Assert-RemovalTaskPath $root}
}
function WorkerReceipt($s) {
 return @{Nonce=$s.Nonce;TaskPath=$s.Path;Host='LAPTOP-NP49RG6H';Sid=$s.Sid;SessionId=$s.SessionId
  IsAdministrator=$false;Pid=5678;CreationUtcTicks=[DateTime]::UtcNow.Ticks;WorkerSha256=$s.WorkerSha256
  ProbeExitKnown=$true;ProbeExitCode=1;Owners=@();WorkerExitCode=1}
}
function FakeWorker($s,$r) {
 $p=[pscustomobject]@{Id=$r.Pid;Handle=41;StartTime=([DateTime]::new($r.CreationUtcTicks,[DateTimeKind]::Utc)).ToLocalTime()
  SessionId=$s.SessionId;MainModule=[pscustomobject]@{FileName=$s.Executable};HasExited=$false;ExitCode=1}
 $p|Add-Member ScriptMethod Refresh {}
 return $p
}
function Open-RemovalWorkerProcess([int]$WorkerPid) {return $script:mockWorker}
function Read-RemovalWorkerProcessMetadata($Process) {return $script:mockWorkerMetadata}
function BindFakeWorker($o,$r) {
 $script:mockWorker=FakeWorker $o.Spec $r
 $script:mockWorkerMetadata=@{Pid=$r.Pid;Sid=$o.Spec.Sid;IsAdministrator=$false;IsElevated=$false;Executable=$o.Spec.Executable
  CommandLine=('"'+$o.Spec.Executable+'" '+$o.Spec.Arguments)}
 Attach-RemovalWorkerProcess -Owner $o -Receipt $r
}
function CompleteFakeTask($o,[int]$Code=1) {
 $o.RegisteredTask.LastTaskResult=$Code
 $o.RegisteredTask.LastRunTime=[DateTime]::Now
 return Get-RemovalTaskObservation $o
}
Case 'registration-intent-remains-owned-after-ambiguous-commit' {
 $s=Spec;$o=New-RemovalTaskOwner -Spec $s
 $script:mockScheduler=FakeScheduler $s 'throw'
 Register-RemovalTaskDefinition -Owner $o
 Check ($o.RegistrationAttempted-and-not$o.RegistrationKnown-and$o.Error) 'Ambiguous registration lost'
 Check ([object]::ReferenceEquals($o.Scheduler,$script:mockScheduler)-and
  [object]::ReferenceEquals($o.Folder,$script:mockScheduler.Folder)-and$null-ne$o.TaskDefinition) 'Original COM owners lost'
 Reject {Register-RemovalTaskDefinition -Owner $o}
 Reject {Start-RemovalTaskOnce $o}
 Check ($script:mockScheduler.Folder.Calls-eq1) 'Repeated unknown registration'
}
Case 'pre-com-admission-refusal-is-not-an-unknown-registration' {
 $s=Spec;$o=New-RemovalTaskOwner -Spec $s;$script:mockScheduler=FakeScheduler $s 'ok'
 function Assert-RemovalTaskNativeApproval($Owner,[string]$Operation){if($Operation-ceq'TaskRegisterDispatch'){throw 'Fresh admission expired before COM.'}}
 Register-RemovalTaskDefinition -Owner $o
 Check (-not$o.RegistrationAttempted-and-not$o.RegistrationKnown-and$o.Error-and$script:mockScheduler.Folder.Calls-eq0) 'Predispatch refusal invented an unresolved native registration.'
}
Case 'final-runex-admission-refusal-does-not-dispatch' {
 $s=Spec;$task=FakeTask $s;$o=New-RemovalTaskOwner -Spec $s -RegisteredTask $task
 $task.State=3;$task.LastTaskResult=267011
 function Assert-RemovalTaskNativeApproval($Owner,[string]$Operation){if($Operation-ceq'TaskDispatchInvoke'){throw 'Fresh admission expired before RunEx.'}}
 Reject {Start-RemovalTaskOnce $o}
 Check (-not$o.DispatchAttempted-and$task.RunCalls-eq0) 'Final admission refusal dispatched RunEx or invented unknown dispatch.'
}
Case 'wrong-registration-return-is-retained-and-cannot-dispatch' {
 $s=Spec;$o=New-RemovalTaskOwner -Spec $s
 $script:mockScheduler=FakeScheduler $s 'wrong'
 Register-RemovalTaskDefinition -Owner $o
 Check ($o.RegistrationAttempted-and-not$o.RegistrationKnown-and$null-ne$o.RegisteredTask-and$o.Error) 'Returned invalid task lost'
 Reject {Start-RemovalTaskOnce $o}
 Check ($o.RegisteredTask.RunCalls-eq0) 'Invalid registration dispatched'
}
Case 'registration-success-uses-create-only-and-retains-exact-task' {
 $s=Spec;$o=New-RemovalTaskOwner -Spec $s
 $script:mockScheduler=FakeScheduler $s 'success'
 Register-RemovalTaskDefinition -Owner $o
 Check ($o.RegistrationKnown-and$o.RegistrationAttempted-and$null-eq$o.Error-and
  [object]::ReferenceEquals($o.RegisteredTask,$script:mockScheduler.Task)) 'Exact created task not retained'
 Start-RemovalTaskOnce $o
 Check ($o.DispatchAttempted-and$o.RegisteredTask.RunCalls-eq1) 'Created task not dispatched exactly once'
}
Case 'spec-binds-observed-session-nonce-argument-and-exact-worker' {
 $s=Spec;Check ($s.SessionId-eq2-and$s.Path-ceq('\'+$s.Name)-and$s.Sid-ceq'S-1-5-21-3080296863-1242338503-1346362208-1001') 'Wrong identity'
 Check ($s.Arguments.Contains($s.Nonce)-and$s.Arguments.Contains($s.WorkerPath)-and-not$s.ReviewedForNative) 'Missing exact nonce/entry or false approval'
}
Case 'managed-worker-spec-pins-run-configuration-in-exact-task-commandline' {
 $s=New-RemovalTaskSpec -WorkerPath $worker -WorkerSha256 (Get-FileHash -LiteralPath $worker).Hash -RunDirectory $root -SessionId 2 -Nonce ([Guid]::NewGuid().ToString('N')) -RunSpecSha256 ('B'*64)
 Check ($s.Arguments.Contains('"-RunSpecSha256" "'+('B'*64)+'"')) 'Run configuration hash absent from task action'
}
Case 'definition-check-rejects-extra-actions-triggers-or-highest-token' {
 foreach($mode in @('action','trigger','privilege','session')){
  $s=Spec;$t=FakeTask $s
  switch($mode){'action'{$t.Definition.Actions.Count=2};'trigger'{$t.Definition.Triggers.Count=1};'privilege'{$t.Definition.Principal.RunLevel=1};'session'{$t.Definition.Principal.UserId='S-1-5-18'}}
  Reject {Owner $s $t}
 }
}
Case 'exact-once-dispatch-retains-running-instance-not-engine-as-worker-pid' {
 $s=Spec;$t=FakeTask $s;$o=Owner $s $t;Start-RemovalTaskOnce $o
 Check ($o.DispatchAttempted-and$o.InstanceGuid-ceq'{d5d7acdf-e49d-4134-ac20-3d965accb18c}'-and$o.RunningTask.Path-ceq$s.Path-and$t.RunCalls-eq1) 'Task instance lost'
 Check ($null-eq$o.WorkerProcess) 'Scheduler EnginePID incorrectly adopted as worker'
 Reject {Start-RemovalTaskOnce $o};Check ($t.RunCalls-eq1) 'Dispatched twice'
}
Case 'unknown-dispatch-is-sticky-and-never-reruns' {
 $s=Spec;$t=FakeTask $s;$t.RunThrows=$true;$o=Owner $s $t;Start-RemovalTaskOnce $o
 Check ($o.DispatchAttempted-and$null-eq$o.RunningTask-and$o.Error-and$t.RunCalls-eq1) 'Unknown dispatch not retained'
 Reject {Start-RemovalTaskOnce $o};Check ($t.RunCalls-eq1) 'Retried unknown dispatch'
}
Case 'modified-definition-or-live-instance-refuses-before-dispatch' {
 foreach($mode in @('path','live','queued','hasrun')){
  $s=Spec;$t=FakeTask $s;$o=Owner $s $t
  switch($mode){'path'{$t.Definition.Actions.Action.Path='C:\other.exe'};'live'{$t.Instances=1};'queued'{$t.State=2};'hasrun'{$t.LastTaskResult=0}}
  Reject {Start-RemovalTaskOnce $o};Check ($t.RunCalls-eq0-and-not$o.DispatchAttempted) 'Dispatch despite changed/busy task'
 }
}
Case 'worker-start-receipt-binds-session-sid-nonce-and-real-creation' {
 $s=Spec;$o=Owner $s (FakeTask $s);Start-RemovalTaskOnce $o
 $r=@{Nonce=$s.Nonce;TaskPath=$s.Path;Host='LAPTOP-NP49RG6H';Sid=$s.Sid;SessionId=2;IsAdministrator=$false;Pid=5678;CreationUtcTicks=638981280000000000L;WorkerSha256=$s.WorkerSha256}
 Assert-RemovalWorkerReceipt $o $r
 $r.Nonce='wrong';Reject {Assert-RemovalWorkerReceipt $o $r}
 $r.Nonce=$s.Nonce;$r.IsAdministrator=$true;Reject {Assert-RemovalWorkerReceipt $o $r}
}
Case 'terminal-task-without-run-and-worker-exit-is-not-complete' {
 $s=Spec;$o=Owner $s (FakeTask $s);Start-RemovalTaskOnce $o
 $r=Test-RemovalTaskTerminal $o @{State=3;LastTaskResult=267011;LastRunKnown=$false;Instances=0} $null
 Check (-not$r.Terminal-and$r.NeedsObservation) 'Unrun task called terminal'
}
Case 'detached-worker-result-and-caller-run-boolean-cannot-establish-terminal' {
 $s=Spec;$o=Owner $s (FakeTask $s);Start-RemovalTaskOnce $o
 $r=Test-RemovalTaskTerminal $o @{State=3;LastTaskResult=1;LastRunKnown=$true;Instances=0} (WorkerReceipt $s)
 Check (-not$r.Terminal) 'Detached receipt accepted without a retained exited worker'
}
Case 'worker-binding-retains-process-before-metadata-failure-and-never-reacquires' {
 $s=Spec;$o=Owner $s (FakeTask $s);Start-RemovalTaskOnce $o;$r=WorkerReceipt $s
 $script:mockWorker=FakeWorker $s $r
 $script:mockWorkerMetadata=@{Pid=$r.Pid;Sid='S-1-5-18';IsAdministrator=$false;IsElevated=$false;Executable=$s.Executable;CommandLine=('"'+$s.Executable+'" '+$s.Arguments)}
 Reject {Attach-RemovalWorkerProcess -Owner $o -Receipt $r}
 Check ([object]::ReferenceEquals($o.WorkerProcess,$script:mockWorker)-and-not$o.WorkerBindingKnown-and$o.WorkerBindingError) 'Failed initial binding not retained'
 Reject {Attach-RemovalWorkerProcess -Owner $o -Receipt $r}
}
Case 'worker-binding-checks-real-creation-session-image-and-commandline' {
 foreach($mode in @('pid','creation','session','image','commandline','old','administrator','elevated','untyped-token')){
  $s=Spec;$o=Owner $s (FakeTask $s);Start-RemovalTaskOnce $o;$r=WorkerReceipt $s
  $script:mockWorker=FakeWorker $s $r
  $script:mockWorkerMetadata=@{Pid=$r.Pid;Sid=$s.Sid;IsAdministrator=$false;IsElevated=$false;Executable=$s.Executable;CommandLine=('"'+$s.Executable+'" '+$s.Arguments)}
  switch($mode){'pid'{$script:mockWorker.Id++};'creation'{$script:mockWorker.StartTime=$script:mockWorker.StartTime.AddTicks(1)}
   'session'{$script:mockWorker.SessionId++};'image'{$script:mockWorker.MainModule.FileName='C:\other.exe'}
   'commandline'{$script:mockWorkerMetadata.CommandLine+=' unexpected'}
   'administrator'{$script:mockWorkerMetadata.IsAdministrator=$true};'elevated'{$script:mockWorkerMetadata.IsElevated=$true}
   'untyped-token'{$script:mockWorkerMetadata.IsElevated='false'}
   'old'{$r.CreationUtcTicks=[DateTime]::UtcNow.AddMinutes(-1).Ticks
    $script:mockWorker.StartTime=([DateTime]::new($r.CreationUtcTicks,[DateTimeKind]::Utc)).ToLocalTime()}}
  Reject {Attach-RemovalWorkerProcess -Owner $o -Receipt $r}
  Check (-not$o.WorkerBindingKnown) 'Mismatched worker adopted'
 }
}
Case 'task-observation-rejects-unrun-stale-and-unknown-dispatch' {
 $s=Spec;$o=Owner $s (FakeTask $s);Start-RemovalTaskOnce $o
 $state=Get-RemovalTaskObservation $o
 Check (-not$state.LastRunKnown) 'Never-run scheduler task accepted'
 $o.RegisteredTask.LastTaskResult=1;$o.RegisteredTask.LastRunTime=[DateTime]::Now.AddMinutes(-1)
 $state=Get-RemovalTaskObservation $o;Check (-not$state.LastRunKnown) 'Stale task run accepted'
 $o.RegisteredTask.LastRunTime=[DateTime]::Now;$o.InstanceGuid=$null
 $state=Get-RemovalTaskObservation $o;Check (-not$state.LastRunKnown) 'Unknown dispatch instance accepted'
}
Case 'terminal-result-requires-known-probe-exit-and-no-owners' {
 $s=Spec;$o=Owner $s (FakeTask $s);Start-RemovalTaskOnce $o
 $worker=WorkerReceipt $s;BindFakeWorker $o $worker
 $state=CompleteFakeTask $o
 $r=Test-RemovalTaskTerminal $o $state $worker
 Check (-not$r.WorkerTerminal-and$r.WorkerNeedsObservation-and-not$r.Terminal) 'Live worker called terminal'
 $script:mockWorker.HasExited=$true
 $r=Test-RemovalTaskTerminal $o $state $worker
 Check ($r.WorkerTerminal-and-not$r.WorkerNeedsObservation-and$r.ReportedProbeExitCode-eq1-and-not$r.NativePassed) 'Failed worker terminal evidence lost'
 Check (-not$r.Terminal-and$r.NeedsObservation-and-not$r.ProbeOwnershipVerified) 'Detached probe evidence promoted to full terminal'
 $worker.Owners=@('live');$r=Test-RemovalTaskTerminal $o $state $worker;Check (-not$r.WorkerTerminal) 'Reported live owner accepted'
 $worker.Owners=@();$worker.ProbeExitKnown=$false;$r=Test-RemovalTaskTerminal $o $state $worker;Check (-not$r.WorkerTerminal) 'Unknown reported probe accepted'
 $worker.ProbeExitKnown=$true;$worker.WorkerExitCode=$true
 $r=Test-RemovalTaskTerminal $o $state $worker;Check (-not$r.WorkerTerminal) 'Boolean worker exit code accepted as integer'
}
Case 'retained-worker-liveness-does-not-require-a-final-json-receipt' {
 $s=Spec;$o=Owner $s (FakeTask $s);Start-RemovalTaskOnce $o;$r=WorkerReceipt $s;BindFakeWorker $o $r
 $live=Observe-RemovalRetainedWorker $o
 Check (-not$live.ExitKnown-and$live.NeedsObservation) 'Live worker treated as exited'
 $script:mockWorker.HasExited=$true;$done=Observe-RemovalRetainedWorker $o
 Check ($done.ExitKnown-and$done.ExitCode-eq1-and-not$done.NeedsObservation-and-not$done.NativePassed) 'Actual worker exit requires detached result'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed)
if($failed){exit 1}
