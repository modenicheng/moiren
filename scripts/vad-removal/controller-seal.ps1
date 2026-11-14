#requires -Version 5.1
# Same-process lifetime correlation, not a security boundary against arbitrary code.
Set-StrictMode -Version 2
if($null-eq(Get-Variable RemovalControllerLifetimes -Scope Script -ErrorAction SilentlyContinue)){
    $script:RemovalControllerLifetimes=New-Object 'Runtime.CompilerServices.ConditionalWeakTable[object,object]'
}

function Get-RemovalSealProperty($Value,[string]$Name){
    if($null-eq$Value){return $null}
    if($Value-is[Collections.IDictionary]){return $Value[$Name]}
    if($null-ne$Value.PSObject.Properties[$Name]){return $Value.$Name}
    return $null
}

function Get-RemovalControllerCollectorJson($Preflight){
    $fields=[ordered]@{}
    foreach($name in @('FiniteToolPath','FiniteToolSha256','CollectorPath','CollectorSha256',
        'ExecutionMode','ObservationMode','ReadToolPinsJson')){
        $fields[$name]=Get-RemovalSealProperty $Preflight $name
    }
    ConvertTo-Json -InputObject $fields -Compress
}

function Register-RemovalControllerLifetime($Owner){
    $self=[Diagnostics.Process]::GetCurrentProcess()
    $preflight=$Owner.RetainedBefore.RetainedPreflight
    $c=[pscustomobject]@{Root=$Owner;Pid=$self.Id;Creation=$self.StartTime.ToUniversalTime().Ticks
        Directory=$Owner.OriginalDirectory;Nonce=$Owner.Nonce;ConfigJson=$Owner.ConfigJson
        Inputs=$Owner.RetainedInputs;Lease=$Owner.RetainedLease;Before=$Owner.RetainedBefore
        Preflight=$Owner.RetainedBefore.RetainedPreflight;Trace=$Owner.RetainedTrace;Native=$Owner.OriginalNativeOwner
        Readback=$Owner.OriginalReadback;Managed=$null;Task=$null;Probe=$null;Finish=$null;After=$null
        BeforeCollector=$null;BeforeDispatch=$null;AfterPreflight=$null;AfterCollector=$null;Writer=$null;WriterHandle=$null
        StagedScriptPins=@();InputJson=(ConvertTo-Json -InputObject $Owner.RetainedInputs -Depth 18 -Compress)
        ResultPin=$null;ResultPinJson=$null;PreparedJson=$null;SealJson=$null;Error=$null
        WorkerProcess=$null;ProbeProcess=$null;DrainOnly=$false;WorkerDrain=$null;ProbeDrain=$null;EarlyRefused=$false
        Clock=$Owner.Clock;DeadlineMilliseconds=$Owner.DeadlineMilliseconds
        TimestampOrigin=[Diagnostics.Stopwatch]::GetTimestamp()
        CollectorIdentityJson=(Get-RemovalControllerCollectorJson $preflight)
        CollectorSourceIdentityJson=(Get-RemovalSealProperty $preflight 'SourceIdentityJson')
        ReleaseAttempted=$false;ReleaseGranted=$false;ReleaseError=$null;ReleaseHandlesClosed=$false
        ReleaseHandles=(New-Object Collections.Generic.List[object]);ReleaseSafeHandles=@();ReleaseHandlePins=@()}
    $script:RemovalControllerLifetimes.Add($Owner,$c)
}

function Get-RemovalControllerLifetime($Owner){
    $c=$null
    if($null-eq$Owner-or-not$script:RemovalControllerLifetimes.TryGetValue($Owner,[ref]$c)){
        throw 'Original constructor-retained controller required; public completion is not export authority.'
    }
    return $c
}

function Test-RemovalControllerDeadlineExpired($Owner){
    $c=Get-RemovalControllerLifetime $Owner
    $now=[Diagnostics.Stopwatch]::GetTimestamp()
    if($c.TimestampOrigin-isnot[long]-or$c.TimestampOrigin-gt$now-or
        $c.DeadlineMilliseconds-isnot[long]-or$c.DeadlineMilliseconds-ne240000){
        throw 'Original finite controller timestamp/deadline required.'
    }
    # The public Stopwatch is observational. Stop/Reset/Restart cannot renew
    # the constructor budget used by both birth gates and controller timeout.
    return (($now-$c.TimestampOrigin)*1000.0/[Diagnostics.Stopwatch]::Frequency-ge$c.DeadlineMilliseconds)
}

