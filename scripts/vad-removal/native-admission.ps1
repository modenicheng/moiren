#requires -Version 5.1
# Admission is a retained controller/resource association, never a public bool.
# Conditional release belongs only to the original constructor and locked
# independently reviewed source bundle. It is not native qualification.
Set-StrictMode -Version 2
if($null -eq (Get-Variable -Name RemovalOperationContexts -Scope Script -ErrorAction SilentlyContinue)){
    $script:RemovalOperationContexts=New-Object 'Runtime.CompilerServices.ConditionalWeakTable[object,object]'
}

function Assert-RemovalDiagnosticRelease {
    param($Owner)
    $ErrorActionPreference='Stop'
    $c=Get-RemovalControllerLifetime $Owner
    Assert-RemovalControllerSealedIdentity $Owner
    if($c.ReleaseAttempted){throw 'Original release already attempted; retain its locks, never reacquire.'}
    Assert-RemovalDiagnosticAdmission $Owner
    if(Test-RemovalControllerDeadlineExpired $Owner){throw 'Original finite diagnostic admission expired.'}
    if(-not$Owner.StartAttempted-or-not[Object]::ReferenceEquals($Owner.Clock,$c.Clock)-or
        $c.DeadlineMilliseconds-ne240000-or$Owner.DeadlineMilliseconds-ne240000-or
        $c.Clock-isnot[Diagnostics.Stopwatch]-or
        $c.Native.GetType().FullName-cne'Moiren.RemovalTraceNativeV1.Owner'-or
        $null-ne$c.Native.Boundary-or$null-ne$c.Trace.QueryBoundary-or
        $c.Native.StartAttempted-or$c.Trace.StartAttempted-or
        $c.Preflight.ExecutionMode-cne'RepositoryCollector'){
        throw 'Fresh original repository controller and unused noninjected native path required.'
    }
    $c.ReleaseAttempted=$true
    try{
        # Expected bytes/hash/text come only from the constructor snapshot.
        # Hold every successful open even on later failure; terminal sealing,
        # not a catch/freshness check, releases this original owner work.
        $inputs=$c.Inputs
        $null=Read-RemovalManifestPinnedFile $inputs.ManifestPin 1048576 $c.ReleaseHandles
        $file=Read-RemovalDecodeFile $inputs.ManifestPin.Path 1048576 -Text
        if($file.Sha256-ine$inputs.ExpectedSha256-or$file.Text-cne$inputs.ManifestJson-or
            $inputs.ManifestPath-cne$inputs.ManifestPin.Path){throw 'Original independently pinned manifest content differs.'}
        $manifest=ConvertFrom-RemovalManifestJson $file.Text
        Assert-RemovalManifestFields $manifest @('SchemaVersion','SourcesRevision','SourceProvenance','Sources','Probe','Driver','Schedule','Tools','Symbols')
        $sources=Copy-RemovalRunSources $manifest.Sources $inputs.SourceDirectory
        foreach($row in $sources){
            $pin=[pscustomobject][ordered]@{Path=$row.Path;Bytes=$row.Bytes;Sha256=$row.Sha256}
            $null=Read-RemovalManifestPinnedFile $pin 1048576 $c.ReleaseHandles
        }
        $provenance=Read-RemovalManifestSourceProvenance $manifest.SourceProvenance $sources $manifest.SourcesRevision $c.ReleaseHandles
        $c.ReleaseHandlePins=@($inputs.ManifestPin)+@($sources)+@($provenance.BundleManifestPin)
        # All dependency hashes and current readiness are checked again while
        # original manifest/bundle/source paths exclude both writers and delete.
        $c.ReleaseGranted=$true
        Assert-RemovalDiagnosticAdmission $Owner
        if(Test-RemovalControllerDeadlineExpired $Owner){throw 'Diagnostic expired during source lock acquisition.'}
    }catch{
        $c.ReleaseGranted=$false;$c.ReleaseError=$_.Exception.Message
        throw
    }finally{
        # Capture before Dispose: Windows PowerShell's disposed FileStream
        # getter cannot be used later as proof that its original handle closed.
        $c.ReleaseSafeHandles=@($c.ReleaseHandles|ForEach-Object{$_.SafeFileHandle})
    }
}

function Get-RemovalOperationContext($Resource) {
    $context=$null
    if($null -eq $Resource -or -not$script:RemovalOperationContexts.TryGetValue($Resource,[ref]$context)){
        throw 'No original retained diagnostic operation context; detached approval is not authority.'
    }
    return $context
}

