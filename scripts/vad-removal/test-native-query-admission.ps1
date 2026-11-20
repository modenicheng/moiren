#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){
    $refused=$false
    try {&$Action|Out-Null} catch {$refused=$true}
    Check $refused 'Expected refusal before native reader dispatch.'
}
function Case([string]$Name,[scriptblock]$Action){
    try {&$Action;$checks.Add(@{name=$Name;passed=$true})}
    catch {$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}
}

# This test process substitutes the complete managed reader type. Its methods
# have no P/Invoke imports, so even the old unguarded real branch cannot use ETW.
Add-Type -TypeDefinition @'
using System;
namespace Moiren.RemovalTraceQueryV1 {
    public static class Reader {
        public static int Queries, MockQueries;
        public static object Query(string name,Guid guid,ulong handle,Guid provider) {
            Queries++; return "managed-native-reader-stub";
        }
        public static object QueryWith(string name,Guid guid,ulong handle,Guid provider,
            Func<ulong,IntPtr,uint> query,Func<IntPtr,uint,IntPtr,uint> enumerate) {
            MockQueries++; query(handle,IntPtr.Zero); enumerate(IntPtr.Zero,0,IntPtr.Zero);
            return "managed-injected-reader-stub";
        }
    }
}
'@
. (Join-Path $PSScriptRoot 'trace-query.ps1')
function Initialize-RemovalTraceQuery {$script:initializations++}
function Fixture {
    $script:initializations=0;$script:approvals=0;$script:approvedResource=$null;$script:approvedOperation=$null
    [Moiren.RemovalTraceQueryV1.Reader]::Queries=0;[Moiren.RemovalTraceQueryV1.Reader]::MockQueries=0
    [pscustomobject]@{Session='MoirenRemoval613admission';SessionGuid=[Guid]'3567dc4b-0698-46a7-9292-8d7e250214d2';Handle=[uint64]701}
}
function Read-Fixture($Owner) {
    Get-RemovalTraceQuerySnapshot -SessionName 'MoirenRemoval613admission' `
        -SessionGuid ([Guid]'3567dc4b-0698-46a7-9292-8d7e250214d2') -Handle 701 -NativeOwner $Owner
}
function Assert-NoReader {
    Check ($script:initializations-eq0-and[Moiren.RemovalTraceQueryV1.Reader]::Queries-eq0) 'Initialized/dispatched reader before refusal'
}

Case 'standalone-query-without-owner-refuses-before-initialization' {
    $null=Fixture
    Reject {Get-RemovalTraceQuerySnapshot -SessionName 'MoirenRemoval613admission' `
        -SessionGuid ([Guid]'3567dc4b-0698-46a7-9292-8d7e250214d2') -Handle 701}
    Assert-NoReader
}
Case 'exact-owner-without-admission-function-refuses-before-initialization' {
    $owner=Fixture
    Check ($null-eq(Get-Command Assert-RemovalNativeOperation -ErrorAction SilentlyContinue)) 'Test unexpectedly loaded native admission'
    Reject {Read-Fixture $owner}
    Assert-NoReader
}

