#requires -Version 5.1
# No native dispatch: real trace identity/lease, synthetic readiness and APIs.
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){throw 'Host-only native admission tests must not run on target.'}
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$refused=$false;try{&$Action|Out-Null}catch{$refused=$true};Check $refused 'Expected operation refusal.'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message;at=$_.ScriptStackTrace})}}
$library=Join-Path $PSScriptRoot 'native-admission.ps1'
Case 'retained-operation-admission-api-exists' {
 Check (Test-Path -LiteralPath $library) 'Owner-aware native admission implementation missing.'
 . $library
 foreach($name in @('Initialize-RemovalNativeContext','Register-RemovalNativeDecode','Assert-RemovalNativeOperation')){
  Check ($null-ne(Get-Command $name -ErrorAction SilentlyContinue)) ('Missing '+$name)
 }
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_CALLS=0';exit 1}
. (Join-Path $PSScriptRoot 'diagnostic-controller.ps1')
. $library
Initialize-RemovalTraceNative
Case 'executable-release-remains-closed-without-reviewed-target-and-archive-runner' {
 Check ($null-ne(Get-Command Assert-RemovalDiagnosticRelease -ErrorAction SilentlyContinue)) 'Separate executable release guard missing.'
 Reject {Assert-RemovalDiagnosticRelease}
}
$root=Join-Path $PSScriptRoot ('logs\native-admission-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$script:FreshAdmissionCalls=0
function Assert-RemovalTaskHost {}
function Assert-RemovalDiagnosticRelease {} # TEST ONLY; every API remains synthetic.
# TEST ONLY elapsed seam for this preexisting DTO resource unit fixture.
# Real constructor timestamp/CWT checks are retained in test-diagnostic-release.
function Test-RemovalControllerDeadlineExpired($Owner){return ($Owner.Clock.ElapsedMilliseconds-ge$Owner.DeadlineMilliseconds)}
function Assert-RemovalDiagnosticAdmission($Owner){
 $script:FreshAdmissionCalls++
 if($Owner.Errors.Count-or$Owner.TimedOut-or$Owner.Completed-or-not$Owner.ReadyFixture){throw 'Fresh admission refused.'}
 Assert-RemovalDiagnosticLease $Owner.RetainedLease
}
function Assert-RemovalRunInputs($Owner){if($Owner.Changed){throw 'Input pins changed.'}}
function Test-RemovalTraceCaptureAdmission($Readback){return $Readback.Accepted}
function Assert-RemovalFinishIdentity($Finish){Check $Finish.IdentityFixture 'Original finish changed.'}
function Assert-RemovalFinishManagedProvenance($Finish){Check $Finish.IdentityFixture 'Original finish provenance changed.'}
function Get-RemovalFinishTerminal($Finish){if($Finish.TerminalFixture){return @{Terminal=$true}};return $null}
function Assert-RemovalDecodeStoppedSession($Session){
 Check ([Object]::ReferenceEquals($Session.StoppedReceipt,$Session.NativeOwner.StopReceipt)-and$Session.NativeOwner.TraceStopped) 'Original STOP required.'
 return $Session.StoppedReceipt
}
function Assert-RemovalDecodeInputs($Decode){if($Decode.ChangedFixture){throw 'Decode pins changed.'}}
function Assert-RemovalDecodeManagedReport($Decode){return @{fixture='original'}}
function New-Fixture {
 $dir=Join-Path $root ([Guid]::NewGuid().ToString('N'));$null=[IO.Directory]::CreateDirectory($dir)
 $self=[Diagnostics.Process]::GetCurrentProcess();$nonce=[Guid]::NewGuid().ToString('N')
 $lease=New-RemovalDiagnosticLease -Path (Join-Path $dir 'writer.lock');Acquire-RemovalDiagnosticLease $lease
 $native=New-RemovalNativeTrace -Session ('MoirenRemoval613'+$nonce) -EtlPath (Join-Path $dir 'driver.etl')
 $readback=[pscustomobject]@{Trace=$native;Accepted=$true}
 $trace=[pscustomobject]@{NativeOwner=$native;Readback=$readback;QueryBoundary=$null;StartAttempted=$false;StopAttempted=$false;StoppedReceipt=$null}
 $managedDir=Join-Path $dir 'managed'
 $probe=[pscustomobject]@{Path=(Join-Path $dir 'e2e_latency_probe.exe');Sha256=('B'*64);Bytes=1}
 $task=[pscustomobject]@{Spec=[pscustomobject]@{RunDirectory=$managedDir;Nonce=$nonce;Path=('\Moiren-Removal613-'+$nonce);Sid='S-1-5-21-3080296863-1242338503-1346362208-1001';SessionId=7}
  RegistrationAttempted=$false;RegistrationKnown=$false;DispatchAttempted=$false;RetirementAttempted=$false;Retired=$false;Error=$null;Closed=$false
  Scheduler=$null;Folder=$null;TaskDefinition=$null;RegisteredTask=$null;RegisteredXml=$null}
 $args=@('live-vad','--artifact-dir',(Join-Path $managedDir 'artifacts'),'--seconds','8')
 $config=[pscustomobject]@{SchemaVersion=1;Nonce=$nonce;RunDirectory=$managedDir;SessionId=7;Sid=$task.Spec.Sid
  ControllerPid=$self.Id;ControllerCreationUtcTicks=$self.StartTime.ToUniversalTime().Ticks
  Executable=$probe.Path;ExpectedSha256=$probe.Sha256;ProbeArguments=$args}
 $managed=[pscustomobject]@{RunDirectory=$managedDir;Config=$config;ControllerPid=$self.Id;ControllerCreationUtcTicks=$self.StartTime.ToUniversalTime().Ticks
  TaskOwner=$task;ProbeOwner=$null;TraceSession=$trace;TraceReadback=$readback;Error=$null;LastResult=$null}
 $inputs=[pscustomobject]@{Changed=$false;Pins=@{Probe=@{File=$probe};DriverPackage=@{Path='C:\pins\package.json'}
  Tracefmt=@{Path='C:\tools\tracefmt.exe';Sha256=('C'*64)};Python=@{Path='C:\tools\python.exe';Sha256=('D'*64)}
  Reader=@{Path='C:\tools\vad-removal-stages.py'};Symbols=@{Directory='C:\symbols'}}}
 $before=[pscustomobject]@{RetainedPreflight=@{ExecutionMode='RepositoryCollector'};RetainedUserSession=@{Snapshot=@{SessionId=7;Sid=$task.Spec.Sid}}}
 $o=[pscustomobject]@{RunDirectory=$dir;OriginalDirectory=$dir;Nonce=$nonce;ControllerPid=$self.Id;ControllerCreationUtcTicks=$self.StartTime.ToUniversalTime().Ticks
  ConfigJson='original-controller';StartAttempted=$true;ReadyFixture=$true;Errors=(New-Object Collections.Generic.List[string]);TimedOut=$false;Completed=$false
  Clock=[Diagnostics.Stopwatch]::StartNew();DeadlineMilliseconds=[long]240000
  RetainedLease=$lease;RetainedInputs=$inputs;RetainedBefore=$before;RetainedTrace=$trace;OriginalNativeOwner=$native;OriginalReadback=$readback
  RetainedManaged=$managed;RetainedFinish=$null;AbortEvidencePin=$null}
 return $o
}
function Close-Fixture($Owner){Release-RemovalDiagnosticLease $Owner.RetainedLease}
function Seal-Task($Owner){
 $t=$Owner.RetainedManaged.TaskOwner
 $t.Scheduler=[pscustomobject]@{fixture='scheduler'};$t.Folder=[pscustomobject]@{fixture='folder'};$t.TaskDefinition=[pscustomobject]@{fixture='definition'}
 Assert-RemovalNativeOperation -Resource $t -Operation TaskRegisterDispatch
 $t.RegistrationAttempted=$true
 $t.RegisteredTask=[pscustomobject]@{fixture='returned-task';Xml='<original/>'};$t.RegisteredXml=$t.RegisteredTask.Xml
 Assert-RemovalNativeOperation -Resource $t -Operation TaskRegistered
 $t.RegistrationKnown=$true
}
function Make-Owned($Owner,[switch]$FailedEnable){
 $n=$Owner.OriginalNativeOwner;$bridge=$null
 Check ($script:RemovalNativeBridges.TryGetValue($n,[ref]$bridge)) 'Actual native bridge missing.'
 $bridge.BeginStart();$bridge.CaptureStart([uint32]0,[uint64]701);$bridge.BeginIdle();$bridge.CaptureIdle([uint32]0,$true)
 $bridge.BeginEnable();$bridge.CaptureEnable([uint32]$(if($FailedEnable){5}else{0}))
 if($FailedEnable){$n.Fail('synthetic enable failure')}
 $Owner.RetainedTrace.StartAttempted=$true
 return $bridge
}
function Make-Stopped($Owner){
 $n=$Owner.OriginalNativeOwner;$b=$null
 if($n.TraceOwned){$null=$script:RemovalNativeBridges.TryGetValue($n,[ref]$b)}else{$b=Make-Owned $Owner}
 $v=New-Object Moiren.RemovalTraceQueryV1.Session
 $v.SessionName=$n.Session;$v.LogFileName=$n.EtlPath;$v.SessionGuid=$n.SessionGuid;$v.Handle=$n.Handle
 $v.ClientContext=1;$v.LogFileMode=1;$v.MaximumFileSize=16
 $b.BeginStopQuery();$b.CaptureStopQuery([uint32]0);$b.CaptureStopQuerySnapshot($v);$b.BeginStop();$b.CaptureStop([uint32]0);$b.CaptureFinal($v)
 $Owner.RetainedTrace.StopAttempted=$true;$Owner.RetainedTrace.StoppedReceipt=$n.StopReceipt
}
function Bind-Decode($Owner,[switch]$AlternateTool){
 Make-Stopped $Owner
 $d=[pscustomobject]@{TraceSession=$Owner.RetainedTrace;ManagedSession=$Owner.RetainedManaged;StopReceipt=$Owner.OriginalNativeOwner.StopReceipt
  OutputDirectory=(Join-Path $Owner.OriginalDirectory 'decode');TracePath=(Join-Path $Owner.OriginalDirectory 'decode\driver.txt')
  SummaryPath=(Join-Path $Owner.OriginalDirectory 'decode\driver.txt.sum');AnalysisPath=(Join-Path $Owner.OriginalDirectory 'decode\association.json')
  Pins=@{Tracefmt=@{Path='C:\tools\tracefmt.exe';Sha256=('C'*64)};Python=@{Path='C:\tools\python.exe';Sha256=('D'*64)}
   Reader=@{Path='C:\tools\vad-removal-stages.py'};Etl=@{Path=$Owner.OriginalNativeOwner.EtlPath};Symbols=@{Directory='C:\symbols'}
   DriverPackage=@{Path='C:\pins\package.json'};Probe=@{File=$Owner.RetainedInputs.Pins.Probe.File}}
  Attempted=$false;Completed=$false;Failed=$false;ChangedFixture=$false
  Steps=(New-Object Collections.Generic.List[object])}
 $f=[pscustomobject]@{Session=$Owner.RetainedManaged;RetainedDecode=$d;Decode=$d;IdentityFixture=$true;TerminalFixture=$true}
 if($AlternateTool){$d.Pins.Tracefmt.Path='C:\foreign\tracefmt.exe'}
 $Owner.RetainedFinish=$f;Register-RemovalNativeDecode $Owner
 return $d
}
Case 'unbound-owner-and-detached-approval-booleans-refuse' {
 $o=New-Fixture
 try{Reject {Assert-RemovalNativeOperation -Resource $o.OriginalNativeOwner -Operation TraceStart}
  Reject {Assert-RemovalNativeOperation -Resource ([pscustomobject]@{NativeApproved=$true;Reviewed=$true}) -Operation TraceStart}
 }finally{Close-Fixture $o}
}
Case 'binding-requires-fresh-original-staged-resource-and-is-one-shot' {
 $o=New-Fixture
 try{$o.ReadyFixture=$false;Reject {Initialize-RemovalNativeContext $o};$o.ReadyFixture=$true
  Initialize-RemovalNativeContext $o;Reject {Initialize-RemovalNativeContext $o}
  Assert-RemovalNativeOperation -Resource $o.OriginalNativeOwner -Operation TraceStart
  Reject {Assert-RemovalNativeOperation -Resource $o.OriginalNativeOwner -Operation TraceStart}
 }finally{Close-Fixture $o}
}
Case 'each-birth-rechecks-deadline-errors-live-writer-and-resource-config' {
 foreach($mode in @('deadline','error','stale','pins','task-config','closed-lease')){
  $o=New-Fixture
  try{Initialize-RemovalNativeContext $o
   switch($mode){
    'deadline'{$o.DeadlineMilliseconds=0}
    'error'{$o.Errors.Add('latched refusal')}
    'stale'{$o.ReadyFixture=$false}
    'pins'{$o.RetainedInputs.Changed=$true}
    'task-config'{$o.RetainedManaged.TaskOwner.Spec.Nonce='f'*32}
    'closed-lease'{Close-Fixture $o}
   }
   Reject {Assert-RemovalNativeOperation -Resource $o.OriginalNativeOwner -Operation TraceStart}
  }finally{Close-Fixture $o}
 }
}
Case 'expiry-during-source-validation-cannot-escape-final-birth-check' {
 $o=New-Fixture
 try{Initialize-RemovalNativeContext $o
  function Assert-RemovalRunInputs($Inputs){$o.TimedOut=$true}
  Reject {Assert-RemovalNativeOperation -Resource $o.OriginalNativeOwner -Operation TraceStart}
 }finally{Close-Fixture $o}
}
Case 'freshness-uses-original-typed-snapshot-not-json-date-coercion' {
 $o=New-Fixture
 try{
  $observed=[DateTimeOffset]::UtcNow.ToString('o')
  $p=$o.RetainedBefore.RetainedPreflight
  $p|Add-Member NoteProperty SnapshotJson (ConvertTo-Json -InputObject @{ObservedUtc=$observed} -Compress)
  $p|Add-Member NoteProperty RetainedOwner ([pscustomobject]@{Shared=@{Snapshot=@{ObservedUtc=$observed}}})
  Initialize-RemovalNativeContext $o
  Assert-RemovalNativeOperation -Resource $o.OriginalNativeOwner -Operation TraceStart
 }finally{Close-Fixture $o}
}
Case 'trace-cleanup-survives-expired-readiness-input-mutation-and-root-refusal' {
 $o=New-Fixture
 try{Initialize-RemovalNativeContext $o;$b=Make-Owned $o -FailedEnable
  $o.ReadyFixture=$false;$o.RetainedInputs.Changed=$true;$o.Errors.Add('expired root');$o.TimedOut=$true;$o.DeadlineMilliseconds=0
  $count=$script:FreshAdmissionCalls
  Assert-RemovalNativeOperation -Resource $o.OriginalNativeOwner -Operation TraceStopPrepare
  $b.BeginStopQuery();Assert-RemovalNativeOperation -Resource $o.OriginalNativeOwner -Operation TraceStopQuery
  Reject {Assert-RemovalNativeOperation -Resource $o.OriginalNativeOwner -Operation TraceStopPrepare}
  Check ($script:FreshAdmissionCalls-eq$count) 'Cleanup incorrectly required renewed birth admission.'
 }finally{Close-Fixture $o}
}
Case 'foreign-trace-unknown-start-or-released-original-writer-cannot-stop' {
 $o=New-Fixture
 try{Initialize-RemovalNativeContext $o
  Reject {Assert-RemovalNativeOperation -Resource $o.OriginalNativeOwner -Operation TraceStopPrepare}
  $null=Make-Owned $o
  $foreign=New-RemovalNativeTrace -Session 'MoirenRemoval613foreign' -EtlPath (Join-Path $o.OriginalDirectory 'foreign.etl')
  Reject {Assert-RemovalNativeOperation -Resource $foreign -Operation TraceStopPrepare}
  Close-Fixture $o;Reject {Assert-RemovalNativeOperation -Resource $o.OriginalNativeOwner -Operation TraceStopPrepare}
 }finally{Close-Fixture $o}
}
Case 'task-register-and-dispatch-require-owned-admitted-trace-and-are-one-shot' {
 $o=New-Fixture
 try{Initialize-RemovalNativeContext $o;$t=$o.RetainedManaged.TaskOwner
  Reject {Assert-RemovalNativeOperation -Resource $t -Operation TaskRegister}
  $null=Make-Owned $o;Assert-RemovalNativeOperation -Resource $t -Operation TaskRegister
  Reject {Assert-RemovalNativeOperation -Resource $t -Operation TaskRegister}
  Seal-Task $o
  Assert-RemovalNativeOperation -Resource $t -Operation TaskDispatch
  Reject {Assert-RemovalNativeOperation -Resource $t -Operation TaskDispatch}
 }finally{Close-Fixture $o}
}
Case 'registration-completion-seals-original-com-objects-without-renewing-birth' {
 $o=New-Fixture
 try{Initialize-RemovalNativeContext $o;$null=Make-Owned $o;$t=$o.RetainedManaged.TaskOwner
  Assert-RemovalNativeOperation -Resource $t -Operation TaskRegister
  $t.Scheduler=[pscustomobject]@{fixture='scheduler'};$t.Folder=[pscustomobject]@{fixture='folder'};$t.TaskDefinition=[pscustomobject]@{fixture='definition'}
  Assert-RemovalNativeOperation -Resource $t -Operation TaskRegisterDispatch
  $t.RegistrationAttempted=$true
  $o.ReadyFixture=$false
  $t.RegisteredTask=[pscustomobject]@{fixture='returned-task';Xml='<original/>'};$t.RegisteredXml=$t.RegisteredTask.Xml
  Assert-RemovalNativeOperation -Resource $t -Operation TaskRegistered;$t.RegistrationKnown=$true
  $o.ReadyFixture=$true;$t.Folder=[pscustomobject]@{fixture='foreign-folder'}
  Reject {Assert-RemovalNativeOperation -Resource $t -Operation TaskDispatch}
 }finally{Close-Fixture $o}
}
Case 'task-retirement-cannot-bypass-whole-terminal-or-original-evidence' {
 $o=New-Fixture
 try{Initialize-RemovalNativeContext $o;$null=Make-Owned $o;$t=$o.RetainedManaged.TaskOwner
  Assert-RemovalNativeOperation -Resource $t -Operation TaskRegister;Seal-Task $o;Make-Stopped $o;$t.DispatchAttempted=$true
  function Get-RemovalTaskObservation($Task){return @{fixture='observed-worker'}}
  function Complete-RemovalProbeTerminal {return @{Terminal=$false;NeedsObservation=$true;Owners=@('live-probe')}}
  Reject {Assert-RemovalNativeOperation -Resource $t -Operation TaskRetire}
  Reject {Assert-RemovalNativeOperation -Resource $o.RetainedManaged -Operation ManagedRetire}
  function Complete-RemovalProbeTerminal {return @{Terminal=$true;NeedsObservation=$false;Owners=@()}}
  Reject {Assert-RemovalNativeOperation -Resource $t -Operation TaskRetire}
  $path=Join-Path $o.OriginalDirectory 'abort-evidence.json'
  [IO.File]::WriteAllText($path,(@{Stage='BeforeAbortTaskRetirement';Nonce=$o.Nonce;TaskPath=$t.Spec.Path;TraceStopped=$true}|ConvertTo-Json))
  $o.AbortEvidencePin=Read-RemovalDecodeFile $path 1048576
  Assert-RemovalNativeOperation -Resource $o.RetainedManaged -Operation ManagedRetire
  Assert-RemovalNativeOperation -Resource $t -Operation TaskRetire
  Reject {Assert-RemovalNativeOperation -Resource $t -Operation TaskRetire}
 }finally{Close-Fixture $o}
}
Case 'never-dispatched-retirement-needs-original-abort-evidence-but-no-fresh-before' {
 $o=New-Fixture
 try{Initialize-RemovalNativeContext $o;$null=Make-Owned $o;$t=$o.RetainedManaged.TaskOwner
  Assert-RemovalNativeOperation -Resource $t -Operation TaskRegister;Seal-Task $o;Make-Stopped $o
  Reject {Assert-RemovalNativeOperation -Resource $t -Operation TaskRetireUnstarted}
  $path=Join-Path $o.OriginalDirectory 'abort-evidence.json'
  [IO.File]::WriteAllText($path,(@{Stage='BeforeAbortTaskRetirement';Nonce=$o.Nonce;TaskPath=$t.Spec.Path;TraceStopped=$true}|ConvertTo-Json))
  $o.AbortEvidencePin=Read-RemovalDecodeFile $path 1048576
  $o.ReadyFixture=$false;$o.TimedOut=$true;$o.RetainedInputs.Changed=$true
  Assert-RemovalNativeOperation -Resource $t -Operation TaskRetireUnstarted
 }finally{Close-Fixture $o}
}
Case 'task-config-change-never-strands-original-trace-stop' {
 $o=New-Fixture
 try{Initialize-RemovalNativeContext $o;$null=Make-Owned $o -FailedEnable
  $o.RetainedManaged.TaskOwner.Spec.Nonce='f'*32
  Assert-RemovalNativeOperation -Resource $o.OriginalNativeOwner -Operation TraceStopPrepare
 }finally{Close-Fixture $o}
}
Case 'closed-decode-binds-original-finish-and-only-canonical-once-per-step-tools' {
 $o=New-Fixture
 try{Initialize-RemovalNativeContext $o;$d=Bind-Decode $o
  $o.ReadyFixture=$false;$o.TimedOut=$true;$o.Errors.Add('expired capture')
  Assert-RemovalNativeOperation -Resource $d -Operation DecodeBegin;$d.Attempted=$true
  $r=New-RemovalDecodeRequest $d 'tracefmt' $d.Pins.Tracefmt.Path $d.Pins.Tracefmt.Sha256 @($o.OriginalNativeOwner.EtlPath,'-p','C:\symbols','-o',$d.TracePath)
  $d.Steps.Add([pscustomobject]@{Request=$r;Owner=$null;Receipt=$null})
  $bad=$r.PSObject.Copy();$bad.ArgumentList=@('--evil')
  Reject {Assert-RemovalNativeOperation -Resource $d -Operation DecodeTool -Request $bad}
  Assert-RemovalNativeOperation -Resource $d -Operation DecodeTool -Request $r
  Reject {Assert-RemovalNativeOperation -Resource $d -Operation DecodeTool -Request $r}
 }finally{Close-Fixture $o}
}
Case 'decode-cannot-adopt-valid-but-different-late-tool-pins' {
 $o=New-Fixture
 try{Initialize-RemovalNativeContext $o;Reject {Bind-Decode $o -AlternateTool}}finally{Close-Fixture $o}
}
$checks|ConvertTo-Json -Depth 5
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' NATIVE_CALLS=0')
if($failed){exit 1}
