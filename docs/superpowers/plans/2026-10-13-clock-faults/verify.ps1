param(
    [string]$Csv = (Join-Path $PSScriptRoot 'results-120.csv'),
    [ValidateRange(120, 7200)][int]$Seconds = 120
)

$ErrorActionPreference = 'Stop'
$rows = @(Import-Csv -LiteralPath $Csv)
if ($rows.Count -ne 144) { throw "Expected 144 cases, got $($rows.Count)." }
$columns = @('fault', 'input_rate', 'capture_period', 'render_period', 'input_target',
    'input_capacity', 'render_target', 'capture_delay_frames', 'render_delay_frames',
    'late_burst_packets', 'burst_cycle_packets', 'max_segment', 'drift_ppm',
    'phase_output_frames', 'render_first', 'seconds', 'short_callbacks',
    'stable_short_callbacks', 'source_xruns', 'stable_source_xruns',
    'render_missing_frames', 'input_underrun_frames', 'dropped_frames',
    'backlog_recoveries', 'backlog_discarded_frames', 'resets', 'recovery_callbacks',
    'last_short_after_resume_frames', 'low_water_guard_frames')
if (($rows[0].PSObject.Properties.Name -join ',') -cne ($columns -join ',')) {
    throw 'Unexpected CSV columns.'
}
$keys = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
$counts = @{ burst = 0; capture_stall = 0; render_pause = 0 }
$totals = [ordered]@{
    cases = 0
    seconds_per_case = $Seconds
    short_callbacks = [uint64]0
    stable_short_callbacks = [uint64]0
    source_xruns = [uint64]0
    stable_source_xruns = [uint64]0
    render_missing_frames = [uint64]0
    input_underrun_frames = [uint64]0
    dropped_frames = [uint64]0
    backlog_recoveries = [uint64]0
    backlog_discarded_frames = [uint64]0
    resets = [uint64]0
    low_water_guard_frames = [uint64]0
    maximum_last_short_after_resume_frames = [uint64]0
}
foreach ($row in $rows) {
    foreach ($column in $columns) {
        if ([string]::IsNullOrWhiteSpace($row.$column)) { throw "Missing ${column}." }
    }
    if ($row.fault -cnotin @('burst', 'capture_stall', 'render_pause') -or
        $row.input_rate -cnotin @('44100', '48000') -or
        $row.drift_ppm -cnotin @('-1000', '0', '1000') -or
        $row.phase_output_frames -cnotin @('0', '84', '168', '252') -or
        $row.render_first -cnotin @('true', 'false')) {
        throw 'Unexpected matrix coordinates.'
    }
    $key = "$($row.fault)|$($row.input_rate)|$($row.drift_ppm)|$($row.phase_output_frames)|$($row.render_first)"
    if (-not $keys.Add($key)) { throw "Duplicate case: $key" }
    $expectedPeriod = if ($row.input_rate -eq '44100') { '309' } else { '336' }
    $expectedTarget = if ($row.fault -eq 'capture_stall') { '2048' } else { '1024' }
    $expectedDelay = if ($row.fault -eq 'capture_stall') { '1024' } else { '96' }
    if ($row.capture_period -cne $expectedPeriod -or $row.input_target -cne $expectedTarget -or
        $row.capture_delay_frames -cne $expectedDelay -or $row.render_period -cne '336' -or
        $row.input_capacity -cne '8192' -or $row.render_target -cne '480' -or
        $row.render_delay_frames -cne '96' -or $row.late_burst_packets -cne '8' -or
        $row.burst_cycle_packets -cne '17' -or $row.max_segment -cne '256' -or
        $row.seconds -cne [string]$Seconds) {
        throw "Unexpected timing or queue parameters: $key"
    }
    foreach ($field in $columns[16..28]) {
        $parsed = [uint64]0
        if (-not [uint64]::TryParse($row.$field,
                [System.Globalization.NumberStyles]::None,
                [System.Globalization.CultureInfo]::InvariantCulture, [ref]$parsed)) {
            throw "Invalid unsigned ${field}: $key"
        }
    }
    foreach ($field in @('stable_short_callbacks', 'stable_source_xruns', 'input_underrun_frames')) {
        if ([uint64]$row.$field -ne 0) { throw "Unexpected ${field}: $key" }
    }
    if ([uint64]$row.low_water_guard_frames -gt [uint64]$Seconds * [uint64]48000) {
        throw "Invalid guard count: $key"
    }
    if ($row.fault -eq 'render_pause') {
        $minimumDiscard = if ($row.input_rate -eq '44100') { 6859 } else { 6832 }
        if ([uint64]$row.short_callbacks -notin @(1, 2) -or $row.source_xruns -cne '1' -or
            $row.resets -cne '2' -or $row.render_missing_frames -cne '9408' -or
            [uint64]$row.dropped_frames -eq 0 -or $row.backlog_recoveries -cne '1' -or
            [uint64]$row.backlog_discarded_frames -lt [uint64]$minimumDiscard -or
            [uint64]$row.backlog_discarded_frames -gt 7170 -or
            [uint64]$row.recovery_callbacks -lt 1 -or [uint64]$row.recovery_callbacks -gt 11 -or
            [uint64]$row.last_short_after_resume_frames -ge 3360) {
            throw "Unexpected pause/recovery result: $key"
        }
    } else {
        foreach ($field in @('short_callbacks', 'source_xruns', 'render_missing_frames',
                'dropped_frames', 'backlog_recoveries', 'backlog_discarded_frames',
                'resets', 'recovery_callbacks', 'last_short_after_resume_frames')) {
            if ([uint64]$row.$field -ne 0) { throw "Unexpected ${field}: $key" }
        }
    }
    foreach ($field in @('short_callbacks', 'stable_short_callbacks', 'source_xruns',
            'stable_source_xruns', 'render_missing_frames', 'input_underrun_frames',
            'dropped_frames', 'backlog_recoveries', 'backlog_discarded_frames',
            'resets', 'low_water_guard_frames')) {
        $totals[$field] += [uint64]$row.$field
    }
    $totals.maximum_last_short_after_resume_frames = [Math]::Max(
        $totals.maximum_last_short_after_resume_frames,
        [uint64]$row.last_short_after_resume_frames)
    $counts[$row.fault] += 1
}
foreach ($fault in $counts.Keys) {
    if ($counts[$fault] -ne 48) { throw "Expected 48 ${fault} cases." }
}
$totals.cases = $rows.Count
$totals | ConvertTo-Json
