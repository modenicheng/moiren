#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$no=$false;try{&$Action|Out-Null}catch{$no=$true};Check $no 'Expected refusal.'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
$library=Join-Path $PSScriptRoot 'session-identity.ps1'
Case 'definitions-only-session-discovery-api' {
 Check (Test-Path -LiteralPath $library) 'Retained process-based session discovery is missing.'
 . $library
 foreach($name in @('New-RemovalSessionIdentity','Read-RemovalSessionIdentity','Assert-RemovalSessionIdentity')){
  Check ($null-ne(Get-Command $name -ErrorAction SilentlyContinue)) ('Missing '+$name)
 }
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 PROCESS_ENUMERATIONS=0 TOKEN_READS=0';exit 1}
. $library

# Every process, handle, image and token is synthetic. Exercise the production
# metadata composition, replacing only enumeration/open and the reused token DDI.
$script:targetSid='S-1-5-21-3080296863-1242338503-1346362208-1001'
$script:otherSid='S-1-5-21-100-200-300-1002'
function Assert-RemovalTaskHost {$script:f.HostChecks++}
function Get-RemovalSessionExplorerPath {return 'C:\Windows\explorer.exe'}
function Get-RemovalSessionProcessIds {
 $script:f.Enumerations++
 if($script:f.FailEnumeration){throw 'mock enumeration unavailable'}
 return $script:f.Ids
}
function Open-RemovalSessionProcess([int]$ProcessId) {
 $script:f.Opens++
 if(-not$script:f.Processes.ContainsKey($ProcessId)){throw 'mock process open unavailable'}
 return $script:f.Processes[$ProcessId]
}
function Read-RemovalWorkerToken($Process) {
 $script:f.TokenReads++
 Check (@($script:f.Owner.Candidates|Where-Object{[Object]::ReferenceEquals($_.Process,$Process)}).Count-eq1) 'Metadata read before original Process retention.'
 if($script:f.FailToken){throw 'mock token unavailable'}
 return $Process.Token
}
function Fixture {
 $script:f=@{Ids=@();Processes=@{};Owner=$null;HostChecks=0;Enumerations=0;Opens=0;TokenReads=0;FailEnumeration=$false;FailToken=$false}
}
function Add-Explorer([int]$Id,[int]$SessionId,[string]$Sid=$script:targetSid) {
 $p=[pscustomobject]@{Id=$Id;SessionId=$SessionId;StartTime=[DateTime]'2026-11-08T01:00:00Z';HasExited=$false
  MainModule=[pscustomobject]@{FileName='C:\Windows\explorer.exe'};Refreshes=0;HandleFailure=$false
  Token=@{Sid=$Sid;IsAdministrator=$false;IsElevated=$false}}
 $p|Add-Member ScriptMethod Refresh {$this.Refreshes++}
 $p|Add-Member ScriptProperty Handle {
  Check (@($script:f.Owner.Candidates|Where-Object{[Object]::ReferenceEquals($_.Process,$this)}).Count-eq1) 'Handle read before original Process retention.'
  if($this.HandleFailure){throw 'mock handle unavailable'}
  return [IntPtr]123
 }
 $script:f.Ids+=,$Id;$script:f.Processes[$Id]=$p
 return $p
}
function Owner([int]$SessionId=0) {
 $script:f.Owner=New-RemovalSessionIdentity -Sid $script:targetSid -WorkerSessionId $SessionId
 return $script:f.Owner
}
Case 'constructor-never-enumerates-or-reads-tokens' {
 Fixture;$o=Owner
 Check (-not$o.Attempted-and-not$o.Ready-and$o.Candidates.Count-eq0-and$f.Enumerations-eq0-and$f.TokenReads-eq0) 'Constructor performed discovery.'
}
Case 'unique-observed-nonzero-session-not-session-one-and-retained-handle-refresh' {
 Fixture;$p=Add-Explorer 41 7;$o=Owner
 $r=Read-RemovalSessionIdentity $o
 Check ($o.Ready-and$r.SessionId-eq7-and$r.Sid-ceq$targetSid-and$r.SchemaVersion-eq1-and-not$r.NativeApproved-and-not$r.AudioAccessProven) 'Observed session or discovery-only meaning lost.'
 Check ($o.Candidates.Count-eq1-and[Object]::ReferenceEquals($o.Candidates[0].Process,$p)-and$r.ProcessId-eq41-and$r.CreationUtcTicks-eq$p.StartTime.ToUniversalTime().Ticks) 'Original process identity not retained.'
 $opens=$f.Opens;Assert-RemovalSessionIdentity $o
 Check ($f.Enumerations-eq1-and$f.Opens-eq$opens-and$f.TokenReads-eq2) 'Revalidation reacquired PID or skipped original token.'
}
Case 'multiple-valid-explorers-in-one-session-remain-one-session' {
 Fixture;$null=Add-Explorer 41 8;$null=Add-Explorer 42 8;$o=Owner
 $r=Read-RemovalSessionIdentity $o
 Check ($r.SessionId-eq8-and$o.SelectedCandidates.Count-eq2-and$o.Candidates.Count-eq2) 'Process count confused with session count.'
}
Case 'multiple-target-sessions-require-explicit-observed-selection' {
 Fixture;$null=Add-Explorer 41 7;$null=Add-Explorer 42 8;$o=Owner
 Reject {Read-RemovalSessionIdentity $o}
 Check (-not$o.Ready-and$o.Candidates.Count-eq2-and$o.Error) 'Ambiguity discarded candidate owners.'
 Fixture;$null=Add-Explorer 41 7;$p=Add-Explorer 42 8;$o=Owner 8
 $r=Read-RemovalSessionIdentity $o
 Check ($r.SessionId-eq8-and[Object]::ReferenceEquals($o.SelectedCandidates[0].Process,$p)) 'Explicit session was not matched to observed candidate.'
}
Case 'explicit-session-cannot-invent-a-missing-or-session-zero-user' {
 Fixture;$null=Add-Explorer 41 7;$o=Owner 9;Reject {Read-RemovalSessionIdentity $o}
 Fixture;$null=Add-Explorer 41 0;$o=Owner;Reject {Read-RemovalSessionIdentity $o}
 Fixture;$o=Owner 7;Reject {Read-RemovalSessionIdentity $o}
}
Case 'foreign-sid-is-not-adopted-or-confused-with-target-session' {
 Fixture;$null=Add-Explorer 41 9 $otherSid;$null=Add-Explorer 42 6;$o=Owner
 $r=Read-RemovalSessionIdentity $o
 Check ($r.SessionId-eq6-and$o.Candidates.Count-eq2-and$o.SelectedCandidates.Count-eq1) 'Foreign user affected target selection.'
}
Case 'selected-session-refresh-does-not-adopt-or-depend-on-unselected-processes' {
 Fixture;$foreign=Add-Explorer 41 9 $otherSid;$other=Add-Explorer 42 8;$selected=Add-Explorer 43 7;$o=Owner 7
 $null=Read-RemovalSessionIdentity $o;$foreign.HasExited=$true;$other.HasExited=$true
 $reads=$f.TokenReads;Assert-RemovalSessionIdentity $o
 Check ($o.Ready-and$f.TokenReads-eq$reads+1-and[Object]::ReferenceEquals($o.SelectedCandidates[0].Process,$selected)) 'Unselected user/session became a refresh prerequisite.'
}
Case 'exact-windows-image-limited-token-and-typed-identity-are-required' {
 foreach($mode in @('image','admin','elevated','null-elevation','string-admin','bad-sid','pid','session','exited','zero-handle')){
  Fixture;$p=Add-Explorer 41 7
  switch($mode){
   'image'{$p.MainModule.FileName='C:\Other\explorer.exe'}
   'admin'{$p.Token.IsAdministrator=$true}
   'elevated'{$p.Token.IsElevated=$true}
   'null-elevation'{$p.Token.IsElevated=$null}
   'string-admin'{$p.Token.IsAdministrator='false'}
   'bad-sid'{$p.Token.Sid=$null}
   'pid'{$p.Id=42}
   'session'{$p.SessionId='7'}
   'exited'{$p.HasExited=$true}
   'zero-handle'{$p|Add-Member ScriptProperty Handle {[IntPtr]::Zero} -Force}
  }
  $o=Owner;Reject {Read-RemovalSessionIdentity $o}
  Check (-not$o.Ready-and$o.Candidates.Count-eq1-and[Object]::ReferenceEquals($o.Candidates[0].Process,$p)) ('Failed candidate lost original owner: '+$mode)
 }
}
Case 'enumeration-and-open-failures-are-sticky-with-no-rediscovery' {
 foreach($mode in @('enumerate','open','duplicate','malformed')){
  Fixture;$p=Add-Explorer 41 7
  switch($mode){'enumerate'{$f.FailEnumeration=$true};'open'{$f.Processes.Clear()};'duplicate'{$f.Ids+=,41};'malformed'{$f.Ids=@('41')}}
  $o=Owner;Reject {Read-RemovalSessionIdentity $o};$enumerations=$f.Enumerations;$opens=$f.Opens
  Reject {Read-RemovalSessionIdentity $o}
  Check ($f.Enumerations-eq$enumerations-and$f.Opens-eq$opens-and-not$o.Ready) 'Failed discovery retried/reacquired.'
 }
}
Case 'handle-and-token-failures-retain-process-before-metadata' {
 foreach($mode in @('handle','token')){
  Fixture;$p=Add-Explorer 41 7
  if($mode-ceq'handle'){$p.HandleFailure=$true}else{$f.FailToken=$true}
  $o=Owner;Reject {Read-RemovalSessionIdentity $o}
  Check ($o.Candidates.Count-eq1-and[Object]::ReferenceEquals($o.Candidates[0].Process,$p)-and$o.Error) 'Uncertain Process discarded.'
 }
}
Case 'revalidation-refuses-changed-original-process-session-token-path-or-evidence' {
 foreach($mode in @('exit','creation','session','sid','elevation','image','process','snapshot','requested-session','target-sid','expected-image')){
  Fixture;$p=Add-Explorer 41 7;$o=Owner 7;$null=Read-RemovalSessionIdentity $o
  switch($mode){
   'exit'{$p.HasExited=$true};'creation'{$p.StartTime=$p.StartTime.AddSeconds(1)}
   'session'{$p.SessionId=8};'sid'{$p.Token.Sid=$otherSid};'elevation'{$p.Token.IsElevated=$true}
   'image'{$p.MainModule.FileName='C:\Other\explorer.exe'}
   'process'{$o.Candidates[0].Process=[pscustomobject]@{Id=41}}
   'snapshot'{$o.Snapshot.SessionId=8};'requested-session'{$o.RequestedSessionId=8}
   'target-sid'{$o.Sid=$otherSid};'expected-image'{$o.ExplorerPath='C:\Other\explorer.exe'}
  }
  $opens=$f.Opens;Reject {Assert-RemovalSessionIdentity $o}
  Check (-not$o.Ready-and$o.Error-and$f.Opens-eq$opens-and$f.Enumerations-eq1) ('Stale selection survived: '+$mode)
 }
}
Case 'fresh-validation-does-not-depend-on-desktop-unlock-or-wts-token' {
 Fixture;$p=Add-Explorer 41 7;$p|Add-Member NoteProperty DesktopLocked $true
 $o=Owner 7;$r=Read-RemovalSessionIdentity $o;Assert-RemovalSessionIdentity $o
 Check ($r.SessionId-eq7-and$o.Ready-and-not$r.AudioAccessProven) 'Discovery invented unlock/audio admission.'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' PROCESS_ENUMERATIONS=mocked TOKEN_READS=mocked WTS_CALLS=0 AUDIO_CALLS=0')
if($failed){exit 1}
