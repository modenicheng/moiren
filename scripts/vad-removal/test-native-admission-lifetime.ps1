#requires -Version 5.1
# Real root/context/lease/C# lifecycle, synthetic readiness and API transport.
# This suite never executes an OS tracing, scheduler, device or audio operation.
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){throw 'Run admission lifetime composition on a non-target host.'}
. (Join-Path $PSScriptRoot 'diagnostic-controller.ps1')
$script:RealLifetimeLease=${function:New-RemovalDiagnosticLease}
$script:RealLifetimeTarget=${function:New-RemovalTargetReadiness}
$script:RealLifetimeTargetParameters=(Get-Command New-RemovalTargetReadiness).Parameters
$script:RealLifetimeAudit=${function:New-RemovalSafetyAudit}

# Reuse definitions only, not the other suite's cases or top-level overrides.
# Keep every collector owner and receipt truthfully InjectedCollectorSource.
$template=Join-Path $PSScriptRoot 'test-diagnostic-lifetime.ps1'
$tokens=$null;$parseErrors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($template,[ref]$tokens,[ref]$parseErrors)
if($parseErrors.Count){throw 'Lifetime fixture source did not parse.'}
$helpers=@('Check','Case','Event','Assert-RemovalTaskHost','Get-RemovalSessionExplorerPath',
    'Get-RemovalSessionProcessIds','Open-RemovalSessionProcess','Read-RemovalWorkerToken',
    'Assert-RemovalRunInputs','Assert-RemovalDiagnosticAdmission','Assert-RemovalControllerCollectorSource','New-RemovalDiagnosticLease',
    'New-RemovalTargetReadiness','New-RemovalSafetyAudit','Write-LifetimeTraceBuffer',
    'New-LifetimeSnapshot','Write-LifetimeCollector','New-LifetimeFixture','Get-LifetimePin')
