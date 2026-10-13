param(
    [string]$Csv = (Join-Path $PSScriptRoot 'results-120.csv'),
    [ValidateRange(120, 7200)][int]$Seconds = 120
)

$ErrorActionPreference = 'Stop'
$rows = @(Import-Csv -LiteralPath $Csv)
if ($rows.Count -ne 48) { throw "Expected 48 cases, got $($rows.Count)." }
$columns = @('input_rate', 'capture_period', 'render_period', 'input_target', 'max_segment',
    'drift_ppm', 'late_output_frames', 'render_first', 'phase_output_frames', 'seconds',
    'short_callbacks', 'underrun_frames', 'dropped_frames', 'backlog_recoveries',
    'low_water_guard_frames', 'mean_correction_ppm', 'last_correction_ppm', 'ending_inventory_frames')
if (($rows[0].PSObject.Properties.Name -join ',') -cne ($columns -join ',')) {
    throw 'Unexpected CSV columns.'
}
$culture = [System.Globalization.CultureInfo]::InvariantCulture
$keys = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
$guardFrames = [uint64]0
$maximumError = 0.0
foreach ($row in $rows) {
    foreach ($column in $columns) {
        if ([string]::IsNullOrWhiteSpace($row.$column)) { throw "Missing ${column}." }
    }
    if ($row.input_rate -cnotin @('44100', '48000') -or
        $row.drift_ppm -cnotin @('-1000', '0', '1000') -or
        $row.render_first -cnotin @('true', 'false') -or
        $row.phase_output_frames -cnotin @('0', '84', '168', '252')) {
        throw 'Unexpected matrix coordinates.'
    }
    $key = "$($row.input_rate)|$($row.drift_ppm)|$($row.render_first)|$($row.phase_output_frames)"
    if (-not $keys.Add($key)) { throw "Duplicate case: $key" }
    $expectedCapturePeriod = if ($row.input_rate -eq '44100') { '309' } else { '336' }
    if ($row.capture_period -ne $expectedCapturePeriod -or $row.render_period -ne '336' -or
        $row.input_target -ne '1024' -or $row.max_segment -ne '256' -or
        $row.late_output_frames -ne '96' -or [int]$row.seconds -ne $Seconds) {
        throw "Unexpected timing or queue parameters: $key"
    }
    foreach ($field in @('short_callbacks', 'underrun_frames', 'dropped_frames', 'backlog_recoveries')) {
        if ([uint64]$row.$field -ne 0) { throw "Nonzero ${field}: $key" }
    }
    $guarded = [uint64]$row.low_water_guard_frames
    if ($guarded -gt [uint64]$Seconds * [uint64]48000) { throw "Invalid guard count: $key" }
    $mean = [Convert]::ToDouble($row.mean_correction_ppm, $culture)
    $last = [Convert]::ToDouble($row.last_correction_ppm, $culture)
    $inventory = [Convert]::ToDouble($row.ending_inventory_frames, $culture)
    foreach ($value in @($mean, $last, $inventory)) {
        if ([double]::IsNaN($value) -or [double]::IsInfinity($value)) {
            throw "Nonfinite result: $key"
        }
    }
    $errorPpm = [Math]::Abs($mean - [Convert]::ToDouble($row.drift_ppm, $culture))
    if ($errorPpm -ge 300.0 -or [Math]::Abs($last) -gt 2000.0 -or
        $inventory -lt 1.0 -or $inventory -gt 8194.0) {
        throw "Invalid controller or inventory result: $key"
    }
    $maximumError = [Math]::Max($maximumError, $errorPpm)
    $guardFrames += $guarded
}
[ordered]@{
    cases = $rows.Count
    seconds_per_case = $Seconds
    maximum_mean_error_ppm = $maximumError
    low_water_guard_frames = $guardFrames
    short_callbacks = 0
    underrun_frames = 0
    dropped_frames = 0
    backlog_recoveries = 0
} | ConvertTo-Json
