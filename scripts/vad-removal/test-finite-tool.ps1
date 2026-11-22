#requires -Version 5.1
param([string]$Tool,[string]$LogRoot)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if(-not $Tool){$Tool=Join-Path $PSScriptRoot 'finite-tool.ps1'}
if(-not $LogRoot){$LogRoot=Join-Path $PSScriptRoot ('logs\test-'+[Guid]::NewGuid().ToString('N'))}
$null=[IO.Directory]::CreateDirectory($LogRoot)
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
function Reject([scriptblock]$Action){$refused=$false;try{&$Action|Out-Null}catch{$refused=$true};Check $refused 'Expected refusal'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.ToString()})}}
if(Test-Path -LiteralPath $Tool){. $Tool}
$exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$hash=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
function RunDirectory {Join-Path $LogRoot ([Guid]::NewGuid().ToString('N'))}
function ChildArguments([string]$Code){@('-NoLogo','-NoProfile','-NonInteractive','-Command',$Code)}
function StartChild([string]$Code,[double]$Seconds=15,[long]$Cap=65536){
 Start-FiniteTool -Executable $exe -ExpectedSha256 $hash -ArgumentList (ChildArguments $Code) -OutputDirectory (RunDirectory) -TimeoutSeconds $Seconds -MaxLogBytes $Cap
}
function FakeProcess([bool]$StartThrows,[bool]$IdentityThrows=$false){
 $p=[pscustomobject]@{StartInfo=$null;StartCalls=0;Disposed=$false;IdentityThrows=$IdentityThrows;StartThrows=$StartThrows}
 $p|Add-Member ScriptMethod Start {$this.StartCalls++;if($this.StartThrows){throw 'ambiguous dispatch'};return $true}
 $p|Add-Member ScriptProperty Handle {if($this.IdentityThrows){throw 'handle unavailable'};return [IntPtr]123}
 $p|Add-Member ScriptProperty Id {return 4321}
 $p|Add-Member ScriptProperty StartTime {return [DateTime]::UtcNow}
 $p|Add-Member ScriptMethod Dispose {$this.Disposed=$true}
 return $p
}
Case 'CRT-empty-space-quote-trailing-slashes' {
 Check ((Quote-FiniteToolArgument '')-ceq'""') 'Empty argument'
 Check ((Quote-FiniteToolArgument 'two words')-ceq'"two words"') 'Space argument'
 Check ((Quote-FiniteToolArgument 'a"b')-ceq'"a\"b"') 'Embedded quote'
 Check ((Quote-FiniteToolArgument 'C:\tail\')-ceq'"C:\tail\\"') 'Trailing slash'
}
Case 'bad-exe-hash-refuses-before-factory-or-log-write' {
 $script:factoryCalls=0;$path=RunDirectory
 Reject {Start-FiniteTool -Executable $exe -ExpectedSha256 ('0'*64) -ArgumentList @() -OutputDirectory $path -ProcessFactory {$script:factoryCalls++;throw 'must not reach'}}
 Check ($script:factoryCalls-eq0-and-not(Test-Path -LiteralPath $path)) 'Bad pin reached launch/write'
}
Case 'relative-exe-refused' {
 $path=RunDirectory;Reject {Start-FiniteTool -Executable 'powershell.exe' -ExpectedSha256 $hash -OutputDirectory $path}
 Check (-not(Test-Path -LiteralPath $path)) 'Relative exe wrote logs'
}
Case 'existing-log-directory-preserved' {
 $path=RunDirectory;$null=[IO.Directory]::CreateDirectory($path)
 Reject {Start-FiniteTool -Executable $exe -ExpectedSha256 $hash -OutputDirectory $path}
}
Case 'NUL-CR-LF-argv-refused-before-start' {
 foreach($bad in @("a`0b","a`rb","a`nb")){
  $path=RunDirectory;Reject {Start-FiniteTool -Executable $exe -ExpectedSha256 $hash -OutputDirectory $path -ArgumentList @($bad)}
  Check (-not(Test-Path -LiteralPath $path)) 'Unsafe argv wrote logs'
 }
}
Case 'ambiguous-dispatch-retains-exact-object-and-no-relaunch' {
 $script:fake=FakeProcess $true;$owner=Start-FiniteTool -Executable $exe -ExpectedSha256 $hash -OutputDirectory (RunDirectory) -ProcessFactory {$script:fake}
 Check ($owner.Process-eq$script:fake-and$owner.LaunchAttempted-and$owner.LaunchUnknown) 'Lost ambiguous owner'
 $r=Observe-FiniteTool -Owner $owner -ObservationMilliseconds 10
 Check (-not$r.ExitedConfirmed-and$r.ExitCode-eq$null-and$r.NeedsObservation) 'Invented terminal after ambiguous start'
 Check ($script:fake.StartCalls-eq1-and-not$script:fake.Disposed) 'Relaunched/disposed unknown owner'
 Reject {Close-FiniteToolOwner $owner}
 Check (@($owner.Files|Where-Object{$_.SafeFileHandle.IsClosed}).Count-eq0) 'Unknown dispatch silently closed files'
 # This factory is a no-child mock; only the test may release its file handles.
 foreach($file in $owner.Files){$file.Dispose()}
}
Case 'identity-capture-failure-retains-process-before-observation' {
 $script:fake=FakeProcess $false $true;$owner=Start-FiniteTool -Executable $exe -ExpectedSha256 $hash -OutputDirectory (RunDirectory) -ProcessFactory {$script:fake}
 Check ($owner.Process-eq$script:fake-and-not$owner.IdentityConfirmed) 'Lost unpinned process'
 $r=Observe-FiniteTool -Owner $owner -ObservationMilliseconds 10
 Check ($r.NeedsObservation-and-not$r.Success-and$script:fake.StartCalls-eq1) 'False identity success'
 Reject {Close-FiniteToolOwner $owner}
 Check (@($owner.Files|Where-Object{$_.SafeFileHandle.IsClosed}).Count-eq0-and-not$script:fake.Disposed) 'Unpinned owner silently disposed'
 foreach($file in $owner.Files){$file.Dispose()}
}
Case 'factory-failure-before-dispatch-can-release-only-unlaunched-resources' {
 $owner=Start-FiniteTool -Executable $exe -ExpectedSha256 $hash -OutputDirectory (RunDirectory) -ProcessFactory {throw 'factory failure'}
 Check (-not$owner.LaunchAttempted-and$null-eq$owner.Process) 'Invented dispatch'
 Close-FiniteToolOwner $owner;Check $owner.Closed 'Unlaunched files retained unnecessarily'
}
Case 'real-unicode-both-streams-and-exit-zero' {
 $owner=StartChild '[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);[Console]::WriteLine(([string][char]0x4e2d)+[char]0x6587);[Console]::Error.WriteLine(([string][char]0x9519)+[char]0x8bef);exit 0'
 Check ($owner.Process-is[Diagnostics.Process]-and$owner.IdentityConfirmed-and$owner.ProcessId-gt0-and$owner.CreationUtcTicks-gt0-and$owner.Handle-ne[IntPtr]::Zero) 'Exact identity missing'
 $r=Observe-FiniteTool $owner
 Check ($r.Success-and$r.ExitedConfirmed-and$r.ExitCode-eq0-and$r.OutputStable) 'Real child did not complete'
 Close-FiniteToolOwner $owner;Check $owner.Closed 'Terminal owner not closed'
 $utf8=[Text.UTF8Encoding]::new($false,$true)
 Check ([IO.File]::ReadAllText($r.StdoutPath,$utf8)-ceq(([string][char]0x4e2d)+[char]0x6587+"`r`n")) 'Unicode stdout changed'
 Check ([IO.File]::ReadAllText($r.StderrPath,$utf8)-ceq(([string][char]0x9519)+[char]0x8bef+"`r`n")) 'Unicode stderr changed'
}
Case 'real-nonzero-exit-is-not-success' {
 $owner=StartChild '[Console]::Write("retained");exit 3';$r=Observe-FiniteTool $owner
 Check ($r.ExitedConfirmed-and$r.ExitCode-eq3-and-not$r.Success-and$r.OutputStable) 'Nonzero exit misreported'
 Close-FiniteToolOwner $owner
}
Case 'completed-in-time-reobservation-does-not-invent-a-later-timeout' {
 $owner=StartChild 'exit 0';$first=Observe-FiniteTool $owner
 Check $first.Success 'Initial completion failed'
 $owner.Clock=[pscustomobject]@{ElapsedMilliseconds=$owner.DeadlineMilliseconds+1}
 $again=Observe-FiniteTool $owner
 Check ($again.Success-and-not$again.TimedOut) 'Wall time after completed receipt invented timeout'
 Close-FiniteToolOwner $owner
}
Case 'first-terminal-observation-at-or-after-deadline-is-sticky-refusal' {
 foreach($elapsed in @(50,51)){
  $owner=StartChild 'exit 0';Check ($owner.Process.WaitForExit(5000)) 'Child failed to exit'
  $owner.DeadlineMilliseconds=50;$owner.Clock=[pscustomobject]@{ElapsedMilliseconds=[long]$elapsed}
  $r=Observe-FiniteTool $owner
  Check ($r.ExitedConfirmed-and$r.TimedOut-and-not$r.Success) 'Late observed exit accepted'
  $later=Observe-FiniteTool -Owner $owner -ObservationMilliseconds 5000
  Check ($later.OutputStable-and$later.TimedOut-and-not$later.Success) 'Late terminal erased refusal'
  Close-FiniteToolOwner $owner
 }
}
Case 'real-CRT-argv-roundtrip-empty-spaces-quotes-and-trailing-slash' {
 $code='[Console]::Write((ConvertTo-Json -Compress -InputObject ([Environment]::GetCommandLineArgs())));exit 0 #'
 $values=@('','two words','a"b','C:\tail\')
 $owner=Start-FiniteTool -Executable $exe -ExpectedSha256 $hash -OutputDirectory (RunDirectory) -ArgumentList ((ChildArguments $code)+$values)
 $r=Observe-FiniteTool $owner;Check $r.Success 'CRT child failed';Close-FiniteToolOwner $owner
 $argv=ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($r.StdoutPath))
 for($i=0;$i-lt$values.Count;$i++){Check ($argv[$argv.Count-$values.Count+$i]-ceq$values[$i]) 'CRT argv changed'}
}
Case 'real-joint-log-cap-is-sticky-and-does-not-block-child-drain' {
 $owner=StartChild '$x="x"*4096;for($i=0;$i-lt32;$i++){[Console]::Out.Write($x);[Console]::Error.Write($x)};exit 0' 15 1024
 $r=Observe-FiniteTool $owner
 Check ($r.ExitedConfirmed-and$r.LogLimitExceeded-and-not$r.Success-and-not$r.OutputStable) 'Cap failure hidden'
 $bytes=(Get-Item -LiteralPath $r.StdoutPath).Length+(Get-Item -LiteralPath $r.StderrPath).Length
 Check ($bytes-le1024) 'Joint output cap exceeded'
 Close-FiniteToolOwner $owner
}
Case 'real-deadline-retains-owner-no-kill-and-later-natural-exit-stays-failed' {
 $owner=StartChild 'Start-Sleep -Milliseconds 1700;[Console]::Write("natural exit");exit 0' 0.05
 $r=Observe-FiniteTool $owner
 Check ($r.TimedOut-and-not$r.ExitedConfirmed-and$r.ExitCode-eq$null-and$r.NeedsObservation) 'Timeout invented terminal'
 Check ($r.Owner-eq$owner-and-not$owner.Process.HasExited-and-not$owner.Closed) 'Timeout killed/disposed owner'
 Reject {Close-FiniteToolOwner $owner}
 $later=Observe-FiniteTool -Owner $owner -ObservationMilliseconds 10000
 Check ($later.ExitedConfirmed-and$later.ExitCode-eq0-and$later.TimedOut-and-not$later.Success) 'Late exit erased timeout'
 Close-FiniteToolOwner $owner
}
Case 'flush-observation-is-bounded-and-retains-pending-exact-files' {
 $owner=StartChild 'exit 0'
 $null=$owner.Process.WaitForExit(5000)
 $proxies=@()
 foreach($file in $owner.Files){
  $proxy=[pscustomobject]@{File=$file;FlushCalls=0;Disposed=$false}
  $proxy|Add-Member ScriptMethod FlushAsync {$this.FlushCalls++;return [Threading.Tasks.Task]::Delay(500)}
  $proxy|Add-Member ScriptMethod Dispose {$this.File.Dispose();$this.Disposed=$true}
  $proxies+=,$proxy
 }
 $owner.Files=$proxies
 $owner.DeadlineMilliseconds=$owner.Clock.ElapsedMilliseconds+50
 $watch=[Diagnostics.Stopwatch]::StartNew();$r=Observe-FiniteTool $owner;$watch.Stop()
 Check ($watch.ElapsedMilliseconds-lt400) 'Flush exceeded observation bound'
 Check ($r.ExitedConfirmed-and$r.TimedOut-and-not$r.OutputStable-and$r.NeedsObservation) 'Pending flush falsely stable'
 Check ($proxies[0].FlushCalls-eq1-and-not$proxies[0].Disposed) 'Flush owner lost/repeated'
 Reject {Close-FiniteToolOwner $owner}
 $later=Observe-FiniteTool -Owner $owner -ObservationMilliseconds 5000
 Check ($later.OutputStable-and$later.TimedOut-and-not$later.Success) 'Flush recovery erased deadline'
 Close-FiniteToolOwner $owner
 Check ($proxies[0].Disposed-and$proxies[0].FlushCalls-eq1) 'Exact flush owner not closed'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed)
if($failed){exit 1}