function Assert-RemovalControllerCollectorSource($Owner){
    $c=Get-RemovalControllerLifetime $Owner;Assert-RemovalClosedLifetimeIdentity $c
    $p=$c.Preflight
    if((Get-RemovalControllerCollectorJson $p)-cne$c.CollectorIdentityJson-or
        (Get-RemovalSealProperty $p 'SourceIdentityJson')-cne$c.CollectorSourceIdentityJson-or
        $c.CollectorIdentityJson-cne$c.CollectorSourceIdentityJson){
        throw 'Original constructor collector source identity changed; no public repinning.'
    }
    foreach($name in @('FiniteToolPath','FiniteToolSha256','CollectorPath','CollectorSha256',
        'ExecutionMode','ObservationMode','ReadToolPinsJson')){
        $value=Get-RemovalSealProperty $p $name
        if($value-isnot[string]-or-not$value){throw 'Complete original collector source identity required.'}
    }
    if($p.ExecutionMode-cne'RepositoryCollector'-or$p.ObservationMode-cne'Admission'){
        throw 'Original repository admission collector source required.'
    }
    if((ConvertTo-Json -InputObject $c.Inputs -Depth 18 -Compress)-cne$c.InputJson){
        throw 'Original constructor diagnostic input content changed; no collector repinning.'
    }
    # Derive the expected row only from the immutable constructor InputJson.
    # Finish-only synthetic roots may omit this schema; Start/admission cannot.
    $inputs=ConvertFrom-RemovalManifestJson $c.InputJson
    $directory=Get-RemovalSealProperty $inputs 'SourceDirectory'
    $pins=Get-RemovalSealProperty $inputs 'Pins'
    $rows=@((Get-RemovalSealProperty $pins 'Sources')|Where-Object{(Get-RemovalSealProperty $_ 'Name')-ceq'readonly-preflight.ps1'})
    if($directory-isnot[string]-or$directory-cne$PSScriptRoot-or$rows.Count-ne1){
        throw 'Original pinned repository collector source row required.'
    }
    $row=$rows[0]
    if($row.Path-cne(Join-Path $directory 'readonly-preflight.ps1')-or
        $p.CollectorPath-cne$row.Path-or$p.CollectorSha256-ine$row.Sha256){
        throw 'Original collector source pin differs from constructor inputs.'
    }
    $null=Copy-RemovalDecodePin $row 1048576
}

