# SPDX-License-Identifier: MIT
# Run inside the dedicated Windows guest. Does not install or reconfigure drivers.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExpectedSha256,
    [Parameter(Mandatory = $true)][ValidatePattern('^[a-fA-F0-9]{40}$')][string]$SourceCommit,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [ValidateRange(0, 28800)][int]$IdleSeconds = 28800,
    [ValidateRange(0, 28800)][int]$PcmSeconds = 28800,
    # Optional looping WAV driving the PCM phase (the probe's music mode).
    [string]$PcmFile,
    [string]$ExpectedComputer = 'DRIVER-TEST'
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Confirm-SoakReport($Report, [int]$Idle, [int]$Pcm, [double]$ElapsedSeconds) {
    if ($Report.status -ne 'passed' -or -not $Report.service_joined -or -not $Report.control_session_released) {
        throw 'Final report does not confirm requested stages and orderly service teardown'
    }
    foreach ($phase in @(@{ Name = 'soak_idle'; Seconds = $Idle }, @{ Name = 'soak_pcm'; Seconds = $Pcm })) {
        if ($phase.Seconds -eq 0) { continue }
        $evidence = $Report.($phase.Name)
        $requested = [long]$phase.Seconds * 1000
        if ($null -eq $evidence -or $evidence.completed -ne $true -or
            $evidence.requested_ms -ne $requested -or $evidence.elapsed_ms -lt $requested) {
            throw "Missing, incomplete, or mismatched $($phase.Name) duration evidence"
        }
    }
    if ($ElapsedSeconds -lt $Idle + $Pcm) { throw 'Probe completed before the requested continuous durations' }
}

if ($env:COMPUTERNAME -ne $ExpectedComputer) { throw 'Unexpected test computer' }
if ($IdleSeconds + $PcmSeconds -eq 0) { throw 'Select at least one soak phase' }
if ($PcmFile) {
    if ($PcmSeconds -eq 0) { throw 'PcmFile requires a PCM phase' }
    $PcmFile = (Resolve-Path -LiteralPath $PcmFile).Path
    # Start-Process on Windows joins its argument array into a command line.
    if ($PcmFile -match '[\s"\x00]') { throw 'Use a material path without whitespace or quotes' }
}
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
# Start-Process on Windows joins its argument array into a command line.
if ($OutputDirectory -match '[\s"\x00]') { throw 'Use an output path without whitespace or quotes' }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Evidence directory already exists' }
$actualHash = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash
if ($actualHash -ne $ExpectedSha256) { throw 'Executable digest does not match the reviewed build' }
if (@(Get-Process vad*, moiren* -ErrorAction SilentlyContinue).Count) {
    throw 'Another audio probe or application is active'
}

New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$probeDirectory = Join-Path $OutputDirectory 'probe'
$timeoutSeconds = $IdleSeconds + $PcmSeconds + 180
$arguments = @('--exercise', "--timeout-seconds=$timeoutSeconds", "--output-dir=$probeDirectory")
if ($IdleSeconds) { $arguments += "--soak-idle-seconds=$IdleSeconds" }
if ($PcmSeconds) { $arguments += "--soak-pcm-seconds=$PcmSeconds" }
if ($PcmFile) { $arguments += "--soak-pcm-file=$PcmFile" }
$env:RUST_LOG = 'info'
$process = $null
$probeClock = $null
$writer = $null
$clock = [Diagnostics.Stopwatch]::StartNew()
$started = [DateTime]::UtcNow.ToString('o')
$status = 'failed'
$failure = $null
$exitCode = $null
$resourceErrors = 0
$sampleCount = 0
$poolSamples = 0
$forcedTermination = $false
$terminationConfirmed = $false
$cleanupErrors = [Collections.Generic.List[string]]::new()
$powerOutcomes = [Collections.Generic.List[object]]::new()
$lastPoolSeconds = -60.0
$nextSampleSeconds = 0.0

