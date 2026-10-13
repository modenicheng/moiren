param(
    [Parameter(Mandatory)][string]$ProbePath,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$CaptureEndpointId,
    [Parameter(Mandatory)][string]$RenderEndpointId,
    [string]$SourceCommit,
    [string]$SourceTree
)
$ErrorActionPreference = 'Stop'
$ProbePath = (Resolve-Path -LiteralPath $ProbePath).Path
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$OutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
if (Get-ChildItem -LiteralPath $OutputDirectory) { throw 'Use an empty output directory to preserve previous evidence' }
$taskEndpoints = (& $ProbePath --list | ConvertFrom-Json)
if ($LASTEXITCODE -ne 0) { throw 'Endpoint enumeration failed' }
$taskCapture = @($taskEndpoints.capture | Where-Object { $_.endpoint_id -eq $CaptureEndpointId })
$taskRender = @($taskEndpoints.render | Where-Object { $_.endpoint_id -eq $RenderEndpointId })
if ($taskCapture.Count -ne 1 -or $taskRender.Count -ne 1 -or !$taskCapture[0].format_supported -or !$taskRender[0].format_supported) { throw 'Selected endpoints are missing or unsupported' }
[ordered]@{
    source_commit = $SourceCommit
    source_tree = $SourceTree
    timestamp = (Get-Date).ToString('o')
    profile = 'caller_supplied_binary'
    os_version = [Environment]::OSVersion.Version.ToString()
    logical_processors = [Environment]::ProcessorCount
    probe_sha256 = (Get-FileHash -LiteralPath $ProbePath -Algorithm SHA256).Hash.ToLowerInvariant()
    capture = $taskCapture[0]
    render = $taskRender[0]
    gain = 0
    measurement_method = 'unknown'
} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath "$OutputDirectory\environment.json" -Encoding utf8
$taskCases = @(
    @{Name='compatible-default-summary'; Period='default'; Target='capacity'; Mode='summary'; Seconds=5},
    @{Name='compatible-small-summary'; Period='5'; Target='capacity'; Mode='summary'; Seconds=5},
    @{Name='joint-disabled'; Period='5'; Target='480'; Mode='disabled'; Seconds=5},
    @{Name='joint-summary'; Period='5'; Target='480'; Mode='summary'; Seconds=5},
    @{Name='joint-trace'; Period='5'; Target='480'; Mode='trace'; Seconds=5},
    @{Name='compatible-small-summary-120s'; Period='5'; Target='capacity'; Mode='summary'; Seconds=120},
    @{Name='joint-summary-120s'; Period='5'; Target='480'; Mode='summary'; Seconds=120},
    @{Name='joint-load-summary-120s'; Period='5'; Target='480'; Mode='summary'; Seconds=120; Load=$true}
)
foreach ($taskCase in $taskCases) {
    $taskHelpers = @()
    try {
        if ($taskCase.Load) {
            $taskLoadRoot = Join-Path $OutputDirectory '_cpu-load'
            New-Item -ItemType Directory -Path $taskLoadRoot | Out-Null
            for ($taskIndex = 0; $taskIndex -lt 2; $taskIndex++) {
                $taskReady = "$taskLoadRoot\ready-$taskIndex.txt"
                $taskResult = "$taskLoadRoot\result-$taskIndex.json"
                $taskHelperArgs = @('-NoProfile', '-File', ('"{0}"' -f "$PSScriptRoot\cpu-load.ps1"), '-Seconds', '120', '-ReadyPath', ('"{0}"' -f $taskReady), '-StartPath', ('"{0}"' -f "$taskLoadRoot\start.txt"), '-ResultPath', ('"{0}"' -f $taskResult))
                $taskProcess = Start-Process -FilePath (Get-Command pwsh).Source -ArgumentList $taskHelperArgs -WindowStyle Hidden -PassThru -RedirectStandardOutput "$taskLoadRoot\stdout-$taskIndex.txt" -RedirectStandardError "$taskLoadRoot\stderr-$taskIndex.txt"
                $taskHelpers += [pscustomobject]@{Process=$taskProcess; Ready=$taskReady; Result=$taskResult}
            }
            $taskWait = [Diagnostics.Stopwatch]::StartNew()
            while (@($taskHelpers | Where-Object { !(Test-Path -LiteralPath $_.Ready) }).Count -gt 0) {
                if ($taskWait.Elapsed.TotalSeconds -gt 30 -or @($taskHelpers | Where-Object { $_.Process.HasExited }).Count -gt 0) { throw 'CPU helper startup failed' }
                Start-Sleep -Milliseconds 100
            }
            Set-Content -LiteralPath "$taskLoadRoot\start.txt" -Value 'start'
        }
        $taskArguments = @('--input', $CaptureEndpointId, '--output', $RenderEndpointId, '--seconds', [string]$taskCase.Seconds, '--gain', '0', '--mode', $taskCase.Mode, '--period-ms', $taskCase.Period, '--render-target-frames', $taskCase.Target, '--wake-margin-frames', '96', '--callback-budget-frames', '48')
        & $ProbePath @taskArguments > "$OutputDirectory\$($taskCase.Name).json" 2> "$OutputDirectory\$($taskCase.Name).stderr.txt"
        if ($LASTEXITCODE -ne 0) { throw "$($taskCase.Name) probe failed; preserve raw output" }
        if ($taskCase.Load) {
            $taskWorkers = foreach ($taskHelper in $taskHelpers) {
                if (!$taskHelper.Process.WaitForExit(10000)) { throw 'CPU helper exceeded its bounded duration' }
                $taskWorker = Get-Content -LiteralPath $taskHelper.Result -Raw | ConvertFrom-Json
                $taskWorker | Add-Member -NotePropertyName exit_code -NotePropertyValue $taskHelper.Process.ExitCode
                $taskWorker
            }
            [ordered]@{kind='two_busy_cpu_workers'; logical_processors=[Environment]::ProcessorCount; workers=@($taskWorkers)} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath "$OutputDirectory\cpu-load-results.json" -Encoding utf8
        }
        Write-Output "$($taskCase.Name) finished"
    } finally {
        foreach ($taskHelper in $taskHelpers) {
            if (!$taskHelper.Process.HasExited) { Stop-Process -Id $taskHelper.Process.Id }
            $taskHelper.Process.Dispose()
        }
    }
}
$taskRejections = foreach ($taskCase in @(
    @{Name='rejected-small-479'; Period='5'; Target='479'; Expected='render target cannot cover period, wake margin and callback budget'},
    @{Name='rejected-default-624'; Period='default'; Target='624'; Expected='input target cannot cover render demand, capture gap, jitter and lookahead'}
)) {
    $taskClock = [Diagnostics.Stopwatch]::StartNew()
    $taskArguments = @('--input', $CaptureEndpointId, '--output', $RenderEndpointId, '--seconds', '5', '--gain', '0', '--mode', 'summary', '--period-ms', $taskCase.Period, '--render-target-frames', $taskCase.Target, '--wake-margin-frames', '96', '--callback-budget-frames', '48')
    & $ProbePath @taskArguments > "$OutputDirectory\$($taskCase.Name).stdout.txt" 2> "$OutputDirectory\$($taskCase.Name).txt"
    [pscustomobject]@{case=$taskCase.Name; exit_code=$LASTEXITCODE; elapsed_seconds=$taskClock.Elapsed.TotalSeconds; expected_message=$taskCase.Expected; period=$taskCase.Period; render_target=$taskCase.Target}
}
$taskRejections | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath "$OutputDirectory\rejections.json" -Encoding utf8
& "$PSScriptRoot\verify.ps1" -EvidenceDirectory $OutputDirectory -UpdateCsv
