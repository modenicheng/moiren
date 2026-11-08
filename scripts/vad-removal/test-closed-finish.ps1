#requires -Version 5.1
# Production Finish -> original controller seal -> ZIP/local ack. Scheduler,
# trace APIs and worker/probe metadata are synthetic; decoder children are the
# existing harmless PowerShell/offline-reader fixtures, never audio/native work.
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){throw 'Run closed-finish composition only on the development host.'}
. (Join-Path $PSScriptRoot 'diagnostic-controller.ps1')
. (Join-Path $PSScriptRoot 'closed-export.ps1')
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$refused=$false;try{&$Action|Out-Null}catch{$refused=$true};Check $refused 'Expected original-owner refusal.'}
function Case([string]$Name,[scriptblock]$Action){
    try{&$Action;$checks.Add(@{name=$Name;passed=$true})}
    catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message;at=$_.ScriptStackTrace})}
}
Case 'actual-finish-seal-and-export-apis-load-with-release-still-closed' {
    foreach($name in @('Save-RemovalControllerResult','Get-RemovalClosedControllerSeal','New-RemovalManagedFinish',
        'Observe-RemovalManagedFinish','Export-RemovalClosedRun','Confirm-RemovalClosedRunExport')){
        Check ($null-ne(Get-Command $name -ErrorAction SilentlyContinue)) ('Missing production composition API: '+$name)
    }
    Reject {Assert-RemovalDiagnosticRelease}
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_OPERATIONS=0';exit 1}
. (Join-Path $PSScriptRoot 'test-trace-decode-fixtures.ps1')
$evidence=Join-Path $PSScriptRoot 'logs\closed-finish'
$null=[IO.Directory]::CreateDirectory($evidence)
$script:fixtureRoot=Join-Path $evidence ('finish-'+$PSVersionTable.PSVersion.Major+'-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($script:fixtureRoot)

function Import-ClosedFinishHelpers([string]$Path,[string[]]$Names){
    $tokens=$null;$errors=$null
    $ast=[Management.Automation.Language.Parser]::ParseFile($Path,[ref]$tokens,[ref]$errors)
    if($errors.Count){throw 'Existing shared finish fixture did not parse.'}
    foreach($name in $Names){
        $nodes=@($ast.EndBlock.Statements|Where-Object{$_-is[Management.Automation.Language.FunctionDefinitionAst]-and$_.Name-ceq$name})
        if($nodes.Count-ne1){throw ('Unique existing finish helper required: '+$name)}
        $body=$nodes[0].Body.Extent.Text.Substring(1,$nodes[0].Body.Extent.Text.Length-2)
        if($nodes[0].Parameters.Count){$body='param('+ (($nodes[0].Parameters|ForEach-Object{$_.Extent.Text})-join',') +')'+"`n"+$body}
        Set-Item -Path ('Function:script:'+ $name) -Value ([scriptblock]::Create($body))
    }
}
# Load helper definitions only: never execute the unrelated finish suite.
Import-ClosedFinishHelpers (Join-Path $PSScriptRoot 'test-finish-session.ps1') @('New-FinishFixture','Attach-Decode')

# Keep synthetic EXE/TMF pins outside the archived run. Adapt only these two
# fixture output locations; all staging, native-owner and managed binding code
# remains the existing shared fixture, selected in $script:fixture.
$fixtureBody=${function:New-Fixture}.ToString().Replace('$PSScriptRoot',("'"+$PSScriptRoot.Replace("'","''")+"'"))
$oldRun='$run=Join-Path $root ''managed-run'''
$newRun='$run=Join-Path $root ''run\managed'''
$oldEtl='$etl=Join-Path $root ''driver.etl'';[IO.File]::WriteAllBytes'
$newEtl='$null=[IO.Directory]::CreateDirectory((Join-Path $root ''run''));$etl=Join-Path $root ''run\driver.etl'';[IO.File]::WriteAllBytes'
foreach($old in @($oldRun,$oldEtl)){
    if($fixtureBody.IndexOf($old)-lt0-or$fixtureBody.IndexOf($old)-ne$fixtureBody.LastIndexOf($old)){
        throw 'Exactly one shared fixture run/ETL location required; no broad source rewrite.'
    }
}
Set-Item Function:New-Fixture ([scriptblock]::Create($fixtureBody.Replace($oldRun,$newRun).Replace($oldEtl,$newEtl)))
function Assert-RemovalTaskCleanupApproval {
    param($Owner,[string]$Operation)
    Check ($Operation-in@('ManagedRetire','TaskRetire')) 'Unexpected cleanup operation in host fixture.'
    $expected=if($Operation-ceq'ManagedRetire'){$script:fixture.ManagedSession}else{$script:fixture.ManagedSession.TaskOwner}
    Check ([Object]::ReferenceEquals($Owner,$expected)) 'Cleanup did not retain original managed/task owner.'
}

function New-ClosedFinishFixture([string]$Mode='normal'){
    $f=New-FinishFixture
    $f.Mode=$Mode
    $manifest=Join-Path $f.Root 'fixture-inputs.json'
    Write-Text $manifest '{"Fixture":true,"NoNativeAuthority":true}'
    $inputs=[pscustomobject]@{SourceDirectory=$PSScriptRoot;ManifestPin=(Pin $manifest);Pins=@{Tools=$null;Sources=@()}}
    $run=Join-Path $f.Root 'run';$beforeDirectory=Join-Path $run 'before'
    $null=[IO.Directory]::CreateDirectory($beforeDirectory)
    $baseline=Join-Path $beforeDirectory 'fixture-baseline.json'
    Write-Text $baseline '{"Fixture":true,"ExecutionMode":"InjectedCollectorSource","Qualified":false}'
    $pin=Pin $baseline
    $before=[pscustomobject]@{ReceiptPin=$pin;RetainedPreflight=[pscustomobject]@{ReceiptPin=$pin
        RetainedOwner=$null;StartAttempted=$false;ExecutionMode='InjectedCollectorSource'}}
    $lease=New-RemovalDiagnosticLease -Path (Join-Path $f.Root 'controller.lock')
    $self=[Diagnostics.Process]::GetCurrentProcess();$nonce=$f.ManagedSession.TaskOwner.Spec.Nonce
    # This is an explicit constructor-shaped root fixture, not execution of
    # New/Start/Observe diagnostic or real BEFORE admission. Register/bind/save/
    # seal/getter are unmodified production CWT lifetime functions throughout.
    $config=ConvertTo-Json -InputObject ([ordered]@{RunDirectory=$run;Nonce=$nonce;ControllerPid=$self.Id
        ControllerCreationUtcTicks=$self.StartTime.ToUniversalTime().Ticks;WorkerSessionId=2}) -Compress
    $o=[pscustomobject]@{RunDirectory=$run;OriginalDirectory=$run;Nonce=$nonce;ControllerPid=$self.Id
        ControllerCreationUtcTicks=$self.StartTime.ToUniversalTime().Ticks;WorkerSessionId=2;ConfigJson=$config
        Inputs=$inputs;RetainedInputs=$inputs;Lease=$lease;RetainedLease=$lease;BeforeReadiness=$before;RetainedBefore=$before
        TraceSession=$f.Session;RetainedTrace=$f.Session;OriginalNativeOwner=$f.Session.NativeOwner;OriginalReadback=$f.Session.Readback
        ManagedSession=$null;RetainedManaged=$null;Finish=$null;RetainedFinish=$null;AfterAudit=$null;RetainedAfter=$null
        BeforeSettled=$true;AbortEvidencePin=$null;ResultAttempted=$false;ResultPin=$null;Completed=$false;Phase='Finish'
        Safe=$false;TimedOut=$false;Errors=(New-Object Collections.Generic.List[string]);NativePassed=$false;Qualified=$false}
    Register-RemovalControllerLifetime $o
    Acquire-RemovalDiagnosticLease $lease;Bind-RemovalControllerLifetime $o 'Writer'
    $probe=$f.ManagedSession.ProbeOwner;$f.ManagedSession.ProbeOwner=$null
    $o.ManagedSession=$f.ManagedSession;$o.RetainedManaged=$f.ManagedSession
    Bind-RemovalControllerLifetime $o 'Managed' $f.ManagedSession
    $f.ManagedSession.ProbeOwner=$probe;Bind-RemovalControllerLifetime $o 'Probe' $probe
    $o.Finish=$f.Finish;$o.RetainedFinish=$f.Finish;Bind-RemovalControllerLifetime $o 'Finish' $f.Finish
    $f.ClosedController=$o
    return [pscustomobject]@{Fixture=$f;Controller=$o}
}

function Complete-ClosedFinishFixture($Pair){
    $f=$Pair.Fixture;$h=$f.Finish
    $r=Observe-RemovalManagedFinish $h -ObservationMilliseconds 100
    Check ($r.Phase-ceq'AwaitingDecodePins'-and$f.Session.NativeOwner.TraceStopped) 'Finish did not retain confirmed STOP before decode.'
    Set-RemovalManagedFinishDecode -Finish $h -Pins $f.Pins -OutputDirectory (Join-Path $Pair.Controller.RunDirectory 'decode') `
        -ToolBoundary (New-ToolBoundary)
    for($i=0;$i-lt50;$i++){
        $r=Observe-RemovalManagedFinish $h -ObservationMilliseconds 100
        if($r.Terminal){break}
    }
    Check ($r.Terminal-and$r.TaskRetired-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq1-and
        $f.ManagedSession.TaskOwner.Folder.EvidenceBeforeDelete) ('Actual Finish not retired: '+($h.Errors-join'; '))
    foreach($owner in $h.RetainedDecode.ToolOwners){Check $owner.Closed 'Original decoder owner remained pending.'}
    return $r
}
function Close-ClosedFinishFixture($Pair){
    if($null-eq$Pair){return}
    $h=$Pair.Fixture.Finish
    $settled=Observe-RemovalFinishTools $h 15000
    Check $settled 'Original finite tools unresolved; retain fixture ownership.'
    if(-not$Pair.Controller.RetainedLease.Closed){Release-RemovalDiagnosticLease $Pair.Controller.RetainedLease}
}

foreach($mode in @('normal','reader-refusal')){
    Case ('retired-finish-real-seal-whole-archive-local-ack-'+$mode) {
        $pair=$null
        try{
            $pair=New-ClosedFinishFixture $mode;$o=$pair.Controller;$f=$pair.Fixture
            Reject {Get-RemovalClosedControllerSeal $o}
            $writer=$o.RetainedLease.RetainedFile;$handle=$o.RetainedLease.RetainedHandle
            Check ($writer-is[IO.FileStream]-and$writer.CanWrite-and-not$handle.IsClosed) 'Original writer was not live before Finish.'
            $r=Complete-ClosedFinishFixture $pair
            Check (-not$r.NativePassed-and-not$r.Qualified) 'Host fixture promoted native success.'
            if($mode-ceq'reader-refusal'){Check ($f.Finish.RetainedDecode.Failed-and-not$r.AssociationAvailable) 'Expected real decode refusal lost.'}
            else{Check $r.AssociationAvailable 'Existing offline association did not run.'}
            Save-RemovalControllerResult $o
            Check ($o.Completed-and$o.Phase-ceq'Complete'-and-not$o.Safe-and$o.RetainedLease.Closed-and
                -not$o.RetainedLease.Owned-and$handle.IsClosed-and-not$writer.CanWrite) 'Actual result save did not persist refusal and close original writer.'
            $seal=Get-RemovalClosedControllerSeal $o
            $important=@($o.ResultPin,$f.Finish.ResultPin,$f.Finish.ReceiptPin,$f.Finish.RetainedDecode.ReceiptPin)
            foreach($pin in $important){Check (@($seal.TerminalPins|Where-Object{$_.Path-ceq$pin.Path-and$_.Sha256-ceq$pin.Sha256}).Count-eq1) ('Seal omitted retained Finish/result pin: '+$pin.Path)}
            Check (@($seal.TerminalPins|Where-Object{$_.Path-ceq$f.ManagedSession.TaskOwner.RetiredXmlPath-and
                $_.Sha256-ceq$f.ManagedSession.TaskOwner.RetiredXmlSha256}).Count-eq1) 'Retired task XML was not sealed.'
            $export=Export-RemovalClosedRun -Controller $o -ArchivePath ($o.RunDirectory+'.zip')
            $ack=Confirm-RemovalClosedRunExport -Export $export -AcknowledgementPath ($o.RunDirectory+'.archive-local.json')
            $saved=[IO.File]::ReadAllText($ack.Path)|ConvertFrom-Json
            Check ($export.LocalVerified-and$saved.Nonce-ceq$o.Nonce-and$saved.ResultPin.Sha256-ceq$o.ResultPin.Sha256-and
                $saved.ArchivePin.Sha256-ceq$export.ArchivePin.Sha256-and$saved.Meaning-ceq'LOCAL_ARCHIVE_VERIFICATION_ONLY'-and
                -not$saved.Transferred-and-not$saved.NativePassed-and-not$saved.Qualified) 'Real sealed archive/local acknowledgement binding differs.'
            $stream=[IO.File]::OpenRead($export.ArchivePin.Path)
            $zip=[IO.Compression.ZipArchive]::new($stream,[IO.Compression.ZipArchiveMode]::Read,$false)
            try{
                foreach($pin in $seal.TerminalPins){
                    $name='run/'+$pin.Path.Substring($o.RunDirectory.Length+1).Replace('\','/')
                    $entry=$zip.GetEntry($name);Check ($null-ne$entry-and$entry.Length-eq$pin.Bytes) ('Terminal evidence omitted from actual ZIP: '+$name)
                }
                $entry=$zip.GetEntry('export/provenance.json');$reader=[IO.StreamReader]::new($entry.Open())
                try{$p=$reader.ReadToEnd()|ConvertFrom-Json}finally{$reader.Dispose()}
                Check ($p.CollectorProvenance-ceq'InjectedCollectorSource'-and$p.InputJson-ceq$seal.InputJson-and
                    $p.ResultJson-ceq$seal.ResultJson) 'Archived provenance invented native baseline or new input snapshot.'
            }finally{$zip.Dispose();$stream.Dispose()}
            $counts=@($f.Calls.Count,$f.ManagedSession.TaskOwner.Folder.DeleteCalls)
            $null=Get-RemovalClosedControllerSeal $o
            Check ($f.Calls.Count-eq$counts[0]-and$f.ManagedSession.TaskOwner.Folder.DeleteCalls-eq$counts[1]) 'Post-retirement seal repeated tool/retirement operations.'
        }finally{Close-ClosedFinishFixture $pair}
    }
}

foreach($owner in $script:realOwners){
    if(-not$owner.Closed){
        $r=Observe-FiniteTool $owner -ObservationMilliseconds 15000
        Check ($r.ExitedConfirmed-and-not$r.NeedsObservation) 'Original harmless decoder child is retained, not killed.'
        Close-FiniteToolOwner $owner
    }
}
$checks|ConvertTo-Json -Depth 7
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' FINISH=production CONTROLLER_SEAL=production WRITER=FileStream ARCHIVE=production BASELINE=synthetic SCHEDULER=mock TRACE_APIS=injected NATIVE_OPERATIONS=0')
Write-Output ('FIXTURE_ROOT='+$script:fixtureRoot)
if($failed){exit 1}
