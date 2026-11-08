#requires -Version 5.1
# Composition only: native launch remains guarded in trace-native.ps1.
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'trace-query.ps1')
. (Join-Path $PSScriptRoot 'trace-readback.ps1')
. (Join-Path $PSScriptRoot 'trace-native.ps1')

function New-RemovalTraceSession {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$Session,
        [Parameter(Mandatory=$true)][string]$EtlPath,
        [hashtable]$NativeBoundary,[hashtable]$QueryBoundary)
    if(($null -eq $NativeBoundary) -ne ($null -eq $QueryBoundary)){
        throw 'Native owner and readback test boundaries must be supplied together; no partial native fallback.'
    }
    if($null -ne $NativeBoundary){
        foreach($field in @('Start','Enumerate','Enable','Query','Stop')){
            if($NativeBoundary[$field] -isnot [scriptblock]){throw 'Complete native trace test boundary required.'}
        }
        foreach($field in @('Query','Enumerate')){
            if($QueryBoundary[$field] -isnot [scriptblock]){throw 'Complete readback query test boundary required.'}
        }
    }
    $native=New-RemovalNativeTrace -Session $Session -EtlPath $EtlPath -NativeBoundary $NativeBoundary
    [pscustomobject]@{NativeOwner=$native;Readback=(New-RemovalTraceReadback -Trace $native)
        QueryBoundary=$QueryBoundary;StartAttempted=$false;StopAttempted=$false;StoppedReceipt=$null}
}

function Get-RemovalOwnedTraceSnapshot($Session) {
    $arguments=@{SessionName=$Session.NativeOwner.Session;SessionGuid=$Session.NativeOwner.SessionGuid
        Handle=$Session.NativeOwner.Handle;QueryBoundary=$Session.QueryBoundary}
    if($null -eq $Session.QueryBoundary){$arguments.NativeOwner=$Session.NativeOwner}
    Get-RemovalTraceQuerySnapshot @arguments
}

function Start-RemovalTraceSession {
    param([Parameter(Mandatory=$true,Position=0)]$Session)
    if($Session.StartAttempted){throw 'Trace session start already attempted; retain original owner.'}
    $Session.StartAttempted=$true
    $null=Start-RemovalNativeTrace $Session.NativeOwner
    if($Session.NativeOwner.Failed -or -not$Session.NativeOwner.TraceOwned){throw 'Native trace start/enable failed; retain owner for exact cleanup.'}
    Confirm-RemovalTraceReadback $Session.Readback (Get-RemovalOwnedTraceSnapshot $Session)
    if(-not(Test-RemovalTraceCaptureAdmission $Session.Readback)){throw 'Complete owned trace readback did not admit capture.'}
}

function Refresh-RemovalTraceSession {
    param([Parameter(Mandatory=$true,Position=0)]$Session)
    if(-not$Session.StartAttempted -or $Session.StopAttempted -or $Session.NativeOwner.Failed -or
        -not$Session.NativeOwner.TraceOwned -or $Session.NativeOwner.TraceStopped){throw 'No unchanged live owned trace to refresh.'}
    try {Observe-RemovalTraceReadback $Session.Readback (Get-RemovalOwnedTraceSnapshot $Session)}
    catch {
        if(-not$Session.Readback.Invalid){Set-RemovalTraceReadbackError $Session.Readback $_.Exception.Message}
        throw
    }
    if(-not(Test-RemovalTraceCaptureAdmission $Session.Readback)){throw 'Fresh trace readback refused capture.'}
}

function Stop-RemovalTraceSession {
    param([Parameter(Mandatory=$true,Position=0)]$Session)
    if($Session.StopAttempted){throw 'Trace session stop already attempted; observe retained receipt, never retry.'}
    if(-not$Session.NativeOwner.TraceOwned){throw 'No owned successful StartTrace for cleanup.'}
    $Session.StopAttempted=$true
    # The native owner independently verifies the retained handle before STOP.
    # It must also permit exact cleanup after failed enable/readback. A failed
    # initial admission never acquires final-loss or qualification authority.
    $Session.StoppedReceipt=Stop-RemovalNativeTrace $Session.NativeOwner
    if($null -ne $Session.Readback.Initial -and -not$Session.Readback.Invalid){
        Complete-RemovalTraceLoss $Session.Readback $Session.StoppedReceipt
    }
    if(-not$Session.NativeOwner.TraceStopped){throw 'Trace stop remains pending/unknown; retain original owner.'}
}
