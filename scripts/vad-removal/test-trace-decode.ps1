#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$no=$false;try{&$Action|Out-Null}catch{$no=$true};Check $no 'Expected refusal.'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
$library=Join-Path $PSScriptRoot 'trace-decode.ps1'
Case 'closed-etl-decode-definitions-exist-without-tool-dispatch' {
 Check (Test-Path -LiteralPath $library) 'Closed ETL decoder not implemented.'
 . $library
 Check ($null-ne(Get-Command New-RemovalTraceDecode -ErrorAction SilentlyContinue)) 'Decode API missing.'
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 TOOL_CALLS=0 ETW_CALLS=0';exit 1}
. $library
. (Join-Path $PSScriptRoot 'test-trace-decode-fixtures.ps1')
Case 'decode-rechecks-exact-requested-mode-without-masking-old-or-additional-flags' {
 foreach($mode in @([uint32]1,[uint32]0x00800001,[uint32]0x100,[uint32]0x00400101,[uint32]0x00c00001,[uint32]0x20400001)){
  $f=New-Fixture;$f.Session.Readback.Initial.LogFileMode=$mode
  Reject {New-Decode $f (New-ToolBoundary)}
  Check ($f.Calls.Count-eq0) 'Changed initial mode reached decode tools.'
 }
}
Case 'actual-managed-release-manifest-sourcehead-and-unique-exe-row-bind-relocated-probe' {
 $f=New-Fixture;$build=Set-ReleaseManifest $f
 $d=New-Decode $f (New-ToolBoundary);Invoke-RemovalTraceDecode $d|Out-Null
 Check ($d.Completed-and-not$d.Failed-and$d.AssociationAvailable-and
  $d.ManagedProvenance.SourceRevision-ceq$build.SourceHead) 'Actual SourceHead/Files release manifest was not consumed.'
 $saved=[IO.File]::ReadAllText($d.ReceiptPath,[Text.Encoding]::UTF8)|ConvertFrom-Json
 Check ($saved.InputPins.Probe.BuildManifest.Sha256-ceq$f.Pins.Probe.BuildManifest.Sha256-and
  $saved.ManagedProvenance.SourceRevision-ceq$build.SourceHead) 'Actual manifest file pin/source association lost.'
}
Case 'release-manifest-missing-ambiguous-or-mismatched-exe-source-refuses-without-legacy-fallback' {
 foreach($mode in @('source','missing-source','missing-files','scalar-files','duplicate','wrong-name','hash','bytes','failed-build','unmatched-source')){
  $f=New-Fixture;$build=Set-ReleaseManifest $f
  switch($mode){
   'source'{$build.SourceHead='e'*40}
   'missing-source'{$build.Remove('SourceHead')}
   'missing-files'{$build.Remove('Files')}
   'scalar-files'{$build.Files=$build.Files[0]}
   'duplicate'{$build.Files+=,$build.Files[0]}
   'wrong-name'{$build.Files[0].Name='other.exe'}
   'hash'{$build.Files[0].Sha256='f'*64}
   'bytes'{$build.Files[0].Bytes++}
   'failed-build'{$build.BuildExit=1}
   'unmatched-source'{$build.RustSourcesMatchSourceHead=$false}
  }
  # A malformed current schema must not fall through to a matching old shape.
  $build.Source=$f.Pins.Probe.SourceRevision;$build.Sha256=$f.Pins.Probe.File.Sha256
  $build.Bytes=$f.Pins.Probe.File.Bytes;$build.Path=$f.Pins.Probe.File.Path
  Write-Text $f.Pins.Probe.BuildManifest.Path ($build|ConvertTo-Json -Depth 8)
  $f.Pins.Probe.BuildManifest=Pin $f.Pins.Probe.BuildManifest.Path
  Reject {New-Decode $f (New-ToolBoundary)}
  Check ($f.Calls.Count-eq0) ('Invalid release manifest reached tools: '+$mode)
 }
}
Case 'actual-retained-stop-and-real-finite-children-feed-existing-reader-without-native-claims' {
 $f=New-Fixture;$d=New-Decode $f (New-ToolBoundary);Invoke-RemovalTraceDecode $d|Out-Null
 Check ($d.Completed-and-not$d.Failed-and$d.AssociationAvailable-and$d.ToolOwners.Count-eq2) ('Decode did not finish: '+($d.Errors-join';'))
 Check ([Object]::ReferenceEquals($d.StopReceipt,$f.Session.NativeOwner.StopReceipt)-and$d.RawTraceLoss.EventsLost-eq3-and$d.RawTraceLossKnown) 'Original STOP/raw loss provenance lost.'
 Check ($f.Session.Readback.Initial.LogFileMode-eq[UInt32]0x00400001-and
  $d.StopReceipt.Snapshot.LogFileMode-eq[UInt32]0x00400001-and
  $f.Session.Readback.Final.LogFileMode-eq[UInt32]0x00400001) 'Explicit raw trace mode lost before closed decode.'
 Check (-not$d.Qualified-and-not$d.NativePassed-and-not$d.Analysis.qualified-and-not$d.Analysis.direct_removed_observed-and$d.Analysis.raw_trace_loss-ceq'unknown') 'Diagnostic association promoted cleanup/native/loss.'
 Check ($d.OutputPins.Count-eq7-and(Test-Path -LiteralPath $d.ReceiptPath)) 'Output receipt/pins missing.'
 Reject {Invoke-RemovalTraceDecode $d};Check ($f.Calls.Count-eq2) 'Repeated decode launched again.'
}
Case 'moredata-stop-can-retain-diagnostic-association-but-raw-loss-stays-unknown' {
 $f=New-Fixture 234;$d=New-Decode $f (New-ToolBoundary);Invoke-RemovalTraceDecode $d|Out-Null
 Check ($d.Completed-and$d.AssociationAvailable-and-not$d.RawTraceLossKnown-and$d.RawTraceLoss-ceq'unknown'-and-not$d.Qualified) 'MORE_DATA became zero loss or lost bounded diagnostic.'
}
Case 'pending-trace-and-detached-stop-receipts-refuse-before-tools' {
 $f=New-Fixture 2402;Reject {New-Decode $f (New-ToolBoundary)};Check ($f.Calls.Count-eq0) 'Pending trace decoded.'
 $f=New-Fixture;$f.Session.StoppedReceipt=[pscustomobject]@{Status=[UInt32]0};Reject {New-Decode $f (New-ToolBoundary)}
 Check ($f.Calls.Count-eq0) 'Detached receipt decoded.'
}
Case 'wrong-etl-report-symbol-provenance-or-input-pin-refuses-before-tools' {
 foreach($mode in @('etl','report','revision','pdb','tool-length','tmf-extra')){
  $f=New-Fixture
  switch($mode){
   'etl'{$f.Pins.Etl.Path=$f.Pins.ProbeReport.Path}
   'report'{$f.Pins.ProbeReport.Sha256='f'*64}
   'revision'{$f.Pins.Symbols.DriverRevision='f'*40}
   'pdb'{$f.Pins.Symbols.PdbSha256='e'*64}
   'tool-length'{$f.Pins.Tracefmt.Bytes++}
   'tmf-extra'{Write-Text (Join-Path $f.Pins.Symbols.Directory 'unreviewed.tmf') 'not pinned'}
  }
  Reject {New-Decode $f (New-ToolBoundary)};Check ($f.Calls.Count-eq0) ('Invalid provenance reached tools: '+$mode)
 }
}
Case 'default-tool-launch-is-unconditionally-refused-and-partial-test-boundary-is-invalid' {
 $f=New-Fixture;Reject {New-Decode $f @{Start={}}}
 $d=New-Decode $f $null;Reject {Invoke-RemovalTraceDecode $d}
 Check (-not$d.Attempted-and$d.ToolOwners.Count-eq0-and-not(Test-Path -LiteralPath $d.OutputDirectory)) 'Guard was after mutation/launch.'
}
Case 'nonzero-null-timeout-mismatched-owner-and-log-overflow-do-not-authorize-reader' {
 foreach($mode in @('trace-fail','null-exit','timeout','wrong-owner','log-cap')){
  $f=New-Fixture;$f.Mode=$mode;$d=New-Decode $f (New-ToolBoundary);Invoke-RemovalTraceDecode $d|Out-Null
  Check ($d.Failed-and-not$d.Completed-and-not$d.AssociationAvailable-and$f.Calls.Count-eq1-and$d.ToolOwners.Count-eq1) ('Invalid tool result reached reader: '+$mode)
 }
}
Case 'etl-mutation-and-invalid-or-missing-decode-text-stop-before-reader' {
 foreach($mode in @('mutate-etl','bad-utf8','missing-summary')){
  $f=New-Fixture;$f.Mode=$mode;$d=New-Decode $f (New-ToolBoundary);Invoke-RemovalTraceDecode $d|Out-Null
  Check ($d.Failed-and-not$d.Completed-and$f.Calls.Count-eq1) ('Invalid decoded inputs reached reader: '+$mode)
 }
}
Case 'failed-dispatch-retains-original-process-owner-and-never-retries' {
 $f=New-Fixture;$f.Mode='throw-after-launch';$d=New-Decode $f (New-ToolBoundary);Invoke-RemovalTraceDecode $d|Out-Null
 Check ($d.Failed-and$d.ToolOwners.Count-eq1-and[Object]::ReferenceEquals($d.ToolOwners[0],$f.Owners[0])) 'Dispatch exception discarded exact owner.'
 Reject {Invoke-RemovalTraceDecode $d};Check ($f.Calls.Count-eq1) 'Failed dispatch retried.'
}
Case 'offline-reader-refusal-is-retained-not-relabeled-as-association' {
 $f=New-Fixture;$f.Mode='reader-refusal';$d=New-Decode $f (New-ToolBoundary);Invoke-RemovalTraceDecode $d|Out-Null
 Check ($d.Failed-and-not$d.Completed-and-not$d.AssociationAvailable-and$null-ne$d.Analysis-and$d.Analysis.reasons.Count-gt0) 'Offline refusal discarded or promoted.'
 Check ($f.Calls.Count-eq2-and-not$d.NativePassed-and-not$d.Qualified) 'Reader refusal changed native meaning.'
}
Case 'both-original-finite-child-output-streams-are-hash-associated-in-receipt' {
 $f=New-Fixture;$d=New-Decode $f (New-ToolBoundary);Invoke-RemovalTraceDecode $d|Out-Null
 Check ($d.Completed-and$d.OutputPins.Count-eq7) 'Not all trace/sum/analysis and four original tool logs were pinned.'
 foreach($owner in $d.ToolOwners){
  foreach($path in @($owner.StdoutPath,$owner.StderrPath)){
   $pins=@($d.OutputPins|Where-Object{$_.Path-ceq$path})
   Check ($pins.Count-eq1-and$pins[0].Bytes-eq(Get-Item $path).Length-and$pins[0].Sha256-ceq(Get-FileHash $path -Algorithm SHA256).Hash) 'Original finite child log pin missing/wrong.'
  }
 }
 $saved=[IO.File]::ReadAllText($d.ReceiptPath,[Text.Encoding]::UTF8)|ConvertFrom-Json
 Check ($saved.OutputPins.Count-eq7-and$saved.InputPins.Probe.SourceRevision-ceq('d'*40)-and
  $saved.RawTraceLoss.EventsLost-eq3-and-not$saved.Qualified-and-not$saved.NativePassed) 'Persisted receipt lost pins/provenance/raw loss.'
 Check ($saved.ManagedProvenance.ProbePid-eq9876-and$saved.ManagedProvenance.WorkerPid-eq5678-and
  $saved.ManagedProvenance.RunDirectory-ceq$f.ManagedSession.RunDirectory-and
  $saved.ManagedProvenance.ReportPath-ceq$f.Pins.ProbeReport.Path-and$saved.ManagedProvenance.ProbeExitCode-eq1) 'Receipt lost original managed failure/process/report association.'
}
Case 'receipt-distinguishes-injected-real-child-image-from-requested-tracefmt' {
 $f=New-Fixture;$d=New-Decode $f (New-ToolBoundary);Invoke-RemovalTraceDecode $d|Out-Null
 $saved=[IO.File]::ReadAllText($d.ReceiptPath,[Text.Encoding]::UTF8)|ConvertFrom-Json
 Check ($saved.ExecutionMode-ceq'InjectedSubprocessBoundary'-and$saved.Steps[0].Executable-ceq$f.Pins.Tracefmt.Path-and
  $saved.Steps[0].ActualOwner.Executable-ceq$script:ps-and$saved.Steps[0].ActualOwner.ExecutableSha256-ceq$script:psPin.Sha256-and
  $saved.Steps[0].ActualOwner.ProcessId-gt0-and$saved.Steps[0].ActualOwner.CreationUtcTicks-gt0-and
  $saved.Steps[0].ActualOwner.Closed-and$saved.Steps[1].ActualOwner.Executable-ceq$script:python) 'Receipt relabeled harmless child as actual tracefmt or lost exact owner.'
}
Case 'report-or-decoded-input-mutation-during-association-cannot-complete' {
 foreach($mode in @('mutate-report','mutate-trace')){
  $f=New-Fixture;$f.Mode=$mode;$d=New-Decode $f (New-ToolBoundary);Invoke-RemovalTraceDecode $d|Out-Null
  Check ($d.Failed-and-not$d.Completed-and-not$d.AssociationAvailable-and$f.Calls.Count-eq2) ('Mutated association input accepted: '+$mode)
 }
}
Case 'managed-report-cannot-pair-a-different-run-probe-or-stale-terminal-boolean' {
 foreach($mode in @('missing','trace','readback','report','probe','pid','live','source')){
  $f=New-Fixture
  switch($mode){
   'missing'{$f.ManagedSession=$null}
   'trace'{$f.ManagedSession.TraceSession=[pscustomobject]@{}}
   'readback'{$f.ManagedSession.TraceReadback=[pscustomobject]@{}}
   'report'{$foreign=Join-Path $f.Root 'foreign-report.json';[IO.File]::Copy($f.Pins.ProbeReport.Path,$foreign);$f.Pins.ProbeReport=Pin $foreign}
   'probe'{$f.ManagedSession.Config.ExpectedSha256='f'*64}
   'pid'{$f.ManagedSession.LastResult.ProbePid++}
   'live'{$f.ProbeProcess.HasExited=$false;$f.ManagedSession.Terminal=@{Terminal=$true;ProbeOwnershipVerified=$true}}
   'source'{$f.Pins.Probe.SourceRevision='e'*40}
  }
  Reject {New-Decode $f (New-ToolBoundary)};Check ($f.Calls.Count-eq0) ('Detached managed provenance admitted: '+$mode)
 }
}
Case 'managed-provenance-is-rechecked-at-use-not-only-at-construction' {
 $f=New-Fixture;$d=New-Decode $f (New-ToolBoundary);$f.ManagedSession.LastResult.ProbePid++
 Invoke-RemovalTraceDecode $d|Out-Null
 Check ($d.Failed-and-not$d.Completed-and$f.Calls.Count-eq0-and-not(Test-Path -LiteralPath $d.OutputDirectory)) 'Changed original run reached decoder after construction.'
}
foreach($o in $script:realOwners){
 if(-not$o.Closed){$r=Observe-FiniteTool $o -ObservationMilliseconds 15000;Check ($r.ExitedConfirmed-and-not$r.NeedsObservation) 'Harmless child unresolved: retained, not killed.';Close-FiniteToolOwner $o}
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ETW_CALLS=0 TRACEFMT_CALLS=0 CHILDREN=harmless-powershell-and-existing-offline-python')
if($failed){exit 1}
