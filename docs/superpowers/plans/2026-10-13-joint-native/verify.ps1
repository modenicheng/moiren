param([switch]$UpdateCsv, [string]$EvidenceDirectory = $PSScriptRoot)
$ErrorActionPreference = 'Stop'
$taskCases = @(
    'compatible-default-summary', 'compatible-small-summary',
    'joint-disabled', 'joint-summary', 'joint-trace',
    'compatible-small-summary-120s', 'joint-summary-120s', 'joint-load-summary-120s'
)
$taskRows = foreach ($taskCase in $taskCases) {
    $taskReport = Get-Content -LiteralPath (Join-Path $EvidenceDirectory "$taskCase.json") -Raw | ConvertFrom-Json
    if ($taskReport.schema_version -ne 2 -or $taskReport.capture.schema_version -ne 6 -or $taskReport.render.schema_version -ne 5) { throw "$taskCase schema mismatch" }
    if ($taskReport.measurement_method -ne 'unknown' -or $taskReport.gain -ne 0) { throw "$taskCase unexpected measurement or gain" }
    if ($taskReport.capture.failure -or $taskReport.render.failure -or !$taskReport.capture.stop_succeeded -or !$taskReport.render.stop_succeeded) { throw "$taskCase owner failure" }
    if (!$taskReport.capture.stream_started -or !$taskReport.render.stream_started) { throw "$taskCase was not activated" }
    $taskPeriod = if ($taskCase -eq 'compatible-default-summary') { 480 } else { 336 }
    $taskCapacity = if ($taskCase -eq 'compatible-default-summary') { 1056 } else { 738 }
    if ($taskReport.capture.sample_rate -ne 48000 -or $taskReport.capture.channels -ne 1 -or $taskReport.render.sample_rate -ne 48000 -or $taskReport.render.channels -ne 2) { throw "$taskCase native format mismatch" }
    if ($taskReport.capture.shared_period.actual_period_frames -ne $taskPeriod -or $taskReport.render.engine_period_frames -ne $taskPeriod -or $taskReport.render.buffer_frames -ne $taskCapacity) { throw "$taskCase native timing mismatch" }
    $taskInput = $taskReport.capture.input_queue
    $taskQueue = $taskReport.render.queue
    if ($taskInput.capacity_frames -ne 8192 -or $taskInput.prime_frames -ne $taskInput.servo_reference_frames) { throw "$taskCase input metadata mismatch" }
    if ($taskQueue.target_frames -ne $taskReport.render.stats.primed_frames) { throw "$taskCase prefill mismatch" }
    if ($taskCase.StartsWith('joint-')) {
        if ($taskInput.prime_frames -ne 1024 -or $taskInput.low_water_frames -ne 482 -or $taskInput.modeled_minimum_frames -ne 914 -or $taskQueue.target_frames -ne 480 -or $taskQueue.deadline_budget_frames -ne 480) { throw "$taskCase custom budget mismatch" }
    } elseif ($taskInput.prime_frames -ne 2048 -or $taskQueue.target_frames -ne $taskReport.render.buffer_frames) { throw "$taskCase compatible budget mismatch" }
    foreach ($taskCount in @($taskReport.bridge.underrun_frames, $taskReport.bridge.dropped_frames, $taskReport.render.stats.bridge_shortfall_frames, $taskReport.render.stats.empty_padding_wakes)) {
        if ($taskCount -ne 0) { throw "$taskCase audio failure count $taskCount" }
    }
    [pscustomobject][ordered]@{
        case = $taskCase
        mode = $taskReport.render.latency.mode
        seconds = $taskReport.render.requested_seconds
        capture_period = $taskReport.capture.shared_period.actual_period_frames
        render_period = $taskReport.render.engine_period_frames
        render_capacity = $taskReport.render.buffer_frames
        input_capacity = $taskInput.capacity_frames
        input_prime = $taskInput.prime_frames
        input_servo = $taskInput.servo_reference_frames
        input_low_water = $taskInput.low_water_frames
        input_high_water = $taskInput.high_water_frames
        input_minimum = $taskInput.modeled_minimum_frames
        render_target = $taskQueue.target_frames
        prefill = $taskReport.render.stats.primed_frames
        render_elapsed = $taskReport.render.elapsed_seconds
        callback_p99_100ns = $taskReport.render.latency.callback_duration_100ns.p99_upper_bound
        callback_max_100ns = $taskReport.render.latency.callback_duration_100ns.max
        wake_interval_max_100ns = $taskReport.render.latency.wake_interval_100ns.max
        padding_min = $taskReport.render.latency.padding_before_frames.min
        padding_max = $taskReport.render.latency.padding_before_frames.max
        max_demand = $taskReport.render.latency.demand_frames.max
        bridge_min_fill = $taskReport.bridge.min_fill_frames
        bridge_max_fill = $taskReport.bridge.max_fill_frames
        bridge_final_fill = $taskReport.bridge.fill_frames
        bridge_correction_ppm = $taskReport.bridge.correction_ppm
        bridge_priming_frames = $taskReport.bridge.priming_frames
        bridge_prime_discarded_frames = $taskReport.bridge.prime_discarded_frames
        bridge_resets = $taskReport.bridge.resets
        bridge_discontinuities = $taskReport.bridge.discontinuities
        bridge_underrun_frames = $taskReport.bridge.underrun_frames
        bridge_dropped_frames = $taskReport.bridge.dropped_frames
        render_shortfall_frames = $taskReport.render.stats.bridge_shortfall_frames
        render_empty_padding_wakes = $taskReport.render.stats.empty_padding_wakes
        capture_timeout_wakes = $taskReport.capture.timeout_wakes
        render_timeout_wakes = $taskReport.render.stats.timeout_wakes
        capture_trace_dropped = $taskReport.capture.latency.trace_dropped
        render_trace_dropped = $taskReport.render.latency.trace_dropped
    }
}
foreach ($taskRow in $taskRows) {
    foreach ($taskProperty in $taskRow.PSObject.Properties) {
        $taskProperty.Value = [Convert]::ToString([object]$taskProperty.Value, [Globalization.CultureInfo]::InvariantCulture)
    }
}
$taskCsvPath = Join-Path $EvidenceDirectory 'results.csv'
if ($UpdateCsv) { $taskRows | Export-Csv -LiteralPath $taskCsvPath -NoTypeInformation -Encoding utf8 }
$taskCsv = @(Import-Csv -LiteralPath $taskCsvPath)
if ($taskCsv.Count -ne $taskRows.Count) { throw 'CSV case count mismatch' }
for ($taskIndex = 0; $taskIndex -lt $taskRows.Count; $taskIndex++) {
    foreach ($taskProperty in $taskRows[$taskIndex].PSObject.Properties) {
        if ([string]$taskCsv[$taskIndex].($taskProperty.Name) -cne [string]$taskProperty.Value) { throw "$($taskRows[$taskIndex].case) CSV mismatch in $($taskProperty.Name)" }
    }
}
$taskLoad = Get-Content -LiteralPath (Join-Path $EvidenceDirectory 'cpu-load-results.json') -Raw | ConvertFrom-Json
if ($taskLoad.workers.Count -ne 2) { throw 'Expected two CPU load workers' }
foreach ($taskWorker in $taskLoad.workers) {
    if ($taskWorker.exit_code -ne 0 -or $taskWorker.seconds -ne 120 -or $taskWorker.elapsed_seconds -lt 120) { throw 'CPU load worker failed or ended early' }
}
$taskRejections = Get-Content -LiteralPath (Join-Path $EvidenceDirectory 'rejections.json') -Raw | ConvertFrom-Json
if ($taskRejections.Count -ne 2) { throw 'Expected two admission rejections' }
foreach ($taskRejection in $taskRejections) {
    if ($taskRejection.exit_code -eq 0 -or $taskRejection.elapsed_seconds -gt 5) { throw 'Expected a prompt pre-activation rejection' }
    $taskText = Get-Content -LiteralPath (Join-Path $EvidenceDirectory "$($taskRejection.case).txt") -Raw
    if (!$taskText.Contains($taskRejection.expected_message)) { throw 'Missing expected admission rejection' }
}
Write-Output "Verified $($taskRows.Count) joint hardware reports, every CSV field, two CPU workers and $($taskRejections.Count) admission rejections"
