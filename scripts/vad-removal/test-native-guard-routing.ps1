#requires -Version 5.1
# Guard routing only. The dispatcher always refuses before any native API/tool.
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){throw 'Run host-only guard routing tests on a non-target host.'}
. (Join-Path $PSScriptRoot 'trace-decode.ps1')
$checks=New-Object Collections.Generic.List[object]
$routes=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Case([string]$Name,[scriptblock]$Action){
    try{&$Action;$checks.Add(@{name=$Name;passed=$true})}
    catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}
}
function Assert-RemovalTaskHost {}
function Assert-Route([scriptblock]$Action,$Resource,[string]$Operation,$Request=$null){
    $script:routes.Clear();$errorText=$null
    try{&$Action|Out-Null}catch{$errorText=$_.Exception.Message}
    Check ($script:routes.Count-eq1) ('Expected one refusing dispatcher call for '+$Operation+', got '+$script:routes.Count+': '+$errorText)
    $r=$script:routes[0]
    Check ([Object]::ReferenceEquals($r.Resource,$Resource)) ('Original resource not forwarded for '+$Operation)
    Check ($r.Operation-ceq$Operation) ('Exact operation not forwarded: '+$Operation+' != '+$r.Operation)
    Check ([Object]::ReferenceEquals($r.Request,$Request)) ('Original request not forwarded for '+$Operation)
    Check ($errorText-ceq'Host-only dispatcher refusal') ('Dispatcher refusal not propagated for '+$Operation)
}
$resource=[pscustomobject]@{Marker='original-resource'}
$request=[pscustomobject]@{Marker='original-request'}
Check ($null-eq(Get-Command Assert-RemovalNativeOperation -ErrorAction SilentlyContinue)) 'Standalone imports unexpectedly supplied native admission.'
foreach($guard in @('Assert-RemovalTaskNativeApproval','Assert-RemovalTaskCleanupApproval',
    'Assert-RemovalTraceNativeOwnerApproval','Assert-RemovalTraceDecodeToolApproval')){
    Case ('missing-admission-command-remains-closed-'+$guard) {
        $refused=$false
        try{& $guard $resource 'UnsupportedOperation'|Out-Null}catch{$refused=$true}
        Check $refused 'Standalone guard admitted without the admission command.'
        Check ($null-eq(Get-Command Assert-RemovalNativeOperation -ErrorAction SilentlyContinue)) 'Guard imported an authority into the staged worker.'
    }
}
Case 'managed-admission-remains-closed-without-admission-command' {
    $readback=[pscustomobject]@{Marker='original-managed-readback'}
    $managed=[pscustomobject]@{TraceReadback=$readback;TraceSession=[pscustomobject]@{
        Readback=$readback;NativeOwner=[pscustomobject]@{Boundary=$null};StartAttempted=$false}}
    $refused=$false
    try{Assert-RemovalManagedTraceAdmission $managed}catch{$refused=$true}
    Check $refused 'Managed admission opened without the admission command.'
}
function Assert-RemovalNativeOperation {
    param([Parameter(Mandatory=$true)]$Resource,[Parameter(Mandatory=$true)][string]$Operation,$Request=$null)
    # The named binding itself is part of the contract, not positional guessing.
    Check ($MyInvocation.Line-match'-Resource'-and$MyInvocation.Line-match'-Operation') 'Guard did not use named resource/operation arguments.'
    $script:routes.Add([pscustomobject]@{Resource=$Resource;Operation=$Operation;Request=$Request})
    throw 'Host-only dispatcher refusal'
}
foreach($entry in @(@{Guard='Assert-RemovalTaskNativeApproval';Operation='TaskRegister'},
    @{Guard='Assert-RemovalTaskCleanupApproval';Operation='TaskRetire'},
    @{Guard='Assert-RemovalTraceNativeOwnerApproval';Operation='TraceStopPrepare'})){
    Case ('guard-forwards-original-resource-'+$entry.Guard) {
        Assert-Route {& $entry.Guard $resource $entry.Operation} $resource $entry.Operation
    }
}
Case 'decode-guard-forwards-original-request-with-named-binding' {
    Assert-Route {Assert-RemovalTraceDecodeToolApproval $resource 'DecodeTool' $request} $resource 'DecodeTool' $request
}
foreach($entry in @(@{Entry='Register-RemovalTaskDefinition';Operation='TaskRegister'},
    @{Entry='Start-RemovalTaskOnce';Operation='TaskDispatch'},
    @{Entry='Remove-RemovalOwnedTask';Operation='TaskRetire'},
    @{Entry='Remove-RemovalUnstartedTask';Operation='TaskRetireUnstarted'},
    @{Entry='Remove-RemovalDispatchedNeverRunTask';Operation='TaskRetireDispatchedNeverRun'},
    @{Entry='Complete-RemovalManagedTaskCleanup';Operation='ManagedRetire'})){
    Case ('production-entry-routes-before-resource-work-'+$entry.Operation) {
        Assert-Route {& $entry.Entry $resource} $resource $entry.Operation
    }
}
Case 'managed-admission-routes-original-session-before-live-refresh' {
    $readback=[pscustomobject]@{Marker='original-managed-readback'}
    $managed=[pscustomobject]@{TraceReadback=$readback;TraceSession=[pscustomobject]@{
        Readback=$readback;NativeOwner=[pscustomobject]@{Boundary=$null};StartAttempted=$false}}
    Assert-Route {Assert-RemovalManagedTraceAdmission $managed} $managed 'ManagedAdmit'
}
Case 'injected-managed-trace-keeps-refresh-without-native-admission' {
    $script:routes.Clear();$script:managedRefreshCount=0
    $readback=[pscustomobject]@{Marker='injected-managed-readback'}
    $managed=[pscustomobject]@{TraceReadback=$readback;TraceSession=[pscustomobject]@{
        Readback=$readback;NativeOwner=[pscustomobject]@{Boundary=@{}}}}
    function Refresh-RemovalTraceSession($Session){
        Check ([Object]::ReferenceEquals($Session,$managed.TraceSession)) 'Injected refresh lost original session.'
        $script:managedRefreshCount++
    }
    function Test-RemovalTraceCaptureAdmission($Readback){
        Check ([Object]::ReferenceEquals($Readback,$managed.TraceReadback)) 'Injected capture lost original readback.'
        return $true
    }
    Assert-RemovalManagedTraceAdmission $managed
    Check ($script:routes.Count-eq0-and$script:managedRefreshCount-eq1) 'Injected managed trace changed admission/refresh behavior.'
}
Initialize-RemovalTraceNative
foreach($entry in @(@{Entry='Start-RemovalNativeTrace';Operation='TraceStart'},
    @{Entry='Stop-RemovalNativeTrace';Operation='TraceStopPrepare'})){
    Case ('trace-entry-routes-before-bridge-flags-'+$entry.Operation) {
        $owner=New-RemovalNativeTrace -Session 'MoirenRemoval613guardroute' -EtlPath 'D:\owned\guard-routing.etl'
        Assert-Route {& $entry.Entry $owner} $owner $entry.Operation
        Check (-not$owner.StartAttempted-and-not$owner.StopQueryAttempted-and-not$owner.StopAttempted) 'Refused pre-entry mutated bridge lifecycle flags.'
    }
}
foreach($entry in @(@{Native='Start';Operation='TraceStartDispatch'},@{Native='Enumerate';Operation='TraceEnumerate'},
    @{Native='Enable';Operation='TraceEnable'},@{Native='Query';Operation='TraceStopQuery'},@{Native='Stop';Operation='TraceStop'})){
    Case ('native-dispatch-routes-exact-operation-'+$entry.Native) {
        $owner=[pscustomobject]@{Boundary=$null;Marker='original-native-owner'}
        Assert-Route {Invoke-RemovalNativeBoundary $owner $entry.Native @()} $owner $entry.Operation
    }
}
Case 'decode-begin-routes-original-before-attempt-flag-or-directory' {
    $decode=[pscustomobject]@{Attempted=$false;ToolBoundary=$null}
    Assert-Route {Invoke-RemovalTraceDecode $decode} $decode 'DecodeBegin'
    Check (-not$decode.Attempted) 'Refused DecodeBegin changed attempt state.'
}
Case 'decode-tool-routes-original-owner-and-exact-retained-request' {
    $decode=[pscustomobject]@{ToolBoundary=$null;Steps=(New-Object Collections.Generic.List[object])}
    Assert-Route {Invoke-RemovalDecodeTool $decode $request} $decode 'DecodeTool' $request
    Check ($decode.Steps.Count-eq1-and[Object]::ReferenceEquals($decode.Steps[0].Request,$request)-and
        $null-eq$decode.Steps[0].Owner) 'Refused tool lost original step or created an owner.'
    Check ($null-ne$decode.Steps[0].PSObject.Properties['DispatchAttempted']-and-not$decode.Steps[0].DispatchAttempted) 'Predispatch tool refusal invented an unknown process.'
}
Case 'injected-native-delegates-still-bypass-native-admission-without-fallback' {
    $script:routes.Clear();$script:delegateCalls=New-Object Collections.Generic.List[string]
    $boundary=@{}
    foreach($name in @('Start','Enumerate','Enable','Query','Stop')){
        $boundary[$name]={$script:delegateCalls.Add('injected');return [uint32]0}
    }
    $owner=[pscustomobject]@{Boundary=$boundary}
    foreach($name in @('Start','Enumerate','Enable','Query','Stop')){
        $status=Invoke-RemovalNativeBoundary $owner $name @()
        Check ($status-is[uint32]-and$status-eq0) 'Injected delegate status changed.'
    }
    Check ($script:delegateCalls.Count-eq5-and$script:routes.Count-eq0) 'Injected boundary entered native routing or fell back.'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ADMISSION=always-refusing NATIVE_ETW=0 SCHEDULER=0 TOOL_LAUNCH=0')
if($failed){exit 1}
