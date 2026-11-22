# Offline evidence evaluation only. No native recording/source or hardware proof.
# Requires PowerShell 7 for literal native argument handling and bounded reads.
# Reports are supplied evidence; this wrapper does not generate passing reports.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ProbeExecutable,
    [Parameter(Mandatory)][string]$ExpectedProbeSha256,
    [Parameter(Mandatory)][string]$ReportsPath,
    [Parameter(Mandatory)][string]$ExpectationPath,
    [Parameter(Mandatory)][string]$CurrentBinaryPath,
    [Parameter(Mandatory)][string]$CurrentConfigurationPath,
    [Parameter(Mandatory)][string]$CalibrationPath,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateSet('experimental', 'default')][string]$Level = 'experimental',
    [ValidateRange(1, 1073741824)][long]$MaxInputBytes = 16777216,
    [ValidateRange(1, 1000000)][int]$MaxRuns = 128
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 or newer is required' }
$PSNativeCommandUseErrorActionPreference = $false
$PSNativeCommandArgumentPassing = 'Standard'

function Read-BoundedBytes([string]$Path, [long]$Limit) {
    $stream = [IO.File]::OpenRead($Path)
    try {
        if ($stream.Length -gt $Limit) { throw "Input byte cap exceeded: $Path" }
        $memory = [IO.MemoryStream]::new()
        try {
            $buffer = [byte[]]::new(8192)
            while (($read = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                if ($memory.Length + $read -gt $Limit) { throw "Input grew beyond byte cap: $Path" }
                $memory.Write($buffer, 0, $read)
            }
            return ,$memory.ToArray()
        } finally { $memory.Dispose() }
    } finally { $stream.Dispose() }
}
function Write-Json([string]$Path, $Value) {
    [IO.File]::WriteAllText($Path, (ConvertTo-Json -InputObject $Value -Depth 40), [Text.UTF8Encoding]::new($false))
}
function File-Digest([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Evidence directory already exists' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$receipt = [ordered]@{
    schema_version = 1; mode = 'offline_report_evaluation'; status = 'failed'; cli_exit_code = $null
    started_utc = [DateTime]::UtcNow.ToString('o'); error = $null; verified_hashes = $null
    gaps = @('No live recording/source, calibration procedure, waveform continuity validation or complete Stage 7 harness is performed.')
}
try {
    foreach ($name in @('ProbeExecutable', 'ReportsPath', 'ExpectationPath', 'CurrentBinaryPath', 'CurrentConfigurationPath', 'CalibrationPath')) {
        $resolved = (Resolve-Path -LiteralPath (Get-Variable -Name $name -ValueOnly)).Path
        Set-Variable -Name $name -Value $resolved
    }
    $reportBytes = Read-BoundedBytes $ReportsPath $MaxInputBytes
    $expectationBytes = Read-BoundedBytes $ExpectationPath $MaxInputBytes
    # Retain exact original inputs even if independent hash preflight refuses.
    [IO.File]::WriteAllBytes((Join-Path $OutputDirectory 'reports.json'), $reportBytes)
    [IO.File]::WriteAllBytes((Join-Path $OutputDirectory 'expectation.original.json'), $expectationBytes)
    $expected = [Text.Encoding]::UTF8.GetString($expectationBytes).TrimStart([char]0xfeff) | ConvertFrom-Json -AsHashtable
    $hashes = [ordered]@{
        probe_sha256 = File-Digest $ProbeExecutable
        binary_sha256 = File-Digest $CurrentBinaryPath
        configuration_sha256 = File-Digest $CurrentConfigurationPath
        calibration_sha256 = File-Digest $CalibrationPath
        reports_sha256 = File-Digest (Join-Path $OutputDirectory 'reports.json')
        expectation_sha256 = File-Digest (Join-Path $OutputDirectory 'expectation.original.json')
    }
    $receipt.verified_hashes = $hashes
    if ($ExpectedProbeSha256 -notmatch '^[a-fA-F0-9]{64}$' -or $hashes.probe_sha256 -cne $ExpectedProbeSha256.ToLowerInvariant()) {
        throw 'Probe executable hash does not match independent expected SHA256'
    }
    foreach ($entry in @(
        @{Field = 'binary_hash'; Actual = $hashes.binary_sha256; Label = 'binary hash'},
        @{Field = 'current_config_hash'; Actual = $hashes.configuration_sha256; Label = 'configuration hash'},
        @{Field = 'calibration_hash'; Actual = $hashes.calibration_sha256; Label = 'calibration hash'}
    )) {
        if (-not $expected.ContainsKey($entry.Field) -or [string]$expected[$entry.Field] -cne $entry.Actual) {
            throw "Current $($entry.Label) does not match independent expectation"
        }
    }
    # Refresh evaluation time without rewriting collection evidence or hashes.
    $expected.evaluated_at_unix_seconds = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $currentExpectation = Join-Path $OutputDirectory 'expectation.current.json'
    Write-Json $currentExpectation $expected
    if ((Get-Item -LiteralPath $currentExpectation).Length -gt $MaxInputBytes) { throw 'Current expectation exceeds byte cap' }
    $evaluation = Join-Path $OutputDirectory 'evaluation.json'
    $probeArguments = @(
        'evaluate', '--reports', (Join-Path $OutputDirectory 'reports.json'),
        '--expectation', $currentExpectation, '--level', $Level,
        '--max-input-bytes', $MaxInputBytes.ToString([Globalization.CultureInfo]::InvariantCulture),
        '--max-runs', $MaxRuns.ToString([Globalization.CultureInfo]::InvariantCulture), '--output', $evaluation
    )
    & $ProbeExecutable @probeArguments 1> (Join-Path $OutputDirectory 'stdout.log') 2> (Join-Path $OutputDirectory 'stderr.log')
    $receipt.cli_exit_code = $LASTEXITCODE
    # Refuse artifacts changed during evaluation; hashes identify exact bytes.
    if ((File-Digest $ProbeExecutable) -cne $hashes.probe_sha256 -or
        (File-Digest $CurrentBinaryPath) -cne $hashes.binary_sha256 -or
        (File-Digest $CurrentConfigurationPath) -cne $hashes.configuration_sha256 -or
        (File-Digest $CalibrationPath) -cne $hashes.calibration_sha256) { throw 'Current artifact changed during evaluation' }
    if ($receipt.cli_exit_code -ne 0) {
        $receipt.status = 'unqualified'
        throw "Evidence unqualified: CLI exit $($receipt.cli_exit_code); failed reports retained"
    }
    if (-not (Test-Path -LiteralPath $evaluation)) { throw 'CLI did not retain an evaluation result' }
    # Output contains the bounded report array plus expectation and diagnostics.
    $resultBytes = Read-BoundedBytes $evaluation ($MaxInputBytes * 8 + 65536)
    $result = [Text.Encoding]::UTF8.GetString($resultBytes) | ConvertFrom-Json
    if ($result.qualified -ne $true -or $result.mode -ne 'offline_report_evaluation') { throw 'CLI result is unqualified or has wrong mode' }
    $receipt.status = 'accepted_evidence'
} catch {
    $receipt.error = $_.Exception.Message
    throw
} finally {
    $receipt.completed_utc = [DateTime]::UtcNow.ToString('o')
    Write-Json (Join-Path $OutputDirectory 'receipt.json') $receipt
}