function Get-RemovalOperationDecodeStaticJson($Pins) {
    ConvertTo-Json -InputObject ([ordered]@{DriverPackage=$Pins.DriverPackage;Tracefmt=$Pins.Tracefmt
        Python=$Pins.Python;Reader=$Pins.Reader;Probe=$Pins.Probe;Symbols=$Pins.Symbols}) -Depth 14 -Compress
}

function Assert-RemovalOperationLifetime($Context) {
    Assert-RemovalTaskHost
    $r=$Context.Root;$self=[Diagnostics.Process]::GetCurrentProcess()
    if($self.Id -ne $Context.ControllerPid -or $self.StartTime.ToUniversalTime().Ticks -ne $Context.ControllerCreationUtcTicks -or
        $r.ControllerPid -ne $Context.ControllerPid -or $r.ControllerCreationUtcTicks -ne $Context.ControllerCreationUtcTicks -or
        $r.ConfigJson -cne $Context.ConfigJson -or $r.OriginalDirectory -cne $Context.Directory -or
        $r.RunDirectory -cne $Context.Directory -or $r.Nonce -cne $Context.Nonce -or
        -not[Object]::ReferenceEquals($r.RetainedLease,$Context.Lease) -or
        -not[Object]::ReferenceEquals($r.RetainedInputs,$Context.Inputs) -or
        -not[Object]::ReferenceEquals($r.RetainedTrace,$Context.Trace) -or
        -not[Object]::ReferenceEquals($r.OriginalNativeOwner,$Context.Native) -or
        -not[Object]::ReferenceEquals($r.OriginalReadback,$Context.Readback) -or
        -not[Object]::ReferenceEquals($r.RetainedManaged,$Context.Managed) -or
        -not[Object]::ReferenceEquals($Context.Trace.NativeOwner,$Context.Native) -or
        -not[Object]::ReferenceEquals($Context.Trace.Readback,$Context.Readback) -or
        -not[Object]::ReferenceEquals($Context.Readback.Trace,$Context.Native) -or
        -not[Object]::ReferenceEquals($Context.Managed.TaskOwner,$Context.Task) -or
        -not[Object]::ReferenceEquals($Context.Managed.TraceSession,$Context.Trace) -or
        -not[Object]::ReferenceEquals($Context.Managed.TraceReadback,$Context.Readback)){
        throw 'Original operation controller/resource identity changed.'
    }
    # Teardown depends on the live original writer, not fresh BEFORE/Explorer,
    # changed source bytes, lease-file contents, or expired capture admission.
    $lease=$Context.Lease;Assert-RemovalLeaseController $lease
    if($lease.Closed -or -not$lease.Owned -or -not[Object]::ReferenceEquals($lease.RetainedFile,$Context.Writer) -or
        -not[Object]::ReferenceEquals($lease.RetainedHandle,$Context.WriterHandle) -or
        $Context.Writer -isnot [IO.FileStream] -or $null -eq $Context.WriterHandle -or
        $Context.WriterHandle.IsClosed -or $Context.WriterHandle.IsInvalid -or -not$Context.Writer.CanWrite){
        throw 'Original operation writer lifetime is closed or replaced.'
    }
}

function Assert-RemovalOperationBirth($Context) {
    Assert-RemovalOperationLifetime $Context
    $r=$Context.Root
    if(-not[Object]::ReferenceEquals($r.Clock,$Context.Clock) -or $r.DeadlineMilliseconds -ne $Context.DeadlineMilliseconds -or
        (Test-RemovalControllerDeadlineExpired $r) -or
        $r.Completed -or $r.TimedOut -or $r.Errors.Count){throw 'Original finite diagnostic admission expired or closed.'}
    Assert-RemovalDiagnosticAdmission $r
    Assert-RemovalRunInputs $Context.Inputs
    if((ConvertTo-Json -InputObject $Context.Task.Spec -Depth 8 -Compress) -cne $Context.TaskSpecJson -or
        (ConvertTo-Json -InputObject $Context.Managed.Config -Depth 10 -Compress) -cne $Context.ManagedConfigJson){
        throw 'Original task/probe configuration changed before native birth.'
    }
    # File hashing is synchronous and not a hard process deadline. Recheck
    # after it returns so expensive validation cannot authorize a late birth.
    if((Test-RemovalControllerDeadlineExpired $r) -or
        $r.Completed -or $r.TimedOut -or $r.Errors.Count){throw 'Diagnostic expired during synchronous source validation.'}
    $preflight=$r.RetainedBefore.RetainedPreflight
    if($null -ne $preflight.PSObject.Properties['SnapshotJson']){
        # PS7 can coerce JSON date strings into DateTime. Keep the original
        # already-validated collector snapshot's explicit-zone string instead.
        $observed=Convert-RemovalPreflightTime $preflight.RetainedOwner.Shared.Snapshot.ObservedUtc
        $age=([DateTimeOffset]::UtcNow-$observed).TotalSeconds
        if($age -lt 0 -or $age -gt 120){throw 'Original BEFORE became stale during synchronous source validation.'}
    }
}

