# SPDX-License-Identifier: MIT
# Isolated runner regressions: fake process and power commands; no driver access.
[CmdletBinding()]
param([string]$Runner)
$ErrorActionPreference = 'Stop'
if (-not $env:COMPUTERNAME) { $env:COMPUTERNAME = [Environment]::MachineName }
if (-not $Runner) { $Runner = Join-Path $PSScriptRoot '..\vad-soak.ps1' }
$runner = (Resolve-Path -LiteralPath $Runner).Path
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($runner, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Runner does not parse' }
$functionAst = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Confirm-SoakReport'
}, $false))
if ($functionAst.Count -ne 1) { throw 'Expected one production report validator' }
# Exercise the actual function, not a copy of its conditions.
Invoke-Expression $functionAst[0].Extent.Text

function Require([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Reject([scriptblock]$Action, [string]$Expected) {
    try { & $Action } catch {
        Require ($_.Exception.Message -like "*$Expected*") "Unexpected failure: $_"
        return
    }
    throw "Expected rejection: $Expected"
}
function GoodReport {
    [pscustomobject]@{
        status = 'passed'
        service_joined = $true
        control_session_released = $true
        soak_idle = [pscustomobject]@{ completed = $true; requested_ms = 60000; elapsed_ms = 60000 }
        soak_pcm = [pscustomobject]@{ completed = $true; requested_ms = 60000; elapsed_ms = 60000 }
    }
}

Confirm-SoakReport (GoodReport) 60 60 120
$report = GoodReport
$report.soak_pcm = $null
Reject { Confirm-SoakReport $report 60 60 120 } 'soak_pcm'
$report = GoodReport
$report.soak_idle.completed = $false
Reject { Confirm-SoakReport $report 60 60 120 } 'soak_idle'
$report = GoodReport
$report.soak_idle.elapsed_ms = 59999
Reject { Confirm-SoakReport $report 60 60 120 } 'soak_idle'
$report = GoodReport
$report.soak_pcm.requested_ms = 1000
Reject { Confirm-SoakReport $report 60 60 120 } 'soak_pcm'
Reject { Confirm-SoakReport (GoodReport) 60 60 10 } 'before the requested'
$report = GoodReport
$report.service_joined = $false
Reject { Confirm-SoakReport $report 60 60 120 } 'orderly service teardown'
Write-Output 'PASS: report phase identity/completion/duration, aggregate elapsed, teardown'

$testRoot = Join-Path $PSScriptRoot ("..\..\driver\vad\build\tests\soak-runner-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
$executable = Join-Path $testRoot 'fake.exe'
[IO.File]::WriteAllText($executable, 'Not an executable; Start-Process is replaced in this test.')
$digest = (Get-FileHash -LiteralPath $executable).Hash
$global:VadSoakTest = $null

function global:Get-Process { @() }
function global:powercfg.exe {
    $global:VadSoakTest.PowerCalls++
    if ($global:VadSoakTest.Case -eq 'preflight-duration' -and $global:VadSoakTest.PowerCalls -eq 1) {
        Start-Sleep -Milliseconds 1100
    }
    $global:LASTEXITCODE = 0
    if ($global:VadSoakTest.Case -eq 'partial-power' -and $global:VadSoakTest.PowerCalls -eq 2) {
        # Exercise real native stderr under Windows PowerShell's error policy.
        & (Join-Path ([Environment]::GetFolderPath('System')) 'cmd.exe') /d /c 'echo injected native stderr 1>&2 & exit /b 5'
        $global:LASTEXITCODE = $LASTEXITCODE
    }
}
function global:Start-Process {
    $global:VadSoakTest.Launches++
    $root = $global:VadSoakTest.Output
    New-Item -ItemType Directory -Path (Join-Path $root 'probe') | Out-Null
    @{ status = 'passed'; service_joined = $true; control_session_released = $true } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'probe\report.json')
    if ($global:VadSoakTest.Case -eq 'preflight-duration') {
        @{ status = 'passed'; service_joined = $true; control_session_released = $true
            soak_idle = @{ completed = $true; requested_ms = 1000; elapsed_ms = 1000 }
        } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'probe\report.json')
    }
    if ($global:VadSoakTest.Case -eq 'cancel') {
        [IO.File]::WriteAllText((Join-Path $root 'cancel'), '')
    }
    $fake = [pscustomobject]@{
        Id = 4242; Handle = [IntPtr]1; StartTime = [DateTime]::Now; ExitCode = 0
        HasExited = $global:VadSoakTest.Case -ne 'cancel'
    }
    $fake | Add-Member ScriptMethod Refresh {
        if ($global:VadSoakTest.Case -eq 'cleanup-error') { throw 'injected refresh failure' }
    }
    $fake | Add-Member ScriptMethod WaitForExit { param($milliseconds) return $true }
    $fake | Add-Member ScriptMethod Kill {
        $global:VadSoakTest.Kills++
        $this.HasExited = $true
    }
    $fake | Add-Member ScriptMethod Dispose { $global:VadSoakTest.Disposed = $true }
    return $fake
}

try {
    foreach ($case in @('missing-phase', 'partial-power', 'cleanup-error', 'cancel', 'preflight-duration')) {
        $output = [IO.Path]::GetFullPath((Join-Path $testRoot $case))
        $global:VadSoakTest = @{
            Case = $case; Output = $output; PowerCalls = 0; Launches = 0; Kills = 0; Disposed = $false
        }
        $failed = $false
        try {
            $idle = 60
            $pcm = 60
            if ($case -eq 'preflight-duration') { $idle = 1; $pcm = 0 }
            & $runner -Executable $executable -ExpectedSha256 $digest -SourceCommit ('a' * 40) `
                -OutputDirectory $output -IdleSeconds $idle -PcmSeconds $pcm -ExpectedComputer $env:COMPUTERNAME
        } catch {
            $failed = $true
            if (-not (Test-Path -LiteralPath (Join-Path $output 'result.json'))) {
                throw "Runner failed before its final receipt: $_"
            }
        }
        Require $failed "$case incorrectly succeeded"
        $result = Get-Content -LiteralPath (Join-Path $output 'result.json') -Raw | ConvertFrom-Json
        Require ($result.status -ne 'passed') "$case wrote a successful receipt"
        switch ($case) {
            'partial-power' {
                Require ($global:VadSoakTest.Launches -eq 0) 'Probe launched after failed power preflight'
                Require ($result.power_outcomes.Count -eq 2) 'Partial power outcomes were lost'
                Require ($result.power_outcomes[0].exit_code -eq 0 -and $result.power_outcomes[1].exit_code -eq 5) 'Power exit codes lost'
                Require ($result.power_outcomes[1].output -like '*injected native stderr*') 'Native stderr diagnostic lost'
            }
            'cleanup-error' {
                Require ($result.error -like '*soak_idle*') 'Original evidence failure was replaced'
                Require ($result.cleanup_errors.Count -eq 1) 'Injected cleanup failure not retained'
                Require (-not $result.termination_confirmed) 'Failed observation reported confirmed termination'
                Require $global:VadSoakTest.Disposed 'Process disposal skipped after refresh failure'
            }
            'cancel' {
                Require ($result.status -eq 'cancelled') 'Cancellation status lost'
                Require ($result.forced_termination -and $result.termination_confirmed) 'Owned cancellation termination not confirmed'
                Require ($global:VadSoakTest.Kills -eq 1) 'Cancellation did not kill exactly the owned fake'
            }
            'missing-phase' {
                Require ($result.error -like '*soak_idle*') 'Short lifecycle result not rejected by phase evidence'
            }
            'preflight-duration' {
                Require ($result.elapsed_seconds -gt 1 -and $result.probe_elapsed_seconds -lt 1) 'Preflight/probe clocks were not separated'
                Require ($result.error -like '*before the requested*') 'Preflight time was credited toward child soak duration'
            }
        }
        if ($global:VadSoakTest.Launches) {
            # An exclusive reopen proves writer cleanup still ran on error.
            $stream = [IO.File]::Open((Join-Path $output 'resources.jsonl'), 'Open', 'ReadWrite', 'None')
            $stream.Dispose()
        }
        Write-Output "PASS: $case final receipt and cleanup"
    }
} finally {
    Remove-Item Function:\Get-Process, Function:\powercfg.exe, Function:\Start-Process
    Remove-Variable VadSoakTest -Scope Global
}
Write-Output "Isolated evidence: $([IO.Path]::GetFullPath($testRoot))"
