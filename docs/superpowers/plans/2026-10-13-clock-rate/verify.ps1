param(
    [string]$Csv = (Join-Path $PSScriptRoot 'rate-120.csv'),
    [ValidateSet(120, 7200)][int]$Seconds = 120
)

$ErrorActionPreference = 'Stop'
$rows = @(Import-Csv -LiteralPath $Csv)
if ($rows.Count -ne 16) { throw "Expected 16 cases, got $($rows.Count)." }
$columns = @('input_rate', 'capture_drift_ppm', 'render_drift_ppm', 'invalid_clock',
    'seconds', 'short_callbacks', 'underrun_frames', 'dropped_frames',
    'backlog_recoveries', 'mean_correction_ppm', 'last_correction_ppm',
    'last_capture_rate_ppm')
if (($rows[0].PSObject.Properties.Name -join ',') -cne ($columns -join ',')) {
    throw 'Unexpected CSV columns.'
}
$keys = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
$culture = [System.Globalization.CultureInfo]::InvariantCulture
$maximumError = 0.0
foreach ($row in $rows) {
    foreach ($column in $columns) {
        if ([string]::IsNullOrWhiteSpace($row.$column)) { throw "Missing ${column}." }
    }
    if ($row.input_rate -cnotin @('44100', '48000') -or
        $row.capture_drift_ppm -cnotin @('-1000', '1000') -or
        $row.render_drift_ppm -cnotin @('-500', '500') -or
        $row.invalid_clock -cnotin @('true', 'false') -or
        $row.seconds -cne [string]$Seconds) {
        throw 'Unexpected coordinates or duration.'
    }
    $key = "$($row.input_rate)|$($row.capture_drift_ppm)|$($row.render_drift_ppm)|$($row.invalid_clock)"
    if (-not $keys.Add($key)) { throw "Duplicate case: $key" }
    foreach ($field in @('short_callbacks', 'underrun_frames', 'dropped_frames', 'backlog_recoveries')) {
        if ($row.$field -cnotmatch '^[0-9]+$' -or [uint64]$row.$field -ne 0) {
            throw "Invalid or nonzero ${field}: $key"
        }
    }
    $expectedStamp = if ($row.invalid_clock -ceq 'true') { 'none' } else { $row.capture_drift_ppm }
    if ($row.last_capture_rate_ppm -cne $expectedStamp) {
        throw "Wrong queued capture rate or invalid-clock fallback: $key"
    }
    $mean = [Convert]::ToDouble($row.mean_correction_ppm, $culture)
    $last = [Convert]::ToDouble($row.last_correction_ppm, $culture)
    if (-not [double]::IsFinite($mean) -or -not [double]::IsFinite($last)) {
        throw "Nonfinite controller result: $key"
    }
    $capture = [Convert]::ToDouble($row.capture_drift_ppm, $culture)
    $render = [Convert]::ToDouble($row.render_drift_ppm, $culture)
    $relative = ((1.0 + $capture / 1000000.0) / (1.0 + $render / 1000000.0) - 1.0) * 1000000.0
    $errorPpm = [Math]::Abs($mean - $relative)
    if ($errorPpm -ge 300.0 -or [Math]::Abs($last) -gt 2000.0) {
        throw "Invalid relative correction or total limit: $key"
    }
    $maximumError = [Math]::Max($maximumError, $errorPpm)
}
[ordered]@{
    cases = $rows.Count
    seconds_per_case = $Seconds
    maximum_relative_error_ppm = $maximumError
    valid_capture_clocks = 8
    timestamp_error_fallbacks = 8
    short_callbacks = 0
    underrun_frames = 0
    dropped_frames = 0
    backlog_recoveries = 0
} | ConvertTo-Json