function Initialize-RemovalNativeContext {
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    Assert-RemovalDiagnosticRelease $Owner
    Assert-RemovalDiagnosticAdmission $Owner;Assert-RemovalRunInputs $Owner.RetainedInputs
    $m=$Owner.RetainedManaged;$n=$Owner.OriginalNativeOwner;$t=$Owner.RetainedTrace
    if(-not$Owner.StartAttempted -or $Owner.DeadlineMilliseconds -ne 240000 -or
        $Owner.Clock -isnot [Diagnostics.Stopwatch] -or (Test-RemovalControllerDeadlineExpired $Owner) -or
        $null -eq $m -or $null -eq $m.TaskOwner -or $null -ne $m.ProbeOwner -or
        $n.GetType().FullName -cne 'Moiren.RemovalTraceNativeV1.Owner' -or $null -ne $n.Boundary -or
        $null -ne $t.QueryBoundary -or $n.StartAttempted -or $t.StartAttempted -or
        $m.TaskOwner.RegistrationAttempted -or $m.TaskOwner.DispatchAttempted -or
        $Owner.RetainedBefore.RetainedPreflight.ExecutionMode -cne 'RepositoryCollector'){
        throw 'Fresh staged repository controller and unused real native owners required.'
    }
    $self=[Diagnostics.Process]::GetCurrentProcess();$dir=$Owner.OriginalDirectory
    if($m.RunDirectory -cne (Join-Path $dir 'managed') -or $m.Config.RunDirectory -cne $m.RunDirectory -or
        $m.TaskOwner.Spec.RunDirectory -cne $m.RunDirectory -or $m.TaskOwner.Spec.Nonce -cne $Owner.Nonce -or
        $m.Config.Nonce -cne $Owner.Nonce -or $n.Session -cne ('MoirenRemoval613'+$Owner.Nonce) -or
        $n.EtlPath -cne (Join-Path $dir 'driver.etl') -or
        $m.ControllerPid -ne $self.Id -or $m.ControllerCreationUtcTicks -ne $self.StartTime.ToUniversalTime().Ticks -or
        $m.Config.ControllerPid -ne $m.ControllerPid -or $m.Config.ControllerCreationUtcTicks -ne $m.ControllerCreationUtcTicks -or
        $m.Config.Executable -cne $Owner.RetainedInputs.Pins.Probe.File.Path -or
        $m.Config.ExpectedSha256 -cne $Owner.RetainedInputs.Pins.Probe.File.Sha256 -or
        $m.TaskOwner.Spec.SessionId -ne $Owner.RetainedBefore.RetainedUserSession.Snapshot.SessionId -or
        $m.TaskOwner.Spec.Sid -cne $Owner.RetainedBefore.RetainedUserSession.Snapshot.Sid -or
        $m.Config.SessionId -ne $m.TaskOwner.Spec.SessionId -or $m.Config.Sid -cne $m.TaskOwner.Spec.Sid){
        throw 'Staged native resources do not belong to the exact controller/input/session.'
    }
    $resources=@($n,$m.TaskOwner,$m)
    foreach($resource in $resources){$existing=$null
        if($script:RemovalOperationContexts.TryGetValue($resource,[ref]$existing)){throw 'Operation context already bound; never rebind.'}
    }
    $c=[pscustomobject]@{Root=$Owner;ControllerPid=$Owner.ControllerPid;ControllerCreationUtcTicks=$Owner.ControllerCreationUtcTicks
        ConfigJson=$Owner.ConfigJson;Directory=$dir;Nonce=$Owner.Nonce;Lease=$Owner.RetainedLease;Inputs=$Owner.RetainedInputs
        Writer=$Owner.RetainedLease.RetainedFile;WriterHandle=$Owner.RetainedLease.RetainedHandle
        Clock=$Owner.Clock;DeadlineMilliseconds=[long]240000;Trace=$t;Native=$n;Readback=$Owner.OriginalReadback
        Managed=$m;Task=$m.TaskOwner;TaskSpecJson=(ConvertTo-Json -InputObject $m.TaskOwner.Spec -Depth 8 -Compress)
        ManagedConfigJson=(ConvertTo-Json -InputObject $m.Config -Depth 10 -Compress)
        StaticDecodePinsJson=(Get-RemovalOperationDecodeStaticJson $Owner.RetainedInputs.Pins)
        Scheduler=$null;Folder=$null;TaskDefinition=$null;RegisteredTask=$null;RegisteredXml=$null
        Finish=$null;Decode=$null;DecodePinsJson=$null;DecodeConfigJson=$null;Probe=$null;Used=@{}}
    Assert-RemovalOperationLifetime $c
    foreach($resource in $resources){$script:RemovalOperationContexts.Add($resource,$c)}
}

