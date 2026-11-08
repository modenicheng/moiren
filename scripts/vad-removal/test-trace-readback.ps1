#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$library=Join-Path $PSScriptRoot 'trace-readback.ps1'
if(Test-Path -LiteralPath $library){. $library}
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
function Reject([scriptblock]$Action){$no=$false;try{&$Action|Out-Null}catch{$no=$true};Check $no 'Expected refusal'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
function Snapshot {
 @{SchemaVersion=1;Complete=$true;QueryStatus=[uint32]0;SessionName='MoirenRemoval613Unit_02'
  SessionGuid=[Guid]'d10a5f71-f826-46bb-ae40-92027928c015';Handle=[uint64]0x123456780017
  LoggerId=[uint32]23;LogFileName='D:\MoirenTest\readback-fixture\trace.etl';ClientContext=[uint32]1
  LogFileMode=[uint32]1;MaximumFileSize=[uint32]16;EventsLost=[uint32]0
  LogBuffersLost=[uint32]0;RealTimeBuffersLost=[uint32]0
  Provider=@{Guid=[Guid]'7dc75ba9-8274-4eae-a42d-b4c74e0209d2';Enabled=$true;LoggerId=[uint32]23
   Level=[byte]4;EnableFlags=[uint32]15;Kind='ClassicWpp'}}
}
function Fixture {
 $trace=[pscustomobject]@{Session='MoirenRemoval613Unit_02';EtlPath='D:\MoirenTest\readback-fixture\trace.etl'
  TraceOwned=$true;TraceStopped=$false;OwnershipBasis='ExactCreateExitZero';SessionReadbackConfirmed=$false
  Handle=[uint64]0x123456780017;SessionGuid=[Guid]'d10a5f71-f826-46bb-ae40-92027928c015'
  StartStatus=[uint32]0;EnableStatus=[uint32]0
  StopReceipt=$null
  RawTraceLoss='unknown';Failed=$false;Errors=New-Object Collections.Generic.List[string]}
 $trace.OwnershipBasis='StartTraceHandleGuid'
 $readback=New-RemovalTraceReadback -Trace $trace
 return @($trace,$readback)
}
function StopReceipt($Trace,$Snapshot) {
 $session=@{}
 foreach($field in @('SessionName','SessionGuid','Handle','LogFileName','ClientContext','LogFileMode','MaximumFileSize','EventsLost','LogBuffersLost','RealTimeBuffersLost')){
  $session[$field]=$Snapshot[$field]
 }
 $receipt=[pscustomobject]@{Operation='Stop';Status=[uint32]0;Owner=$Trace;OwnedHandle=$Trace.Handle
  SessionGuid=$Trace.SessionGuid;SessionName=$Trace.Session;EtlPath=$Trace.EtlPath
  StoppedConfirmed=$true;FinalCountersKnown=$true;Snapshot=[pscustomobject]$session}
 $Trace.StopReceipt=$receipt
 return $receipt
}
Case 'successful-command-without-readback-is-not-capture-admission' {
 $trace,$owner=Fixture
 Check (-not(Test-RemovalTraceCaptureAdmission $owner)) 'Command exit alone admitted capture'
 Confirm-RemovalTraceReadback $owner (Snapshot)
 Check ((Test-RemovalTraceCaptureAdmission $owner)-and$trace.SessionReadbackConfirmed) 'Exact readback did not admit capture'
 Check ($trace.RawTraceLoss-ceq'unknown') 'Initial counters invented final loss'
}
Case 'initial-session-provider-output-clock-mode-and-caps-must-all-match' {
 foreach($field in @('Complete','QueryStatus','SessionName','SessionGuid','Handle','LoggerId','LogFileName','ClientContext','LogFileMode','MaximumFileSize')){
  $trace,$owner=Fixture;$s=Snapshot
  switch($field){'Complete'{$s[$field]=$false};'QueryStatus'{$s[$field]=[uint32]5};'SessionName'{$s[$field]+='other'}
   'SessionGuid'{$s[$field]=[Guid]::Empty};'Handle'{$s[$field]=[uint64]0};'LoggerId'{$s[$field]=[uint32]0}
   'LogFileName'{$s[$field]='D:\foreign.etl'};'ClientContext'{$s[$field]=[uint32]2}
   'LogFileMode'{$s[$field]=[uint32]2};'MaximumFileSize'{$s[$field]=[uint32]0}}
  Reject {Confirm-RemovalTraceReadback $owner $s}
  Check (-not$trace.SessionReadbackConfirmed-and-not(Test-RemovalTraceCaptureAdmission $owner)) ('Mismatched readback admitted '+$field)
 }
 foreach($field in @('Guid','Enabled','LoggerId','Level','EnableFlags','Kind')){
  $trace,$owner=Fixture;$s=Snapshot
  switch($field){'Guid'{$s.Provider[$field]=[Guid]::Empty};'Enabled'{$s.Provider[$field]=$false};'LoggerId'{$s.Provider[$field]++}
   'Level'{$s.Provider[$field]=[byte]5};'EnableFlags'{$s.Provider[$field]=[uint32]1};'Kind'{$s.Provider[$field]='PresenceOnly'}}
  Reject {Confirm-RemovalTraceReadback $owner $s};Check (-not$trace.SessionReadbackConfirmed) ('Provider mismatch admitted '+$field)
 }
}
Case 'typed-native-scalars-cannot-be-replaced-with-coercible-strings' {
 foreach($field in @('SchemaVersion','Complete','QueryStatus','Handle','LoggerId','ClientContext','LogFileMode','MaximumFileSize','EventsLost','LogBuffersLost','RealTimeBuffersLost')){
  $trace,$owner=Fixture;$s=Snapshot;$s[$field]=[string]$s[$field]
  Reject {Confirm-RemovalTraceReadback $owner $s}
 }
}
Case 'classic-provider-native-uint32-level-is-accepted-without-string-coercion' {
 $trace,$owner=Fixture;$s=Snapshot;$s.Provider.Level=[uint32]4
 Confirm-RemovalTraceReadback $owner $s
 Check (Test-RemovalTraceCaptureAdmission $owner) 'Native uint32 provider level was refused'
 $trace,$owner=Fixture;$s=Snapshot;$s.Provider.Level='4'
 Reject {Confirm-RemovalTraceReadback $owner $s}
}
Case 'documented-ulong-provider-session-id-has-no-guessed-16bit-limit' {
 $trace,$owner=Fixture;$s=Snapshot;$s.LoggerId=[uint32]0x12340017;$s.Provider.LoggerId=$s.LoggerId
 Confirm-RemovalTraceReadback $owner $s
 Check (Test-RemovalTraceCaptureAdmission $owner) 'Associated ULONG session handle truncated to guessed bits'
}
Case 'later-snapshot-cannot-adopt-replaced-session-or-decreasing-loss' {
 foreach($field in @('SessionGuid','Handle','LoggerId')){
  $trace,$owner=Fixture;$first=Snapshot;Confirm-RemovalTraceReadback $owner $first
  $next=Snapshot;if($field-ceq'SessionGuid'){$next[$field]=[Guid]::NewGuid()}else{$next[$field]++}
  Reject {Observe-RemovalTraceReadback $owner $next}
  Check (-not(Test-RemovalTraceCaptureAdmission $owner)) 'Replaced session admitted'
 }
 $trace,$owner=Fixture;$first=Snapshot;$first.EventsLost=[uint32]2;Confirm-RemovalTraceReadback $owner $first
 Reject {Observe-RemovalTraceReadback $owner (Snapshot)}
}
Case 'final-raw-loss-requires-confirmed-stop-and-final-identity-snapshot' {
 $trace,$owner=Fixture;Confirm-RemovalTraceReadback $owner (Snapshot)
 Reject {Complete-RemovalTraceLoss $owner (Snapshot)}
 $trace,$owner=Fixture;Confirm-RemovalTraceReadback $owner (Snapshot);$trace.TraceStopped=$true
 $final=Snapshot;$final.EventsLost=[uint32]2;$final.LogBuffersLost=[uint32]1
 Complete-RemovalTraceLoss $owner (StopReceipt $trace $final)
 Check ($owner.FinalCountersKnown-and$trace.RawTraceLoss.EventsLost-eq2-and$trace.RawTraceLoss.LogBuffersLost-eq1-and-not$owner.Lossless) 'Final raw counters lost or normalized to zero'
 Check (-not(Test-RemovalTraceCaptureAdmission $owner)) 'Stopped trace admitted another capture'
}
Case 'initial-query-must-match-retained-start-handle-guid-not-name-or-command-success' {
 foreach($change in @('basis','handle','guid','start','enable')){
  $trace,$owner=Fixture
  switch($change){'basis'{$trace.OwnershipBasis='ExactCreateExitZero'};'handle'{$trace.Handle++}
   'guid'{$trace.SessionGuid=[Guid]::NewGuid()};'start'{$trace.StartStatus=[uint32]183};'enable'{$trace.EnableStatus=[uint32]5}}
  Reject {Confirm-RemovalTraceReadback $owner (Snapshot)}
  Check (-not(Test-RemovalTraceCaptureAdmission $owner)) ('Non-owned session admitted: '+$change)
 }
}
Case 'complete-stop-cannot-reuse-prestop-query-or-a-different-owner-receipt' {
 foreach($change in @('query','owner','handle','guid','operation','moredata','unknown','missing','counter')){
  $trace,$owner=Fixture;Confirm-RemovalTraceReadback $owner (Snapshot);$trace.TraceStopped=$true
  $final=Snapshot;$receipt=StopReceipt $trace $final
  switch($change){'query'{$receipt=$final};'owner'{$receipt.Owner=[pscustomobject]@{}}
   'handle'{$receipt.OwnedHandle++};'guid'{$receipt.SessionGuid=[Guid]::NewGuid()}
   'operation'{$receipt.Operation='Query'};'moredata'{$receipt.Status=[uint32]234;$receipt.FinalCountersKnown=$false}
   'unknown'{$receipt.FinalCountersKnown=$false};'missing'{$receipt.Snapshot=$null}
   'counter'{$receipt.Snapshot.EventsLost='0'}}
  Reject {Complete-RemovalTraceLoss $owner $receipt}
  Check (-not$owner.FinalCountersKnown-and$trace.RawTraceLoss-ceq'unknown') ('Unknown/foreign stop became lossless: '+$change)
 }
}
Case 'repacked-matching-counters-cannot-replace-retained-moredata-stop-receipt' {
 $trace,$owner=Fixture;Confirm-RemovalTraceReadback $owner (Snapshot);$trace.TraceStopped=$true
 $unknown=StopReceipt $trace (Snapshot);$unknown.Status=[uint32]234;$unknown.FinalCountersKnown=$false
 $forged=[pscustomobject]@{Operation='Stop';Status=[uint32]0;Owner=$trace;OwnedHandle=$trace.Handle
  SessionGuid=$trace.SessionGuid;SessionName=$trace.Session;EtlPath=$trace.EtlPath
  StoppedConfirmed=$true;FinalCountersKnown=$true;Snapshot=$unknown.Snapshot}
 Reject {Complete-RemovalTraceLoss $owner $forged}
 Check (-not$owner.FinalCountersKnown-and$trace.RawTraceLoss-ceq'unknown') 'Repacked pre-stop counters became final'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ETW_EXECUTED=false')
if($failed){exit 1}
