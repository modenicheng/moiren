param(
    [ValidateRange(1, 600)][int]$Seconds = 120,
    [Parameter(Mandatory)][string]$ReadyPath,
    [Parameter(Mandatory)][string]$StartPath,
    [Parameter(Mandatory)][string]$ResultPath
)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @"
using System.Diagnostics;
public static class MoirenLatencyLoad {
    public static ulong Run(int seconds) {
        var clock = Stopwatch.StartNew();
        ulong checksum = 0x123456789abcdef;
        while (clock.Elapsed.TotalSeconds < seconds) {
            for (int iteration = 0; iteration < 100000; iteration++) {
                checksum ^= checksum << 13;
                checksum ^= checksum >> 7;
                checksum ^= checksum << 17;
            }
        }
        return checksum;
    }
}
"@
Set-Content -LiteralPath $ReadyPath -Value 'ready' -Encoding utf8
$taskWait = [Diagnostics.Stopwatch]::StartNew()
while (!(Test-Path -LiteralPath $StartPath)) {
    if ($taskWait.Elapsed.TotalSeconds -gt 30) { throw 'Load start signal timed out' }
    Start-Sleep -Milliseconds 20
}
$taskStarted = Get-Date
$taskClock = [Diagnostics.Stopwatch]::StartNew()
$taskChecksum = [MoirenLatencyLoad]::Run($Seconds)
[ordered]@{
    pid = $PID
    started = $taskStarted.ToString('o')
    seconds = $Seconds
    elapsed_seconds = $taskClock.Elapsed.TotalSeconds
    checksum = [string]$taskChecksum
} | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding utf8
