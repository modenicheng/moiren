#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$no=$false;try{&$Action|Out-Null}catch{$no=$true};Check $no 'Expected refusal.'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
$library=Join-Path $PSScriptRoot 'preflight-session.ps1'
Case 'retained-preflight-composition-exists-without-launch' {
 Check (Test-Path -LiteralPath $library) 'Managed preflight admission is missing.'
 . $library
 foreach($name in @('New-RemovalPreflightSession','Start-RemovalPreflightSession','Observe-RemovalPreflightSession','Assert-RemovalPreflightSession')){
  Check ($null-ne(Get-Command $name -ErrorAction SilentlyContinue)) ('Missing '+$name)
 }
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_EXECUTED=false';exit 1}
. $library
$root=Join-Path $PSScriptRoot ('logs\preflight-session-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$script:sessions=New-Object Collections.Generic.List[object]
function Assert-RemovalTaskHost {}
function Fixture {
 $r=Join-Path $root ([Guid]::NewGuid().ToString('N'));$null=[IO.Directory]::CreateDirectory($r)
 $now=[DateTimeOffset]::UtcNow;$boot=$now.AddSeconds(-60)
 $p=@{schema_version=1;host='LAPTOP-NP49RG6H';sid='S-1-5-21-3080296863-1242338503-1346362208-1001';session=[Diagnostics.Process]::GetCurrentProcess().SessionId
  CollectorStartedUtc=$now.AddSeconds(-1).ToString('o');BootUtc=$boot.ToString('o');ObservedUtc=$now.ToString('o')
  UptimeMilliseconds=[uint64]60000;BootAfterDeployment=$true;SignaturesVerified=$true;CatalogMembersVerified=$true
  NativeOwners=@();RunningTasks=@();PresentRawChildren=@();ExternalOwners=@();ExternalOwnersProvenance='ObservedComplete'
  ExternalOwnersObservationStartedUtc=$now.AddMilliseconds(-10).ToString('o');ExternalOwnersObservedUtc=$now.ToString('o')
  Defaults=@(foreach($flow in @('capture','render')){foreach($role in @('communications','console','multimedia')){
   @{flow=$flow;role=$role;id='endpoint-'+$flow}}});Render=@(@{id='render-one';volume_scalar=[double]0.5;muted=$false})
  Security=@{SecureBoot=$false;Vbs=2;CodeIntegrity=2;UserCodeIntegrity=0;Configured=@(2);Running=@(2);Bcd=('a'*64)}
  Driver=@{Version='3.1.1.613';Inf='oem207.inf';State='Running';InstanceId='ROOT\MEDIA\0001';PnpStatus='OK';ProblemCode=0
   SysPath='C:\Windows\System32\DriverStore\FileRepository\reviewed\MoirenVad.sys'
   SysSha256='0A1DFF698923B2DF39D0625979D3CD1AEE486228396C1BF601EADEA565EB3B76'
   InfSha256='24574ED3BAE942EF7ED91601513F8F0B181272BF1F15D2067F0F6EE2014F827C'}
  NativeTestStarted=$false;RebootRequested=$false;Qualified=$false;IdentityScope='Fresh postboot service-file/PnP provenance, not in-memory image attestation.'}
 [pscustomobject]@{Root=$r;Snapshot=$p;Mode='normal';Session=$null}
}
function Prepare($f,[string]$Prefix='',[string]$ObservationMode='Admission'){
 $collector=Join-Path $f.Root 'collector.ps1'
 $data=[Management.Automation.PSSerializer]::Serialize($f.Snapshot,16)
 $escaped=$data.Replace("'","''")
 $restore="`$s=[Management.Automation.PSSerializer]::Deserialize('$escaped');"+
  'foreach($key in @("NativeOwners","RunningTasks","PresentRawChildren","ExternalOwners","Defaults","Render")){if($null-ne$s[$key]){$s[$key]=[object[]]$s[$key]}};'+
  'foreach($key in @("Configured","Running")){$s.Security[$key]=[object[]]$s.Security[$key]}; return $s'
 $source="function Read-RemovalPreflight { param(`$EvidenceDirectory,`$ToolOwners) $Prefix $restore }"
 [IO.File]::WriteAllText($collector,$source,[Text.UTF8Encoding]::new($false))
 $options=@{};if($ObservationMode-cne'Admission'){$options.ObservationMode=$ObservationMode}
 $f.Session=New-RemovalPreflightSession -EvidenceDirectory (Join-Path $f.Root 'evidence') `
  -CollectorPath $collector -CollectorSha256 (Get-FileHash $collector).Hash -UserRebootConfirmed @options
 $script:sessions.Add($f.Session)
}
function Run($f){
 Start-RemovalPreflightSession $f.Session
 $r=$null
 for($i=0;$i-lt100;$i++){$r=Observe-RemovalPreflightSession $f.Session -ObservationMilliseconds 100;if(-not$r.NeedsObservation){break}}
 return $r
}
Case 'actual-collector-owner-snapshot-and-persisted-pin-authorize-readiness-not-native-success' {
 $f=Fixture;Prepare $f
 Check ($null-eq$f.Session.CollectorOwner-and-not(Test-Path (Join-Path $f.Root 'evidence'))) 'Construction launched collector.'
 $r=Run $f
 Check ($r.Ready-and-not$r.NeedsObservation-and-not$r.Qualified-and-not$r.NativeApproved) 'Valid retained collection refused or promoted.'
 Check ($f.Session.CollectorOwner.Completed-and$f.Session.CollectorOwner.Closed-and$null-ne$f.Session.ReceiptPin) 'Original collection not settled/persisted.'
 Assert-RemovalPreflightSession $f.Session
 $saved=[IO.File]::ReadAllText($f.Session.ReceiptPin.Path)|ConvertFrom-Json
 Check ($saved.ControllerPid-eq$PID-and$saved.Snapshot.session-eq[Diagnostics.Process]::GetCurrentProcess().SessionId-and-not$saved.NativeApproved) 'Receipt changed collector session/owner.'
 Reject {Start-RemovalPreflightSession $f.Session}
}
Case 'user-confirmation-is-required-but-never-substitutes-for-post-install-boot' {
 $f=Fixture;Reject {New-RemovalPreflightSession -EvidenceDirectory (Join-Path $f.Root 'none')}
 $f.Snapshot.BootUtc='2026-10-30T01:50:55.3826800Z';Prepare $f;$r=Run $f
 Check (-not$r.Ready-and-not(Test-Path (Join-Path $f.Root 'evidence\preflight-receipt.json'))) 'Old boot gained authority from confirmation.'
}
Case 'fresh-complete-empty-external-list-is-legal-but-busy-or-unknown-fields-refuse' {
 foreach($mode in @('native','task','raw','external-null','uptime-null','future-time','stale','session','zone')){
  $f=Fixture
  switch($mode){'native'{$f.Snapshot.NativeOwners=@('busy')};'task'{$f.Snapshot.RunningTasks=@('queued')};'raw'{$f.Snapshot.PresentRawChildren=@('held')}
   'external-null'{$f.Snapshot.ExternalOwners=$null};'uptime-null'{$f.Snapshot.UptimeMilliseconds=$null}
   'future-time'{$f.Snapshot.ObservedUtc=[DateTimeOffset]::UtcNow.AddMinutes(10).ToString('o')}
   'stale'{$f.Snapshot.ObservedUtc=[DateTimeOffset]::UtcNow.AddMinutes(-10).ToString('o')}
   'session'{$f.Snapshot.session=65535};'zone'{$f.Snapshot.BootUtc='2026-11-08T01:00:00'}}
  Prepare $f;$r=Run $f;Check (-not$r.Ready) ('Invalid snapshot authorized: '+$mode)
 }
}
Case 'driver-default-volume-security-and-external-identity-are-strictly-validated' {
 foreach($mode in @('driver','problem-bool','default-duplicate','volume-string','security-missing','external-duplicate')){
  $f=Fixture
  switch($mode){'driver'{$f.Snapshot.Driver.SysSha256='b'*64};'problem-bool'{$f.Snapshot.Driver.ProblemCode=$false}
   'default-duplicate'{$f.Snapshot.Defaults[0]=$f.Snapshot.Defaults[1]};'volume-string'{$f.Snapshot.Render[0].volume_scalar='0.5'}
   'security-missing'{$f.Snapshot.Security.Remove('Vbs')};'external-duplicate'{$owner=@{Pid=42;Name='QQMusic.exe';Session=2;CreationUnixMs=[long]1794000000000;ExecutablePath='D:\QQMusic\QQMusic.exe'};$f.Snapshot.ExternalOwners=@($owner,$owner)}}
  Prepare $f;$r=Run $f;Check (-not$r.Ready) ('Malformed safety evidence authorized: '+$mode)
 }
}
Case 'snapshot-or-receipt-replacement-after-success-invalidates-admission' {
 foreach($mode in @('snapshot','receipt','receipt-repin','owner','source-repin')){
  $f=Fixture;Prepare $f;$r=Run $f;Check $r.Ready 'Baseline not ready.'
  switch($mode){'snapshot'{$f.Session.CollectorOwner.Shared.Snapshot.Driver.SysSha256='b'*64}
   'receipt'{[IO.File]::AppendAllText($f.Session.ReceiptPin.Path,' ')}
   'receipt-repin'{$other=Join-Path $f.Root 'replacement-receipt.json';[IO.File]::WriteAllText($other,'{}');$f.Session.ReceiptPin=[pscustomobject]@{Path=$other;Bytes=2;Sha256=(Get-FileHash $other).Hash}}
   'owner'{$f.Session.CollectorOwner=[pscustomobject]@{Completed=$true}}
   'source-repin'{$other=Join-Path $f.Root 'replacement.ps1';[IO.File]::WriteAllText($other,'# valid different source never executed');$f.Session.CollectorPath=$other;$f.Session.CollectorSha256=(Get-FileHash $other).Hash}}
  Reject {Assert-RemovalPreflightSession $f.Session}
 }
}
Case 'timeout-keeps-one-original-runspace-and-never-restarts-or-accepts-late-result' {
 $f=Fixture;Prepare $f 'Start-Sleep -Milliseconds 250;'
 Start-RemovalPreflightSession $f.Session;$owner=$f.Session.CollectorOwner;$owner.DeadlineMilliseconds=[long]0
 $r=Observe-RemovalPreflightSession $f.Session -ObservationMilliseconds 1
 Check (-not$r.Ready-and[Object]::ReferenceEquals($owner,$f.Session.CollectorOwner)) 'Timeout accepted/replaced collector.'
 Reject {Start-RemovalPreflightSession $f.Session}
 for($i=0;$i-lt100-and$r.NeedsObservation;$i++){$r=Observe-RemovalPreflightSession $f.Session -ObservationMilliseconds 100}
 Check (-not$r.Ready-and$owner.TimedOut-and$owner.Closed) 'Late collection became success or owner leaked.'
}
Case 'unexpected-diagnostics-refuse-without-saving-readiness' {
 $f=Fixture;Prepare $f 'Write-Error "synthetic failure";'
 $r=Run $f;Check (-not$r.Ready) 'Failed collector gained readiness.'
}
Case 'audit-observation-preserves-typed-busy-rows-without-admission' {
 $f=Fixture
 $f.Snapshot.NativeOwners=@(@{Pid=42;Name='vad_native.exe';Session=7;CreationUnixMs=[long]1794000000000;ExecutablePath='D:\probe\vad_native.exe'})
 $f.Snapshot.RunningTasks=@(@{Path='\';Name='MoirenPending';State='Queued'})
 $f.Snapshot.PresentRawChildren=@(@{InstanceId='MOIREN\VAD_RAW\held';Status='OK'})
 Prepare $f '' 'Audit';$r=Run $f
 Check ($r.ObservationAccepted-and-not$r.Ready-and-not$r.NativeApproved-and-not$r.Qualified) 'Audit observation was lost or authorized admission.'
 Check ($f.Session.RetainedOwner.Closed-and$null-ne$r.ReceiptPin) 'Busy audit did not settle and persist.'
 Assert-RemovalPreflightObservation $f.Session
 Reject {Assert-RemovalPreflightSession $f.Session}
 $saved=[IO.File]::ReadAllText($r.ReceiptPin.Path)|ConvertFrom-Json
 Check ($saved.ObservationMode-ceq'Audit'-and$saved.Snapshot.NativeOwners.Count-eq1-and$saved.Snapshot.RunningTasks.Count-eq1-and$saved.Snapshot.PresentRawChildren.Count-eq1) 'Busy evidence was filtered out.'
}
Case 'audit-mode-still-refuses-malformed-busy-rows-package-and-freshness' {
 foreach($mode in @('native','task','raw','cap','driver','stale')){
  $f=Fixture
  switch($mode){
   'native'{$f.Snapshot.NativeOwners=@('busy')}
   'task'{$f.Snapshot.RunningTasks=@(@{Path='\';Name='MoirenPending';State=7})}
   'raw'{$f.Snapshot.PresentRawChildren=@(@{InstanceId='other';Status='OK'})}
   'cap'{$f.Snapshot.PresentRawChildren=@(1..257|ForEach-Object{@{InstanceId=('MOIREN\VAD_RAW\'+$_);Status='OK'}})}
   'driver'{$f.Snapshot.Driver.SysSha256='b'*64}
   'stale'{$f.Snapshot.ObservedUtc=[DateTimeOffset]::UtcNow.AddMinutes(-10).ToString('o')}
  }
  Prepare $f '' 'Audit';$r=Run $f
  Check (-not$r.ObservationAccepted-and-not$r.Ready-and$r.Error-and$f.Session.RetainedOwner.Closed) ('Malformed audit accepted: '+$mode)
 }
}
Case 'audit-mode-and-original-observation-receipt-cannot-be-relabelled' {
 $f=Fixture;Prepare $f '' 'Audit';$r=Run $f
 Check $r.ObservationAccepted 'Empty audit observation refused.'
 $f.Session.ObservationMode='Admission'
 Reject {Assert-RemovalPreflightObservation $f.Session}
 $f.Session.ObservationMode='Audit'
 [IO.File]::AppendAllText($r.ReceiptPin.Path,' ')
 Reject {Assert-RemovalPreflightObservation $f.Session}
}
Case 'pending-real-inner-child-is-settled-through-original-handle-after-collector-failure' {
 $f=Fixture;Prepare $f
 $source=@'
function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){
 $exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
 $release=Join-Path ([IO.Path]::GetDirectoryName($EvidenceDirectory)) 'release-inner'
 $code='while(-not [IO.File]::Exists('+("'"+$release+"'" )+')){Start-Sleep -Milliseconds 20};exit 0'
 $o=Start-FiniteTool -Executable $exe -ExpectedSha256 (Get-FileHash $exe).Hash -OutputDirectory $EvidenceDirectory `
  -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-Command',$code) -TimeoutSeconds 0.02
 $ToolOwners.Add($o);$null=Observe-FiniteTool $o;throw 'Original inner timeout'
}
'@
 [IO.File]::WriteAllText($f.Session.CollectorPath,$source,[Text.UTF8Encoding]::new($false))
 $f.Session=New-RemovalPreflightSession -EvidenceDirectory $f.Session.EvidenceDirectory `
  -CollectorPath $f.Session.CollectorPath -CollectorSha256 (Get-FileHash $f.Session.CollectorPath).Hash -UserRebootConfirmed
 $script:sessions.Add($f.Session)
 Start-RemovalPreflightSession $f.Session;$r=$null
 try{
  for($i=0;$i-lt100;$i++){$r=Observe-RemovalPreflightSession $f.Session -ObservationMilliseconds 25;if($f.Session.RetainedOwner.Completed){break}}
  Check ($r.NeedsObservation-and-not$r.Ready-and$f.Session.RetainedOwner.ToolOwners.Count-eq1) 'Pending exact child disappeared.'
  $o=$f.Session.RetainedOwner.ToolOwners[0]
  Check ($o.Process-is[Diagnostics.Process]-and$o.TimedOut-and-not$o.Closed) 'Live process closed or timeout erased.'
  Reject {Start-RemovalPreflightSession $f.Session}
 }finally{[IO.File]::WriteAllText((Join-Path $f.Root 'release-inner'),'exit naturally')}
 for($i=0;$i-lt100-and$r.NeedsObservation;$i++){$r=Observe-RemovalPreflightSession $f.Session -ObservationMilliseconds 100}
 Check (-not$r.Ready-and-not$r.NeedsObservation-and$o.Closed-and$o.TimedOut-and$f.Session.RetainedOwner.Closed) 'Late child not settled or became readiness.'
}
foreach($s in $script:sessions){
 if($null-ne$s.RetainedOwner-and-not$s.RetainedOwner.Closed){
  for($i=0;$i-lt100;$i++){$r=Observe-RemovalPreflightSession $s -ObservationMilliseconds 100;if(-not$r.NeedsObservation){break}}
  Check $s.RetainedOwner.Closed 'Harmless managed collector unresolved: retained.'
 }
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' COLLECTORS=synthetic-runspace NATIVE_EXECUTED=false')
if($failed){exit 1}
