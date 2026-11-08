#requires -Version 5.1
param([string]$Collector)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if(-not $Collector){$Collector=Join-Path $PSScriptRoot 'readonly-preflight.ps1'}
if(Test-Path -LiteralPath $Collector){. $Collector}
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
function Reject([scriptblock]$Action){$refused=$false;try{&$Action|Out-Null}catch{$refused=$true};Check $refused 'Expected refusal'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
function ProcessRow([int]$Id,[string]$Name,[string]$Path='C:\External\app.exe'){
 [pscustomobject]@{ProcessId=$Id;Name=$Name;SessionId=1;CreationDate=[DateTime]'2026-11-08T00:00:00Z';ExecutablePath=$Path}
}
function Catalog {
 @{schema_version=1;errors=@();before=@{errors=@();defaults=@(
   foreach($flow in @('render','capture')){foreach($role in @('console','multimedia','communications')){@{flow=$flow;role=$role;id='endpoint-'+$flow}}}
  );endpoints=@(@{id='render-one';flow='render';volume_scalar=0.5;muted=$false;errors=@()})}}
}
Case 'real-uptime-on-PS5-and-PS7-is-typed-not-null' {
 $value=Read-RemovalUptime;Check ($value-is[uint64]-and$value-gt0) 'No real typed uptime'
}
Case 'boot-must-follow-install-not-just-recent-observation' {
 $r=New-RemovalBootObservation ([DateTimeOffset]'2026-10-30T00:00:00Z') ([DateTimeOffset]'2026-11-08T00:01:00Z') ([uint64]60000)
 Check (-not$r.BootAfterDeployment) 'Old boot accepted'
}
Case 'boot-uptime-consistency-and-explicit-zone' {
 $r=New-RemovalBootObservation ([DateTimeOffset]'2026-11-08T00:00:00Z') ([DateTimeOffset]'2026-11-08T00:01:00Z') ([uint64]60000)
 Check ($r.BootAfterDeployment-and$r.ClockConsistent-and$r.UptimeMilliseconds-is[uint64]) 'Valid boot rejected'
 Check ($r.BootUtc-match'\+00:00$') 'Explicit timezone missing'
 $r=New-RemovalBootObservation ([DateTimeOffset]'2026-11-08T00:00:00Z') ([DateTimeOffset]'2026-11-08T00:01:00Z') ([uint64]1)
 Check (-not$r.ClockConsistent) 'Inconsistent uptime accepted'
}
Case 'empty-external-owners-is-explicit-complete-not-launch-instruction' {
 $r=Convert-RemovalProcesses @() 900
 Check ($r.ExternalOwners-is[Array]-and$r.ExternalOwners.Count-eq0-and$r.NativeOwners.Count-eq0) 'Empty observation lost'
}
Case 'external-OBS-and-QQMusic-are-preserved-not-owned-tests' {
 $r=Convert-RemovalProcesses @((ProcessRow 44 'obs64.exe'),(ProcessRow 45 'QQMusic.exe'),(ProcessRow 900 'powershell.exe')) 900
 Check ($r.ExternalOwners.Count-eq2-and$r.NativeOwners.Count-eq0) 'External instance classified as test owner'
 Check ($r.ExternalOwners[0].CreationUnixMs-is[long]-and$r.ExternalOwners[0].ExecutablePath-ceq'C:\External\app.exe') 'External exact identity lost'
}
Case 'dedicated-fixed-OBS-and-probe-are-busy' {
 $r=Convert-RemovalProcesses @((ProcessRow 44 'obs64.exe' 'D:\MoirenTest\20261101-clean-comb-probe-1320\obs-portable\bin\64bit\obs64.exe'),(ProcessRow 45 'e2e_latency_probe.exe')) 900
 Check ($r.NativeOwners.Count-eq2-and$r.ExternalOwners.Count-eq0) 'Dedicated fixture/probe falsely idle'
}
Case 'missing-path-creation-or-duplicate-protected-process-is-unknown' {
 $p=ProcessRow 44 'obs64.exe';$p.CreationDate=$null;Reject {Convert-RemovalProcesses @($p) 900}
 $p=ProcessRow 44 'QQMusic.exe';$p.ExecutablePath=$null;Reject {Convert-RemovalProcesses @($p) 900}
 $p=ProcessRow 44 'obs64.exe';Reject {Convert-RemovalProcesses @($p,$p) 900}
}
Case 'invalid-creation-refuses-even-with-caller-Continue-preference' {
 $p=ProcessRow 44 'QQMusic.exe';$p.CreationDate='not-a-timestamp'
 $old=$ErrorActionPreference;$ErrorActionPreference='Continue'
 try {Reject {Convert-RemovalProcesses @($p) 900}} finally {$ErrorActionPreference=$old}
}
Case 'queued-and-unknown-tasks-are-not-idle' {
 foreach($state in @('Queued','Unknown','Running')){
  $r=@(Convert-RemovalTasks @([pscustomobject]@{TaskPath='\';TaskName='MoirenTest';State=$state}))
  Check ($r.Count-eq1-and$r[0].State-ceq$state) 'Pending/unknown task falsely idle'
 }
 foreach($state in @('Ready','Disabled')){
  $r=@(Convert-RemovalTasks @([pscustomobject]@{TaskPath='\';TaskName='MoirenTest';State=$state}))
  Check ($r.Count-eq0) 'Known idle task falsely busy'
 }
}
Case 'catalog-canonicalizes-and-preserves-numeric-types' {
 $r=Convert-RemovalCatalog (Catalog)
 Check ($r.Defaults.Count-eq6-and$r.Render.Count-eq1-and$r.Render[0].volume_scalar-is[double]) 'Catalog incomplete/coerced'
 Check ($r.Defaults[0].flow-ceq'capture') 'Noncanonical order'
}
Case 'catalog-errors-missing-array-or-nonnumeric-controls-refused' {
 $c=Catalog;$c.before.errors=@('read failed');Reject {Convert-RemovalCatalog $c}
 $c=Catalog;$c.before.endpoints[0].volume_scalar='0.5';Reject {Convert-RemovalCatalog $c}
 $c=Catalog;$c.before.endpoints[0].volume_scalar=[double]::NaN;Reject {Convert-RemovalCatalog $c}
 $c=Catalog;$c.before.defaults=$c.before.defaults[0..4];Reject {Convert-RemovalCatalog $c}
 $c=Catalog;$c.before.endpoints[0].Remove('errors');Reject {Convert-RemovalCatalog $c}
}
Case 'no-hardcoded-desktop-session-for-SSH-collector' {
 $s=New-RemovalIdentity 'LAPTOP-NP49RG6H' 'S-1-5-21-3080296863-1242338503-1346362208-1001' 0
 Check ($s.session-eq0) 'Session zero collector refused'
 Reject {New-RemovalIdentity 'other' $s.sid 0}
 Reject {New-RemovalIdentity $s.host 'S-1-5-18' 0}
}
Case 'definitions-have-no-install-reboot-native-audio-or-network-command' {
 $tokens=$null;$errors=$null;$ast=[Management.Automation.Language.Parser]::ParseFile($Collector,[ref]$tokens,[ref]$errors)
 Check ($errors.Count-eq0) 'Collector parse failure'
 $commands=@($ast.FindAll({param($n)$n-is[Management.Automation.Language.CommandAst]},$true)|ForEach-Object{$_.GetCommandName()})
 foreach($bad in @('Restart-Computer','Stop-Computer','Restart-Service','Set-PnpDeviceProperty','pnputil','ssh','scp','Set-NetFirewallProfile','Stop-Process')){
  Check (-not($commands-contains$bad)) ('Disruptive command present: '+$bad)
 }
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed)
if($failed){exit 1}
