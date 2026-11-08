#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$no=$false;try{&$Action|Out-Null}catch{$no=$true};Check $no 'Expected refusal.'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
$library=Join-Path $PSScriptRoot 'diagnostic-controller.ps1'
Case 'actual-retained-full-controller-api' {
 Check (Test-Path -LiteralPath $library) 'Full retained diagnostic controller is missing.'
 . $library
 foreach($name in @('New-RemovalDiagnosticController','Start-RemovalDiagnosticController','Observe-RemovalDiagnosticController')){
  Check ($null-ne(Get-Command $name -ErrorAction SilentlyContinue)) ('Missing '+$name)
 }
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_AUDIO=false';exit 1}
. $library
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){throw 'Host-only controller boundary tests must not run on target.'}
$root=Join-Path $PSScriptRoot ('logs\controller-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
function Event([string]$Name){$script:f.Events.Add($Name)}
# TEST ONLY: this suite mocks every lifetime resource. Actual private sealing
# and FileStream/collector closure are exercised by test-closed-controller.
function Register-RemovalControllerLifetime($Owner){}
function Bind-RemovalControllerLifetime($Owner){}
function Assert-RemovalControllerSealedIdentity($Owner){}
# TEST ONLY: this DTO orchestration fixture has no constructor CWT or real
# source rows. Actual provenance and immutable time use test-diagnostic-release.
function Assert-RemovalControllerCollectorSource($Owner){}
function Test-RemovalControllerDeadlineExpired($Owner){return ($Owner.Clock.ElapsedMilliseconds-ge$Owner.DeadlineMilliseconds)}
# TEST ONLY: all lifetime resources in this orchestration unit suite are DTO
# mocks. Production CWT/InputJson/read-lock admission is independently exercised
# by test-diagnostic-release; keep this suite's existing admission assertions.
function Assert-RemovalDiagnosticAdmission($Owner){
 Assert-RemovalControllerIdentity $Owner
 if($Owner.Errors.Count-or$Owner.TimedOut-or$Owner.Completed){throw 'Diagnostic admission is closed.'}
 Assert-RemovalDiagnosticLease $Owner.RetainedLease
 Assert-RemovalRunInputs $Owner.RetainedInputs
 Assert-RemovalTargetReadiness $Owner.RetainedBefore
 if($Owner.RetainedBefore.RetainedPreflight.ExecutionMode-cne'RepositoryCollector'){
  throw 'Injected collector readiness cannot authorize a real diagnostic.'
 }
}
function Confirm-RemovalControllerEarlyRefusal($Owner){}
function Test-RemovalControllerDrainOnly($Owner,[int]$ObservationMilliseconds){return $false}
function Prepare-RemovalClosedControllerSeal($Owner){}
function Complete-RemovalClosedControllerSeal($Owner){}
function Assert-RemovalTaskHost {}
function Assert-RemovalRunInputs($Owner){Event 'inputs';if($script:f.InputsChanged){throw 'Original inputs changed.'}}
function New-RemovalDiagnosticLease {
 [pscustomobject]@{Owned=$false;Closed=$false}
}
function Acquire-RemovalDiagnosticLease($Owner){Event 'lease-acquire';if($script:f.LeaseRefused){throw 'Another controller owns the writer.'};$Owner.Owned=$true}
function Assert-RemovalDiagnosticLease($Owner){Check ($Owner.Owned-and-not$Owner.Closed) 'Lease is not held.'}
function Release-RemovalDiagnosticLease($Owner){Event 'lease-release';$Owner.Owned=$false;$Owner.Closed=$true}
function New-RemovalTargetReadiness {
 param($EvidenceDirectory,[switch]$UserRebootConfirmed,$WorkerSessionId,$ReadToolPins)
 [pscustomobject]@{Ready=$false;StartAttempted=$false;Error=$null;ReceiptPin=$null
  RetainedPreflight=[pscustomobject]@{ControllerSession=0;ExecutionMode='RepositoryCollector';ReceiptPin=$null;RetainedOwner=$null
   ReadToolPinsJson=(ConvertTo-Json -InputObject $ReadToolPins -Depth 5 -Compress)}
  RetainedUserSession=[pscustomobject]@{Snapshot=@{Sid='S-1-5-21-3080296863-1242338503-1346362208-1001';SessionId=7}}}
}
function Start-RemovalTargetReadiness($Owner){Event 'before-start';$Owner.StartAttempted=$true}
function Observe-RemovalTargetReadiness {
 param($Owner,$ObservationMilliseconds)
 Event 'before-observe'
 if($script:f.BeforeFailed){$Owner.Error='Fresh boot not established.'}
 $Owner.Ready=-not$script:f.BeforePending-and-not$script:f.BeforeFailed
 @{Ready=$Owner.Ready;NeedsObservation=$script:f.BeforePending;Error=$Owner.Error}
}
function Assert-RemovalTargetReadiness($Owner){Check $Owner.Ready 'Original readiness refused.'}
function New-RemovalTraceSession {
 param($Session,$EtlPath)
 [pscustomobject]@{StartAttempted=$false;StopAttempted=$false;Readback=[pscustomobject]@{fixture='original-readback'}
  NativeOwner=[pscustomobject]@{TraceOwned=$false;TraceStopped=$false;StartAttempted=$false;StartStatus=$null;Failed=$false;StopReceipt=$null;Boundary=@{}};StoppedReceipt=$null}
}
function Start-RemovalTraceSession($Session){
 Event 'trace-start';$Session.StartAttempted=$true;$Session.NativeOwner.StartAttempted=$true
 if($script:f.TraceStartFailure){$Session.NativeOwner.StartStatus=[uint32]5;throw 'Start refused with native error 5.'}
 if($script:f.TraceStartUnknown){throw 'Start result unknown.'}
 $Session.NativeOwner.StartStatus=[uint32]0;$Session.NativeOwner.TraceOwned=$true
 if($script:f.TraceEnableFails){$Session.NativeOwner.Failed=$true;throw 'Enable failed after successful Start.'}
}
function Stop-RemovalTraceSession($Session){
 Event 'trace-stop';$Session.StopAttempted=$true
 if($script:f.StopPending){throw 'Original STOP pending.'}
 $Session.NativeOwner.TraceStopped=$true
 $Session.StoppedReceipt=[pscustomobject]@{Owner=$Session.NativeOwner;StoppedConfirmed=$true;Status=[uint32]0}
 $Session.NativeOwner.StopReceipt=$Session.StoppedReceipt
}
function Get-RemovalDiagnosticArguments {param($Owner,$ArtifactDirectory)@('live-vad','--artifact-dir',$ArtifactDirectory,'--seconds','8')}
function New-RemovalManagedSession {
 param($RunDirectory,$SourceDirectory,$Executable,$ExpectedSha256,$ProbeArguments,$SessionId,$Sid,$Nonce,$TraceSession)
 Event 'stage'
 [pscustomobject]@{RunDirectory=$RunDirectory;Error=$null;Terminal=$null;LastResult=$null
  TaskOwner=[pscustomobject]@{RegistrationAttempted=$false;RegistrationKnown=$false;DispatchAttempted=$false
   WorkerBindingKnown=$false;Retired=$false;RetirementAttempted=$false;Error=$null;Spec=@{Path='\Moiren-Removal613-fixture'}}
  ProbeOwner=$null;TraceSession=$TraceSession;TraceReadback=$TraceSession.Readback}
}
function Register-RemovalTaskDefinition($Owner){
 Event 'task-register'
 if($script:f.RegisterRefused){throw 'Registration refused before native dispatch.'}
 $Owner.RegistrationAttempted=$true;$Owner.RegistrationKnown=-not$script:f.RegisterUnknown
 if($script:f.RegisterUnknown){$Owner.Error='Registration result unknown.'}
}
function Start-RemovalTaskOnce($Owner){
 if($script:f.BeforeRunRefused){throw 'Dispatch refused before RunEx.'}
 Event 'task-run';$Owner.DispatchAttempted=$true
 if($script:f.RunUnknown){$Owner.Error='RunEx result unknown.'}
}
function Observe-RemovalManagedSession($Session){
 Event 'managed-observe'
 if($script:f.ManagedPending){return @{Terminal=$false;NeedsObservation=$true}}
 if($null-eq$Session.ProbeOwner){$Session.ProbeOwner=[pscustomobject]@{AdmissionAttempted=$true;AdmissionWritten=$true}}
 $Session.Terminal=@{Terminal=$true;NeedsObservation=$false}
 return $Session.Terminal
}
function New-RemovalManagedFinish($Session){Event 'finish-create';[pscustomobject]@{Session=$Session;Phase='WaitingManaged';Terminal=$false;RetainedDecode=$null;ResultPin=$null}}
function Observe-RemovalManagedFinish {
 param($Finish,$ObservationMilliseconds)
 Event 'finish-observe'
 if(-not$Finish.Session.TraceSession.StopAttempted){Stop-RemovalTraceSession $Finish.Session.TraceSession}
 if($null-eq$Finish.RetainedDecode){$Finish.Phase='AwaitingDecodePins'}
 elseif($script:f.FinishPending){$Finish.Phase='WaitingTools'}
 else{$Finish.Terminal=$true;$Finish.Phase='Complete';$Finish.Session.TaskOwner.Retired=$true}
 @{Terminal=$Finish.Terminal;Phase=$Finish.Phase;NeedsObservation=(-not$Finish.Terminal)}
}
function New-RemovalDiagnosticDecodePins {param($Owner,$TraceSession,$ManagedSession)Event 'closed-pins';@{fixture='original-pins'}}
function Set-RemovalManagedFinishDecode {param($Finish,$Pins,$OutputDirectory)Event 'decode-attach';$Finish.RetainedDecode=[pscustomobject]@{Completed=$true;Failed=$false}}
function New-RemovalSafetyAudit {param($BeforeReadiness,$EvidenceDirectory,[switch]$UserRebootConfirmed)Event 'after-create';[pscustomobject]@{EvidenceDirectory=$EvidenceDirectory;Complete=$false;Safe=$false;ReceiptPin=$null}}
function Start-RemovalSafetyAudit($Owner){Event 'after-start'}
function Observe-RemovalSafetyAudit {
 param($Owner,$ObservationMilliseconds)
 Event 'after-observe';$Owner.Complete=-not$script:f.AfterPending-and-not$script:f.AfterSaveFails;$Owner.Safe=-not$script:f.AfterUnsafe
 if($Owner.Complete-and$null-eq$Owner.ReceiptPin){
  $null=[IO.Directory]::CreateDirectory($Owner.EvidenceDirectory);$path=Join-Path $Owner.EvidenceDirectory 'safety-audit.json'
  [IO.File]::WriteAllText($path,'{"NativePassed":false,"Qualified":false}')
  $Owner.ReceiptPin=@{Path=$path;Bytes=(Get-Item $path).Length;Sha256=(Get-FileHash $path).Hash}
 }
 @{Complete=$Owner.Complete;Safe=$Owner.Safe;NeedsObservation=$script:f.AfterPending}
}
function Assert-RemovalSafetyAudit($Owner){Check ($Owner.Complete-and$Owner.Safe) 'Original after-audit unsafe.'}
function Assert-RemovalSafetyReceipt($Owner){Check ($Owner.Complete-and$null-ne$Owner.ReceiptPin) 'Original after receipt missing.';$null=Copy-RemovalDecodePin $Owner.ReceiptPin 2097152}
function Complete-RemovalManagedTaskCleanup($Session){Event 'UNSAFE-RETIRE';$Session.TaskOwner.Retired=$true}
function Remove-RemovalUnstartedTask($Owner){
 Event 'unstarted-retire'
 if($Owner.Retired-or$Owner.RetirementAttempted){throw 'Original task retirement cannot repeat.'}
 $Owner.RetirementAttempted=$true
 if($script:f.RetirementUnknown){throw 'Original retirement result unknown.'}
 $Owner.Retired=$true
}
function Fixture {
 $script:f=[pscustomobject]@{Events=(New-Object Collections.Generic.List[string]);BeforePending=$false;BeforeFailed=$false
  InputsChanged=$false;LeaseRefused=$false;TraceEnableFails=$false;TraceStartFailure=$false;TraceStartUnknown=$false
  RegisterRefused=$false;RegisterUnknown=$false;RunUnknown=$false;BeforeRunRefused=$false
  ManagedPending=$false;FinishPending=$false;StopPending=$false;AfterPending=$false;AfterUnsafe=$false;AfterSaveFails=$false;RetirementUnknown=$false}
 $inputs=[pscustomobject]@{SourceDirectory=$PSScriptRoot;ManifestPin=@{Path='C:\fixture\manifest.json';Bytes=2;Sha256='A'*64}
  Pins=@{Tools=@{};Probe=@{File=@{Path='C:\fixture\e2e_latency_probe.exe';Sha256='B'*64}}}}
 New-RemovalDiagnosticController -RunDirectory (Join-Path $root ([Guid]::NewGuid().ToString('N'))) -Inputs $inputs -UserRebootConfirmed
}
function Settle($Owner){
 for($i=0;$i-lt20;$i++){$r=Observe-RemovalDiagnosticController $Owner -ObservationMilliseconds 1;if(-not$r.NeedsObservation){return $r}}
 throw ('Controller boundary fixture remained pending: '+($Owner.Errors -join '; '))
}
Case 'real-controller-binds-original-context-before-start-and-decode-before-tools' {
 $o=Fixture;$o.OriginalNativeOwner.Boundary=$null
 function Initialize-RemovalNativeContext($Owner){Event 'operation-context';Check ([Object]::ReferenceEquals($Owner,$o)) 'Context rebound root.'}
 function Register-RemovalNativeDecode($Owner){Event 'decode-context';Check ($null-ne$Owner.RetainedFinish.RetainedDecode) 'Decode binding precedes original owner.'}
 Start-RemovalDiagnosticController $o;$r=Settle $o
 Check ($r.Completed-and$r.Safe) 'Native-boundary fixture did not settle.'
 $events=$f.Events.ToArray()
 Check (($events-contains'operation-context')-and[Array]::IndexOf($events,'operation-context')-lt[Array]::IndexOf($events,'trace-start')) 'Controller started native work without retained operation binding.'
 Check (($events-contains'decode-context')-and[Array]::IndexOf($events,'decode-attach')-lt[Array]::IndexOf($events,'decode-context')) 'Original decode never received operation binding.'
}
Case 'constructor-retains-owners-without-dispatch-and-confirmation-is-required' {
 $o=Fixture
 Check (-not$o.StartAttempted-and-not(Test-Path $o.RunDirectory)-and$f.Events.Count-eq1) 'Constructor created/dispatched resources.'
 Check ($o.RetainedBefore.RetainedPreflight.ReadToolPinsJson-ceq(ConvertTo-Json -InputObject $o.Inputs.Pins.Tools -Depth 5 -Compress)) 'Original input read-tool pins were not bound to baseline.'
 Reject {New-RemovalDiagnosticController -RunDirectory (Join-Path $root 'unconfirmed') -Inputs $o.Inputs}
}
Case 'complete-order-pins-before-admission-and-audit-before-release' {
 $o=Fixture;Start-RemovalDiagnosticController $o;$r=Settle $o
 Check ($r.Completed-and$r.Safe-and-not$r.Qualified-and-not$r.NativePassed) 'Full diagnostic completion promoted qualification or failed.'
 $events=$f.Events.ToArray()
 foreach($pair in @(@('lease-acquire','before-start'),@('before-observe','trace-start'),@('stage','trace-start'),
  @('trace-start','task-register'),@('task-register','task-run'),@('managed-observe','finish-create'),
  @('finish-observe','closed-pins'),@('decode-attach','after-start'),@('after-observe','lease-release'))){
  Check ([Array]::IndexOf($events,$pair[0])-lt[Array]::IndexOf($events,$pair[1])) ('Wrong ownership order: '+($pair-join' -> '))
 }
 Check ($null-ne$o.ResultPin-and(Test-Path $o.ResultPin.Path)) 'No persisted controller receipt.'
 $count=$f.Events.Count;$null=Observe-RemovalDiagnosticController $o
 Check ($f.Events.Count-eq$count) 'Completed observation repeated dispatch.'
 Reject {Start-RemovalDiagnosticController $o}
}
Case 'pending-before-collection-never-starts-trace-task-or-after-audit' {
 $o=Fixture;$f.BeforePending=$true;Start-RemovalDiagnosticController $o;$r=Observe-RemovalDiagnosticController $o
 Check ($r.NeedsObservation-and-not($f.Events-contains'trace-start')-and-not($f.Events-contains'after-start')-and$o.Lease.Owned) 'Pending baseline released/birthed resources.'
 $f.BeforePending=$false;$r=Settle $o;Check $r.Completed 'Original baseline did not resume.'
}
Case 'successful-trace-then-enable-failure-cleans-original-without-decode-or-task' {
 $o=Fixture;$f.TraceEnableFails=$true;Start-RemovalDiagnosticController $o;$r=Settle $o
 Check ($r.Completed-and-not$r.Safe-and$o.TraceSession.NativeOwner.TraceStopped) 'Failed enable lost original trace.'
 Check (@($f.Events|Where-Object{$_-ceq'trace-stop'}).Count-eq1-and-not($f.Events-contains'task-register')-and-not($f.Events-contains'decode-attach')) 'Abort repeated STOP, launched task or decoded unadmitted run.'
}
Case 'unknown-registration-keeps-original-lease-and-does-not-run-or-adopt-task' {
 $o=Fixture;$f.RegisterUnknown=$true;Start-RemovalDiagnosticController $o
 for($i=0;$i-lt3;$i++){$r=Observe-RemovalDiagnosticController $o}
 Check ($r.NeedsObservation-and-not$r.Completed-and$o.Lease.Owned-and-not($f.Events-contains'task-run')) 'Unknown registration became absent or launched.'
 Check (@($f.Events|Where-Object{$_-ceq'task-register'}).Count-eq1) 'Registration retried.'
}
Case 'unknown-runex-drains-original-session-and-never-reruns' {
 $o=Fixture;$f.RunUnknown=$true;$f.ManagedPending=$true;Start-RemovalDiagnosticController $o
 for($i=0;$i-lt3;$i++){$r=Observe-RemovalDiagnosticController $o}
 Check ($r.NeedsObservation-and$o.Lease.Owned-and$o.ManagedSession.Error-and($f.Events-contains'managed-observe')) 'Unknown RunEx lost retained session or left admission open.'
 Check (@($f.Events|Where-Object{$_-ceq'task-run'}).Count-eq1-and-not($f.Events-contains'lease-release')) 'RunEx retried or unresolved owner released.'
}
Case 'unsafe-after-audit-is-persisted-and-never-accepted-as-safe' {
 $o=Fixture;$f.AfterUnsafe=$true;Start-RemovalDiagnosticController $o;$r=Settle $o
 Check ($r.Completed-and-not$r.Safe-and$null-ne$o.ResultPin-and$o.Lease.Closed) 'Unsafe after result lost evidence or became success.'
 $saved=[IO.File]::ReadAllText($o.ResultPin.Path)|ConvertFrom-Json
 Check (-not$saved.Safe-and-not$saved.Qualified-and$saved.Errors.Count-gt0) 'Unsafe audit not persisted.'
}
Case 'changed-inputs-after-start-close-admission-before-native-birth' {
 $o=Fixture;$f.BeforePending=$true;Start-RemovalDiagnosticController $o
 $null=Observe-RemovalDiagnosticController $o;$f.InputsChanged=$true;$f.BeforePending=$false
 $r=Settle $o
 Check ($r.Completed-and-not$r.Safe-and-not($f.Events-contains'trace-start')-and-not($f.Events-contains'task-run')) 'Changed original pins reached native birth.'
}
Case 'root-timeout-never-bypasses-pending-original-finish-tools-or-validation' {
 $o=Fixture;$f.FinishPending=$true;Start-RemovalDiagnosticController $o
 $null=Observe-RemovalDiagnosticController $o
 Check ($null-ne$o.Finish-and$null-ne$o.Finish.RetainedDecode) 'Fixture never reached original decode attachment.'
 $original=$o.Finish;$o.DeadlineMilliseconds=[long]0
 for($i=0;$i-lt3;$i++){$r=Observe-RemovalDiagnosticController $o}
 Check ($r.NeedsObservation-and-not$r.Completed-and$o.Lease.Owned-and[Object]::ReferenceEquals($original,$o.Finish)) 'Root timeout discarded original pending finish.'
 Check (-not($f.Events-contains'UNSAFE-RETIRE')-and-not($f.Events-contains'after-start')-and-not($f.Events-contains'lease-release')) 'Abort bypassed original tools/evidence validation.'
 $f.FinishPending=$false;$r=Settle $o
 Check ($r.Completed-and-not$r.Safe) 'Original finish did not settle after timeout.'
}
Case 'replaced-public-before-owner-cannot-strand-original-pending-observation' {
 $o=Fixture;$f.BeforePending=$true;Start-RemovalDiagnosticController $o
 $null=Observe-RemovalDiagnosticController $o;$original=$o.RetainedBefore
 $o.BeforeReadiness=[pscustomobject]@{fixture='replacement'};$observations=@($f.Events|Where-Object{$_-ceq'before-observe'}).Count
 $null=Observe-RemovalDiagnosticController $o
 Check (@($f.Events|Where-Object{$_-ceq'before-observe'}).Count-eq$observations+1) 'Outer refusal stranded original runspace observation.'
 $f.BeforePending=$false;$r=Settle $o
 Check ($r.Completed-and-not$r.Safe-and[Object]::ReferenceEquals($o.RetainedBefore,$original)-and-not($f.Events-contains'trace-start')) 'Original failure was replaced/admitted instead of drained.'
}
Case 'lease-refusal-finishes-without-creating-or-observing-target-resources' {
 $o=Fixture;$f.LeaseRefused=$true;Reject {Start-RemovalDiagnosticController $o}
 $r=Observe-RemovalDiagnosticController $o
 Check (-not$r.NeedsObservation-and-not$r.Completed-and-not(Test-Path $o.RunDirectory)) 'Known acquisition refusal invented pending target resources.'
 Check (-not($f.Events-contains'before-observe')-and-not($f.Events-contains'trace-start')) 'Lease refusal reached target collection.'
}
Case 'expired-or-refused-live-readiness-does-not-skip-after-original-acquisition' {
 $o=Fixture;Start-RemovalDiagnosticController $o;$null=Observe-RemovalDiagnosticController $o
 $original=$o.AfterAudit;$o.BeforeReadiness.Ready=$false;$o.DeadlineMilliseconds=[long]0
 $r=Settle $o
 Check ($r.Completed-and-not$r.Safe-and[Object]::ReferenceEquals($original,$o.AfterAudit)-and($f.Events-contains'after-start')) 'Later readiness refusal skipped historical after audit.'
}
Case 'after-receipt-save-failure-never-releases-lease-or-completes-root' {
 $o=Fixture;$f.AfterSaveFails=$true;Start-RemovalDiagnosticController $o
 for($i=0;$i-lt4;$i++){$r=Observe-RemovalDiagnosticController $o}
 Check ($r.NeedsObservation-and-not$r.Completed-and$o.Lease.Owned-and$null-eq$o.ResultPin) 'Missing original AFTER evidence became root completion.'
 Check (-not($f.Events-contains'lease-release')) 'Lease released after safety receipt persistence failure.'
}
Case 'trace-reference-refusal-still-drains-original-pending-finish-without-stop' {
 $o=Fixture;$f.FinishPending=$true;Start-RemovalDiagnosticController $o
 $null=Observe-RemovalDiagnosticController $o
 $o.TraceSession.Readback=[pscustomobject]@{fixture='foreign-readback'}
 $observations=@($f.Events|Where-Object{$_-ceq'finish-observe'}).Count
 $stops=@($f.Events|Where-Object{$_-ceq'trace-stop'}).Count
 $r=Observe-RemovalDiagnosticController $o
 Check ($r.NeedsObservation-and@($f.Events|Where-Object{$_-ceq'finish-observe'}).Count-eq$observations+1) 'Trace reference refusal stranded retained decoder observation.'
 Check (@($f.Events|Where-Object{$_-ceq'trace-stop'}).Count-eq$stops-and-not($f.Events-contains'UNSAFE-RETIRE')) 'Changed trace composition dispatched STOP or retirement.'
}
Case 'outer-reference-refusal-still-observes-original-pending-after-collector' {
 $o=Fixture;$f.AfterPending=$true;Start-RemovalDiagnosticController $o
 for($i=0;$i-lt3;$i++){$null=Observe-RemovalDiagnosticController $o}
 Check $o.AfterStartAttempted 'Fixture never started original after collector.'
 $o.TraceSession.Readback=[pscustomobject]@{fixture='foreign-readback'}
 $observations=@($f.Events|Where-Object{$_-ceq'after-observe'}).Count
 $r=Observe-RemovalDiagnosticController $o
 Check ($r.NeedsObservation-and@($f.Events|Where-Object{$_-ceq'after-observe'}).Count-eq$observations+1) 'Outer refusal stranded original after collector.'
}
Case 'known-never-dispatched-task-retires-once-while-original-after-remains-pending' {
 $o=Fixture;$f.BeforeRunRefused=$true;$f.AfterPending=$true
 Start-RemovalDiagnosticController $o
 for($i=0;$i-lt4;$i++){$r=Observe-RemovalDiagnosticController $o}
 Check ($r.NeedsObservation-and$o.Lease.Owned-and$o.ManagedSession.TaskOwner.Retired) 'Retired task lost pending after ownership.'
 Check (@($f.Events|Where-Object{$_-ceq'unstarted-retire'}).Count-eq1) 'Never-dispatched retirement repeated while awaiting AFTER.'
 $f.AfterPending=$false;$r=Settle $o
 Check ($r.Completed-and-not$r.Safe-and$o.Lease.Closed) 'Confirmed retirement prevented original after completion.'
 Check (@($f.Events|Where-Object{$_-ceq'unstarted-retire'}).Count-eq1-and-not($f.Events-contains'task-run')) 'Retired task repeated deletion or reached RunEx.'
}
Case 'unknown-never-dispatched-retirement-keeps-lease-without-retry-or-after-audit' {
 $o=Fixture;$f.BeforeRunRefused=$true;$f.RetirementUnknown=$true
 Start-RemovalDiagnosticController $o
 for($i=0;$i-lt4;$i++){$r=Observe-RemovalDiagnosticController $o}
 Check ($r.NeedsObservation-and$o.Lease.Owned-and-not$o.ManagedSession.TaskOwner.Retired) 'Unknown retirement became terminal.'
 Check (@($f.Events|Where-Object{$_-ceq'unstarted-retire'}).Count-eq1-and-not($f.Events-contains'after-start')) 'Unknown deletion retried or became an idle safety audit.'
}
Case 'known-unsuccessful-start-never-stops-or-adopts-trace-and-can-audit-refusal' {
 $o=Fixture;$f.TraceStartFailure=$true;Start-RemovalDiagnosticController $o;$r=Settle $o
 Check ($r.Completed-and-not$r.Safe-and$o.Lease.Closed-and($f.Events-contains'after-start')) 'Known failed Start invented unresolved owned trace.'
 Check (@($f.Events|Where-Object{$_-ceq'trace-start'}).Count-eq1-and-not($f.Events-contains'trace-stop')-and-not($f.Events-contains'task-register')) 'Failed Start retried, stopped foreign trace or dispatched task.'
}
Case 'unknown-start-retains-lease-and-never-stops-by-name-or-audits-idle' {
 $o=Fixture;$f.TraceStartUnknown=$true;Start-RemovalDiagnosticController $o
 for($i=0;$i-lt4;$i++){$r=Observe-RemovalDiagnosticController $o}
 Check ($r.NeedsObservation-and$o.Lease.Owned-and-not$r.Completed) 'Unknown Start became resource absence.'
 Check (@($f.Events|Where-Object{$_-ceq'trace-start'}).Count-eq1-and-not($f.Events-contains'trace-stop')-and-not($f.Events-contains'after-start')) 'Unknown Start retried or adopted by name.'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' LOWER_BOUNDARIES=mocked NATIVE_AUDIO=false')
if($failed){exit 1}
