#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$no=$false;try{&$Action|Out-Null}catch{$no=$true};Check $no 'Expected refusal.'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
$library=Join-Path $PSScriptRoot 'target-readiness.ps1'
Case 'actual-composed-target-readiness-api' {
 Check (Test-Path -LiteralPath $library) 'Actual collector/user-session readiness composition is missing.'
 . $library
 foreach($name in @('New-RemovalTargetReadiness','Start-RemovalTargetReadiness','Observe-RemovalTargetReadiness','Assert-RemovalTargetReadiness')){
  Check ($null-ne(Get-Command $name -ErrorAction SilentlyContinue)) ('Missing '+$name)
 }
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_EXECUTED=false';exit 1}
. $library
$root=Join-Path $PSScriptRoot ('logs\target-readiness-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$owners=New-Object Collections.Generic.List[object]
function Assert-RemovalTaskHost {}
function Get-RemovalSessionExplorerPath {'C:\Windows\explorer.exe'}
function Get-RemovalSessionProcessIds {$script:f.Enumerations++;return @([int]41)}
function Open-RemovalSessionProcess([int]$ProcessId){$script:f.Opens++;return $script:f.Process}
function Read-RemovalWorkerToken($Process){return $Process.Token}
function Fixture {
 $dir=Join-Path $root ([Guid]::NewGuid().ToString('N'));$null=[IO.Directory]::CreateDirectory($dir)
 $process=[pscustomobject]@{Id=41;SessionId=7;StartTime=[DateTime]'2026-11-08T01:00:00Z';HasExited=$false;Handle=[IntPtr]123
  MainModule=[pscustomobject]@{FileName='C:\Windows\explorer.exe'}
  Token=@{Sid='S-1-5-21-3080296863-1242338503-1346362208-1001';IsAdministrator=$false;IsElevated=$false}}
 $process|Add-Member ScriptMethod Refresh {}
 $script:f=[pscustomobject]@{Root=$dir;Process=$process;Enumerations=0;Opens=0;Owner=$null}
 $now=[DateTimeOffset]::UtcNow
 $p=@{schema_version=1;host='LAPTOP-NP49RG6H';sid=$process.Token.Sid;session=[Diagnostics.Process]::GetCurrentProcess().SessionId
  CollectorStartedUtc=$now.AddSeconds(-1).ToString('o');BootUtc=$now.AddSeconds(-60).ToString('o');ObservedUtc=$now.ToString('o')
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
  NativeTestStarted=$false;RebootRequested=$false;Qualified=$false}
 $data=[Management.Automation.PSSerializer]::Serialize($p,16).Replace("'","''")
 $body="`$s=[Management.Automation.PSSerializer]::Deserialize('$data');"+
  'foreach($key in @("NativeOwners","RunningTasks","PresentRawChildren","ExternalOwners","Defaults","Render")){$s[$key]=[object[]]$s[$key]};'+
  'foreach($key in @("Configured","Running")){$s.Security[$key]=[object[]]$s.Security[$key]};return $s'
 $collector=Join-Path $dir 'collector.ps1'
 [IO.File]::WriteAllText($collector,('function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){'+$body+'}'),[Text.UTF8Encoding]::new($false))
 $o=New-RemovalTargetReadiness -EvidenceDirectory (Join-Path $dir 'evidence') -UserRebootConfirmed `
  -CollectorPath $collector -CollectorSha256 (Get-FileHash $collector).Hash
 $script:f.Owner=$o;$owners.Add($o)
 return $o
}
function Run($Owner){
 Start-RemovalTargetReadiness $Owner
 for($i=0;$i-lt100;$i++){$r=Observe-RemovalTargetReadiness $Owner -ObservationMilliseconds 100;if(-not$r.NeedsObservation){return $r}}
 throw 'Harmless readiness runspace did not complete.'
}
Case 'constructor-retains-both-owners-without-discovery-or-collection' {
 $o=Fixture
 Check ($f.Enumerations-eq0-and$f.Opens-eq0-and-not$o.StartAttempted-and$null-eq$o.Preflight.CollectorOwner) 'Constructor started work.'
 Check ([Object]::ReferenceEquals($o.Preflight,$o.RetainedPreflight)-and[Object]::ReferenceEquals($o.UserSession,$o.RetainedUserSession)) 'Nested original owners not retained.'
}
Case 'actual-runspace-and-session-discovery-compose-with-distinct-controller-and-worker' {
 $o=Fixture;$r=Run $o
 Check ($r.Ready-and-not$r.NeedsObservation-and-not$r.NativeApproved-and-not$r.Qualified) 'Composed readonly readiness failed/promoted.'
 Check ($o.UserSession.Snapshot.SessionId-eq7-and$o.Preflight.ControllerSession-eq[Diagnostics.Process]::GetCurrentProcess().SessionId) 'Controller relabelled as worker.'
 Assert-RemovalTargetReadiness $o
 $saved=[IO.File]::ReadAllText($o.ReceiptPin.Path)|ConvertFrom-Json
 Check ($saved.WorkerIdentity.SessionId-eq7-and$saved.ControllerSession-eq$o.Preflight.ControllerSession-and$saved.ExecutionMode-ceq'InjectedCollectorSource') 'Receipt lost observed session or test-source status.'
 Check ($saved.PreflightReceipt.Sha256-ceq$o.Preflight.ReceiptPin.Sha256-and-not$saved.NativeApproved-and-not$saved.AudioAccessProven) 'Readiness receipt lost original preflight or invented audio approval.'
 $opens=$f.Opens;Assert-RemovalTargetReadiness $o
 Check ($f.Enumerations-eq1-and$f.Opens-eq$opens) 'Readiness assertion reopened/rediscovered foreign process.'
 Reject {Start-RemovalTargetReadiness $o}
}
Case 'session-discovery-failure-prevents-collector-dispatch' {
 $o=Fixture;$f.Process.Token.IsElevated=$true
 Reject {Start-RemovalTargetReadiness $o}
 Check ($null-eq$o.Preflight.RetainedOwner-and-not$o.Ready-and$o.Error) 'Failed Limited discovery dispatched collector.'
 Reject {Start-RemovalTargetReadiness $o}
}
Case 'explicit-worker-session-must-be-actually-observed' {
 $o=Fixture
 $other=New-RemovalTargetReadiness -EvidenceDirectory (Join-Path $f.Root 'other') -UserRebootConfirmed -WorkerSessionId 9 `
  -CollectorPath $o.Preflight.CollectorPath -CollectorSha256 $o.Preflight.CollectorSha256
 $owners.Add($other);Reject {Start-RemovalTargetReadiness $other}
 Check ($null-eq$other.Preflight.RetainedOwner) 'Unobserved selected session dispatched collector.'
}
Case 'changed-original-session-token-refuses-readiness-with-no-rediscovery' {
 foreach($mode in @('exit','session','token','snapshot')){
  $o=Fixture;$null=Run $o
  switch($mode){'exit'{$f.Process.HasExited=$true};'session'{$f.Process.SessionId=8};'token'{$f.Process.Token.IsElevated=$true};'snapshot'{$o.UserSession.Snapshot.SessionId=8}}
  $opens=$f.Opens;Reject {Assert-RemovalTargetReadiness $o}
  Check (-not$o.Ready-and$o.Error-and$f.Opens-eq$opens-and$f.Enumerations-eq1) ('Changed user identity accepted: '+$mode)
 }
}
Case 'original-nested-owners-and-readiness-evidence-cannot-be-replaced' {
 foreach($mode in @('preflight','session','receipt','preflight-receipt','preflight-repin','directory')){
  $o=Fixture;$null=Run $o
  switch($mode){'preflight'{$o.Preflight=[pscustomobject]@{Ready=$true}};'session'{$o.UserSession=[pscustomobject]@{Ready=$true}}
   'receipt'{[IO.File]::AppendAllText($o.ReceiptPin.Path,' ')}
   'preflight-receipt'{[IO.File]::AppendAllText($o.Preflight.ReceiptPin.Path,' ')}
   'preflight-repin'{$other=Join-Path $f.Root 'replacement-receipt.json';[IO.File]::WriteAllText($other,'{}');$o.Preflight.ReceiptPin=[pscustomobject]@{Path=$other;Bytes=2;Sha256=(Get-FileHash $other).Hash}}
   'directory'{$o.EvidenceDirectory=Join-Path $f.Root 'substituted'}}
  Reject {Assert-RemovalTargetReadiness $o}
 }
}
Case 'late-original-collector-is-drained-but-never-accepted-or-relaunched' {
 $o=Fixture;Start-RemovalTargetReadiness $o
 $original=$o.Preflight.RetainedOwner;$original.DeadlineMilliseconds=[long]0
 $r=Observe-RemovalTargetReadiness $o -ObservationMilliseconds 1
 Reject {Start-RemovalTargetReadiness $o}
 for($i=0;$i-lt100-and$r.NeedsObservation;$i++){$r=Observe-RemovalTargetReadiness $o -ObservationMilliseconds 100}
 Check (-not$r.Ready-and-not$r.NeedsObservation-and$original.Closed-and$original.TimedOut-and[Object]::ReferenceEquals($original,$o.Preflight.RetainedOwner)) 'Original timeout gained acceptance or lost owner.'
}
Case 'production-entry-uses-composed-readiness-not-bare-collector' {
 $entry=[IO.File]::ReadAllText((Join-Path $PSScriptRoot 'invoke-preflight.ps1'))
 foreach($name in @('New-RemovalTargetReadiness','Start-RemovalTargetReadiness','Observe-RemovalTargetReadiness','Assert-RemovalTargetReadiness')){
  Check ($entry.Contains($name)) ('Entry not wired to '+$name)
 }
}
foreach($o in $owners){
 if($null-ne$o.RetainedPreflight.RetainedOwner-and-not$o.RetainedPreflight.RetainedOwner.Closed){
  for($i=0;$i-lt100;$i++){$r=Observe-RemovalPreflightSession $o.RetainedPreflight -ObservationMilliseconds 100;if(-not$r.NeedsObservation){break}}
  Check $o.RetainedPreflight.RetainedOwner.Closed 'Original harmless collector unresolved.'
 }
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' COLLECTORS=synthetic-runspace SESSION_TOKENS=mocked NATIVE_EXECUTED=false')
if($failed){exit 1}
