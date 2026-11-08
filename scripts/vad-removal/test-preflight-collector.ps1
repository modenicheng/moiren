#requires -Version 5.1
param([string]$Collector)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if(-not $Collector){$Collector=Join-Path $PSScriptRoot 'readonly-preflight.ps1'}
. $Collector
$checks=New-Object Collections.Generic.List[object]
$root=Join-Path $PSScriptRoot ('logs\migration-preflight-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
function Reject([scriptblock]$Action){$refused=$false;try{&$Action|Out-Null}catch{$refused=$true};Check $refused 'Expected refusal'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
function Reset {
 $script:events=New-Object Collections.Generic.List[object];$script:mode='';$script:owners=New-Object Collections.Generic.List[object]
 $script:boot=[DateTimeOffset]::UtcNow.AddSeconds(-60);$script:osCalls=0
}
function FreshDirectory {Join-Path $root ([Guid]::NewGuid().ToString('N'))}
function Read-RemovalTargetIdentity {@{host='LAPTOP-NP49RG6H';sid='S-1-5-21-3080296863-1242338503-1346362208-1001';session=0}}
function Read-RemovalUptime {return [uint64]60000}
function Assert-FiniteToolPath([string]$Path,[bool]$File) {}
function Get-CimInstance([string]$ClassName,[string]$Filter,[string]$Namespace,[int]$OperationTimeoutSec,[string]$ErrorAction) {
 $script:events.Add('cim-'+$ClassName)
 switch($ClassName){
  'Win32_OperatingSystem' {$script:osCalls++;$b=$script:boot;if($script:mode-eq'boot-changed'-and$script:osCalls-gt1){$b=$b.AddDays(-1)};return [pscustomobject]@{LastBootUpTime=$b.LocalDateTime}}
  'Win32_SystemDriver' {return [pscustomobject]@{PathName='\SystemRoot\System32\drivers\MoirenVad.sys';State='Running'}}
  'Win32_DeviceGuard' {return [pscustomobject]@{VirtualizationBasedSecurityStatus=2;CodeIntegrityPolicyEnforcementStatus=2;UsermodeCodeIntegrityPolicyEnforcementStatus=1;SecurityServicesConfigured=@(1,2);SecurityServicesRunning=@(1,2)}}
  'Win32_Process' {return @()}
  default {throw 'Unexpected CIM query'}
 }
}
function Get-PnpDevice([string]$InstanceId,[switch]$PresentOnly,[string]$ErrorAction) {
 $script:events.Add('pnp-'+$InstanceId)
 if($PresentOnly){return @()}
 [pscustomobject]@{InstanceId=$InstanceId;Status='OK'}
}
function Get-PnpDeviceProperty([string]$InstanceId,[string]$KeyName,[string]$ErrorAction) {
 $value=switch($KeyName){
  'DEVPKEY_Device_HardwareIds' {'ROOT\MoirenVad'}
  'DEVPKEY_Device_ProblemCode' {0}
  'DEVPKEY_Device_DriverInfPath' {'oem207.inf'}
  'DEVPKEY_Device_DriverVersion' {if($script:mode-eq'old-driver'){'3.1.1.612'}else{'3.1.1.613'}}
  default {throw 'Unexpected device property'}
 }
 [pscustomobject]@{Data=$value}
}
function Get-ScheduledTask([string]$ErrorAction) {
 if($script:mode -eq 'task-queued'){return [pscustomobject]@{TaskPath='\';TaskName='MoirenTest';State='Queued'}}
 if($script:mode -eq 'task-unknown'){return [pscustomobject]@{TaskPath='\';TaskName='MoirenTest';State='Unknown'}}
 return @()
}
function Confirm-SecureBootUEFI {return $true}
function Get-AuthenticodeSignature([string]$LiteralPath) {
 [pscustomobject]@{Status=$(if($script:mode-eq'bad-signature'){'NotSigned'}else{'Valid'});SignerCertificate=[pscustomobject]@{Thumbprint='7F17CCDCA257D6ABC84255DD0D1854E93B9B5C1D'}}
}
function Get-Item([string]$LiteralPath,[switch]$Force,[string]$ErrorAction) {
 $length=if($LiteralPath-like'*moirenvad.cat'){2514}elseif($LiteralPath-like'*.inf'){1799}elseif($LiteralPath-like'*rollback-612*'){147352}else{152472}
 [pscustomobject]@{Length=$length;PSIsContainer=$false}
}
function Get-FileHash([string]$LiteralPath) {
 $hash=switch -Wildcard ($LiteralPath){
  '*direct-remote-driver-1755\package-manifest.json' {'94883DAC472B67A3344D22D7DA10B355ECD6565C87BCD022DFC10D56D1C44EAF';break}
  '*package-manifest.json' {'DEF621D149EF43753D27B8CE2A8DFA1237E6D8B106D369931A50F07E24AC1317';break}
  '*rollback-612\moirenvad.cat' {'AD5090790C6B359A6D169A7B3206FF3EA6F0CE026507087E80F29A1BEF3A83BA';break}
  '*rollback-612\MoirenVad.inf' {'BB526217B87462C8A986A62FC3348959C83E28CE97862C47D496D2F4CF84624D';break}
  '*rollback-612\MoirenVad.sys' {'B25CA2E8DB8DD3E98D9B90AA6CF19822C879E6CA13D52B35D160CEF2056D1737';break}
  '*moirenvad.cat' {'A6171C7C474667BF4A17B999D3187A325D509BA742EF89BA7322C8384E181E20';break}
  '*.inf' {'24574ED3BAE942EF7ED91601513F8F0B181272BF1F15D2067F0F6EE2014F827C';break}
  '*MoirenVad.sys' {'0A1DFF698923B2DF39D0625979D3CD1AEE486228396C1BF601EADEA565EB3B76';break}
  default {'1'*64}
 }
 if($script:mode-eq'bad-manifest'-and$LiteralPath-like'*package-manifest.json'){$hash='0'*64}
 [pscustomobject]@{Hash=$hash}
}
function Invoke-RemovalReadTool([string]$Name,[string]$Executable,[string]$Sha256,[string[]]$Argv,[string]$EvidenceDirectory,[Collections.Generic.List[object]]$Owners) {
 $script:events.Add([pscustomobject]@{Name=$Name;Executable=$Executable;Argv=$Argv})
 if($script:mode-eq'tool-unknown'){$Owners.Add([pscustomobject]@{ExactMockOwner=$true;ExitedConfirmed=$false});throw 'Read tool unknown'}
 $path=Join-Path $EvidenceDirectory ($Name+'.json')
 $data=if($Name-eq'catalog'){
  @{schema_version=1;errors=@();before=@{errors=@();defaults=@(foreach($f in @('render','capture')){foreach($r in @('console','multimedia','communications')){@{flow=$f;role=$r;id=$f}}});endpoints=@(@{id='out';flow='render';volume_scalar=0.5;muted=$false;errors=@()})}}|ConvertTo-Json -Depth 12
 }else{'mock finite tool stdout'}
 [IO.File]::WriteAllText($path,$data)
 [pscustomobject]@{Success=$true;StdoutPath=$path}
}
Case 'full-mocked-read-collects-real-shape-and-ten-read-tools-only' {
 Reset;$r=Read-RemovalPreflight -EvidenceDirectory (FreshDirectory) -ToolOwners $script:owners
 Check ($r.SignaturesVerified-and$r.CatalogMembersVerified-and$r.session-eq0-and$r.ExternalOwners.Count-eq0) 'Wrong snapshot provenance'
 Check ($r.Defaults.Count-eq6-and$r.Render.Count-eq1-and$r.Driver.Version-ceq'3.1.1.613') 'Missing snapshot surfaces'
 $tools=@($script:events|Where-Object{$_-isnot[string]})
 Check ($tools.Count-eq10) 'Wrong verification/catalog/bcd count'
 Check (@($tools|Where-Object{$_.Argv[0]-eq'verify'}).Count-eq8) 'Missing candidate/rollback signatures or membership'
 Check (-not$r.NativeTestStarted-and-not$r.RebootRequested-and-not$r.Qualified) 'False native activity/acceptance'
}
Case 'owner-observation-time-not-refreshed-after-later-collectors' {
 Reset;$r=Read-RemovalPreflight -EvidenceDirectory (FreshDirectory) -ToolOwners $script:owners
 $processIndex=$script:events.IndexOf('cim-Win32_Process')
 Check ($processIndex-eq($script:events.Count-1)) 'Process observation followed by later collectors'
 Check ($r.ExternalOwnersObservationStartedUtc-is[string]-and$r.ExternalOwnersObservedUtc-ceq$r.ObservedUtc) 'Missing actual observation interval'
 Check ([DateTimeOffset]$r.ExternalOwnersObservationStartedUtc-le[DateTimeOffset]$r.ExternalOwnersObservedUtc) 'Invalid process observation interval'
}
foreach($state in @('Queued','Unknown')){
 $script:testState=$state
 Case ('actual-collector-retains-task-'+$state) {
  Reset;$script:mode='task-'+$script:testState.ToLowerInvariant()
  $r=Read-RemovalPreflight -EvidenceDirectory (FreshDirectory) -ToolOwners $script:owners
  Check ($r.RunningTasks.Count-eq1-and$r.RunningTasks[0].State-ceq$script:testState) 'Collector erased pending/unknown task'
 }
}
Case 'preboot-refusal-before-signature-tool-or-device-observation' {
 Reset;$script:boot=[DateTimeOffset]'2026-10-30T00:00:00Z'
 Reject {Read-RemovalPreflight -EvidenceDirectory (FreshDirectory) -ToolOwners $script:owners}
 Check ($script:events.Count-eq1-and$script:events[0]-ceq'cim-Win32_OperatingSystem') 'Old boot reached native/tool collectors'
}
foreach($failure in @('bad-manifest','bad-signature','old-driver','boot-changed')){
 $script:testFailure=$failure
 Case ('refuses-'+$failure) {Reset;$script:mode=$script:testFailure;Reject {Read-RemovalPreflight -EvidenceDirectory (FreshDirectory) -ToolOwners $script:owners}}
}
Case 'unknown-tool-retains-owner-and-stops-without-next-dispatch' {
 Reset;$script:mode='tool-unknown';Reject {Read-RemovalPreflight -EvidenceDirectory (FreshDirectory) -ToolOwners $script:owners}
 Check ($script:owners.Count-eq1-and-not$script:owners[0].ExitedConfirmed) 'Exact unresolved owner lost'
 Check (@($script:events|Where-Object{$_-isnot[string]}).Count-eq1) 'Continued after unknown tool'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed)
if($failed){exit 1}
