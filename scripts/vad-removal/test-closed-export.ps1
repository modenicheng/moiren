#requires -Version 5.1
# Host-only filesystem composition. The controller-seal getter is synthetic;
# archive, independent readback, retained export association and ack are real.
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){throw 'Run closed-export tests only on the development host.'}
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action,[string]$Reason=''){
    $message=$null;try{&$Action|Out-Null}catch{$message=$_.Exception.Message}
    Check ($null-ne$message) 'Expected refusal.'
    if($Reason){Check ($message-like('*'+$Reason+'*')) ('Wrong refusal: '+$message)}
}
function Case([string]$Name,[scriptblock]$Action){
    try{&$Action;$checks.Add(@{name=$Name;passed=$true})}
    catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}
}
$library=Join-Path $PSScriptRoot 'closed-export.ps1'
Case 'closed-run-export-api-exists-without-dispatch' {
    Check (Test-Path -LiteralPath $library) 'Closed-run export not implemented.'
    . $library
    foreach($name in @('Export-RemovalClosedRun','Confirm-RemovalClosedRunExport')){
        Check ($null-ne(Get-Command $name -ErrorAction SilentlyContinue)) ('Missing API '+$name)
    }
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_OS_CALLS=0';exit 1}
. $library
$evidence=Join-Path $PSScriptRoot 'logs\closed-export'
$null=[IO.Directory]::CreateDirectory($evidence)
$root=Join-Path $evidence ('fixtures-'+$PSVersionTable.PSVersion.Major+'-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$script:TestClosedSeals=New-Object 'Runtime.CompilerServices.ConditionalWeakTable[object,object]'
function Get-RemovalClosedControllerSeal($Controller){
    $seal=$null
    if(-not$script:TestClosedSeals.TryGetValue($Controller,[ref]$seal)){throw 'No synthetic original terminal seal.'}
    return $seal
}
function Write-Text([string]$Path,[string]$Text){
    $null=[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Path))
    [IO.File]::WriteAllText($Path,$Text,[Text.UTF8Encoding]::new($false))
}
function Pin([string]$Path){
    [pscustomobject]@{Path=$Path;Bytes=(Get-Item -LiteralPath $Path).Length;Sha256=(Get-FileHash -LiteralPath $Path).Hash}
}
function New-Fixture {
    $dir=Join-Path $root ([Guid]::NewGuid().ToString('N'));$run=Join-Path $dir 'run'
    $null=[IO.Directory]::CreateDirectory($run)
    $nonce=[Guid]::NewGuid().ToString('N')
    $resultJson=ConvertTo-Json -InputObject ([ordered]@{SchemaVersion=1;ControllerComplete=$true;Safe=$false
        Nonce=$nonce;NativePassed=$false;Qualified=$false;Errors=@('Synthetic failed enable')}) -Compress
    $result=Join-Path $run 'controller-result.json';Write-Text $result $resultJson
    $before=Join-Path $run 'before\preflight-receipt.json';Write-Text $before '{"Ready":false,"NativeApproved":false}'
    Write-Text (Join-Path $run 'before\stderr.log') ''
    Write-Text (Join-Path $run 'after\safety-audit.json') '{"Complete":true,"Safe":false}'
    $null=[IO.Directory]::CreateDirectory((Join-Path $run 'empty'))
    $staged=@();$sources=@()
    foreach($name in @('finite-tool.ps1','task-boundary.ps1','probe-boundary.ps1','worker-boundary.ps1','worker.ps1')){
        $path=Join-Path $run ('managed\'+$name);Write-Text $path ('# pinned test stage '+$name)
        $staged+=,(Pin $path)
        $external=Join-Path $dir ('sources\'+$name);Write-Text $external ('# original source '+$name)
        $source=Pin $external;$source|Add-Member NoteProperty Name $name;$sources+=,$source
    }
    $manifest=Join-Path $dir 'manifest.json';Write-Text $manifest '{"ExternalExecutable":"C:\\pinned\\probe.exe"}'
    $seal=[pscustomobject]@{Directory=$run;Nonce=$nonce;ResultPin=(Pin $result);ResultJson=$resultJson
        ManifestPin=(Pin $manifest);InputJson='{"Pins":{"Probe":{"Path":"C:\\pinned\\probe.exe"}},"Retained":true}'
        SourcePins=$sources;CollectorProvenance='InjectedCollectorSource';StagedScriptPins=$staged
        TerminalPins=@((Pin $result),(Pin $before))}
    $controller=[pscustomobject]@{Completed=$true;Safe=$false;NativePassed=$false;Qualified=$false}
    $script:TestClosedSeals.Add($controller,$seal)
    [pscustomobject]@{Directory=$dir;Run=$run;Controller=$controller;Seal=$seal
        Archive=(Join-Path $dir 'run.zip');Ack=(Join-Path $dir 'run.ack.json')}
}
function Read-Zip([string]$Path){
    $stream=[IO.File]::OpenRead($Path)
    $zip=[IO.Compression.ZipArchive]::new($stream,[IO.Compression.ZipArchiveMode]::Read,$false)
    try{
        $rows=@()
        foreach($entry in $zip.Entries){
            $input=$entry.Open();$copy=New-Object IO.MemoryStream
            try{$input.CopyTo($copy);$rows+=,[pscustomobject]@{Name=$entry.FullName;Bytes=$copy.ToArray()}}
            finally{$copy.Dispose();$input.Dispose()}
        }
        return ,$rows
    }finally{$zip.Dispose();$stream.Dispose()}
}
function Rewrite-Zip([string]$Path,[scriptblock]$Change){
    $file=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    $zip=[IO.Compression.ZipArchive]::new($file,[IO.Compression.ZipArchiveMode]::Update,$false)
    try{&$Change $zip}finally{$zip.Dispose();$file.Dispose()}
}
Case 'completed-refused-run-whole-tree-archive-and-local-ack' {
    $f=New-Fixture
    $x=Export-RemovalClosedRun -Controller $f.Controller -ArchivePath $f.Archive
    Check ($x.LocalVerified-and-not$x.NativePassed-and-not$x.Qualified) 'Export promoted diagnostic acceptance.'
    $rows=Read-Zip $f.Archive
    Check (@($rows|Where-Object{$_.Name-ceq'run/empty/'}).Count-eq1) 'Empty original directory omitted.'
    Check (@($rows|Where-Object{$_.Name-ceq'run/before/stderr.log'}).Count-eq1) 'Empty failed diagnostic omitted.'
    $provenance=@($rows|Where-Object{$_.Name-ceq'export/provenance.json'})
    Check ($provenance.Count-eq1) 'Original input provenance missing.'
    $p=[Text.Encoding]::UTF8.GetString($provenance[0].Bytes)|ConvertFrom-Json
    Check ($p.InputJson-ceq$f.Seal.InputJson-and$p.CollectorProvenance-ceq'InjectedCollectorSource'-and
        $p.ResultJson-ceq$f.Seal.ResultJson) 'Original sealed inputs/result/provenance lost.'
    Check ($rows.Count-eq16) ('Whole tree plus metadata membership differs: '+$rows.Count)
    $ack=Confirm-RemovalClosedRunExport -Export $x -AcknowledgementPath $f.Ack
    $saved=[IO.File]::ReadAllText($ack.Path)|ConvertFrom-Json
    Check ($saved.Meaning-ceq'LOCAL_ARCHIVE_VERIFICATION_ONLY'-and$saved.Nonce-ceq$f.Seal.Nonce-and
        $saved.ResultPin.Sha256-ceq$f.Seal.ResultPin.Sha256-and$saved.ArchivePin.Sha256-ceq$x.ArchivePin.Sha256-and
        $saved.InventoryPin.Sha256-ceq$x.InventoryPin.Sha256-and-not$saved.Transferred-and-not$saved.Qualified-and
        -not$saved.NativePassed) 'Ack lacks original archive/result/inventory binding or claims transfer.'
    Check (-not$f.Controller.Safe-and-not$f.Controller.Qualified) 'Original refused result was changed.'
}
Case 'external-manifest-source-changes-do-not-reopen-failed-run-admission' {
    $f=New-Fixture;Write-Text $f.Seal.ManifestPin.Path 'changed after diagnostic failure'
    foreach($pin in $f.Seal.SourcePins){Write-Text $pin.Path 'changed source after cleanup'}
    $x=Export-RemovalClosedRun $f.Controller $f.Archive
    $null=Confirm-RemovalClosedRunExport $x $f.Ack
    Check $x.LocalVerified 'Historical provenance incorrectly reread external sources.'
}
Case 'actual-probe-f32le-capture-and-stop-artifacts-are-retained' {
    $f=New-Fixture
    foreach($name in @('capture.f32le','measurement.f32le','vad-input-reference.f32le','controlled-load.stop')){
        $path=Join-Path $f.Run ('managed\artifacts\'+$name)
        Write-Text $path ('host synthetic bytes for '+$name)
        $f.Seal.TerminalPins+=,(Pin $path)
    }
    $x=Export-RemovalClosedRun $f.Controller $f.Archive
    $rows=Read-Zip $f.Archive
    foreach($name in @('capture.f32le','measurement.f32le','vad-input-reference.f32le','controlled-load.stop')){
        Check (@($rows|Where-Object{$_.Name-ceq('run/managed/artifacts/'+$name)}).Count-eq1) ('Actual probe artifact omitted: '+$name)
    }
    $null=Confirm-RemovalClosedRunExport $x $f.Ack
}
Case 'detached-root-and-caller-completed-flags-do-not-grant-export' {
    $f=New-Fixture
    Reject {Export-RemovalClosedRun ([pscustomobject]@{Completed=$true;Safe=$true;ResultPin=$f.Seal.ResultPin}) $f.Archive} 'seal'
    Check (-not[IO.File]::Exists($f.Archive)) 'Detached root created archive.'
}
Case 'missing-or-mutated-terminal-pin-refuses-before-output' {
    foreach($mode in @('missing','hash','result-json')){
        $f=New-Fixture
        if($mode-ceq'missing'){$f.Seal.TerminalPins+=,[pscustomobject]@{Path=(Join-Path $f.Run 'missing.json');Bytes=1;Sha256=('A'*64)}}
        elseif($mode-ceq'hash'){Write-Text $f.Seal.TerminalPins[1].Path 'changed terminal receipt'}
        else{$f.Seal.ResultJson+=' '}
        Reject {Export-RemovalClosedRun $f.Controller $f.Archive}
        Check (-not[IO.File]::Exists($f.Archive)) ('Invalid '+$mode+' terminal created archive.')
    }
}
Case 'only-exact-five-staged-script-pins-allow-executable-content' {
    foreach($mode in @('extra-script','changed-stage','exe','renamed-pe')){
        $f=New-Fixture
        switch($mode){
            'extra-script'{Write-Text (Join-Path $f.Run 'other.ps1') '# not staged'}
            'changed-stage'{Write-Text $f.Seal.StagedScriptPins[0].Path '# changed stage'}
            'exe'{Write-Text (Join-Path $f.Run 'probe.exe') 'not an exe but prohibited copy'}
            'renamed-pe'{[IO.File]::WriteAllBytes((Join-Path $f.Run 'hidden.log'),[byte[]]@(77,90,0,0))}
        }
        Reject {Export-RemovalClosedRun $f.Controller $f.Archive}
        Check (-not[IO.File]::Exists($f.Archive)) ('Executable tree copied: '+$mode)
    }
}
Case 'caps-bound-depth-member-count-single-file-and-total' {
    foreach($mode in @('depth','count','file','total')){
        $f=New-Fixture
        switch($mode){
            'depth'{Write-Text (Join-Path $f.Run 'a\b\c\d\e\f\g\h\i.log') 'too deep'}
            'count'{for($i=0;$i-lt512;$i++){Write-Text (Join-Path $f.Run ('member-'+$i+'.log')) ''}}
            'file'{$s=[IO.File]::Create((Join-Path $f.Run 'oversize.etl'));try{$s.SetLength(16777217)}finally{$s.Dispose()}}
            'total'{for($i=0;$i-lt4;$i++){$s=[IO.File]::Create((Join-Path $f.Run ('capture-'+$i+'.raw')));try{$s.SetLength(16777216)}finally{$s.Dispose()}}}
        }
        Reject {Export-RemovalClosedRun $f.Controller $f.Archive}
        Check (-not[IO.File]::Exists($f.Archive)) ('Cap '+$mode+' created archive.')
    }
}
Case 'create-new-sibling-output-no-overwrite-or-in-run-export' {
    $f=New-Fixture;Write-Text $f.Archive 'preserve original'
    Reject {Export-RemovalClosedRun $f.Controller $f.Archive}
    Check ([IO.File]::ReadAllText($f.Archive)-ceq'preserve original') 'Archive overwritten.'
    $g=New-Fixture;Reject {Export-RemovalClosedRun $g.Controller (Join-Path $g.Run 'inside.zip')}
    $h=New-Fixture;Write-Text ($h.Archive+'.inventory.json') 'existing sidecar'
    Reject {Export-RemovalClosedRun $h.Controller $h.Archive}
    Check (-not[IO.File]::Exists($h.Archive)) 'Known sidecar collision left avoidable archive.'
}
Case 'original-export-reference-required-and-single-ack-no-retry' {
    $f=New-Fixture;$x=Export-RemovalClosedRun $f.Controller $f.Archive
    $clone=[pscustomobject]@{};foreach($p in $x.PSObject.Properties){$clone|Add-Member NoteProperty $p.Name $p.Value}
    Reject {Confirm-RemovalClosedRunExport $clone $f.Ack}
    $null=Confirm-RemovalClosedRunExport $x $f.Ack
    Reject {Confirm-RemovalClosedRunExport $x (Join-Path $f.Directory 'retry.ack.json')}
    Reject {Export-RemovalClosedRun $f.Controller (Join-Path $f.Directory 'retry.zip')}
}
Case 'ack-rechecks-archive-sidecar-result-and-whole-run-membership' {
    foreach($mode in @('archive','inventory','result','member','addition')){
        $f=New-Fixture;$x=Export-RemovalClosedRun $f.Controller $f.Archive
        switch($mode){
            'archive'{Rewrite-Zip $f.Archive {param($zip) $e=$zip.CreateEntry('unexpected.log');$s=$e.Open();try{$s.WriteByte(65)}finally{$s.Dispose()}}}
            'inventory'{Write-Text ($f.Archive+'.inventory.json') '{}'}
            'result'{Write-Text $f.Seal.ResultPin.Path 'changed result'}
            'member'{Write-Text (Join-Path $f.Run 'before\stderr.log') 'late mutation'}
            'addition'{Write-Text (Join-Path $f.Run 'late.log') 'late member'}
        }
        Reject {Confirm-RemovalClosedRunExport $x $f.Ack}
        Check (-not[IO.File]::Exists($f.Ack)) ('Ack accepted '+$mode+' mutation.')
    }
}
Case 'caller-export-projection-mutation-cannot-repin-private-archive' {
    $f=New-Fixture;$x=Export-RemovalClosedRun $f.Controller $f.Archive
    $x.ArchivePin.Sha256='F'*64
    Reject {Confirm-RemovalClosedRunExport $x $f.Ack}
    Check (-not[IO.File]::Exists($f.Ack)) 'Caller-mutated projection minted ack.'
}
Case 'independent-readback-rejects-missing-duplicate-and-changed-members' {
    foreach($mode in @('missing','duplicate','bytes')){
        $f=New-Fixture;$x=Export-RemovalClosedRun $f.Controller $f.Archive
        Rewrite-Zip $f.Archive {
            param($zip)
            if($mode-ceq'missing'){$zip.GetEntry('run/before/stderr.log').Delete()}
            elseif($mode-ceq'duplicate'){$null=$zip.CreateEntry('run/before/stderr.log')}
            else{$entry=$zip.GetEntry('run/controller-result.json');$s=$entry.Open();try{$s.Position=0;$s.WriteByte(88)}finally{$s.Dispose()}}
        }
        Reject {Confirm-RemovalClosedRunExport $x $f.Ack} 'archive'
        Check (-not[IO.File]::Exists($f.Ack)) ('Independent readback accepted '+$mode+' member.')
    }
}
Case 'failed-readback-preserves-partial-output-and-refuses-reexport' {
    $f=New-Fixture
    $original=(Get-Command Test-RemovalClosedExportArchive).ScriptBlock
    try{
        function Test-RemovalClosedExportArchive($State){throw 'TEST-ONLY independent readback failure after archive persistence'}
        Reject {Export-RemovalClosedRun $f.Controller $f.Archive} 'independent readback failure'
    }finally{Set-Item Function:Test-RemovalClosedExportArchive $original}
    Check ([IO.File]::Exists($f.Archive)-and[IO.File]::Exists($f.Archive+'.inventory.json')) 'Failure deleted persisted output.'
    $pin=Pin $f.Archive
    Reject {Export-RemovalClosedRun $f.Controller (Join-Path $f.Directory 'retry.zip')} 'already attempted'
    Check ((Pin $f.Archive).Sha256-ceq$pin.Sha256) 'Retry changed preserved archive.'
}
Case 'reparse-and-noncanonical-paths-refuse-without-following' {
    $f=New-Fixture
    # A junction is only a host filesystem fixture; never traverse or remove it.
    $outside=Join-Path $f.Directory 'outside';$null=[IO.Directory]::CreateDirectory($outside)
    $null=New-Item -ItemType Junction -Path (Join-Path $f.Run 'linked') -Target $outside
    Reject {Export-RemovalClosedRun $f.Controller $f.Archive} 'Reparse'
    $g=New-Fixture
    Reject {Export-RemovalClosedRun $g.Controller (Join-Path $g.Directory 'sub\..\bad.zip')}
    Reject {Export-RemovalClosedRun $g.Controller ($g.Archive+':alternate')}
}
$checks|ConvertTo-Json -Depth 7
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' SEAL=TEST-ONLY-synthetic ARCHIVE=production ACK=LOCAL_ONLY NATIVE_OS_CALLS=0')
Write-Output ('FIXTURE_ROOT='+$root)
if($failed){exit 1}
