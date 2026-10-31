# Target-only regression. Produce the input with the installed-driver probe:
# vad_native --exercise --seconds=1 --timer > report.json
# pwsh -NoProfile -File scripts/tests/vad-native-playback.tests.ps1 -ReportPath report.json
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ReportPath
)
$ErrorActionPreference = 'Stop'
$report = Get-Content -LiteralPath $ReportPath -Raw | ConvertFrom-Json
if ($report.schema_version -ne 2 -or
    $report.native_mode -notin @('exclusive-timer', 'exclusive-event')) {
    throw 'Expected a complete native VAD matrix report.'
}
if (($report.defaults_before | ConvertTo-Json -Compress) -ne
    ($report.defaults_after | ConvertTo-Json -Compress) -or
    @($report.defaults_before).Count -ne 6) {
    throw 'System default-device identities changed or are missing.'
}
$cases = @($report.cases | Where-Object { $_.intent -eq 'Playback' })
if ($cases.Count -ne 4) {
    throw "Expected four playback cases, found $($cases.Count)."
}
foreach ($channels in @(1, 2)) {
    foreach ($format in @('Pcm16', 'Float32')) {
        $matches = @($cases | Where-Object {
            $_.channels -eq $channels -and $_.format -eq $format
        })
        if ($matches.Count -ne 1) {
            throw "Missing or repeated playback case: $channels/$format."
        }
        $case = $matches[0]
        if ($case.observation.signal_frames -le 0 -or
            $case.observation.silent_frames -ne 0 -or
            $case.microphone_dropped_packets -ne 0 -or
            $case.native_discontinuities -ne 0) {
            throw ("Playback quality failed: {0}/{1}, signal={2}, silent={3}, drops={4}, discontinuities={5}" -f
                $channels, $format, $case.observation.signal_frames,
                $case.observation.silent_frames, $case.microphone_dropped_packets,
                $case.native_discontinuities)
        }
    }
}
Write-Output "PASS: four $($report.native_mode) playback cases have matching PCM and zero quality failures."
