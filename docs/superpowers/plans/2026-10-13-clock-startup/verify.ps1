param(
    [string]$Csv = (Join-Path $PSScriptRoot 'results-120.csv'),
    [ValidateRange(120, 7200)][int]$Seconds = 120
)

$ErrorActionPreference = 'Stop'
$rows = @(Import-Csv -LiteralPath $Csv)
if ($rows.Count -ne 90) { throw "Expected 90 cases, got $($rows.Count)." }
$columns = @('input_rate', 'capture_period', 'render_period', 'input_target',
    'render_target', 'native_capacity', 'drift_ppm', 'phase_output_frames', 'start_order',
    'max_segment', 'seconds', 'first_audio_at_output_frame', 'startup_partial_callbacks',
    'short_callbacks', 'source_xruns', 'render_missing_frames', 'priming_frames',
    'prime_discarded_frames', 'underrun_frames', 'dropped_frames', 'resets',
    'low_water_guard_frames', 'last_correction_ppm')
if (($rows[0].PSObject.Properties.Name -join ',') -cne ($columns -join ',')) {
    throw 'Unexpected CSV columns.'
}
$culture = [System.Globalization.CultureInfo]::InvariantCulture
$keys = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
$guardFrames = [uint64]0
$primeFrames = [uint64]0
$discardedFrames = [uint64]0
$maximumStartWait = [uint64]0
foreach ($row in $rows) {
    foreach ($column in $columns) {
        if ([string]::IsNullOrWhiteSpace($row.$column)) { throw "Missing ${column}." }
    }
    if ($row.input_rate -cnotin @('44100', '48000') -or
        $row.render_period -cnotin @('336', '480') -or
        $row.drift_ppm -cnotin @('-1000', '0', '1000') -or
        $row.start_order -cnotin @('together', 'capture_first', 'render_first')) {
        throw 'Unexpected matrix coordinates.'
    }
    $period = [uint64]$row.render_period
    $phases = if ($period -eq 336) { @('0', '84', '168', '335') } else { @('479') }
    if ($row.phase_output_frames -cnotin $phases) { throw 'Unexpected phase.' }
    $key = "$($row.input_rate)|$period|$($row.drift_ppm)|$($row.phase_output_frames)|$($row.start_order)"
    if (-not $keys.Add($key)) { throw "Duplicate case: $key" }
    $capturePeriod = [uint64][Math]::Ceiling($period * [uint64]$row.input_rate / 48000.0)
    $target = if ($period -eq 336) { '1024' } else { '1280' }
    if ([uint64]$row.capture_period -ne $capturePeriod -or $row.input_target -cne $target -or
        [uint64]$row.render_target -ne $period + 144 -or $row.native_capacity -cne '1056' -or
        $row.max_segment -cne '256' -or [int]$row.seconds -ne $Seconds) {
        throw "Unexpected timing or queue parameters: $key"
    }
    foreach ($field in @('startup_partial_callbacks', 'short_callbacks', 'source_xruns',
        'render_missing_frames', 'underrun_frames', 'dropped_frames', 'resets')) {
        if ([uint64]$row.$field -ne 0) { throw "Nonzero ${field}: $key" }
    }
    $bothStarted = if ($row.start_order -ceq 'together') { [uint64]0 } else { 12 * $period }
    $firstAudio = [uint64]$row.first_audio_at_output_frame
    if ($firstAudio -lt $bothStarted -or $firstAudio -gt $bothStarted + 12 * $period) {
        throw "Invalid first-audio bound: $key"
    }
    $primed = [uint64]$row.priming_frames
    $discarded = [uint64]$row.prime_discarded_frames
    $guarded = [uint64]$row.low_water_guard_frames
    if ($primed -gt [uint64]$Seconds * 48000 -or $discarded -gt 8192 -or
        $guarded -gt [uint64]$Seconds * 48000) { throw "Invalid frame count: $key" }
    if ($row.start_order -ceq 'capture_first' -and ($discarded -eq 0 -or $primed -ne 0)) {
        throw "Capture-first startup must trim its accumulated input: $key"
    }
    if ($row.start_order -ceq 'render_first' -and $primed -eq 0) {
        throw "Render-first startup must expose its initial silence: $key"
    }
    $correction = [Convert]::ToDouble($row.last_correction_ppm, $culture)
    if ([double]::IsNaN($correction) -or [double]::IsInfinity($correction) -or
        [Math]::Abs($correction) -gt 2000.0) { throw "Invalid correction: $key" }
    $maximumStartWait = [Math]::Max($maximumStartWait, $firstAudio - $bothStarted)
    $primeFrames += $primed
    $discardedFrames += $discarded
    $guardFrames += $guarded
}
[ordered]@{
    cases = $rows.Count
    seconds_per_case = $Seconds
    maximum_first_audio_after_both_started_frames = $maximumStartWait
    priming_frames = $primeFrames
    prime_discarded_frames = $discardedFrames
    low_water_guard_frames = $guardFrames
    startup_partial_callbacks = 0
    short_callbacks = 0
    source_xruns = 0
    render_missing_frames = 0
    underrun_frames = 0
    dropped_frames = 0
    resets = 0
} | ConvertTo-Json
