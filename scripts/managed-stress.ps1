param(
    [string]$Executable = (Join-Path $PSScriptRoot '..\target\release\examples\managed_stress.exe'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\docs\superpowers\plans\2026-10-14-engine-stress'),
    [int]$BaselineSeconds = 60,
    [int]$LoadSeconds = 120,
    [switch]$FrequentSwaps,
    [ValidateRange(10, 1800)]
    [int]$SaturationSeconds = 300
)

$ErrorActionPreference = 'Stop'
if ($BaselineSeconds -lt 10 -or $BaselineSeconds -gt 1800 -or $LoadSeconds -lt 10 -or $LoadSeconds -gt 1800) {
    throw 'Use baseline/load windows between 10 and 1800 seconds'
}
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Output directory already exists; preserve previous evidence' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$cases = @(
    @{ name = 'baseline'; seconds = $BaselineSeconds; nodes = 8; edits = 0; parameters = 0; cycles = 10 },
    @{ name = 'updates'; seconds = $LoadSeconds; nodes = 64; edits = 20; parameters = 200; cycles = 20 },
    @{ name = 'overload'; seconds = $LoadSeconds; nodes = 120; edits = 400; parameters = 2000; cycles = 30 }
)
if ($FrequentSwaps) {
    $cases = @(
        @{ name = 'swap-baseline'; seconds = $BaselineSeconds; nodes = 120; edits = 0; parameters = 200; cycles = 0 },
        @{ name = 'swap-20'; seconds = $LoadSeconds; nodes = 64; edits = 20; parameters = 200; cycles = 0 },
        @{ name = 'swap-50'; seconds = $LoadSeconds; nodes = 120; edits = 50; parameters = 200; cycles = 0 },
        @{ name = 'swap-100'; seconds = $LoadSeconds; nodes = 120; edits = 100; parameters = 200; cycles = 0 },
        @{ name = 'swap-saturation'; seconds = $SaturationSeconds; nodes = 120; edits = 2000; parameters = 200; cycles = 10 }
    )
}
$results = [System.Collections.Generic.List[object]]::new()
foreach ($case in $cases) {
    $stdout = Join-Path $OutputDirectory ($case.name + '.stdout.jsonl')
    $stderr = Join-Path $OutputDirectory ($case.name + '.stderr.log')
    $arguments = @('--seconds', $case.seconds, '--nodes', $case.nodes,
        '--edits-per-second', $case.edits, '--parameters-per-second', $case.parameters,
        '--cycles', $case.cycles)
    if ($FrequentSwaps) { $arguments += @('--confirmed-swaps', '1') }
    Write-Output "START $($case.name): $($arguments -join ' ')"
    $process = Start-Process -FilePath $Executable -ArgumentList $arguments -WindowStyle Hidden `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    $samples = [System.Collections.Generic.List[object]]::new()
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $deadline = $case.seconds + $case.cycles * 10 + 45
    try {
        while (-not $process.HasExited) {
            if ($clock.Elapsed.TotalSeconds -gt $deadline) { throw "Owned probe PID $($process.Id) exceeded its deadline" }
            try {
                $observed = Get-Process -Id $process.Id -ErrorAction Stop
                $samples.Add([pscustomobject]@{
                    seconds = $clock.Elapsed.TotalSeconds
                    cpu_seconds = $observed.TotalProcessorTime.TotalSeconds
                    private_bytes = $observed.PrivateMemorySize64
                    working_set_bytes = $observed.WorkingSet64
                    handles = $observed.HandleCount
                    threads = $observed.Threads.Count
                })
            } catch {
                $process.Refresh()
                if (-not $process.HasExited) { throw }
            }
            $process.WaitForExit(1000) | Out-Null
            $process.Refresh()
        }
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) {
            Get-Content -LiteralPath $stderr -Tail 35 | Write-Output
            throw "$($case.name) failed with exit $($process.ExitCode)"
        }
        $events = @(Get-Content -LiteralPath $stdout | ForEach-Object { $_ | ConvertFrom-Json })
        $report = $events | Where-Object { $_.event -eq 'complete' } | Select-Object -Last 1
        if ($null -eq $report) { throw 'Probe completed without its final report' }
        $steady = @($samples | Where-Object { $_.seconds -ge 5 -and $_.seconds -lt $case.seconds })
        if ($steady.Count -lt 2) { throw 'Insufficient process samples' }
        $first = $steady[0]
        $last = $steady[-1]
        $sampleSeconds = $last.seconds - $first.seconds
        $oneCorePercent = 100 * ($last.cpu_seconds - $first.cpu_seconds) / $sampleSeconds
        $result = [pscustomobject]@{
            case = $case
            pid = $process.Id
            elapsed_seconds = $clock.Elapsed.TotalSeconds
            sampled_steady_seconds = $sampleSeconds
            logical_processors = [Environment]::ProcessorCount
            cpu_one_core_percent = $oneCorePercent
            cpu_machine_percent = $oneCorePercent / [Environment]::ProcessorCount
            private_peak_bytes = ($steady | Measure-Object private_bytes -Maximum).Maximum
            private_first_bytes = $first.private_bytes
            private_last_bytes = $last.private_bytes
            private_delta_bytes = $last.private_bytes - $first.private_bytes
            working_set_peak_bytes = ($steady | Measure-Object working_set_bytes -Maximum).Maximum
            handles_min = ($steady | Measure-Object handles -Minimum).Minimum
            handles_max = ($steady | Measure-Object handles -Maximum).Maximum
            threads_min = ($steady | Measure-Object threads -Minimum).Minimum
            threads_max = ($steady | Measure-Object threads -Maximum).Maximum
            confirmed_swaps = $report.confirmed_swaps
            samples = $samples.ToArray()
            engine = $report
        }
        $result | ConvertTo-Json -Depth 35 | Set-Content -LiteralPath (Join-Path $OutputDirectory ($case.name + '.json'))
        $results.Add($result)
        Write-Output ("PASS {0}: CPU {1:N2}% of one core; private peak {2:N2} MiB; delta {3:N2} MiB; handles {4}-{5}; threads {6}-{7}" -f
            $case.name, $oneCorePercent, ($result.private_peak_bytes / 1MB), ($result.private_delta_bytes / 1MB),
            $result.handles_min, $result.handles_max, $result.threads_min, $result.threads_max)
        if ($null -ne $report.confirmed_swaps) {
            Write-Output ("SWAPS {0}: {1} confirmed; {2:N2}/s; activation p99 {3} ms; max {4:N2} ms" -f
                $case.name, $report.confirmed_swaps.completed, $report.confirmed_swaps.per_second,
                $report.confirmed_swaps.latency_p99_ms, $report.confirmed_swaps.latency_max_ms)
        }
    } catch {
        [pscustomobject]@{ case = $case; error = $_.ToString(); samples = $samples.ToArray() } |
            ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $OutputDirectory ($case.name + '.failure.json'))
        throw
    } finally {
        $process.Refresh()
        if (-not $process.HasExited) {
            $process.Kill($true)
            if (-not $process.WaitForExit(10000)) { throw 'Owned probe did not terminate' }
        }
        $process.Dispose()
    }
}
$results.ToArray() | Select-Object -Property * -ExcludeProperty samples,engine |
    ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'summary.json')
Write-Output "All owned probes exited; evidence: $OutputDirectory"