function Bind-RemovalControllerLifetime($Owner,[string]$Kind='Validate',$Resource=$null){
    $c=Get-RemovalControllerLifetime $Owner
    # Creation-specific local results may be captured once. Ordinary observation
    # validates expected nulls too; it must never adopt a caller-injected owner.
    $capture=@()
    switch -CaseSensitive ($Kind){
        'Writer'{$capture=@(@('Writer',(Get-RemovalSealProperty $c.Lease 'RetainedFile')),@('WriterHandle',(Get-RemovalSealProperty $c.Lease 'RetainedHandle')))}
        'Before'{$capture=@(@('BeforeCollector',(Get-RemovalSealProperty $c.Preflight 'RetainedOwner')),@('BeforeDispatch',(Get-RemovalSealProperty $c.Preflight 'DispatchState')))}
        'Managed'{$capture=,@('Managed',$Resource)}
        'Finish'{$capture=,@('Finish',$Resource)}
        'After'{$capture=@(@('After',$Resource),@('AfterPreflight',(Get-RemovalSealProperty $Resource 'RetainedAfterPreflight')))}
        'AfterCollector'{$capture=,@('AfterCollector',(Get-RemovalSealProperty $c.AfterPreflight 'RetainedOwner'))}
        'Probe'{$capture=,@('Probe',$Resource)}
        'Validate'{}
        default{throw 'Unknown controller creation seam.'}
    }
    foreach($pair in $capture){
        $name=$pair[0];$actual=$pair[1]
        if($null-eq$c.$name){$c.$name=$actual}
        elseif(-not[Object]::ReferenceEquals($c.$name,$actual)){$c.Error='Original closed-run lifetime resource replaced: '+$name}
    }
    foreach($pair in @(@('Managed',$Owner.RetainedManaged),@('Finish',$Owner.RetainedFinish),@('After',$Owner.RetainedAfter))){
        if(-not[Object]::ReferenceEquals($c.($pair[0]),$pair[1])){$c.Error='Unexpected first or replaced controller resource: '+$pair[0]}
    }
    if($Kind-ceq'Managed'-and$null-ne$c.Managed){
        if($null-eq$c.Task){
            $c.Task=$c.Managed.TaskOwner
            foreach($name in @('finite-tool.ps1','task-boundary.ps1','probe-boundary.ps1','worker-boundary.ps1','worker.ps1')){
                $path=Join-Path $c.Managed.RunDirectory $name
                $pin=Read-RemovalDecodeFile $path 1048576
                $c.StagedScriptPins+=,[pscustomobject]@{Path=$pin.Path;Bytes=$pin.Bytes;Sha256=$pin.Sha256}
            }
        }elseif(-not[Object]::ReferenceEquals($c.Task,$c.Managed.TaskOwner)){$c.Error='Original closed-run task replaced.'}
    }
    if($null-ne$c.Managed-and-not[Object]::ReferenceEquals($c.Probe,$c.Managed.ProbeOwner)){
        $c.Error='Unexpected first or replaced probe owner.'
    }
    if($Kind-ceq'Probe'-and$null-ne$c.Task){
        if($null-eq$c.WorkerProcess){$c.WorkerProcess=$c.Task.WorkerProcess}
        elseif(-not[Object]::ReferenceEquals($c.WorkerProcess,$c.Task.WorkerProcess)){$c.Error='Original worker process replaced.'}
        if($null-ne$c.Probe){
            if($null-eq$c.ProbeProcess){$c.ProbeProcess=$c.Probe.Process}
            elseif(-not[Object]::ReferenceEquals($c.ProbeProcess,$c.Probe.Process)){$c.Error='Original probe process replaced.'}
        }
    }
}

function Assert-RemovalClosedLifetimeIdentity($c){
    $r=$c.Root;$self=[Diagnostics.Process]::GetCurrentProcess()
    if($c.Error-or$self.Id-ne$c.Pid-or$self.StartTime.ToUniversalTime().Ticks-ne$c.Creation-or
        $r.ControllerPid-ne$c.Pid-or$r.ControllerCreationUtcTicks-ne$c.Creation-or
        $r.RunDirectory-cne$c.Directory-or$r.OriginalDirectory-cne$c.Directory-or$r.Nonce-cne$c.Nonce-or$r.ConfigJson-cne$c.ConfigJson){
        throw 'Original terminal controller identity changed or lifetime capture failed.'
    }
    foreach($pair in @(@('Inputs','RetainedInputs',$c.Inputs),@('Lease','RetainedLease',$c.Lease),
        @('BeforeReadiness','RetainedBefore',$c.Before),@('TraceSession','RetainedTrace',$c.Trace),
        @('ManagedSession','RetainedManaged',$c.Managed),@('Finish','RetainedFinish',$c.Finish),@('AfterAudit','RetainedAfter',$c.After))){
        if(-not[Object]::ReferenceEquals($r.($pair[0]),$pair[2])-or-not[Object]::ReferenceEquals($r.($pair[1]),$pair[2])){
            throw ('Original closed controller reference changed: '+$pair[0])
        }
    }
    if(-not[Object]::ReferenceEquals($r.OriginalNativeOwner,$c.Native)-or
        -not[Object]::ReferenceEquals($r.OriginalReadback,$c.Readback)-or
        -not[Object]::ReferenceEquals($c.Trace.NativeOwner,$c.Native)-or
        -not[Object]::ReferenceEquals($c.Trace.Readback,$c.Readback)-or
        -not[Object]::ReferenceEquals($c.Before.RetainedPreflight,$c.Preflight)){
        throw 'Original closed trace or baseline owner replaced.'
    }
    if($null-ne$c.Managed-and(-not[Object]::ReferenceEquals($c.Managed.TaskOwner,$c.Task)-or
        -not[Object]::ReferenceEquals($c.Managed.ProbeOwner,$c.Probe))){throw 'Original closed task/probe replaced.'}
    if(($null-ne$c.Task-and-not[Object]::ReferenceEquals($c.Task.WorkerProcess,$c.WorkerProcess))-or
        ($null-ne$c.Probe-and-not[Object]::ReferenceEquals($c.Probe.Process,$c.ProbeProcess))){
        throw 'Original retained process reference changed.'
    }
    if($null-ne$c.After-and-not[Object]::ReferenceEquals($c.After.RetainedAfterPreflight,$c.AfterPreflight)){
        throw 'Original closed AFTER preflight replaced.'
    }
    foreach($pair in @(@($c.Preflight,$c.BeforeCollector),@($c.AfterPreflight,$c.AfterCollector))){
        if($null-ne$pair[0]-and-not[Object]::ReferenceEquals($pair[0].RetainedOwner,$pair[1])){throw 'Original collector replaced.'}
    }
}

