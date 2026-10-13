$ErrorActionPreference = 'Stop'
$taskRows = @(Import-Csv -LiteralPath (Join-Path $PSScriptRoot 'results.csv'))
if ($taskRows.Count -ne 4) { throw 'Expected four hardware cases' }
foreach ($taskRow in $taskRows) {
    $taskReport = Get-Content -LiteralPath (Join-Path $PSScriptRoot "$($taskRow.case).json") -Raw | ConvertFrom-Json
    $taskExpected = [ordered]@{
        mode = $taskReport.render.latency.mode
        seconds = $taskReport.render.requested_seconds
        capture_period = $taskReport.capture.shared_period.actual_period_frames
        render_period = $taskReport.render.engine_period_frames
        render_capacity = $taskReport.render.buffer_frames
        render_target = $taskReport.render.queue.target_frames
        deadline_budget = $taskReport.render.queue.deadline_budget_frames
        prefill = $taskReport.render.stats.primed_frames
        render_callback_p99_100ns = $taskReport.render.latency.callback_duration_100ns.p99_upper_bound
        render_callback_max_100ns = $taskReport.render.latency.callback_duration_100ns.max
        wake_interval_max_100ns = $taskReport.render.latency.wake_interval_100ns.max
        padding_min = $taskReport.render.latency.padding_before_frames.min
        padding_max = $taskReport.render.latency.padding_before_frames.max
        max_demand = $taskReport.render.latency.demand_frames.max
        zero_demand_wakes = $taskReport.render.stats.zero_demand_wakes
        empty_padding_wakes = $taskReport.render.stats.empty_padding_wakes
        bridge_underrun_frames = $taskReport.bridge.underrun_frames
        bridge_dropped_frames = $taskReport.bridge.dropped_frames
        capture_timeout_wakes = $taskReport.capture.timeout_wakes
        render_timeout_wakes = $taskReport.render.stats.timeout_wakes
        capture_trace_dropped = $taskReport.capture.latency.trace_dropped
        render_trace_dropped = $taskReport.render.latency.trace_dropped
    }
    foreach ($taskKey in $taskExpected.Keys) {
        if ([string]$taskRow.$taskKey -cne [string]$taskExpected[$taskKey]) {
            throw "$($taskRow.case): CSV mismatch in $taskKey"
        }
    }
    if ($taskReport.capture.schema_version -ne 5 -or $taskReport.render.schema_version -ne 5) { throw 'Unexpected owner schema' }
    if ($taskReport.render.queue.target_frames -ne $taskReport.render.stats.primed_frames) { throw 'Prefill mismatch' }
    if ($taskReport.capture.shared_period.actual_period_frames -ne 336 -or $taskReport.render.engine_period_frames -ne 336) { throw 'Unexpected actual period' }
    if ($taskReport.capture.failure -or $taskReport.render.failure -or !$taskReport.capture.stop_succeeded -or !$taskReport.render.stop_succeeded) { throw 'Owner cleanup failure' }
    if ($taskReport.measurement_method -ne 'unknown') { throw 'Unexpected measurement claim' }
    foreach ($taskCount in @($taskReport.bridge.underrun_frames, $taskReport.bridge.dropped_frames, $taskReport.render.stats.bridge_shortfall_frames, $taskReport.render.stats.empty_padding_wakes, $taskReport.capture.timeout_wakes, $taskReport.render.stats.timeout_wakes)) {
        if ($taskCount -ne 0) { throw 'Unexpected audio failure count' }
    }
}
$taskRejection = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'rejected-479.txt') -Raw
if (!$taskRejection.Contains('render target cannot cover period, wake margin and callback budget')) { throw 'Missing expected budget rejection' }
Write-Output "Verified $($taskRows.Count) raw hardware reports against all CSV fields and the expected rejection"
