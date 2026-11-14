#requires -Version 5.1
param([string]$Library)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){
    throw 'Host-only collector tests must not run on the actual test target.'
}
if(-not $Library){$Library=Join-Path $PSScriptRoot 'collector-worker.ps1'}
if(Test-Path -LiteralPath $Library){. $Library}
$checks=New-Object Collections.Generic.List[object]
$root=Join-Path $PSScriptRoot ('logs\migration-collector-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$finite=(Join-Path $PSScriptRoot 'finite-tool.ps1')
$finiteHash=(Get-FileHash -LiteralPath $finite -Algorithm SHA256).Hash
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
function Reject([scriptblock]$Action){$rejected=$false;try{&$Action|Out-Null}catch{$rejected=$true};Check $rejected 'Expected refusal'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
function Fixture([string]$Body){
 $path=Join-Path $root ([Guid]::NewGuid().ToString('N')+'.ps1')
 [IO.File]::WriteAllText($path,$Body,[Text.UTF8Encoding]::new($false));return $path
}
function StartFixture([string]$Body,[double]$Seconds=15){
 $collector=Fixture $Body
 Start-RemovalCollectorWorker -FiniteToolPath $finite -FiniteToolSha256 $finiteHash -CollectorPath $collector `
  -CollectorSha256 (Get-FileHash -LiteralPath $collector -Algorithm SHA256).Hash `
  -EvidenceDirectory (Join-Path $root ([Guid]::NewGuid().ToString('N'))) -TimeoutSeconds $Seconds
}
Case 'pinned-fixture-collects-once-preserves-real-dictionary-and-closes' {
 $o=StartFixture 'function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){@{schema_version=1;session=0;NativeTestStarted=$false;Qualified=$false;ExternalOwners=@();Value=42}}'
 $r=Observe-RemovalCollectorWorker $o
 Check ($r.CollectionComplete-and$r.Completed-and-not$r.TimedOut-and$r.Snapshot.Value-eq42-and$r.Snapshot.session-eq0) 'Collection or dictionary values lost'
 Check (-not$r.NativeApproved-and-not$r.Qualified-and$r.InnerOwnersClosed) 'False native approval/unclosed owner'
 Close-RemovalCollectorWorker $o;Check $o.Closed 'Completed worker not closed'
}
Case 'original-read-tool-pins-cross-the-actual-runspace-as-detached-values' {
 $collector=Fixture 'function Read-RemovalPreflight {param($EvidenceDirectory,$ToolOwners,$ReadToolPins)@{schema_version=1;NativeTestStarted=$false;Qualified=$false;BcdPath=$ReadToolPins.BcdEdit.Path;BcdHash=$ReadToolPins.BcdEdit.Sha256}}'
 $pins=@{BcdEdit=@{Path='C:\Windows\System32\bcdedit.exe';Bytes=123;Sha256=('A'*64)}}
 $original=ConvertTo-Json -InputObject $pins -Depth 4 -Compress
 $o=Start-RemovalCollectorWorker -FiniteToolPath $finite -FiniteToolSha256 $finiteHash -CollectorPath $collector `
  -CollectorSha256 (Get-FileHash -LiteralPath $collector).Hash -ReadToolPinsJson $original `
  -EvidenceDirectory (Join-Path $root ([Guid]::NewGuid().ToString('N')))
 $pins.BcdEdit.Sha256='B'*64
 $r=Observe-RemovalCollectorWorker $o
 Check ($r.CollectionComplete-and$r.Snapshot.BcdPath-ceq'C:\Windows\System32\bcdedit.exe'-and$r.Snapshot.BcdHash-ceq('A'*64)) 'Original read-tool pins missing or aliased after dispatch.'
 Close-RemovalCollectorWorker $o
}
Case 'bad-hash-refuses-before-worker-or-evidence-directory' {
 $collector=Fixture 'throw "must never run"';$directory=Join-Path $root ([Guid]::NewGuid().ToString('N'))
 Reject {Start-RemovalCollectorWorker -FiniteToolPath $finite -FiniteToolSha256 $finiteHash -CollectorPath $collector -CollectorSha256 ('0'*64) -EvidenceDirectory $directory}
 Check (-not(Test-Path -LiteralPath $directory)) 'Bad pin reached worker'
}
Case 'worker-throw-is-failed-not-empty-success' {
 $o=StartFixture 'function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){throw "expected read failure"}'
 $r=Observe-RemovalCollectorWorker $o
 Check ($r.Completed-and-not$r.CollectionComplete-and$r.Error-like'*expected read failure*') 'Collector failure hidden'
 Close-RemovalCollectorWorker $o
}
Case 'deadline-retains-same-invocation-and-late-completion-cannot-pass' {
 $o=StartFixture 'function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){Start-Sleep -Milliseconds 700;@{schema_version=1;NativeTestStarted=$false;Qualified=$false}}' 0.05
 $handle=$o.Invocation;$r=Observe-RemovalCollectorWorker $o
 Check ($r.TimedOut-and-not$r.Completed-and-not$r.CollectionComplete-and$o.Invocation-eq$handle-and-not$o.Closed) 'Timeout lost original live invocation'
 Reject {Close-RemovalCollectorWorker $o}
 $later=Observe-RemovalCollectorWorker -Owner $o -ObservationMilliseconds 5000
 Check ($later.Completed-and$later.TimedOut-and-not$later.CollectionComplete-and$o.Invocation-eq$handle) 'Late completion erased failure or relaunched'
 Close-RemovalCollectorWorker $o
}
Case 'successful-first-observation-is-not-invalidated-by-later-walltime' {
 $o=StartFixture 'function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){@{schema_version=1;NativeTestStarted=$false;Qualified=$false}}'
 $r=Observe-RemovalCollectorWorker $o;Check $r.CollectionComplete 'Initial collection failed'
 $o.Clock=[pscustomobject]@{ElapsedMilliseconds=$o.DeadlineMilliseconds+1}
 $r=Observe-RemovalCollectorWorker $o;Check ($r.CollectionComplete-and-not$r.TimedOut) 'Reobservation invented timeout'
 Close-RemovalCollectorWorker $o
}
Case 'unresolved-inner-owner-retained-after-worker-failure' {
 $o=StartFixture 'function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){$ToolOwners.Add([pscustomobject]@{Closed=$false;Marker="same exact owner"});throw "inner pending"}'
 $r=Observe-RemovalCollectorWorker $o
 Check ($r.Completed-and-not$r.InnerOwnersClosed-and$r.NeedsObservation-and$o.ToolOwners.Count-eq1-and$o.ToolOwners[0].Marker-ceq'same exact owner') 'Unresolved internal owner lost'
 Reject {Close-RemovalCollectorWorker $o}
 $o.ToolOwners[0].Closed=$true;Close-RemovalCollectorWorker $o
}
Case 'actual-inner-child-timeout-retains-live-process-across-runspace-boundary' {
 $body=@'
function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){
 $exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
 $owner=Start-FiniteTool -Executable $exe -ExpectedSha256 (Get-FileHash -LiteralPath $exe).Hash -OutputDirectory $EvidenceDirectory -TimeoutSeconds 0.02 `
  -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-Command','Start-Sleep -Milliseconds 900;[Console]::Write("late natural exit");exit 0')
 $ToolOwners.Add($owner)
 $r=Observe-FiniteTool $owner
 if(-not$r.Success){throw 'Expected exact inner timeout'}
 throw 'Unexpected inner acceptance'
}
'@
 $o=StartFixture $body;$r=Observe-RemovalCollectorWorker $o
 Check ($r.Completed-and-not$r.CollectionComplete-and-not$r.InnerOwnersClosed-and$o.ToolOwners.Count-eq1) 'Actual child owner lost after runspace failure'
 $child=$o.ToolOwners[0]
 Check ($child.Process-is[Diagnostics.Process]-and$child.IdentityConfirmed-and$child.TimedOut-and-not$child.Closed) 'Original process identity/timeout lost'
 Reject {Close-RemovalCollectorWorker $o}
 . $finite
 $later=Observe-FiniteTool -Owner $child -ObservationMilliseconds 10000
 Check ($later.ExitedConfirmed-and$later.OutputStable-and$later.TimedOut-and-not$later.Success) 'Natural inner exit erased timeout'
 Close-FiniteToolOwner $child
 Close-RemovalCollectorWorker $o
 Check ($child.Closed-and$o.Closed) 'Exact nested lifecycle not closed'
}
Case 'malformed-or-multiple-worker-output-refused' {
 foreach($body in @('return $null','"unexpected output";@{schema_version=1;NativeTestStarted=$false;Qualified=$false}','@{schema_version=1;NativeTestStarted=$true;Qualified=$true}')){
  $o=StartFixture ('function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){'+$body+'}')
  $r=Observe-RemovalCollectorWorker $o;Check ($r.Completed-and-not$r.CollectionComplete) 'Malformed/native result accepted'
  Close-RemovalCollectorWorker $o
 }
}
Case 'multiple-output-stops-at-first-invalid-value-without-large-array' {
 $o=StartFixture 'function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){$m=[pscustomobject]@{Closed=$true;Produced=0};$ToolOwners.Add($m);for($i=0;$i-lt1000;$i++){$m.Produced++;"unexpected output"}}'
 $r=Observe-RemovalCollectorWorker $o
 Check ($r.Completed-and-not$r.TimedOut-and-not$r.CollectionComplete-and$o.ToolOwners[0].Produced-eq1) 'Unexpected stream was collected without an early bound'
 Close-RemovalCollectorWorker $o
}
Case 'warning-stream-cannot-be-silently-accepted-with-snapshot' {
 $o=StartFixture 'function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){Write-Warning "unexpected warning";@{schema_version=1;NativeTestStarted=$false;Qualified=$false}}'
 $r=Observe-RemovalCollectorWorker $o
 Check ($r.Completed-and-not$r.CollectionComplete-and$r.Error-like'*unexpected warning*') 'Warning ignored as success'
 Close-RemovalCollectorWorker $o
}
Case 'nonterminating-error-stream-prevents-success-snapshot' {
 $o=StartFixture 'function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){Write-Error "explicit read error" -ErrorAction Continue;@{schema_version=1;NativeTestStarted=$false;Qualified=$false}}'
 $r=Observe-RemovalCollectorWorker $o
 Check ($r.Completed-and-not$r.CollectionComplete-and$r.Error) 'Nonterminating error was accepted as success'
 Close-RemovalCollectorWorker $o
}
Case 'oversize-single-snapshot-refused-before-acceptance' {
 $o=StartFixture 'function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){@{schema_version=1;NativeTestStarted=$false;Qualified=$false;UnexpectedBlob=("x"*1048576)}}'
 $r=Observe-RemovalCollectorWorker $o
 Check ($r.Completed-and-not$r.CollectionComplete-and$r.Error) 'Oversize single snapshot accepted'
 Close-RemovalCollectorWorker $o
}
Case 'explicit-diagnostic-stream-flood-is-drained-not-retained' {
 $o=StartFixture 'function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){for($i=0;$i-lt100;$i++){Write-Error "bounded diagnostic" -ErrorAction Continue};@{schema_version=1;NativeTestStarted=$false;Qualified=$false}}'
 $r=Observe-RemovalCollectorWorker $o
 Check ($r.Completed-and-not$r.CollectionComplete-and$o.PowerShell.Streams.Error.Count-eq0-and$o.Diagnostics.EmittedCount-eq100) 'Error stream retained or forgotten'
 Close-RemovalCollectorWorker $o
}
Case 'top-level-load-output-refused-not-dropped-at-EndInvoke' {
 $o=StartFixture '"unexpected top-level output";function Read-RemovalPreflight($EvidenceDirectory,$ToolOwners){@{schema_version=1;NativeTestStarted=$false;Qualified=$false}}'
 $r=Observe-RemovalCollectorWorker $o
 Check ($r.Completed-and-not$r.CollectionComplete-and$r.Error) 'Top-level load output bypassed collector contract'
 Close-RemovalCollectorWorker $o
}
Case 'host-actual-collector-refuses-before-native-read' {
 $collector=(Join-Path $PSScriptRoot 'readonly-preflight.ps1')
 $o=Start-RemovalCollectorWorker -FiniteToolPath $finite -FiniteToolSha256 $finiteHash -CollectorPath $collector `
  -CollectorSha256 (Get-FileHash -LiteralPath $collector).Hash -EvidenceDirectory (Join-Path $root ([Guid]::NewGuid().ToString('N')))
 $r=Observe-RemovalCollectorWorker $o
 Check ($r.Completed-and-not$r.CollectionComplete-and$r.Error-like'*Wrong target identity*') 'Development host reached actual collection'
 Check (-not(Test-Path -LiteralPath $o.EvidenceDirectory)) 'Development host created target evidence'
 Close-RemovalCollectorWorker $o
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed)
if($failed){exit 1}