try {
    [ordered]@{
        status = 'preflight'
        computer = $env:COMPUTERNAME
        started_utc = $started
        executable_sha256 = $actualHash
        source_commit = $SourceCommit
        power_changes_persist = $true
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $OutputDirectory 'preflight.json') -Encoding UTF8
    # These changes are intentional dedicated-guest policy, including after a failed run.
    foreach ($powerArguments in @(
        @('/change', 'standby-timeout-ac', '0'),
        @('/change', 'standby-timeout-dc', '0'),
        @('/hibernate', 'off')
    )) {
        $powerExit = $null
        $powerOutput = ''
        $powerError = $null
        $savedPreference = $ErrorActionPreference
        try {
            # Windows PowerShell turns native stderr into error records. Collect
            # it before deciding success from the native exit code.
            $ErrorActionPreference = 'Continue'
            $powerOutput = & powercfg.exe @powerArguments 2>&1 | Out-String
            $powerExit = $LASTEXITCODE
        } catch {
            $powerError = $_.Exception.Message
        } finally {
            $ErrorActionPreference = $savedPreference
        }
        $powerOutcomes.Add([ordered]@{
            arguments = $powerArguments
            exit_code = $powerExit
            output = $powerOutput.Substring(0, [Math]::Min(4096, $powerOutput.Length))
            error = $powerError
        })
        $powerOutcomes | ConvertTo-Json -Depth 4 |
            Set-Content -LiteralPath (Join-Path $OutputDirectory 'power.json') -Encoding UTF8
        if ($powerError -or $null -eq $powerExit -or $powerExit -ne 0) {
            throw "Power preflight failed: $($powerArguments -join ' '), exit $powerExit; $powerError"
        }
    }
    if (-not ('MoirenVadSoak.NativeMemory' -as [type])) {
        # GetPerformanceInfo returns a snapshot without WMI/PnP enumeration on the supervisor.
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace MoirenVadSoak {
    [StructLayout(LayoutKind.Sequential)]
    public struct MemoryInfo {
        public uint Size;
        public UIntPtr CommitTotal, CommitLimit, CommitPeak, PhysicalTotal;
        public UIntPtr PhysicalAvailable, SystemCache, KernelTotal, KernelPaged;
        public UIntPtr KernelNonpaged, PageSize;
        public uint HandleCount, ProcessCount, ThreadCount;
    }
    public static class NativeMemory {
        [DllImport("psapi.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetPerformanceInfo(ref MemoryInfo info, uint size);
    }
}
'@
    }
    $process = Start-Process -FilePath $Executable -ArgumentList $arguments -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $OutputDirectory 'stdout.log') `
        -RedirectStandardError (Join-Path $OutputDirectory 'stderr.log')
    $probeClock = [Diagnostics.Stopwatch]::StartNew()
    # Retain the process handle: cancellation never targets a subsequently reused PID.
    $null = $process.Handle
    [ordered]@{
        schema_version = 1
        status = 'running'
        computer = $env:COMPUTERNAME
        runner_pid = $PID
        probe_pid = $process.Id
        probe_start_utc = $process.StartTime.ToUniversalTime().ToString('o')
        started_utc = $started
        executable = $Executable
        executable_sha256 = $actualHash
        source_commit = $SourceCommit
        idle_seconds = $IdleSeconds
        pcm_seconds = $PcmSeconds
        probe_timeout_seconds = $timeoutSeconds
        outer_timeout_seconds = $timeoutSeconds + 15
        cancel_file = Join-Path $OutputDirectory 'cancel'
        arguments = $arguments
        limitations = @(
            'System pool counters include all guest drivers; they do not attribute allocations to Moiren.',
            'This supervisor does not enumerate PnP; take separate passive before/after snapshots.',
            'Creating the cancel file terminates only the owned probe; orderly native cleanup is then unconfirmed.'
        )
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'running.json') -Encoding UTF8
    $writer = [IO.StreamWriter]::new((Join-Path $OutputDirectory 'resources.jsonl'), $false, [Text.UTF8Encoding]::new($false))
    $writer.AutoFlush = $true
    while (-not $process.HasExited) {
        if (Test-Path -LiteralPath (Join-Path $OutputDirectory 'cancel')) {
            $status = 'cancelled'
            throw 'Cancellation requested'
        }
        if ($clock.Elapsed.TotalSeconds -ge $timeoutSeconds + 15) {
            $status = 'timed_out'
            throw 'Outer deadline exceeded'
        }
        if ($clock.Elapsed.TotalSeconds -ge $nextSampleSeconds) {
            $process.Refresh()
            if ($process.HasExited) { break }
            $sample = [ordered]@{
                utc = [DateTime]::UtcNow.ToString('o')
                elapsed_seconds = $clock.Elapsed.TotalSeconds
                probe_pid = $process.Id
                cpu_seconds = $process.TotalProcessorTime.TotalSeconds
                private_bytes = $process.PrivateMemorySize64
                working_set_bytes = $process.WorkingSet64
                handles = $process.HandleCount
                threads = $process.Threads.Count
            }
            # Stream samples to disk; never retain a duration-proportional array.
            if ($clock.Elapsed.TotalSeconds - $lastPoolSeconds -ge 60) {
                try {
                    $pool = [MoirenVadSoak.MemoryInfo]::new()
                    $pool.Size = [Runtime.InteropServices.Marshal]::SizeOf($pool)
                    if (-not [MoirenVadSoak.NativeMemory]::GetPerformanceInfo([ref]$pool, $pool.Size)) {
                        throw "GetPerformanceInfo failed: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())"
                    }
                    $pageSize = $pool.PageSize.ToUInt64()
                    $sample.pool_nonpaged_bytes = $pool.KernelNonpaged.ToUInt64() * $pageSize
                    $sample.pool_paged_bytes = $pool.KernelPaged.ToUInt64() * $pageSize
                    $sample.available_bytes = $pool.PhysicalAvailable.ToUInt64() * $pageSize
                    $poolSamples++
                } catch {
                    $sample.pool_error = $_.Exception.Message
                    $resourceErrors++
                }
                $lastPoolSeconds = $clock.Elapsed.TotalSeconds
            }
            $writer.WriteLine(($sample | ConvertTo-Json -Depth 6 -Compress))
            $sampleCount++
            $nextSampleSeconds = $clock.Elapsed.TotalSeconds + 10
        }
        $null = $process.WaitForExit(1000)
    }
    $null = $process.WaitForExit()
    $probeClock.Stop()
    $exitCode = $process.ExitCode
    if ($exitCode -ne 0) { throw "Probe failed with exit code $exitCode" }
    $reportPath = Join-Path $probeDirectory 'report.json'
    if (-not (Test-Path -LiteralPath $reportPath)) { throw 'Probe exited without its final report' }
    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    Confirm-SoakReport $report $IdleSeconds $PcmSeconds $probeClock.Elapsed.TotalSeconds
    if ($sampleCount -lt 2 -or $poolSamples -lt 1) { throw 'Insufficient resource samples' }
    if ($resourceErrors) { throw "Probe passed but $resourceErrors resource observations failed" }
    $status = 'passed'
} catch {
    $failure = $_.Exception.Message
} finally {
    if ($null -ne $process) {
        try {
            $process.Refresh()
            if (-not $process.HasExited) {
                $forcedTermination = $true
                $process.Kill()
                $terminationConfirmed = $process.WaitForExit(5000)
                if (-not $terminationConfirmed) { throw 'Owned probe did not terminate' }
            } else {
                $terminationConfirmed = $true
            }
            if ($terminationConfirmed) { $exitCode = $process.ExitCode }
        } catch {
            $cleanupErrors.Add("process cleanup: $($_.Exception.Message)")
        }
        try { $process.Dispose() } catch { $cleanupErrors.Add("process dispose: $($_.Exception.Message)") }
    }
    if ($null -ne $writer) {
        try { $writer.Dispose() } catch { $cleanupErrors.Add("writer dispose: $($_.Exception.Message)") }
    }
    if ($null -ne $probeClock) { $probeClock.Stop() }
    if ($cleanupErrors.Count -and $status -eq 'passed') {
        $status = 'failed'
        if (-not $failure) { $failure = 'Cleanup failed after probe completion' }
    }
    # Cleanup failures must not suppress the original failure or its final receipt.
    try {
        [ordered]@{
            schema_version = 1
            status = $status
            started_utc = $started
            finished_utc = [DateTime]::UtcNow.ToString('o')
            elapsed_seconds = $clock.Elapsed.TotalSeconds
            probe_elapsed_seconds = if ($null -ne $probeClock) { $probeClock.Elapsed.TotalSeconds } else { $null }
            source_commit = $SourceCommit
            executable_sha256 = $actualHash
            idle_seconds = $IdleSeconds
            pcm_seconds = $PcmSeconds
            exit_code = $exitCode
            forced_termination = $forcedTermination
            termination_confirmed = $terminationConfirmed
            cleanup_errors = $cleanupErrors.ToArray()
            power_outcomes = $powerOutcomes.ToArray()
            resource_samples = $sampleCount
            pool_samples = $poolSamples
            resource_errors = $resourceErrors
            resource_growth_qualified = $false
            error = $failure
        } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'result.json') -Encoding UTF8
    } catch {
        Write-Error "Could not persist final receipt: $($_.Exception.Message)"
    }
}
if ($status -ne 'passed') { throw "Soak $status`: $failure" }
Write-Output "Soak passed; evidence: $OutputDirectory"
