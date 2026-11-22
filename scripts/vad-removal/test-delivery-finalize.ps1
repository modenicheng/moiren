#requires -Version 5.1
# Host-only durable-file composition. Only the target host/SID guard is replaced;
# ZIP payloads, host receipt bytes, independent pins and target ack are real IO.
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName-ceq'LAPTOP-NP49RG6H'){throw 'Run delivery-finalize tests only on the development host.'}
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action,[string]$Reason=''){
    $message=$null;try{&$Action|Out-Null}catch{$message=$_.Exception.Message}
    Check ($null-ne$message) 'Expected delivery refusal.'
    if($Reason){Check ($message-like('*'+$Reason+'*')) ('Wrong refusal: '+$message)}
}
function Case([string]$Name,[scriptblock]$Action){
    try{&$Action;$checks.Add(@{name=$Name;passed=$true})}
    catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message;at=$_.ScriptStackTrace})}
}
$library=Join-Path $PSScriptRoot 'delivery-finalize.ps1'
Case 'durable-target-finalizer-loads-without-dispatch' {
    Check (Test-Path -LiteralPath $library) 'Durable target delivery finalizer is missing.'
    . $library
    Check ($null-ne(Get-Command Confirm-RemovalRunDelivery -ErrorAction SilentlyContinue)) 'Target finalization API missing.'
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_OPERATIONS=0';exit 1}
. $library
$actualGuard=${function:Assert-RemovalDeliveryTargetHost}
function Assert-RemovalDeliveryTargetHost([string]$RunDirectory){
    Check ($RunDirectory.StartsWith($script:fixtureRoot+'\',[StringComparison]::Ordinal)) 'Synthetic target escaped fixture root.'
    $script:guardCalls++
}
$script:guardCalls=0
$script:fixtureRoot=Join-Path $PSScriptRoot ('logs\delivery-finalize\'+$PSVersionTable.PSVersion.Major+'-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($script:fixtureRoot)
function Write-Text([string]$Path,[string]$Text){
    $null=[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Path))
    [IO.File]::WriteAllText($Path,$Text,[Text.UTF8Encoding]::new($false))
}
function Json($Value){ConvertTo-Json -InputObject $Value -Depth 24 -Compress}
function Pin([string]$Path){[pscustomobject]@{Path=$Path;Bytes=(Get-Item -LiteralPath $Path).Length;Sha256=(Get-FileHash -LiteralPath $Path).Hash}}
function As-HostPin($Pin,[string]$Directory){
    $name=if($Pin.Path.EndsWith('.zip.inventory.json')){'inventory.json'}elseif($Pin.Path.EndsWith('.archive-local.json')){'local-ack.json'}else{'archive.zip'}
    [pscustomobject]@{Path=(Join-Path $Directory $name);Bytes=$Pin.Bytes;Sha256=$Pin.Sha256}
}
function Write-FixtureDelivery($f){
    Write-Text $f.DeliveryPath (Json $f.Receipt)
    $f.ExpectedReceiptSha256=(Pin $f.DeliveryPath).Sha256
}
function New-DeliveryFixture([string]$ArchiveMode='normal'){
    $dir=Join-Path $script:fixtureRoot ([Guid]::NewGuid().ToString('N'));$run=Join-Path $dir 'run'
    $null=[IO.Directory]::CreateDirectory($run)
    $nonce=[Guid]::NewGuid().ToString('N')
    $manifest=[pscustomobject]@{Path='D:\external-original\manifest.json';Bytes=19;Sha256=('A'*64)}
    $resultValue=[ordered]@{SchemaVersion=1;ControllerComplete=$true;Safe=$false;Nonce=$nonce;Manifest=$manifest;NativePassed=$false;Qualified=$false}
    if($ArchiveMode-ceq'result-schema-string'){$resultValue.SchemaVersion='1'}
    if($ArchiveMode-ceq'result-manifest'){$resultValue.Manifest=[pscustomobject]@{Path=$manifest.Path;Bytes=19;Sha256=('F'*64)}}
    if($ArchiveMode-ceq'result-manifest-missing'){$resultValue.Remove('Manifest')}
    $resultJson=Json $resultValue
    $resultPath=Join-Path $run 'controller-result.json';Write-Text $resultPath $resultJson;$result=Pin $resultPath
    $baselinePath=Join-Path $run 'before.json';Write-Text $baselinePath '{"Fixture":true,"Qualified":false}';$baseline=Pin $baselinePath
    # External manifest is intentionally absent. Its original pin is provenance,
    # never a reason to reopen sources or baseline after diagnostic shutdown.
    $sources=@()
    if($ArchiveMode-like'sources-*'){
        $sources=@([pscustomobject]@{Name='diagnostic-controller.ps1';Path='D:\external-original\diagnostic-controller.ps1';Bytes=123;Sha256=('B'*64)})
    }
    $inputJson=Json ([ordered]@{ManifestPin=$manifest;Pins=@{Sources=$sources;Probe=@{Path='D:\external-original\probe.exe'}}})
    $provenance=[ordered]@{SchemaVersion=1;Nonce=$nonce;ResultPin=$result;ResultJson=$resultJson;ManifestPin=$manifest
        InputJson=$inputJson;SourcePins=$sources;StagedScriptPins=@();TerminalPins=@($result,$baseline)
        CollectorProvenance='InjectedCollectorSource';Transferred=$false;NativePassed=$false;Qualified=$false}
    switch($ArchiveMode){
        'provenance-schema-string'{$provenance.SchemaVersion='1'}
        'provenance-terminal-scalar'{$provenance.TerminalPins=$result}
        'provenance-input-duplicate'{$provenance.InputJson=$inputJson.Replace('"ManifestPin":','"ManifestPin":null,"ManifestPin":')}
        'sources-mismatch'{$provenance.SourcePins=@([pscustomobject]@{Name=$sources[0].Name;Path=$sources[0].Path;Bytes=123;Sha256=('C'*64)})}
        'sources-extra-field'{$sources[0]|Add-Member NoteProperty Unreviewed 'not source schema';$provenance.InputJson=Json ([ordered]@{ManifestPin=$manifest;Pins=@{Sources=$sources}})}
    }
    $encoding=[Text.UTF8Encoding]::new($false)
    $provenanceJson=Json $provenance
    if($ArchiveMode-ceq'duplicate-provenance'){$provenanceJson=$provenanceJson.Replace('"SchemaVersion":1,','"SchemaVersion":1,"SchemaVersion":1,')}
    $provenanceBytes=$encoding.GetBytes($provenanceJson)
    $members=@([pscustomobject]@{Name='run/';Kind='Directory';Bytes=[long]0;Sha256=$null}
        [pscustomobject]@{Name='run/before.json';Kind='File';Bytes=$baseline.Bytes;Sha256=$baseline.Sha256}
        [pscustomobject]@{Name='run/controller-result.json';Kind='File';Bytes=$result.Bytes;Sha256=$result.Sha256}
        [pscustomobject]@{Name='export/provenance.json';Kind='File';Bytes=$provenanceBytes.Length;Sha256=(Get-RemovalClosedExportHash $provenanceBytes)})
    switch($ArchiveMode){
        'duplicate'{$members+=,$members[1]}
        'traversal'{$members[1].Name='run/../before.json'}
        'unknown'{$members[1].Name='run/tool.exe'}
    }
    $inventory=[ordered]@{SchemaVersion=1;Nonce=$nonce;ResultPin=$result
        Limits=[ordered]@{RunRelativeDepth=8;Members=512;FileBytes=16777216;TotalBytes=67108864};Members=$members}
    if($ArchiveMode-ceq'caps'){$inventory.Limits.TotalBytes=67108865}
    $inventoryJson=Json $inventory
    if($ArchiveMode-ceq'duplicate-inventory'){$inventoryJson=$inventoryJson.Replace('"SchemaVersion":1,','"SchemaVersion":1,"SchemaVersion":1,')}
    $inventoryBytes=$encoding.GetBytes($inventoryJson);$archive=$run+'.zip';$sidecar=$archive+'.inventory.json'
    [IO.File]::WriteAllBytes($sidecar,$inventoryBytes)
    $stream=[IO.File]::Open($archive,[IO.FileMode]::CreateNew,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    $zip=[IO.Compression.ZipArchive]::new($stream,[IO.Compression.ZipArchiveMode]::Create,$false)
    try{
        foreach($member in $members){
            $entry=$zip.CreateEntry($member.Name)
            if($ArchiveMode-ceq'zip-link'-and$member.Name-ceq'run/before.json'){$entry.ExternalAttributes=1024}
            if($member.Kind-ceq'Directory'){continue}
            $bytes=if($member.Name-ceq'export/provenance.json'){$provenanceBytes}
                elseif($member.Name-ceq'run/controller-result.json'){$encoding.GetBytes($resultJson)}else{[IO.File]::ReadAllBytes($baselinePath)}
            $s=$entry.Open();try{$s.Write($bytes,0,$bytes.Length)}finally{$s.Dispose()}
        }
        if($ArchiveMode-ceq'extra'){$null=$zip.CreateEntry('unexpected.log')}
        $entry=$zip.CreateEntry('export/inventory.json');$s=$entry.Open()
        try{$s.Write($inventoryBytes,0,$inventoryBytes.Length)}finally{$s.Dispose()}
    }finally{$zip.Dispose();$stream.Dispose()}
    $archivePin=Pin $archive;$inventoryPin=Pin $sidecar;$ackPath=$run+'.archive-local.json'
    Write-Text $ackPath (Json ([ordered]@{SchemaVersion=1;Meaning='LOCAL_ARCHIVE_VERIFICATION_ONLY';Nonce=$nonce
        ResultPin=$result;InventoryPin=$inventoryPin;ArchivePin=$archivePin;Transferred=$false;NativePassed=$false;Qualified=$false}))
    $localAck=Pin $ackPath;$hostDirectory='C:\synthetic-host-evidence\'+$nonce
    $receipt=[pscustomobject][ordered]@{SchemaVersion=1;Meaning='HOST_ARCHIVE_DELIVERY_VERIFIED';Target='moiren-lan'
        RunDirectory=$run;Nonce=$nonce;ResultPin=$result;ManifestPin=$manifest;SourceLocalAckPin=$localAck
        ArchivePin=$archivePin;InventoryPin=$inventoryPin;HostArchivePin=(As-HostPin $archivePin $hostDirectory)
        HostInventoryPin=(As-HostPin $inventoryPin $hostDirectory);HostLocalAckPin=(As-HostPin $localAck $hostDirectory)
        HostEvidenceDirectory=$hostDirectory;Transferred=$true;NativePassed=$false;Qualified=$false}
    $f=[pscustomobject]@{Root=$dir;Run=$run;Nonce=$nonce;Receipt=$receipt;DeliveryPath=($run+'.host-delivery.json')
        ExpectedReceiptSha256=$null;ExpectedLocalAckSha256=$localAck.Sha256;Output=($run+'.archive-delivered.json')}
    Write-FixtureDelivery $f
    return $f
}
function Confirm-Fixture($f){
    Confirm-RemovalRunDelivery -RunDirectory $f.Run -DeliveryReceiptPath $f.DeliveryPath `
        -ExpectedDeliveryReceiptSha256 $f.ExpectedReceiptSha256 -ExpectedLocalAckSha256 $f.ExpectedLocalAckSha256 -UserRebootConfirmed
}
Case 'durable-refused-run-host-receipt-produces-target-local-delivery-ack' {
    $f=New-DeliveryFixture;$pin=Confirm-Fixture $f;$saved=[IO.File]::ReadAllText($pin.Path)|ConvertFrom-Json
    Check ($pin.Path-ceq$f.Output-and$pin.Sha256-ceq(Pin $f.Output).Sha256) 'Final target receipt pin/readback differs.'
    Check ($saved.Meaning-ceq'TARGET_ARCHIVE_DELIVERY_ACKNOWLEDGED'-and$saved.Target-ceq'moiren-lan'-and
        $saved.Nonce-ceq$f.Nonce-and$saved.Transferred-and-not$saved.NativePassed-and-not$saved.Qualified-and
        $saved.DeliveryReceiptPin.Sha256-ceq$f.ExpectedReceiptSha256-and
        $saved.SourceLocalAckPin.Sha256-ceq$f.ExpectedLocalAckSha256-and
        $saved.ManifestPin.Sha256-ceq$f.Receipt.ManifestPin.Sha256) 'Durable target receipt lost exact provenance or promoted qualification.'
    Check ($saved.DeliveryReceiptJson-ceq[IO.File]::ReadAllText($f.DeliveryPath)) 'Exact host receipt bytes were not retained as UTF8 JSON snapshot.'
    Check (-not([IO.File]::ReadAllText((Join-Path $f.Run 'controller-result.json'))|ConvertFrom-Json).Safe) 'Refused diagnostic was promoted.'
}
Case 'confirmation-and-real-host-guard-stay-closed-on-development-host' {
    $f=New-DeliveryFixture;$calls=$script:guardCalls
    Reject {Confirm-RemovalRunDelivery -RunDirectory $f.Run -DeliveryReceiptPath $f.DeliveryPath `
        -ExpectedDeliveryReceiptSha256 $f.ExpectedReceiptSha256 -ExpectedLocalAckSha256 $f.ExpectedLocalAckSha256} 'reboot confirmation'
    Check ($script:guardCalls-eq$calls-and-not[IO.File]::Exists($f.Output)) 'Unconfirmed caller reached guard/output.'
    $wrapper=${function:Assert-RemovalDeliveryTargetHost}
    try{Set-Item Function:Assert-RemovalDeliveryTargetHost $actualGuard;Reject {Confirm-Fixture $f} 'target host/SID'}
    finally{Set-Item Function:Assert-RemovalDeliveryTargetHost $wrapper}
}
Case 'independent-host-receipt-and-original-local-ack-hashes-cannot-be-repinned' {
    foreach($mode in @('receipt','local-ack')){
        $f=New-DeliveryFixture
        if($mode-ceq'receipt'){$f.ExpectedReceiptSha256='F'*64}else{$f.ExpectedLocalAckSha256='F'*64}
        Reject {Confirm-Fixture $f};Check (-not[IO.File]::Exists($f.Output)) 'Independent expected hash ignored.'
    }
}
Case 'host-metadata-must-bind-exact-target-nonce-and-provenance' {
    foreach($mode in @('target','run','nonce','manifest','host-hash','host-outside','transferred','qualified','string-bool')){
        $f=New-DeliveryFixture
        switch($mode){
            'target'{$f.Receipt.Target='other-host'}
            'run'{$f.Receipt.RunDirectory+='-other'}
            'nonce'{$f.Receipt.Nonce='0'*32}
            'manifest'{$f.Receipt.ManifestPin.Sha256='F'*64}
            'host-hash'{$f.Receipt.HostArchivePin.Sha256='F'*64}
            'host-outside'{$f.Receipt.HostLocalAckPin.Path='C:\elsewhere\run.archive-local.json'}
            'transferred'{$f.Receipt.Transferred=$false}
            'qualified'{$f.Receipt.Qualified=$true}
            'string-bool'{$f.Receipt.Transferred='true'}
        }
        Write-FixtureDelivery $f;Reject {Confirm-Fixture $f}
        Check (-not[IO.File]::Exists($f.Output)) ('Invalid host receipt accepted: '+$mode)
    }
}
Case 'changed-target-result-archive-inventory-local-ack-or-whole-tree-refuses' {
    foreach($mode in @('result','archive','inventory','local-ack','extra-file')){
        $f=New-DeliveryFixture
        switch($mode){
            'result'{[IO.File]::AppendAllText((Join-Path $f.Run 'controller-result.json'),' ')}
            'archive'{[IO.File]::AppendAllText(($f.Run+'.zip'),' ')}
            'inventory'{[IO.File]::AppendAllText(($f.Run+'.zip.inventory.json'),' ')}
            'local-ack'{[IO.File]::AppendAllText(($f.Run+'.archive-local.json'),' ')}
            'extra-file'{Write-Text (Join-Path $f.Run 'late.log') 'late mutation'}
        }
        Reject {Confirm-Fixture $f};Check (-not[IO.File]::Exists($f.Output)) ('Changed target file accepted: '+$mode)
    }
}
Case 'zip-membership-inventory-schema-and-caps-are-independently-validated' {
    foreach($mode in @('duplicate','traversal','unknown','caps','extra')){
        $f=New-DeliveryFixture $mode
        Reject {Confirm-Fixture $f};Check (-not[IO.File]::Exists($f.Output)) ('Malformed archive accepted: '+$mode)
    }
}
Case 'duplicate-json-keys-refuse-even-with-independent-receipt-hash' {
    foreach($mode in @('duplicate','escaped','nested','case')){
        $f=New-DeliveryFixture;$json=Json $f.Receipt
        switch($mode){
            'duplicate'{$json=$json.Replace('"SchemaVersion":1,','"SchemaVersion":1,"SchemaVersion":1,')}
            'escaped'{$json=$json.Replace('"SchemaVersion":1,','"SchemaVersion":1,"\u0053chemaVersion":1,')}
            'nested'{$json=$json.Replace('"Bytes":','"Bytes":0,"Bytes":')}
            'case'{$json=$json.Replace('"SchemaVersion":1,','"SchemaVersion":1,"schemaVersion":1,')}
        }
        Write-Text $f.DeliveryPath $json;$f.ExpectedReceiptSha256=(Pin $f.DeliveryPath).Sha256
        Reject {Confirm-Fixture $f} 'Duplicate JSON'
        Check (-not[IO.File]::Exists($f.Output)) ('Duplicate JSON accepted: '+$mode)
    }
}
Case 'archived-result-provenance-input-and-inventory-are-strictly-typed-json' {
    foreach($mode in @('result-schema-string','result-manifest','result-manifest-missing','provenance-schema-string','provenance-terminal-scalar',
        'provenance-input-duplicate','duplicate-provenance','duplicate-inventory','zip-link')){
        $f=New-DeliveryFixture $mode
        Reject {Confirm-Fixture $f}
        Check (-not[IO.File]::Exists($f.Output)) ('Non-strict durable archive accepted: '+$mode)
    }
}
Case 'real-source-name-path-byte-hash-shape-retained-without-external-reread' {
    $f=New-DeliveryFixture 'sources-normal';$pin=Confirm-Fixture $f
    Check ([IO.File]::Exists($pin.Path)) 'Actual four-field source pin shape was rejected.'
}
Case 'source-snapshot-mismatch-extra-fields-and-root-array-cannot-finalize' {
    foreach($mode in @('sources-mismatch','sources-extra-field','root-array')){
        $f=New-DeliveryFixture $mode
        if($mode-ceq'root-array'){
            Write-Text $f.DeliveryPath ('['+(Json $f.Receipt)+']')
            $f.ExpectedReceiptSha256=(Pin $f.DeliveryPath).Sha256
        }
        Reject {Confirm-Fixture $f}
        Check (-not[IO.File]::Exists($f.Output)) ('Invalid source snapshot/root schema accepted: '+$mode)
    }
}
Case 'create-new-delivered-receipt-never-overwrites-or-retries' {
    $f=New-DeliveryFixture;Write-Text $f.Output 'preserve existing'
    Reject {Confirm-Fixture $f};Check ([IO.File]::ReadAllText($f.Output)-ceq'preserve existing') 'Existing target ack overwritten.'
    $g=New-DeliveryFixture;$pin=Confirm-Fixture $g
    Reject {Confirm-Fixture $g};Check ((Pin $g.Output).Sha256-ceq$pin.Sha256) 'Repeated finalization rewrote original ack.'
}
$checks|ConvertTo-Json -Depth 7
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' HOST_GUARD=TEST-ONLY-override DURABLE_READBACK=production CWT_RECONSTRUCTION=none NATIVE_OPERATIONS=0')
Write-Output ('FIXTURE_ROOT='+$script:fixtureRoot)
if($failed){exit 1}