$definitions=@($ast.EndBlock.Statements|Where-Object {$_-is[Management.Automation.Language.FunctionDefinitionAst]})
foreach($name in $helpers){
    $found=@($definitions|Where-Object {$_.Name-ceq$name})
    if($found.Count-ne1){throw ('Unique shared fixture definition required: '+$name)}
    $source=$found[0].Extent.Text.Replace('$PSScriptRoot',("'"+$PSScriptRoot.Replace("'","''")+"'"))
    . ([scriptblock]::Create($source))
}
$checks=New-Object Collections.Generic.List[object]
$root=Join-Path $PSScriptRoot ('logs\native-admission-lifetime-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$script:ProductionContextInitializer=${function:Initialize-RemovalNativeContext}
$script:ProductionRelease=${function:Assert-RemovalDiagnosticRelease}
Initialize-RemovalTraceNative

Case 'production-executable-release-is-still-closed' {
    $refused=$false;try{& $script:ProductionRelease}catch{$refused=$true}
    Check $refused 'Executable release unexpectedly opened.'
}
function Assert-RemovalDiagnosticRelease {Event 'TEST-ONLY-release'}

function Invoke-RemovalNativeBoundary($Owner,[string]$Operation,[object[]]$Values){
    # TEST ONLY transport replacement. The owner has no injected Boundary, so
    # real pre-entry guards/context binding and bridge flags remain in use.
    Check ($null-eq$Owner.Boundary-and[Object]::ReferenceEquals($Owner,$script:fixture.Controller.OriginalNativeOwner)) 'API transport lost real-path original owner.'
    $mapped=switch -CaseSensitive ($Operation){
        'Start'{'TraceStartDispatch'};'Enumerate'{'TraceEnumerate'};'Enable'{'TraceEnable'}
        'Query'{'TraceStopQuery'};'Stop'{'TraceStop'};default{throw 'Unknown synthetic transport operation.'}
    }
    Assert-RemovalTraceNativeOwnerApproval $Owner $mapped
    if($Operation-ceq'Start'){
        $bridge=$null
        Check ($script:RemovalNativeBridges.TryGetValue($Owner,[ref]$bridge)) 'Original synthetic Start bridge missing.'
        $bridge.BeginStartDispatch()
    }
    Event ('guarded-'+$mapped)
    switch -CaseSensitive ($Operation){
        'Start'{
            Check $Owner.StartDispatchAttempted 'Guarded synthetic Start did not retain actual dispatch intent.'
            $c=Get-RemovalOperationContext $Owner
            Check ([Object]::ReferenceEquals($c.Writer,$script:fixture.Controller.RetainedLease.RetainedFile)-and
                $c.Writer-is[IO.FileStream]-and-not$c.WriterHandle.IsClosed) 'Context did not bind original live writer.'
            Check (Test-Path -LiteralPath (Join-Path $c.Managed.RunDirectory 'run-spec.json')) 'Context did not bind actual managed staging.'
            $script:fixture.Guid=$Owner.SessionGuid;$script:fixture.TraceName=$Owner.Session;$script:fixture.EtlPath=$Owner.EtlPath
            [Runtime.InteropServices.Marshal]::WriteInt64($Values[0],701);return [uint32]0
        }
        'Enumerate'{
            $row=New-Object Moiren.RemovalTraceQueryV1.GuidProperties
            $row.Guid=[Guid]'7dc75ba9-8274-4eae-a42d-b4c74e0209d2';$row.IsEnable=[byte]0
            [Runtime.InteropServices.Marshal]::StructureToPtr($row,[Runtime.InteropServices.Marshal]::ReadIntPtr($Values[0]),$false)
            [Runtime.InteropServices.Marshal]::WriteInt32($Values[2],1);return [uint32]0
        }
        'Enable'{
            if($script:fixture.CloseBirth){
                $r=$script:fixture.Controller
                $r.DeadlineMilliseconds=[long]0;$r.TimedOut=$true;$r.Errors.Add('Synthetic root refusal at failed enable')
                $r.RetainedBefore.Ready=$false
                [IO.File]::AppendAllText($script:fixture.Inputs.Schedule.Path,' ')
            }
            $script:fixture.FreshCallsAtEnable=@($script:fixture.Events|Where-Object {$_-in@('admission','inputs')}).Count
            return [uint32]5
        }
        {$_-in@('Query','Stop')}{
            Check ($Values[0]-eq[uint64]701) 'Cleanup changed original owned handle.'
            Check (@($script:fixture.Events|Where-Object {$_-in@('admission','inputs')}).Count-eq$script:fixture.FreshCallsAtEnable) 'Cleanup incorrectly renewed birth/readiness/input admission.'
            Write-LifetimeTraceBuffer $Values[1];return [uint32]0
        }
    }
}
function Run-AdmissionLifetime($f){
    $o=$f.Controller
    # These diagnostics never decode. Still provide a complete explicit static
    # pin shape for context capture; no null member may trigger a native lookup.
    foreach($name in @('DriverPackage','Tracefmt','Python','Reader','Symbols')){
        $f.Inputs.Pins[$name]=$null
    }
    $f.InputsJson=ConvertTo-Json -InputObject $f.Inputs -Depth 10 -Compress
    Start-RemovalDiagnosticController $o
    for($i=0;$i-lt150;$i++){
        $r=Observe-RemovalDiagnosticController $o -ObservationMilliseconds 50
        if(-not$r.NeedsObservation){return $r}
    }
    throw ('Original admission lifetime did not complete: '+($o.Errors-join'; '))
}
function Close-AdmissionLifetime($f){
    $o=$f.Controller
    # Only synthetic transport was used. Drain owned collector runspaces; the
    # finally is failure cleanup, never success evidence or an OS STOP fallback.
    foreach($p in @($o.RetainedBefore.RetainedPreflight,$(if($null-ne$o.RetainedAfter){$o.RetainedAfter.RetainedAfterPreflight}))){
        if($null-eq$p-or$null-eq$p.RetainedOwner){continue}
        for($i=0;$i-lt150-and-not$p.RetainedOwner.Closed;$i++){$null=Observe-RemovalPreflightSession $p -ObservationMilliseconds 100}
        Check $p.RetainedOwner.Closed 'Original synthetic runspace unresolved.'
    }
    if(-not$o.RetainedLease.Closed){Release-RemovalDiagnosticLease $o.RetainedLease}
}
Case 'unmodified-production-initializer-rejects-truthfully-injected-readiness' {
    $f=New-LifetimeFixture
    $f|Add-Member NoteProperty CloseBirth $false
    try{
        $r=Run-AdmissionLifetime $f;$o=$f.Controller
        Check ($r.Completed-and-not$r.Safe-and-not$o.OriginalNativeOwner.StartAttempted) 'Production accepted synthetic provenance or lost refusal completion.'
        Check ($o.RetainedBefore.RetainedPreflight.ExecutionMode-ceq'InjectedCollectorSource'-and
            $o.RetainedAfter.RetainedAfterPreflight.ExecutionMode-ceq'InjectedCollectorSource') 'Synthetic readiness was relabelled.'
        Check (@($f.Events|Where-Object {$_-like'guarded-*'}).Count-eq0) 'Provenance refusal reached transport.'
    }finally{Close-AdmissionLifetime $f}
}

# Explicit TEST ONLY initializer adapter: change exactly the source-provenance
# predicate, not any root/resource/lifetime validation and never the owner DTO.
$body=$script:ProductionContextInitializer.ToString()
$predicate='$Owner.RetainedBefore.RetainedPreflight.ExecutionMode -cne ''RepositoryCollector'''
$predicateNodes=@($script:ProductionContextInitializer.Ast.FindAll({param($node)
    $node-is[Management.Automation.Language.BinaryExpressionAst]-and$node.Extent.Text-ceq$predicate
},$true))
if($predicateNodes.Count-ne1-or$body.IndexOf($predicate)-lt0-or$body.IndexOf($predicate)-ne$body.LastIndexOf($predicate)){
    throw 'Exactly one AST-matched production provenance predicate required.'
}
Set-Item Function:Initialize-RemovalNativeContext ([scriptblock]::Create($body.Replace($predicate,
    '$Owner.RetainedBefore.RetainedPreflight.ExecutionMode -cne ''InjectedCollectorSource''')))

foreach($stale in @($false,$true)){
    Case ('real-operation-context-failed-enable-cleanup-and-root-receipts-stale-birth-'+$stale) {
        $f=New-LifetimeFixture
        $f|Add-Member NoteProperty CloseBirth $stale
        $f|Add-Member NoteProperty FreshCallsAtEnable 0
        try{
            $r=Run-AdmissionLifetime $f;$o=$f.Controller;$n=$o.OriginalNativeOwner
            Check ($r.Completed-and-not$r.Safe-and-not$r.NativePassed-and-not$r.Qualified) ('Root completion missing/refusal promoted: '+($o.Errors-join'; '))
            $c=Get-RemovalOperationContext $n
            Check ([Object]::ReferenceEquals($c.Root,$o)-and[Object]::ReferenceEquals($c.Managed,$o.RetainedManaged)-and
                [Object]::ReferenceEquals($c,(Get-RemovalOperationContext $o.RetainedManaged.TaskOwner))-and
                [Object]::ReferenceEquals($c,(Get-RemovalOperationContext $o.RetainedManaged))) 'Original production context/resource association lost.'
            foreach($op in @('TraceStart','TraceStartDispatch','TraceEnumerate','TraceEnable','TraceStopPrepare','TraceStopQuery','TraceStop')){
                Check ($c.Used.ContainsKey($op)-and$c.Used[$op]) ('Real context missed admission: '+$op)
            }
            Check ($c.Used.Count-eq7-and$n.StartDispatchAttempted-and$n.StartStatus-eq[uint32]0-and$n.EnableStatus-eq[uint32]5-and$n.StopStatus-eq[uint32]0-and
                $n.TraceStopped-and[Object]::ReferenceEquals($n.StopReceipt,$o.RetainedTrace.StoppedReceipt)) 'Failed-enable once-only C# lifecycle/STOP receipt differs.'
            foreach($op in @('TraceStartDispatch','TraceEnumerate','TraceEnable','TraceStopQuery','TraceStop')){
                Check (@($f.Events|Where-Object {$_-ceq('guarded-'+$op)}).Count-eq1) ('Transport missing/repeated: '+$op)
            }
            Check ($null-eq$n.Boundary-and$null-eq$o.RetainedTrace.QueryBoundary-and$null-eq$o.OriginalReadback.Initial-and
                -not$o.OriginalReadback.FinalCountersKnown-and-not$n.Qualified-and-not$n.NativeApproved) 'Native path was replaced/relabelled or failed capture qualified.'
            Check ($o.RetainedAfter.Complete-and$o.RetainedAfter.Safe-and$o.RetainedAfter.RetainedAfterPreflight.RetainedOwner.Closed-and
                $o.RetainedBefore.RetainedPreflight.RetainedOwner.Closed) 'Actual synthetic before/after did not settle safely.'
            Assert-RemovalSafetyReceipt $o.RetainedAfter
            foreach($pin in @($o.RetainedBefore.ReceiptPin,$o.RetainedAfter.ReceiptPin,$o.ResultPin)){$null=Copy-RemovalDecodePin $pin 2097152}
            $saved=[IO.File]::ReadAllText($o.ResultPin.Path)|ConvertFrom-Json
            Check ($saved.ControllerComplete-and-not$saved.Safe-and-not$saved.NativePassed-and-not$saved.Qualified-and
                $saved.After.Sha256-ceq$o.RetainedAfter.ReceiptPin.Sha256) 'Root failed result/after provenance not persisted.'
            Check (-not$o.RegistrationRequested-and-not$o.DispatchRequested-and$null-eq$o.RetainedManaged.TaskOwner.Scheduler-and
                $o.RetainedLease.Closed-and$c.WriterHandle.IsClosed) 'Failed enable reached scheduler or failed original lease release.'
            if($stale){Check ($o.TimedOut-and-not$o.RetainedBefore.Ready-and
                (Get-FileHash -LiteralPath $f.Inputs.Schedule.Path).Hash-cne$f.Inputs.Schedule.Sha256) 'Stale birth fixture was not exercised.'}
            $count=$f.Events.Count;$null=Observe-RemovalDiagnosticController $o
            Check ($f.Events.Count-eq$count) 'Completed root repeated operations.'
        }finally{Close-AdmissionLifetime $f}
    }
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object {-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ROOT_CONTEXT_LEASE=production READINESS=synthetic-runspace PROVENANCE=truthfully-injected RELEASE=TEST-ONLY-override API_TRANSPORT=TEST-ONLY-guarded NATIVE_OS_CALLS=0')
Write-Output ('FIXTURE_ROOT='+$root)
if($failed){exit 1}
