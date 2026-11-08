#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
function Reject([scriptblock]$Action){
    $refused=$false;try {&$Action|Out-Null} catch {$refused=$true}
    Check $refused 'Expected refusal.'
}
function Case([string]$Name,[scriptblock]$Action){
    try {&$Action;$checks.Add(@{name=$Name;passed=$true})}
    catch {$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}
}
$library=Join-Path $PSScriptRoot 'trace-native.ps1'
Case 'native-owner-definitions-exist-without-native-dispatch' {
    Check (Test-Path -LiteralPath $library) 'Native trace owner not implemented.'
    . $library
    Check ($null -ne (Get-Command New-RemovalNativeTrace -ErrorAction SilentlyContinue)) 'Owner entry missing.'
}
if(@($checks|Where-Object{-not $_.passed}).Count){
    $checks|ConvertTo-Json -Depth 6
    Write-Output ('CHECKS='+$checks.Count+' FAILED=1 ETW_CALLS=0')
    exit 1
}
. $library
Initialize-RemovalTraceNative
function New-Fixture {
    $script:mock=@{
        Handle=[UInt64]701;StartStatus=[UInt32]0;EnableStatus=[UInt32]0
        QueryStatus=[UInt32]0;StopStatus=[UInt32]0;Calls=New-Object Collections.Generic.List[string]
        StartThrow=$false;EnableThrow=$false;QueryThrow=$false;StopThrow=$false
        QueryMutation=$null;StopMutation=$null;StartInput=$null;Lost=[UInt32]8;QueryLost=[UInt32]3
        LogLost=[UInt32]2;RealtimeLost=[UInt32]1
        ProviderStatus=[UInt32]0;ProviderReturned=[UInt32]1;ProviderRows=1;ProviderEnabled=[byte]0
        StartNoStatus=$false;EnableNoStatus=$false;QueryNoStatus=$false;StopNoStatus=$false
    }
    @{
        Start={
            param([IntPtr]$Handle,[string]$Name,[IntPtr]$Properties)
            $script:mock.Calls.Add('start')
            $v=[Runtime.InteropServices.Marshal]::PtrToStructure($Properties,[type][Moiren.RemovalTraceQueryV1.Properties])
            $script:mock.StartInput=$v
            Check ($Name -ceq 'MoirenRemoval613diagnostic') 'Unexpected session name.'
            Check ($v.Wnode.BufferSize -eq [Moiren.RemovalTraceQueryV1.Reader]::BufferBytes -and
                $v.Wnode.ClientContext -eq 1 -and $v.Wnode.Flags -eq 0x20000 -and
                $v.LogFileMode -eq 1 -and $v.MaximumFileSize -eq 16 -and $v.EnableFlags -eq 0) 'Start configuration differs.'
            Check ([Runtime.InteropServices.Marshal]::PtrToStringUni([IntPtr]::Add($Properties,[int]$v.LogFileNameOffset)) -ceq 'D:\owned\removal.etl') 'Start ETL string missing.'
            $raw=[BitConverter]::ToInt64([BitConverter]::GetBytes([UInt64]$script:mock.Handle),0)
            [Runtime.InteropServices.Marshal]::WriteInt64($Handle,$raw)
            if($script:mock.StartThrow){throw 'mock start exception after output'}
            if($script:mock.StartNoStatus){return}
            return [UInt32]$script:mock.StartStatus
        }
        Enumerate={
            param([IntPtr]$Pointers,[UInt32]$Capacity,[IntPtr]$Returned)
            $script:mock.Calls.Add('enumerate')
            Check ($Capacity -eq 4096) 'Provider enumeration cap differs.'
            for($i=0;$i -lt $script:mock.ProviderRows;$i++){
                $row=New-Object Moiren.RemovalTraceQueryV1.GuidProperties
                $row.Guid=[Guid]'7dc75ba9-8274-4eae-a42d-b4c74e0209d2'
                $row.IsEnable=$script:mock.ProviderEnabled;$row.LoggerId=31
                $p=[Runtime.InteropServices.Marshal]::ReadIntPtr($Pointers,$i*[IntPtr]::Size)
                [Runtime.InteropServices.Marshal]::StructureToPtr($row,$p,$false)
            }
            [Runtime.InteropServices.Marshal]::WriteInt32($Returned,[int]$script:mock.ProviderReturned)
            return [UInt32]$script:mock.ProviderStatus
        }
        Enable={
            param([UInt64]$Handle,[Guid]$Provider,[UInt32]$Code,[byte]$Level,[UInt64]$Any,[UInt64]$All,[UInt32]$Timeout)
            $script:mock.Calls.Add('enable')
            Check ($Handle -eq 701 -and $Provider -eq [Guid]'7dc75ba9-8274-4eae-a42d-b4c74e0209d2' -and
                $Code -eq 1 -and $Level -eq 4 -and $Any -eq 15 -and $All -eq 0 -and $Timeout -eq 5000) 'Classic enable arguments differ.'
            if($script:mock.EnableThrow){throw 'mock enable exception'}
            if($script:mock.EnableNoStatus){return}
            return [UInt32]$script:mock.EnableStatus
        }
        Query={
            param([UInt64]$Handle,[IntPtr]$Properties)
            $script:mock.Calls.Add('query')
            if($script:mock.QueryThrow){throw 'mock query exception'}
            if($script:mock.QueryNoStatus){return}
            if($script:mock.QueryStatus -eq 0){Write-ReturnedProperties $Properties $Handle $script:mock.QueryLost $script:mock.QueryMutation}
            return [UInt32]$script:mock.QueryStatus
        }
        Stop={
            param([UInt64]$Handle,[IntPtr]$Properties)
            $script:mock.Calls.Add('stop')
            Write-ReturnedProperties $Properties $Handle $script:mock.Lost $script:mock.StopMutation
            if($script:mock.StopThrow){throw 'mock stop exception'}
            if($script:mock.StopNoStatus){return}
            return [UInt32]$script:mock.StopStatus
        }
    }
}
function Write-ReturnedProperties([IntPtr]$Pointer,[UInt64]$Handle,[UInt32]$Lost,$Mutation){
    $v=[Runtime.InteropServices.Marshal]::PtrToStructure($Pointer,[type][Moiren.RemovalTraceQueryV1.Properties])
    Check ($Handle -eq 701 -and $v.Wnode.Guid -ne [Guid]::Empty -and
        $v.Wnode.HistoricalContext -eq 0 -and $v.EventsLost -eq 0) 'QUERY/STOP input was seeded with expected output.'
    $w=$v.Wnode;$w.HistoricalContext=$Handle;$w.ClientContext=1;$v.Wnode=$w
    $v.LogFileMode=1;$v.MaximumFileSize=16;$v.EventsLost=$Lost
    $v.LogBuffersLost=$script:mock.LogLost;$v.RealTimeBuffersLost=$script:mock.RealtimeLost
    [Runtime.InteropServices.Marshal]::StructureToPtr($v,$Pointer,$false)
    foreach($entry in @(@{Offset=$v.LoggerNameOffset;Text='MoirenRemoval613diagnostic'},@{Offset=$v.LogFileNameOffset;Text='D:\owned\removal.etl'})){
        $bytes=[Text.Encoding]::Unicode.GetBytes($entry.Text+[char]0)
        [Runtime.InteropServices.Marshal]::Copy($bytes,0,[IntPtr]::Add($Pointer,[int]$entry.Offset),$bytes.Length)
    }
    if($null -ne $Mutation){&$Mutation $Pointer}
}
function New-Owner($Boundary){
    New-RemovalNativeTrace -Session 'MoirenRemoval613diagnostic' -EtlPath 'D:\owned\removal.etl' -NativeBoundary $Boundary
}
function Started-Owner {
    $b=New-Fixture;$o=New-Owner $b;Start-RemovalNativeTrace $o|Out-Null;return $o
}
Case 'successful-create-retains-fresh-guid-handle-and-enables-classic-provider-once' {
    $o=Started-Owner
    Check ($o.StartAttempted -and $o.StartStatus -eq 0 -and $o.Handle -is [UInt64] -and $o.Handle -eq 701 -and
        $o.SessionGuid -is [Guid] -and $o.SessionGuid -ne [Guid]::Empty -and $o.TraceOwned -and
        $o.OwnershipBasis -ceq 'StartTraceHandleGuid' -and $o.EnableAttempted -and $o.EnableStatus -eq 0 -and -not $o.Failed) 'Missing retained successful owner.'
    Check ($o.ProviderIdleAttempted -and $o.ProviderIdleStatus -is [UInt32] -and $o.ProviderIdleStatus -eq 0 -and $o.ProviderIdleConfirmed) 'No exact classic idle observation.'
    Check (($script:mock.Calls -join ',') -ceq 'start,enumerate,enable') 'Unexpected operation/retry.'
    Reject {Start-RemovalNativeTrace $o}
    Check ($script:mock.Calls.Count -eq 3) 'Duplicate Start reached API.'
}
Case 'each-owner-has-fresh-guid-with-stable-descriptive-session-name' {
    $b=New-Fixture;$a=New-Owner $b;$c=New-Owner $b
    Check ($a.SessionGuid -ne $c.SessionGuid -and $a.Session -ceq $c.Session) 'GUID not fresh or name randomized.'
}
Case 'existing-session-failure-is-never-adopted-enabled-or-stopped' {
    $b=New-Fixture;$script:mock.StartStatus=183;$o=New-Owner $b;Start-RemovalNativeTrace $o|Out-Null
    Check ($o.StartStatus -eq 183 -and $o.Handle -eq 701 -and -not $o.TraceOwned -and $o.Failed -and -not $o.EnableAttempted) 'Existing/failed trace adopted.'
    Reject {Stop-RemovalNativeTrace $o}
    Check (($script:mock.Calls -join ',') -ceq 'start') 'Foreign trace touched.'
}
Case 'exception-after-start-output-retains-handle-but-not-ownership' {
    $b=New-Fixture;$script:mock.StartThrow=$true;$o=New-Owner $b;Start-RemovalNativeTrace $o|Out-Null
    Check ($o.StartAttempted -and $null -eq $o.StartStatus -and $o.Handle -eq 701 -and $o.Failed -and -not $o.TraceOwned) 'Ambiguous Start output lost/promoted.'
    Reject {Start-RemovalNativeTrace $o};Reject {Stop-RemovalNativeTrace $o}
    Check ($script:mock.Calls.Count -eq 1) 'Ambiguous Start retried/adopted.'
}
Case 'successful-start-with-invalid-handle-refuses-enable' {
    foreach($handle in @([UInt64]0,[UInt64]::MaxValue)){
        $b=New-Fixture;$script:mock.Handle=$handle;$o=New-Owner $b;Start-RemovalNativeTrace $o|Out-Null
        Check ($o.StartStatus -eq 0 -and $o.Handle -eq $handle -and -not $o.TraceOwned -and $o.Failed -and -not $o.EnableAttempted) 'Invalid handle granted ownership.'
    }
}
Case 'enable-failure-retains-created-owner-for-exact-cleanup' {
    foreach($mode in @('status','exception')){
        $b=New-Fixture
        if($mode -eq 'status'){$script:mock.EnableStatus=1460}else{$script:mock.EnableThrow=$true}
        $o=New-Owner $b;Start-RemovalNativeTrace $o|Out-Null
        Check ($o.TraceOwned -and $o.Failed -and $o.Handle -eq 701) 'Enable failure discarded create owner.'
        $r=Stop-RemovalNativeTrace $o
        Check ($r.StoppedConfirmed -and $r.FinalCountersKnown -and $o.Failed) 'Cleanup unavailable or erased sticky enable failure.'
        Check (($script:mock.Calls -join ',') -ceq 'start,enumerate,enable,query,stop') 'Cleanup retried enable or used provider as ownership.'
    }
}
Case 'classic-enabled-missing-ambiguous-or-truncated-provider-never-enables-or-steals' {
    foreach($mode in @('enabled','missing','duplicate','truncated','error','invalid')){
        $b=New-Fixture
        switch($mode){
            'enabled'{$script:mock.ProviderEnabled=[byte]1}
            'missing'{$script:mock.ProviderReturned=0;$script:mock.ProviderRows=0}
            'duplicate'{$script:mock.ProviderReturned=2;$script:mock.ProviderRows=2}
            'truncated'{$script:mock.ProviderReturned=4097}
            'error'{$script:mock.ProviderStatus=234}
            'invalid'{$script:mock.ProviderEnabled=[byte]2}
        }
        $o=New-Owner $b;Start-RemovalNativeTrace $o|Out-Null
        Check ($o.TraceOwned -and $o.Failed -and -not $o.EnableAttempted -and -not $o.ProviderIdleConfirmed) ('Unsafe classic enable: '+$mode)
        $r=Stop-RemovalNativeTrace $o
        Check ($r.StoppedConfirmed -and ($script:mock.Calls -join ',') -ceq 'start,enumerate,query,stop') 'Refusal lost exact created-owner cleanup.'
    }
}
Case 'stop-queries-owned-handle-and-final-counters-come-only-from-stop-return-buffer' {
    $o=Started-Owner;$r=Stop-RemovalNativeTrace $o
    Check ([Object]::ReferenceEquals($r.Owner,$o) -and [Object]::ReferenceEquals($o.StopReceipt,$r)) 'Exact owner/receipt not retained.'
    Check ($r.Operation -ceq 'Stop' -and $r.Status -is [UInt32] -and $r.Status -eq 0 -and
        $r.OwnedHandle -eq $o.Handle -and $r.SessionGuid -eq $o.SessionGuid -and $r.SessionName -ceq $o.Session -and $r.EtlPath -ceq $o.EtlPath) 'STOP provenance missing.'
    Check ($r.StoppedConfirmed -and $o.TraceStopped -and $r.FinalCountersKnown -and $r.Snapshot.EventsLost -eq 8 -and
        $r.Snapshot.LogBuffersLost -eq 2 -and $r.Snapshot.RealTimeBuffersLost -eq 1) 'STOP final counters not decoded.'
    Check (($script:mock.Calls -join ',') -ceq 'start,enumerate,enable,query,stop') 'Post-stop query or retry occurred.'
    Reject {Stop-RemovalNativeTrace $o}
    Check ($script:mock.Calls.Count -eq 5) 'Repeated STOP reached API.'
}
Case 'stop-more-data-means-stopped-but-no-final-counter-claim' {
    $o=Started-Owner;$script:mock.StopStatus=234;$r=Stop-RemovalNativeTrace $o
    Check ($r.Status -eq 234 -and $r.StoppedConfirmed -and $o.TraceStopped -and -not $r.FinalCountersKnown -and
        $null -eq $r.Snapshot -and $o.RawTraceLoss -ceq 'unknown' -and $o.Failed) 'MORE_DATA promoted/invented final counters.'
    Reject {Stop-RemovalNativeTrace $o}
}
Case 'stop-active-connections-retains-pending-owner-without-retry' {
    $o=Started-Owner;$script:mock.StopStatus=2402;$r=Stop-RemovalNativeTrace $o
    Check ($r.Status -eq 2402 -and $r.Pending -and -not $r.StoppedConfirmed -and -not $o.TraceStopped -and
        -not $r.FinalCountersKnown -and $o.TraceOwned -and $o.Handle -eq 701 -and $o.Failed) 'Pending stop discarded/promoted.'
    Reject {Stop-RemovalNativeTrace $o}
}
Case 'stop-other-error-or-exception-is-unresolved-not-closed' {
    foreach($mode in @('status','exception')){
        $o=Started-Owner
        if($mode -eq 'status'){$script:mock.StopStatus=4201}else{$script:mock.StopThrow=$true}
        $r=Stop-RemovalNativeTrace $o
        Check (-not $r.StoppedConfirmed -and -not $r.FinalCountersKnown -and $null -eq $r.Snapshot -and $o.Failed -and $o.StopAttempted) 'Unknown STOP became closed/final.'
        if($mode -eq 'exception'){Check ($null -eq $r.Status) 'Exception invented native status.'}
        Reject {Stop-RemovalNativeTrace $o}
    }
}
Case 'sdk-other-statuses-are-not-misclassified-as-active-connections' {
    foreach($status in @([UInt32]170,[UInt32]1800,[UInt32]7007)){
        $o=Started-Owner;$script:mock.StopStatus=$status;$r=Stop-RemovalNativeTrace $o
        Check ($r.Status -eq $status -and -not $r.Pending -and -not $r.StoppedConfirmed -and -not $r.FinalCountersKnown -and $o.Failed) 'Wrong Win32 status classified as ACTIVE_CONNECTIONS.'
    }
}
Case 'successful-stop-malformed-return-still-stopped-with-unknown-counters' {
    $o=Started-Owner
    $script:mock.StopMutation={param($p)$v=[Runtime.InteropServices.Marshal]::PtrToStructure($p,[type][Moiren.RemovalTraceQueryV1.Properties]);$v.LoggerNameOffset=4;[Runtime.InteropServices.Marshal]::StructureToPtr($v,$p,$false)}
    $r=Stop-RemovalNativeTrace $o
    Check ($r.Status -eq 0 -and $r.StoppedConfirmed -and $o.TraceStopped -and -not $r.FinalCountersKnown -and $null -eq $r.Snapshot -and $o.Failed) 'Decode failure lost stop success or invented final evidence.'
}
Case 'successful-stop-foreign-identity-or-counter-regression-is-not-final' {
    foreach($mode in @('guid','handle','path','counter')){
        $o=Started-Owner
        $script:mock.StopMutation={
            param($p)
            $v=[Runtime.InteropServices.Marshal]::PtrToStructure($p,[type][Moiren.RemovalTraceQueryV1.Properties]);$w=$v.Wnode
            switch($mode){
                'guid'{$w.Guid=[Guid]::NewGuid()}
                'handle'{$w.HistoricalContext=702}
                'path'{[Runtime.InteropServices.Marshal]::WriteInt16($p,[int]$v.LogFileNameOffset,88)}
                'counter'{$v.EventsLost=2}
            }
            $v.Wnode=$w;[Runtime.InteropServices.Marshal]::StructureToPtr($v,$p,$false)
        }
        $r=Stop-RemovalNativeTrace $o
        Check ($r.StoppedConfirmed -and -not $r.FinalCountersKnown -and $null -eq $r.Snapshot -and $o.Failed) ('Untrusted STOP data accepted: '+$mode)
    }
}
Case 'prestop-query-error-or-foreign-identity-never-stops-or-retries' {
    foreach($mode in @('status','exception','guid')){
        $o=Started-Owner
        switch($mode){
            'status'{$script:mock.QueryStatus=4201}
            'exception'{$script:mock.QueryThrow=$true}
            'guid'{$script:mock.QueryMutation={param($p)$v=[Runtime.InteropServices.Marshal]::PtrToStructure($p,[type][Moiren.RemovalTraceQueryV1.Properties]);$w=$v.Wnode;$w.Guid=[Guid]::NewGuid();$v.Wnode=$w;[Runtime.InteropServices.Marshal]::StructureToPtr($v,$p,$false)}}
        }
        $r=Stop-RemovalNativeTrace $o
        Check ($o.StopQueryAttempted -and -not $o.StopAttempted -and $null -eq $r.Status -and -not $r.StoppedConfirmed -and $o.TraceOwned -and $o.Failed) 'Failed query caused blind STOP/owner loss.'
        Reject {Stop-RemovalNativeTrace $o}
        Check (($script:mock.Calls -join ',') -ceq 'start,enumerate,enable,query') 'Failed query retried or stopped by name.'
    }
}
Case 'complete-mock-boundary-required-and-owner-identity-is-readonly' {
    Reject {New-Owner @{Start={};Enable={};Query={};Stop={}}}
    $b=New-Fixture;$o=New-Owner $b
    Reject {$o.Handle=[UInt64]9};Reject {$o.SessionGuid=[Guid]::NewGuid()};Reject {$o.TraceOwned=$true}
    Check ($script:mock.Calls.Count -eq 0) 'Construction dispatched API.'
}
Case 'missing-native-status-is-unknown-never-implicit-success' {
    foreach($operation in @('Start','Enable','Query','Stop')){
        $b=New-Fixture
        if($operation -eq 'Start'){
            $script:mock.StartNoStatus=$true;$o=New-Owner $b;Start-RemovalNativeTrace $o|Out-Null
            Check ($null -eq $o.StartStatus -and $o.Handle -eq 701 -and -not $o.TraceOwned -and $o.Failed) 'Missing Start status became zero/owned.'
        } elseif($operation -eq 'Enable'){
            $script:mock.EnableNoStatus=$true;$o=New-Owner $b;Start-RemovalNativeTrace $o|Out-Null
            Check ($null -eq $o.EnableStatus -and $o.TraceOwned -and $o.Failed) 'Missing Enable status became zero.'
        } else {
            $o=New-Owner $b;Start-RemovalNativeTrace $o|Out-Null
            $script:mock[$operation+'NoStatus']=$true;$r=Stop-RemovalNativeTrace $o
            Check ($null -eq $r.Status -and -not $r.StoppedConfirmed -and -not $r.FinalCountersKnown -and $o.Failed) ('Missing '+$operation+' status became successful STOP.')
        }
    }
}
Case 'actual-created-owner-and-retained-stop-receipt-compose-with-readback' {
    . (Join-Path $PSScriptRoot 'trace-readback.ps1')
    $o=Started-Owner
    $snapshot=[pscustomobject]@{
        SchemaVersion=[UInt32]1;Complete=$true;QueryStatus=[UInt32]0
        SessionName=$o.Session;SessionGuid=$o.SessionGuid;Handle=$o.Handle;LoggerId=[UInt32]17
        LogFileName=$o.EtlPath;ClientContext=[UInt32]1;LogFileMode=[UInt32]1;MaximumFileSize=[UInt32]16
        EventsLost=[UInt32]3;LogBuffersLost=[UInt32]2;RealTimeBuffersLost=[UInt32]1
        Provider=[pscustomobject]@{Guid=[Guid]'7dc75ba9-8274-4eae-a42d-b4c74e0209d2';Enabled=$true
            LoggerId=[UInt32]17;Level=[UInt32]4;EnableFlags=[UInt32]15;Kind='ClassicWpp'}
    }
    $readback=New-RemovalTraceReadback -Trace $o
    Confirm-RemovalTraceReadback $readback $snapshot
    Check (Test-RemovalTraceCaptureAdmission $readback) 'Actual created owner did not compose with readback.'
    $r=Stop-RemovalNativeTrace $o
    Complete-RemovalTraceLoss $readback $r
    Check ($readback.FinalCountersKnown -and -not $readback.Lossless -and $o.RawTraceLoss.EventsLost -eq 8 -and
        -not (Test-RemovalTraceCaptureAdmission $readback)) 'Actual returned STOP receipt/counters did not compose.'
    Check (($script:mock.Calls -join ',') -ceq 'start,enumerate,enable,query,stop') 'Readback caused STOP retry or post-stop query.'
}
Case 'returned-stop-counter-and-path-are-readonly-before-final-loss-policy' {
    . (Join-Path $PSScriptRoot 'trace-readback.ps1')
    $b=New-Fixture;$script:mock.QueryLost=0;$script:mock.Lost=3;$script:mock.LogLost=0;$script:mock.RealtimeLost=0
    $o=New-Owner $b;Start-RemovalNativeTrace $o|Out-Null
    $snapshot=[pscustomobject]@{
        SchemaVersion=[UInt32]1;Complete=$true;QueryStatus=[UInt32]0
        SessionName=$o.Session;SessionGuid=$o.SessionGuid;Handle=$o.Handle;LoggerId=[UInt32]17
        LogFileName=$o.EtlPath;ClientContext=[UInt32]1;LogFileMode=[UInt32]1;MaximumFileSize=[UInt32]16
        EventsLost=[UInt32]0;LogBuffersLost=[UInt32]0;RealTimeBuffersLost=[UInt32]0
        Provider=[pscustomobject]@{Guid=[Guid]'7dc75ba9-8274-4eae-a42d-b4c74e0209d2';Enabled=$true
            LoggerId=[UInt32]17;Level=[UInt32]4;EnableFlags=[UInt32]15;Kind='ClassicWpp'}
    }
    $readback=New-RemovalTraceReadback -Trace $o;Confirm-RemovalTraceReadback $readback $snapshot
    $r=Stop-RemovalNativeTrace $o
    Reject {$r.Snapshot.EventsLost=[UInt32]0}
    Reject {$r.Snapshot.LogFileName='D:\foreign\lossless.etl'}
    Reject {$r.Snapshot.SessionGuid=[Guid]::NewGuid()}
    Complete-RemovalTraceLoss $readback $r
    Check ($readback.FinalCountersKnown -and -not $readback.Lossless -and $readback.Final.EventsLost -eq 3 -and
        $readback.Final.LogFileName -ceq $o.EtlPath) 'Actual STOP loss was erased by caller mutation.'
}
Case 'established-stop-payload-cannot-be-replaced-through-public-capture' {
    $o=Started-Owner;$r=Stop-RemovalNativeTrace $o;$original=$r.Snapshot
    $replacement=New-Object Moiren.RemovalTraceQueryV1.Session
    $replacement.EventsLost=0;$replacement.LogFileName='D:\foreign\lossless.etl'
    Reject {$o.CaptureFinal($replacement)}
    Check ([Object]::ReferenceEquals($r.Snapshot,$original) -and $r.Snapshot.EventsLost -eq 8) 'Established STOP payload replaced.'
}
Case 'retained-owner-does-not-expose-native-provenance-mutators' {
    $b=New-Fixture;$script:mock.StartStatus=183;$o=New-Owner $b;Start-RemovalNativeTrace $o|Out-Null
    foreach($method in @('BeginStart','CaptureStart','BeginIdle','CaptureIdle','BeginEnable','CaptureEnable',
        'BeginStopQuery','CaptureStopQuery','BeginStop','CaptureStop','CaptureFinal')){
        Check ($null -eq $o.GetType().GetMethod($method)) ('Public provenance mutation API remains: '+$method)
    }
    Reject {$o.CaptureStart([UInt32]0,[UInt64]701)}
    Check (-not $o.TraceOwned -and $o.StartStatus -eq 183 -and $o.Failed) 'Failed Start provenance could be upgraded.'
}
Case 'dispatcher-final-capture-is-stage-bound-one-shot-and-detached-from-decoded-session' {
    foreach($status in @([UInt32]0,[UInt32]234,[UInt32]2402)){
        $bridge=[Moiren.RemovalTraceNativeV1.Bridge]::new('MoirenRemoval613diagnostic','D:\owned\removal.etl',$null)
        $o=$bridge.Owner
        $decoded=New-Object Moiren.RemovalTraceQueryV1.Session
        $decoded.SessionName=$o.Session;$decoded.LogFileName=$o.EtlPath;$decoded.SessionGuid=$o.SessionGuid
        $decoded.Handle=[UInt64]701;$decoded.ClientContext=1;$decoded.LogFileMode=1;$decoded.MaximumFileSize=16
        $decoded.EventsLost=3;$decoded.LogBuffersLost=0;$decoded.RealTimeBuffersLost=0
        Reject {$bridge.CaptureFinal($decoded)}
        $bridge.BeginStart();$bridge.CaptureStart([UInt32]0,[UInt64]701)
        Reject {$bridge.CaptureStart([UInt32]183,[UInt64]702)}
        $bridge.BeginStopQuery();$bridge.CaptureStopQuery([UInt32]0);$bridge.CaptureStopQuerySnapshot($decoded)
        Reject {$bridge.CaptureFinal($decoded)}
        $bridge.BeginStop();$bridge.CaptureStop($status)
        if($status -eq 0){
            $bridge.CaptureFinal($decoded);$captured=$o.StopReceipt.Snapshot
            $decoded.EventsLost=0;$decoded.LogFileName='D:\foreign\lossless.etl'
            Check ($captured.EventsLost -eq 3 -and $captured.LogFileName -ceq $o.EtlPath) 'STOP proof aliases decoded mutable session.'
            Reject {$bridge.CaptureFinal($decoded)}
            Reject {$bridge.CaptureStop([UInt32]234)}
            Check ([Object]::ReferenceEquals($captured,$o.StopReceipt.Snapshot) -and $o.StopReceipt.Status -eq 0) 'Captured payload/status changed.'
        } else {
            Reject {$bridge.CaptureFinal($decoded)}
            Check (-not $o.StopReceipt.FinalCountersKnown -and $null -eq $o.StopReceipt.Snapshot) 'Nonzero STOP status acquired final counters.'
        }
    }
}
Case 'default-native-start-refuses-before-attempt-or-import' {
    $o=New-RemovalNativeTrace -Session 'MoirenRemoval613diagnostic' -EtlPath 'D:\owned\removal.etl'
    Reject {Start-RemovalNativeTrace $o}
    Check (-not $o.StartAttempted -and -not $o.TraceOwned -and $o.Handle -eq 0) 'Native guard ran after mutation/dispatch.'
    $source=[IO.File]::ReadAllText($library)
    Check (-not $source.Contains('CloseTrace(')) 'Controller incorrectly closes consumer handle.'
    Check ($source.Contains('StartTraceW') -and $source.Contains('EnableTraceEx2') -and $source.Contains('ControlTraceW')) 'Actual APIs missing.'
}
Case 'second-start-admission-refusal-retains-known-no-native-dispatch' {
    $o=New-RemovalNativeTrace -Session 'MoirenRemoval613diagnostic' -EtlPath 'D:\owned\removal.etl'
    function Assert-RemovalTraceNativeOwnerApproval($Owner,[string]$Operation){
        if($Operation-cne'TraceStart'){throw 'Fresh admission expired before StartTrace.'}
    }
    $null=Start-RemovalNativeTrace $o
    Check ($o.StartAttempted-and$o.Failed-and-not$o.TraceOwned-and$null-eq$o.StartStatus) 'Predispatch refusal fabricated native status.'
    Check ($null-ne$o.PSObject.Properties['StartDispatchAttempted']-and-not$o.StartDispatchAttempted) 'Known no-dispatch was not retained separately from unknown native Start.'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not $_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ETW_CALLS=0')
if($failed){exit 1}
