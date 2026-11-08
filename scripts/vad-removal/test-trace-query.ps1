#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
function Reject([scriptblock]$Action){
    $refused=$false
    try {&$Action|Out-Null} catch {$refused=$true}
    Check $refused 'Expected refusal.'
}
function Case([string]$Name,[scriptblock]$Action){
    try {&$Action;$checks.Add(@{name=$Name;passed=$true})}
    catch {$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}
}
$library=Join-Path $PSScriptRoot 'trace-query.ps1'
Case 'query-definitions-exist-without-native-dispatch' {
    Check (Test-Path -LiteralPath $library) 'Trace query helper is not implemented.'
    . $library
    Check ($null -ne (Get-Command Get-RemovalTraceQuerySnapshot -ErrorAction SilentlyContinue)) 'Query entry missing.'
}
if(@($checks|Where-Object{-not $_.passed}).Count){
    $checks|ConvertTo-Json -Depth 6
    Write-Output ('CHECKS='+$checks.Count+' FAILED=1 ETW_QUERIES=0')
    exit 1
}
. $library
Initialize-RemovalTraceQuery
function New-Fixture {
    $script:mock=@{
        Name='MoirenRemoval613mock';Guid=[Guid]'0d77d391-7678-483a-a008-a28b90c22d2d'
        Provider=[Guid]'7dc75ba9-8274-4eae-a42d-b4c74e0209d2';Handle=[UInt64]701;LoggerId=[UInt32]17
        Path='D:\owned\removal.etl';Clock=[UInt32]1;Mode=[UInt32]1;Max=[UInt32]16
        QueryStatuses=@([UInt32]0,[UInt32]0,[UInt32]0);ProviderStatus=[UInt32]0
        Returned=[UInt32]1;Rows=1;Enabled=[byte]1;Level=[UInt32]4;Flags=[UInt32]15
        Lost=[UInt32]3;LogLost=[UInt32]4;RealtimeLost=[UInt32]5
        Queries=New-Object Collections.Generic.List[UInt64];Enumerations=0
        Mutate=$null
    }
    @{
        Query={
            param([UInt64]$Id,[IntPtr]$Properties)
            $script:mock.Queries.Add($Id)
            $n=$script:mock.Queries.Count
            if($script:mock.QueryStatuses[$n-1] -ne 0){return [UInt32]$script:mock.QueryStatuses[$n-1]}
            $v=New-Object Moiren.RemovalTraceQueryV1.Properties
            $w=New-Object Moiren.RemovalTraceQueryV1.Wnode
            $w.BufferSize=[Moiren.RemovalTraceQueryV1.Reader]::BufferBytes
            $w.HistoricalContext=$script:mock.Handle;$w.Guid=$script:mock.Guid;$w.ClientContext=$script:mock.Clock
            $v.Wnode=$w;$v.MaximumFileSize=$script:mock.Max;$v.LogFileMode=$script:mock.Mode
            $v.EventsLost=$script:mock.Lost;$v.LogBuffersLost=$script:mock.LogLost;$v.RealTimeBuffersLost=$script:mock.RealtimeLost
            $v.LoggerNameOffset=[Moiren.RemovalTraceQueryV1.Reader]::PropertiesSize
            $v.LogFileNameOffset=$v.LoggerNameOffset+2048
            $name=[Text.Encoding]::Unicode.GetBytes($script:mock.Name+[char]0)
            $path=[Text.Encoding]::Unicode.GetBytes($script:mock.Path+[char]0)
            [Runtime.InteropServices.Marshal]::Copy($name,0,[IntPtr]::Add($Properties,[int]$v.LoggerNameOffset),$name.Length)
            [Runtime.InteropServices.Marshal]::Copy($path,0,[IntPtr]::Add($Properties,[int]$v.LogFileNameOffset),$path.Length)
            [Runtime.InteropServices.Marshal]::StructureToPtr($v,$Properties,$false)
            if($null -ne $script:mock.Mutate){&$script:mock.Mutate $n $Properties}
            return [UInt32]0
        }
        Enumerate={
            param([IntPtr]$Pointers,[UInt32]$Capacity,[IntPtr]$Returned)
            $script:mock.Enumerations++
            Check ($Capacity -eq 4096) 'Provider slab cap changed.'
            for($i=0;$i -lt $script:mock.Rows;$i++){
                $row=New-Object Moiren.RemovalTraceQueryV1.GuidProperties
                $row.Guid=$script:mock.Provider;$row.LoggerId=$script:mock.LoggerId
                $row.EnableLevel=$script:mock.Level;$row.EnableFlags=$script:mock.Flags;$row.IsEnable=$script:mock.Enabled
                $p=[Runtime.InteropServices.Marshal]::ReadIntPtr($Pointers,$i*[IntPtr]::Size)
                [Runtime.InteropServices.Marshal]::StructureToPtr($row,$p,$false)
            }
            [Runtime.InteropServices.Marshal]::WriteInt32($Returned,[int]$script:mock.Returned)
            return [UInt32]$script:mock.ProviderStatus
        }
    }
}
function Read-Fixture($Boundary) {
    Get-RemovalTraceQuerySnapshot -SessionName 'MoirenRemoval613mock' -SessionGuid ([Guid]'0d77d391-7678-483a-a008-a28b90c22d2d') `
        -Handle 701 -QueryBoundary $Boundary
}
function Change-Properties([IntPtr]$Pointer,[hashtable]$Changes) {
    $v=[Runtime.InteropServices.Marshal]::PtrToStructure($Pointer,[type][Moiren.RemovalTraceQueryV1.Properties])
    foreach($key in $Changes.Keys){$v.$key=$Changes[$key]}
    [Runtime.InteropServices.Marshal]::StructureToPtr($v,$Pointer,$false)
}
Case 'sdk-layouts-have-correct-pointer-sized-properties-and-byte-boolean' {
    Check ([Runtime.InteropServices.Marshal]::SizeOf([type][Moiren.RemovalTraceQueryV1.Wnode]) -eq 48) 'WNODE size differs.'
    Check ([Moiren.RemovalTraceQueryV1.Reader]::PropertiesSize -eq 120) 'x64 properties size differs.'
    Check ([Moiren.RemovalTraceQueryV1.Reader]::GuidPropertiesSize -eq 36) 'GUID properties size differs.'
    Check ([Runtime.InteropServices.Marshal]::OffsetOf([type][Moiren.RemovalTraceQueryV1.Properties],'LoggerThreadId').ToInt32() -eq 104) 'LoggerThreadId offset differs.'
    Check ([Runtime.InteropServices.Marshal]::OffsetOf([type][Moiren.RemovalTraceQueryV1.Properties],'LoggerNameOffset').ToInt32() -eq 116) 'LoggerNameOffset differs.'
    Check ([Runtime.InteropServices.Marshal]::OffsetOf([type][Moiren.RemovalTraceQueryV1.GuidProperties],'IsEnable').ToInt32() -eq 32) 'BOOLEAN layout differs.'
}
Case 'actual-buffer-path-returns-exact-owned-session-and-classic-association' {
    $b=New-Fixture;$r=Read-Fixture $b
    Check ($r.Complete -and $r.QueryStatus -eq 0 -and $r.ProviderQueryStatus -eq 0 -and $r.AssociationQueryStatus -eq 0 -and $r.FinalQueryStatus -eq 0) ('Unexpected incomplete snapshot: '+$r.Error)
    Check ($r.SchemaVersion -eq 1 -and $r.SessionName -ceq $script:mock.Name -and $r.SessionGuid -eq $script:mock.Guid -and $r.Handle -eq 701 -and $r.LoggerId -eq 17) 'Session identity was not decoded.'
    Check ($r.LogFileName -ceq $script:mock.Path -and $r.ClientContext -eq 1 -and $r.LogFileMode -eq 1 -and $r.MaximumFileSize -eq 16) 'Configuration was not decoded.'
    Check ($r.Provider.Guid -eq $script:mock.Provider -and $r.Provider.Enabled -is [bool] -and $r.Provider.Enabled -and $r.Provider.LoggerId -eq 17 -and $r.Provider.Level -eq 4 -and $r.Provider.EnableFlags -eq 15 -and $r.Provider.Kind -ceq 'ClassicWpp') 'Classic provider was not decoded.'
    Check ($null -eq $r.Provider.MatchAnyKeyword -and $null -eq $r.Provider.MatchAllKeyword) 'Invented modern keyword evidence.'
    Check (($script:mock.Queries -join ',') -ceq '701,17,701' -and $script:mock.Enumerations -eq 1) 'Association was not queried or retry occurred.'
}
Case 'actual-native-buffer-query-composes-with-readback-policy' {
    . (Join-Path $PSScriptRoot 'trace-readback.ps1')
    $b=New-Fixture;$r=Read-Fixture $b
    Check ($r.Complete -and $r.Provider.Level -is [uint32]) 'Actual query did not produce the native uint32 provider level.'
    $trace=[pscustomobject]@{
        Session=$script:mock.Name;EtlPath=$script:mock.Path
        TraceOwned=$true;TraceStopped=$false;OwnershipBasis='StartTraceHandleGuid'
        Handle=$script:mock.Handle;SessionGuid=$script:mock.Guid;StartStatus=[uint32]0;EnableStatus=[uint32]0
        SessionReadbackConfirmed=$false;RawTraceLoss='unknown';Failed=$false
        Errors=New-Object Collections.Generic.List[string]
    }
    $owner=New-RemovalTraceReadback -Trace $trace
    Confirm-RemovalTraceReadback $owner $r
    Check ((Test-RemovalTraceCaptureAdmission $owner) -and $trace.SessionReadbackConfirmed) 'Actual typed query snapshot was refused by readback policy.'
    Check ($owner.Initial.Provider.Level -is [uint32] -and $owner.Initial.Provider.Level -eq 4) 'Readback copy changed the native level type/value.'
    Check ($owner.Initial.EventsLost -eq 3 -and $owner.Initial.LogBuffersLost -eq 4 -and $owner.Initial.RealTimeBuffersLost -eq 5) 'Initial raw loss counters were normalized.'
    Check ($trace.RawTraceLoss -ceq 'unknown' -and -not $owner.FinalCountersKnown -and -not $owner.Lossless) 'Initial readback was promoted to final/lossless evidence.'
    Check (($script:mock.Queries -join ',') -ceq '701,17,701' -and $script:mock.Enumerations -eq 1) 'Readback policy performed extra native operations.'
}
Case 'classic-ulong-session-handle-is-queried-full-width-without-bitfield-inference' {
    $b=New-Fixture;$script:mock.LoggerId=[uint32]0x12340017;$r=Read-Fixture $b
    Check ($r.Complete-and$r.LoggerId-eq[uint32]0x12340017) 'Documented ULONG association was truncated or refused'
    Check ($script:mock.Queries[1]-is[uint64]-and$script:mock.Queries[1]-eq[uint64]0x12340017) 'Association did not query full zero-extended ULONG session handle'
}
Case 'query-input-initializes-session-guid-without-seeding-output-handle-or-counters' {
    $b=New-Fixture
    $script:queryInputGuids=New-Object Collections.Generic.List[Guid]
    $script:originalQuery=$b.Query
    $b.Query={
        param([UInt64]$Id,[IntPtr]$Properties)
        $v=[Runtime.InteropServices.Marshal]::PtrToStructure($Properties,[type][Moiren.RemovalTraceQueryV1.Properties])
        $script:queryInputGuids.Add($v.Wnode.Guid)
        Check ($v.Wnode.HistoricalContext -eq 0 -and $v.EventsLost -eq 0 -and $v.LogBuffersLost -eq 0 -and $v.RealTimeBuffersLost -eq 0) 'Expected output was pre-populated.'
        &$script:originalQuery $Id $Properties
    }
    $r=Read-Fixture $b
    Check $r.Complete ('Query input contract failed: '+$r.Error)
    Check ($script:queryInputGuids.Count -eq 3) 'Missing query input capture.'
    foreach($guid in $script:queryInputGuids){
        Check ($guid -eq $script:mock.Guid) 'ControlTrace QUERY input did not initialize the owned session GUID.'
    }
}
Case 'loss-counters-and-raw-config-are-not-normalized-to-gates' {
    $b=New-Fixture;$script:mock.Clock=2;$script:mock.Mode=2;$script:mock.Max=17
    $script:mock.Level=5;$script:mock.Flags=31;$script:mock.Lost=[UInt32]::MaxValue
    $r=Read-Fixture $b
    Check ($r.Complete -and $r.ClientContext -eq 2 -and $r.LogFileMode -eq 2 -and $r.MaximumFileSize -eq 17 -and $r.Provider.Level -eq 5 -and $r.Provider.EnableFlags -eq 31) 'Reader concealed nonmatching raw config.'
    Check ($r.EventsLost -eq [UInt32]::MaxValue -and $r.LogBuffersLost -eq 4 -and $r.RealTimeBuffersLost -eq 5) 'Raw loss evidence changed.'
}
Case 'first-query-native-error-does-not-enumerate-or-invent-loss-zeroes' {
    $b=New-Fixture;$script:mock.QueryStatuses[0]=[UInt32]5;$r=Read-Fixture $b
    Check (-not $r.Complete -and $r.QueryStatus -eq 5 -and $null -eq $r.EventsLost -and $null -eq $r.ProviderQueryStatus) 'First query error converted to success/zero.'
    Check ($script:mock.Queries.Count -eq 1 -and $script:mock.Enumerations -eq 0) 'Retried failed query.'
}
Case 'provider-native-error-and-truncation-never-prove-empty-or-enabled' {
    foreach($status in @([UInt32]50,[UInt32]234,[UInt32]5)){
        $b=New-Fixture;$script:mock.ProviderStatus=$status;$r=Read-Fixture $b
        Check (-not $r.Complete -and $r.QueryStatus -eq 0 -and $r.ProviderQueryStatus -eq $status -and $null -eq $r.Provider) 'Provider error promoted to complete.'
        Check ($script:mock.Queries.Count -eq 1 -and $script:mock.Enumerations -eq 1) 'Provider query retried.'
    }
    $b=New-Fixture;$script:mock.Returned=4097;$r=Read-Fixture $b
    Check (-not $r.Complete) 'Oversized successful provider count accepted.'
}
Case 'missing-disabled-ambiguous-or-wrong-provider-is-not-a-binding' {
    foreach($mode in @('missing','disabled','duplicate','wrong','badboolean','badlevel','zerologger')){
        $b=New-Fixture
        switch($mode){
            'missing' {$script:mock.Returned=0;$script:mock.Rows=0}
            'disabled' {$script:mock.Enabled=[byte]0}
            'duplicate' {$script:mock.Returned=2;$script:mock.Rows=2}
            'wrong' {$script:mock.Provider=[Guid]::NewGuid()}
            'badboolean' {$script:mock.Enabled=[byte]2}
            'badlevel' {$script:mock.Level=256}
            'zerologger' {$script:mock.LoggerId=0}
        }
        $r=Read-Fixture $b
        Check (-not $r.Complete -and $r.ProviderQueryStatus -eq 0 -and $null -eq $r.AssociationQueryStatus) ('Invalid classic binding accepted: '+$mode)
    }
}
Case 'foreign-session-handle-guid-name-never-matches-by-caller-input' {
    foreach($mode in @('handle','guid','name')){
        $b=New-Fixture
        switch($mode){'handle'{$script:mock.Handle=702};'guid'{$script:mock.Guid=[Guid]::NewGuid()};'name'{$script:mock.Name='MoirenRemoval613foreign'}}
        $r=Read-Fixture $b
        Check (-not $r.Complete -and $r.QueryStatus -eq 0 -and $script:mock.Enumerations -eq 0) ('Foreign session accepted: '+$mode)
    }
}
Case 'provider-logger-must-query-back-to-the-same-owned-session' {
    $b=New-Fixture;$script:mock.Mutate={param($n,$p)if($n -eq 2){$v=[Runtime.InteropServices.Marshal]::PtrToStructure($p,[type][Moiren.RemovalTraceQueryV1.Properties]);$w=$v.Wnode;$w.Guid=[Guid]::NewGuid();$v.Wnode=$w;[Runtime.InteropServices.Marshal]::StructureToPtr($v,$p,$false)}}
    $r=Read-Fixture $b
    Check (-not $r.Complete -and $r.AssociationQueryStatus -eq 0 -and $null -eq $r.FinalQueryStatus) 'Foreign LoggerId was associated by low-bit/ordinal guess.'
}
Case 'association-or-final-api-error-stays-incomplete' {
    foreach($n in @(1,2)){
        $b=New-Fixture;$script:mock.QueryStatuses[$n]=[UInt32]4201;$r=Read-Fixture $b
        Check (-not $r.Complete) 'Stopped/missing session promoted to complete.'
        if($n -eq 1){Check ($r.AssociationQueryStatus -eq 4201 -and $null -eq $r.FinalQueryStatus) 'Association status lost.'}
        else {Check ($r.FinalQueryStatus -eq 4201) 'Final status lost.'}
    }
}
Case 'final-requery-rejects-config-change-and-counter-regression' {
    foreach($mode in @('config','counter')){
        $b=New-Fixture
        if($mode -eq 'config'){$script:mock.Mutate={param($n,$p)if($n -eq 3){Change-Properties $p @{MaximumFileSize=[UInt32]999}}}}
        else {$script:mock.Mutate={param($n,$p)if($n -eq 3){Change-Properties $p @{EventsLost=[UInt32]2}}}}
        $r=Read-Fixture $b
        Check (-not $r.Complete -and $r.FinalQueryStatus -eq 0) 'Changing trace identity/counters accepted.'
    }
    $b=New-Fixture;$script:mock.Mutate={param($n,$p)if($n -eq 3){Change-Properties $p @{EventsLost=[UInt32]6}}}
    $r=Read-Fixture $b;Check ($r.Complete -and $r.EventsLost -eq 6) 'Monotonic latest loss not retained.'
}
Case 'malformed-successful-properties-preserve-api-zero-without-complete' {
    foreach($mode in @('offset','odd','length','unterminated','utf16','emptyguid','zerohandle')){
        $b=New-Fixture
        $script:mock.Mutate={
            param($n,$p)
            if($mode -eq 'offset'){Change-Properties $p @{LoggerNameOffset=[UInt32]4}}
            elseif($mode -eq 'odd'){Change-Properties $p @{LoggerNameOffset=[UInt32]121}}
            elseif($mode -eq 'length'){$v=[Runtime.InteropServices.Marshal]::PtrToStructure($p,[type][Moiren.RemovalTraceQueryV1.Properties]);$w=$v.Wnode;$w.BufferSize=50000;$v.Wnode=$w;[Runtime.InteropServices.Marshal]::StructureToPtr($v,$p,$false)}
            elseif($mode -eq 'unterminated'){for($j=0;$j -lt 1024;$j++){[Runtime.InteropServices.Marshal]::WriteInt16($p,120+$j*2,65)}}
            elseif($mode -eq 'utf16'){[Runtime.InteropServices.Marshal]::WriteInt16($p,120,[int16]-10240);[Runtime.InteropServices.Marshal]::WriteInt16($p,122,[int16]0)}
            else {$v=[Runtime.InteropServices.Marshal]::PtrToStructure($p,[type][Moiren.RemovalTraceQueryV1.Properties]);$w=$v.Wnode;if($mode -eq 'emptyguid'){$w.Guid=[Guid]::Empty}else{$w.HistoricalContext=0};$v.Wnode=$w;[Runtime.InteropServices.Marshal]::StructureToPtr($v,$p,$false)}
        }
        $r=Read-Fixture $b
        Check (-not $r.Complete -and $r.QueryStatus -eq 0 -and $r.Error) ('Malformed successful API result was complete or raw status lost: '+$mode+' complete='+$r.Complete+' status='+$r.QueryStatus+' error='+$r.Error)
    }
}
Case 'both-mocked-boundaries-are-required-with-no-native-fallback' {
    Reject {Get-RemovalTraceQuerySnapshot -SessionName 'MoirenRemoval613mock' -SessionGuid ([Guid]::NewGuid()) -Handle 701 -QueryBoundary @{Query={throw 'not used'}}}
    Reject {Get-RemovalTraceQuerySnapshot -SessionName 'wrong-name' -SessionGuid ([Guid]::NewGuid()) -Handle 701 -QueryBoundary @{Query={};Enumerate={}}}
    Reject {Get-RemovalTraceQuerySnapshot -SessionName 'MoirenRemoval613mock' -SessionGuid ([Guid]::Empty) -Handle 701 -QueryBoundary @{Query={};Enumerate={}}}
}
Case 'stop-return-buffer-decoder-needs-no-live-session-or-provider-query' {
    $b=New-Fixture
    $p=[Runtime.InteropServices.Marshal]::AllocHGlobal([Moiren.RemovalTraceQueryV1.Reader]::BufferBytes)
    try {
        $zero=New-Object byte[] ([Moiren.RemovalTraceQueryV1.Reader]::BufferBytes)
        [Runtime.InteropServices.Marshal]::Copy($zero,0,$p,$zero.Length)
        $status=&$b.Query ([UInt64]701) $p
        Check ($status -eq 0) 'Mock STOP properties not populated.'
        Change-Properties $p @{EventsLost=[UInt32]8;LogBuffersLost=[UInt32]9;RealTimeBuffersLost=[UInt32]10}
        # Decode only: this path can consume a successful STOP's returned buffer
        # while it is still allocated. It neither calls STOP nor re-queries it.
        $r=[Moiren.RemovalTraceQueryV1.Reader]::DecodeSession($p,[Moiren.RemovalTraceQueryV1.Reader]::BufferBytes)
        Check ($r.Handle -eq 701 -and $r.SessionGuid -eq $script:mock.Guid -and $r.EventsLost -eq 8 -and $r.LogBuffersLost -eq 9 -and $r.RealTimeBuffersLost -eq 10) 'Final raw counter fields were lost.'
        Check ($script:mock.Queries.Count -eq 1 -and $script:mock.Enumerations -eq 0) 'Decoder queried a stopped session/provider.'
    } finally {[Runtime.InteropServices.Marshal]::FreeHGlobal($p)}
}
Case 'exceptions-in-query-or-enumeration-remain-incomplete-without-retry' {
    $b=New-Fixture;$b.Query={param($id,$p)throw 'mock query failure'};$r=Read-Fixture $b
    Check (-not $r.Complete -and $null -eq $r.QueryStatus -and $r.Error) 'Throwing native boundary invented query status.'
    $b=New-Fixture;$b.Enumerate={param($p,$n,$count)throw 'mock enumeration failure'};$r=Read-Fixture $b
    Check (-not $r.Complete -and $r.QueryStatus -eq 0 -and $null -eq $r.ProviderQueryStatus -and $r.Error) 'Throwing enumeration invented provider status.'
    Check ($script:mock.Queries.Count -eq 1) 'Retried after ambiguous enumeration.'
}
Case 'native-imports-are-query-only-and-tests-do-not-invoke-them' {
    $source=[IO.File]::ReadAllText($library)
    $imports=[regex]::Matches($source,'private static extern uint ([A-Za-z0-9]+)\(')
    Check ($imports.Count -eq 2 -and $imports[0].Groups[1].Value -ceq 'ControlTrace' -and $imports[1].Groups[1].Value -ceq 'EnumerateTraceGuids') 'Unexpected native import surface.'
    Check ($source.Contains('return ControlTrace(id,null,p,0);')) 'Native query boundary is not fixed QUERY.'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not $_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ETW_QUERIES=0')
if($failed){exit 1}
