#requires -Version 5.1
param([string]$Library)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if(-not $Library){$Library=Join-Path $PSScriptRoot 'trace-boundary.ps1'}
. $Library
$finite=Join-Path $PSScriptRoot 'finite-tool.ps1'
Assert-RemovalTraceFinitePin $finite
. $finite
$script:tracePs=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$script:tracePsHash=(Get-FileHash -LiteralPath $script:tracePs -Algorithm SHA256).Hash
$script:traceCalls=New-Object Collections.Generic.List[object]
$script:traceMode='normal'
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
function Case([string]$Name,[scriptblock]$Action){
    try{&$Action;$checks.Add(@{name=$Name;passed=$true})}
    catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}
}
function NewTraceFiniteFixture {
    # Each fixture root is created by the frozen finite-tool; no audio/ETW tool
    # is dispatched. The fake adapter validates production requests first, then
    # substitutes ONLY the exact hash-pinned harmless local PowerShell child.
    $directory=Join-Path $PSScriptRoot ('trace-finite-fixture-'+[Guid]::NewGuid().ToString('N'))
    $adapter=@{
        Start={
            param($request)
            $script:traceCalls.Add($request)
            Check ($request.TimeoutSeconds -eq 15 -and $request.MaxLogBytes -eq 1048576) 'Lost finite caps.'
            $session='MoirenRemoval613Finite_01'
            switch -CaseSensitive ($request.Step){
                inventory {
                    Check (($request.ArgumentList -join '|') -ceq 'query|-ets') 'Wrong inventory argv.'
                    $text="Data Collector Set                      Type                          Status`r`n-------------------------------------------------------------------------------`r`nOtherSession                            Trace                         Running`r`nThe command completed successfully.`r`n"
                    if($script:traceMode -ceq 'garbage'){$text='successful garbage'}
                }
                start {
                    $expected=@('create','trace',$session,'-o',(Join-Path ([IO.Path]::GetDirectoryName($request.OutputDirectory)) 'trace.etl'),
                        '-p','{7DC75BA9-8274-4EAE-A42D-B4C74E0209D2}','0xf','4','-f','bin','-max','16','-ct','perf','-ets')
                    Check (($request.ArgumentList -join '|') -ceq ($expected -join '|')) 'Wrong fixed create argv.'
                    $text='mock create completed; no ETW executed'
                }
                loss {
                    Check (($request.ArgumentList -join '|') -ceq ('-q|'+$session)) 'Wrong exact loss argv.'
                    $text='mock loss bytes; counters intentionally not interpreted'
                }
                stop {
                    Check (($request.ArgumentList -join '|') -ceq ('stop|'+$session+'|-ets')) 'Wrong exact stop argv.'
                    $text='mock stop completed; no ETW executed'
                }
                default {throw 'Unexpected trace step.'}
            }
            if($request.Step -ceq 'loss'){
                Check ($request.Executable -ceq 'D:\MoirenTest\tools\tracelog.exe' -and $request.ExpectedSha256 -ceq ('2'*64)) 'Wrong tracelog pins.'
            } else {
                Check ($request.Executable -ceq 'C:\Windows\System32\logman.exe' -and $request.ExpectedSha256 -ceq ('1'*64)) 'Wrong logman pins.'
            }
            $data=[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($text))
            $code='[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);[Console]::Write([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String("'+$data+'")));[Console]::Error.Write("harmless stderr");exit 0'
            Start-FiniteTool -Executable $script:tracePs -ExpectedSha256 $script:tracePsHash `
                -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-Command',$code) `
                -OutputDirectory $request.OutputDirectory -TimeoutSeconds $request.TimeoutSeconds -MaxLogBytes $request.MaxLogBytes
        }
        Observe={
            param($owner,$trace)
            Check ([object]::ReferenceEquals($trace.ToolOwners[$trace.ToolOwners.Count-1],$owner)) 'Exact real child not retained before observe.'
            Observe-FiniteTool -Owner $owner
        }
        Read={param($receipt)Read-RemovalTraceOutput $receipt}
    }
    New-RemovalTraceBoundary -Session 'MoirenRemoval613Finite_01' -EtlPath (Join-Path $directory 'trace.etl') `
        -EvidenceDirectory $directory -LogmanPath 'C:\Windows\System32\logman.exe' -LogmanSha256 ('1'*64) `
        -TracelogPath 'D:\MoirenTest\tools\tracelog.exe' -TracelogSha256 ('2'*64) -ToolBoundary $adapter
}
function CloseTraceFiniteOwners($Trace) {
    foreach($owner in $Trace.ToolOwners){
        if($owner.Closed){continue}
        $r=Observe-FiniteTool -Owner $owner -ObservationMilliseconds 15000
        Check ($r.ExitedConfirmed -and $r.OutputStable -and -not $r.NeedsObservation) 'Unresolved real child retained; do not kill/dispose/relaunch.'
        Close-FiniteToolOwner -Owner $owner
    }
}
Case 'actual-frozen-finite-tool-composes-four-harmless-children-and-exact-handles' {
    $script:traceMode='normal';$t=NewTraceFiniteFixture;$baseCalls=$script:traceCalls.Count
    try {
        foreach($step in @('inventory','start','loss','stop')){
            $r=Invoke-RemovalTraceStep $t $step
            Check $r.Success ('Real finite composition failed: '+$step)
            Check ($r.ToolOwner.Process -is [Diagnostics.Process] -and $r.ToolOwner.IdentityConfirmed -and
                $r.ToolOwner.Handle -ne [IntPtr]::Zero -and $r.ToolReceipt.ExitedConfirmed -and $r.ToolReceipt.OutputStable) 'Lost real process/handle/exit/pipes.'
            Check ([object]::ReferenceEquals($r.ToolOwner,$t.ToolOwners[$t.ToolOwners.Count-1])) 'Original real owner replaced.'
            $err=[IO.File]::Open($r.ToolReceipt.StderrPath,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
            try{Check ($err.Length -eq 15) 'stderr not captured.'}finally{$err.Dispose()}
        }
        Check ($t.TraceOwned -and $t.TraceStopped -and -not $t.Failed -and $script:traceCalls.Count-$baseCalls -eq 4) 'Lifecycle/call count differs.'
        Check ($t.RawTraceLoss -ceq 'unknown' -and -not $t.Qualified -and -not $t.NativeApproved) 'Fake child yielded native qualification.'
    } finally {CloseTraceFiniteOwners $t}
    Check (@($t.ToolOwners|Where-Object{-not $_.Closed}).Count -eq 0) 'Resolved real owner not explicitly closed.'
}
Case 'real-exit-zero-garbage-inventory-refuses-before-create-and-closes-original-owner' {
    $script:traceMode='garbage';$t=NewTraceFiniteFixture;$baseCalls=$script:traceCalls.Count
    try {
        $r=Invoke-RemovalTraceStep $t inventory
        Check ($r.ToolReceipt.Success -and -not $r.Success -and -not $t.InventoryConfirmed -and $t.Failed) 'Real successful garbage proved absence.'
        Check ($script:traceCalls.Count-$baseCalls -eq 1 -and $t.ToolOwners.Count -eq 1 -and -not $t.TraceOwned) 'Garbage reached create.'
    } finally {CloseTraceFiniteOwners $t}
    Check $t.ToolOwners[0].Closed 'Refused inventory left resolved handle open.'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not $_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ETW_EXECUTED=false CHILD=real-harmless-powershell')
if($failed){exit 1}
