#requires -Version 5.1
# Terminal ownership -> exact STOP -> closed-file diagnosis -> evidence -> task
# retirement. This composes existing boundaries, not native launch approval.
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'trace-decode.ps1')

function Add-RemovalFinishError($Finish,[string]$Message) {
    if($Finish.Errors.Count -lt 16 -and -not$Finish.Errors.Contains($Message)){$Finish.Errors.Add($Message)}
}

function Assert-RemovalFinishIdentity($Finish) {
    $s=$Finish.Session;$self=[Diagnostics.Process]::GetCurrentProcess()
    if($self.Id -ne $Finish.ControllerPid -or $self.StartTime.ToUniversalTime().Ticks -ne $Finish.ControllerCreationUtcTicks -or
        $s.ControllerPid -ne $Finish.ControllerPid -or $s.ControllerCreationUtcTicks -ne $Finish.ControllerCreationUtcTicks -or
        -not[Object]::ReferenceEquals($s.TaskOwner,$Finish.TaskOwner) -or
        -not[Object]::ReferenceEquals($s.ProbeOwner,$Finish.ProbeOwner) -or
        -not[Object]::ReferenceEquals($s.TraceSession,$Finish.TraceSession) -or
        -not[Object]::ReferenceEquals($s.TraceReadback,$Finish.TraceReadback) -or
        -not[Object]::ReferenceEquals($Finish.TraceReadback,$Finish.TraceSession.Readback) -or
        -not[Object]::ReferenceEquals($Finish.TraceReadback.Trace,$Finish.NativeOwner) -or
        -not[Object]::ReferenceEquals($Finish.TraceSession.NativeOwner,$Finish.NativeOwner) -or
        -not[Object]::ReferenceEquals($Finish.Decode,$Finish.RetainedDecode)){
        throw 'Original finish controller/task/probe/trace/readback/decode identity changed.'
    }
    $d=$Finish.RetainedDecode
    if($null -ne $d -and (-not[Object]::ReferenceEquals($d.TraceSession,$Finish.TraceSession) -or
        -not[Object]::ReferenceEquals($d.ManagedSession,$s) -or
        -not[Object]::ReferenceEquals($d.StopReceipt,$Finish.NativeOwner.StopReceipt))){
        throw 'Decode no longer binds this original managed trace/STOP.'
    }
}

function Get-RemovalFinishTerminal($Finish) {
    $state=Get-RemovalTaskObservation $Finish.TaskOwner
    $r=Complete-RemovalProbeTerminal $Finish.TaskOwner $state $Finish.ProbeOwner $Finish.Session.LastResult
    if($r.Terminal -isnot [bool] -or -not$r.Terminal -or $r.NeedsObservation -or
        $r.ProbeOwnershipVerified -isnot [bool] -or -not$r.ProbeOwnershipVerified -or
        $r.Owners -isnot [Array] -or $r.Owners.Count -ne 0){return $null}
    return $r
}

function New-RemovalManagedFinish {
    param([Parameter(Mandatory=$true,Position=0)]$Session)
    $self=[Diagnostics.Process]::GetCurrentProcess()
    if($null -eq $Session.TaskOwner -or $null -eq $Session.ProbeOwner -or $null -eq $Session.TraceSession -or
        $null -eq $Session.TraceReadback -or $Session.TaskOwner.Retired -or $Session.TaskOwner.RetirementAttempted){
        throw 'Original managed task/probe/trace owners required before finish.'
    }
    $h=[pscustomobject]@{Session=$Session;ControllerPid=$self.Id;ControllerCreationUtcTicks=$self.StartTime.ToUniversalTime().Ticks
        TaskOwner=$Session.TaskOwner;ProbeOwner=$Session.ProbeOwner;TraceSession=$Session.TraceSession
        NativeOwner=$Session.TraceSession.NativeOwner;TraceReadback=$Session.TraceReadback
        Decode=$null;RetainedDecode=$null;ManagedProvenanceJson=$null;DecodeInvokeAttempted=$false;EvidenceAttempted=$false;ReceiptPin=$null
        RetirementInvoked=$false;ResultAttempted=$false;ResultPin=$null;Terminal=$false;Phase='WaitingManaged';LastTerminal=$null
        ToolReceipts=@();ToolLogPins=@();Errors=(New-Object Collections.Generic.List[string])}
    Assert-RemovalFinishIdentity $h
    return $h
}