function Register-RemovalNativeDecode {
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    $c=Get-RemovalOperationContext $Owner.OriginalNativeOwner
    Assert-RemovalOperationLifetime $c
    if(-not[Object]::ReferenceEquals($Owner,$c.Root) -or $null -ne $c.Decode){throw 'Original unused diagnostic decode context required.'}
    $f=$Owner.RetainedFinish;$d=$f.RetainedDecode
    Assert-RemovalFinishIdentity $f
    if(-not[Object]::ReferenceEquals($f.Session,$c.Managed) -or -not[Object]::ReferenceEquals($f.Decode,$d) -or
        -not[Object]::ReferenceEquals($d.TraceSession,$c.Trace) -or -not[Object]::ReferenceEquals($d.ManagedSession,$c.Managed) -or
        -not[Object]::ReferenceEquals($d.StopReceipt,$c.Native.StopReceipt) -or
        $d.OutputDirectory -cne (Join-Path $c.Directory 'decode') -or $d.Attempted -or $null -eq (Get-RemovalFinishTerminal $f)){
        throw 'Original post-STOP terminal finish/decode required.'
    }
    $null=Assert-RemovalDecodeStoppedSession $c.Trace;Assert-RemovalDecodeInputs $d
    if((Get-RemovalOperationDecodeStaticJson $d.Pins) -cne $c.StaticDecodePinsJson){
        throw 'Closed decode tools/package/probe/symbols differ from original admitted inputs.'
    }
    $c.Finish=$f;$c.Decode=$d;$c.Probe=$c.Managed.ProbeOwner
    $c.DecodePinsJson=ConvertTo-Json -InputObject $d.Pins -Depth 14 -Compress
    $c.DecodeConfigJson=ConvertTo-Json -InputObject @($d.OutputDirectory,$d.TracePath,$d.SummaryPath,$d.AnalysisPath) -Compress
    $script:RemovalOperationContexts.Add($d,$c)
}

function Assert-RemovalOperationDecode($Context) {
    $d=$Context.Decode
    if($null -eq $d -or -not[Object]::ReferenceEquals($Context.Root.RetainedFinish,$Context.Finish) -or
        -not[Object]::ReferenceEquals($Context.Finish.RetainedDecode,$d) -or
        -not[Object]::ReferenceEquals($Context.Managed.ProbeOwner,$Context.Probe) -or
        (ConvertTo-Json -InputObject $d.Pins -Depth 14 -Compress) -cne $Context.DecodePinsJson -or
        (ConvertTo-Json -InputObject @($d.OutputDirectory,$d.TracePath,$d.SummaryPath,$d.AnalysisPath) -Compress) -cne $Context.DecodeConfigJson){
        throw 'Original closed diagnostic decode association changed.'
    }
    Assert-RemovalFinishIdentity $Context.Finish
    if((ConvertTo-Json -InputObject $Context.Managed.Config -Depth 10 -Compress) -cne $Context.ManagedConfigJson){
        throw 'Original managed argv/configuration changed before closed decode or retirement.'
    }
    $null=Assert-RemovalDecodeStoppedSession $Context.Trace
    Assert-RemovalDecodeInputs $d;$null=Assert-RemovalDecodeManagedReport $d
    Assert-RemovalFinishManagedProvenance $Context.Finish
    if($null -eq (Get-RemovalFinishTerminal $Context.Finish)){throw 'Original managed resources are not terminal.'}
}