function Assert-RemovalControllerSealedIdentity($Owner){
    Assert-RemovalClosedLifetimeIdentity (Get-RemovalControllerLifetime $Owner)
}

function Close-RemovalDiagnosticReleaseHandles($Owner){
    $c=Get-RemovalControllerLifetime $Owner;Assert-RemovalClosedLifetimeIdentity $c
    if(-not$c.ReleaseAttempted){return}
    if($c.ReleaseHandlesClosed){return}
    # Only the existing settlement/result/writer seal can end these reads.
    # Admission expiry and changed external inputs never relinquish ownership.
    if($null-eq$c.SealJson-or$c.SealJson-cne$c.PreparedJson-or$null-eq$c.ResultPin-or
        -not$c.Lease.Closed-or$c.Lease.Owned-or$null-eq$c.WriterHandle-or
        -not$c.WriterHandle.IsClosed-or$c.Writer.CanWrite){
        throw 'Original settled persisted controller and closed writer required before releasing sources.'
    }
    $null=Copy-RemovalDecodePin $c.ResultPin 1048576
    foreach($handle in $c.ReleaseHandles){$handle.Dispose()}
    $c.ReleaseHandlesClosed=$true
}

function Assert-RemovalDiagnosticReleaseHandlesClosed($c){
    if(-not$c.ReleaseAttempted){return}
    if(-not$c.ReleaseHandlesClosed-or$c.ReleaseSafeHandles.Count-ne$c.ReleaseHandles.Count){
        throw 'Original source handles are not confirmed closed; no terminal/export authority.'
    }
    foreach($handle in $c.ReleaseSafeHandles){
        if(-not$handle.IsClosed){throw 'Original source handle is still open; no terminal/export authority.'}
    }
}

function Confirm-RemovalControllerEarlyRefusal($Owner){
    $c=Get-RemovalControllerLifetime $Owner;Assert-RemovalClosedLifetimeIdentity $c
    if($c.Before.StartAttempted-or$c.Native.StartAttempted-or$null-ne$c.Managed-or$c.Lease.Owned-or
        ($null-ne$c.WriterHandle-and-not$c.WriterHandle.IsClosed)){
        throw 'Early refusal still retains acquired work; no terminal attestation.'
    }
    $c.EarlyRefused=$true
}

function Observe-RemovalClosedCollectorDrain($Collector,[int]$ObservationMilliseconds){
    if($null-eq$Collector-or$Collector.Closed){return}
    $null=Observe-RemovalCollectorWorker $Collector -ObservationMilliseconds $ObservationMilliseconds
    if(-not$Collector.Completed){return}
    foreach($tool in $Collector.ToolOwners){
        if($tool.Closed){continue}
        $null=Observe-FiniteTool $tool -ObservationMilliseconds $ObservationMilliseconds
        if($tool.ExitedConfirmed-and$tool.Copies.Count-eq2-and$tool.Flushes.Count-eq2-and
            $null-ne$tool.CompletionObservedMilliseconds){
            $settled=$true
            foreach($task in @($tool.Copies)+@($tool.Flushes)){
                if(-not$task.IsCompleted){$settled=$false}else{$null=$task.GetAwaiter().GetResult()}
            }
            if($settled){Close-FiniteToolOwner $tool}
        }
    }
    if(@($Collector.ToolOwners|Where-Object{-not$_.Closed}).Count-eq0){Close-RemovalCollectorWorker $Collector}
}

