#requires -Version 5.1
# The controller owns the complete lifetime. Loading it grants no native approval.
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'diagnostic-lease.ps1')
. (Join-Path $PSScriptRoot 'target-readiness.ps1')
. (Join-Path $PSScriptRoot 'run-inputs.ps1')
. (Join-Path $PSScriptRoot 'safety-audit.ps1')
. (Join-Path $PSScriptRoot 'finish-session.ps1')
. (Join-Path $PSScriptRoot 'native-admission.ps1')
. (Join-Path $PSScriptRoot 'controller-seal.ps1')

function Add-RemovalControllerError($Owner,[string]$Message) {
    Add-RemovalControllerErrorRecord $Owner $Message
    # Late original receipts must remain observable; only new audio admission
    # closes. Observe-RemovalManagedSession retains/binds those owners on failure.
    if($null -ne $Owner.RetainedManaged -and -not$Owner.RetainedManaged.Error){$Owner.RetainedManaged.Error=$Message}
}

function Add-RemovalControllerErrorRecord($Owner,[string]$Message) {
    if($Owner.Errors.Count -lt 16 -and -not$Owner.Errors.Contains($Message)){$Owner.Errors.Add($Message)}
}

function New-RemovalDiagnosticController {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$RunDirectory,[Parameter(Mandatory=$true)]$Inputs,
        [switch]$UserRebootConfirmed,[ValidateRange(0,65535)][int]$WorkerSessionId=0)
    if(-not$UserRebootConfirmed){throw 'User reboot confirmation required; controller never reboots.'}
    Assert-RemovalTaskHost;Assert-RemovalTaskPath $RunDirectory;Assert-RemovalRunInputs $Inputs
    if(Test-Path -LiteralPath $RunDirectory){throw 'Create-new full diagnostic directory required.'}
    if($Inputs.SourceDirectory -cne $PSScriptRoot){throw 'Controller must use its own exact pinned source directory.'}
    $self=[Diagnostics.Process]::GetCurrentProcess();$nonce=[Guid]::NewGuid().ToString('N')
    $lease=New-RemovalDiagnosticLease
    $before=New-RemovalTargetReadiness -EvidenceDirectory (Join-Path $RunDirectory 'before') `
        -UserRebootConfirmed -WorkerSessionId $WorkerSessionId -ReadToolPins $Inputs.Pins.Tools
    $trace=New-RemovalTraceSession -Session ('MoirenRemoval613'+$nonce) -EtlPath (Join-Path $RunDirectory 'driver.etl')
    $config=ConvertTo-Json -InputObject ([ordered]@{RunDirectory=$RunDirectory;Nonce=$nonce;ControllerPid=$self.Id
        ControllerCreationUtcTicks=$self.StartTime.ToUniversalTime().Ticks;WorkerSessionId=$WorkerSessionId}) -Compress
    $owner=[pscustomobject]@{RunDirectory=$RunDirectory;OriginalDirectory=$RunDirectory;Nonce=$nonce;ControllerPid=$self.Id
        ControllerCreationUtcTicks=$self.StartTime.ToUniversalTime().Ticks;WorkerSessionId=$WorkerSessionId;ConfigJson=$config
        Inputs=$Inputs;RetainedInputs=$Inputs;Lease=$lease;RetainedLease=$lease;BeforeReadiness=$before;RetainedBefore=$before
        TraceSession=$trace;RetainedTrace=$trace;OriginalNativeOwner=$trace.NativeOwner;OriginalReadback=$trace.Readback
        ManagedSession=$null;RetainedManaged=$null;Finish=$null;RetainedFinish=$null;AfterAudit=$null;RetainedAfter=$null
        StartAttempted=$false;BeforeSettled=$false;BeforeAccepted=$false;AfterStartAttempted=$false;RegistrationRequested=$false;DispatchRequested=$false
        AbortEvidenceAttempted=$false;AbortEvidencePin=$null;ResultAttempted=$false;ResultPin=$null
        Completed=$false;Refused=$false;Safe=$false;Phase='Prepared';Errors=(New-Object Collections.Generic.List[string])
        Clock=[Diagnostics.Stopwatch]::StartNew();DeadlineMilliseconds=[long]240000;TimedOut=$false
        NativePassed=$false;Qualified=$false}
    Register-RemovalControllerLifetime $owner
    return $owner
}

function Assert-RemovalControllerIdentity($Owner) {
    Assert-RemovalTaskHost
    $self=[Diagnostics.Process]::GetCurrentProcess()
    $config=ConvertTo-Json -InputObject ([ordered]@{RunDirectory=$Owner.RunDirectory;Nonce=$Owner.Nonce;ControllerPid=$Owner.ControllerPid
        ControllerCreationUtcTicks=$Owner.ControllerCreationUtcTicks;WorkerSessionId=$Owner.WorkerSessionId}) -Compress
    if($config -cne $Owner.ConfigJson -or $Owner.OriginalDirectory -cne $Owner.RunDirectory -or $self.Id -ne $Owner.ControllerPid -or
        $self.StartTime.ToUniversalTime().Ticks -ne $Owner.ControllerCreationUtcTicks){throw 'Original diagnostic controller configuration changed.'}
    foreach($pair in @(@('Inputs','RetainedInputs'),@('Lease','RetainedLease'),@('BeforeReadiness','RetainedBefore'),
        @('TraceSession','RetainedTrace'),@('ManagedSession','RetainedManaged'),@('Finish','RetainedFinish'),@('AfterAudit','RetainedAfter'))){
        if(-not[Object]::ReferenceEquals($Owner.($pair[0]),$Owner.($pair[1]))){throw ('Original diagnostic owner replaced: '+$pair[0])}
    }
    if(-not[Object]::ReferenceEquals($Owner.RetainedTrace.NativeOwner,$Owner.OriginalNativeOwner) -or
        -not[Object]::ReferenceEquals($Owner.RetainedTrace.Readback,$Owner.OriginalReadback)){
        throw 'Original trace native owner/readback changed.'
    }
}

function Assert-RemovalDiagnosticAdmission($Owner) {
    Assert-RemovalControllerIdentity $Owner
    if($Owner.Errors.Count -or $Owner.TimedOut -or $Owner.Completed){throw 'Diagnostic admission is closed.'}
    Assert-RemovalDiagnosticLease $Owner.RetainedLease
    Assert-RemovalRunInputs $Owner.RetainedInputs
    Assert-RemovalTargetReadiness $Owner.RetainedBefore
    if($Owner.RetainedBefore.RetainedPreflight.ExecutionMode -cne 'RepositoryCollector'){
        throw 'Injected collector readiness cannot authorize a real diagnostic.'
    }
}

function Start-RemovalDiagnosticController {
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    $ErrorActionPreference='Stop'
    if($Owner.StartAttempted){throw 'Full diagnostic already attempted; observe original owners, never restart.'}
    $Owner.StartAttempted=$true
    try {
        Assert-RemovalControllerIdentity $Owner
        Acquire-RemovalDiagnosticLease $Owner.RetainedLease
        Bind-RemovalControllerLifetime $Owner 'Writer'
        $null=[IO.Directory]::CreateDirectory($Owner.RunDirectory)
        try{Start-RemovalTargetReadiness $Owner.RetainedBefore}
        finally{Bind-RemovalControllerLifetime $Owner 'Before'}
        $Owner.Phase='Before'
    } catch {
        Add-RemovalControllerError $Owner $_.Exception.Message
        if(-not$Owner.RetainedBefore.StartAttempted -and -not$Owner.OriginalNativeOwner.StartAttempted -and
            $null -eq $Owner.RetainedManaged){
            if($Owner.RetainedLease.Owned){Release-RemovalDiagnosticLease $Owner.RetainedLease}
            Confirm-RemovalControllerEarlyRefusal $Owner
            $Owner.Refused=$true;$Owner.Phase='RefusedBeforeAcquisition'
        }
        throw
    }
}

function Write-RemovalControllerReceipt($Owner,[string]$Name,$Value) {
    $path=Join-Path $Owner.OriginalDirectory $Name;Assert-RemovalTaskPath $path
    $bytes=[Text.UTF8Encoding]::new($false).GetBytes((ConvertTo-Json -InputObject $Value -Depth 14))
    if($bytes.Length -gt 1048576){throw 'Controller receipt exceeds 1 MiB cap.'}
    $file=[IO.File]::Open($path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try{$file.Write($bytes,0,$bytes.Length);$file.Flush($true)}finally{$file.Dispose()}
    $sha=[Security.Cryptography.SHA256]::Create()
    try{$expected=[BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','')}finally{$sha.Dispose()}
    $pin=Read-RemovalDecodeFile $path 1048576
    if($pin.Bytes -ne $bytes.Length -or $pin.Sha256 -cne $expected){throw 'Original controller receipt readback differs.'}
    [pscustomobject]@{Path=$pin.Path;Bytes=$pin.Bytes;Sha256=$pin.Sha256}
}

function Test-RemovalControllerTraceStopped($Owner) {
    $trace=$Owner.RetainedTrace;$native=$Owner.OriginalNativeOwner
    if(-not$native.TraceOwned){
        # A captured nonzero Start result is failure, not ownership of an
        # existing session. A missing result (or success with invalid handle)
        # remains unresolved; neither path ever grants STOP by name.
        return (-not$native.StartAttempted -or ($null -ne $native.PSObject.Properties['StartDispatchAttempted'] -and
            -not$native.StartDispatchAttempted) -or ($null -ne $native.StartStatus -and
            $native.StartStatus -is [uint32] -and $native.StartStatus -ne 0))
    }
    $receipt=$native.StopReceipt
    return ($native.TraceStopped -and $null -ne $receipt -and
        [Object]::ReferenceEquals($receipt,$trace.StoppedReceipt) -and
        [Object]::ReferenceEquals($receipt.Owner,$native) -and $receipt.StoppedConfirmed -and $receipt.Status -in @(0,234))
}

function Save-RemovalControllerAbortEvidence($Owner) {
    if($Owner.AbortEvidenceAttempted){throw 'Abort evidence already attempted; no overwrite or cleanup retry.'}
    $Owner.AbortEvidenceAttempted=$true
    $managed=$Owner.RetainedManaged
    $value=[ordered]@{SchemaVersion=1;Stage='BeforeAbortTaskRetirement';NativePassed=$false;Qualified=$false
        Nonce=$Owner.Nonce;Manifest=$Owner.RetainedInputs.ManifestPin;Before=$Owner.RetainedBefore.ReceiptPin
        TraceStopped=(Test-RemovalControllerTraceStopped $Owner);Errors=@($Owner.Errors.ToArray())
        TaskPath=$(if($null -ne $managed){$managed.TaskOwner.Spec.Path}else{$null})}
    $Owner.AbortEvidencePin=Write-RemovalControllerReceipt $Owner 'abort-evidence.json' $value
}

function Observe-RemovalControllerAbort($Owner,[int]$ObservationMilliseconds=25) {
    $trace=$Owner.RetainedTrace;$managed=$Owner.RetainedManaged;$native=$Owner.OriginalNativeOwner
    $finished=$null;$managedObservation=$null
    if($null -ne $Owner.RetainedFinish){
        # Finish drains its original tools before refusing changed provenance.
        # Let that observation run even when root trace composition was altered.
        $finished=Observe-RemovalManagedFinish $Owner.RetainedFinish -ObservationMilliseconds $ObservationMilliseconds
    } elseif($null -ne $managed -and $managed.TaskOwner.DispatchAttempted){
        Bind-RemovalControllerLifetime $Owner
        Assert-RemovalControllerSealedIdentity $Owner
        try{$managedObservation=Observe-RemovalManagedSession $managed}
        finally{Bind-RemovalControllerLifetime $Owner 'Probe' $managed.ProbeOwner}
    }
    if(-not[Object]::ReferenceEquals($trace.NativeOwner,$native) -or
        -not[Object]::ReferenceEquals($trace.Readback,$Owner.OriginalReadback)){
        throw 'Original abort trace composition changed; retain ownership without STOP.'
    }
    if($null -ne $Owner.RetainedFinish){
        # Root failure never bypasses pending decoder children, changed pins,
        # pre-delete evidence or an uncertain retirement already owned by Finish.
        $r=$finished
        if($r.Phase -ceq 'AwaitingDecodePins' -and $null -eq $Owner.RetainedFinish.RetainedDecode){
            $pins=New-RemovalDiagnosticDecodePins $Owner.RetainedInputs -TraceSession $trace -ManagedSession $managed
            Set-RemovalManagedFinishDecode -Finish $Owner.RetainedFinish -Pins $pins -OutputDirectory (Join-Path $Owner.OriginalDirectory 'decode')
            if($null -eq $Owner.OriginalNativeOwner.Boundary){Register-RemovalNativeDecode $Owner}
        }
        if(-not$r.Terminal){$Owner.Phase=$r.Phase}
        return $r.Terminal
    }
    if($null -ne $managed -and $null -ne $managed.ProbeOwner -and $managed.ProbeOwner.AdmissionAttempted){
        # Even a lost acknowledgement reply may have admitted native work.
        # Wait real terminal ownership before STOP, then retain normal Finish.
        $r=$managedObservation
        if(-not$r.Terminal){$Owner.Phase='PendingManaged';return $false}
        $finish=New-RemovalManagedFinish $managed;$Owner.RetainedFinish=$finish;$Owner.Finish=$finish
        Bind-RemovalControllerLifetime $Owner 'Finish' $finish
        $Owner.Phase='Finish';return $false
    }
    # Self-owned STOP is independent of current source/Explorer freshness.
    # No decode/qualification is attempted after failed enable or readback.
    if($native.TraceOwned -and -not$trace.StopAttempted){
        try{Stop-RemovalTraceSession $trace}catch{Add-RemovalControllerError $Owner $_.Exception.Message}
    }
    if(-not(Test-RemovalControllerTraceStopped $Owner)){$Owner.Phase='PendingTrace';return $false}
    if($null -eq $managed){return $true}
    $task=$managed.TaskOwner
    if(-not$task.RegistrationAttempted){return $true}
    if(-not$task.RegistrationKnown){$Owner.Phase='PendingRegistration';return $false}
    if(-not$task.DispatchAttempted){
        if($null -eq $Owner.AbortEvidencePin){Save-RemovalControllerAbortEvidence $Owner}
        $null=Copy-RemovalDecodePin $Owner.AbortEvidencePin 1048576
        if(-not$task.Retired){
            if($task.RetirementAttempted){$Owner.Phase='PendingTaskRetirement';return $false}
            Remove-RemovalUnstartedTask $task
        }
        return $task.Retired
    }
    # No receipt or scheduler-idle flag proves a dispatched child absent.
    # Continue the same session even when root admission is already closed.
    $r=$managedObservation
    if(-not$r.Terminal){$Owner.Phase='PendingManaged';return $false}
    if($null -eq $Owner.AbortEvidencePin){Save-RemovalControllerAbortEvidence $Owner}
    $null=Copy-RemovalDecodePin $Owner.AbortEvidencePin 1048576
    if(-not$task.Retired){Complete-RemovalManagedTaskCleanup $managed}
    return $task.Retired
}

function Save-RemovalControllerResult($Owner) {
    if($Owner.ResultAttempted){throw 'Controller result already attempted; preserve original cleanup outcomes.'}
    $Owner.ResultAttempted=$true
    $after=$Owner.RetainedAfter;$finish=$Owner.RetainedFinish
    $Owner.Safe=($Owner.Errors.Count -eq 0 -and $null -ne $after -and $after.Safe -and $null -ne $finish -and $finish.Terminal)
    $value=[ordered]@{SchemaVersion=1;ControllerComplete=$true;Safe=$Owner.Safe;NativePassed=$false;Qualified=$false
        Nonce=$Owner.Nonce;ControllerPid=$Owner.ControllerPid;ControllerCreationUtcTicks=$Owner.ControllerCreationUtcTicks
        Manifest=$Owner.RetainedInputs.ManifestPin;Before=$Owner.RetainedBefore.ReceiptPin
        Finish=$(if($null -ne $finish){$finish.ResultPin}else{$null})
        AbortEvidence=$Owner.AbortEvidencePin;After=$(if($null -ne $after){$after.ReceiptPin}else{$null})
        Errors=@($Owner.Errors.ToArray());TimedOut=$Owner.TimedOut}
    $Owner.ResultPin=Write-RemovalControllerReceipt $Owner 'controller-result.json' $value
    Prepare-RemovalClosedControllerSeal $Owner
    # Keep the exclusive writer through all actual collection and evidence.
    # Only a persisted result releases this original handle; no file deletion.
    Release-RemovalDiagnosticLease $Owner.RetainedLease
    Complete-RemovalClosedControllerSeal $Owner
    $Owner.Completed=$true;$Owner.Phase='Complete'
}

function Get-RemovalControllerResult($Owner) {
    [pscustomobject]@{Completed=$Owner.Completed;Refused=$Owner.Refused;Safe=$Owner.Safe;NeedsObservation=(-not$Owner.Completed -and -not$Owner.Refused)
        NativePassed=$false;Qualified=$false;Phase=$Owner.Phase;ReceiptPin=$Owner.ResultPin
        Errors=@($Owner.Errors.ToArray());TimedOut=$Owner.TimedOut}
}

function Observe-RemovalDiagnosticController {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Owner,[ValidateRange(1,15000)][int]$ObservationMilliseconds=25)
    $ErrorActionPreference='Stop'
    if(Test-RemovalControllerDrainOnly $Owner $ObservationMilliseconds){
        return [pscustomobject]@{Completed=$false;Refused=$false;Safe=$false;NeedsObservation=$true
            NativePassed=$false;Qualified=$false;Phase='IdentityRefusedDrainOnly';ReceiptPin=$null
            Errors=@($Owner.Errors.ToArray());TimedOut=$Owner.TimedOut}
    }
    if($Owner.Completed -or $Owner.Refused){return Get-RemovalControllerResult $Owner}
    $afterObservation=$null
    try {
        if($Owner.AfterStartAttempted -and $null -ne $Owner.RetainedAfter){
            $afterObservation=Observe-RemovalSafetyAudit $Owner.RetainedAfter -ObservationMilliseconds $ObservationMilliseconds
        }
        $beforePending=$false
        if($Owner.StartAttempted -and -not$Owner.BeforeSettled){
            # Original work is observed before public reference/source refusal;
            # a changed outer DTO cannot strand an owned collector or child.
            $before=Observe-RemovalTargetReadiness $Owner.RetainedBefore -ObservationMilliseconds $ObservationMilliseconds
            $beforePending=$before.NeedsObservation
            if(-not$beforePending){
                $Owner.BeforeSettled=$true
                if($before.Ready){$Owner.BeforeAccepted=$true}
                else{Add-RemovalControllerError $Owner ('Preflight refused: '+$before.Error)}
            }
        }
        Bind-RemovalControllerLifetime $Owner
        Assert-RemovalControllerSealedIdentity $Owner
        Assert-RemovalControllerIdentity $Owner
        if(-not$Owner.StartAttempted){throw 'Diagnostic controller has not started.'}
        if($Owner.Clock.ElapsedMilliseconds -ge $Owner.DeadlineMilliseconds){
            $Owner.TimedOut=$true;Add-RemovalControllerError $Owner 'Controller acceptance timed out; original owners remain under observation.'
        }
        if($beforePending){return Get-RemovalControllerResult $Owner}
        if(-not$Owner.Errors.Count){
            if($null -eq $Owner.RetainedManaged){
                Assert-RemovalDiagnosticAdmission $Owner
                # Bind the accepted historical baseline before native birth.
                # Later expiry or Explorer exit must not prevent AFTER reads.
                if($null -eq $Owner.RetainedAfter){
                    $audit=New-RemovalSafetyAudit -BeforeReadiness $Owner.RetainedBefore `
                        -EvidenceDirectory (Join-Path $Owner.OriginalDirectory 'after') -UserRebootConfirmed
                    $Owner.RetainedAfter=$audit;$Owner.AfterAudit=$audit
                    Bind-RemovalControllerLifetime $Owner 'After' $audit
                }
                $path=Join-Path $Owner.RunDirectory 'managed'
                $arguments=Get-RemovalDiagnosticArguments $Owner.RetainedInputs -ArtifactDirectory (Join-Path $path 'artifacts')
                $user=$Owner.RetainedBefore.RetainedUserSession.Snapshot;$pins=$Owner.RetainedInputs.Pins
                $managed=New-RemovalManagedSession -RunDirectory $path -SourceDirectory $Owner.RetainedInputs.SourceDirectory `
                    -Executable $pins.Probe.File.Path -ExpectedSha256 $pins.Probe.File.Sha256 -ProbeArguments $arguments `
                    -SessionId $user.SessionId -Sid $user.Sid -Nonce $Owner.Nonce -TraceSession $Owner.RetainedTrace
                $Owner.RetainedManaged=$managed;$Owner.ManagedSession=$managed
                Bind-RemovalControllerLifetime $Owner 'Managed' $managed
                if($null -eq $Owner.OriginalNativeOwner.Boundary){Initialize-RemovalNativeContext $Owner}
            }
            if(-not$Owner.RetainedTrace.StartAttempted){
                Assert-RemovalDiagnosticAdmission $Owner
                Start-RemovalTraceSession $Owner.RetainedTrace
            }
            if(-not$Owner.RegistrationRequested){
                Assert-RemovalDiagnosticAdmission $Owner;$Owner.RegistrationRequested=$true
                Register-RemovalTaskDefinition $Owner.RetainedManaged.TaskOwner
                if(-not$Owner.RetainedManaged.TaskOwner.RegistrationKnown){throw 'Original task registration failed/unknown.'}
            }
            if(-not$Owner.DispatchRequested){
                Assert-RemovalDiagnosticAdmission $Owner;$Owner.DispatchRequested=$true
                Start-RemovalTaskOnce $Owner.RetainedManaged.TaskOwner
                if($Owner.RetainedManaged.TaskOwner.Error){throw $Owner.RetainedManaged.TaskOwner.Error}
            }
            # This check precedes the call that can write probe-admit.json.
            # If it fails, the catch latches Session.Error before drain-only reads.
            if($null -eq $Owner.RetainedFinish){
                Assert-RemovalDiagnosticAdmission $Owner
                Bind-RemovalControllerLifetime $Owner
                Assert-RemovalControllerSealedIdentity $Owner
                try{$managed=Observe-RemovalManagedSession $Owner.RetainedManaged}
                finally{Bind-RemovalControllerLifetime $Owner 'Probe' $Owner.RetainedManaged.ProbeOwner}
                if(-not$managed.Terminal){$Owner.Phase='Managed';return Get-RemovalControllerResult $Owner}
                $finish=New-RemovalManagedFinish $Owner.RetainedManaged
                $Owner.RetainedFinish=$finish;$Owner.Finish=$finish
                Bind-RemovalControllerLifetime $Owner 'Finish' $finish
            }
            $finished=Observe-RemovalManagedFinish $Owner.RetainedFinish -ObservationMilliseconds $ObservationMilliseconds
            if($finished.Phase -ceq 'AwaitingDecodePins' -and $null -eq $Owner.RetainedFinish.RetainedDecode){
                $pins=New-RemovalDiagnosticDecodePins $Owner.RetainedInputs -TraceSession $Owner.RetainedTrace -ManagedSession $Owner.RetainedManaged
                Set-RemovalManagedFinishDecode -Finish $Owner.RetainedFinish -Pins $pins -OutputDirectory (Join-Path $Owner.RunDirectory 'decode')
                if($null -eq $Owner.OriginalNativeOwner.Boundary){Register-RemovalNativeDecode $Owner}
                $Owner.Phase='Finish';return Get-RemovalControllerResult $Owner
            }
            if(-not$finished.Terminal){$Owner.Phase=$finished.Phase;return Get-RemovalControllerResult $Owner}
        }
    } catch {Add-RemovalControllerError $Owner $_.Exception.Message}
    try {
        # A pending before owner must always settle first, including after an
        # earlier exception. Observation never grants permission to restart it.
        if(-not$Owner.BeforeSettled){return Get-RemovalControllerResult $Owner}
        if($Owner.Errors.Count -and -not(Observe-RemovalControllerAbort $Owner $ObservationMilliseconds)){return Get-RemovalControllerResult $Owner}
        if(-not$Owner.BeforeAccepted){
            Save-RemovalControllerResult $Owner;return Get-RemovalControllerResult $Owner
        }
        if($null -eq $Owner.RetainedAfter){
            $audit=New-RemovalSafetyAudit -BeforeReadiness $Owner.RetainedBefore -EvidenceDirectory (Join-Path $Owner.OriginalDirectory 'after') -UserRebootConfirmed
            $Owner.RetainedAfter=$audit;$Owner.AfterAudit=$audit
            Bind-RemovalControllerLifetime $Owner 'After' $audit
        }
        if(-not$Owner.AfterStartAttempted){
            $Owner.AfterStartAttempted=$true
            $audit=$Owner.RetainedAfter
            try{Start-RemovalSafetyAudit $audit}
            finally{Bind-RemovalControllerLifetime $Owner 'AfterCollector'}
        }
        $after=if($null -ne $afterObservation){$afterObservation}else{
            Observe-RemovalSafetyAudit $Owner.RetainedAfter -ObservationMilliseconds $ObservationMilliseconds
        }
        if($after.NeedsObservation){$Owner.Phase='After';return Get-RemovalControllerResult $Owner}
        if(-not$after.Complete -or $null -eq $Owner.RetainedAfter.ReceiptPin){
            $Owner.Phase='PendingAfterEvidence'
            Add-RemovalControllerError $Owner 'Original AFTER receipt is missing; no controller completion or lease release.'
            return Get-RemovalControllerResult $Owner
        }
        Assert-RemovalSafetyReceipt $Owner.RetainedAfter
        if(-not$after.Safe){Add-RemovalControllerError $Owner 'After-collection safety audit failed; original snapshot and refusal are preserved.'}
        Save-RemovalControllerResult $Owner
    } catch {Add-RemovalControllerError $Owner $_.Exception.Message}
    return Get-RemovalControllerResult $Owner
}
