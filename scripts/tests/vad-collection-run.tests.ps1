# SPDX-License-Identifier: MIT
# Real Windows PowerShell children only; no audio, driver, VM or build needed.
# Keep the small test evidence directory, including failed runner receipts.
[CmdletBinding()]
param([string]$Runner)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2
if ($PSVersionTable.PSVersion.Major -ne 5 -or $PSVersionTable.PSVersion.Minor -ne 1) {
    throw 'Run these regressions with Windows PowerShell 5.1 (powershell.exe)'
}
if (-not $Runner) { $Runner = Join-Path $PSScriptRoot '..\vad-collection-run.ps1' }
$Runner = (Resolve-Path -LiteralPath $Runner).ProviderPath
function Require([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Read-Json([string]$Path) {
    [IO.File]::ReadAllText($Path, [Text.Encoding]::UTF8) | ConvertFrom-Json
}

$testRoot = Join-Path $PSScriptRoot ('..\..\target\collection-run-tests-' + [Guid]::NewGuid().ToString('N'))
$testRoot = [IO.Path]::GetFullPath($testRoot)
$null = New-Item -ItemType Directory -Path $testRoot
$child = Join-Path $testRoot 'child with spaces.ps1'
[IO.File]::WriteAllText($child, @'
param([string]$Mode, [string]$Directory)
$ErrorActionPreference = 'Stop'
[Console]::Out.WriteLine('child stdout retained')
[Console]::Error.WriteLine('child stderr retained')
switch ($Mode) {
    'zero' { exit 0 }
    'nonzero' { exit 37 }
    'metadata-failure' {
        $null = [IO.Directory]::CreateDirectory((Join-Path $Directory 'artifact-manifest.json'))
        exit 37
    }
    'arguments' {
        [IO.File]::WriteAllText((Join-Path $Directory 'arguments.json'),
            (ConvertTo-Json -InputObject @($args)), [Text.UTF8Encoding]::new($false))
        exit 0
    }
    'timeout' {
        [IO.File]::WriteAllText((Join-Path $Directory 'started.pid'), [string]$PID)
        Start-Sleep -Seconds 30
        exit 0
    }
    default { exit 99 }
}
'@, [Text.UTF8Encoding]::new($false))

$nativeSource = Join-Path $testRoot 'fast-exit.c'
$nativeExecutable = Join-Path $testRoot 'fast-exit.exe'
[IO.File]::WriteAllText($nativeSource, @'
#include <stdio.h>
int main(void) {
    fputs("native stdout retained\n", stdout);
    fputs("native stderr retained\n", stderr);
    fflush(stdout);
    fflush(stderr);
    return 37;
}
'@, [Text.UTF8Encoding]::new($false))
$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
if (-not (Test-Path -LiteralPath $compiler -PathType Leaf)) {
    throw 'Native fast-exit regression requires the .NET Framework C# compiler'
}
$csharpSource = Join-Path $testRoot 'fast-exit.cs'
[IO.File]::WriteAllText($csharpSource, @'
using System;
using System.Diagnostics;
using System.Threading;
class FastExit {
    static int Main(string[] args) {
        if (args.Length == 4 && args[0] == "hold-pipes") {
            using (var release = EventWaitHandle.OpenExisting(args[1]))
            using (var ready = EventWaitHandle.OpenExisting(args[2]))
            using (var closed = EventWaitHandle.OpenExisting(args[3])) {
                try {
                    Console.Out.WriteLine("inherited stdout retained");
                    Console.Error.WriteLine("inherited stderr retained");
                    ready.Set();
                    return release.WaitOne(30000) ? 0 : 98;
                } finally {
                    Console.Out.Close();
                    Console.Error.Close();
                    closed.Set();
                }
            }
        }
        Console.Out.WriteLine("native stdout retained");
        Console.Error.WriteLine("native stderr retained");
        if (args.Length == 4 && args[0] == "spawn-holder") {
            var start = new ProcessStartInfo {
                FileName = Environment.GetCommandLineArgs()[0],
                Arguments = "hold-pipes " + args[1] + " " + args[2] + " " + args[3],
                UseShellExecute = false,
                CreateNoWindow = true,
                WindowStyle = ProcessWindowStyle.Hidden
            };
            // Inherit both output handles, then exit while the holder keeps
            // them open. The test releases only this fixture via named events.
            using (var holder = Process.Start(start))
            using (var ready = EventWaitHandle.OpenExisting(args[2])) {
                return ready.WaitOne(5000) ? 0 : 97;
            }
        }
        return 37;
    }
}
'@, [Text.UTF8Encoding]::new($false))
$nativeSource = $csharpSource
$null = & $compiler /nologo /optimize+ "/out:$nativeExecutable" $nativeSource
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $nativeExecutable -PathType Leaf)) {
    throw 'Could not compile the tiny native fast-exit child'
}

$executable = Join-Path $PSHOME 'powershell.exe'
$digest = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash
$common = @{
    Executable = $executable; ExpectedSha256 = $digest; SourceCommit = ('a' * 40)
    ExpectedComputer = [Environment]::MachineName; TimeoutSeconds = 15
}
function Invoke-Case([string]$Name, [string]$Mode, [string[]]$Extra = @(), [int]$Timeout = 15) {
    $output = Join-Path $testRoot $Name
    $arguments = @('-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
        '-File', $child, $Mode, $output) + $Extra
    $parameters = $common.Clone()
    $parameters.TimeoutSeconds = $Timeout
    $failure = $null
    try { $null = & $Runner @parameters -OutputDirectory $output -ArgumentList $arguments }
    catch { $failure = $_.Exception.Message }
    $path = Join-Path $output 'receipt.json'
    Require (Test-Path -LiteralPath $path -PathType Leaf) "$Name lost child receipt: $failure"
    $receipt = Read-Json $path
    Require ($receipt.child_exit_code -is [int]) "$Name lost actual numeric ExitCode"
    Require ($receipt.termination_confirmed -eq $true) "$Name did not confirm child exit"
    Require ($receipt.source_commit -eq ('a' * 40)) "$Name lost source commit provenance"
    Require ($receipt.executable_sha256 -eq $digest) "$Name lost executable hash"
    Require ($receipt.computer -eq [Environment]::MachineName) "$Name lost machine identity"
    $bytes = [IO.File]::ReadAllBytes($path)
    Require (-not ($bytes.Length -ge 3 -and $bytes[0] -eq 239 -and
        $bytes[1] -eq 187 -and $bytes[2] -eq 191)) "$Name receipt must be UTF-8 without BOM"
    Require ([IO.File]::ReadAllText((Join-Path $output 'stdout.log')).Contains('child stdout retained')) "$Name lost stdout"
    Require ([IO.File]::ReadAllText((Join-Path $output 'stderr.log')).Contains('child stderr retained')) "$Name lost stderr"
    [pscustomobject]@{ Output = $output; Receipt = $receipt; Failure = $failure }
}

Write-Output "Test evidence: $testRoot"
$zero = Invoke-Case 'fast zero' 'zero'
Require ($null -eq $zero.Failure -and $zero.Receipt.child_exit_code -eq 0) 'Exit 0 did not complete normally'
Require ($zero.Receipt.status -eq 'child_exited') 'Runner must not claim audio qualification'
Require (-not $zero.Receipt.timed_out -and -not $zero.Receipt.forced_termination) 'Normal exit marked as forced'
$manifest = Read-Json (Join-Path $zero.Output 'artifact-manifest.json')
Require (@($manifest.artifacts).Count -eq 3) 'Manifest must describe the three runner-owned artifacts'
foreach ($artifact in $manifest.artifacts) {
    $hash = (Get-FileHash -LiteralPath (Join-Path $zero.Output $artifact.file) -Algorithm SHA256).Hash
    Require ($hash -eq $artifact.sha256) "Manifest hash mismatch: $($artifact.file)"
}
Write-Output 'PASS: real fast exit 0, numeric receipt, hash provenance, UTF-8, runner artifact manifest'

foreach ($index in 1..3) {
    $nonzero = Invoke-Case "fast nonzero $index" 'nonzero'
    Require ($nonzero.Receipt.child_exit_code -eq 37 -and $nonzero.Receipt.status -eq 'child_failed') 'Nonzero child exit was lost'
    Require ($nonzero.Failure -like '*37*') 'Nonzero child exit must fail the runner'
}
Write-Output 'PASS: three real fast exit 37 children retain their exact exit code'

$nativeHash = (Get-FileHash -LiteralPath $nativeExecutable -Algorithm SHA256).Hash
$nativeCommon = @{
    Executable = $nativeExecutable; ExpectedSha256 = $nativeHash; SourceCommit = ('b' * 40)
    ExpectedComputer = [Environment]::MachineName; TimeoutSeconds = 15
}
foreach ($index in 1..12) {
    $output = Join-Path $testRoot "native fast nonzero $index"
    $failure = $null
    try { $null = & $Runner @nativeCommon -OutputDirectory $output -ArgumentList @() }
    catch { $failure = $_.Exception.Message }
    $receipt = Read-Json (Join-Path $output 'receipt.json')
    Require ($receipt.child_exit_code -eq 37 -and $receipt.status -eq 'child_failed' -and
        $receipt.termination_confirmed) "Native .exe run $index lost its held-handle exit code: $failure"
    Require ($failure -like '*37*') "Native .exe run $index unexpectedly succeeded"
    Require ([IO.File]::ReadAllText((Join-Path $output 'stdout.log')).Contains('native stdout retained')) "Native .exe run $index lost stdout"
    Require ([IO.File]::ReadAllText((Join-Path $output 'stderr.log')).Contains('native stderr retained')) "Native .exe run $index lost stderr"
}
Write-Output 'PASS: twelve genuine fast native .exe exit-37 children preserve code and both streams'

$eventPrefix = 'Local\MoirenRunnerDrain-' + [Guid]::NewGuid().ToString('N')
$releaseName = $eventPrefix + '-release'
$readyName = $eventPrefix + '-ready'
$closedName = $eventPrefix + '-closed'
$release = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, $releaseName)
$ready = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, $readyName)
$closed = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, $closedName)
try {
    $output = Join-Path $testRoot 'inherited output drain timeout'
    $failure = $null
    $clock = [Diagnostics.Stopwatch]::StartNew()
    try {
        $null = & $Runner @nativeCommon -OutputDirectory $output `
            -ArgumentList @('spawn-holder', $releaseName, $readyName, $closedName)
    } catch { $failure = $_.Exception.Message }
    $clock.Stop()
    $receipt = Read-Json (Join-Path $output 'receipt.json')
    Require ($receipt.child_exit_code -eq 0 -and $receipt.termination_confirmed) 'Drain timeout lost the actual exited child result'
    Require ($receipt.status -eq 'runner_failed' -and $failure -like '*did not drain*') 'Held inherited pipes did not trigger a real asynchronous drain timeout'
    Require (-not (Test-Path -LiteralPath (Join-Path $output 'artifact-manifest.json'))) 'Drain timeout published a manifest for output that was not proven stable'
    Require ($receipt.output_state -eq 'unavailable' -and $receipt.artifact_manifest_state -eq 'unavailable') 'Drain timeout receipt must explicitly mark output and manifest unavailable'
    Require ($clock.Elapsed.TotalSeconds -lt 20 -and -not $closed.WaitOne(0)) 'Runner must return within its bound while inherited pipes remain held'
    Require ([IO.File]::ReadAllText((Join-Path $output 'stdout.log')).Contains('native stdout retained')) 'Drain timeout lost captured partial stdout'
    Require ([IO.File]::ReadAllText((Join-Path $output 'stderr.log')).Contains('native stderr retained')) 'Drain timeout lost captured partial stderr'
} finally {
    # Never look up or kill descendants. Release this fixture's inherited
    # handles cooperatively; its own wait also has a 30-second safety bound.
    $null = $release.Set()
    try { Require ($closed.WaitOne(5000)) 'Inherited-pipe fixture did not close its output within its cleanup bound' }
    finally { $release.Dispose(); $ready.Dispose(); $closed.Dispose() }
}
Write-Output 'PASS: held inherited pipes fail collection without hashing incomplete logs; bounded fixture cleanup'
Require ($zero.Receipt.output_state -eq 'stable' -and $zero.Receipt.artifact_manifest_state -eq 'eligible') 'Completed drains must remain eligible for the runner manifest'

$metadata = Invoke-Case 'metadata failure' 'metadata-failure'
Require ($metadata.Receipt.child_exit_code -eq 37 -and $metadata.Receipt.status -eq 'child_failed') 'Metadata failure erased original child result'
Require ($metadata.Failure -like '*37*' -and $metadata.Failure -like '*manifest*') 'Child and manifest errors must both be reported'
Require (Test-Path -LiteralPath (Join-Path $metadata.Output 'artifact-manifest.json') -PathType Container) 'Runner changed obstructing child evidence'
Write-Output 'PASS: real artifact-manifest write failure cannot erase child receipt or failed evidence'

$values = @('', 'two words', 'C:\path with spaces\', 'embedded"quote', 'slashes\\"quote',
    '& | < > ^ % ; $()', 'plain')
$quoted = Invoke-Case 'argument roundtrip' 'arguments' $values
Require ($null -eq $quoted.Failure) "Quoted child failed: $($quoted.Failure)"
$actual = Read-Json (Join-Path $quoted.Output 'arguments.json')
Require ($actual.Count -eq $values.Count) 'Windows argument quoting lost empty or separated arguments'
for ($index = 0; $index -lt $values.Count; $index++) {
    Require ($actual[$index] -ceq $values[$index]) "Windows argument changed at index $index"
}
Write-Output 'PASS: exact empty/space/quote/backslash/metacharacter arguments reach the real child'

# An independent child must survive the runner's timeout; retain its handle so
# test cleanup, too, cannot target a reused PID or any unrelated process.
$sentinel = Start-Process -FilePath $executable -WindowStyle Hidden -PassThru `
    -ArgumentList '-NoLogo -NoProfile -NonInteractive -Command "Start-Sleep -Seconds 30"'
$null = $sentinel.Handle
try {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $timeout = Invoke-Case 'timeout' 'timeout' @() 2
    $clock.Stop()
    Require (-not $sentinel.HasExited) 'Runner terminated a child it does not own'
} finally {
    if (-not $sentinel.HasExited) { $sentinel.Kill() }
    Require ($sentinel.WaitForExit(5000)) 'Test sentinel did not exit within its cleanup bound'
    $sentinel.Dispose()
}
Require ($timeout.Receipt.timed_out -and $timeout.Receipt.forced_termination -and
    $timeout.Receipt.status -eq 'timed_out') 'Deadline did not terminate the owned child'
Require ($timeout.Failure -like '*timed_out*') 'Timeout must fail the runner'
Require ($clock.Elapsed.TotalSeconds -lt 12) 'Timeout wait/termination exceeded its bound'
$started = [int][IO.File]::ReadAllText((Join-Path $timeout.Output 'started.pid'))
Require ($timeout.Receipt.child_pid -eq $started) 'Timeout receipt targets the wrong child'
Require ($null -eq (Get-Process -Id $started -ErrorAction SilentlyContinue)) 'Timed-out child remains alive'
Write-Output 'PASS: bounded timeout kills only the retained owned child and confirms exit'

$before = (Get-FileHash -LiteralPath (Join-Path $zero.Output 'receipt.json')).Hash
$failure = $null
try { $null = & $Runner @common -OutputDirectory $zero.Output -ArgumentList @() }
catch { $failure = $_.Exception.Message }
Require ($failure -like '*already exists*') 'Existing evidence directory was not refused'
Require ((Get-FileHash -LiteralPath (Join-Path $zero.Output 'receipt.json')).Hash -eq $before) 'Existing receipt was overwritten'
Write-Output 'PASS: existing evidence is immutable; no automatic deletion or reconfiguration'

foreach ($guard in @('computer', 'digest', 'argument')) {
    $parameters = $common.Clone()
    $output = Join-Path $testRoot ("refused-$guard")
    $arguments = @()
    switch ($guard) {
        'computer' { $parameters.ExpectedComputer = 'not-this-computer' }
        'digest' { $parameters.ExpectedSha256 = '0' * 64 }
        'argument' { $arguments = @("invalid`0argument") }
    }
    $failure = $null
    try { $null = & $Runner @parameters -OutputDirectory $output -ArgumentList $arguments }
    catch { $failure = $_.Exception.Message }
    Require ($null -ne $failure -and -not (Test-Path -LiteralPath $output)) "$guard launched or changed output after refused preflight"
}
Write-Output 'PASS: wrong computer/hash and invalid arguments are refused before launch or directory creation'
Write-Output "PASS: Windows PowerShell $($PSVersionTable.PSVersion); all real-child regressions; evidence retained: $testRoot"