function Assert-RemovalOperationRetirement($Context,[string]$Operation) {
    $c=$Context;$task=$c.Task
    Assert-RemovalOperationTaskSeal $c
    if($Operation -ceq 'TaskRetireUnstarted'){
        if($task.DispatchAttempted){throw 'Never-dispatched original task required.'}
    } else {
        if(-not$task.DispatchAttempted){throw 'Original dispatched task required.'}
        # Direct calls to RemoveOwnedTask must not bypass whole-chain terminal
        # checks in CompleteManagedCleanup or delete before evidence persists.
        $state=Get-RemovalTaskObservation $task
        $terminal=Complete-RemovalProbeTerminal $task $state $c.Managed.ProbeOwner $c.Managed.LastResult
        if($null -eq $terminal -or $terminal.Terminal -isnot [bool] -or -not$terminal.Terminal -or
            $terminal.NeedsObservation -or $terminal.Owners -isnot [Array] -or $terminal.Owners.Count -ne 0){
            throw 'Original whole task/worker/probe chain must be independently terminal before retirement.'
        }
    }
    if($c.Native.TraceOwned -and (-not$c.Native.TraceStopped -or $null -eq $c.Native.StopReceipt -or
        -not$c.Native.StopReceipt.StoppedConfirmed)){throw 'Original trace must be confirmed stopped before retirement.'}
    $finish=$c.Root.RetainedFinish
    if($null -ne $finish){
        if(-not[Object]::ReferenceEquals($finish,$c.Finish) -or $null -eq $c.Decode){throw 'Original bound finish/decode required for task retirement.'}
        Assert-RemovalOperationDecode $c
        if($null -eq $finish.ReceiptPin -or $finish.ReceiptPin.Path -cne (Join-Path $c.Decode.OutputDirectory 'finish-evidence.json')){
            throw 'Original persisted finish evidence required before retirement.'
        }
        $null=Copy-RemovalDecodePin $finish.ReceiptPin 1048576
        $saved=(Read-RemovalDecodeFile $finish.ReceiptPin.Path 1048576 -Text).Text|ConvertFrom-Json
        if($saved.Stage -cne 'BeforeTaskRetirement' -or $saved.TaskRetired -isnot [bool] -or $saved.TaskRetired -or
            $saved.ManagedProvenance.Nonce -cne $c.Nonce -or $saved.ManagedProvenance.RunDirectory -cne $c.Managed.RunDirectory){
            throw 'Persisted finish evidence differs from this original task.'
        }
        $null=Copy-RemovalDecodePin $saved.DecodeReceipt 1048576
        foreach($pin in $saved.OutputPins){$null=Copy-RemovalDecodePin $pin 16777216 -AllowEmpty}
    } else {
        $pin=$c.Root.AbortEvidencePin
        if($null -eq $pin -or $pin.Path -cne (Join-Path $c.Directory 'abort-evidence.json')){
            throw 'Original persisted abort evidence required before task retirement.'
        }
        $null=Copy-RemovalDecodePin $pin 1048576
        $saved=(Read-RemovalDecodeFile $pin.Path 1048576 -Text).Text|ConvertFrom-Json
        if($saved.Stage -cne 'BeforeAbortTaskRetirement' -or $saved.Nonce -cne $c.Nonce -or
            $saved.TaskPath -cne $task.Spec.Path -or $saved.TraceStopped -isnot [bool] -or -not$saved.TraceStopped){
            throw 'Persisted abort evidence differs from this original task.'
        }
    }
}

function Assert-RemovalOperationTaskSeal($Context) {
    $c=$Context;$t=$c.Task
    if((ConvertTo-Json -InputObject $t.Spec -Depth 8 -Compress) -cne $c.TaskSpecJson -or
        $null -eq $c.RegisteredTask -or -not$c.Used.ContainsKey('TaskRegistered') -or
        -not[Object]::ReferenceEquals($t.Scheduler,$c.Scheduler) -or
        -not[Object]::ReferenceEquals($t.Folder,$c.Folder) -or
        -not[Object]::ReferenceEquals($t.TaskDefinition,$c.TaskDefinition) -or
        -not[Object]::ReferenceEquals($t.RegisteredTask,$c.RegisteredTask) -or
        $t.RegisteredXml -cne $c.RegisteredXml -or $t.RegisteredTask.Xml -cne $c.RegisteredXml){
        throw 'Original registered task COM owners/specification/XML changed or not sealed.'
    }
}

