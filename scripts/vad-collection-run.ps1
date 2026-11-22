# SPDX-License-Identifier: MIT
# Windows PowerShell 5.1. Collection only: exit 0 is not audio qualification.
# OutputDirectory must be new. Pass probe output paths explicitly in ArgumentList;
# only receipt.json/stdout.log/stderr.log are hashed in the runner manifest.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExpectedSha256,
    [Parameter(Mandatory = $true)][ValidatePattern('^[a-fA-F0-9]{40}$')][string]$SourceCommit,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [AllowEmptyCollection()][AllowEmptyString()][string[]]$ArgumentList = @(),
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 90,
    [ValidateRange(1, 67108864)][long]$MaxLogBytes = 16777216,
    [string]$ExpectedComputer = 'DRIVER-TEST'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2

function Quote-WindowsArgument([string]$Value) {
    # ProcessStartInfo.Arguments accepts one Windows command line; quote each
    # logical argument using the CRT backslash-before-quote rules.
    $text = [Text.StringBuilder]::new()
    $null = $text.Append('"')
    $slashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq '\') { $slashes++; continue }
        if ($character -eq '"') {
            $null = $text.Append(('\' * ($slashes * 2 + 1)))
        } else {
            $null = $text.Append(('\' * $slashes))
        }
        $null = $text.Append($character)
        $slashes = 0
    }
    $null = $text.Append(('\' * ($slashes * 2))).Append('"')
    $text.ToString()
}

function Write-NewJson([string]$Path, $Value) {
    # CreateNew preserves existing evidence. Flush/close the child receipt
    # before any manifest hashing/writing can fail. No PS7 encoding names.
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes((ConvertTo-Json -InputObject $Value -Depth 6))
    $file = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $file.Write($bytes, 0, $bytes.Length); $file.Flush($true) }
    finally { $file.Dispose() }
}

$computer = [Environment]::MachineName
if ($computer -ne $ExpectedComputer) { throw 'Unexpected test computer' }
$Executable = (Resolve-Path -LiteralPath $Executable).ProviderPath
if ([IO.Path]::GetExtension($Executable) -ine '.exe' -or
    -not (Test-Path -LiteralPath $Executable -PathType Leaf)) { throw 'Executable must be a Windows .exe file' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Evidence directory already exists' }
$actualHash = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash
if ($actualHash -ne $ExpectedSha256) { throw 'Executable digest does not match the reviewed build' }
$quoted = foreach ($argument in $ArgumentList) {
    if ($null -eq $argument -or $argument -match '[\x00\r\n]') { throw 'Arguments cannot contain null, NUL, CR or LF' }
    Quote-WindowsArgument $argument
}
$commandLine = $quoted -join ' '
if ($commandLine.Length + $Executable.Length + 3 -gt 32766) { throw 'Windows command line exceeds its length limit' }
if (-not ('Moiren.CollectionLogCapV1.Budget' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Threading;
using System.Threading.Tasks;

namespace Moiren.CollectionLogCapV1 {
    public sealed class Budget {
        private readonly object gate = new object();
        private readonly long limit;
        private long reserved;
        private volatile bool exceeded;
        public Budget(long limit) { this.limit = limit; }
        public bool Exceeded { get { return exceeded; } }
        internal int Reserve(int count) {
            lock (gate) {
                if (exceeded) throw Overflow();
                int allowed = (int)Math.Min((long)count, limit - reserved);
                reserved += allowed;
                if (allowed != count) exceeded = true;
                return allowed;
            }
        }
        internal static IOException Overflow() {
            return new IOException("Combined redirected output exceeded MaxLogBytes");
        }
    }

    // Reserve from one shared budget before either file write is issued.
    // Reservations are never refunded after IO failure. This bounds physical
    // output even when the two CopyToAsync continuations run concurrently.
    public sealed class Sink : Stream {
        private readonly Stream destination;
        private readonly Budget budget;
        public Sink(Stream destination, Budget budget) {
            this.destination = destination;
            this.budget = budget;
        }
        public override bool CanRead { get { return false; } }
        public override bool CanSeek { get { return false; } }
        public override bool CanWrite { get { return true; } }
        public override long Length { get { throw new NotSupportedException(); } }
        public override long Position {
            get { throw new NotSupportedException(); }
            set { throw new NotSupportedException(); }
        }
        public override void Flush() { destination.Flush(); }
        public override int Read(byte[] buffer, int offset, int count) { throw new NotSupportedException(); }
        public override long Seek(long offset, SeekOrigin origin) { throw new NotSupportedException(); }
        public override void SetLength(long value) { throw new NotSupportedException(); }
        public override void Write(byte[] buffer, int offset, int count) {
            int allowed = budget.Reserve(count);
            if (allowed != 0) destination.Write(buffer, offset, allowed);
            if (allowed != count) throw Budget.Overflow();
        }
        public override async Task WriteAsync(byte[] buffer, int offset, int count, CancellationToken token) {
            int allowed = budget.Reserve(count);
            if (allowed != 0) await destination.WriteAsync(buffer, offset, allowed, token).ConfigureAwait(false);
            if (allowed != count) throw Budget.Overflow();
        }
        // The runner owns and closes the underlying files after bounded drains.
    }
}
'@
}
$logBudget = [Moiren.CollectionLogCapV1.Budget]::new($MaxLogBytes)
$null = New-Item -ItemType Directory -Path $OutputDirectory
$process = $null
$stdoutFile = $null
$stderrFile = $null
$stdoutCopy = $null
$stderrCopy = $null
$childPid = $null
$exitCode = $null
$failure = $null
$timedOut = $false
$logLimitExceeded = $false
$forced = $false
$confirmed = $false
$outputStable = $false
$cleanupErrors = [Collections.Generic.List[string]]::new()
$started = [DateTime]::UtcNow.ToString('o')
$clock = [Diagnostics.Stopwatch]::StartNew()
try {
    $stdoutFile = [IO.File]::Open(
        (Join-Path $OutputDirectory 'stdout.log'), [IO.FileMode]::CreateNew,
        [IO.FileAccess]::Write, [IO.FileShare]::Read)
    $stderrFile = [IO.File]::Open(
        (Join-Path $OutputDirectory 'stderr.log'), [IO.FileMode]::CreateNew,
        [IO.FileAccess]::Write, [IO.FileShare]::Read)
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Executable
    $start.Arguments = $commandLine
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    if (-not $process.Start()) { throw 'Could not start owned child process' }
    # Process.Start retains the original CreateProcess handle before returning;
    # never reacquire the child by PID after a fast exit.
    $null = $process.Handle
    $childPid = $process.Id
    $stdoutCopy = $process.StandardOutput.BaseStream.CopyToAsync(
        [Moiren.CollectionLogCapV1.Sink]::new($stdoutFile, $logBudget))
    $stderrCopy = $process.StandardError.BaseStream.CopyToAsync(
        [Moiren.CollectionLogCapV1.Sink]::new($stderrFile, $logBudget))
    while ($true) {
        if ($logBudget.Exceeded) { break }
        foreach ($copy in @($stdoutCopy, $stderrCopy)) {
            if ($copy.IsFaulted -or $copy.IsCanceled) { $copy.GetAwaiter().GetResult() }
        }
        $remaining = [int][Math]::Max(0, $TimeoutSeconds * 1000 - $clock.ElapsedMilliseconds)
        if ($remaining -eq 0) { $timedOut = -not $process.WaitForExit(0); break }
        if ($process.WaitForExit([int][Math]::Min(50, $remaining))) { break }
    }
} catch {
    $failure = $_.Exception.Message
} finally {
    if ($null -ne $process) {
        try {
            if (-not $process.HasExited) {
                $forced = $true
                # Only this Process/retained handle: no name/PID lookup or tree kill.
                try { $process.Kill() } catch {
                    if (-not $process.HasExited) { throw }
                }
            }
            $confirmed = $process.WaitForExit(5000)
            if (-not $confirmed) { throw 'Owned child did not confirm exit within 5 seconds' }
            $exitCode = $process.ExitCode
            if ($null -eq $exitCode) { throw 'Owned child exited without a readable ExitCode' }
        } catch { $cleanupErrors.Add($_.Exception.Message) }
        $outputStable = $confirmed -and $null -ne $stdoutCopy -and $null -ne $stderrCopy
        foreach ($copy in @($stdoutCopy, $stderrCopy)) {
            if ($null -eq $copy) { continue }
            try {
                if (-not $copy.Wait(5000)) { throw 'Redirected child output did not drain within 5 seconds' }
                $copy.GetAwaiter().GetResult()
            } catch { $outputStable = $false; $cleanupErrors.Add($_.Exception.Message) }
        }
        try { $process.Dispose() } catch { $outputStable = $false; $cleanupErrors.Add($_.Exception.Message) }
    }
    foreach ($stream in @($stdoutFile, $stderrFile)) {
        if ($null -ne $stream) {
            try { $stream.Dispose() } catch { $outputStable = $false; $cleanupErrors.Add($_.Exception.Message) }
        }
    }
    $logLimitExceeded = $logBudget.Exceeded
    if ($logLimitExceeded) { $outputStable = $false }
    $clock.Stop()
}
$status = 'child_exited'
if ($timedOut) { $status = 'timed_out'; $failure = 'Owned child exceeded TimeoutSeconds' }
elseif ($logLimitExceeded) { $status = 'log_limit_exceeded'; $failure = 'Combined redirected output exceeded MaxLogBytes' }
elseif ($failure -or $cleanupErrors.Count) { $status = 'runner_failed' }
elseif ($exitCode -ne 0) { $status = 'child_failed'; $failure = "Child exited with code $exitCode" }
$receipt = [ordered]@{
    schema_version = 1; status = $status; computer = $computer
    source_commit = $SourceCommit; executable = $Executable; executable_sha256 = $actualHash
    arguments = @($ArgumentList); timeout_seconds = $TimeoutSeconds; runner_pid = $PID; child_pid = $childPid
    started_utc = $started; finished_utc = [DateTime]::UtcNow.ToString('o'); elapsed_seconds = $clock.Elapsed.TotalSeconds
    child_exit_code = $exitCode; timed_out = $timedOut; forced_termination = $forced
    max_log_bytes = $MaxLogBytes; log_limit_exceeded = $logLimitExceeded
    termination_confirmed = $confirmed; error = $failure; cleanup_errors = $cleanupErrors.ToArray()
    output_state = $(if ($outputStable) { 'stable' } else { 'unavailable' })
    artifact_manifest_state = $(if ($outputStable) { 'eligible' } else { 'unavailable' })
}
Write-NewJson (Join-Path $OutputDirectory 'receipt.json') $receipt

# Receipt is immutable from here. Metadata errors must never replace the child
# outcome or remove stdout/stderr/probe artifacts. No recursive artifact sweep.
$metadataError = $null
if (-not $outputStable) {
    # A timed-out copy may still own an in-flight write after Dispose. Retain
    # partial logs, but never hash them without successful drains and closes.
    $metadataError = 'Artifact manifest unavailable: redirected output was not proven stable; receipt retained'
} else {
    try {
        $artifacts = foreach ($name in @('receipt.json', 'stdout.log', 'stderr.log')) {
            $path = Join-Path $OutputDirectory $name
            if (Test-Path -LiteralPath $path -PathType Leaf) {
                [ordered]@{ file = $name; bytes = (Get-Item -LiteralPath $path).Length
                    sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash }
            }
        }
        Write-NewJson (Join-Path $OutputDirectory 'artifact-manifest.json') ([ordered]@{
            schema_version = 1; scope = 'runner_owned_artifacts'; artifacts = @($artifacts)
        })
    } catch { $metadataError = "Artifact manifest failed; receipt retained: $($_.Exception.Message)" }
}
if ($status -ne 'child_exited') { throw "Collection $status`: $failure; $($cleanupErrors -join '; '); $metadataError" }
if ($metadataError) { throw $metadataError }
Write-Output "Collection child exited 0; receipt: $(Join-Path $OutputDirectory 'receipt.json')"