function Test-RemovalControllerDrainOnly($Owner,[int]$ObservationMilliseconds){
    $c=Get-RemovalControllerLifetime $Owner
    if(-not$c.DrainOnly){
        try{
            Assert-RemovalClosedLifetimeIdentity $c
            if($Owner.Completed-and($null-eq$c.SealJson-or-not$c.Lease.Closed-or$c.Lease.Owned-or
                -not$c.WriterHandle.IsClosed-or$c.Writer.CanWrite)){
                throw 'Public controller completion lacks original closed-writer attestation.'
            }
            if($Owner.Completed){Assert-RemovalDiagnosticReleaseHandlesClosed $c}
            if($Owner.Refused-and-not$c.EarlyRefused){throw 'Public controller refusal lacks known pre-acquisition attestation.'}
            return $false
        }
        catch{$c.Error=$_.Exception.Message;$c.DrainOnly=$true}
    }
    # Identity refusal is sticky and cannot strand original handles. No receipt
    # binding, admission, new Finish, STOP or retirement is authorized here.
    if($null-ne$c.Managed-and-not$c.Managed.Error){$c.Managed.Error=$c.Error}
    Add-RemovalControllerErrorRecord $Owner $c.Error
    foreach($name in @('Worker','Probe')){
        $process=$c.($name+'Process');$observation=[pscustomobject]@{ExitKnown=$false;ExitCode=$null;Error=$null}
        try{
            if($null-ne$process){
                $process.Refresh()
                if($process.HasExited){$observation.ExitKnown=$true;$observation.ExitCode=[int]$process.ExitCode}
            }
            if($name-ceq'Probe'-and$null-ne$c.Probe-and[Object]::ReferenceEquals($c.Probe.Process,$process)){
                $null=Observe-RemovalProbe $c.Probe -ObservationMilliseconds $ObservationMilliseconds
            }
        }catch{$observation.Error=$_.Exception.Message}
        $c.($name+'Drain')=$observation
    }
    foreach($collector in @($c.BeforeCollector,$c.AfterCollector)){
        try{Observe-RemovalClosedCollectorDrain $collector $ObservationMilliseconds}
        catch{Add-RemovalControllerErrorRecord $Owner $_.Exception.Message}
    }
    # Wrapper evidence may be persisted only while it still points at the
    # captured collector. Never follow a substituted nested public alias.
    if($null-ne$c.After-and$c.After.StartAttempted-and
        [Object]::ReferenceEquals($c.After.RetainedAfterPreflight,$c.AfterPreflight)-and
        [Object]::ReferenceEquals($c.AfterPreflight.RetainedOwner,$c.AfterCollector)-and
        [Object]::ReferenceEquals($c.AfterPreflight.CollectorOwner,$c.AfterCollector)){
        try{$null=Observe-RemovalSafetyAudit $c.After -ObservationMilliseconds $ObservationMilliseconds}
        catch{Add-RemovalControllerErrorRecord $Owner $_.Exception.Message}
    }
    if($null-ne$c.Finish){
        try{$null=Observe-RemovalFinishTools $c.Finish $ObservationMilliseconds}
        catch{Add-RemovalControllerErrorRecord $Owner $_.Exception.Message}
    }
    return $true
}

