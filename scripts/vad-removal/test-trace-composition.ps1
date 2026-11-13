#requires -Version 5.1
# Actual production native buffers/query/readback/session, mocked API delegates only.
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'trace-session.ps1')
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$refused=$false;try{&$Action|Out-Null}catch{$refused=$true};Check $refused 'Expected refusal'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
Initialize-RemovalTraceNative
function Write-SessionBuffer([IntPtr]$Pointer,[uint64]$Handle,[uint32]$Loss) {
 $v=[Runtime.InteropServices.Marshal]::PtrToStructure($Pointer,[type][Moiren.RemovalTraceQueryV1.Properties])
 $w=$v.Wnode;$w.HistoricalContext=$Handle;$w.Guid=$script:state.Guid;$w.ClientContext=$script:state.Clock;$v.Wnode=$w
 $v.LogFileMode=$script:state.Mode;$v.MaximumFileSize=$script:state.Cap;$v.EventsLost=$Loss
 [Runtime.InteropServices.Marshal]::StructureToPtr($v,$Pointer,$false)
 foreach($entry in @(@{Offset=$v.LoggerNameOffset;Text='MoirenRemoval613composition'},@{Offset=$v.LogFileNameOffset;Text='D:\owned\composition.etl'})){
  $bytes=[Text.Encoding]::Unicode.GetBytes($entry.Text+[char]0)
  [Runtime.InteropServices.Marshal]::Copy($bytes,0,[IntPtr]::Add($Pointer,[int]$entry.Offset),$bytes.Length)
 }
}
function Enumerate-Provider([IntPtr]$Pointers,[uint32]$Capacity,[IntPtr]$Returned) {
 $row=New-Object Moiren.RemovalTraceQueryV1.GuidProperties
 $row.Guid=[Guid]'7dc75ba9-8274-4eae-a42d-b4c74e0209d2';$row.LoggerId=17
 $row.IsEnable=[byte]$script:state.Enabled;$row.EnableLevel=4;$row.EnableFlags=15
 [Runtime.InteropServices.Marshal]::StructureToPtr($row,[Runtime.InteropServices.Marshal]::ReadIntPtr($Pointers),$false)
 [Runtime.InteropServices.Marshal]::WriteInt32($Returned,1)
 return [uint32]0
}
function Fixture {
 $script:state=@{Guid=[Guid]::Empty;Enabled=0;StopStatus=[uint32]0;Queries=0;Stops=0;Loss=[uint32]0
  QueryStatus=[uint32]0;QueryThrows=$false;Mode=[uint32]0x00400001;Clock=[uint32]1;Cap=[uint32]16;ReadbackMode=$null;RequestedMode=$null}
 $native=@{
  Start={param([IntPtr]$Handle,[string]$Name,[IntPtr]$Properties)
   $v=[Runtime.InteropServices.Marshal]::PtrToStructure($Properties,[type][Moiren.RemovalTraceQueryV1.Properties])
   $script:state.RequestedMode=$v.LogFileMode
   $script:state.Guid=$v.Wnode.Guid;[Runtime.InteropServices.Marshal]::WriteInt64($Handle,701);return [uint32]0}
  Enumerate={param([IntPtr]$Pointers,[uint32]$Capacity,[IntPtr]$Returned)Enumerate-Provider $Pointers $Capacity $Returned}
  Enable={param($Handle,$Provider,$Code,$Level,$Any,$All,$Timeout)$script:state.Enabled=1;return [uint32]0}
  Query={param([uint64]$Handle,[IntPtr]$Properties)Write-SessionBuffer $Properties 701 $script:state.Loss;return [uint32]0}
  Stop={param([uint64]$Handle,[IntPtr]$Properties)$script:state.Stops++;Write-SessionBuffer $Properties 701 ([uint32]3);return [uint32]$script:state.StopStatus}
 }
 $query=@{
  Query={param([uint64]$Handle,[IntPtr]$Properties)
   $script:state.Queries++
   if($script:state.QueryThrows){throw 'Injected fresh native query transport failure'}
   Write-SessionBuffer $Properties 701 $script:state.Loss
   if($null-ne$script:state.ReadbackMode){
    $v=[Runtime.InteropServices.Marshal]::PtrToStructure($Properties,[type][Moiren.RemovalTraceQueryV1.Properties])
    $v.LogFileMode=$script:state.ReadbackMode;[Runtime.InteropServices.Marshal]::StructureToPtr($v,$Properties,$false)
   }
   return [uint32]$script:state.QueryStatus}
  Enumerate={param([IntPtr]$Pointers,[uint32]$Capacity,[IntPtr]$Returned)Enumerate-Provider $Pointers $Capacity $Returned}
 }
 New-RemovalTraceSession -Session 'MoirenRemoval613composition' -EtlPath 'D:\owned\composition.etl' -NativeBoundary $native -QueryBoundary $query
}
Case 'actual-owner-query-readback-final-stop-composition-keeps-native-types-and-nonzero-loss' {
 $s=Fixture;Start-RemovalTraceSession $s
 Check ($s.NativeOwner.StartStatus-is[uint32]-and$s.NativeOwner.EnableStatus-is[uint32]-and(Test-RemovalTraceCaptureAdmission $s.Readback)) 'Actual owner/query composition failed admission'
 Refresh-RemovalTraceSession $s
 Check ($script:state.Queries-eq6) 'Fresh refresh missing independent query association'
 Stop-RemovalTraceSession $s
 Check ($script:state.Stops-eq1-and$script:state.Queries-eq6) 'STOP retried or queried stopped session'
 Check ($s.Readback.FinalCountersKnown-and$s.Readback.Final.EventsLost-eq3-and-not$s.Readback.Lossless) 'Final native loss normalized'
 Check ([object]::ReferenceEquals($s.StoppedReceipt,$s.NativeOwner.StopReceipt)) 'Lost original native STOP receipt'
}
Case 'explicit-stop-on-hybrid-mode-is-requested-and-retained-through-readback-and-stop' {
 $s=Fixture;Start-RemovalTraceSession $s
 Check ($script:state.RequestedMode-eq[uint32]0x00400001) 'Start omitted explicit STOP_ON_HYBRID_SHUTDOWN.'
 Check ($s.Readback.Initial.LogFileMode-eq[uint32]0x00400001) 'Initial raw mode was normalized or refused.'
 Refresh-RemovalTraceSession $s;Stop-RemovalTraceSession $s
 Check ($s.Readback.Latest.LogFileMode-eq[uint32]0x00400001-and
  $s.StoppedReceipt.Snapshot.LogFileMode-eq[uint32]0x00400001-and
  $s.Readback.Final.LogFileMode-eq[uint32]0x00400001-and$script:state.Stops-eq1) 'Exact mode lost across retained STOP chain.'
 Check (-not$s.NativeOwner.Qualified-and-not$s.Readback.Lossless) 'Mode acceptance promoted capture qualification.'
}
Case 'failed-initial-readback-mode-refuses-capture-but-known-original-mode-still-cleans-up' {
 foreach($mode in @([uint32]1,[uint32]0x00800001,[uint32]0x100,[uint32]0x00400101,[uint32]0x00c00001,[uint32]0x20400001)){
  $s=Fixture;$script:state.ReadbackMode=$mode
  Reject {Start-RemovalTraceSession $s}
  Check ($s.Readback.Invalid-and-not(Test-RemovalTraceCaptureAdmission $s.Readback)) 'Unexpected readback mode admitted capture.'
  Stop-RemovalTraceSession $s
  Check ($s.NativeOwner.TraceStopped-and$script:state.Stops-eq1-and
   $s.StoppedReceipt.Snapshot.LogFileMode-eq[uint32]0x00400001-and-not$s.Readback.FinalCountersKnown) 'Failed readback lost same-owner cleanup or gained final capture evidence.'
 }
}
Case 'persistent-wrong-mode-on-every-query-still-stops-exact-original-owner-once-with-unknown-loss' {
 foreach($mode in @([uint32]1,[uint32]0x00800001,[uint32]0x100,[uint32]0x00400101,[uint32]0x00c00001,[uint32]0x20400001)){
  $s=Fixture;$original=$s.NativeOwner;$script:state.Mode=$mode
  Reject {Start-RemovalTraceSession $s}
  Check ($s.Readback.Invalid-and$s.NativeOwner.Failed-and-not(Test-RemovalTraceCaptureAdmission $s.Readback)) 'Persistent wrong mode admitted capture.'
  Stop-RemovalTraceSession $s
  Check ([object]::ReferenceEquals($original,$s.NativeOwner)-and$s.NativeOwner.TraceStopped-and$script:state.Stops-eq1) 'Persistent config mismatch stranded or replaced original owner.'
  Check ($s.StoppedReceipt.StoppedConfirmed-and-not$s.StoppedReceipt.FinalCountersKnown-and
   -not$s.Readback.FinalCountersKnown-and$s.NativeOwner.RawTraceLoss-ceq'unknown'-and$s.NativeOwner.Failed) 'Config refusal became trusted loss/capture.'
  $queries=$script:state.Queries;Reject {Stop-RemovalTraceSession $s}
  Check ($script:state.Stops-eq1-and$script:state.Queries-eq$queries) 'Exact cleanup retried or requeried after STOP.'
 }
}
Case 'persistent-wrong-clock-or-cap-refuses-capture-but-not-owned-cleanup' {
 foreach($field in @('Clock','Cap')){
  $s=Fixture;$script:state[$field]=[uint32]17
  Reject {Start-RemovalTraceSession $s};Stop-RemovalTraceSession $s
  Check ($s.Readback.Invalid-and$s.NativeOwner.TraceStopped-and$script:state.Stops-eq1-and
   $s.StoppedReceipt.StoppedConfirmed-and-not$s.StoppedReceipt.FinalCountersKnown-and
   -not$s.Readback.FinalCountersKnown-and$s.NativeOwner.RawTraceLoss-ceq'unknown') 'Persistent clock/cap mismatch stranded owner or acquired final evidence.'
 }
}
Case 'config-drift-after-accepted-readback-stays-invalid-through-persistent-query-and-stop' {
 foreach($field in @('Mode','Clock','Cap')){
  $s=Fixture;Start-RemovalTraceSession $s;$script:state[$field]=[uint32]17
  Reject {Refresh-RemovalTraceSession $s};Stop-RemovalTraceSession $s
  Check ($s.Readback.Invalid-and$s.NativeOwner.Failed-and$s.NativeOwner.TraceStopped-and
   $script:state.Stops-eq1-and-not$s.StoppedReceipt.FinalCountersKnown-and
   -not$s.Readback.FinalCountersKnown-and$s.NativeOwner.RawTraceLoss-ceq'unknown') 'Config drift regained capture evidence or stranded original owner.'
 }
}
Case 'actual-provider-disabled-after-initial-query-blocks-final-collector-ack' {
 . (Join-Path $PSScriptRoot 'collector-boundary.ps1')
 $s=Fixture;Start-RemovalTraceSession $s;$script:state.Enabled=0
 $collector=[pscustomobject]@{TraceSession=$s;TraceReadback=$s.Readback}
 Reject {Assert-RemovalManagedTraceAdmission $collector}
 Check ($s.Readback.Invalid-and-not(Test-RemovalTraceCaptureAdmission $s.Readback)) 'Stale provider readback remained admitted'
 Stop-RemovalTraceSession $s
 Check ($s.NativeOwner.TraceStopped-and-not$s.Readback.FinalCountersKnown) 'Invalid capture prevented cleanup or became qualified'
}
Case 'actual-moredata-stop-cannot-be-repacked-into-known-zero-loss' {
 $s=Fixture;Start-RemovalTraceSession $s;$script:state.StopStatus=[uint32]234
 Reject {Stop-RemovalTraceSession $s}
 Check ($s.NativeOwner.TraceStopped-and-not$s.Readback.FinalCountersKnown-and$s.NativeOwner.RawTraceLoss-ceq'unknown') 'MORE_DATA became lossless'
 $receipt=$s.NativeOwner.StopReceipt
 $clone=[pscustomobject]@{Operation='Stop';Status=[uint32]0;Owner=$s.NativeOwner;OwnedHandle=$receipt.OwnedHandle
  SessionGuid=$receipt.SessionGuid;SessionName=$receipt.SessionName;EtlPath=$receipt.EtlPath
  StoppedConfirmed=$true;FinalCountersKnown=$true;Snapshot=$s.Readback.Initial}
 Reject {Complete-RemovalTraceLoss $s.Readback $clone}
 Check (-not$s.Readback.FinalCountersKnown-and$script:state.Stops-eq1) 'Cloned receipt admitted or native stop repeated'
}
foreach($failure in @('status','throw')){
 Case ('actual-fresh-query-'+$failure+'-blocks-collector-ack-but-retains-owned-cleanup') {
  . (Join-Path $PSScriptRoot 'collector-boundary.ps1')
  $s=Fixture;Start-RemovalTraceSession $s
  if($failure-ceq'status'){$script:state.QueryStatus=[uint32]5}else{$script:state.QueryThrows=$true}
  $collector=[pscustomobject]@{TraceSession=$s;TraceReadback=$s.Readback}
  Reject {Assert-RemovalManagedTraceAdmission $collector}
  Check ($s.Readback.Invalid-and-not(Test-RemovalTraceCaptureAdmission $s.Readback)) 'Failed fresh query retained cached admission'
  $queries=$script:state.Queries
  Reject {Assert-RemovalManagedTraceAdmission $collector}
  Check ($script:state.Queries-eq$queries) 'Sticky invalid readback silently retried native query'
  Stop-RemovalTraceSession $s
  Check ($s.NativeOwner.TraceStopped-and$script:state.Stops-eq1-and-not$s.Readback.FinalCountersKnown) 'Failed capture blocked exact cleanup or gained qualification'
 }
}
Case 'actual-final-stop-counters-below-latest-query-remain-unknown' {
 $s=Fixture;Start-RemovalTraceSession $s
 $script:state.Loss=[uint32]7;Refresh-RemovalTraceSession $s
 Check ($s.Readback.Latest.EventsLost-eq7) 'Latest readback did not retain observed counter'
 # STOP returns 3 after its private pre-STOP query returns 2. This satisfies
 # native-owner monotonicity but must not replace the earlier observed 7.
 $script:state.Loss=[uint32]2
 Reject {Stop-RemovalTraceSession $s}
 Check ($s.NativeOwner.TraceStopped-and$s.NativeOwner.StopReceipt.FinalCountersKnown) 'Native STOP receipt was not retained'
 Check ($s.Readback.Invalid-and-not$s.Readback.FinalCountersKnown-and$s.NativeOwner.RawTraceLoss-ceq'unknown') 'Counter regression against Latest became final loss'
 Check ($script:state.Stops-eq1) 'Counter failure retried STOP'
}
Case 'actual-active-connections-stop-retains-pending-owner-and-never-retries' {
 $s=Fixture;Start-RemovalTraceSession $s;$script:state.StopStatus=[uint32]2402
 Reject {Stop-RemovalTraceSession $s};$receipt=$s.NativeOwner.StopReceipt
 Check ($receipt.Pending-and-not$s.NativeOwner.TraceStopped-and$s.NativeOwner.TraceOwned) 'Pending STOP lost exact retained owner'
 Check (-not$s.Readback.FinalCountersKnown-and$s.NativeOwner.RawTraceLoss-ceq'unknown') 'Pending STOP invented final counters'
 Reject {Stop-RemovalTraceSession $s}
 Check ($script:state.Stops-eq1-and[object]::ReferenceEquals($receipt,$s.NativeOwner.StopReceipt)) 'Pending STOP retried or replaced original receipt'
}
Case 'actual-successful-stop-repacked-receipt-refuses-while-readback-was-valid' {
 $s=Fixture;Start-RemovalTraceSession $s
 $receipt=Stop-RemovalNativeTrace $s.NativeOwner
 Check (-not$s.Readback.Invalid-and-not$s.Readback.FinalCountersKnown-and$receipt.FinalCountersKnown) 'Identity regression fixture already invalid'
 $clone=[pscustomobject]@{Operation=$receipt.Operation;Status=$receipt.Status;Owner=$receipt.Owner
  OwnedHandle=$receipt.OwnedHandle;SessionGuid=$receipt.SessionGuid;SessionName=$receipt.SessionName
  EtlPath=$receipt.EtlPath;StoppedConfirmed=$receipt.StoppedConfirmed
  FinalCountersKnown=$receipt.FinalCountersKnown;Snapshot=$receipt.Snapshot}
 Reject {Complete-RemovalTraceLoss $s.Readback $clone}
 Check ($s.Readback.Invalid-and-not$s.Readback.FinalCountersKnown-and$s.NativeOwner.RawTraceLoss-ceq'unknown') 'Equal-looking clone replaced retained native STOP proof'
}
Case 'actual-published-stop-snapshot-cannot-rewrite-final-nonzero-loss' {
 $s=Fixture;Start-RemovalTraceSession $s;Stop-RemovalTraceSession $s
 $snapshot=$s.NativeOwner.StopReceipt.Snapshot
 Reject {$snapshot.EventsLost=[uint32]0}
 Check ($snapshot.EventsLost-eq3-and$s.Readback.Final.EventsLost-eq3-and-not$s.Readback.Lossless) 'Published final STOP loss remained mutable'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ETW_CALLS=0')
if($failed){exit 1}