function Assert-RemovalNativeOperation {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)]$Resource,[Parameter(Mandatory=$true)][string]$Operation)
    $script:approvals++;$script:approvedResource=$Resource;$script:approvedOperation=$Operation
    if($script:denyApproval){throw 'Mock native admission refused.'}
    Check ($script:initializations-eq0-and[Moiren.RemovalTraceQueryV1.Reader]::Queries-eq0) 'Approval was checked after reader initialization'
}
$script:denyApproval=$false
Case 'authorized-exact-owner-is-passed-by-reference-before-one-read' {
    $owner=Fixture;$r=Read-Fixture $owner
    Check ($r-ceq'managed-native-reader-stub'-and$script:approvals-eq1-and$script:initializations-eq1-and
        [Moiren.RemovalTraceQueryV1.Reader]::Queries-eq1) 'Expected exactly one authorized reader call'
    Check ([Object]::ReferenceEquals($owner,$script:approvedResource)-and$script:approvedOperation-ceq'TraceReadback') 'Resource/operation admission was not exact'
}
Case 'admission-refusal-does-not-initialize-or-query-reader' {
    $owner=Fixture;$script:denyApproval=$true
    try {Reject {Read-Fixture $owner};Assert-NoReader}
    finally {$script:denyApproval=$false}
}
foreach($field in @('Session','SessionGuid','Handle')){
    $script:wrongField=$field
    Case ('foreign-owner-'+$field+'-refuses-before-admission-or-reader') {
        $owner=Fixture
        switch($script:wrongField){
            'Session' {$owner.Session='MoirenRemoval613foreign'}
            'SessionGuid' {$owner.SessionGuid=[Guid]::NewGuid()}
            'Handle' {$owner.Handle=[uint64]702}
        }
        Reject {Read-Fixture $owner};Assert-NoReader
        Check ($script:approvals-eq0) 'Foreign owner reached approval boundary'
    }
}
Case 'null-or-typeless-owner-never-reaches-admission' {
    $null=Fixture
    Reject {Read-Fixture $null};Reject {Read-Fixture ([pscustomobject]@{})};Assert-NoReader
    Check ($script:approvals-eq0) 'Missing owner reached approval boundary'
}
Case 'complete-injected-boundaries-remain-independent-of-native-admission' {
    $owner=Fixture;$script:denyApproval=$true;$script:injectedQueries=0;$script:injectedEnumerations=0
    try {
        $r=Get-RemovalTraceQuerySnapshot -SessionName $owner.Session -SessionGuid $owner.SessionGuid -Handle $owner.Handle `
            -QueryBoundary @{Query={param($id,$p)$script:injectedQueries++;return [uint32]0};Enumerate={param($p,$n,$count)$script:injectedEnumerations++;return [uint32]0}}
        Check ($r-ceq'managed-injected-reader-stub'-and$script:approvals-eq0-and$script:injectedQueries-eq1-and
            $script:injectedEnumerations-eq1-and[Moiren.RemovalTraceQueryV1.Reader]::Queries-eq0) 'Injected readback acquired native permission or fell through'
    } finally {$script:denyApproval=$false}
}
Case 'partial-injected-boundary-refuses-before-initialization' {
    $owner=Fixture
    Reject {Get-RemovalTraceQuerySnapshot -SessionName $owner.Session -SessionGuid $owner.SessionGuid -Handle $owner.Handle -QueryBoundary @{Query={}}}
    Assert-NoReader
}

. (Join-Path $PSScriptRoot 'trace-session.ps1')
Case 'real-session-readback-forwards-exact-native-owner' {
    $owner=Fixture;$session=[pscustomobject]@{NativeOwner=$owner;QueryBoundary=$null}
    function Get-RemovalTraceQuerySnapshot {
        [CmdletBinding()]
        param($SessionName,$SessionGuid,$Handle,$QueryBoundary,$NativeOwner)
        Check ([Object]::ReferenceEquals($NativeOwner,$script:expectedOwner)) 'Retained native owner was not forwarded'
        Check ($SessionName-ceq$NativeOwner.Session-and$SessionGuid-eq$NativeOwner.SessionGuid-and$Handle-eq$NativeOwner.Handle) 'Forwarded tuple differs'
        return 'owner-forwarded'
    }
    $script:expectedOwner=$owner
    Check ((Get-RemovalOwnedTraceSnapshot $session)-ceq'owner-forwarded') 'Real session query forwarding failed'
}
Case 'mock-session-readback-keeps-legacy-query-delegate-signature' {
    $owner=Fixture;$boundary=@{Query={};Enumerate={}};$script:expectedBoundary=$boundary
    $session=[pscustomobject]@{NativeOwner=$owner;QueryBoundary=$boundary}
    function Get-RemovalTraceQuerySnapshot {
        [CmdletBinding()]
        param($SessionName,$SessionGuid,$Handle,$QueryBoundary)
        Check ([Object]::ReferenceEquals($QueryBoundary,$script:expectedBoundary)) 'Injected query boundary replaced'
        return 'legacy-query-forwarded'
    }
    Check ((Get-RemovalOwnedTraceSnapshot $session)-ceq'legacy-query-forwarded') 'Mock query received unsupported NativeOwner argument'
}

$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ETW_QUERIES=0')
if($failed){exit 1}