function Prepare-RemovalClosedControllerSeal($Owner){
    $c=Get-RemovalControllerLifetime $Owner
    if($null-ne$c.PreparedJson-or$null-ne$c.SealJson){throw 'Original completion seal already attempted; no repinning.'}
    Assert-RemovalClosedLifetimeIdentity $c
    if(-not$Owner.ResultAttempted-or$null-eq$Owner.ResultPin-or-not$Owner.BeforeSettled-or
        -not(Test-RemovalControllerTraceStopped $Owner)){throw 'Persisted result and original settled trace/baseline required.'}
    $lease=$c.Lease
    if(-not$lease.Owned-or$lease.Closed-or$c.Writer-isnot[IO.FileStream]-or
        -not[Object]::ReferenceEquals($lease.RetainedFile,$c.Writer)-or
        -not[Object]::ReferenceEquals($lease.RetainedHandle,$c.WriterHandle)-or$c.WriterHandle.IsClosed){
        throw 'Original live writer must persist through terminal sealing.'
    }
    foreach($collector in @($c.BeforeCollector,$c.AfterCollector)){
        if($null-eq$collector){continue}
        $noCall=(Get-RemovalSealProperty $collector 'KnownNoDispatch')-eq$true
        if($noCall){
            if(-not$collector.Completed-or-not$collector.Closed-or$collector.EndInvoked-or$collector.LaunchAttempted-or
                $null-ne$collector.Invocation-or$collector.ToolOwners.Count){throw 'Original no-call collector has ambiguous dispatch.'}
        }elseif(-not$collector.Completed-or-not$collector.Closed-or-not$collector.EndInvoked-or
            $null-eq$collector.Invocation-or-not$collector.Invocation.IsCompleted){throw 'Original collector invocation remains pending.'}
        foreach($tool in $collector.ToolOwners){
            # Capped or failed output stays unsafe, but a naturally exited and
            # drained original process must not strand the writer forever.
            if(-not$tool.Closed-or-not$tool.ExitedConfirmed-or$null-eq$tool.CompletionObservedMilliseconds-or
                $tool.Copies.Count-ne2-or$tool.Flushes.Count-ne2){throw 'Original collector child remains unresolved.'}
            foreach($task in @($tool.Copies)+@($tool.Flushes)){
                if(-not$task.IsCompleted){throw 'Original collector output remains pending.'}
                $null=$task.GetAwaiter().GetResult()
            }
        }
    }
    if($c.Preflight.StartAttempted-and$null-eq$c.BeforeCollector){
        $state=$c.BeforeDispatch
        if($null-eq$state-or$state.Entered-isnot[bool]-or-not$state.Entered-or
            $state.BeginInvokeAttempted-isnot[bool]-or$state.BeginInvokeAttempted-or$null-ne$state.Owner){
            throw 'Original baseline dispatch unknown; missing output never establishes absence.'
        }
    }
    if($null-ne$c.After-and($null-eq$c.AfterCollector-or-not$c.After.Complete)){throw 'Original AFTER evidence is not terminal.'}
    if($null-ne$c.Task-and$c.Task.RegistrationAttempted-and(-not$c.Task.RegistrationKnown-or-not$c.Task.Retired)){
        throw 'Original task registration/retirement remains unresolved.'
    }
    if($null-ne$c.Finish){
        # Retirement has already deleted the task. Reuse its retained final
        # observation and independently recheck the original process handles;
        # querying a deleted task RCW cannot establish a fresh scheduler state.
        $terminal=Complete-RemovalProbeTerminal $c.Task $c.Task.LastObservation $c.Probe $c.Managed.LastResult
        if(-not$c.Finish.Terminal-or$null-eq$c.Finish.ResultPin-or$terminal.Terminal-isnot[bool]-or
            -not$terminal.Terminal-or$terminal.NeedsObservation-or$terminal.Owners.Count){
            throw 'Original managed finish/task/probe chain remains unresolved.'
        }
        if(-not(Observe-RemovalFinishTools $c.Finish 1)){throw 'Original decoder children remain unresolved.'}
    }
    $pins=New-Object Collections.Generic.List[object]
    foreach($p in @($Owner.ResultPin,$c.Before.ReceiptPin,$c.Preflight.ReceiptPin,$Owner.AbortEvidencePin,
        $(if($null-ne$c.After){$c.After.ReceiptPin}),$(if($null-ne$c.AfterPreflight){$c.AfterPreflight.ReceiptPin}),
        $(if($null-ne$c.Finish){$c.Finish.ResultPin}),$(if($null-ne$c.Finish){$c.Finish.ReceiptPin}))){
        if($null-ne$p){$pins.Add((Copy-RemovalDecodePin $p 16777216 -AllowEmpty))}
    }
    if($null-ne$c.Task-and$c.Task.Retired){
        $p=Read-RemovalDecodeFile $c.Task.RetiredXmlPath 1048576
        if($p.Sha256-cne$c.Task.RetiredXmlSha256){throw 'Original retired task XML changed.'}
        $pins.Add([pscustomobject]@{Path=$p.Path;Bytes=$p.Bytes;Sha256=$p.Sha256})
    }
    if($null-ne$c.Finish){
        $d=$c.Finish.RetainedDecode
        if($null-eq$d-or$null-eq$d.ReceiptPin){throw 'Original final decode evidence missing.'}
        $pins.Add((Copy-RemovalDecodePin $d.ReceiptPin 1048576))
        $evidence=(Read-RemovalDecodeFile $c.Finish.ReceiptPin.Path 1048576 -Text).Text|ConvertFrom-Json
        foreach($p in @($evidence.OutputPins)+@($c.Finish.ToolLogPins)){
            $pins.Add((Copy-RemovalDecodePin $p 16777216 -AllowEmpty))
        }
    }
    $result=Read-RemovalDecodeFile $Owner.ResultPin.Path 1048576 -Text
    $saved=$result.Text|ConvertFrom-Json
    if($saved.ControllerComplete-isnot[bool]-or-not$saved.ControllerComplete-or$saved.Nonce-cne$c.Nonce-or
        $saved.NativePassed-isnot[bool]-or$saved.NativePassed-or$saved.Qualified-isnot[bool]-or$saved.Qualified){
        throw 'Original result cannot establish terminal identity or promote qualification.'
    }
    $c.ResultPin=$Owner.ResultPin;$c.ResultPinJson=ConvertTo-Json -InputObject $Owner.ResultPin -Compress
    $inputSnapshot=$c.InputJson|ConvertFrom-Json
    $sourcePins=Get-RemovalSealProperty (Get-RemovalSealProperty $inputSnapshot 'Pins') 'Sources'
    $c.PreparedJson=ConvertTo-Json -InputObject ([ordered]@{Directory=$c.Directory;Nonce=$c.Nonce;ResultPin=$Owner.ResultPin
        ResultJson=$result.Text;ManifestPin=$inputSnapshot.ManifestPin;SourcePins=@($sourcePins);TerminalPins=@($pins.ToArray())
        InputJson=$c.InputJson;CollectorProvenance=$c.Preflight.ExecutionMode;StagedScriptPins=@($c.StagedScriptPins)}) -Depth 22 -Compress
}

