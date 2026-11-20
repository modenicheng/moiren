#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$no=$false;try{&$Action|Out-Null}catch{$no=$true};Check $no 'Expected refusal.'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
$library=Join-Path $PSScriptRoot 'finish-session.ps1'
Case 'finish-composition-exists-without-opening-native-dispatch' {
 Check (Test-Path -LiteralPath $library) 'Managed finish composition is missing.'
 . $library
 foreach($name in @('New-RemovalManagedFinish','Observe-RemovalManagedFinish','Set-RemovalManagedFinishDecode')){
  Check ($null-ne(Get-Command $name -ErrorAction SilentlyContinue)) ('Missing API: '+$name)
 }
 Reject {Assert-RemovalTaskCleanupApproval}
 Reject {Assert-RemovalTraceDecodeToolApproval}
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_EXECUTED=false';exit 1}
. $library
. (Join-Path $PSScriptRoot 'test-trace-decode-fixtures.ps1')
function Assert-RemovalTaskCleanupApproval {}
function New-FinishFixture([UInt32]$StopStatus=0){
 $f=New-Fixture -StopStatus $StopStatus -LeaveTraceRunning
 $folder=[pscustomobject]@{Task=$f.ManagedSession.TaskOwner.RegisteredTask;Spec=$f.ManagedSession.TaskOwner.Spec
  Removed=$false;DeleteCalls=0;DeleteThrows=$false;EvidenceBeforeDelete=$false;Finish=$null;ResultCollisionAfterDelete=$false}
 $folder|Add-Member ScriptMethod GetTask {
  param($name)
  if($this.Removed){throw [Runtime.InteropServices.COMException]::new('Exact task absent',-2147024894)}
  return $this.Task
 }
 $folder|Add-Member ScriptMethod DeleteTask {
  param($name,$flags)
  $this.DeleteCalls++
  Check ($name-ceq$this.Spec.Name-and$flags-eq0) 'Wrong scheduler target.'
  $finish=$this.Finish
  $this.EvidenceBeforeDelete=($null-ne$finish.ReceiptPin-and(Test-Path -LiteralPath $finish.ReceiptPin.Path))
  Check $this.EvidenceBeforeDelete 'Task deleted before verified finish evidence.'
  $r=[IO.File]::ReadAllText($finish.ReceiptPin.Path)|ConvertFrom-Json
  Check ($r.DecodeReceipt.Sha256-ceq(Get-FileHash -LiteralPath $finish.Decode.ReceiptPath).Hash) 'Decode receipt was not archived before delete.'
  Check (-not$r.NativePassed-and-not$r.Qualified) 'Diagnostic became qualification.'
  Check ($r.Tools.Count-eq$finish.Decode.ToolOwners.Count) 'Missing tool ownership evidence.'
  $this.Removed=$true
  if($this.ResultCollisionAfterDelete){Write-Text (Join-Path $finish.Decode.OutputDirectory 'finish-result.json') 'foreign result'}
  if($this.DeleteThrows){throw 'Delete committed, transport reply unknown.'}
 }
 $f.ManagedSession.TaskOwner.Folder=$folder
 $f.Finish=New-RemovalManagedFinish $f.ManagedSession
 $folder.Finish=$f.Finish
 return $f
}
function Attach-Decode($Fixture,[hashtable]$Boundary){
 Set-RemovalManagedFinishDecode -Finish $Fixture.Finish -Pins $Fixture.Pins `
  -OutputDirectory (Join-Path $Fixture.Root 'decode') -ToolBoundary $Boundary
}
Case 'only-explicit-known-no-dispatch-can-settle-without-a-tool-owner' {
 foreach($mode in @('not-dispatched','unknown-dispatched','missing','nonbool')){
  $step=[pscustomobject]@{Owner=$null}
  switch($mode){
   'not-dispatched'{$step|Add-Member NoteProperty DispatchAttempted $false}
   'unknown-dispatched'{$step|Add-Member NoteProperty DispatchAttempted $true}
   'nonbool'{$step|Add-Member NoteProperty DispatchAttempted 'false'}
  }
  $d=[pscustomobject]@{ToolOwners=@();Steps=@($step)}
  $h=[pscustomobject]@{RetainedDecode=$d;ToolReceipts=@();ToolLogPins=@();Errors=(New-Object Collections.Generic.List[string])}
  $resolved=Observe-RemovalFinishTools $h 1
  Check ($resolved-eq($mode-ceq'not-dispatched')) ('Tool dispatch ambiguity incorrectly settled: '+$mode)
 }
}
Case 'fresh-terminal-then-one-stop-then-retained-decode-then-evidence-then-retirement' {
 $f=New-FinishFixture;$h=$f.Finish
 Check (-not$f.Session.StopAttempted-and$null-eq$h.Decode) 'Construction mutated trace/tools.'
 $r=Observe-RemovalManagedFinish $h
 Check ($r.Phase-ceq'AwaitingDecodePins'-and$f.Session.NativeOwner.TraceStopped-and$f.Calls.Count-eq0) 'STOP/pins ordering wrong.'
 Attach-Decode $f (New-ToolBoundary)
 Check ($null-ne$h.Decode-and-not$h.Decode.Attempted-and$f.Calls.Count-eq0) 'Decode not retained before use.'
 $r=Observe-RemovalManagedFinish $h
 Check ($r.Terminal-and$r.TaskRetired-and-not$r.NeedsObservation-and$f.Calls.Count-eq2) 'Finish not terminal.'
 Check ($f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq1-and$f.ManagedSession.TaskOwner.Folder.EvidenceBeforeDelete) 'Missing exact single retirement.'
 Check (-not$r.NativePassed-and-not$r.Qualified-and$r.AssociationAvailable) 'Finish semantics wrong.'
 Check ($null-ne$h.ResultPin-and$h.ResultPin.Sha256-ceq(Get-FileHash -LiteralPath $h.ResultPin.Path).Hash) 'Terminal retirement result not persisted/pinned.'
 $saved=[IO.File]::ReadAllText($h.ResultPin.Path)|ConvertFrom-Json
 Check ($saved.TaskRetired-and$saved.FinishEvidence.Sha256-ceq$h.ReceiptPin.Sha256-and
  $saved.RetiredTaskXml.Sha256-ceq$f.ManagedSession.TaskOwner.RetiredXmlSha256-and-not$saved.Qualified) 'Final receipt lost retirement XML/evidence or promoted success.'
 $null=Observe-RemovalManagedFinish $h
 Check ($f.Calls.Count-eq2-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq1) 'Terminal observation repeated a mutation.'
}
Case 'cached-terminal-never-stops-trace-for-live-worker-or-probe' {
 foreach($mode in @('worker','probe','task')){
  $f=New-FinishFixture;$f.ManagedSession.Terminal=@{Terminal=$true;NeedsObservation=$false}
  switch($mode){'worker'{$f.WorkerProcess.HasExited=$false};'probe'{$f.ProbeProcess.HasExited=$false};'task'{$f.ManagedSession.TaskOwner.RegisteredTask.State=4}}
  $r=Observe-RemovalManagedFinish $f.Finish
  Check (-not$r.Terminal-and-not$f.Session.StopAttempted-and$f.Calls.Count-eq0) ('Cached flag authorized STOP: '+$mode)
 }
}
Case 'live-worker-can-finish-naturally-without-restarting-managed-session' {
 $f=New-FinishFixture;$f.WorkerProcess.HasExited=$false
 $r=Observe-RemovalManagedFinish $f.Finish
 Check (-not$r.Terminal-and-not$f.Session.StopAttempted) 'Live worker was treated as terminal.'
 $f.WorkerProcess.HasExited=$true
 $r=Observe-RemovalManagedFinish $f.Finish
 Check ($r.Phase-ceq'AwaitingDecodePins'-and$f.Session.NativeOwner.TraceStopped) 'Original worker natural exit did not advance.'
 Attach-Decode $f (New-ToolBoundary);$r=Observe-RemovalManagedFinish $f.Finish
 Check ($r.Terminal-and$f.Calls.Count-eq2) 'Resumed original managed finish failed.'
}
Case 'foreign-task-probe-trace-readback-or-controller-refuses-without-stop' {
 foreach($mode in @('task','probe','trace','readback','controller')){
  $f=New-FinishFixture
  switch($mode){'task'{$f.ManagedSession.TaskOwner=[pscustomobject]@{}};'probe'{$f.ManagedSession.ProbeOwner=[pscustomobject]@{}}
   'trace'{$f.ManagedSession.TraceSession=[pscustomobject]@{}};'readback'{$f.ManagedSession.TraceReadback=[pscustomobject]@{}}
   'controller'{$f.ManagedSession.ControllerCreationUtcTicks++}}
  $r=Observe-RemovalManagedFinish $f.Finish
  Check (-not$r.Terminal-and-not$f.Session.StopAttempted-and$f.Calls.Count-eq0-and$f.Finish.Errors.Count-gt0) ('Foreign original accepted: '+$mode)
 }
}
Case 'pending-stop-retains-owner-and-never-decodes-retires-or-repeats-stop' {
 $f=New-FinishFixture 2402
 $null=Observe-RemovalManagedFinish $f.Finish
 Check ($f.Session.StopAttempted-and-not$f.Session.NativeOwner.TraceStopped) 'Pending STOP lost.'
 Reject {Attach-Decode $f (New-ToolBoundary)}
 $r=Observe-RemovalManagedFinish $f.Finish
 Check (-not$r.Terminal-and$f.Calls.Count-eq0-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq0) 'Pending STOP authorized finish.'
}
Case 'moredata-stop-stays-unknown-even-after-successful-diagnostic-decode' {
 $f=New-FinishFixture 234;$null=Observe-RemovalManagedFinish $f.Finish
 Attach-Decode $f (New-ToolBoundary);$r=Observe-RemovalManagedFinish $f.Finish
 Check ($r.Terminal-and-not$r.RawTraceLossKnown-and-not$r.Qualified) 'MORE_DATA gained raw loss or qualification.'
 $saved=[IO.File]::ReadAllText($f.Finish.ReceiptPin.Path)|ConvertFrom-Json
 Check ($saved.Trace.StopStatus-eq234-and-not$saved.RawTraceLossKnown) 'Persisted receipt invented final loss.'
}
Case 'reader-refusal-retains-diagnostic-reasons-but-safe-terminal-task-can-retire' {
 $f=New-FinishFixture;$f.Mode='reader-refusal';$null=Observe-RemovalManagedFinish $f.Finish
 Attach-Decode $f (New-ToolBoundary);$r=Observe-RemovalManagedFinish $f.Finish
 Check ($r.Terminal-and$r.TaskRetired-and-not$r.AssociationAvailable-and$f.Finish.Decode.Failed) 'Refusal lost or unnecessarily relabelled.'
 $saved=[IO.File]::ReadAllText($f.Finish.ReceiptPin.Path)|ConvertFrom-Json
 Check ($saved.DecodeFailed-and$saved.DecodeErrors.Count-gt0-and-not$saved.NativePassed) 'Refusal evidence lost.'
}
Case 'pending-original-tool-is-observed-after-failed-dispatch-without-relaunch' {
 $f=New-FinishFixture;$null=Observe-RemovalManagedFinish $f.Finish
 $boundary=New-ToolBoundary
 $boundary.Start={param($request)
  Check ([Object]::ReferenceEquals($script:fixture.Finish.Decode,$script:fixture.Finish.RetainedDecode)) 'Decode owner not retained before dispatch.'
  $script:fixture.Calls.Add($request)
  $release=Join-Path $script:fixture.Root 'release-tool'
  $code='while(-not [IO.File]::Exists('+("'"+$release+"'" )+')){Start-Sleep -Milliseconds 20};exit 0'
  $o=Start-FiniteTool -Executable $script:ps -ExpectedSha256 $script:psPin.Sha256 `
   -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-Command',$code) -OutputDirectory $request.OutputDirectory -TimeoutSeconds 15
  $script:fixture.Owners.Add($o);$script:realOwners.Add($o)
  $e=[InvalidOperationException]::new('Dispatch returned unknown after launch');$e.Data['ToolOwner']=$o;throw $e
 }
 Attach-Decode $f $boundary
 $r=Observe-RemovalManagedFinish $f.Finish -ObservationMilliseconds 10
 Check (-not$r.Terminal-and$r.Phase-ceq'WaitingTools'-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq0) 'Pending tool permitted deletion.'
 Check ($f.Finish.Decode.ToolOwners.Count-eq1-and-not$f.Finish.Decode.ToolOwners[0].Closed) 'Exact pending tool lost.'
 $o=$f.Finish.Decode.ToolOwners[0];$o.DeadlineMilliseconds=[long]0
 try{$r=Observe-RemovalManagedFinish $f.Finish -ObservationMilliseconds 10
  Check (-not$r.Terminal-and$f.Calls.Count-eq1-and-not$o.Closed) 'Observation timeout restarted or closed live tool.'
 }finally{Write-Text (Join-Path $f.Root 'release-tool') 'exit naturally'}
 for($i=0;$i-lt30-and-not$r.Terminal;$i++){$r=Observe-RemovalManagedFinish $f.Finish -ObservationMilliseconds 100}
 Check ($r.Terminal-and$o.Closed-and$o.TimedOut-and$f.Calls.Count-eq1) 'Original naturally exited owner not settled without retry.'
 $saved=[IO.File]::ReadAllText($f.Finish.ReceiptPin.Path)|ConvertFrom-Json
 Check ($saved.Tools[0].TimedOut-and$saved.Tools[0].ExitedConfirmed-and$saved.Tools[0].Closed-and$saved.DecodeFailed) 'Late exit erased failed/timeout provenance.'
}
Case 'missing-decode-receipt-does-not-authorize-retirement' {
 $f=New-FinishFixture;$null=Observe-RemovalManagedFinish $f.Finish
 Attach-Decode $f (New-ToolBoundary)
 $null=Invoke-RemovalTraceDecode $f.Finish.Decode
 [IO.File]::Delete($f.Finish.Decode.ReceiptPath)
 $r=Observe-RemovalManagedFinish $f.Finish
 Check (-not$r.Terminal-and$f.Calls.Count-eq2-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq0) 'Missing receipt authorized delete or repeated decode.'
}
Case 'same-run-replacement-decode-receipt-cannot-authorize-retirement' {
 $f=New-FinishFixture;$null=Observe-RemovalManagedFinish $f.Finish;Attach-Decode $f (New-ToolBoundary)
 $null=Invoke-RemovalTraceDecode $f.Finish.Decode
 $path=$f.Finish.Decode.ReceiptPath;$saved=[IO.File]::ReadAllText($path)|ConvertFrom-Json
 $saved.OutputPins=@();$saved.Completed=$false;$saved.Failed=$false
 Write-Text $path ($saved|ConvertTo-Json -Depth 16)
 $r=Observe-RemovalManagedFinish $f.Finish
 Check (-not$r.Terminal-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq0) 'Equal-run replacement receipt acquired destructive authority.'
}
Case 'original-managed-full-argv-is-revalidated-after-decode-before-retirement' {
 $f=New-FinishFixture;$null=Observe-RemovalManagedFinish $f.Finish;Attach-Decode $f (New-ToolBoundary)
 $null=Invoke-RemovalTraceDecode $f.Finish.Decode
 $f.ManagedSession.Config.ProbeArguments[-1]='9'
 $f.ManagedSession.ProbeOwner.Spec.Arguments[-1]='9'
 $r=Observe-RemovalManagedFinish $f.Finish
 Check (-not$r.Terminal-and$f.Calls.Count-eq2-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq0) 'Changed full original argv acquired retirement authority.'
}
Case 'changed-original-inputs-after-decode-prevent-retirement' {
 foreach($mode in @('report-change','etl-missing','tmf-missing')){
  $f=New-FinishFixture;$null=Observe-RemovalManagedFinish $f.Finish;Attach-Decode $f (New-ToolBoundary)
  $null=Invoke-RemovalTraceDecode $f.Finish.Decode
  switch($mode){'report-change'{[IO.File]::AppendAllText($f.Pins.ProbeReport.Path,' changed after decode')}
   'etl-missing'{[IO.File]::Delete($f.Pins.Etl.Path)};'tmf-missing'{[IO.File]::Delete($f.Pins.Symbols.Files[0].Path)}}
  $r=Observe-RemovalManagedFinish $f.Finish
  Check (-not$r.Terminal-and$f.Calls.Count-eq2-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq0) ('Changed input acquired retirement authority: '+$mode)
 }
}
Case 'failed-tool-exit-retains-failure-receipt-and-still-retires-safe-task' {
 $f=New-FinishFixture;$f.Mode='trace-fail';$null=Observe-RemovalManagedFinish $f.Finish
 Attach-Decode $f (New-ToolBoundary);$r=Observe-RemovalManagedFinish $f.Finish
 Check ($r.Terminal-and$f.Calls.Count-eq1-and$f.Finish.Decode.Failed-and-not$r.AssociationAvailable) 'Failed tool was retried or cannot retire a safe task.'
 $saved=[IO.File]::ReadAllText($f.Finish.ReceiptPin.Path)|ConvertFrom-Json
 Check ($saved.Tools[0].ExitCode-eq3-and$saved.Tools[0].Closed-and$saved.DecodeFailed) 'Failure exit lost from persisted evidence.'
}
Case 'default-decoder-guard-does-not-retire-or-start-any-tool' {
 $f=New-FinishFixture;$null=Observe-RemovalManagedFinish $f.Finish
 Attach-Decode $f $null;$r=Observe-RemovalManagedFinish $f.Finish
 Check (-not$r.Terminal-and-not$f.Finish.Decode.Attempted-and$f.Finish.Decode.ToolOwners.Count-eq0-and
  $f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq0) 'Default guard opened dispatch/deletion.'
}
Case 'existing-finish-evidence-is-never-overwritten-or-adopted' {
 $f=New-FinishFixture;$null=Observe-RemovalManagedFinish $f.Finish;Attach-Decode $f (New-ToolBoundary)
 $path=Join-Path $f.Root 'decode\finish-evidence.json';$null=[IO.Directory]::CreateDirectory((Split-Path $path))
 Write-Text $path 'foreign evidence'
 $r=Observe-RemovalManagedFinish $f.Finish
 Check (-not$r.Terminal-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq0-and[IO.File]::ReadAllText($path)-ceq'foreign evidence') 'Foreign evidence overwritten or adopted.'
}
Case 'existing-final-result-refuses-before-deletion-without-overwrite' {
 $f=New-FinishFixture;$null=Observe-RemovalManagedFinish $f.Finish;Attach-Decode $f (New-ToolBoundary)
 $null=Invoke-RemovalTraceDecode $f.Finish.Decode
 $path=Join-Path $f.Root 'decode\finish-result.json';Write-Text $path 'foreign result'
 $r=Observe-RemovalManagedFinish $f.Finish
 Check (-not$r.Terminal-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq0-and[IO.File]::ReadAllText($path)-ceq'foreign result') 'Existing final result overwritten or deletion occurred.'
}
Case 'failed-post-delete-result-preserves-retired-owner-without-delete-retry' {
 $f=New-FinishFixture;$f.ManagedSession.TaskOwner.Folder.ResultCollisionAfterDelete=$true
 $null=Observe-RemovalManagedFinish $f.Finish;Attach-Decode $f (New-ToolBoundary)
 $r=Observe-RemovalManagedFinish $f.Finish;$r=Observe-RemovalManagedFinish $f.Finish
 Check (-not$r.Terminal-and$r.TaskRetired-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq1-and$f.Calls.Count-eq2) 'Final persistence failure lost retired owner or retried deletion.'
 Check ([IO.File]::ReadAllText((Join-Path $f.Root 'decode\finish-result.json'))-ceq'foreign result') 'Final result collision overwritten.'
}
Case 'ambiguous-task-retirement-is-never-retried' {
 $f=New-FinishFixture;$f.ManagedSession.TaskOwner.Folder.DeleteThrows=$true
 $null=Observe-RemovalManagedFinish $f.Finish;Attach-Decode $f (New-ToolBoundary)
 $r=Observe-RemovalManagedFinish $f.Finish;$r=Observe-RemovalManagedFinish $f.Finish
 Check (-not$r.Terminal-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq1-and$f.Calls.Count-eq2) 'Ambiguous retirement retried or promoted.'
}
foreach($o in $script:realOwners){
 if(-not$o.Closed){$r=Observe-FiniteTool $o -ObservationMilliseconds 15000;Check ($r.ExitedConfirmed-and-not$r.NeedsObservation) 'Harmless child unresolved: retained, not killed.';Close-FiniteToolOwner $o}
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' SCHEDULER=mock ETW=mock CHILDREN=harmless-powershell-and-offline-reader')
if($failed){exit 1}
