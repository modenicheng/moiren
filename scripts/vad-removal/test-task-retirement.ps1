#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'collector-boundary.ps1')
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$no=$false;try{&$Action|Out-Null}catch{$no=$true};Check $no 'Expected refusal'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
Case 'managed-retirement-entry-exists-with-native-cleanup-guard' {
 Check ($null-ne(Get-Command Complete-RemovalManagedTaskCleanup -ErrorAction SilentlyContinue)) 'Missing managed retirement composition'
 Reject {Assert-RemovalTaskCleanupApproval}
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 SCHEDULER=mock NATIVE_EXECUTED=false';exit 1}
$actualCleanup=${function:Complete-RemovalManagedTaskCleanup}
$root=Join-Path $PSScriptRoot ('task-fixtures\retirement-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
function Assert-RemovalTaskHost {}
function Assert-RemovalTaskCleanupApproval {}
function Fixture {
 $run=Join-Path $root ([Guid]::NewGuid().ToString('N'));$null=[IO.Directory]::CreateDirectory($run)
 $worker=Join-Path $run 'worker.ps1';[IO.File]::WriteAllText($worker,'# never executed')
 $spec=New-RemovalTaskSpec -WorkerPath $worker -WorkerSha256 (Get-FileHash -LiteralPath $worker).Hash `
  -RunDirectory $run -SessionId 2 -Nonce ([Guid]::NewGuid().ToString('N'))
 $task=[pscustomobject]@{Path=$spec.Path;Name=$spec.Name;Enabled=$true;State=3;Instances=0;LastTaskResult=1
  LastRunTime=[DateTime]::Now;Xml='<exact-created-task/>'
  Definition=[pscustomobject]@{Principal=[pscustomobject]@{UserId=$spec.Sid;LogonType=3;RunLevel=0}
   Triggers=[pscustomobject]@{Count=0};Actions=[pscustomobject]@{Count=1
    Action=[pscustomobject]@{Type=0;Path=$spec.Executable;Arguments=$spec.Arguments;WorkingDirectory=$run}}
   Settings=[pscustomobject]@{AllowDemandStart=$true;Enabled=$true;MultipleInstances=2;RestartCount=0
    AllowHardTerminate=$false;ExecutionTimeLimit='PT0S';StopIfGoingOnBatteries=$false;DisallowStartIfOnBatteries=$false}}}
 $task.Definition.Actions|Add-Member ScriptMethod Item {param($i)if($i-ne1){throw 'Unexpected action index'};return $this.Action}
 $task|Add-Member ScriptMethod GetInstances {param($flags)return [pscustomobject]@{Count=$this.Instances}}
 $o=New-RemovalTaskOwner -Spec $spec -RegisteredTask $task
 $o.DispatchAttempted=$true;$o.InstanceGuid='{d5d7acdf-e49d-4134-ac20-3d965accb18c}'
 $o.DispatchUtc=[DateTimeOffset]::UtcNow.AddSeconds(-1).ToString('o')
 $o.WorkerBindingKnown=$true;$o.WorkerPid=5678;$o.WorkerCreationUtcTicks=[DateTime]::UtcNow.AddSeconds(-1).Ticks
 $p=[pscustomobject]@{HasExited=$true;ExitCode=1};$p|Add-Member ScriptMethod Refresh {}
 $o.WorkerProcess=$p
 $folder=[pscustomobject]@{Task=$task;Removed=$false;DeleteCalls=0;GetCalls=0;DeleteThrows=$false;ReadbackError=0
  Spec=$spec;ArchivedBeforeDelete=$false}
 $folder|Add-Member ScriptMethod DeleteTask {
  param($name,$flags)
  $this.DeleteCalls++
  $path=Join-Path $this.Spec.RunDirectory 'retired-task.xml'
  $this.ArchivedBeforeDelete=(Test-Path -LiteralPath $path)-and([IO.File]::ReadAllText($path)-ceq$this.Task.Xml)
  if(-not$this.ArchivedBeforeDelete-or$name-cne$this.Spec.Name-or$flags-ne0){throw 'Missing verified exact XML or wrong delete target'}
  $this.Removed=$true
  if($this.DeleteThrows){throw 'Delete may have committed'}
 }
 $folder|Add-Member ScriptMethod GetTask {
  param($name)
  $this.GetCalls++
  if($this.Removed-and$this.ReadbackError-ne0){throw [Runtime.InteropServices.COMException]::new('Injected readback failure',[int]$this.ReadbackError)}
  if($this.Removed){throw [Runtime.InteropServices.COMException]::new('Task absent',-2147024894)}
  return $this.Task
 }
 $o.Folder=$folder
 $wr=@{Nonce=$spec.Nonce;TaskPath=$spec.Path;Host='LAPTOP-NP49RG6H';Sid=$spec.Sid;SessionId=2
  IsAdministrator=$false;Pid=$o.WorkerPid;CreationUtcTicks=$o.WorkerCreationUtcTicks;WorkerSha256=$spec.WorkerSha256
  ProbeExitKnown=$true;ProbeExitCode=1;Owners=@();WorkerExitCode=1;OutputStable=$true
  ProbePid=6789;ProbeCreationUtcTicks=$o.WorkerCreationUtcTicks+1}
 $probe=[pscustomobject]@{BindingKnown=$true;BindingError=$null;ProcessId=6789;CreationUtcTicks=$wr.ProbeCreationUtcTicks
  Spec=[pscustomobject]@{Nonce=$spec.Nonce;RunDirectory=$run;SessionId=2;Sid=$spec.Sid
   WorkerPid=$o.WorkerPid;WorkerCreationUtcTicks=$o.WorkerCreationUtcTicks}}
 $stop=[pscustomobject]@{Operation='Stop';Owner=$null;StoppedConfirmed=$true}
 $native=[pscustomobject]@{TraceOwned=$true;TraceStopped=$true;StopReceipt=$stop};$stop.Owner=$native
 $readback=[pscustomobject]@{Trace=$native}
 $trace=[pscustomobject]@{NativeOwner=$native;Readback=$readback;StoppedReceipt=$stop;StopAttempted=$true}
 [pscustomobject]@{TaskOwner=$o;ProbeOwner=$probe;LastResult=$wr;TraceSession=$trace;TraceReadback=$readback;Terminal=@{Terminal=$true}}
}
# Keep task definition, current scheduler observation and terminal composition
# real. Only the external scheduler and probe-process observation are fixtures.
function Observe-RemovalProbe {param($Owner)[pscustomobject]@{ExitKnown=$true;ExitCode=1;TimedOut=$false;NeedsObservation=$false}}
Case 'terminal-error-exit-retires-only-exact-task-after-verified-xml' {
 $s=Fixture;&$actualCleanup $s
 Check ($s.TaskOwner.Retired-and$s.TaskOwner.RetirementAttempted-and$s.TaskOwner.Folder.DeleteCalls-eq1) 'Exact task not retired once'
 Check ($s.TaskOwner.Folder.ArchivedBeforeDelete-and(Test-Path -LiteralPath $s.TaskOwner.RetiredXmlPath)) 'XML not retained before deletion'
 Check ($s.TaskOwner.RetiredXmlSha256-ceq(Get-FileHash -LiteralPath $s.TaskOwner.RetiredXmlPath).Hash) 'Retired XML hash differs'
 Check (-not$s.Terminal.NativePassed-and-not$s.Terminal.Qualified) 'Error exit cleanup became native success'
 Reject {&$actualCleanup $s};Check ($s.TaskOwner.Folder.DeleteCalls-eq1) 'Retirement repeated'
}
Case 'cached-terminal-flag-does-not-authorize-live-worker-or-probe-cleanup' {
 foreach($mode in @('worker','probe','unrun','result')){
  $s=Fixture
  switch($mode){'worker'{$s.TaskOwner.WorkerProcess.HasExited=$false}
   'probe'{$s.LastResult.ProbePid++}
   'unrun'{$s.TaskOwner.InstanceGuid=$null}
   'result'{$s.LastResult.Owners=@('pending')}}
  Reject {&$actualCleanup $s}
  Check ($s.TaskOwner.Folder.DeleteCalls-eq0-and-not$s.TaskOwner.RetirementAttempted) ('Unresolved owner deleted: '+$mode)
 }
}
Case 'pending-or-detached-trace-stop-cannot-authorize-task-retirement' {
 foreach($mode in @('live','clone','owner','unattempted')){
  $s=Fixture
  switch($mode){'live'{$s.TraceSession.NativeOwner.TraceStopped=$false}
   'clone'{$s.TraceSession.StoppedReceipt=[pscustomobject]@{Operation='Stop';Owner=$s.TraceSession.NativeOwner;StoppedConfirmed=$true}}
   'owner'{$s.TraceSession.NativeOwner.StopReceipt.Owner=[pscustomobject]@{}}
   'unattempted'{$s.TraceSession.StopAttempted=$false}}
  Reject {&$actualCleanup $s};Check ($s.TaskOwner.Folder.DeleteCalls-eq0) ('Unresolved trace cleanup: '+$mode)
 }
}
Case 'another-stopped-trace-session-cannot-replace-this-runs-retained-trace' {
 $s=Fixture;$original=$s.TraceReadback;$original.Trace.TraceStopped=$false
 $other=Fixture;$s.TraceSession=$other.TraceSession
 Reject {&$actualCleanup $s}
 Check ($s.TaskOwner.Folder.DeleteCalls-eq0-and-not$s.TaskOwner.RetirementAttempted) 'Another genuinely stopped trace authorized this run cleanup'
 $s=Fixture;$s.TraceSession.Readback.Trace=[pscustomobject]@{TraceStopped=$true}
 Reject {&$actualCleanup $s}
 Check ($s.TaskOwner.Folder.DeleteCalls-eq0) 'Readback for another owner authorized retirement'
}
Case 'changed-task-xml-action-or-active-instance-refuses-without-delete' {
 foreach($mode in @('xml','action','instance')){
  $s=Fixture
  switch($mode){'xml'{$s.TaskOwner.RegisteredTask.Xml='<changed/>'}
   'action'{$s.TaskOwner.RegisteredTask.Definition.Actions.Action.Arguments+=' wrong'}
   'instance'{$s.TaskOwner.RegisteredTask.Instances=1}}
  Reject {&$actualCleanup $s};Check ($s.TaskOwner.Folder.DeleteCalls-eq0) ('Changed or active task deleted: '+$mode)
 }
}
Case 'archive-collision-refuses-without-overwrite-or-delete' {
 $s=Fixture;$path=Join-Path $s.TaskOwner.Spec.RunDirectory 'retired-task.xml';[IO.File]::WriteAllText($path,'user-owned')
 Reject {&$actualCleanup $s}
 Check ($s.TaskOwner.Folder.DeleteCalls-eq0-and[IO.File]::ReadAllText($path)-ceq'user-owned') 'Existing evidence replaced or task deleted'
}
Case 'ambiguous-delete-retains-original-owner-and-never-retries' {
 $s=Fixture;$s.TaskOwner.Folder.DeleteThrows=$true
 Reject {&$actualCleanup $s}
 Check ($s.TaskOwner.RetirementAttempted-and-not$s.TaskOwner.Retired-and$s.TaskOwner.RetirementError) 'Ambiguous deletion forgotten'
 Reject {&$actualCleanup $s};Check ($s.TaskOwner.Folder.DeleteCalls-eq1) 'Unknown delete retried'
}
Case 'readback-access-error-is-not-task-absence' {
 $s=Fixture;$s.TaskOwner.Folder.ReadbackError=-2147024891
 Reject {&$actualCleanup $s}
 Check ($s.TaskOwner.RetirementAttempted-and-not$s.TaskOwner.Retired-and$s.TaskOwner.Folder.DeleteCalls-eq1) 'Access denied became verified retirement'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' SCHEDULER=mock NATIVE_EXECUTED=false')
if($failed){exit 1}