function Assert-RemovalNativeOperation {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)]$Resource,[Parameter(Mandatory=$true)][string]$Operation,$Request=$null)
    $ErrorActionPreference='Stop';$c=Get-RemovalOperationContext $Resource
    Assert-RemovalOperationLifetime $c
    $n=$c.Native;$task=$c.Task;$key=$null
    $birth=$Operation -in @('TraceStart','TraceStartDispatch','TraceEnumerate','TraceEnable','TraceReadback','TaskRegister','TaskRegisterDispatch','TaskDispatch','TaskDispatchInvoke','ManagedAdmit')
    if($birth){Assert-RemovalOperationBirth $c}
    $traceOperation=$Operation -like 'Trace*'
    $taskOperation=$Operation -like 'Task*'
    if(($traceOperation -and -not[Object]::ReferenceEquals($Resource,$n)) -or
        ($taskOperation -and -not[Object]::ReferenceEquals($Resource,$task)) -or
        ($Operation -in @('ManagedAdmit','ManagedRetire') -and -not[Object]::ReferenceEquals($Resource,$c.Managed)) -or
        ($Operation -like 'Decode*' -and -not[Object]::ReferenceEquals($Resource,$c.Decode))){throw 'Operation does not match its original resource.'}
    switch -CaseSensitive ($Operation){
        'TraceStart'{if($n.StartAttempted){throw 'Native start already attempted.'};$key=$Operation}
        'TraceStartDispatch'{if(-not$n.StartAttempted -or $null -ne $n.StartStatus){throw 'Original pending native start required.'};$key=$Operation}
        'TraceEnumerate'{if(-not$n.TraceOwned -or -not$n.ProviderIdleAttempted -or $n.EnableAttempted -or $n.Failed){throw 'Original fresh provider-idle attempt required.'};$key=$Operation}
        'TraceEnable'{if(-not$n.TraceOwned -or -not$n.ProviderIdleConfirmed -or -not$n.EnableAttempted -or $null -ne $n.EnableStatus -or $n.Failed){throw 'Original pending provider enable required.'};$key=$Operation}
        'TraceReadback'{if(-not$n.TraceOwned -or $n.EnableStatus -ne 0 -or $n.Failed -or $n.StopQueryAttempted -or $n.TraceStopped){throw 'Live original enabled trace required.'}}
        'TraceStopPrepare'{if(-not$n.TraceOwned -or $n.StopQueryAttempted -or $n.StopAttempted -or $n.TraceStopped){throw 'Original successful Start required for once-only cleanup.'};$key=$Operation}
        'TraceStopQuery'{if(-not$n.TraceOwned -or -not$n.StopQueryAttempted -or $null -ne $n.StopQueryStatus -or $n.StopAttempted){throw 'Original pending pre-STOP query required.'};$key=$Operation}
        'TraceStop'{if(-not$n.TraceOwned -or -not$n.StopAttempted -or $n.StopQueryStatus -ne 0 -or $null -ne $n.StopStatus){throw 'Original exact pending STOP required.'};$key=$Operation}
        'TaskRegister'{
            if(-not$n.TraceOwned -or $n.Failed -or $n.TraceStopped -or -not(Test-RemovalTraceCaptureAdmission $c.Readback) -or
                $task.RegistrationAttempted -or $task.Error -or $task.Closed){throw 'Unused task and admitted owned trace required.'};$key=$Operation
        }
        'TaskRegisterDispatch'{
            if(-not$c.Used.ContainsKey('TaskRegister') -or $task.RegistrationAttempted -or $task.RegistrationKnown -or
                $null -eq $task.Scheduler -or $null -eq $task.Folder -or $null -eq $task.TaskDefinition -or
                $null -ne $task.RegisteredTask -or $null -ne $c.Scheduler){throw 'Original pending create-only registration required.'}
            $c.Scheduler=$task.Scheduler;$c.Folder=$task.Folder;$c.TaskDefinition=$task.TaskDefinition;$key=$Operation
        }
        'TaskRegistered'{
            if(-not$c.Used.ContainsKey('TaskRegisterDispatch') -or $task.RegistrationKnown -or
                $null -eq $task.RegisteredTask -or $task.RegisteredXml -isnot [string] -or -not$task.RegisteredXml -or
                $task.RegisteredTask.Xml -cne $task.RegisteredXml -or
                -not[Object]::ReferenceEquals($task.Scheduler,$c.Scheduler) -or
                -not[Object]::ReferenceEquals($task.Folder,$c.Folder) -or
                -not[Object]::ReferenceEquals($task.TaskDefinition,$c.TaskDefinition) -or
                (ConvertTo-Json -InputObject $task.Spec -Depth 8 -Compress) -cne $c.TaskSpecJson){
                throw 'Original create-only registration result required; no post-hoc adoption.'
            }
            $c.RegisteredTask=$task.RegisteredTask;$c.RegisteredXml=$task.RegisteredXml;$key=$Operation
        }
        'TaskDispatch'{
            Assert-RemovalOperationTaskSeal $c
            if(-not$n.TraceOwned -or $n.Failed -or $n.TraceStopped -or -not(Test-RemovalTraceCaptureAdmission $c.Readback) -or
                -not$task.RegistrationKnown -or $task.DispatchAttempted -or $task.Error -or $task.Closed){throw 'Verified unused registered task and owned trace required.'};$key=$Operation
        }
        'TaskDispatchInvoke'{
            Assert-RemovalOperationTaskSeal $c
            if(-not$c.Used.ContainsKey('TaskDispatch') -or $task.DispatchAttempted -or $task.Error -or $task.Closed -or
                -not$task.RegistrationKnown -or -not$n.TraceOwned -or $n.Failed -or $n.TraceStopped -or
                -not(Test-RemovalTraceCaptureAdmission $c.Readback)){throw 'Original once-only pending RunEx dispatch required.'}
            $key=$Operation
        }
        'ManagedAdmit'{
            Assert-RemovalOperationTaskSeal $c
            if(-not$task.DispatchAttempted -or $c.Managed.Error -or $n.Failed -or $n.TraceStopped -or
                -not(Test-RemovalTraceCaptureAdmission $c.Readback)){throw 'Original dispatched task and admitted trace required.'}
        }
        {$_ -in @('TaskRetire','TaskRetireUnstarted','ManagedRetire')}{
            if($task.Retired -or $task.RetirementAttempted -or -not$task.RegistrationKnown){throw 'Original task retirement already attempted/unknown.'}
            Assert-RemovalOperationRetirement $c $Operation
            $key=$Operation
        }
        'DecodeBegin'{Assert-RemovalOperationDecode $c;if($c.Decode.Attempted){throw 'Decode already attempted.'};$key=$Operation}
        'DecodeTool'{
            Assert-RemovalOperationDecode $c;$d=$c.Decode
            if(-not$d.Attempted -or $null -eq $Request -or $d.Steps.Count -lt 1 -or
                -not[Object]::ReferenceEquals($d.Steps[$d.Steps.Count-1].Request,$Request)){
                throw 'Exact original pending decode tool request required.'
            }
            if($Request.Step -ceq 'tracefmt'){
                $expected=New-RemovalDecodeRequest $d 'tracefmt' $d.Pins.Tracefmt.Path $d.Pins.Tracefmt.Sha256 `
                    @($d.Pins.Etl.Path,'-p',$d.Pins.Symbols.Directory,'-o',$d.TracePath)
            } elseif($Request.Step -ceq 'associate'){
                $expected=New-RemovalDecodeRequest $d 'associate' $d.Pins.Python.Path $d.Pins.Python.Sha256 `
                    @($d.Pins.Reader.Path,'--trace',$d.TracePath,'--summary',$d.SummaryPath,'--report',$d.Pins.ProbeReport.Path,
                        '--output',$d.AnalysisPath,'--max-input-bytes','16777216')
            } else{throw 'Unknown decode step.'}
            if((ConvertTo-Json -InputObject $Request -Depth 6 -Compress) -cne
                (ConvertTo-Json -InputObject $expected -Depth 6 -Compress)){throw 'Decode executable/hash/argv/output/limits differ from canonical request.'}
            $key='DecodeTool:'+$Request.Step
        }
        default{throw 'Unknown native diagnostic operation; no fallback approval.'}
    }
    if($key){
        if($c.Used.ContainsKey($key)){throw 'Original native operation already admitted; no retry.'}
        # Retain authorization intent before dispatch; a lost return is not a
        # second capability. Existing owners retain their own native receipts.
        $c.Used[$key]=$true
    }
}
