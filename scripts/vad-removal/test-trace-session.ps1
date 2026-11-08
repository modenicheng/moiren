#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$refused=$false;try{&$Action|Out-Null}catch{$refused=$true};Check $refused 'Expected refusal'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
$library=Join-Path $PSScriptRoot 'trace-session.ps1'
Case 'composition-library-defines-retained-session-without-dispatch' {
 Check (Test-Path -LiteralPath $library) 'Trace session composition missing'
 . $library
 Check ($null-ne(Get-Command New-RemovalTraceSession -ErrorAction SilentlyContinue)) 'Missing trace session entry'
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_EXECUTED=false';exit 1}
. $library
$script:calls=New-Object Collections.Generic.List[string]
function New-RemovalNativeTrace {
 param($Session,$EtlPath,$NativeBoundary)
 [pscustomobject]@{Session=$Session;EtlPath=$EtlPath;SessionGuid=[Guid]::NewGuid();Handle=[uint64]701
  TraceOwned=$false;TraceStopped=$false;OwnershipBasis=$null;StartStatus=$null;EnableStatus=$null
  StartAttempted=$false;StopAttempted=$false;Failed=$false;Errors=New-Object Collections.Generic.List[string]
  RawTraceLoss='unknown';SessionReadbackConfirmed=$false;StopReceipt=$null}
}
function Start-RemovalNativeTrace {
 param($Owner)
 if($Owner.StartAttempted){throw 'No repeated start'}
 $script:calls.Add('start');$Owner.StartAttempted=$true;$Owner.TraceOwned=$true
 $Owner.StartStatus=[uint32]0;$Owner.EnableStatus=[uint32]0;$Owner.OwnershipBasis='StartTraceHandleGuid'
 if($script:startFailure){$Owner.Failed=$true;throw 'provider enable failed after successful Start'}
}
function Stop-RemovalNativeTrace {
 param($Owner)
 if($Owner.StopAttempted){throw 'No repeated stop'}
 $script:calls.Add('stop');$Owner.StopAttempted=$true;$Owner.TraceStopped=$true
 $snapshot=QuerySnapshot $Owner
 $session=@{};foreach($field in @('SessionName','SessionGuid','Handle','LogFileName','ClientContext','LogFileMode','MaximumFileSize','EventsLost','LogBuffersLost','RealTimeBuffersLost')){$session[$field]=$snapshot[$field]}
 $receipt=[pscustomobject]@{Operation='Stop';Status=[uint32]$script:stopStatus;Owner=$Owner;OwnedHandle=$Owner.Handle
  SessionGuid=$Owner.SessionGuid;SessionName=$Owner.Session;EtlPath=$Owner.EtlPath;StoppedConfirmed=$true
  FinalCountersKnown=($script:stopStatus-eq0);Snapshot=[pscustomobject]$session}
 $Owner.StopReceipt=$receipt;return $receipt
}
function QuerySnapshot($Owner) {
 @{SchemaVersion=[uint32]1;Complete=$true;QueryStatus=[uint32]0;SessionName=$Owner.Session
  SessionGuid=$Owner.SessionGuid;Handle=$Owner.Handle;LoggerId=[uint32]17;LogFileName=$Owner.EtlPath
  ClientContext=[uint32]1;LogFileMode=[uint32]1;MaximumFileSize=[uint32]16;EventsLost=[uint32]$script:loss
  LogBuffersLost=[uint32]0;RealTimeBuffersLost=[uint32]0
  Provider=@{Guid=[Guid]'7dc75ba9-8274-4eae-a42d-b4c74e0209d2';Enabled=$true;LoggerId=[uint32]17
   Level=[uint32]4;EnableFlags=[uint32]15;Kind='ClassicWpp'}}
}
function Get-RemovalTraceQuerySnapshot {
 param($SessionName,$SessionGuid,$Handle,$QueryBoundary)
 $script:calls.Add('query')
 Check ($Handle-eq$script:current.NativeOwner.Handle-and$SessionGuid-eq$script:current.NativeOwner.SessionGuid) 'Query did not use retained Start identity'
 $s=QuerySnapshot $script:current.NativeOwner
 if($script:replace){$s.SessionGuid=[Guid]::NewGuid()}
 return $s
}
function Fixture {
 $script:calls.Clear();$script:loss=0;$script:replace=$false;$script:startFailure=$false;$script:stopStatus=0
 $script:current=New-RemovalTraceSession -Session 'MoirenRemoval613session' -EtlPath 'D:\owned\removal.etl' `
  -NativeBoundary @{Start={};Enumerate={};Enable={};Query={};Stop={}} -QueryBoundary @{Query={};Enumerate={}}
 return $script:current
}
Case 'start-queries-native-session-before-capture-and-refresh-queries-again' {
 $s=Fixture
 Check ($script:calls.Count-eq0) 'Construction dispatched tools'
 Start-RemovalTraceSession $s
 Check (($script:calls-join',')-ceq'start,query'-and(Test-RemovalTraceCaptureAdmission $s.Readback)) 'No initial query/admission'
 Refresh-RemovalTraceSession $s
 Check (($script:calls-join',')-ceq'start,query,query') 'Refresh reused detached cached permission'
}
Case 'one-sided-mock-boundaries-refuse-before-owner-construction' {
 Reject {New-RemovalTraceSession -Session 'MoirenRemoval613session' -EtlPath 'D:\owned\removal.etl' -NativeBoundary @{fixture=$true}}
 Reject {New-RemovalTraceSession -Session 'MoirenRemoval613session' -EtlPath 'D:\owned\removal.etl' -QueryBoundary @{fixture=$true}}
}
Case 'replaced-session-refuses-new-capture-but-retains-original-owner-for-cleanup' {
 $s=Fixture;Start-RemovalTraceSession $s;$original=$s.NativeOwner;$script:replace=$true
 Reject {Refresh-RemovalTraceSession $s}
 Check ([object]::ReferenceEquals($original,$s.NativeOwner)-and-not(Test-RemovalTraceCaptureAdmission $s.Readback)) 'Replacement adopted or owner abandoned'
}
Case 'stop-uses-returned-final-counters-with-no-poststop-query' {
 $s=Fixture;Start-RemovalTraceSession $s;$script:loss=3
 Stop-RemovalTraceSession $s
 Check (($script:calls-join',')-ceq'start,query,stop') 'STOP queried already stopped provider/session'
 Check ($s.Readback.FinalCountersKnown-and$s.NativeOwner.RawTraceLoss.EventsLost-eq3-and-not$s.Readback.Lossless) 'Final loss concealed'
 Reject {Refresh-RemovalTraceSession $s}
 Reject {Stop-RemovalTraceSession $s}
}
Case 'stop-more-data-is-stopped-but-final-loss-remains-unknown' {
 $s=Fixture;Start-RemovalTraceSession $s;$script:stopStatus=234
 Reject {Stop-RemovalTraceSession $s}
 Check ($s.NativeOwner.TraceStopped-and-not$s.Readback.FinalCountersKnown-and$s.NativeOwner.RawTraceLoss-ceq'unknown') 'MORE_DATA invented final counters'
 Check (($script:calls-join',')-ceq'start,query,stop') 'Retried stop'
}
Case 'failed-enable-retains-created-owner-and-does-not-query-or-admit' {
 $s=Fixture;$script:startFailure=$true;Reject {Start-RemovalTraceSession $s}
 Check ($s.NativeOwner.TraceOwned-and-not(Test-RemovalTraceCaptureAdmission $s.Readback)) 'Created failed owner lost or admitted'
 Check (($script:calls-join',')-ceq'start') 'Queried failed enable'
 Stop-RemovalTraceSession $s
 Check ($s.NativeOwner.TraceStopped-and-not$s.Readback.FinalCountersKnown) 'Failed initialization prevented owned cleanup or invented qualification'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' NATIVE_EXECUTED=false')
if($failed){exit 1}
