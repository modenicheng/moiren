#requires -Version 5.1
param([string]$Library)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if(-not $Library){$Library=Join-Path $PSScriptRoot 'trace-boundary.ps1'}
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
function Reject([scriptblock]$Action){
    $refused=$false
    try{&$Action|Out-Null}catch{$refused=$true}
    Check $refused 'Expected refusal.'
}
function Case([string]$Name,[scriptblock]$Action){
    try{&$Action;$checks.Add(@{name=$Name;passed=$true})}
    catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}
}
Case 'definitions-exist-and-load-with-no-tool-dispatch' {
    Check (Test-Path -LiteralPath $Library) 'Trace boundary definitions not implemented.'
    $script:loadLaunches=0
    function Start-FiniteTool {$script:loadLaunches++;throw 'Unexpected tool dispatch.'}
    . $Library
    Check ($script:loadLaunches -eq 0) 'Dot-source dispatched a tool.'
}
if(@($checks|Where-Object{-not $_.passed}).Count){
    $checks|ConvertTo-Json -Depth 6
    Write-Output ('CHECKS='+$checks.Count+' FAILED=1')
    exit 1
}
. $Library

# Only fake tool boundaries are used. No logman/tracelog, files or ETW are opened.
$script:mock=$null
function FakeReceipt($Owner) {
    [pscustomobject]@{
        Owner=$Owner;ProcessId=$Owner.ProcessId;CreationUtcTicks=$Owner.CreationUtcTicks
        ExitedConfirmed=$true;ExitCode=0;TimedOut=$false;NeedsObservation=$false
        OutputStable=$true;LogLimitExceeded=$false;Success=$true;Errors=@()
        StdoutPath=($Owner.Step+'.stdout.log');StderrPath=($Owner.Step+'.stderr.log')
    }
}
function NewFixture {
    $script:mock=@{
        Started=New-Object Collections.Generic.List[object]
        Observed=New-Object Collections.Generic.List[object]
        NextPid=100;ThrowStart=$null;ThrowObserve=$null;ThrowRead=$false
        Mutate=$null;Inventory=[Text.Encoding]::UTF8.GetBytes("Data Collector Set                      Type                          Status`r`n-------------------------------------------------------------------------------`r`nOtherSession                            Trace                         Running`r`nThe command completed successfully.`r`n")
    }
    $boundary=@{
        Start={
            param($Request)
            $script:mock.Started.Add($Request)
            if($script:mock.ThrowStart -ceq $Request.Step){throw 'Synthetic dispatch failure.'}
            $script:mock.NextPid++
            [pscustomobject]@{
                Step=$Request.Step;Closed=$false;IdentityConfirmed=$true
                ProcessId=$script:mock.NextPid;CreationUtcTicks=[long]1234567
            }
        }
        Observe={
            param($Owner,$Trace)
            Check ([object]::ReferenceEquals($Trace.ToolOwners[$Trace.ToolOwners.Count-1],$Owner)) 'Owner not retained before observation.'
            $script:mock.Observed.Add($Owner)
            if($script:mock.ThrowObserve -ceq $Owner.Step){throw 'Synthetic observation failure.'}
            $r=FakeReceipt $Owner
            if($null -ne $script:mock.Mutate){&$script:mock.Mutate $r $Owner.Step}
            return $r
        }
        Read={
            param($Receipt)
            if($script:mock.ThrowRead){throw 'Synthetic output read failure.'}
            return ,$script:mock.Inventory
        }
    }
    New-RemovalTraceBoundary -Session 'MoirenRemoval613Unit_01' `
        -EtlPath 'D:\MoirenTest\trace-unit-only\trace.etl' -EvidenceDirectory 'D:\MoirenTest\trace-unit-only' `
        -LogmanPath 'C:\Windows\System32\logman.exe' -LogmanSha256 ('1'*64) `
        -TracelogPath 'D:\MoirenTest\tools\tracelog.exe' -TracelogSha256 ('2'*64) -ToolBoundary $boundary
}
function Inventory($Trace){$r=Invoke-RemovalTraceStep $Trace inventory;Check $r.Success 'Inventory did not succeed.'}
function Own($Trace){Inventory $Trace;$r=Invoke-RemovalTraceStep $Trace start;Check ($r.Success -and $Trace.TraceOwned) 'Start did not confirm ownership.'}

Case 'happy-path-exact-argv-fixed-size-provider-clock-and-raw-loss' {
    $t=NewFixture;Own $t
    $loss=Invoke-RemovalTraceStep $t loss
    $stop=Invoke-RemovalTraceStep $t stop
    Check ($loss.Success -and $stop.Success -and $t.TraceStopped -and -not $t.Failed) 'Trace lifecycle failed.'
    Check ($script:mock.Started.Count -eq 4 -and $t.ToolOwners.Count -eq 4) 'Not exactly four tool launches/owners.'
    $inv=$script:mock.Started[0];$start=$script:mock.Started[1];$query=$script:mock.Started[2];$end=$script:mock.Started[3]
    Check (($inv.ArgumentList -join '|') -ceq 'query|-ets') 'Inventory is not exhaustive active-session query.'
    Check (($start.ArgumentList -join '|') -ceq ('create|trace|'+$t.Session+'|-o|'+$t.EtlPath+'|-p|{7DC75BA9-8274-4EAE-A42D-B4C74E0209D2}|0xf|4|-f|bin|-max|16|-ct|perf|-ets')) 'Fixed create parameters changed.'
    Check (($query.ArgumentList -join '|') -ceq ('-q|'+$t.Session)) 'Loss query is not exact-session.'
    Check (($end.ArgumentList -join '|') -ceq ('stop|'+$t.Session+'|-ets')) 'Stop is not exact-session.'
    Check ($start.Executable -ceq $t.LogmanPath -and $start.ExpectedSha256 -ceq ('1'*64) -and
        $query.Executable -ceq $t.TracelogPath -and $query.ExpectedSha256 -ceq ('2'*64)) 'Wrong tool pins.'
    foreach($request in $script:mock.Started){
        Check ($request.TimeoutSeconds -eq 15 -and $request.MaxLogBytes -eq 1048576) 'Tool deadline/log cap changed.'
    }
    Check ($t.RawTraceLoss -ceq 'unknown' -and -not $t.Qualified -and -not $t.NativeApproved) 'Invented trace qualification/loss.'
    Check ($t.Steps[2].ToolReceipt.StdoutPath -ceq 'loss.stdout.log') 'Raw loss paths not retained.'
}
Case 'inventory-blocks-only-exact-session-case-insensitively-not-prefix' {
    foreach($name in @('MoirenRemoval613Unit_01','moirenremoval613unit_01')){
        $t=NewFixture;$script:mock.Inventory=[Text.Encoding]::UTF8.GetBytes(" $name Trace Running`r`n")
        $r=Invoke-RemovalTraceStep $t inventory
        Check (-not $r.Success -and $r.Sessions.Count -eq 1 -and $t.Failed) 'Existing exact session not refused.'
        Reject {Invoke-RemovalTraceStep $t start}
        Check ($script:mock.Started.Count -eq 1 -and -not $t.TraceOwned) 'Existing session adopted.'
    }
    $t=NewFixture;$script:mock.Inventory=[Text.Encoding]::UTF8.GetBytes("Data Collector Set                      Type                          Status`r`n-------------------------------------------------------------------------------`r`nMoirenRemoval613Unit_010                 Trace                         Running`r`nThe command completed successfully.`r`n")
    Own $t
}
Case 'localized-and-bom-inventory-preserves-exact-ascii-identity' {
    foreach($encoding in @([Text.Encoding]::UTF8,[Text.Encoding]::Unicode,[Text.Encoding]::BigEndianUnicode)){
        $t=NewFixture
        $bytes=$encoding.GetPreamble()+$encoding.GetBytes(([char]0x4e2d).ToString()+" MoirenRemoval613Unit_01 Trace`r`n")
        $script:mock.Inventory=[byte[]]$bytes
        $r=Invoke-RemovalTraceStep $t inventory
        Check (-not $r.Success -and $r.Sessions.Count -eq 1) 'Localized/BOM inventory missed exact session.'
    }
}
Case 'empty-oversize-invalid-inventory-or-read-failure-cannot-authorize-start' {
    foreach($bytes in @([byte[]]@(),[byte[]](0,0),[byte[]](255,254,65),[byte[]](32,32))){
        $t=NewFixture;$script:mock.Inventory=$bytes
        $r=Invoke-RemovalTraceStep $t inventory
        Check (-not $r.Success -and $t.Failed) 'Ambiguous inventory accepted.'
        Reject {Invoke-RemovalTraceStep $t start}
    }
    $t=NewFixture;$script:mock.Inventory=[byte[]]::new(1048577)
    $r=Invoke-RemovalTraceStep $t inventory;Check (-not $r.Success) 'Over-cap inventory accepted.'
    $t=NewFixture;$script:mock.ThrowRead=$true
    $r=Invoke-RemovalTraceStep $t inventory;Check (-not $r.Success -and $t.ToolOwners.Count -eq 1) 'Read failure dropped owner.'
}
Case 'successful-garbage-truncated-table-and-unrecognized-locale-never-prove-absence' {
    foreach($text in @(
        'arbitrary successful garbage',
        "Data Collector Set                      Type                          Status`r`n-------------------------------------------------------------------------------`r`nOtherSession                            Trace                         Running`r`n",
        "Data Collector Set                      Type                          Status`r`n-------------------------------------------------------------------------------`r`nOtherSession                            Trace                         Running`r`nThe command completed successfully.`r`nunparsed tail",
        "Data Collector Set                      Type                          Status`r`n-------------------------------------------------------------------------------`r`ntruncated row`r`nThe command completed successfully.",
        "Unreviewed localized table`r`nOtherSession Trace Running`r`n"
    )){
        $t=NewFixture;$script:mock.Inventory=[Text.Encoding]::UTF8.GetBytes($text)
        $r=Invoke-RemovalTraceStep $t inventory
        Check (-not $r.Success -and -not $t.InventoryConfirmed -and $t.Failed) 'Incomplete/unreviewed inventory proved absence.'
        Reject {Invoke-RemovalTraceStep $t start}
        Check ($script:mock.Started.Count -eq 1) 'Invalid inventory authorized create.'
    }
}
Case 'complete-reviewed-table-retains-format-and-all-parsed-session-count' {
    $t=NewFixture
    $r=Invoke-RemovalTraceStep $t inventory
    Check ($r.Success -and $r.InventoryFormat -ceq 'EnglishLogmanQueryV1' -and $r.InventorySessionCount -eq 1) 'Inventory completeness provenance missing.'
}
Case 'start-needs-inventory-and-each-stage-is-one-attempt' {
    $t=NewFixture;Reject {Invoke-RemovalTraceStep $t start};Check ($script:mock.Started.Count -eq 0) 'Start bypassed inventory.'
    $t=NewFixture;Own $t
    Reject {Invoke-RemovalTraceStep $t inventory};Reject {Invoke-RemovalTraceStep $t start}
    $null=Invoke-RemovalTraceStep $t loss;Reject {Invoke-RemovalTraceStep $t loss}
    $null=Invoke-RemovalTraceStep $t stop;Reject {Invoke-RemovalTraceStep $t stop}
    Check ($script:mock.Started.Count -eq 4 -and $t.TraceStopped -and $t.Failed) 'Retry occurred or misuse not sticky.'
}
Case 'create-nonzero-timeout-output-failure-or-malformed-success-never-owns' {
    foreach($mutation in @(
        {$args[0].ExitCode=1},{$args[0].TimedOut=$true},{$args[0].OutputStable=$false},
        {$args[0].LogLimitExceeded=$true},{$args[0].ExitedConfirmed=$false},
        {$args[0].Errors=@('output fault')},{$args[0].NeedsObservation=$true},
        {$args[0].Success=$null},{$args[0].TimedOut='False'},{$args[0].ExitCode='0'},
        {$args[0].Owner=[pscustomobject]@{}},{$args[0].ProcessId=999},
        {$args[0].CreationUtcTicks=[long]1}
    )){
        $t=NewFixture;Inventory $t;$script:mock.Mutate=$mutation
        $r=Invoke-RemovalTraceStep $t start
        Check (-not $r.Success -and -not $t.TraceOwned -and $t.Failed) 'Bad create receipt granted ownership.'
        Reject {Invoke-RemovalTraceStep $t stop}
        Check ($script:mock.Started.Count -eq 2 -and $t.ToolOwners.Count -eq 2) 'Unconfirmed create retried/stopped or owner lost.'
    }
}
Case 'throwing-dispatch-and-observation-remain-sticky-with-no-stop' {
    $t=NewFixture;Inventory $t;$script:mock.ThrowStart='start'
    $r=Invoke-RemovalTraceStep $t start
    Check (-not $r.Success -and $t.Failed -and $t.StartAttempted -and $t.ToolOwners.Count -eq 1) 'Dispatch ambiguity erased.'
    Reject {Invoke-RemovalTraceStep $t start};Reject {Invoke-RemovalTraceStep $t stop}
    $t=NewFixture;Inventory $t;$script:mock.ThrowObserve='start'
    $r=Invoke-RemovalTraceStep $t start
    Check (-not $r.Success -and $t.ToolOwners.Count -eq 2 -and $t.ToolOwners[1].Step -ceq 'start') 'Observation exception lost exact owner.'
    Reject {Invoke-RemovalTraceStep $t stop}
}
Case 'loss-failure-does-not-suppress-one-confirmed-owned-stop' {
    foreach($failure in @('throw','timeout','exit')){
        $t=NewFixture;Own $t
        if($failure -ceq 'throw'){$script:mock.ThrowObserve='loss'}
        if($failure -ceq 'timeout'){$script:mock.Mutate={param($r,$step)if($step -ceq 'loss'){$r.TimedOut=$true}}}
        if($failure -ceq 'exit'){$script:mock.Mutate={param($r,$step)if($step -ceq 'loss'){$r.ExitCode=1}}}
        $loss=Invoke-RemovalTraceStep $t loss;$stop=Invoke-RemovalTraceStep $t stop
        Check (-not $loss.Success -and $stop.Success -and $t.TraceStopped -and $t.Failed -and $t.ToolOwners.Count -eq 4) 'Loss failure suppressed cleanup or cleared failure.'
        Check ($t.RawTraceLoss -ceq 'unknown') 'Failure invented zero loss.'
    }
}
Case 'stop-failure-is-unknown-and-cannot-retry-even-if-later-owner-exits' {
    $t=NewFixture;Own $t;$script:mock.ThrowObserve='stop'
    $r=Invoke-RemovalTraceStep $t stop
    Check (-not $r.Success -and -not $t.TraceStopped -and $t.StopAttempted -and $t.ToolOwners.Count -eq 3) 'Stop ambiguity lost.'
    $owner=$t.ToolOwners[2];$script:mock.ThrowObserve=$null
    Reject {Invoke-RemovalTraceStep $t stop}
    Check ([object]::ReferenceEquals($t.ToolOwners[2],$owner) -and $script:mock.Started.Count -eq 3) 'Stop relaunched or owner replaced.'
}
Case 'loss-after-stop-and-unowned-stop-cannot-dispatch' {
    $t=NewFixture;Reject {Invoke-RemovalTraceStep $t stop}
    Check ($script:mock.Started.Count -eq 0) 'Unowned stop dispatched.'
    $t=NewFixture;Own $t;$null=Invoke-RemovalTraceStep $t stop
    Reject {Invoke-RemovalTraceStep $t loss}
    Check ($script:mock.Started.Count -eq 3) 'Post-stop loss dispatched.'
}
Case 'failed-tool-keeps-original-exit-and-timeout-in-stage-receipt' {
    $t=NewFixture;Inventory $t
    $script:mock.Mutate={param($r,$step)$r.ExitCode=87;$r.TimedOut=$true}
    $r=Invoke-RemovalTraceStep $t start
    Check (-not $r.Success -and $r.ExitCode -eq 87 -and $r.TimedOut -is [bool] -and $r.TimedOut) 'Original failure/timeout hidden by default fields.'
}
Case 'etl-created-after-inventory-is-refused-before-create-dispatch' {
    $t=NewFixture;Inventory $t
    function Test-Path {
        param([string]$LiteralPath)
        if($LiteralPath -ceq $t.EtlPath){return $true}
        Microsoft.PowerShell.Management\Test-Path -LiteralPath $LiteralPath
    }
    $r=Invoke-RemovalTraceStep $t start
    Check (-not $r.Success -and -not $t.TraceOwned -and $script:mock.Started.Count -eq 1) 'ETL freshness not rechecked immediately before create.'
}
Case 'invalid-session-path-tools-and-finite-pin-refuse-before-dispatch' {
    $base=@{
        Session='MoirenRemoval613Unit_01';EtlPath='D:\MoirenTest\trace-unit-only\trace.etl';EvidenceDirectory='D:\MoirenTest\trace-unit-only'
        LogmanPath='C:\Windows\System32\logman.exe';LogmanSha256=('1'*64)
        TracelogPath='D:\MoirenTest\tools\tracelog.exe';TracelogSha256=('2'*64)
    }
    foreach($change in @(
        @{Session='OtherTrace'},@{Session='MoirenRemoval613'},@{Session=('MoirenRemoval613'+('a'*65))},
        @{Session="MoirenRemoval613Bad`n"},@{EtlPath='D:\MoirenTest\trace-unit-only\..\bad.etl'},
        @{EtlPath='\\server\share\trace.etl'},@{EtlPath='D:\MoirenTest\trace-unit-only\trace.etl:stream'},
        @{EtlPath='D:\MoirenTest\trace-unit-only\not.txt'},@{EtlPath='D:\Elsewhere\trace.etl'},
        @{EvidenceDirectory='D:\MoirenTest\trace-unit-only\..\other'},
        @{LogmanPath='C:\Windows\System32\powershell.exe'},@{TracelogPath='tracelog.exe'},
        @{LogmanSha256='invalid'},@{FiniteToolPath=$PSCommandPath}
    )){
        $argsCopy=$base.Clone();foreach($key in $change.Keys){$argsCopy[$key]=$change[$key]}
        Reject {New-RemovalTraceBoundary @argsCopy}
    }
}
Case 'native-launcher-unconditionally-refuses-before-finite-load-even-if-field-flipped' {
    $traces=@((NewFixture),(NewFixture))
    $script:approvalPinReads=0
    # This fail-closed sentinel prevents the RED test from ever loading or
    # launching finite-tool. Only the new approval guard may precede it.
    function Assert-RemovalTraceFinitePin {
        param($Path)
        $script:approvalPinReads++
        throw 'TEST SENTINEL: native launcher reached finite-tool pin/load.'
    }
    foreach($t in $traces){
        $t.NativeApproved=($t -eq $traces[1])
        $t.ToolBoundary=$null
        $message=$null
        try{Start-RemovalTraceFiniteTool $t $null|Out-Null}catch{$message=$_.Exception.Message}
        Check ($null -ne $message -and $message -like '*not approved for native*') 'Real launcher lacks unconditional native-approval refusal.'
        Check ($script:approvalPinReads -eq 0) 'Unapproved launcher touched finite-tool before refusing.'
    }
}
Case 'default-native-step-cannot-dispatch-with-mutable-approval-and-stays-sticky' {
    $t=NewFixture;$t.ToolBoundary=$null;$t.NativeApproved=$true
    $script:approvalStepPinReads=0
    function Assert-RemovalTraceFinitePin {
        param($Path)
        $script:approvalStepPinReads++
        throw 'TEST SENTINEL: native step reached finite-tool pin/load.'
    }
    $r=Invoke-RemovalTraceStep $t inventory
    Check (-not $r.Success -and $t.Failed -and $t.InventoryAttempted -and
        $t.ToolOwners.Count -eq 0 -and -not $t.InventoryConfirmed -and -not $t.TraceOwned) 'Refused native step was accepted/owned.'
    Check ($script:approvalStepPinReads -eq 0) 'Mutable approval authorized default native tool path.'
    Reject {Invoke-RemovalTraceStep $t inventory}
    Check ($script:approvalStepPinReads -eq 0) 'Refused native step retried.'
}
Case 'default-dispatch-retains-owner-before-observe-with-native-wrappers-mocked' {
    # Mock the complete native wrapper: never call real finite-tool/logman here.
    $t=NewFixture
    $t.ToolBoundary=$null
    $script:defaultOwners=$t.ToolOwners
    function Start-RemovalTraceFiniteTool {
        param($Trace,$Request)
        Check ($Request.ExpectedSha256 -ceq ('1'*64) -and $Request.TimeoutSeconds -eq 15 -and $Request.MaxLogBytes -eq 1048576) 'Default dispatch lost pins/caps.'
        [pscustomobject]@{Step='inventory';Closed=$false;IdentityConfirmed=$true;ProcessId=101;CreationUtcTicks=[long]1234567}
    }
    function Observe-RemovalTraceFiniteTool {
        param($Trace,$Owner)
        Check ($script:defaultOwners.Count -eq 1 -and [object]::ReferenceEquals($Owner,$script:defaultOwners[0])) 'Production wrapper did not retain owner.'
        $r=FakeReceipt $Owner;$r.ExitCode=1;return $r
    }
    $r=Invoke-RemovalTraceStep $t inventory
    Check (-not $r.Success -and $t.ToolOwners.Count -eq 1 -and $t.Failed) 'Production wrapper did not keep failed owner.'
}
Case 'inventory-reader-can-read-stable-log-while-finite-owner-retains-writer' {
    $path=Join-Path $PSScriptRoot ('trace-sharing-fixture-'+[Guid]::NewGuid().ToString('N')+'.log')
    $writer=[IO.File]::Open($path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::Read)
    try {
        $expected=[Text.Encoding]::UTF8.GetBytes("OtherSession Trace Running`r`n")
        $writer.Write($expected,0,$expected.Length);$writer.Flush()
        $bytes=Read-RemovalTraceOutput ([pscustomobject]@{StdoutPath=$path})
        Check ([Convert]::ToBase64String($bytes) -ceq [Convert]::ToBase64String($expected)) 'Stable bytes changed/lost while owner retained.'
    } finally {$writer.Dispose()}
}

$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not $_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed)
if($failed){exit 1}