function Complete-RemovalClosedControllerSeal($Owner){
    $c=Get-RemovalControllerLifetime $Owner;Assert-RemovalClosedLifetimeIdentity $c
    if($null-eq$c.PreparedJson-or$null-ne$c.SealJson-or-not$c.Lease.Closed-or$c.Lease.Owned-or
        -not$c.WriterHandle.IsClosed-or$c.Writer.CanWrite){throw 'Original writer closure is not confirmed; no closed-run export.'}
    $c.SealJson=$c.PreparedJson
    Close-RemovalDiagnosticReleaseHandles $Owner
    Assert-RemovalDiagnosticReleaseHandlesClosed $c
}

function Get-RemovalClosedControllerSeal {
    param([Parameter(Mandatory=$true,Position=0)]$Controller)
    $c=Get-RemovalControllerLifetime $Controller;Assert-RemovalClosedLifetimeIdentity $c
    Assert-RemovalDiagnosticReleaseHandlesClosed $c
    if($null-eq$c.SealJson-or-not$Controller.Completed-or$Controller.Phase-cne'Complete'-or
        -not[Object]::ReferenceEquals($Controller.ResultPin,$c.ResultPin)-or
        (ConvertTo-Json -InputObject $Controller.ResultPin -Compress)-cne$c.ResultPinJson-or
        -not$c.Lease.Closed-or$c.Lease.Owned-or-not$c.WriterHandle.IsClosed-or$c.Writer.CanWrite){
        throw 'Original persisted terminal result and closed writer seal required; pending/public flags grant no export.'
    }
    $value=$c.SealJson|ConvertFrom-Json
    foreach($pin in $value.TerminalPins){$null=Copy-RemovalDecodePin $pin 16777216 -AllowEmpty}
    return $value
}
