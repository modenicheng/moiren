# SPDX-License-Identifier: MIT
# Run inside the Windows test VM, before driver deployment or long tests.
[CmdletBinding()]
param([switch]$VerifyOnly)
$ErrorActionPreference = 'Stop'

$computer = Get-CimInstance Win32_ComputerSystem
if ($computer.Manufacturer -notmatch 'VMware' -or $computer.Model -notmatch 'VMware') {
    throw 'Run this script inside the VMware Windows target, not on the development host.'
}

function Invoke-PowerConfiguration {
    param([string[]]$Arguments)
    $output = & "$env:SystemRoot\System32\powercfg.exe" @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "powercfg failed ($LASTEXITCODE): $output"
    }
    $output
}

if (-not $VerifyOnly) {
    $principal = [Security.Principal.WindowsPrincipal]::new(
        [Security.Principal.WindowsIdentity]::GetCurrent())
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Run in an elevated PowerShell inside the target VM to change its power settings.'
    }
    Invoke-PowerConfiguration -Arguments @('/change', 'standby-timeout-ac', '0')
    Invoke-PowerConfiguration -Arguments @('/change', 'standby-timeout-dc', '0')
    Invoke-PowerConfiguration -Arguments @('/change', 'hibernate-timeout-ac', '0')
    Invoke-PowerConfiguration -Arguments @('/change', 'hibernate-timeout-dc', '0')
    Invoke-PowerConfiguration -Arguments @('/hibernate', 'off')
}

$sleep = Invoke-PowerConfiguration -Arguments @('/query', 'SCHEME_CURRENT', 'SUB_SLEEP', 'STANDBYIDLE')
# Numeric values are invariant across localized powercfg output; the final two
# 32-bit hexadecimal values are the AC and DC timeout indices.
$indices = [regex]::Matches(($sleep -join "`n"), '0x[0-9a-fA-F]{8}')
if ($indices.Count -lt 2 -or
    $indices[$indices.Count - 2].Value -ne '0x00000000' -or
    $indices[$indices.Count - 1].Value -ne '0x00000000') {
    throw "Target automatic sleep remains enabled: $($sleep -join [Environment]::NewLine)"
}
$powerKey = Get-ItemProperty -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\Power'
if ($powerKey.HibernateEnabled -ne 0) {
    throw 'Target hibernation remains enabled.'
}
[pscustomobject]@{
    ComputerName = $env:COMPUTERNAME
    Model = $computer.Model
    AutomaticSleepAC = 'Disabled'
    AutomaticSleepDC = 'Disabled'
    Hibernation = 'Disabled'
    VerifiedAt = (Get-Date).ToString('o')
}
