#requires -Version 5.1
param(
    [Parameter(Mandatory=$true)][string]$TestExecutable,
    [Parameter(Mandatory=$true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$ExpectedSha256
)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'probe-boundary.ps1')

# Only the Rust test fixture is selected, never the executable's native main.
# Its admitted closure creates cli-stub-only and returns 1; libtest then exits 0.
$fixture='managed_removal::native::tests::process_fixture'
$root=Join-Path $PSScriptRoot ('task-fixtures\managed-protocol-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$checks=0;$failed=$false;$worker=$null;$probe=$null;$listing=$null
$script:ManagedProtocolRetainedOwners=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why};$script:checks++}
function Finish-ShortTool($Owner) {
    $clock=[Diagnostics.Stopwatch]::StartNew()
    do {
        $r=Observe-FiniteTool $Owner -ObservationMilliseconds 50
        if(-not $r.NeedsObservation){return $r}
    } while($clock.ElapsedMilliseconds -lt 15000)
    throw 'Harmless tool/pipes unresolved; exact owner retained, no kill/relaunch.'
}

try {
    Assert-FiniteToolPath $TestExecutable $true
    Check ((Get-FileHash -LiteralPath $TestExecutable -Algorithm SHA256).Hash -ieq $ExpectedSha256) 'Rust test executable pin differs.'
    # Detect a missing/wrong test binary before any success assertion. A libtest
    # command matching zero tests exits 0 and is not protocol evidence.
    $listing=Start-FiniteTool -Executable $TestExecutable -ExpectedSha256 $ExpectedSha256 `
        -OutputDirectory (Join-Path $root 'list') -ArgumentList @('--list','--exact',$fixture) -TimeoutSeconds 10
    $script:ManagedProtocolRetainedOwners.Add($listing)
    $listed=Finish-ShortTool $listing
    Check $listed.Success 'Rust fixture listing failed/late/unresolved.'
    Close-FiniteToolOwner $listing
    $text=[IO.File]::ReadAllText($listing.StdoutPath)
    Check ($text -match ('(?m)^'+[regex]::Escape($fixture)+': test\r?$') -and $text -match '(?m)^1 test, 0 benchmarks\r?$') 'Exact Rust fixture is absent.'

    $self=[Diagnostics.Process]::GetCurrentProcess()
    $sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    Check ($self.SessionId -gt 0 -and -not ([Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) 'Host smoke requires a non-elevated nonzero session.'
    $nonce=[Guid]::NewGuid().ToString('N')
    $run=Join-Path $root 'exchange';$null=[IO.Directory]::CreateDirectory($run)
    $config=@{Executable=$TestExecutable;ExpectedSha256=$ExpectedSha256;RunDirectory=$run;Nonce=$nonce
        ControllerPid=$self.Id;ControllerCreationUtcTicks=$self.StartTime.ToUniversalTime().Ticks
        SessionId=$self.SessionId;Sid=$sid;Library=(Join-Path $PSScriptRoot 'worker-boundary.ps1')}
    $configPath=Join-Path $root 'fixture.json';Write-RemovalJsonOnce $configPath $config
    $entry=Join-Path $root 'worker.ps1'
    $source=@'
param([string]$Configuration)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
# PS7's inherited module path is not a WinPS5 module environment.
$env:PSModulePath=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\Modules'
$config=Get-Content -LiteralPath $Configuration -Raw -Encoding UTF8|ConvertFrom-Json
. $config.Library
$current=[Diagnostics.Process]::GetCurrentProcess()
$s=New-RemovalProbeSpec -Executable $config.Executable -ExpectedSha256 $config.ExpectedSha256 `
    -RunDirectory $config.RunDirectory -Nonce $config.Nonce -SessionId $config.SessionId -Sid $config.Sid `
    -ControllerPid $config.ControllerPid -ControllerCreationUtcTicks $config.ControllerCreationUtcTicks `
    -WorkerPid $current.Id -WorkerCreationUtcTicks $current.StartTime.ToUniversalTime().Ticks -ProbeArguments @('live-vad','--seconds','8')
# The smoke's finite admission budget is 5s (supported Rust range 1..15000).
# Environment is private to this fresh worker and its one harmless test child.
$s.Arguments[14]='5000'
$env:MOIREN_MANAGED_REMOVAL_TEST_FIXTURE='success'
$env:MOIREN_MANAGED_REMOVAL_TEST_DIRECTORY=$config.RunDirectory
$env:MOIREN_MANAGED_REMOVAL_TEST_ARGUMENTS=ConvertTo-Json -InputObject @($s.Arguments) -Compress
$s.CommandLine=(@('--exact','managed_removal::native::tests::process_fixture','--nocapture')|ForEach-Object{Quote-FiniteToolArgument $_}) -join ' '
$owner=New-RemovalWorkerProbeOwner $s
$script:ManagedProtocolProbeOwner=$owner
Start-RemovalWorkerProbe $owner
if(-not $owner.IdentityConfirmed){throw 'Harmless Rust child identity unknown; owner retained.'}
do {$r=Observe-RemovalWorkerProbe $owner -ObservationMilliseconds 50} while($r.NeedsObservation)
$result=@{ProbePid=$owner.ProcessId;ProbeCreationUtcTicks=$owner.CreationUtcTicks;ProbeExitCode=$r.ExitCode
    OutputStable=$r.OutputStable;Errors=$r.Errors;TimedOut=$r.TimedOut;LogLimitExceeded=$r.LogLimitExceeded}
Write-RemovalJsonOnce (Join-Path $config.RunDirectory 'smoke-worker-result.json') $result
Close-RemovalWorkerProbeOwner $owner
if($r.ExitCode -ne 0 -or -not $r.OutputStable -or $r.TimedOut -or $r.LogLimitExceeded -or $r.Errors.Count){exit 1}
exit 0
'@
    [IO.File]::WriteAllText($entry,$source,[Text.UTF8Encoding]::new($false))
    $ps=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $worker=Start-FiniteTool -Executable $ps -ExpectedSha256 (Get-FileHash -LiteralPath $ps -Algorithm SHA256).Hash `
        -OutputDirectory (Join-Path $root 'worker-output') -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-File',$entry,'-Configuration',$configPath) -TimeoutSeconds 15
    $script:ManagedProtocolRetainedOwners.Add($worker)
    Check ($worker.IdentityConfirmed -and $worker.Errors.Count -eq 0) 'Original harmless worker identity/output setup failed.'
    $readyPath=Join-Path $run 'probe-ready.json';$clock=[Diagnostics.Stopwatch]::StartNew()
    while(-not(Test-Path -LiteralPath $readyPath)){
        if($worker.Process.HasExited -or $clock.ElapsedMilliseconds -ge 10000){throw 'Actual Rust ready receipt missing; no admission/retry.'}
        Start-Sleep -Milliseconds 5
    }
    $ready=Read-RemovalJsonBounded $readyPath
    $s=New-RemovalProbeSpec -Executable $TestExecutable -ExpectedSha256 $ExpectedSha256 `
        -RunDirectory $run -Nonce $nonce -SessionId $self.SessionId -Sid $sid `
        -ControllerPid $self.Id -ControllerCreationUtcTicks $self.StartTime.ToUniversalTime().Ticks `
        -WorkerPid $worker.ProcessId -WorkerCreationUtcTicks $worker.CreationUtcTicks -ProbeArguments @('live-vad','--seconds','8')
    $s.CommandLine=(@('--exact',$fixture,'--nocapture')|ForEach-Object{Quote-FiniteToolArgument $_}) -join ' '
    $probe=New-RemovalProbeOwner $s
    $script:ManagedProtocolRetainedOwners.Add($probe)
    Attach-RemovalProbeProcess $probe $ready
    Check ($probe.BindingKnown -and $ready.probe_creation_utc_ticks -is [long] -and $ready.controller_creation_utc_ticks -ceq $config.ControllerCreationUtcTicks -and $ready.worker_creation_utc_ticks -ceq $worker.CreationUtcTicks) 'Rust numeric ticks did not survive actual PS parse/handle binding.'
    Check (-not(Test-Path -LiteralPath (Join-Path $run 'cli-stub-only')) -and -not $probe.Process.HasExited) 'Rust gate entered before admission.'
    Admit-RemovalProbe $probe
    $ack=Read-RemovalJsonBounded (Join-Path $run 'probe-admit.json')
    Assert-RemovalProbeReady $s $ack
    Check ($probe.AdmissionWritten -and $ack.probe_pid -eq $ready.probe_pid -and $ack.probe_creation_utc_ticks -ceq $ready.probe_creation_utc_ticks) 'Actual PS admission changed the Rust receipt.'
    $done=Finish-ShortTool $worker
    Check $done.Success 'Rust fixture/worker did not complete successfully after PS admission.'
    $observed=Observe-RemovalProbe $probe
    Check ($observed.ExitKnown -and $observed.ExitCode -eq 0 -and -not $observed.NeedsObservation) 'Independent original Rust process exit was not confirmed.'
    $result=Read-RemovalJsonBounded (Join-Path $run 'smoke-worker-result.json')
    Check ($result.ProbePid -eq $probe.ProcessId -and $result.ProbeCreationUtcTicks -ceq $probe.CreationUtcTicks -and $result.ProbeExitCode -eq $observed.ExitCode -and $result.OutputStable -and $result.Errors.Count -eq 0 -and -not $result.TimedOut -and -not $result.LogLimitExceeded) 'Worker result disagrees with original Rust process.'
    Check ((Test-Path -LiteralPath (Join-Path $run 'cli-stub-only')) -and [IO.File]::ReadAllText((Join-Path $run 'cli-stub-only')) -ceq '1') 'Rust admitted stub was not called exactly once.'
    $output=[IO.File]::ReadAllText((Join-Path $run 'probe-stdout.log'))
    Check ($output.Contains('test result: ok. 1 passed; 0 failed;') -and $output.Contains($fixture)) 'Exact Rust subtest did not pass.'
} catch {
    $failed=$true
    Write-Output ('ERROR='+$_.Exception.Message)
} finally {
    # Explicitly observe natural exit only. On any unresolved owner, retain the
    # original objects in script scope; never kill, reacquire or retry dispatch.
    foreach($owner in @($listing,$worker)){
        if($null -eq $owner -or $owner.Closed){continue}
        try {
            $r=Observe-FiniteTool $owner -ObservationMilliseconds 1000
            if(-not $r.NeedsObservation){Close-FiniteToolOwner $owner}
        } catch {$failed=$true;Write-Output ('CLEANUP_ERROR='+$_.Exception.Message)}
        if(-not $owner.Closed){$failed=$true;Write-Output ('PENDING_EXACT_PROCESS='+$owner.ProcessId)}
    }
    if($null -ne $probe -and $null -ne $probe.Process){
        if($probe.Process.WaitForExit(0)){$probe.Process.Dispose();$probe.Closed=$true}
        else {$failed=$true;Write-Output ('PENDING_EXACT_PROBE='+$probe.ProcessId)}
    }
}
Write-Output ('FIXTURE_ROOT='+$root)
Write-Output ('CHECKS='+$checks+' FAILED='+[int]$failed+' RUST_FIXTURE=real PS_PROTOCOL=real SCHEDULER=not_used AUDIO_CLI=not_called NATIVE_EXECUTED=false')
if($failed){exit 1}
