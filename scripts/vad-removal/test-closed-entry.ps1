#requires -Version 5.1
# Exercise the actual entry's one shared terminal-export tail, with export IO replaced.
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$tokens=$null;$errors=$null
$path=Join-Path $PSScriptRoot 'invoke-diagnostic.ps1'
$ast=[Management.Automation.Language.Parser]::ParseFile($path,[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'Diagnostic entry did not parse.'}
$tails=@($ast.EndBlock.Statements|Where-Object{$_.Extent.Text-like'if($null -ne $controller -and $controller.Completed)*'})
if($tails.Count-ne1){Write-Output 'CHECKS=1 FAILED=1: actual entry has no single shared completed-controller export tail';exit 1}
$tail=[scriptblock]::Create($tails[0].Extent.Text)
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
function Export-RemovalClosedRun {
    param($Controller,$ArchivePath)
    Check ([Object]::ReferenceEquals($Controller,$script:EntryFixture.Root)) 'Entry exported a replacement root.'
    Check ($ArchivePath-ceq($RunDirectory+'.zip')) 'Entry did not use fixed sibling archive.'
    $script:EntryFixture.Events.Add('export')
    if($script:EntryFixture.ThrowExport){throw 'Expected archive refusal'}
    return $script:EntryFixture.Export
}
function Confirm-RemovalClosedRunExport {
    param($Export,$AcknowledgementPath)
    Check ([Object]::ReferenceEquals($Export,$script:EntryFixture.Export)) 'Entry replaced original export owner.'
    Check ($AcknowledgementPath-ceq($RunDirectory+'.archive-local.json')) 'Entry local acknowledgment is not a sibling.'
    $script:EntryFixture.Events.Add('ack')
    return @{LocalArchiveVerified=$true;DeliveryConfirmed=$false;Qualified=$false}
}
foreach($mode in @('safe','refused','after-catch','pending','archive-failed')){
    Case ('actual-shared-tail-'+$mode){
        $controller=[pscustomobject]@{Completed=($mode-cne'pending')}
        $script:EntryFixture=[pscustomobject]@{Root=$controller;Export=[pscustomobject]@{fixture='original-export'}
            Events=(New-Object Collections.Generic.List[string]);ThrowExport=($mode-ceq'archive-failed')}
        $RunDirectory='D:\MoirenTest\fixture-closed-run';$result=@{Completed=$controller.Completed;Safe=($mode-ceq'safe');Qualified=$false}
        $exitCode=if($mode-ceq'safe'){0}else{1}
        . $tail|Out-Null
        if($mode-ceq'pending'){Check ($EntryFixture.Events.Count-eq0) 'Pending owner exported.'}
        elseif($mode-ceq'archive-failed'){Check (($EntryFixture.Events-join',')-ceq'export'-and$exitCode-eq1) 'Archive failure acknowledged or became success.'}
        else{Check (($EntryFixture.Events-join',')-ceq'export,ack') 'Entry tail did not export and locally acknowledge exactly once.'}
        if($mode-cne'safe'){Check ($exitCode-eq1) 'Refused/catch result became success after archive.'}
    }
}
$checks|ConvertTo-Json -Depth 4
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ENTRY_TAIL=production EXPORT_IO=injected NATIVE_OPERATIONS=0')
if($failed){exit 1}
