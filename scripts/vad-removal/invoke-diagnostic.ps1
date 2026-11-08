#requires -Version 5.1
[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$RunDirectory,
    [Parameter(Mandatory=$true)][string]$ManifestPath,
    [Parameter(Mandatory=$true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$ManifestSha256,
    [switch]$UserRebootConfirmed,[ValidateRange(0,65535)][int]$WorkerSessionId=0)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$controller=$null
$result=$null;$exitCode=1
try {
    if(-not$UserRebootConfirmed){throw 'User reboot confirmation required; this entry never reboots.'}
    if([Environment]::MachineName -cne 'LAPTOP-NP49RG6H' -or
        [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -cne 'S-1-5-21-3080296863-1242338503-1346362208-1001'){
        throw 'Wrong diagnostic target host/SID; no imports, collection or dispatch.'
    }
    if(-not([Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)){throw 'An already elevated trace controller is required; no automatic elevation.'}
    if($RunDirectory -cnotmatch '\AD:\\MoirenTest\\'){throw 'Diagnostic run directory must be under the existing test workspace.'}
    . (Join-Path $PSScriptRoot 'diagnostic-controller.ps1')
    . (Join-Path $PSScriptRoot 'closed-export.ps1')
    $inputs=New-RemovalRunInputs -ManifestPath $ManifestPath -ExpectedSha256 $ManifestSha256 -SourceDirectory $PSScriptRoot
    $controller=New-RemovalDiagnosticController -RunDirectory $RunDirectory -Inputs $inputs `
        -UserRebootConfirmed -WorkerSessionId $WorkerSessionId
    Start-RemovalDiagnosticController $controller
    do {
        $r=Observe-RemovalDiagnosticController $controller -ObservationMilliseconds 250
        if($r.NeedsObservation){Start-Sleep -Milliseconds 100}
    } while($r.NeedsObservation)
    $result=$r
    if($r.Completed -and $r.Safe){$exitCode=0}
} catch {
    [Console]::Error.WriteLine('Removal diagnostic failed/unknown: '+$_.Exception.Message)
    # A synchronous observer or transport failure does not relinquish original
    # processes, pipes, trace or scheduler intent. Never launch a replacement.
    if($null -ne $controller){
        Add-RemovalControllerError $controller $_.Exception.Message
        $reported=$false
        do {
            try {
                $pending=Observe-RemovalDiagnosticController $controller -ObservationMilliseconds 250
                if(-not$pending.NeedsObservation){$result=$pending;break}
            } catch {
                if(-not$reported){[Console]::Error.WriteLine('Retaining original diagnostic owners after observation refusal.');$reported=$true}
            }
            Start-Sleep -Milliseconds 100
        } while($true)
    }
}
# Both normal and caught/drained completion retain the same root for export.
# Local acknowledgment proves archive readback only, never remote delivery.
if($null -ne $controller -and $controller.Completed){
    try {
        $export=Export-RemovalClosedRun -Controller $controller -ArchivePath ($RunDirectory+'.zip')
        $ack=Confirm-RemovalClosedRunExport -Export $export -AcknowledgementPath ($RunDirectory+'.archive-local.json')
        [pscustomobject]@{ControllerResult=$result;Archive=$export;LocalAcknowledgement=$ack;Qualified=$false}|ConvertTo-Json -Depth 10
    } catch {
        [Console]::Error.WriteLine('Closed-run archive failed; original result/evidence retained: '+$_.Exception.Message)
        $exitCode=1
    }
}
elseif($null -ne $result){$result|ConvertTo-Json -Depth 7}
exit $exitCode
