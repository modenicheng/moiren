#requires -Version 5.1
# Real harmless PowerShell children and files; only the target guard and child
# entry script are fixtures. No target, scheduler, ETW, audio or driver APIs run.
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName-ceq'LAPTOP-NP49RG6H'){throw 'Entry launcher tests are development-host only.'}
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$refused=$false;try{&$Action|Out-Null}catch{$refused=$true};Check $refused 'Expected refusal.'}
function Case([string]$Name,[scriptblock]$Action){
    try{&$Action;$checks.Add(@{name=$Name;passed=$true})}
    catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message;at=$_.ScriptStackTrace})}
}
$library=Join-Path $PSScriptRoot 'entry-launch.ps1'
Case 'entry-launch-api-exists-with-no-import-dispatch' {
    Check ([IO.File]::Exists($library)) 'Retained entry launcher is missing.'
    . $library
    Check ($null-ne(Get-Command Invoke-RemovalDiagnosticEntry -ErrorAction SilentlyContinue)) 'Entry launcher API missing.'
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_OPERATIONS=0';exit 1}
. $library
$script:fixtureRoot=Join-Path $PSScriptRoot ('logs\entry-launch\'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($script:fixtureRoot)
$script:guardCalls=0
function Assert-RemovalDiagnosticEntryTarget([string]$RunDirectory){$script:guardCalls++}
function Get-RemovalDiagnosticEntryScript([string]$SourceDirectory){return $script:fixtureChild}
function Write-Text([string]$Path,[string]$Text){[IO.File]::WriteAllText($Path,$Text,[Text.UTF8Encoding]::new($false,$true))}
function Pin([string]$Path){[pscustomobject]@{Path=$Path;Bytes=[long]([IO.FileInfo]$Path).Length;Sha256=(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash}}
$exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$exePin=Pin $exe
$childPrefix=@'
param([string]$RunDirectory,[string]$ManifestPath,[string]$ManifestSha256,[switch]$UserRebootConfirmed,[int]$WorkerSessionId)
$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSHOME 'Modules\Microsoft.PowerShell.Utility\Microsoft.PowerShell.Utility.psd1') -ErrorAction Stop
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
function WriteJson($Path,$Value){[IO.File]::WriteAllText($Path,($Value|ConvertTo-Json -Depth 16 -Compress),[Text.UTF8Encoding]::new($false))}
function Pin($Path){[pscustomobject]@{Path=$Path;Bytes=[long]([IO.FileInfo]$Path).Length;Sha256=(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash}}
function MakeAck {
 $null=[IO.Directory]::CreateDirectory($RunDirectory);$nonce='0123456789abcdef0123456789abcdef'
 $result=Join-Path $RunDirectory 'controller-result.json'
 WriteJson $result ([ordered]@{SchemaVersion=1;ControllerComplete=$true;Safe=$false;NativePassed=$false;Qualified=$false;Nonce=$nonce;Manifest=(Pin $ManifestPath)})
 [IO.File]::WriteAllText(($RunDirectory+'.zip'),'harmless synthetic archive bytes')
 $rp=Pin $result;$ap=Pin ($RunDirectory+'.zip')
 WriteJson ($RunDirectory+'.zip.inventory.json') ([ordered]@{SchemaVersion=1;Nonce=$nonce;ResultPin=$rp;Members=@()})
 $ack=[ordered]@{SchemaVersion=1;Meaning='LOCAL_ARCHIVE_VERIFICATION_ONLY';Nonce=$nonce;ResultPin=$rp;ArchivePin=$ap;InventoryPin=(Pin ($RunDirectory+'.zip.inventory.json'));Transferred=$false;NativePassed=$false;Qualified=$false}
 WriteJson ($RunDirectory+'.archive-local.json') $ack
 $export=[ordered]@{SchemaVersion=1;Nonce=$nonce;ResultPin=$rp;ArchivePin=$ap;InventoryPin=$ack.InventoryPin;LocalVerified=$true;Transferred=$false;NativePassed=$false;Qualified=$false}
 $output=[ordered]@{ControllerResult=[ordered]@{Completed=$true;Refused=$false;Safe=$false;NeedsObservation=$false;NativePassed=$false;Qualified=$false;Phase='Complete';ReceiptPin=$rp;Errors=@();TimedOut=$false};Archive=$export;LocalAcknowledgement=(Pin ($RunDirectory+'.archive-local.json'));Qualified=$false}
 return $output
}
'@
function New-Fixture([string]$Body){
    $dir=Join-Path $script:fixtureRoot ([Guid]::NewGuid().ToString('N'));$null=[IO.Directory]::CreateDirectory($dir)
    $source=Join-Path $dir 'source';$null=[IO.Directory]::CreateDirectory($source)
    $entry=Join-Path $source 'invoke-diagnostic.ps1';Write-Text $entry ($childPrefix+"`r`n"+$Body)
    $manifest=Join-Path $dir 'manifest.json';Write-Text $manifest '{"SchemaVersion":1,"Fixture":true}'
    [pscustomobject]@{Run=(Join-Path $dir 'run');Source=$source;Entry=$entry;Manifest=$manifest;ManifestSha=(Pin $manifest).Sha256}
}
function Invoke-Fixture($Fixture,[switch]$NoConfirmation){
    $script:fixtureChild=$Fixture.Entry
    Invoke-RemovalDiagnosticEntry -RunDirectory $Fixture.Run -ManifestPath $Fixture.Manifest -ManifestSha256 $Fixture.ManifestSha `
        -SourceDirectory $PSScriptRoot -PowerShellPin $exePin -WorkerSessionId 7 -UserRebootConfirmed:(-not$NoConfirmation)
}
function Read-Terminal($Fixture,$Pin){
    Check ($Pin.Path-ceq($Fixture.Run+'.entry-terminal.json')-and@($Pin.PSObject.Properties).Count-eq3) 'Return is not the exact terminal PIN.'
    Check ((Pin $Pin.Path).Sha256-ceq$Pin.Sha256) 'Terminal readback hash differs.'
    $r=[IO.File]::ReadAllText($Pin.Path)|ConvertFrom-Json
    Check ($r.Meaning-ceq'ORIGINAL_DIAGNOSTIC_ENTRY_TERMINAL'-and$r.Target-ceq'moiren-lan'-and$r.RunDirectory-ceq$Fixture.Run) 'Terminal identity differs.'
    Check ($r.NativePassed-eq$false-and$r.Qualified-eq$false-and$r.Process.Pid-gt0-and$r.Process.CreationUtcTicks-gt0-and$r.Process.ExitedConfirmed) 'Original process identity missing.'
    Check ($r.StdoutPin.Path-ceq($Fixture.Run+'.entry-stdout.log')-and$r.StderrPin.Path-ceq($Fixture.Run+'.entry-stderr.log')) 'Output paths differ.'
    return $r
}
Case 'exit-one-closed-failure-retains-independent-ack-and-original-child' {
    $f=New-Fixture '$output=MakeAck;[Console]::Write(($output|ConvertTo-Json -Depth 16 -Compress));[Console]::Error.Write("closed failure");exit 1'
    $pin=Invoke-Fixture $f;$r=Read-Terminal $f $pin
    Check ($r.ExitCode-eq1-and$r.OutputStable-and-not$r.TimedOut-and$r.Errors.Count-eq0) 'Exit1 evidence rejected or relabeled.'
    Check ($r.LocalAcknowledgementPin.Sha256-ceq(Pin ($f.Run+'.archive-local.json')).Sha256) 'Ack pin not independently read.'
    Check ($r.EntrySourcePin.Sha256-ceq(Pin $library).Sha256-and$r.ChildSourcePin.Sha256-ceq(Pin $f.Entry).Sha256-and$r.ManifestPin.Sha256-ceq$f.ManifestSha) 'Original source/manifest pins missing.'
    Check ([IO.File]::ReadAllText($r.StderrPin.Path)-ceq'closed failure') 'stderr lost.'
    $intent=[IO.File]::ReadAllText(($f.Run+'.entry-intent.json'))|ConvertFrom-Json
    Check ($intent.WorkerSessionId-eq7-and$intent.DeadlineSeconds-eq600) 'Original launch intent not persisted.'
}
Case 'exit-zero-without-ack-persists-terminal-but-cannot-deliver' {
    $f=New-Fixture '[Console]::Write("no closed ack");exit 0';$r=Read-Terminal $f (Invoke-Fixture $f)
    Check ($r.ExitCode-eq0-and$r.OutputStable-and$null-eq$r.LocalAcknowledgementPin-and$r.Errors.Count-gt0) 'Missing ack accepted.'
}
Case 'shared-one-mib-cap-drains-original-child-to-natural-exit' {
    $f=New-Fixture '$s="x"*8192;for($i=0;$i-lt160;$i++){[Console]::Out.Write($s);[Console]::Error.Write($s)};[IO.File]::WriteAllText(($RunDirectory+".natural"),[string]$PID);exit 1'
    $r=Read-Terminal $f (Invoke-Fixture $f)
    Check ($r.ExitCode-eq1-and-not$r.OutputStable-and$r.Errors.Count-gt0-and$null-eq$r.LocalAcknowledgementPin) 'Cap failure hidden.'
    Check (($r.StdoutPin.Bytes+$r.StderrPin.Bytes)-le1048576) 'Shared output cap exceeded.'
    Check ([int][IO.File]::ReadAllText(($f.Run+'.natural'))-eq$r.Process.Pid) 'Original child did not naturally finish.'
}
Case 'second-channel-predispatch-setup-refusal-resumes-exact-pipes-and-child' {
    $priorSink=Get-Command New-RemovalDiagnosticEntrySink -ErrorAction SilentlyContinue
    $script:setupOwner=$null;$script:firstCopy=$null;$script:firstSource=$null;$script:firstSink=$null
    $script:sinkCalls=@(0,0);$script:setupRefused=$false
    function New-RemovalDiagnosticEntrySink($Owner,$Channel){
        if($null-eq$script:setupOwner){$script:setupOwner=$Owner}
        Check ([Object]::ReferenceEquals($script:setupOwner,$Owner)) 'Setup replaced original child owner.'
        $script:sinkCalls[$Channel.Index]++
        if($Channel.Index-eq1-and-not$script:setupRefused){
            $script:setupRefused=$true;$script:firstCopy=$Owner.Channels[0].Copy
            $script:firstSource=$Owner.Channels[0].Source;$script:firstSink=$Owner.Channels[0].Sink
            throw 'Test-only stderr sink refusal before CopyToAsync dispatch.'
        }
        return [Moiren.RemovalFiniteToolV1.Sink]::new($Owner.Files[$Channel.Index],$Owner.Budget)
    }
    try{
        $f=New-Fixture '$s="e"*8192;for($i=0;$i-lt64;$i++){[Console]::Error.Write($s)};[Console]::Write("drained original");[IO.File]::WriteAllText(($RunDirectory+".natural"),[string]$PID);exit 1'
        $r=Read-Terminal $f (Invoke-Fixture $f)
        Check $script:setupRefused 'Production did not route the second-channel setup boundary.'
        Check ($script:sinkCalls[0]-eq1-and$script:sinkCalls[1]-eq2) 'Sink setup was not resumed once on the missing channel only.'
        Check ($r.ExitCode-eq1-and$r.OutputStable-and$r.Errors.Count-gt0-and$null-eq$r.LocalAcknowledgementPin) 'Setup recovery erased sticky failure.'
        Check ($r.StderrPin.Bytes-eq524288-and[IO.File]::ReadAllText($r.StdoutPin.Path)-ceq'drained original') 'Recovered original pipes were not fully drained.'
        Check ([int][IO.File]::ReadAllText(($f.Run+'.natural'))-eq$r.Process.Pid-and$script:setupOwner.Pid-eq$r.Process.Pid) 'Child was replaced or did not naturally exit.'
        Check ([Object]::ReferenceEquals($script:firstCopy,$script:setupOwner.Channels[0].Copy)-and
            [Object]::ReferenceEquals($script:firstSource,$script:setupOwner.Channels[0].Source)-and
            [Object]::ReferenceEquals($script:firstSink,$script:setupOwner.Channels[0].Sink)) 'Already-dispatched first channel was replaced.'
        Check ($script:setupOwner.Closed-and$script:setupOwner.Copies.Count-eq2-and
            @($script:setupOwner.Channels|Where-Object{$_.DispatchState-cne'Dispatched'}).Count-eq0) 'Recovered exact owner did not close.'
    }finally{
        if($null-ne$priorSink){Set-Item Function:New-RemovalDiagnosticEntrySink $priorSink.ScriptBlock}
        else{Remove-Item Function:New-RemovalDiagnosticEntrySink -ErrorAction SilentlyContinue}
    }
}
Case 'ambiguous-copy-call-is-never-repeated-and-other-channel-still-progresses' {
    Check ($null-ne(Get-Command Start-RemovalDiagnosticEntryCopies -ErrorAction SilentlyContinue)) 'Incremental original-channel setup is missing.'
    $source=[pscustomobject]@{Calls=0}
    $source|Add-Member ScriptMethod CopyToAsync {$this.Calls++;throw 'Unknown dispatch: never retry this stream call.'}
    $good=[IO.MemoryStream]::new([byte[]]@(65,66,67));$out=[IO.MemoryStream]::new()
    $owner=[pscustomobject]@{Channels=@(
        [pscustomobject]@{Index=0;Source=$source;Sink=$out;Copy=$null;DispatchState='NotDispatched'},
        [pscustomobject]@{Index=1;Source=$good;Sink=$out;Copy=$null;DispatchState='NotDispatched'})
        Copies=(New-Object Collections.Generic.List[object]);Sinks=(New-Object Collections.Generic.List[object])
        Errors=(New-Object Collections.Generic.List[string])}
    try{
        Start-RemovalDiagnosticEntryCopies $owner;Start-RemovalDiagnosticEntryCopies $owner
        Check ($source.Calls-eq1-and$owner.Channels[0].DispatchState-ceq'Unknown'-and$null-eq$owner.Channels[0].Copy) 'Ambiguous copy call was repeated or invented.'
        Check ($owner.Channels[1].DispatchState-ceq'Dispatched'-and$null-ne$owner.Channels[1].Copy-and$owner.Errors.Count-gt0) 'Other original channel did not progress independently.'
        $null=$owner.Channels[1].Copy.GetAwaiter().GetResult()
        Check ($out.Length-eq3) 'Other channel bytes did not drain.'
    }finally{$good.Dispose();$out.Dispose()}
}
Case 'exit-zero-valid-original-stdout-pin-is-retained-without-qualification' {
    $f=New-Fixture '$output=MakeAck;[Console]::Write(($output|ConvertTo-Json -Depth 16 -Compress));exit 0'
    $r=Read-Terminal $f (Invoke-Fixture $f)
    Check ($r.ExitCode-eq0-and$r.OutputStable-and$r.Errors.Count-eq0-and$null-ne$r.LocalAcknowledgementPin) 'Normal closed stdout envelope was refused.'
    Check ($r.LocalAcknowledgementPin.Sha256-ceq(Pin ($f.Run+'.archive-local.json')).Sha256) 'Normal stdout ack bytes differ.'
}
Case 'no-reboot-confirmation-refuses-before-guard-or-writes' {
    $f=New-Fixture 'exit 0';$before=$script:guardCalls;Reject {Invoke-Fixture $f -NoConfirmation}
    Check ($script:guardCalls-eq$before-and@(Get-ChildItem -LiteralPath ([IO.Path]::GetDirectoryName($f.Run)) -Filter 'run*').Count-eq0) 'Confirmation refusal wrote/dispatched.'
}
Case 'strict-duplicate-output-and-mismatched-ack-never-produce-delivery-pin' {
    foreach($body in @('$output=MakeAck;$json=$output|ConvertTo-Json -Depth 16 -Compress;[Console]::Write($json.Replace(''"Qualified":false'',''"Qualified":false,"Qualified":false''));exit 1',
        '$output=MakeAck;$output.LocalAcknowledgement.Sha256="0"*64;[Console]::Write(($output|ConvertTo-Json -Depth 16 -Compress));exit 1',
        '$output=MakeAck;$output.Archive.Nonce="f"*32;[Console]::Write(($output|ConvertTo-Json -Depth 16 -Compress));exit 1')){
        $f=New-Fixture $body;$r=Read-Terminal $f (Invoke-Fixture $f)
        Check ($null-eq$r.LocalAcknowledgementPin-and$r.Errors.Count-gt0) 'Unsafe output/ack accepted.'
    }
}
Case 'unexpected-exit-code-is-recorded-without-delivery-authority' {
    $f=New-Fixture '$output=MakeAck;[Console]::Write(($output|ConvertTo-Json -Depth 16 -Compress));exit 7'
    $r=Read-Terminal $f (Invoke-Fixture $f);Check ($r.ExitCode-eq7-and$r.Errors.Count-gt0-and$null-eq$r.LocalAcknowledgementPin) 'Unexpected exit hidden.'
}
Case 'stdout-original-pin-is-not-replaced-by-current-ack-file' {
    foreach($body in @('$output=MakeAck;[IO.File]::AppendAllText(($RunDirectory+".archive-local.json")," ");[Console]::Write(($output|ConvertTo-Json -Depth 16 -Compress));exit 1',
        '$output=MakeAck;[Console]::Write("{}");exit 1')){
        $f=New-Fixture $body;$r=Read-Terminal $f (Invoke-Fixture $f)
        Check ($null-eq$r.LocalAcknowledgementPin-and$r.Errors.Count-gt0) 'Current ack substituted for original stdout pin.'
    }
}
Case 'existing-entry-output-refuses-without-overwrite-or-second-child' {
    $f=New-Fixture 'exit 0';Write-Text ($f.Run+'.entry-intent.json') 'preserved'
    Reject {Invoke-Fixture $f};Check ([IO.File]::ReadAllText(($f.Run+'.entry-intent.json'))-ceq'preserved') 'Existing intent overwritten.'
    Check (-not[IO.File]::Exists(($f.Run+'.entry-terminal.json'))) 'Existing attempt gained false terminal.'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' REAL_CHILDREN=harmless HOST_GUARD=test-only NATIVE_OPERATIONS=0')
Write-Output ('FIXTURE_ROOT='+$script:fixtureRoot)
if($failed){exit 1}
