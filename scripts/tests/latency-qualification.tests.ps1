# Offline runner regressions. No audio device, driver, or live source is opened.
[CmdletBinding()]
param(
    [string]$Runner,
    [string]$ProbeExecutable
)
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($Runner)) {
    $Runner = Join-Path $PSScriptRoot '..\latency-qualification.ps1'
}
$Runner = (Resolve-Path -LiteralPath $Runner).Path
function Require([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Reject([scriptblock]$Action, [string]$Expected) {
    try { & $Action } catch {
        Require ($_.Exception.Message -like "*$Expected*") "Unexpected error: $_"
        return
    }
    throw "Expected rejection containing $Expected"
}

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('moiren-qualification-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
try {
    $config = Join-Path $testRoot 'current config.json'
    $binary = Join-Path $testRoot 'current binary.bin'
    $calibration = Join-Path $testRoot 'current calibration.json'
    $reports = Join-Path $testRoot 'reports.json'
    $expectation = Join-Path $testRoot 'expectation.json'
    [IO.File]::WriteAllText($config, '{"configuration":"genuine-current-test-fixture"}')
    [IO.File]::WriteAllText($binary, 'synthetic test binary artifact')
    [IO.File]::WriteAllText($calibration, '{"calibration":"synthetic-test-only"}')
    [IO.File]::WriteAllText($reports, '[]')
    $probe = Join-Path $testRoot 'not launched.exe'
    [IO.File]::WriteAllText($probe, 'synthetic executable for hash-refusal tests only')
    $format = @{sample_rate = 48000; channels = 2; sample_format = 'f32'}
    $configuration = @{
        source_kind = 'process_loopback'; source_identity = 'fixture-process'; output_endpoint_id = 'fixture-output'
        processing_format = $format; native_input_format = $format; native_output_format = $format; measurement_format = $format
        graph_id = 'input-bus-output'; signal_id = 'marker-v1'; controlled_load_id = 'fixture-load'
    }
    $expected = @{
        source_revision = 'fixture-revision'; binary_hash = (Get-FileHash -LiteralPath $binary).Hash.ToLowerInvariant()
        current_config_hash = (Get-FileHash -LiteralPath $config).Hash.ToLowerInvariant()
        calibration_hash = (Get-FileHash -LiteralPath $calibration).Hash.ToLowerInvariant()
        target_machine_id = 'fixture-machine'; windows_build = 'fixture-build'; evaluated_at_unix_seconds = 1
        configuration = $configuration
    }
    $expected | ConvertTo-Json -Depth 15 | Set-Content -LiteralPath $expectation -Encoding utf8NoBOM
    $arguments = @{
        ProbeExecutable = $probe; ExpectedProbeSha256 = (Get-FileHash -LiteralPath $probe).Hash
        ReportsPath = $reports; ExpectationPath = $expectation; CurrentBinaryPath = $binary
        CurrentConfigurationPath = $config; CalibrationPath = $calibration
        Level = 'experimental'; MaxInputBytes = 1048576; MaxRuns = 8
    }
    [IO.File]::AppendAllText($binary, 'changed')
    $output = Join-Path $testRoot 'mismatch'
    Reject { & $Runner @arguments -OutputDirectory $output } 'binary hash'
    $receipt = Get-Content -LiteralPath (Join-Path $output 'receipt.json') -Raw | ConvertFrom-Json
    Require ($receipt.status -eq 'failed') 'Hash mismatch reported success'
    Require (Test-Path -LiteralPath (Join-Path $output 'reports.json')) 'Failed preflight lost original reports'
    Require (-not (Test-Path -LiteralPath (Join-Path $output 'evaluation.json'))) 'CLI launched after failed hash check'
    Reject { & $Runner @arguments -OutputDirectory $output } 'already exists'

    if ($ProbeExecutable) {
        $ProbeExecutable = (Resolve-Path -LiteralPath $ProbeExecutable).Path
        $arguments.ProbeExecutable = $ProbeExecutable
        $arguments.ExpectedProbeSha256 = (Get-FileHash -LiteralPath $ProbeExecutable).Hash
        $expected.binary_hash = (Get-FileHash -LiteralPath $binary).Hash.ToLowerInvariant()
        $expected | ConvertTo-Json -Depth 15 | Set-Content -LiteralPath $expectation -Encoding utf8NoBOM
        $output = Join-Path $testRoot 'unqualified empty reports'
        Reject { & $Runner @arguments -OutputDirectory $output } 'unqualified'
        $receipt = Get-Content -LiteralPath (Join-Path $output 'receipt.json') -Raw | ConvertFrom-Json
        Require ($receipt.status -eq 'unqualified' -and $receipt.cli_exit_code -eq 1) 'Unqualified CLI exit status lost'
        $result = Get-Content -LiteralPath (Join-Path $output 'evaluation.json') -Raw | ConvertFrom-Json
        Require (-not $result.qualified) 'Empty report set qualified'
        Require ($result.expectation.evaluated_at_unix_seconds -gt 1) 'Wrapper trusted stale evaluation time'

        $run = @{
            schema_version = 1; identity = @{
                run_id = 'synthetic-test-only'; source_revision = $expected.source_revision; binary_hash = $expected.binary_hash
                config_hash = $expected.current_config_hash; machine_id = $expected.target_machine_id; windows_build = $expected.windows_build
            }
            expected_config_hash = $expected.current_config_hash; configuration = $configuration; boundary = 'external_dual_channel'
            calibration = @{
                reference_id = 'synthetic-test-only'; evidence_id = 'synthetic-calibration'; output_endpoint_id = 'fixture-output'
                measurement_format = $format; common_sample_clock = $true; uncertainty_ms = 0.1
                valid_until_unix_seconds = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds() + 3600; calibration_hash = $expected.calibration_hash
            }
            input_queue = @{}; output_queue = @{}; observed_duration_seconds = 1800; observed_measurement_frames = 86400000
            waveform_evidence_id = 'synthetic-test-only'; steady_state = $true; controlled_load = $true; expected_marker_count = 3
            distribution = @{marker_count = 3; sample_rate = 48000; p50_ms = 5; p95_ms = 8; p99_ms = 8; max_ms = 8}
            quality = @{dropped_frames = 0; silent_frames = 0; underrun_frames = 0; discontinuities = 0
                epoch_resets = 0; marker_missing = 0; sample_gap_frames = 0; delivery_bound_violations = 0}
        }
        foreach ($case in @('mismatched', 'uncalibrated', 'short', 'failed', 'passing-fixture')) {
            $candidate = $run | ConvertTo-Json -Depth 20 | ConvertFrom-Json -AsHashtable
            switch ($case) {
                'mismatched' { $candidate.configuration.output_endpoint_id = 'other' }
                'uncalibrated' { $candidate.calibration = $null }
                'short' { $candidate.observed_duration_seconds = 30 }
                'failed' { $candidate.quality.delivery_bound_violations = 1 }
            }
            ConvertTo-Json -InputObject @($candidate) -Depth 20 | Set-Content -LiteralPath $reports -Encoding utf8NoBOM
            $output = Join-Path $testRoot $case
            if ($case -eq 'passing-fixture') {
                & $Runner @arguments -OutputDirectory $output
            } else {
                Reject { & $Runner @arguments -OutputDirectory $output } 'unqualified'
            }
            $receipt = Get-Content -LiteralPath (Join-Path $output 'receipt.json') -Raw | ConvertFrom-Json
            Require (($receipt.status -eq 'accepted_evidence') -eq ($case -eq 'passing-fixture')) "$case receipt wrong"
            Require (Test-Path -LiteralPath (Join-Path $output 'reports.json')) "$case reports not retained"
            Require (Test-Path -LiteralPath (Join-Path $output 'evaluation.json')) "$case evaluation not retained"
        }
    }
    Write-Output 'PASS: hash preflight, immutable output, retained failures; optional real CLI cases completed'
} finally {
    $resolvedRoot = [IO.Path]::GetFullPath($testRoot)
    $tempPrefix = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    if (-not $resolvedRoot.StartsWith($tempPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        -not ([IO.Path]::GetFileName($resolvedRoot)).StartsWith('moiren-qualification-')) { throw 'Unexpected cleanup target' }
    Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
}
