#requires -Version 5.1
[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$EvidenceDirectory,[switch]$UserRebootConfirmed,
    [ValidateRange(0,65535)][int]$WorkerSessionId=0)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$session=$null
try {
    # Refuse before importing collectors or creating evidence. A confirmed
    # reboot is intent only; actual boot/uptime/package observations decide.
    if(-not$UserRebootConfirmed){throw 'User reboot confirmation required; this entry never reboots the machine.'}
    if([Environment]::MachineName -cne 'LAPTOP-NP49RG6H' -or
        [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -cne 'S-1-5-21-3080296863-1242338503-1346362208-1001'){
        throw 'Wrong target host/SID; no collector or device operation.'
    }
    . (Join-Path $PSScriptRoot 'target-readiness.ps1')
    $session=New-RemovalTargetReadiness -EvidenceDirectory $EvidenceDirectory -UserRebootConfirmed -WorkerSessionId $WorkerSessionId
    Start-RemovalTargetReadiness $session
    do {
        $r=Observe-RemovalTargetReadiness $session -ObservationMilliseconds 250
        if($r.NeedsObservation){Start-Sleep -Milliseconds 50}
    } while($r.NeedsObservation)
    if(-not$r.Ready){throw ('Target preflight refused: '+$r.Error)}
    Assert-RemovalTargetReadiness $session
    [ordered]@{PreflightReady=$true;NativeApproved=$false;Qualified=$false;AudioAccessProven=$false
        ControllerSession=$session.RetainedPreflight.ControllerSession;WorkerIdentity=$session.RetainedUserSession.Snapshot
        ReceiptPin=$session.ReceiptPin;PreflightReceiptPin=$session.RetainedPreflight.ReceiptPin}|ConvertTo-Json -Depth 6
    exit 0
} catch {
    [Console]::Error.WriteLine('Removal preflight failed/unknown: '+$_.Exception.Message)
    # A timeout ends acceptance, not ownership. Keep the original invocation and
    # inner handles until natural completion; do not stop/dispose/relaunch them.
    $reported=$false
    while($null -ne $session -and $null -ne $session.RetainedPreflight.RetainedOwner -and -not$session.RetainedPreflight.RetainedOwner.Closed){
        try{$null=Observe-RemovalTargetReadiness $session -ObservationMilliseconds 250}
        catch{if(-not$reported){[Console]::Error.WriteLine('Retaining original preflight owner after observation failure.');$reported=$true}}
        Start-Sleep -Milliseconds 100
    }
    exit 1
}
