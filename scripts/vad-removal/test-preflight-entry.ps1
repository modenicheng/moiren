#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){
    throw 'Host-only entry tests must not run on the actual test target.'
}
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
$entry=Join-Path $PSScriptRoot 'invoke-preflight.ps1'
Case 'actual-readonly-entry-exists' {Check (Test-Path $entry) 'Postboot preflight entry is missing.'}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_EXECUTED=false';exit 1}
. (Join-Path $PSScriptRoot 'finite-tool.ps1')
$root=Join-Path $PSScriptRoot ('logs\preflight-entry-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$ps=[Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
$hash=(Get-FileHash $ps).Hash
# Run the real entry alone, without any importable sibling helpers. The exact
# refusal reason therefore proves the guard ran before its first import.
$isolatedEntry=Join-Path $root 'invoke-preflight.ps1'
[IO.File]::WriteAllBytes($isolatedEntry,[IO.File]::ReadAllBytes($entry))
Case 'no-user-confirmation-refuses-before-target-collection' {
 $evidence=Join-Path $root 'unconfirmed'
 $o=Start-FiniteTool -Executable $ps -ExpectedSha256 $hash -OutputDirectory (Join-Path $root 'unconfirmed-child') `
  -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-File',$isolatedEntry,'-EvidenceDirectory',$evidence)
 $r=Observe-FiniteTool $o
 Check ($r.ExitedConfirmed-and$r.ExitCode-eq1-and-not(Test-Path $evidence)) 'Unconfirmed entry touched evidence.'
 Close-FiniteToolOwner $o
 Check ([IO.File]::ReadAllText($o.StderrPath)-like'*User reboot confirmation*') 'Wrong refusal reason.'
}
Case 'actual-development-host-refuses-before-import-or-device-collection' {
 if([Environment]::MachineName-ceq'LAPTOP-NP49RG6H'){throw 'This host-only test must not run on target.'}
 $evidence=Join-Path $root 'wrong-host'
 $o=Start-FiniteTool -Executable $ps -ExpectedSha256 $hash -OutputDirectory (Join-Path $root 'wrong-host-child') `
  -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-File',$isolatedEntry,'-EvidenceDirectory',$evidence,'-UserRebootConfirmed')
 $r=Observe-FiniteTool $o
 Check ($r.ExitedConfirmed-and$r.ExitCode-eq1-and-not(Test-Path $evidence)) 'Wrong host reached collection.'
 Close-FiniteToolOwner $o
 Check ([IO.File]::ReadAllText($o.StderrPath)-like'*Wrong target host/SID*') 'Wrong host not refused early.'
}
Case 'entry-has-no-audio-driver-reboot-obs-or-network-dispatch' {
 $tokens=$null;$errors=$null;$ast=[Management.Automation.Language.Parser]::ParseFile($entry,[ref]$tokens,[ref]$errors)
 Check ($errors.Count-eq0) 'Entry parse failed.'
 $commands=@($ast.FindAll({param($n)$n-is[Management.Automation.Language.CommandAst]},$true)|ForEach-Object{$_.GetCommandName()})
 foreach($bad in @('Restart-Computer','Restart-Service','Stop-Process','ssh','scp','pnputil','Register-RemovalTaskDefinition','Start-RemovalNativeTrace','Start-RemovalTaskOnce','Copy-Item')){
  Check (-not($commands-contains$bad)) ('Unexpected destructive/remote operation: '+$bad)
 }
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ENTRY=actual-host-refusal NATIVE_EXECUTED=false')
if($failed){exit 1}
