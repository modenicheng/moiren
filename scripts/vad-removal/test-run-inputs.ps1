#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$rejected=$false;try{&$Action|Out-Null}catch{$rejected=$true};Check $rejected 'Expected refusal.'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
$library=Join-Path $PSScriptRoot 'run-inputs.ps1'
Case 'run-inputs-production-api-exists-without-native-dispatch' {
 Check (Test-Path -LiteralPath $library) 'Pinned diagnostic input implementation missing.'
 . $library
 foreach($name in @('New-RemovalRunInputs','Assert-RemovalRunInputs','Get-RemovalDiagnosticArguments','New-RemovalDiagnosticDecodePins')){
  Check ($null-ne(Get-Command $name -ErrorAction SilentlyContinue)) ('Missing API '+$name)
 }
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_CALLS=0';exit 1}
. $library
$productionFixed=Get-RemovalRunFixedPins
Case 'fixed-release-driver-schedule-and-finite-predecessor-are-the-reviewed-production-values' {
 Check ($productionFixed.ProbeSource-ceq'26cd706f773f9bfb75e9572fc66594b5f932b918'-and
  $productionFixed.ProbeBytes-eq7752704-and$productionFixed.ProbeSha256-ceq'C8AB9DB102B91CE0AE3724D1856279678019909FB0E9F8F271C77B3453A63266') 'Managed release relabelled.'
 Check ($productionFixed.DriverRevision-ceq'23807f3dc482090c0dd6f0a9775cd91b8d6c1da6'-and
  $productionFixed.PdbSha256-ceq'775C5418D7EFC76433B4E4708F1ADEB01FA5F6600F302BFE03B880F13CB100DE'-and
  $productionFixed.ScheduleSha256-ceq'4FBD145DD4CCD8D847DDE56D87B5AA023748B813E4118AB54FC2D08A6066C78D'-and
  $productionFixed.FiniteSha256-ceq'C3621DC9C881519006E01619D2C65E053C3A1F0FBFC100C0E247E3064D16651B') 'Frozen driver/schedule/helper changed.'
}
# Replace only immutable production content constants for tiny local fixtures.
# Real filesystem hashing, all validators and original trace/managed ownership run.
function Get-RemovalRunFixedPins {return $script:inputFixture.Fixed}
. (Join-Path $PSScriptRoot 'test-trace-decode-fixtures.ps1')
function Save-InputManifest($Fixture){
 Write-Text $Fixture.ManifestPath ($Fixture.Manifest|ConvertTo-Json -Depth 14)
 $Fixture.ManifestHash=(Pin $Fixture.ManifestPath).Sha256
}
function New-InputFixture([switch]$Running,[uint32]$StopStatus=0){
 $f=New-Fixture -LeaveTraceRunning:$Running -StopStatus $StopStatus
 $source=Join-Path $f.Root 'sources';$null=[IO.Directory]::CreateDirectory($source)
 $sources=@()
 foreach($name in @(Get-RemovalRunSourceNames)){
  $path=Join-Path $source $name;Write-Text $path ('# pinned fixture '+$name)
  $p=Pin $path;$p.Name=$name;$sources+=,$p
 }
 $pdb=Join-Path $f.Root 'MoirenVad.pdb';Write-Text $pdb 'synthetic matching PDB'
 $driverFiles=@()
 foreach($name in @('moirenvad.cat','MoirenVad.inf','MoirenVad.sys')){
  $path=Join-Path $f.Root $name;Write-Text $path ('synthetic signed-package file '+$name)
  $p=Pin $path;$p.Name=$name;$driverFiles+=,$p
 }
 $package=@{SchemaVersion=1;DriverVersion='3.1.1.613';Revision='23807f3dc482090c0dd6f0a9775cd91b8d6c1da6'
  PdbSha256=(Pin $pdb).Sha256;Files=@($driverFiles|ForEach-Object{@{Name=$_.Name;Bytes=$_.Bytes;Sha256=$_.Sha256}})}
 Write-Text $f.Pins.DriverPackage.Path ($package|ConvertTo-Json -Depth 8)
 $build=@{SourceHead='26cd706f773f9bfb75e9572fc66594b5f932b918';BuildExit=0;RustSourcesMatchSourceHead=$true
  Files=@(@{Name='e2e_latency_probe.exe';Path='D:\build\e2e_latency_probe.exe'
   Bytes=$f.Pins.Probe.File.Bytes;Sha256=$f.Pins.Probe.File.Sha256},
   @{Name='e2e_latency_probe.pdb';Path='D:\build\e2e_latency_probe.pdb';Bytes=123;Sha256=('a'*64)})}
 Write-Text $f.Pins.Probe.BuildManifest.Path ($build|ConvertTo-Json -Depth 8)
 $schedule=Join-Path $f.Root 'source-schedule.json';Write-Text $schedule '{"schema_version":1,"fixture":"pinned schedule"}'
 $tools=@{Tracefmt=$f.Pins.Tracefmt;Python=$f.Pins.Python;Reader=$f.Pins.Reader}
 foreach($row in @(@{Key='SignTool';Name='signtool.exe'},@{Key='Catalog';Name='w00_probe.exe'},@{Key='BcdEdit';Name='bcdedit.exe'})){
  $path=Join-Path $f.Root $row.Name;Write-Text $path ('synthetic read tool '+$row.Name);$tools[$row.Key]=Pin $path
 }
 $fixed=@{ProbeSource=$build.SourceHead;ProbeBytes=$f.Pins.Probe.File.Bytes;ProbeSha256=$f.Pins.Probe.File.Sha256
  DriverRevision=$package.Revision;DriverManifestSha256=(Pin $f.Pins.DriverPackage.Path).Sha256
  PdbSha256=$package.PdbSha256;DriverFiles=$package.Files;ScheduleSha256=(Pin $schedule).Sha256
  SignToolSha256=$tools.SignTool.Sha256;CatalogSha256=$tools.Catalog.Sha256
  FiniteSha256=(@($sources|Where-Object{$_.Name-ceq'finite-tool.ps1'}))[0].Sha256}
 $f.SourceDirectory=$source;$f.Fixed=$fixed
 $f.Manifest=@{SchemaVersion=1;SourcesRevision=('e'*40);Sources=$sources
  Probe=@{File=$f.Pins.Probe.File;BuildManifest=(Pin $f.Pins.Probe.BuildManifest.Path);SourceRevision=$build.SourceHead}
  Driver=@{PackageManifest=(Pin $f.Pins.DriverPackage.Path);Files=$driverFiles;Pdb=(Pin $pdb)}
  Schedule=(Pin $schedule);Tools=$tools;Symbols=@{Directory=$f.Pins.Symbols.Directory;Files=$f.Pins.Symbols.Files}}
 $f.ManifestPath=Join-Path $f.Root 'run-inputs.json'
 Save-InputManifest $f;$script:inputFixture=$f
 return $f
}
function Open-Inputs($Fixture){New-RemovalRunInputs -ManifestPath $Fixture.ManifestPath -ExpectedSha256 $Fixture.ManifestHash -SourceDirectory $Fixture.SourceDirectory}
function Set-FixedManagedArguments($Fixture,$Owner){
 $arguments=@(Get-RemovalDiagnosticArguments $Owner -ArtifactDirectory (Split-Path $Fixture.Pins.ProbeReport.Path))
 $Fixture.ManagedSession.Config.ProbeArguments=$arguments
 $spec=$Fixture.ManagedSession.ProbeOwner.Spec;$separator=[Array]::IndexOf([string[]]$spec.Arguments,'--')
 $spec.Arguments=@($spec.Arguments[0..$separator])+$arguments
 $spec.CommandLine=(@($spec.Arguments|ForEach-Object{Quote-FiniteToolArgument $_})-join' ')
 $Fixture.ProbeMetadata.CommandLine=('"'+$spec.Executable+'" '+$spec.CommandLine)
}
Case 'manifest-and-all-real-source-files-tools-symbols-and-release-are-read-and-pinned' {
 $f=New-InputFixture;$o=Open-Inputs $f;Assert-RemovalRunInputs $o
 Check ($o.ManifestPin.Sha256-ceq$f.ManifestHash-and$o.Pins.Sources.Count-eq@(Get-RemovalRunSourceNames).Count-and
  $o.Pins.Probe.SourceRevision-ceq$f.Fixed.ProbeSource-and$o.Pins.Driver.Pdb.Sha256-ceq$f.Fixed.PdbSha256) 'Actual pins/source identity lost.'
 Check ($f.Calls.Count-eq0) 'Input validation launched a tool.'
}
Case 'durable-delivery-finalizer-is-required-before-opening-a-run' {
 Check (@(Get-RemovalRunSourceNames)-contains'delivery-finalize.ps1') 'Durable delivery dependency missing from canonical production set.'
 $f=New-InputFixture
 $f.Manifest.Sources=@($f.Manifest.Sources|Where-Object{$_.Name-cne'delivery-finalize.ps1'})
 [IO.File]::Delete((Join-Path $f.SourceDirectory 'delivery-finalize.ps1'))
 Save-InputManifest $f;Reject {Open-Inputs $f}
 Check ($f.Calls.Count-eq0) 'Missing durable delivery dependency dispatched a tool.'
}
Case 'wrong-independent-manifest-hash-refuses-and-never-adopts-a-new-file' {
 $f=New-InputFixture
 Reject {New-RemovalRunInputs -ManifestPath $f.ManifestPath -ExpectedSha256 ('0'*64) -SourceDirectory $f.SourceDirectory}
 Check ($f.Calls.Count-eq0) 'Wrong manifest dispatched.'
}
Case 'fixed-legacy-argv-is-exact-and-artifact-directory-is-the-only-variable' {
 $f=New-InputFixture;$o=Open-Inputs $f;$artifact=Join-Path $f.ManagedSession.RunDirectory 'graph'
 $actual=@(Get-RemovalDiagnosticArguments $o -ArtifactDirectory $artifact)
 $expected=@('live-vad','--stable-endpoint-id','4d6f6972656e53315761766530343330',
  '--name','Moiren S1 Shared Waveform 0430','--vad-source-mode','shared','--source-shared-period-frames','480',
  '--source-schedule',$f.Manifest.Schedule.Path,'--vad-input-capture-frames','480000',
  '--output','{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}',
  '--render-mode','shared','--render-period-frames','336','--render-target-frames','512',
  '--wake-margin-frames','64','--callback-budget-frames','64','--engine-block-frames','64',
  '--artifact-dir',$artifact,'--seconds','8')
 Check (($actual-join'|')-ceq($expected-join'|')) 'Historical schedule/fixture/settings changed.'
 Reject {Get-RemovalDiagnosticArguments $o -ArtifactDirectory ($artifact+'\..\other')}
 Check (-not(Test-Path -LiteralPath $artifact)) 'Argv construction created outputs.'
}
Case 'every-required-production-file-is-mandatory-and-unpinned-production-additions-refuse' {
 foreach($mode in @('missing-worker','missing-controller','missing-inputs','duplicate','outside','unlisted','missing-file')){
  $f=New-InputFixture
  switch($mode){
   'missing-worker'{$f.Manifest.Sources=@($f.Manifest.Sources|Where-Object{$_.Name-cne'worker.ps1'})}
   'missing-controller'{$f.Manifest.Sources=@($f.Manifest.Sources|Where-Object{$_.Name-cne'diagnostic-controller.ps1'})}
   'missing-inputs'{$f.Manifest.Sources=@($f.Manifest.Sources|Where-Object{$_.Name-cne'run-inputs.ps1'})}
   'duplicate'{$f.Manifest.Sources+=,$f.Manifest.Sources[0]}
   'outside'{$f.Manifest.Sources[0].Path=$f.Manifest.Schedule.Path}
   'unlisted'{Write-Text (Join-Path $f.SourceDirectory 'new-production.ps1') '# new unreviewed dependency'}
   'missing-file'{[IO.File]::Delete($f.Manifest.Sources[0].Path)}
  }
  Save-InputManifest $f;Reject {Open-Inputs $f}
 }
}
Case 'actual-extra-production-library-is-supported-only-when-fully-pinned-and-tests-are-excluded' {
 $f=New-InputFixture;Write-Text (Join-Path $f.SourceDirectory 'test-host-only.ps1') '# excluded test'
 $path=Join-Path $f.SourceDirectory 'extra-library.ps1';Write-Text $path '# actual production dependency'
 $pin=Pin $path;$pin.Name='extra-library.ps1';$f.Manifest.Sources+=,$pin
 Save-InputManifest $f;$o=Open-Inputs $f;Check ($o.Pins.Sources.Count-eq(@(Get-RemovalRunSourceNames).Count+1)) 'Production dependency silently skipped.'
}
Case 'source-tool-schedule-symbol-and-manifest-mutation-after-construction-refuses' {
 foreach($mode in @('source','tool','schedule','tmf','manifest')){
  $f=New-InputFixture;$o=Open-Inputs $f
  $path=switch($mode){'source'{$f.Manifest.Sources[0].Path};'tool'{$f.Manifest.Tools.Tracefmt.Path}
   'schedule'{$f.Manifest.Schedule.Path};'tmf'{$f.Manifest.Symbols.Files[0].Path};'manifest'{$f.ManifestPath}}
  [IO.File]::AppendAllText($path,'changed')
  Reject {Assert-RemovalRunInputs $o}
 }
}
Case 'mutable-public-config-or-returned-pins-do-not-replace-original-authority' {
 foreach($mode in @('pin','directory','manifest-pin')){
  $f=New-InputFixture;$o=Open-Inputs $f
  switch($mode){'pin'{$o.Pins.Schedule.Sha256='0'*64};'directory'{$o.SourceDirectory=$f.Root};'manifest-pin'{$o.ManifestPin.Sha256='0'*64}}
  Reject {Assert-RemovalRunInputs $o}
 }
}
Case 'malformed-types-and-noncanonical-source-or-tool-paths-refuse' {
 foreach($mode in @('schema-text','source-ref','sources-scalar','file-size-text','hash-shape','tool-name','ads','traversal','tool-null')){
  $f=New-InputFixture
  switch($mode){
   'schema-text'{$f.Manifest.SchemaVersion='1'}
   'source-ref'{$f.Manifest.SourcesRevision='26cd706'}
   'sources-scalar'{$f.Manifest.Sources=$f.Manifest.Sources[0]}
   'file-size-text'{$f.Manifest.Tools.Tracefmt.Bytes=[string]$f.Manifest.Tools.Tracefmt.Bytes}
   'hash-shape'{$f.Manifest.Tools.Tracefmt.Sha256='bad'}
   'tool-name'{$path=Join-Path $f.Root 'other.exe';Write-Text $path 'wrong named tool';$f.Manifest.Tools.Tracefmt=Pin $path}
   'ads'{$f.Manifest.Tools.Tracefmt.Path+=':stream'}
   'traversal'{$f.Manifest.Tools.Tracefmt.Path=(Join-Path $f.Root ('sources\..\tracefmt.exe'))}
   'tool-null'{$f.Manifest.Tools.Python=$null}
  }
  Save-InputManifest $f;Reject {Open-Inputs $f}
 }
}
Case 'release-sourcehead-unique-exe-build-and-exact-content-are-required' {
 foreach($mode in @('source','duplicate','missing','hash','bytes','build','typed-build')){
  $f=New-InputFixture;$path=$f.Manifest.Probe.BuildManifest.Path
  $b=[IO.File]::ReadAllText($path)|ConvertFrom-Json
  switch($mode){'source'{$b.SourceHead='f'*40};'duplicate'{$b.Files+=,$b.Files[0]}
   'missing'{$b.Files=@($b.Files[1])};'hash'{$b.Files[0].Sha256='0'*64}
   'bytes'{$b.Files[0].Bytes++};'build'{$b.RustSourcesMatchSourceHead=$false};'typed-build'{$b.BuildExit='0'}}
  Write-Text $path ($b|ConvertTo-Json -Depth 8);$f.Manifest.Probe.BuildManifest=Pin $path
  Save-InputManifest $f;Reject {Open-Inputs $f}
 }
}
Case 'fixed-driver-package-file-pdb-and-schedule-cannot-be-relabelled' {
 foreach($mode in @('revision','driver','pdb','schedule')){
  $f=New-InputFixture
  switch($mode){
   'revision'{$b=[IO.File]::ReadAllText($f.Manifest.Driver.PackageManifest.Path)|ConvertFrom-Json;$b.Revision='f'*40
    Write-Text $f.Manifest.Driver.PackageManifest.Path ($b|ConvertTo-Json -Depth 8);$f.Manifest.Driver.PackageManifest=Pin $f.Manifest.Driver.PackageManifest.Path}
   'driver'{Write-Text $f.Manifest.Driver.Files[0].Path 'different driver';$p=Pin $f.Manifest.Driver.Files[0].Path;$p.Name=$f.Manifest.Driver.Files[0].Name;$f.Manifest.Driver.Files[0]=$p}
   'pdb'{Write-Text $f.Manifest.Driver.Pdb.Path 'different PDB';$f.Manifest.Driver.Pdb=Pin $f.Manifest.Driver.Pdb.Path}
   'schedule'{Write-Text $f.Manifest.Schedule.Path 'different schedule';$f.Manifest.Schedule=Pin $f.Manifest.Schedule.Path}
  }
  Save-InputManifest $f;Reject {Open-Inputs $f}
 }
}
Case 'tmf-membership-is-flat-exact-nonempty-and-bounded' {
 foreach($mode in @('extra','directory','duplicate','empty')){
  $f=New-InputFixture
  switch($mode){'extra'{Write-Text (Join-Path $f.Manifest.Symbols.Directory 'extra.tmf') 'extra'}
   'directory'{$null=[IO.Directory]::CreateDirectory((Join-Path $f.Manifest.Symbols.Directory 'nested'))}
   'duplicate'{$f.Manifest.Symbols.Files+=,$f.Manifest.Symbols.Files[0]};'empty'{$f.Manifest.Symbols.Files=@()}}
  Save-InputManifest $f;Reject {Open-Inputs $f}
 }
}
Case 'post-stop-pins-use-real-closed-etl-report-and-original-managed-binding' {
 $f=New-InputFixture;$o=Open-Inputs $f;Set-FixedManagedArguments $f $o
 $p=New-RemovalDiagnosticDecodePins $o -TraceSession $f.Session -ManagedSession $f.ManagedSession
 Check ($p.Etl.Sha256-ceq(Pin $f.Session.NativeOwner.EtlPath).Sha256-and
  $p.ProbeReport.Path-ceq$f.Pins.ProbeReport.Path-and$p.Probe.SourceRevision-ceq$f.Fixed.ProbeSource-and
  $p.DriverPackage.Sha256-ceq$f.Manifest.Driver.PackageManifest.Sha256) 'Post-STOP actual pins lost.'
 $d=New-RemovalTraceDecode -TraceSession $f.Session -ManagedSession $f.ManagedSession -Pins $p -OutputDirectory (Join-Path $f.Root 'unused-decode')
 Check (-not$d.Attempted-and$d.ManagedProvenance.ProbePid-eq9876-and$f.Calls.Count-eq0) 'Pins did not compose with actual decoder or launched tools.'
}
Case 'pre-stop-foreign-managed-or-changed-fixed-argv-never-yields-decode-pins' {
 foreach($mode in @('running','foreign','argv','live','report-absent')){
  $f=New-InputFixture -Running:($mode-ceq'running');$o=Open-Inputs $f;Set-FixedManagedArguments $f $o
  switch($mode){'foreign'{$f.ManagedSession.TraceSession=[pscustomobject]@{}}
   'argv'{$f.ManagedSession.Config.ProbeArguments[8]='481'}
   'live'{$f.ProbeProcess.HasExited=$false};'report-absent'{[IO.File]::Delete($f.Pins.ProbeReport.Path)}}
  Reject {New-RemovalDiagnosticDecodePins $o -TraceSession $f.Session -ManagedSession $f.ManagedSession}
  Check ($f.Calls.Count-eq0) 'Invalid post-STOP input dispatched.'
 }
}
Case 'moredata-stop-diagnostic-pins-never-synthesize-loss-and-returned-pins-are-detached' {
 $f=New-InputFixture -StopStatus 234;$o=Open-Inputs $f;Set-FixedManagedArguments $f $o
 $p=New-RemovalDiagnosticDecodePins $o -TraceSession $f.Session -ManagedSession $f.ManagedSession
 $d=New-RemovalTraceDecode -TraceSession $f.Session -ManagedSession $f.ManagedSession -Pins $p -OutputDirectory (Join-Path $f.Root 'unused-decode')
 Check ($f.Session.StoppedReceipt.Status-eq234-and-not$d.RawTraceLossKnown-and$d.RawTraceLoss-ceq'unknown') 'MORE_DATA acquired loss authority.'
 $p.Probe.File.Sha256='0'*64
 Assert-RemovalRunInputs $o
 Check ($o.Pins.Probe.File.Sha256-ceq$f.Fixed.ProbeSha256) 'Returned decode map aliased original input pins.'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' NATIVE_CALLS=0 CHILDREN=0 FILESYSTEM=real OWNERSHIP_BOUNDARIES=mock')
if($failed){exit 1}