function Set-RemovalManagedFinishDecode {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)]$Finish,[Parameter(Mandatory=$true)]$Pins,
        [Parameter(Mandatory=$true)][string]$OutputDirectory,[hashtable]$ToolBoundary)
    Assert-RemovalFinishIdentity $Finish
    if($null -ne $Finish.RetainedDecode -or $Finish.DecodeInvokeAttempted -or $Finish.EvidenceAttempted -or
        $Finish.Phase -cne 'AwaitingDecodePins'){throw 'Unused post-STOP decode attachment required.'}
    $receipt=Assert-RemovalDecodeStoppedSession $Finish.TraceSession
    if($null -eq (Get-RemovalFinishTerminal $Finish)){throw 'Original task/worker/probe no longer terminal.'}
    # The closed ETL hash is supplied only after STOP. Construction validates it
    # but launches nothing; retain the exact object before the first tool call.
    $decode=New-RemovalTraceDecode -TraceSession $Finish.TraceSession -ManagedSession $Finish.Session `
        -Pins $Pins -OutputDirectory $OutputDirectory -ToolBoundary $ToolBoundary
    $Finish.RetainedDecode=$decode;$Finish.Decode=$decode
    $Finish.ManagedProvenanceJson=ConvertTo-Json -InputObject $decode.ManagedProvenance -Depth 8 -Compress
    $Finish.Phase='DecodeReady'
}

function Assert-RemovalFinishManagedProvenance($Finish) {
    $fresh=Assert-RemovalDecodeManagedReport $Finish.RetainedDecode
    if((ConvertTo-Json -InputObject $fresh -Depth 8 -Compress) -cne $Finish.ManagedProvenanceJson -or
        (ConvertTo-Json -InputObject $Finish.RetainedDecode.ManagedProvenance -Depth 8 -Compress) -cne $Finish.ManagedProvenanceJson){
        throw 'Original managed argv/report/source/owner provenance changed after decode attachment.'
    }
}

function Observe-RemovalFinishTools($Finish,[int]$ObservationMilliseconds) {
    $d=$Finish.RetainedDecode
    if($null -eq $d){return $true}
    $all=$true;$receipts=@();$logs=@()
    foreach($owner in $d.ToolOwners){
        try {
            # Ignore the injected observation delegate here: settlement is an
            # independent read of the original finite process/pipes/flushes.
            if(-not$owner.Closed){$null=Observe-FiniteTool $owner -ObservationMilliseconds $ObservationMilliseconds}
            $r=Get-FiniteToolReceipt $owner
            $resolved=$owner.Process -is [Diagnostics.Process] -and $owner.IdentityConfirmed -and
                $owner.ExitedConfirmed -and $owner.ExitCode -is [int] -and $owner.Copies.Count -eq 2 -and
                $owner.Flushes.Count -eq 2 -and $null -ne $owner.CompletionObservedMilliseconds
            foreach($task in @($owner.Copies)+@($owner.Flushes)){
                if(-not$task.IsCompleted){$resolved=$false}else{$null=$task.GetAwaiter().GetResult()}
            }
            if(-not$resolved){$all=$false;continue}
            Close-FiniteToolOwner $owner
            foreach($path in @($owner.StdoutPath,$owner.StderrPath)){
                $file=Read-RemovalDecodeFile $path 1048576 -AllowEmpty
                $logs+=,[pscustomobject]@{Path=$file.Path;Bytes=$file.Bytes;Sha256=$file.Sha256}
            }
            $receipts+=,[pscustomobject]@{Executable=$owner.Executable;ExecutableSha256=$owner.ExecutableSha256
                Arguments=$owner.Arguments;ProcessId=$owner.ProcessId;CreationUtcTicks=$owner.CreationUtcTicks
                ExitedConfirmed=$r.ExitedConfirmed;ExitCode=$r.ExitCode;TimedOut=$r.TimedOut
                OutputStable=$r.OutputStable;LogLimitExceeded=$r.LogLimitExceeded;Closed=$owner.Closed;Errors=$r.Errors}
        } catch {$all=$false;Add-RemovalFinishError $Finish ('Retained tool observation: '+$_.Exception.Message)}
    }
    # A dispatch that never returned an owner cannot establish process absence.
    foreach($step in $d.Steps){
        if($null -eq $step.Owner -and ($null -eq $step.PSObject.Properties['DispatchAttempted'] -or
            $step.DispatchAttempted -isnot [bool] -or $step.DispatchAttempted)){$all=$false}
    }
    $Finish.ToolReceipts=$receipts;$Finish.ToolLogPins=$logs
    return $all
}

function Save-RemovalFinishEvidence($Finish) {
    if($Finish.EvidenceAttempted){throw 'Finish evidence already attempted; never adopt/overwrite a file.'}
    $Finish.EvidenceAttempted=$true
    $d=$Finish.RetainedDecode;$stop=Assert-RemovalDecodeStoppedSession $Finish.TraceSession
    if(-not$d.Attempted){throw 'Decode was not attempted; no saved decode authority.'}
    if($null -eq $d.ReceiptPin -or $d.ReceiptPin.Path -cne $d.ReceiptPath){throw 'Original persisted decode receipt pin required.'}
    $null=Copy-RemovalDecodePin $d.ReceiptPin 1048576
    Assert-RemovalDecodeInputs $d
    Assert-RemovalFinishManagedProvenance $Finish
    $decodePin=Read-RemovalDecodeFile $d.ReceiptPath 1048576 -Text
    $saved=$decodePin.Text|ConvertFrom-Json
    if($saved.SchemaVersion -ne 1 -or $saved.Qualified -isnot [bool] -or $saved.Qualified -or
        $saved.NativePassed -isnot [bool] -or $saved.NativePassed -or
        $saved.ManagedProvenance.RunDirectory -cne $Finish.Session.RunDirectory -or
        $saved.ManagedProvenance.Nonce -cne $Finish.TaskOwner.Spec.Nonce -or
        $saved.ManagedProvenance.ProbePid -ne $Finish.ProbeOwner.ProcessId -or
        $saved.ManagedProvenance.ProbeCreationUtcTicks -ne $Finish.ProbeOwner.CreationUtcTicks -or
        $saved.Trace.SessionGuid -cne $stop.SessionGuid.ToString() -or
        $saved.Trace.OwnedHandle -ne $stop.OwnedHandle -or $saved.Trace.StopStatus -ne $stop.Status){
        throw 'Saved decode receipt does not match this original diagnostic.'
    }
    $outputs=@($d.OutputPins.ToArray())+@($Finish.ToolLogPins)
    foreach($pin in $outputs){$null=Copy-RemovalDecodePin $pin 16777216 -AllowEmpty}
    $r=[ordered]@{SchemaVersion=1;NativePassed=$false;Qualified=$false;TaskRetired=$false
        Stage='BeforeTaskRetirement';DecodeReceipt=@{Path=$decodePin.Path;Bytes=$decodePin.Bytes;Sha256=$decodePin.Sha256}
        ManagedProvenance=$d.ManagedProvenance;InputPins=$d.Pins;OutputPins=$outputs
        DecodeCompleted=$d.Completed;DecodeFailed=$d.Failed;DecodeErrors=@($d.Errors.ToArray())
        AssociationAvailable=$d.AssociationAvailable;RawTraceLossKnown=$d.RawTraceLossKnown;RawTraceLoss=$d.RawTraceLoss
        Trace=@{Session=$stop.SessionName;SessionGuid=$stop.SessionGuid.ToString();OwnedHandle=$stop.OwnedHandle
            StopStatus=$stop.Status;StoppedConfirmed=$stop.StoppedConfirmed;FinalCountersKnown=$stop.FinalCountersKnown}
        Tools=$Finish.ToolReceipts;Errors=@($Finish.Errors.ToArray())
        Limitations=@('Diagnostic association and safe ownership retirement are not native/audio/physical qualification.'
            'This file precedes task retirement; the retained task owner separately records its exact outcome.'
            'No process kill, tool retry, device restart, directory deletion or durable crash recovery is provided.')}
    $bytes=[Text.UTF8Encoding]::new($false).GetBytes((ConvertTo-Json -InputObject $r -Depth 18))
    if($bytes.Length -gt 1048576){throw 'Finish evidence exceeds 1 MiB cap.'}
    $path=Join-Path $d.OutputDirectory 'finish-evidence.json';Assert-FiniteToolPath $path $false
    $file=[IO.File]::Open($path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try{$file.Write($bytes,0,$bytes.Length);$file.Flush($true)}finally{$file.Dispose()}
    $sha=[Security.Cryptography.SHA256]::Create()
    try{$expected=[BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','')}finally{$sha.Dispose()}
    $actual=Read-RemovalDecodeFile $path 1048576
    if($actual.Bytes -ne $bytes.Length -or $actual.Sha256 -cne $expected){throw 'Finish evidence readback differs.'}
    $Finish.ReceiptPin=[pscustomobject]@{Path=$path;Bytes=$actual.Bytes;Sha256=$actual.Sha256}
}

function Get-RemovalFinishResult($Finish) {
    $d=$Finish.RetainedDecode
    [pscustomobject]@{Phase=$Finish.Phase;Terminal=$Finish.Terminal;NeedsObservation=(-not$Finish.Terminal)
        TaskRetired=$Finish.TaskOwner.Retired;NativePassed=$false;Qualified=$false
        AssociationAvailable=($null -ne $d -and $d.AssociationAvailable)
        RawTraceLossKnown=($null -ne $d -and $d.RawTraceLossKnown);ReceiptPin=$Finish.ReceiptPin;ResultPin=$Finish.ResultPin
        Errors=@($Finish.Errors.ToArray())}
}

function Save-RemovalFinishResult($Finish) {
    if($Finish.ResultAttempted){throw 'Final result already attempted; retain original task outcome, never repeat deletion.'}
    $Finish.ResultAttempted=$true
    if(-not$Finish.TaskOwner.Retired -or $null -eq $Finish.ReceiptPin){throw 'Confirmed exact retirement and preceding evidence required.'}
    $null=Copy-RemovalDecodePin $Finish.ReceiptPin 1048576
    $xml=Read-RemovalDecodeFile $Finish.TaskOwner.RetiredXmlPath 1048576
    if($xml.Sha256 -cne $Finish.TaskOwner.RetiredXmlSha256){throw 'Retired original task XML readback differs.'}
    $r=[ordered]@{SchemaVersion=1;NativePassed=$false;Qualified=$false;TaskRetired=$true
        FinishEvidence=$Finish.ReceiptPin;TaskPath=$Finish.TaskOwner.Spec.Path
        RetiredTaskXml=@{Path=$xml.Path;Bytes=$xml.Bytes;Sha256=$xml.Sha256}
        DecodeCompleted=$Finish.RetainedDecode.Completed;DecodeFailed=$Finish.RetainedDecode.Failed
        AssociationAvailable=$Finish.RetainedDecode.AssociationAvailable;Errors=@($Finish.Errors.ToArray())}
    $bytes=[Text.UTF8Encoding]::new($false).GetBytes((ConvertTo-Json -InputObject $r -Depth 8))
    if($bytes.Length -gt 1048576){throw 'Final result exceeds 1 MiB cap.'}
    $path=Join-Path $Finish.RetainedDecode.OutputDirectory 'finish-result.json';Assert-FiniteToolPath $path $false
    $file=[IO.File]::Open($path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try{$file.Write($bytes,0,$bytes.Length);$file.Flush($true)}finally{$file.Dispose()}
    $sha=[Security.Cryptography.SHA256]::Create()
    try{$expected=[BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','')}finally{$sha.Dispose()}
    $actual=Read-RemovalDecodeFile $path 1048576
    if($actual.Bytes -ne $bytes.Length -or $actual.Sha256 -cne $expected){throw 'Final result readback differs.'}
    $Finish.ResultPin=[pscustomobject]@{Path=$actual.Path;Bytes=$actual.Bytes;Sha256=$actual.Sha256}
}

function Observe-RemovalManagedFinish {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Finish,[ValidateRange(1,15000)][int]$ObservationMilliseconds=25)
    $ErrorActionPreference='Stop'
    # Continue original tool observation even if later provenance checks fail.
    $toolsResolved=Observe-RemovalFinishTools $Finish $ObservationMilliseconds
    try {
        Assert-RemovalFinishIdentity $Finish
        if($Finish.Terminal){return Get-RemovalFinishResult $Finish}
        # Retirement succeeded but persistence may have failed afterwards. The
        # original owner remains authoritative; never readopt/delete its name.
        if($Finish.RetirementInvoked){throw 'Original retirement already invoked; retain outcome without retry.'}
        $terminal=Get-RemovalFinishTerminal $Finish
        if($null -eq $terminal){$Finish.Phase='WaitingManaged';return Get-RemovalFinishResult $Finish}
        $Finish.LastTerminal=$terminal
        if(-not$Finish.TraceSession.StopAttempted){
            $Finish.Phase='StoppingTrace'
            try{Stop-RemovalTraceSession $Finish.TraceSession}catch{Add-RemovalFinishError $Finish $_.Exception.Message}
        }
        try{$null=Assert-RemovalDecodeStoppedSession $Finish.TraceSession}
        catch{$Finish.Phase='WaitingTrace';throw}
        if($null -eq $Finish.RetainedDecode){$Finish.Phase='AwaitingDecodePins';return Get-RemovalFinishResult $Finish}
        $d=$Finish.RetainedDecode
        if(-not$Finish.DecodeInvokeAttempted){
            $Finish.DecodeInvokeAttempted=$true
            if(-not$d.Attempted){
                try{$null=Invoke-RemovalTraceDecode $d}catch{Add-RemovalFinishError $Finish ('Decode invocation: '+$_.Exception.Message)}
            }
            $toolsResolved=Observe-RemovalFinishTools $Finish $ObservationMilliseconds
        }
        if(-not$toolsResolved){$Finish.Phase='WaitingTools';return Get-RemovalFinishResult $Finish}
        $Finish.Phase='PersistingEvidence'
        if($null -eq $Finish.ReceiptPin){Save-RemovalFinishEvidence $Finish}
        $null=Copy-RemovalDecodePin $Finish.ReceiptPin 1048576
        # Recheck receipt/input/output pins at the destructive boundary. Decode
        # failure may retire a safe task, never erase diagnostic refusal/loss.
        $saved=(Read-RemovalDecodeFile $Finish.ReceiptPin.Path 1048576 -Text).Text|ConvertFrom-Json
        Assert-RemovalDecodeInputs $d
        Assert-RemovalFinishManagedProvenance $Finish
        $null=Copy-RemovalDecodePin $saved.DecodeReceipt 1048576
        foreach($pin in $saved.OutputPins){$null=Copy-RemovalDecodePin $pin 16777216 -AllowEmpty}
        $resultPath=Join-Path $d.OutputDirectory 'finish-result.json';Assert-FiniteToolPath $resultPath $false
        if(Test-Path -LiteralPath $resultPath){throw 'Preserve existing final result; no task deletion.'}
        $Finish.Phase='RetiringTask'
        if($Finish.RetirementInvoked -or $Finish.TaskOwner.RetirementAttempted){throw 'Task retirement already attempted/unknown; never retry.'}
        $Finish.RetirementInvoked=$true
        Complete-RemovalManagedTaskCleanup $Finish.Session
        if(-not$Finish.TaskOwner.Retired){throw 'Original task retirement not confirmed.'}
        $Finish.Phase='PersistingResult'
        Save-RemovalFinishResult $Finish
        $Finish.Terminal=$true;$Finish.Phase='Complete'
    } catch {Add-RemovalFinishError $Finish $_.Exception.Message}
    return Get-RemovalFinishResult $Finish
}
