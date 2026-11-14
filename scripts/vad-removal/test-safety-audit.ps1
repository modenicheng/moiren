#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){throw 'Run synthetic safety audit tests on a non-target host.'}
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$no=$false;try{&$Action|Out-Null}catch{$no=$true};Check $no 'Expected refusal.'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
$library=Join-Path $PSScriptRoot 'safety-audit.ps1'
Case 'retained-safety-audit-api-loads-without-dispatch' {
 Check (Test-Path -LiteralPath $library) 'Retained after-safety audit is missing.'
 . $library
 foreach($name in @('New-RemovalSafetyAudit','Start-RemovalSafetyAudit','Observe-RemovalSafetyAudit','Assert-RemovalSafetyAudit')){
  Check ($null-ne(Get-Command $name -ErrorAction SilentlyContinue)) ('Missing '+$name)
 }
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_EXECUTED=false';exit 1}
. $library
$root=Join-Path $PSScriptRoot ('logs\safety-audit-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$audits=New-Object Collections.Generic.List[object]
function Assert-RemovalTaskHost {}
function Get-RemovalSessionExplorerPath {'C:\Windows\explorer.exe'}
function Get-RemovalSessionProcessIds {return @([int]41)}
function Open-RemovalSessionProcess([int]$ProcessId){return $script:fixture.Process}
function Read-RemovalWorkerToken($Process){return $Process.Token}
function Snapshot {
 $now=[DateTimeOffset]::UtcNow
 @{schema_version=1;host='LAPTOP-NP49RG6H';sid='S-1-5-21-3080296863-1242338503-1346362208-1001'
  session=[Diagnostics.Process]::GetCurrentProcess().SessionId
  CollectorStartedUtc=$now.ToString('o');BootUtc=$now.AddMinutes(-10).ToString('o');ObservedUtc=$now.ToString('o')
  UptimeMilliseconds=[uint64]600000;BootAfterDeployment=$true;SignaturesVerified=$true;CatalogMembersVerified=$true
  NativeOwners=@();RunningTasks=@();PresentRawChildren=@()
  ExternalOwners=@(@{Pid=42;Name='QQMusic.exe';Session=7;CreationUnixMs=[long]1794000000000;ExecutablePath='D:\QQMusic\QQMusic.exe'},
   @{Pid=43;Name='obs64.exe';Session=7;CreationUnixMs=[long]1794000000001;ExecutablePath='D:\External\obs64.exe'})
  ExternalOwnersProvenance='ObservedComplete';ExternalOwnersObservationStartedUtc=$now.ToString('o');ExternalOwnersObservedUtc=$now.ToString('o')
  Defaults=@(foreach($flow in @('capture','render')){foreach($role in @('communications','console','multimedia')){
   @{flow=$flow;role=$role;id='endpoint-'+$flow}}})
  Render=@(@{id='render-one';volume_scalar=[double]0.5;muted=$false},@{id='render-two';volume_scalar=[double]0.25;muted=$true})
  Security=@{SecureBoot=$false;Vbs=2;CodeIntegrity=2;UserCodeIntegrity=0;Configured=@(1,2);Running=@(1,2);Bcd=('a'*64)}
  Driver=@{Version='3.1.1.613';Inf='oem207.inf';State='Running';InstanceId='ROOT\MEDIA\0001';PnpStatus='OK';ProblemCode=0
   SysPath='C:\Windows\System32\DriverStore\FileRepository\reviewed\MoirenVad.sys'
   SysSha256='0A1DFF698923B2DF39D0625979D3CD1AEE486228396C1BF601EADEA565EB3B76'
   InfSha256='24574ED3BAE942EF7ED91601513F8F0B181272BF1F15D2067F0F6EE2014F827C'}
  NativeTestStarted=$false;RebootRequested=$false;Qualified=$false}
}
function Write-Collector([string]$Path,$Snapshot,[string]$Prefix='',[int]$AgeSeconds=0){
 $data=[Management.Automation.PSSerializer]::Serialize($Snapshot,16).Replace("'","''")
 $body="`$age=$AgeSeconds; `$begin=[DateTimeOffset]::UtcNow.AddSeconds(-`$age); $Prefix `$s=[Management.Automation.PSSerializer]::Deserialize('$data');"+@'
foreach($key in @('NativeOwners','RunningTasks','PresentRawChildren','ExternalOwners','Defaults','Render')){
 if($null-ne$s[$key]){$s[$key]=[object[]]$s[$key]}
}
foreach($key in @('Configured','Running')){$s.Security[$key]=[object[]]$s.Security[$key]}
$now=[DateTimeOffset]::UtcNow.AddSeconds(-$age)
$s.CollectorStartedUtc=$begin.ToString('o');$s.ObservedUtc=$now.ToString('o')
$s.ExternalOwnersObservationStartedUtc=$now.ToString('o');$s.ExternalOwnersObservedUtc=$now.ToString('o')
$s.UptimeMilliseconds=[uint64]($now-[DateTimeOffset]$s.BootUtc).TotalMilliseconds
return $s
'@
 [IO.File]::WriteAllText($Path,('function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){'+$body+'}'),[Text.UTF8Encoding]::new($false))
}
function Fixture([int]$AgeSeconds=0) {
 $dir=Join-Path $root ([Guid]::NewGuid().ToString('N'));$null=[IO.Directory]::CreateDirectory($dir)
 $process=[pscustomobject]@{Id=41;SessionId=7;StartTime=[DateTime]'2026-11-08T01:00:00Z';HasExited=$false;Handle=[IntPtr]123
  MainModule=[pscustomobject]@{FileName='C:\Windows\explorer.exe'}
  Token=@{Sid='S-1-5-21-3080296863-1242338503-1346362208-1001';IsAdministrator=$false;IsElevated=$false}}
 $process|Add-Member ScriptMethod Refresh {}
 $script:fixture=[pscustomobject]@{Root=$dir;Process=$process;Before=$null;After=(Snapshot);Collector=(Join-Path $dir 'after.ps1');Audit=$null}
 $beforePath=Join-Path $dir 'before.ps1';Write-Collector $beforePath $fixture.After '' $AgeSeconds
 $b=New-RemovalTargetReadiness -EvidenceDirectory (Join-Path $dir 'before') -UserRebootConfirmed `
  -CollectorPath $beforePath -CollectorSha256 (Get-FileHash $beforePath).Hash
 Start-RemovalTargetReadiness $b
 for($i=0;$i-lt100;$i++){$r=Observe-RemovalTargetReadiness $b -ObservationMilliseconds 100;if(-not$r.NeedsObservation){break}}
 Check $r.Ready ('Actual synthetic baseline failed: '+$r.Error)
 $fixture.Before=$b
 return $fixture
}
function Prepare($f,[string]$Prefix=''){
 Write-Collector $f.Collector $f.After $Prefix
 $f.Audit=New-RemovalSafetyAudit -BeforeReadiness $f.Before -EvidenceDirectory (Join-Path $f.Root 'after') -UserRebootConfirmed `
  -CollectorPath $f.Collector -CollectorSha256 (Get-FileHash $f.Collector).Hash
 $audits.Add($f.Audit)
 return $f.Audit
}
function Run($Audit){
 Start-RemovalSafetyAudit $Audit
 for($i=0;$i-lt100;$i++){$r=Observe-RemovalSafetyAudit $Audit -ObservationMilliseconds 100;if(-not$r.NeedsObservation){return $r}}
 throw 'Synthetic audit did not settle.'
}
Case 'constructor-requires-actual-retained-readiness-and-reboot-confirmation' {
 $f=Fixture
 Reject {New-RemovalSafetyAudit -BeforeReadiness $f.Before -EvidenceDirectory (Join-Path $f.Root 'no-confirm')}
 Reject {New-RemovalSafetyAudit -BeforeReadiness ([pscustomobject]@{Ready=$true}) -EvidenceDirectory (Join-Path $f.Root 'detached') -UserRebootConfirmed}
 $o=Prepare $f
 Check (-not$o.StartAttempted-and$null-eq$o.AfterPreflight.RetainedOwner-and-not(Test-Path (Join-Path $f.Root 'after'))) 'Constructor collected.'
 Check ([Object]::ReferenceEquals($o.BeforeReadiness,$f.Before)) 'Actual baseline owner not retained.'
}
Case 'unchanged-actual-after-runspace-produces-safe-pinned-evidence-without-authority' {
 $f=Fixture;$o=Prepare $f;$r=Run $o
 Check ($r.Complete-and$r.Safe-and-not$r.Unsafe-and-not$r.NeedsObservation-and-not$r.NativeApproved-and-not$r.Qualified) ('Unchanged audit refused: '+$r.Error)
 Check ($o.AfterPreflight.RetainedOwner.Completed-and$o.AfterPreflight.RetainedOwner.Closed-and-not$o.AfterPreflight.Ready) 'Audit did not settle or became admission.'
 Assert-RemovalSafetyAudit $o
 $saved=[IO.File]::ReadAllText($r.ReceiptPin.Path)|ConvertFrom-Json
 Check ($saved.Safe-and-not$saved.Qualified-and-not$saved.NativeApproved-and$saved.BeforeReadinessReceipt.Sha256-ceq$f.Before.ReceiptPin.Sha256) 'Original baseline not linked.'
 Check ($saved.AfterSnapshot.ExternalOwners.Count-eq2-and$saved.AfterPreflightReceipt.Sha256-ceq$o.AfterPreflight.ReceiptPin.Sha256) 'After original snapshot/receipt missing.'
 Reject {Start-RemovalSafetyAudit $o}
 Reject {Assert-RemovalPreflightSession $o.AfterPreflight}
}
Case 'canonical-comparison-ignores-row-map-order-and-equivalent-boot-zone' {
 $f=Fixture
 $f.After.Defaults=@($f.After.Defaults[5..0]);$f.After.Render=@($f.After.Render[1..0]);$f.After.ExternalOwners=@($f.After.ExternalOwners[1..0])
 $f.After.Security.Configured=@(2,1);$f.After.Security.Running=@(2,1)
 $f.After.BootUtc=([DateTimeOffset]$f.After.BootUtc).ToOffset([TimeSpan]::FromHours(8)).ToString('o')
 $o=Prepare $f;$r=Run $o;Check $r.Safe ('Ordering became safety mutation: '+$r.Error);Assert-RemovalSafetyAudit $o
}
Case 'failed-removed-busy-state-still-collects-and-persists-unsafe-originals' {
 $f=Fixture
 $f.After.NativeOwners=@(@{Pid=90;Name='vad_native.exe';Session=7;CreationUnixMs=[long]1794000000002;ExecutablePath='D:\probe\vad_native.exe'})
 $f.After.RunningTasks=@(@{Path='\';Name='MoirenPending';State='Queued'})
 $f.After.PresentRawChildren=@(@{InstanceId='MOIREN\VAD_RAW\held';Status='OK'})
 $o=Prepare $f;$r=Run $o
 Check ($r.Complete-and$r.Unsafe-and-not$r.Safe-and$r.Differences-contains'NativeOwners'-and$r.Differences-contains'RunningTasks'-and$r.Differences-contains'PresentRawChildren') 'Busy after was discarded or accepted.'
 Check ($o.AfterPreflight.ObservationAccepted-and-not$o.AfterPreflight.Ready-and$o.AfterPreflight.RetainedOwner.Closed) 'Busy observation did not retain actual collector.'
 $saved=[IO.File]::ReadAllText($r.ReceiptPin.Path)|ConvertFrom-Json
 Check ($saved.Unsafe-and$saved.AfterSnapshot.NativeOwners.Count-eq1-and$saved.AfterSnapshot.RunningTasks.Count-eq1-and$saved.AfterSnapshot.PresentRawChildren.Count-eq1) 'Unsafe evidence lost busy rows.'
 Reject {Assert-RemovalSafetyAudit $o}
}
Case 'missing-or-replaced-external-identities-are-unsafe-without-stop-or-reopen' {
 foreach($mode in @('missing','creation','path','session','added')){
  $f=Fixture
  switch($mode){
   'missing'{$f.After.ExternalOwners=@($f.After.ExternalOwners[1])}
   'creation'{$f.After.ExternalOwners[0].CreationUnixMs++}
   'path'{$f.After.ExternalOwners[0].ExecutablePath='D:\Other\QQMusic.exe'}
   'session'{$f.After.ExternalOwners[0].Session=8}
   'added'{$f.After.ExternalOwners+=@{Pid=44;Name='QQMusic.exe';Session=7;CreationUnixMs=[long]1794000000003;ExecutablePath='D:\QQMusic\QQMusic.exe'}}
  }
  $o=Prepare $f;$r=Run $o
  Check ($r.Complete-and$r.Unsafe-and$r.Differences-contains'ExternalOwners'-and$null-ne$r.ReceiptPin) ('External change accepted/lost: '+$mode)
  $kind=if($mode-ceq'missing'){'Missing'}elseif($mode-ceq'added'){'Added'}else{'Replaced'}
  Check (@($r.ExternalIdentityChanges|Where-Object{$_.Kind-ceq$kind}).Count-eq1) ('Exact external identity change not reported: '+$mode)
  Reject {Assert-RemovalSafetyAudit $o}
 }
}
Case 'defaults-render-security-boot-and-driver-path-changes-are-unsafe' {
 foreach($mode in @('Defaults','Render','Security','BootUtc','Driver')){
  $f=Fixture
  switch($mode){
   'Defaults'{$f.After.Defaults[0].id='changed-default'}
   'Render'{$f.After.Render[0].volume_scalar=[double]0.6}
   'Security'{$f.After.Security.SecureBoot=$true}
   'BootUtc'{$f.After.BootUtc=([DateTimeOffset]$f.After.BootUtc).AddSeconds(1).ToString('o')}
   'Driver'{$f.After.Driver.SysPath='C:\Windows\Other\MoirenVad.sys'}
  }
  $o=Prepare $f;$r=Run $o
  Check ($r.Complete-and$r.Unsafe-and$r.Differences-contains$mode) ('Changed safety boundary accepted: '+$mode)
 }
}
Case 'malformed-or-wrong-package-after-preserves-raw-snapshot-and-refusal' {
 foreach($mode in @('rows','package','host','sid','session')){
  $f=Fixture
  switch($mode){
   'rows'{$f.After.PresentRawChildren=@('held')}
   'package'{$f.After.Driver.SysSha256='b'*64}
   'host'{$f.After.host='other'}
   'sid'{$f.After.sid='S-1-5-18'}
   'session'{$f.After.session=65535}
  }
  $o=Prepare $f;$r=Run $o
  Check ($r.Complete-and$r.Unsafe-and-not$r.Safe-and$r.Error-and$null-ne$r.ReceiptPin) ('Refused after snapshot discarded: '+$mode)
  $saved=[IO.File]::ReadAllText($r.ReceiptPin.Path)|ConvertFrom-Json
  Check ($null-ne$saved.AfterSnapshot-and$saved.Error-and-not$saved.Safe) 'Raw failed observation/refusal missing.'
 }
}
Case 'before-snapshot-or-receipt-mutation-still-runs-after-but-cannot-pass' {
 foreach($mode in @('snapshot','receipt','target-receipt','read-tool-pins')){
  $f=Fixture;$o=Prepare $f
  switch($mode){
   'snapshot'{$f.Before.Preflight.RetainedOwner.Shared.Snapshot.Render[0].muted=$true}
   'receipt'{[IO.File]::AppendAllText($f.Before.Preflight.ReceiptPin.Path,' ')}
   'target-receipt'{[IO.File]::AppendAllText($f.Before.ReceiptPin.Path,' ')}
   'read-tool-pins'{$f.Before.Preflight.ReadToolPinsJson='{"BcdEdit":{"Sha256":"changed-baseline-input"}}'}
  }
  $r=Run $o
  Check ($r.Complete-and$r.Unsafe-and$r.Differences-contains'BeforeEvidence'-and$o.AfterPreflight.RetainedOwner.Closed) 'Before refusal skipped after or gained safety.'
 }
}
Case 'expired-baseline-admission-is-still-comparable-as-original-historical-evidence' {
 $f=Fixture 118;$o=Prepare $f 'Start-Sleep -Milliseconds 2500;'
 $r=Run $o
 Check (([DateTimeOffset]::UtcNow-[DateTimeOffset]$f.Before.Preflight.RetainedOwner.Shared.Snapshot.ObservedUtc).TotalSeconds-gt120) 'Baseline did not expire during actual after collection.'
 Check $r.Safe 'Expired admission prevented original historical comparison.'
 Reject {Assert-RemovalTargetReadiness $f.Before}
 Check (-not$f.Before.Ready) 'Expired baseline was still fresh admission.'
 Assert-RemovalSafetyAudit $o
}
Case 'audit-construction-after-admission-expiry-does-not-skip-actual-after' {
 $f=Fixture 118
 Start-Sleep -Milliseconds 2500
 Reject {Assert-RemovalTargetReadiness $f.Before}
 $o=Prepare $f;$r=Run $o
 Check ($r.Complete-and$r.Safe-and$o.AfterPreflight.RetainedOwner.Closed) ('Historical construction skipped/refused after: '+$r.Error)
 Assert-RemovalSafetyAudit $o
}
Case 'baseline-source-receipt-or-explorer-refusal-does-not-prevent-after-construction' {
 foreach($mode in @('source','receipt','explorer')){
  $f=Fixture
  switch($mode){
   'source'{[IO.File]::AppendAllText($f.Before.Preflight.CollectorPath,"`r`n# changed baseline source, never relaunched")}
   'receipt'{[IO.File]::AppendAllText($f.Before.ReceiptPin.Path,' ')}
   'explorer'{$f.Process.HasExited=$true}
  }
  Reject {Assert-RemovalTargetReadiness $f.Before}
  $o=Prepare $f;$r=Run $o
  Check ($r.Complete-and$o.AfterPreflight.RetainedOwner.Closed-and$null-ne$r.ReceiptPin) ('Refused baseline skipped after: '+$mode)
  if($mode-cne'explorer'){Check ($r.Unsafe-and$r.Differences-contains'BeforeEvidence') ('Mutated baseline accepted: '+$mode)}
 }
}
Case 'after-original-snapshot-receipt-or-result-cannot-be-replaced' {
 foreach($mode in @('snapshot','preflight-receipt','receipt','repin','safe','owner')){
  $f=Fixture;$o=Prepare $f;$r=Run $o;Check $r.Safe 'Baseline audit not safe.'
  switch($mode){
   'snapshot'{$o.AfterPreflight.RetainedOwner.Shared.Snapshot.Render[0].muted=$true}
   'preflight-receipt'{[IO.File]::AppendAllText($o.AfterPreflight.ReceiptPin.Path,' ')}
   'receipt'{[IO.File]::AppendAllText($o.ReceiptPin.Path,' ')}
   'repin'{$o.ReceiptPin=[pscustomobject]@{Path=$o.ReceiptPin.Path;Bytes=(Get-Item $o.ReceiptPin.Path).Length;Sha256=('b'*64)}}
   'safe'{$o.Safe=$false}
   'owner'{$o.AfterPreflight=[pscustomobject]@{ObservationAccepted=$true}}
  }
  Reject {Assert-RemovalSafetyAudit $o}
  $later=Observe-RemovalSafetyAudit $o
  Check (-not$later.Safe) ('Invalidated audit returned safety: '+$mode)
 }
}
Case 'timed-out-original-runspace-drains-without-restart-or-late-safety' {
 $f=Fixture;$o=Prepare $f 'Start-Sleep -Milliseconds 250;'
 Start-RemovalSafetyAudit $o;$original=$o.AfterPreflight.RetainedOwner;$handle=$original.Invocation;$original.DeadlineMilliseconds=[long]0
 $r=Observe-RemovalSafetyAudit $o -ObservationMilliseconds 1
 Check ($r.NeedsObservation-and-not$r.Safe-and-not$original.Closed) 'Pending timeout lost original owner.'
 Reject {Start-RemovalSafetyAudit $o}
 for($i=0;$i-lt100-and$r.NeedsObservation;$i++){$r=Observe-RemovalSafetyAudit $o -ObservationMilliseconds 100}
 Check ($r.Complete-and$r.Unsafe-and$original.TimedOut-and$original.Closed-and[Object]::ReferenceEquals($handle,$original.Invocation)) 'Late result gained safety or owner replaced.'
 Check ($null-ne$r.ReceiptPin) 'Late unsafe observation not evidenced.'
}
Case 'original-inner-child-drains-even-after-baseline-evidence-refusal' {
 $f=Fixture;$o=Prepare $f
 $release=Join-Path $f.Root 'release-inner'
 $body=@'
function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){
 $exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
 $release=Join-Path ([IO.Path]::GetDirectoryName($EvidenceDirectory)) 'release-inner'
 $code='while(-not [IO.File]::Exists('+("'"+$release+"'")+')){Start-Sleep -Milliseconds 20};exit 0'
 $child=Start-FiniteTool -Executable $exe -ExpectedSha256 (Get-FileHash $exe).Hash -OutputDirectory $EvidenceDirectory `
  -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-Command',$code) -TimeoutSeconds 0.02
 $ToolOwners.Add($child);$null=Observe-FiniteTool $child;throw 'Original child refused after timeout'
}
'@
 [IO.File]::WriteAllText($f.Collector,$body,[Text.UTF8Encoding]::new($false))
 # Construct a new owner before starting, since source pins cannot be changed.
 $o=New-RemovalSafetyAudit -BeforeReadiness $f.Before -EvidenceDirectory (Join-Path $f.Root 'after') -UserRebootConfirmed `
  -CollectorPath $f.Collector -CollectorSha256 (Get-FileHash $f.Collector).Hash
 $audits.Add($o);Start-RemovalSafetyAudit $o
 try{
  for($i=0;$i-lt100;$i++){$r=Observe-RemovalSafetyAudit $o -ObservationMilliseconds 25;if($o.AfterPreflight.RetainedOwner.Completed){break}}
  $child=$o.AfterPreflight.RetainedOwner.ToolOwners[0]
  Check ($r.NeedsObservation-and-not$child.Closed-and$child.Process-is[Diagnostics.Process]) 'Original inner child lost.'
  [IO.File]::AppendAllText($f.Before.ReceiptPin.Path,' ')
  $r=Observe-RemovalSafetyAudit $o -ObservationMilliseconds 1
  Check ($r.NeedsObservation-and-not$child.Closed) 'Before refusal abandoned child.'
 }finally{[IO.File]::WriteAllText($release,'natural exit')}
 for($i=0;$i-lt100-and$r.NeedsObservation;$i++){$r=Observe-RemovalSafetyAudit $o -ObservationMilliseconds 100}
 Check ($r.Complete-and$r.Unsafe-and$child.Closed-and$child.TimedOut-and$o.AfterPreflight.RetainedOwner.Closed) 'Inner completion lost timeout or did not settle.'
}
Case 'receipt-persistence-failure-refuses-without-overwrite-or-retry' {
 $f=Fixture;$o=Prepare $f 'Start-Sleep -Milliseconds 250;'
 Start-RemovalSafetyAudit $o
 $null=[IO.Directory]::CreateDirectory($o.EvidenceDirectory)
 $path=Join-Path $o.EvidenceDirectory 'safety-audit.json'
 [IO.File]::WriteAllText($path,'original existing evidence')
 for($i=0;$i-lt100;$i++){$r=Observe-RemovalSafetyAudit $o -ObservationMilliseconds 100;if(-not$r.NeedsObservation){break}}
 Check (-not$r.Safe-and$r.Error-and[IO.File]::ReadAllText($path)-ceq'original existing evidence') 'Existing safety evidence overwritten/accepted.'
 $r=Observe-RemovalSafetyAudit $o;Check (-not$r.Safe-and$o.ReceiptAttempted) 'Persistence failure retried/accepted.'
}
foreach($o in $audits){
 $p=$o.RetainedAfterPreflight
 if($null-ne$p.RetainedOwner-and-not$p.RetainedOwner.Closed){
  for($i=0;$i-lt100;$i++){$null=Observe-RemovalSafetyAudit $o -ObservationMilliseconds 100;if($p.RetainedOwner.Closed){break}}
  Check $p.RetainedOwner.Closed 'Synthetic audit owner unresolved; retained.'
 }
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' COLLECTORS=synthetic-runspace SESSION_TOKENS=mocked NATIVE_EXECUTED=false')
if($failed){exit 1}
