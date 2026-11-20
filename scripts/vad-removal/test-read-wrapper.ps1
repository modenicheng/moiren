#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'finite-tool.ps1')
. (Join-Path $PSScriptRoot 'readonly-preflight.ps1')
$exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$hash=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
$root=Join-Path $PSScriptRoot ('logs\migration-read-wrapper-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$owners=New-Object Collections.Generic.List[object]
$r=Invoke-RemovalReadTool 'success' $exe $hash @('-NoLogo','-NoProfile','-NonInteractive','-Command','[Console]::Write("finite readonly integration");exit 0') $root $owners
if($owners.Count -ne 1 -or -not $owners[0].Closed -or -not $r.Success -or [IO.File]::ReadAllText($r.StdoutPath) -cne 'finite readonly integration'){
 throw 'Actual finite read-tool integration failed.'
}
$refused=$false
try {
 $null=Invoke-RemovalReadTool 'nonzero' $exe $hash @('-NoLogo','-NoProfile','-NonInteractive','-Command','[Console]::Error.Write("expected failure");exit 7') $root $owners
}catch{$refused=$true}
if(-not $refused -or $owners.Count -ne 2 -or $owners[1].Closed -or -not $owners[1].ExitedConfirmed -or $owners[1].ExitCode -ne 7){
 throw 'Failed read-tool exact owner or exit evidence lost.'
}
Close-FiniteToolOwner $owners[1]
if(-not $owners[1].Closed){throw 'Explicit terminal owner close failed.'}
Write-Output 'CHECKS=3 FAILED=0: real read wrapper success, nonzero owner retention, explicit terminal cleanup'
