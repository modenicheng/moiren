#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){throw 'Host-only entry tests must not run on target.'}
. (Join-Path $PSScriptRoot 'finite-tool.ps1')
$root=Join-Path $PSScriptRoot ('logs\diagnostic-entry-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$entry=Join-Path $PSScriptRoot 'invoke-diagnostic.ps1'
$isolated=Join-Path $root 'invoke-diagnostic.ps1'
[IO.File]::WriteAllBytes($isolated,[IO.File]::ReadAllBytes($entry))
$exe=[Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
$checks=New-Object Collections.Generic.List[object]
foreach($mode in @('confirmation','host')){
 $path=Join-Path $root $mode
 $arguments=@('-NoLogo','-NoProfile','-NonInteractive','-File',$isolated,'-RunDirectory',$path,
  '-ManifestPath',(Join-Path $root 'missing-manifest.json'),'-ManifestSha256',('A'*64))
 if($mode-ceq'host'){$arguments+=,'-UserRebootConfirmed'}
 $o=Start-FiniteTool -Executable $exe -ExpectedSha256 (Get-FileHash $exe).Hash `
  -OutputDirectory (Join-Path $root ($mode+'-child')) -ArgumentList $arguments
 $r=Observe-FiniteTool $o
 if(-not$r.ExitedConfirmed-or$r.ExitCode-ne1-or$r.NeedsObservation){throw 'Guarded diagnostic entry did not exit with refusal.'}
 Close-FiniteToolOwner $o
 $text=[IO.File]::ReadAllText($o.StderrPath)
 $expected=if($mode-ceq'confirmation'){'*User reboot confirmation required*'}else{'*Wrong diagnostic target host/SID*'}
 if($text-notlike$expected-or(Test-Path $path)){throw 'Entry imported helpers/read manifest/touched target before guard.'}
 $checks.Add(@{name=$mode+'-before-imports-and-files';passed=$true})
}
$checks|ConvertTo-Json -Depth 4
Write-Output ('CHECKS='+$checks.Count+' FAILED=0 ACTUAL_ENTRY_REFUSALS=true NATIVE_AUDIO=false')
