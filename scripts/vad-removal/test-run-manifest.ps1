#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$failed=$false;try{&$Action|Out-Null}catch{$failed=$true};Check $failed 'Expected refusal.'}
function Case([string]$Name,[scriptblock]$Action){
 try{&$Action;$checks.Add(@{name=$Name;passed=$true})}
 catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}
}
$library=Join-Path $PSScriptRoot 'run-manifest.ps1'
Case 'production-manifest-builder-api-exists' {
 Check (Test-Path -LiteralPath $library -PathType Leaf) 'Production run-manifest builder missing.'
 . $library
 Check ($null-ne(Get-Command New-RemovalRunManifest -ErrorAction SilentlyContinue)) 'Production builder API missing.'
}
if(@($checks|Where-Object{-not$_.passed}).Count){
 $checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_CALLS=0 CHILDREN=0';exit 1
}
. $library

# Import only named fixture function definitions, never test script top-levels,
# native fixture initialization, boundary mocks or test-case execution.
function Import-FixtureFunctions([string]$Path,[string[]]$Names){
 $tokens=$null;$errors=$null;$ast=[Management.Automation.Language.Parser]::ParseFile($Path,[ref]$tokens,[ref]$errors)
 Check ($errors.Count-eq0) 'Fixture source parse failed.'
 foreach($name in $Names){
  $nodes=@($ast.EndBlock.Statements|Where-Object{$_-is[Management.Automation.Language.FunctionDefinitionAst]-and$_.Name-ceq$name})
  Check ($nodes.Count-eq1) ('Unique fixture function required: '+$name)
  $definition=$nodes[0].Extent.Text -replace '\Afunction\s+([^\s(]+)','function script:$1'
  . ([scriptblock]::Create($definition))
 }
}
Import-FixtureFunctions (Join-Path $PSScriptRoot 'test-trace-decode-fixtures.ps1') @('Pin','Write-Text')
Import-FixtureFunctions (Join-Path $PSScriptRoot 'test-run-inputs.ps1') @('Save-InputManifest','New-InputFixture')
$script:manifestFixtureRoot=Join-Path $PSScriptRoot ('logs\run-manifest-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($script:manifestFixtureRoot)
$script:fixtureRoot=$script:manifestFixtureRoot

# The existing input fixture consumes this closed-file seed. No trace/native
# owner is needed to exercise the real run-inputs manifest consumer.
function New-Fixture([switch]$LeaveTraceRunning,[uint32]$StopStatus=0){
 $root=Join-Path $script:manifestFixtureRoot ([Guid]::NewGuid().ToString('N'))
 $null=[IO.Directory]::CreateDirectory($root)
 $tmf=Join-Path $root 'tmf';$null=[IO.Directory]::CreateDirectory($tmf)
 $pins=@{}
 foreach($row in @(@('Tracefmt','tracefmt.exe'),@('Python','python.exe'),@('Reader','vad-removal-stages.py'),
  @('DriverPackage','package-manifest.json'))){
  $path=Join-Path $root $row[1];Write-Text $path ('synthetic '+$row[1]);$pins[$row[0]]=Pin $path
 }
 $probe=Join-Path $root 'e2e_latency_probe.exe';Write-Text $probe 'synthetic original probe'
 $build=Join-Path $root 'probe-build.json';Write-Text $build '{}'
 $pins.Probe=@{File=(Pin $probe);BuildManifest=(Pin $build)}
 $symbol=Join-Path $tmf 'reviewed.tmf';Write-Text $symbol 'synthetic reviewed TMF'
 $pins.Symbols=@{Directory=$tmf;Files=@((Pin $symbol))}
 @{Root=$root;Pins=$pins;Calls=New-Object Collections.Generic.List[object]}
}
function Get-RemovalRunFixedPins {return $script:inputFixture.Fixed}
function New-ManifestFixture {
 $f=New-InputFixture
 # Every actual production PS1, including this builder, belongs to the source
 # membership. The consumer deliberately allows fully pinned new libraries.
 $reviewed=@($f.Manifest.Sources|ForEach-Object{$_.Name})
 $production=@(Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.ps1' -File|Where-Object{$_.Name-notlike'test-*'})
 foreach($entry in $production){
  if($reviewed-ccontains$entry.Name){continue}
  $path=Join-Path $f.SourceDirectory $entry.Name;Write-Text $path ('# independently reviewed fixture '+$entry.Name)
  $pin=Pin $path;$pin.Name=$entry.Name;$f.Manifest.Sources+=,$pin
 }
 $f.Layout=($f.Manifest|ConvertTo-Json -Depth 14|ConvertFrom-Json)
 $f.Layout|Add-Member NoteProperty SourceDirectory $f.SourceDirectory
 $f.Layout.Symbols|Add-Member NoteProperty DriverRevision $f.Fixed.DriverRevision
 $f.Layout.Symbols|Add-Member NoteProperty PdbSha256 $f.Fixed.PdbSha256
 $f.LayoutPath=Join-Path $f.Root 'reviewed-layout.json'
 $f.OutputPath=Join-Path $f.Root 'new-run-manifest.json'
 Save-ReviewedBundle $f
 return $f
}
function Save-Layout($Fixture){
 Write-Text $Fixture.LayoutPath ($Fixture.Layout|ConvertTo-Json -Depth 16)
 $Fixture.LayoutHash=(Pin $Fixture.LayoutPath).Sha256
}
function Save-ReviewedBundle($Fixture){
 $bundle=[ordered]@{SchemaVersion=1;SourceKind='ReviewedBundle';BaseCommit=$Fixture.Layout.SourcesRevision
  BundleIdentity='independent-test-review';Sources=@($Fixture.Layout.Sources|ForEach-Object{
   [ordered]@{Name=$_.Name;Bytes=$_.Bytes;Sha256=$_.Sha256}})}
 $path=Join-Path $Fixture.Root 'reviewed-source-bundle.json'
 Write-Text $path ($bundle|ConvertTo-Json -Depth 8)
 $provenance=[pscustomobject]@{SourceKind='ReviewedBundle';BaseCommit=$Fixture.Layout.SourcesRevision
  BundleIdentity='independent-test-review';SourcesRevisionMeaning='ReviewedBundleBaseCommit';BundleManifestPin=(Pin $path)}
 $Fixture.Layout|Add-Member NoteProperty SourceProvenance $provenance -Force
 Save-Layout $Fixture
}
function Build-Manifest($Fixture){
 New-RemovalRunManifest -LayoutPath $Fixture.LayoutPath -ExpectedLayoutSha256 $Fixture.LayoutHash -ManifestPath $Fixture.OutputPath
}

Case 'reviewed-layout-produces-real-consumer-validated-durable-manifest-pin' {
 $f=New-ManifestFixture;$pin=Build-Manifest $f;$actual=Pin $f.OutputPath
 Check ($pin.Path-ceq$f.OutputPath-and$pin.Bytes-eq$actual.Bytes-and$pin.Sha256-ceq$actual.Sha256) 'Exact readback pin lost.'
 $owner=New-RemovalRunInputs -ManifestPath $pin.Path -ExpectedSha256 $pin.Sha256 -SourceDirectory $f.SourceDirectory
 Assert-RemovalRunInputs $owner
 Check ($owner.Pins.Sources.Count-eq$f.Layout.Sources.Count-and$owner.SourceRevision-ceq$f.Layout.SourcesRevision) 'Complete reviewed sources lost.'
 Check ($owner.Pins.Probe.SourceRevision-ceq$f.Fixed.ProbeSource-and$owner.Pins.Driver.Pdb.Sha256-ceq$f.Fixed.PdbSha256) 'Original release/driver relabelled.'
 Check ($f.Calls.Count-eq0) 'Builder launched a process.'
}
Case 'independent-layout-pin-is-required-and-not-recomputed' {
 $f=New-ManifestFixture
 Reject {New-RemovalRunManifest -LayoutPath $f.LayoutPath -ExpectedLayoutSha256 ('0'*64) -ManifestPath $f.OutputPath}
 Check (-not[IO.File]::Exists($f.OutputPath)) 'Wrong layout hash published a manifest.'
 [IO.File]::AppendAllText($f.LayoutPath,' ')
 Reject {Build-Manifest $f}
 Check (-not[IO.File]::Exists($f.OutputPath)) 'Changed layout was silently adopted.'
}
Case 'reviewed-source-content-and-complete-membership-are-authority-not-revision-label' {
 foreach($mode in @('mutated','missing','extra','duplicate','outside','unpin')){
  $f=New-ManifestFixture
  switch($mode){
   'mutated'{[IO.File]::AppendAllText($f.Layout.Sources[0].Path,' changed')}
   'missing'{$f.Layout.Sources=@($f.Layout.Sources|Where-Object{$_.Name-cne'worker.ps1'});Save-Layout $f}
   'extra'{Write-Text (Join-Path $f.SourceDirectory 'unreviewed.ps1') '# unreviewed'}
   'duplicate'{$f.Layout.Sources+=,$f.Layout.Sources[0];Save-Layout $f}
   'outside'{$f.Layout.Sources[0].Path=$f.Layout.Schedule.Path;Save-Layout $f}
   'unpin'{$f.Layout.Sources[0].Sha256=$null;Save-Layout $f}
  }
  Reject {Build-Manifest $f};Check (-not[IO.File]::Exists($f.OutputPath)) ('Invalid source published: '+$mode)
 }
}
Case 'nonfixed-tools-and-tmf-expected-provenance-cannot-be-adopted-after-mutation' {
 foreach($mode in @('tracefmt','python','reader','bcd','tmf','build')){
  $f=New-ManifestFixture
  $path=switch($mode){'tracefmt'{$f.Layout.Tools.Tracefmt.Path};'python'{$f.Layout.Tools.Python.Path}
   'reader'{$f.Layout.Tools.Reader.Path};'bcd'{$f.Layout.Tools.BcdEdit.Path};'tmf'{$f.Layout.Symbols.Files[0].Path}
   'build'{$f.Layout.Probe.BuildManifest.Path}}
  [IO.File]::AppendAllText($path,' changed')
  Reject {Build-Manifest $f};Check (-not[IO.File]::Exists($f.OutputPath)) ('Unreviewed content adopted: '+$mode)
 }
}
Case 'symbol-review-binds-original-driver-and-pdb-without-generating-symbols' {
 foreach($mode in @('revision','pdb','absent')){
  $f=New-ManifestFixture
  switch($mode){'revision'{$f.Layout.Symbols.DriverRevision='f'*40};'pdb'{$f.Layout.Symbols.PdbSha256='0'*64}
   'absent'{$f.Layout.Symbols.PSObject.Properties.Remove('PdbSha256')}}
  Save-Layout $f;Reject {Build-Manifest $f}
  Check (-not[IO.File]::Exists($f.OutputPath)) 'Unbound TMF provenance published.'
 }
}
Case 'typed-layout-schema-paths-and-pin-shapes-refuse-before-create' {
 foreach($mode in @('schema','revision','size','hash','directory','unknown-field')){
  $f=New-ManifestFixture
  switch($mode){'schema'{$f.Layout.SchemaVersion='1'};'revision'{$f.Layout.SourcesRevision='main'}
   'size'{$f.Layout.Tools.Tracefmt.Bytes=[string]$f.Layout.Tools.Tracefmt.Bytes};'hash'{$f.Layout.Tools.Reader.Sha256='bad'}
   'directory'{$f.Layout.SourceDirectory=$f.Root};'unknown-field'{$f.Layout|Add-Member NoteProperty Reviewed $true}}
  Save-Layout $f;Reject {Build-Manifest $f}
  Check (-not[IO.File]::Exists($f.OutputPath)) ('Invalid typed layout published: '+$mode)
 }
}
Case 'malformed-duplicate-case-ambiguous-or-nonfinite-json-is-not-an-authority' {
 foreach($text in @('{','[]','{"SchemaVersion":1,"SchemaVersion":2}',
  '{"SchemaVersion":1,"schemaversion":1}','{"SchemaVersion":NaN}','{"SchemaVersion":1e400}')){
  $f=New-ManifestFixture;Write-Text $f.LayoutPath $text;$f.LayoutHash=(Pin $f.LayoutPath).Sha256
  Reject {Build-Manifest $f};Check (-not[IO.File]::Exists($f.OutputPath)) 'Malformed authority published.'
 }
}
Case 'create-new-output-never-overwrites-layout-input-existing-file-or-missing-parent' {
 $f=New-ManifestFixture;Write-Text $f.OutputPath 'original'
 Reject {Build-Manifest $f};Check ([IO.File]::ReadAllText($f.OutputPath)-ceq'original') 'Existing output overwritten.'
 Reject {New-RemovalRunManifest -LayoutPath $f.LayoutPath -ExpectedLayoutSha256 $f.LayoutHash -ManifestPath $f.LayoutPath}
 $missing=Join-Path $f.Root 'absent\run.json'
 Reject {New-RemovalRunManifest -LayoutPath $f.LayoutPath -ExpectedLayoutSha256 $f.LayoutHash -ManifestPath $missing}
 Check (-not[IO.Directory]::Exists((Split-Path $missing))) 'Builder created an unreviewed parent directory.'
}
Case 'consumer-validation-failure-retains-created-artifact-and-never-returns-success' {
 $f=New-ManifestFixture
 $original=${function:New-RemovalRunInputs}
 try{
  function New-RemovalRunInputs {throw 'injected post-write consumer refusal'}
  Reject {Build-Manifest $f}
  Check ([IO.File]::Exists($f.OutputPath)) 'Failed publication artifact was erased or write was skipped.'
 }finally{Set-Item Function:New-RemovalRunInputs $original}
}
Case 'all-actual-production-dependencies-are-reviewed-including-new-builder' {
 $f=New-ManifestFixture
 $extra=Join-Path $f.SourceDirectory 'extra-library.ps1';Write-Text $extra '# reviewed extra production library'
 $pin=Pin $extra;$pin.Name='extra-library.ps1';$f.Layout.Sources+=,$pin
 Write-Text (Join-Path $f.SourceDirectory 'test-excluded.ps1') '# test only'
 Save-ReviewedBundle $f;$built=Build-Manifest $f
 $owner=New-RemovalRunInputs -ManifestPath $built.Path -ExpectedSha256 $built.Sha256 -SourceDirectory $f.SourceDirectory
 Check ($owner.Pins.Sources.Count-eq$f.Layout.Sources.Count) 'Actual new production dependency was omitted.'
}
Case 'reviewed-bundle-provenance-is-preserved-without-claiming-an-exact-git-tree' {
 $f=New-ManifestFixture;$pin=Build-Manifest $f
 $manifest=[IO.File]::ReadAllText($pin.Path)|ConvertFrom-Json
 Check ($manifest.SourceProvenance.SourceKind-ceq'ReviewedBundle'-and
  $manifest.SourceProvenance.BaseCommit-ceq$f.Layout.SourcesRevision-and
  $manifest.SourceProvenance.SourcesRevisionMeaning-ceq'ReviewedBundleBaseCommit'-and
  $manifest.SourceProvenance.BundleManifestPin.Sha256-ceq$f.Layout.SourceProvenance.BundleManifestPin.Sha256) 'Independent source provenance lost or relabelled as GitTree.'
}
Case 'bundle-pin-content-membership-and-honest-base-identity-are-required' {
 foreach($mode in @('mutation','base','identity','hash','missing','extra','git-label','absent')){
  $f=New-ManifestFixture;$path=$f.Layout.SourceProvenance.BundleManifestPin.Path
  $bundle=[IO.File]::ReadAllText($path)|ConvertFrom-Json
  switch($mode){
   'mutation'{[IO.File]::AppendAllText($path,' ')}
   'base'{$f.Layout.SourceProvenance.BaseCommit='f'*40}
   'identity'{$f.Layout.SourceProvenance.BundleIdentity='other-review'}
   'hash'{$bundle.Sources[0].Sha256='0'*64}
   'missing'{$bundle.Sources=@($bundle.Sources|Select-Object -Skip 1)}
   'extra'{$bundle.Sources+=,[pscustomobject]@{Name='foreign.ps1';Bytes=1;Sha256=('a'*64)}}
   'git-label'{$f.Layout.SourceProvenance.SourceKind='GitTree'}
   'absent'{$f.Layout.PSObject.Properties.Remove('SourceProvenance')}
  }
  if($mode-in@('hash','missing','extra')){
   Write-Text $path ($bundle|ConvertTo-Json -Depth 8)
   $f.Layout.SourceProvenance.BundleManifestPin=Pin $path
  }
  Save-Layout $f;Reject {Build-Manifest $f}
  Check (-not[IO.File]::Exists($f.OutputPath)) ('Invalid bundle authority published: '+$mode)
 }
}
Case 'runtime-relocation-changes-paths-only-and-retains-reviewed-content-and-provenance' {
 $f=New-ManifestFixture;$relocated=Join-Path $f.Root 'relocated-sources'
 $null=[IO.Directory]::CreateDirectory($relocated)
 foreach($row in $f.Layout.Sources){
  $destination=Join-Path $relocated $row.Name
  [IO.File]::Copy($row.Path,$destination,$false);$row.Path=$destination
 }
 $f.Layout.SourceDirectory=$relocated;Save-Layout $f
 $pin=Build-Manifest $f;$manifest=[IO.File]::ReadAllText($pin.Path)|ConvertFrom-Json
 Check ($manifest.Sources[0].Path.StartsWith($relocated+'\')-and
  $manifest.SourceProvenance.BundleManifestPin.Sha256-ceq$f.Layout.SourceProvenance.BundleManifestPin.Sha256) 'Relocation changed reviewed content/provenance.'
}
Case 'inputs-exclude-writers-during-consumer-validation-and-all-owned-handles-close' {
 $f=New-ManifestFixture;$script:manifestLockFixture=$f;$original=${function:Read-RemovalRunPins}
 $script:manifestConsumerOriginal=$original;$script:manifestDenied=0
 try{
  function Read-RemovalRunPins($Manifest,[string]$SourceDirectory){
   foreach($path in @($script:manifestLockFixture.LayoutPath,$script:manifestLockFixture.Layout.Sources[0].Path,
     $script:manifestLockFixture.Layout.Tools.Tracefmt.Path,$script:manifestLockFixture.Layout.SourceProvenance.BundleManifestPin.Path)){
    try{$writer=[IO.File]::Open($path,'Open','Write','ReadWrite');$writer.Dispose();throw 'Expected active input lock.'}
    catch [IO.IOException]{$script:manifestDenied++}
   }
   & $script:manifestConsumerOriginal $Manifest $SourceDirectory
  }
  $pin=Build-Manifest $f
  Check ($script:manifestDenied-ge4) 'Actual input handles did not exclude writers.'
 }finally{Set-Item Function:Read-RemovalRunPins $original}
 foreach($path in @($f.LayoutPath,$f.OutputPath,$f.Layout.Sources[0].Path,$f.Layout.Tools.Tracefmt.Path,
   $f.Layout.SourceProvenance.BundleManifestPin.Path)){
  $exclusive=[IO.File]::Open($path,'Open','ReadWrite','None');$exclusive.Dispose()
 }
}
Case 'post-write-refusal-closes-owned-handles-without-deleting-failed-artifact' {
 $f=New-ManifestFixture;$original=${function:New-RemovalRunInputs}
 try{
  function New-RemovalRunInputs {throw 'injected durable validation fault'}
  Reject {Build-Manifest $f}
 }finally{Set-Item Function:New-RemovalRunInputs $original}
 Check ([IO.File]::Exists($f.OutputPath)) 'Failed artifact missing.'
 foreach($path in @($f.LayoutPath,$f.OutputPath,$f.Layout.Tools.Tracefmt.Path)){
  $exclusive=[IO.File]::Open($path,'Open','ReadWrite','None');$exclusive.Dispose()
 }
}
Case 'invalid-or-nonregular-paths-refuse-without-creating-any-new-output' {
 foreach($mode in @('traversal','ads','trailing-dot','directory-input','directory-output','oversize-layout')){
  $f=New-ManifestFixture
  switch($mode){
   'traversal'{$f.OutputPath=Join-Path $f.Root 'sources\..\escape.json'}
   'ads'{$f.OutputPath+=':stream'}
   'trailing-dot'{$f.OutputPath+='.'}
   'directory-input'{$f.Layout.Tools.Tracefmt.Path=$f.Root;Save-Layout $f}
   'directory-output'{$f.OutputPath=$f.Root}
   'oversize-layout'{Write-Text $f.LayoutPath ('x'*1048577);$f.LayoutHash=(Pin $f.LayoutPath).Sha256}
  }
  Reject {Build-Manifest $f}
 }
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' NATIVE_CALLS=0 CHILDREN=0 FILESYSTEM=real FIXED_PINS=fixture-only')
# Tiny owned fixtures are kept under ignored logs for reproducibility; no
# recursive cleanup, external cache deletion or approval bypass is attempted.
if($failed){exit 1}
