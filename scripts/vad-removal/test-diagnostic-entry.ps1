#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){throw 'Host-only entry tests must not run on target.'}
. (Join-Path $PSScriptRoot 'finite-tool.ps1')
$root=Join-Path $PSScriptRoot ('logs\diagnostic-entry-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$entry=Join-Path $PSScriptRoot 'invoke-diagnostic.ps1'
$isolated=Join-Path $root 'invoke-diagnostic.ps1'
[IO.File]::WriteAllBytes($isolated,[IO.File]::ReadAllBytes($entry))
$exe=[Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
$checks=New-Object Collections.Generic.List[object]
foreach($mode in @('confirmation','host')){
 $path=Join-Path $root $mode
 $arguments=@('-NoLogo','-NoProfile','-NonInteractive','-File',$isolated,'-RunDirectory',$path,
  '-ManifestPath',(Join-Path $root 'missing-manifest.json'),'-ManifestSha256',('A'*64))
 if($mode-ceq'host'){$arguments+=,'-UserRebootConfirmed'}
 $o=Start-FiniteTool -Executable $exe -ExpectedSha256 (Get-FileHash $exe).Hash `
  -OutputDirectory (Join-Path $root ($mode+'-child')) -ArgumentList $arguments
 $r=Observe-FiniteTool $o
 if(-not$r.ExitedConfirmed-or$r.ExitCode-ne1-or$r.NeedsObservation){throw 'Guarded diagnostic entry did not exit with refusal.'}
 Close-FiniteToolOwner $o
 $text=[IO.File]::ReadAllText($o.StderrPath)
 $expected=if($mode-ceq'confirmation'){'*User reboot confirmation required*'}else{'*Wrong diagnostic target host/SID*'}
 if($text-notlike$expected-or(Test-Path $path)){throw 'Entry imported helpers/read manifest/touched target before guard.'}
 $checks.Add(@{name=$mode+'-before-imports-and-files';passed=$true})
}

function Invoke-EncodingRegression {
 [CmdletBinding()]
 param([Parameter(Mandatory=$true)][string]$SourceDirectory,
  [Parameter(Mandatory=$true)][string]$FixtureParent)
$engine=[Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
$enginePin=(Get-FileHash -LiteralPath $engine -Algorithm SHA256).Hash
$root=Join-Path $FixtureParent ('encoding-'+$PSVersionTable.PSVersion.Major+'-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
function Save-EncodingAscii([string]$Path,[string]$Text){
 if(@($Text.ToCharArray()|Where-Object{[int]$_-gt127}).Count){throw 'Candidate/fixture source must remain ASCII.'}
 $s=[IO.File]::Open($Path,'CreateNew','Write','None')
 try{$b=[Text.Encoding]::ASCII.GetBytes($Text);$s.Write($b,0,$b.Length);$s.Flush($true)}finally{$s.Dispose()}
}
function Get-EncodingPin([string]$Path){[ordered]@{Path=$Path;Bytes=[long]([IO.FileInfo]$Path).Length;Sha256=(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash}}
$original=Join-Path $SourceDirectory 'invoke-diagnostic.ps1'
$text=[IO.File]::ReadAllText($original,[Text.UTF8Encoding]::new($false,$true))
$originalText=$text
$tokens=$null;$parseErrors=$null
$ast=[Management.Automation.Language.Parser]::ParseInput($text,[ref]$tokens,[ref]$parseErrors)
if($parseErrors.Count){throw 'Original source parse failed.'}
$guards=@($ast.FindAll({param($node)
 $node-is[Management.Automation.Language.IfStatementAst]-and(
  $node.Extent.Text.StartsWith('if([Environment]::MachineName')-or
  $node.Extent.Text.StartsWith('if(-not([Security.Principal.WindowsPrincipal]')-or
  $node.Extent.Text.StartsWith('if($RunDirectory -cnotmatch'))
},$true))
if($guards.Count-ne3){throw 'Expected exact original host/elevation/root guards.'}
foreach($guard in @($guards|Sort-Object {$_.Extent.StartOffset} -Descending)){
 $text=$text.Remove($guard.Extent.StartOffset,$guard.Extent.EndOffset-$guard.Extent.StartOffset).Insert(
  $guard.Extent.StartOffset,'# host-only fixture: removed target/elevation/root admission guard')
}
$entry=Join-Path $root 'invoke-diagnostic.ps1';Save-EncodingAscii $entry $text
Save-EncodingAscii (Join-Path $root 'diagnostic-controller.ps1') @'
function New-RemovalRunInputs { [pscustomobject]@{Fixture=$true} }
function New-RemovalDiagnosticController { [pscustomobject]@{Completed=$true} }
function Start-RemovalDiagnosticController {
 if($global:EncodingFixtureError){throw $global:EncodingExpectedError}
}
function Add-RemovalControllerError {}
function Observe-RemovalDiagnosticController {
 [pscustomobject]@{Completed=$true;Safe=$false;NeedsObservation=$false;Qualified=$false}
}
'@
Save-EncodingAscii (Join-Path $root 'closed-export.ps1') @'
function Export-RemovalClosedRun { [pscustomobject]@{Qualified=$false} }
function Confirm-RemovalClosedRunExport {
 [pscustomobject]@{Path=($RunDirectory+'.archive-local.json');Bytes=7;Sha256=('A'*64)}
}
'@
$wrapper=Join-Path $root 'legacy-child.ps1'
Save-EncodingAscii $wrapper @'
param([string]$Entry,[string]$Run,[int]$CodePage,[int]$ErrorMode)
$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSHOME 'Modules\Microsoft.PowerShell.Utility\Microsoft.PowerShell.Utility.psd1') -ErrorAction Stop
[Console]::OutputEncoding=[Text.Encoding]::GetEncoding($CodePage)
$global:EncodingFixtureError=($ErrorMode-eq1)
$global:EncodingExpectedError=([string][char]0x9519)+[char]0x8bef+' '+([string][char]0x4e2d)+[char]0x6587
& $Entry -RunDirectory $Run -ManifestPath ($Run+'.manifest.json') -ManifestSha256 ('A'*64) -UserRebootConfirmed
exit $LASTEXITCODE
'@
$encodingChecks=New-Object Collections.Generic.List[object]
$owners=New-Object Collections.Generic.List[object]
$unicode=([string][char]0x4e2d)+[char]0x6587
$expectedError='Removal diagnostic failed/unknown: '+([string][char]0x9519)+[char]0x8bef+' '+$unicode
foreach($codePage in @(437,936)){
 foreach($errorMode in @(0,1)){
  $run=Join-Path $root ('run-'+$codePage+'-'+$errorMode+'-'+$unicode)
  $owner=Start-FiniteTool -Executable $engine -ExpectedSha256 $enginePin -OutputDirectory (Join-Path $root ('output-'+$codePage+'-'+$errorMode)) `
   -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-File',$wrapper,'-Entry',$entry,'-Run',$run,
    '-CodePage',[string]$codePage,'-ErrorMode',[string]$errorMode) -TimeoutSeconds 15 -MaxLogBytes 1048576
  $owners.Add($owner)
  $receipt=Observe-FiniteTool $owner
  while(-not$receipt.ExitedConfirmed-or$receipt.NeedsObservation){
   Start-Sleep -Milliseconds 20;$receipt=Observe-FiniteTool $owner -ObservationMilliseconds 100
  }
  Close-FiniteToolOwner $owner
  $stdout=[IO.File]::ReadAllBytes($owner.StdoutPath);$stderr=[IO.File]::ReadAllBytes($owner.StderrPath)
  $stdoutUtf8=$true;$stdoutExact=$false;$stderrUtf8=$true;$stderrExact=$false
  try{
   $json=[Text.UTF8Encoding]::new($false,$true).GetString($stdout)|ConvertFrom-Json
   $stdoutExact=($json.LocalAcknowledgement.Path-ceq($run+'.archive-local.json'))
  }catch{$stdoutUtf8=$false}
  try{
   $message=[Text.UTF8Encoding]::new($false,$true).GetString($stderr)
   $stderrExact=if($errorMode-eq1){$message.TrimEnd([char]13,[char]10)-ceq$expectedError}else{$stderr.Length-eq0}
  }catch{$stderrUtf8=$false}
  $passed=($receipt.ExitCode-eq1-and-not$receipt.TimedOut-and$receipt.OutputStable-and$stdoutExact-and$stderrExact)
  $encodingChecks.Add([ordered]@{name=('entry-utf8-codepage-'+$codePage+'-error-'+$errorMode);CodePage=$codePage;ErrorMode=$errorMode;passed=$passed;ExitCode=$receipt.ExitCode
   ExitedConfirmed=$receipt.ExitedConfirmed;OutputStable=$receipt.OutputStable;StdoutStrictUtf8=$stdoutUtf8
   StdoutExactUnicode=$stdoutExact;StderrStrictUtf8=$stderrUtf8;StderrExactUnicode=$stderrExact
   StdoutPin=(Get-EncodingPin $owner.StdoutPath);StderrPin=(Get-EncodingPin $owner.StderrPath)})
 }
}
if([IO.File]::ReadAllText($original)-cne$originalText){throw 'Original production source changed; no relabelled candidate result.'}
$result=[ordered]@{SchemaVersion=1;SourcePin=(Get-EncodingPin $original);CandidatePin=(Get-EncodingPin $entry)
 SourceDelta='Only three target/elevation/root IfStatementAst guards replaced; encoding and output unchanged.'
 Expected='Strict UTF8 no-BOM stdout/stderr must preserve exact non-ASCII paths/error text under inherited legacy codepage.'
 Interpreter=$engine;Checks=@($encodingChecks.ToArray());NativeCalls=0;RemoteCalls=0;HarmlessChildren=$owners.Count
 AllOriginalOwnersClosed=(@($owners|Where-Object{-not$_.Closed}).Count-eq0);FixtureRoot=$root}
$json=ConvertTo-Json -InputObject $result -Depth 10
$s=[IO.File]::Open((Join-Path $root 'result.json'),'CreateNew','Write','None')
try{$b=[Text.UTF8Encoding]::new($false).GetBytes($json);$s.Write($b,0,$b.Length);$s.Flush($true)}finally{$s.Dispose()}
return [pscustomobject]$result
}

$encoding=Invoke-EncodingRegression -SourceDirectory $PSScriptRoot -FixtureParent $root
foreach($encodingCheck in $encoding.Checks){$checks.Add($encodingCheck)}
$checks|ConvertTo-Json -Depth 7
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('FIXTURE_ROOT='+$root)
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ACTUAL_ENTRY_REFUSALS=true NATIVE_AUDIO=false')
if($failed){exit 1}
