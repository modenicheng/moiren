#requires -Version 5.1
# Definitions only. Native dispatch is deliberately refused pending combined
# runner/preflight review. Injected delegates exercise the same buffers in tests.
# StartTrace/ControlTrace contracts:
# https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-starttracew
# https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-controltracew
# Classic provider enablement is exclusive and can steal another session:
# https://learn.microsoft.com/en-us/windows/win32/etw/configuring-and-starting-an-event-tracing-session
Set-StrictMode -Version 2

function Initialize-RemovalTraceNative {
    if('Moiren.RemovalTraceNativeV1.Owner' -as [type]){return}
    . (Join-Path $PSScriptRoot 'trace-query.ps1')
    Initialize-RemovalTraceQuery
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

namespace Moiren.RemovalTraceNativeV1 {
    // Copy scalar output while the STOP buffer is still owned by the bridge.
    // QueryV1.Session has public fields and must never be the published proof.
    public sealed class FinalSnapshot {
        public string SessionName { get; private set; }
        public string LogFileName { get; private set; }
        public Guid SessionGuid { get; private set; }
        public ulong Handle { get; private set; }
        public uint ClientContext { get; private set; }
        public uint LogFileMode { get; private set; }
        public uint MaximumFileSize { get; private set; }
        public uint EventsLost { get; private set; }
        public uint LogBuffersLost { get; private set; }
        public uint RealTimeBuffersLost { get; private set; }
        private static T Field<T>(object source,string name) {
            var field=source.GetType().GetField(name);
            if(field==null || field.FieldType!=typeof(T))
                throw new InvalidOperationException("Typed native session field missing: "+name);
            return (T)field.GetValue(source);
        }
        internal FinalSnapshot(object source) {
            if(source==null || source.GetType().FullName!="Moiren.RemovalTraceQueryV1.Session")
                throw new InvalidOperationException("Decoded native session required; no detached replacement.");
            SessionName=Field<string>(source,"SessionName"); LogFileName=Field<string>(source,"LogFileName");
            SessionGuid=Field<Guid>(source,"SessionGuid"); Handle=Field<ulong>(source,"Handle");
            ClientContext=Field<uint>(source,"ClientContext"); LogFileMode=Field<uint>(source,"LogFileMode");
            MaximumFileSize=Field<uint>(source,"MaximumFileSize"); EventsLost=Field<uint>(source,"EventsLost");
            LogBuffersLost=Field<uint>(source,"LogBuffersLost"); RealTimeBuffersLost=Field<uint>(source,"RealTimeBuffersLost");
        }
    }
    public sealed class StopReceipt {
        public string Operation { get { return "Stop"; } }
        public Owner Owner { get; private set; }
        public ulong OwnedHandle { get; private set; }
        public Guid SessionGuid { get; private set; }
        public string SessionName { get; private set; }
        public string EtlPath { get; private set; }
        public uint? Status { get; internal set; }
        public bool StoppedConfirmed { get; internal set; }
        public bool FinalCountersKnown { get; internal set; }
        public bool Pending { get; internal set; }
        public FinalSnapshot Snapshot { get; internal set; }
        public string Error { get; internal set; }
        internal StopReceipt(Owner owner) {
            Owner=owner; OwnedHandle=owner.Handle; SessionGuid=owner.SessionGuid;
            SessionName=owner.Session; EtlPath=owner.EtlPath;
        }
    }
    // No finalizer/IDisposable: loss of a managed reference must never issue
    // STOP, close a consumer handle, or adopt/alter a session by its name.
    public sealed class Owner {
        private bool startCaptured;
        private FinalSnapshot beforeStop;
        public string Session { get; private set; }
        public string EtlPath { get; private set; }
        public Guid SessionGuid { get; private set; }
        public ulong Handle { get; private set; }
        public bool StartAttempted { get; private set; }
        public uint? StartStatus { get; private set; }
        public bool TraceOwned { get; private set; }
        public bool EnableAttempted { get; private set; }
        public uint? EnableStatus { get; private set; }
        public bool ProviderIdleAttempted { get; private set; }
        public uint? ProviderIdleStatus { get; private set; }
        public bool ProviderIdleConfirmed { get; private set; }
        public bool StopQueryAttempted { get; private set; }
        public uint? StopQueryStatus { get; private set; }
        public bool StopAttempted { get; private set; }
        public uint? StopStatus { get; private set; }
        public bool TraceStopped { get; private set; }
        public StopReceipt StopReceipt { get; private set; }
        public string OwnershipBasis { get { return TraceOwned ? "StartTraceHandleGuid" : null; } }
        // Readback policy owns these diagnostic fields, not native ownership.
        public bool SessionReadbackConfirmed { get; set; }
        public object RawTraceLoss { get; set; }
        public bool Failed { get; set; }
        public List<string> Errors { get; private set; }
        public object Boundary { get; private set; }
        public bool Qualified { get { return false; } }
        public bool NativeApproved { get { return false; } }
        internal Owner(string session,string path,object boundary) {
            Session=session; EtlPath=path; Boundary=boundary; SessionGuid=Guid.NewGuid();
            RawTraceLoss="unknown"; Errors=new List<string>();
        }
        public void Fail(string message) {
            Failed=true; SessionReadbackConfirmed=false;
            if(Errors.Count<16 && !Errors.Contains(message)) Errors.Add(message);
            if(StopReceipt!=null) StopReceipt.Error=message;
        }
        internal void BeginStart() {
            if(StartAttempted || Failed) throw new InvalidOperationException("Start already attempted/refused; no retry.");
            StartAttempted=true;
        }
        internal void CaptureStart(uint? status,ulong handle) {
            if(!StartAttempted || startCaptured)
                throw new InvalidOperationException("Start output already captured or no dispatch attempt.");
            startCaptured=true;
            Handle=handle; StartStatus=status;
            TraceOwned=status.HasValue && status.Value==0 && handle!=0 && handle!=ulong.MaxValue;
        }
        internal void BeginIdle() {
            if(!TraceOwned || ProviderIdleAttempted || EnableAttempted || Failed)
                throw new InvalidOperationException("No fresh owned provider-idle attempt.");
            ProviderIdleAttempted=true;
        }
        internal void CaptureIdle(uint status,bool confirmed) {
            if(!ProviderIdleAttempted || EnableAttempted || ProviderIdleConfirmed ||
                (ProviderIdleStatus.HasValue && (ProviderIdleStatus.Value!=status || !confirmed)) ||
                (confirmed && status!=0))
                throw new InvalidOperationException("Idle evidence already captured or invalid stage/status.");
            ProviderIdleStatus=status; ProviderIdleConfirmed=confirmed;
        }
        internal void BeginEnable() {
            if(!TraceOwned || !ProviderIdleConfirmed || EnableAttempted || Failed)
                throw new InvalidOperationException("No fresh idle-provider/create provenance for enable.");
            EnableAttempted=true;
        }
        internal void CaptureEnable(uint status) {
            if(!EnableAttempted || EnableStatus.HasValue || StopQueryAttempted)
                throw new InvalidOperationException("Enable output already captured or no enable attempt.");
            EnableStatus=status;
        }
        internal void BeginStopQuery() {
            if(!TraceOwned || TraceStopped || StopQueryAttempted || StopAttempted)
                throw new InvalidOperationException("No unused exact owned STOP attempt; no retry/name adoption.");
            StopQueryAttempted=true; StopReceipt=new StopReceipt(this);
        }
        internal void CaptureStopQuery(uint status) {
            if(!StopQueryAttempted || StopQueryStatus.HasValue || StopAttempted)
                throw new InvalidOperationException("Pre-STOP query output already captured or no attempt.");
            StopQueryStatus=status;
        }
        private void AssertSession(FinalSnapshot snapshot) {
            if(snapshot.Handle!=Handle || snapshot.SessionGuid!=SessionGuid ||
                !String.Equals(snapshot.SessionName,Session,StringComparison.Ordinal) ||
                !String.Equals(snapshot.LogFileName,EtlPath,StringComparison.Ordinal) ||
                snapshot.ClientContext!=1 || snapshot.LogFileMode!=1 || snapshot.MaximumFileSize!=16)
                throw new InvalidOperationException("Native session proof differs from retained owner/config.");
        }
        internal void CaptureStopQuerySnapshot(object snapshot) {
            if(!StopQueryAttempted || StopQueryStatus!=0 || StopAttempted || beforeStop!=null)
                throw new InvalidOperationException("No unused successful pre-STOP query capture.");
            var copy=new FinalSnapshot(snapshot); AssertSession(copy); beforeStop=copy;
        }
        internal void BeginStop() {
            if(!StopQueryAttempted || StopQueryStatus!=0 || beforeStop==null || StopAttempted)
                throw new InvalidOperationException("No successful exact pre-STOP query.");
            StopAttempted=true;
        }
        internal void CaptureStop(uint status) {
            if(!StopAttempted || StopStatus.HasValue || StopReceipt==null)
                throw new InvalidOperationException("STOP output already captured or no exact STOP attempt.");
            StopStatus=status; StopReceipt.Status=status;
            // winerror.h: MORE_DATA=234, ACTIVE_CONNECTIONS=2402 (not 1800).
            TraceStopped=status==0 || status==234;
            StopReceipt.StoppedConfirmed=TraceStopped; StopReceipt.Pending=status==2402;
            SessionReadbackConfirmed=false;
        }
        internal void CaptureFinal(object snapshot) {
            if(!TraceOwned || !StopAttempted || StopStatus!=0 || !TraceStopped || beforeStop==null ||
                StopReceipt==null || StopReceipt.Status!=0 || !StopReceipt.StoppedConfirmed ||
                StopReceipt.Snapshot!=null || StopReceipt.FinalCountersKnown)
                throw new InvalidOperationException("No unused successful STOP payload capture.");
            var copy=new FinalSnapshot(snapshot); AssertSession(copy);
            if(copy.EventsLost<beforeStop.EventsLost || copy.LogBuffersLost<beforeStop.LogBuffersLost ||
                copy.RealTimeBuffersLost<beforeStop.RealTimeBuffersLost)
                throw new InvalidOperationException("STOP counters decreased from retained pre-STOP proof.");
            StopReceipt.Snapshot=copy; StopReceipt.FinalCountersKnown=true;
        }
    }
    // PowerShell needs public entry points. Give its dispatcher a separate
    // owner-bound capability, never a Capture method on the published Owner.
    // A bridge can only create its own Owner, not attach to an existing one.
    public sealed class Bridge {
        public Owner Owner { get; private set; }
        public Bridge(string session,string path,object boundary) { Owner=new Owner(session,path,boundary); }
        public void BeginStart() { Owner.BeginStart(); }
        public void CaptureStart(uint? status,ulong handle) { Owner.CaptureStart(status,handle); }
        public void BeginIdle() { Owner.BeginIdle(); }
        public void CaptureIdle(uint status,bool confirmed) { Owner.CaptureIdle(status,confirmed); }
        public void BeginEnable() { Owner.BeginEnable(); }
        public void CaptureEnable(uint status) { Owner.CaptureEnable(status); }
        public void BeginStopQuery() { Owner.BeginStopQuery(); }
        public void CaptureStopQuery(uint status) { Owner.CaptureStopQuery(status); }
        public void CaptureStopQuerySnapshot(object snapshot) { Owner.CaptureStopQuerySnapshot(snapshot); }
        public void BeginStop() { Owner.BeginStop(); }
        public void CaptureStop(uint status) { Owner.CaptureStop(status); }
        public void CaptureFinal(object snapshot) { Owner.CaptureFinal(snapshot); }
    }
    public static class Api {
        [DllImport("advapi32.dll", EntryPoint="StartTraceW", CharSet=CharSet.Unicode, ExactSpelling=true)]
        private static extern uint StartTrace(IntPtr handle,string name,IntPtr properties);
        [DllImport("advapi32.dll", ExactSpelling=true)]
        private static extern uint EnableTraceEx2(ulong handle,ref Guid provider,uint code,byte level,
            ulong any,ulong all,uint timeout,IntPtr parameters);
        [DllImport("advapi32.dll", EntryPoint="ControlTraceW", CharSet=CharSet.Unicode, ExactSpelling=true)]
        private static extern uint ControlTrace(ulong handle,string name,IntPtr properties,uint code);
        [DllImport("advapi32.dll", ExactSpelling=true)]
        private static extern uint EnumerateTraceGuids(IntPtr pointers,uint count,out uint returned);
        public static uint Start(IntPtr handle,string name,IntPtr properties) { return StartTrace(handle,name,properties); }
        public static uint Enable(ulong handle,Guid provider,uint code,byte level,ulong any,ulong all,uint timeout) {
            return EnableTraceEx2(handle,ref provider,code,level,any,all,timeout,IntPtr.Zero);
        }
        public static uint Query(ulong handle,IntPtr properties) { return ControlTrace(handle,null,properties,0); }
        public static uint Stop(ulong handle,IntPtr properties) { return ControlTrace(handle,null,properties,1); }
        public static uint Enumerate(IntPtr pointers,uint count,IntPtr returnedPointer) {
            uint returned; uint status=EnumerateTraceGuids(pointers,count,out returned);
            Marshal.WriteInt32(returnedPointer,unchecked((int)returned)); return status;
        }
    }
}
'@
}

function Assert-RemovalTraceNativeOwnerApproval {
    throw 'Native trace owner awaits reviewed combined preflight/runner approval; no host ETW dispatch.'
}

function New-RemovalNativeTrace {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory=$true)][ValidatePattern('\AMoirenRemoval613[A-Za-z0-9_-]{1,64}\z')][string]$Session,
        [Parameter(Mandatory=$true)][string]$EtlPath,
        [hashtable]$NativeBoundary
    )
    $ErrorActionPreference='Stop'
    if($EtlPath -notmatch '^[A-Za-z]:\\' -or $EtlPath -match '[\x00-\x1f"*?]' -or
        $EtlPath.Substring(2).Contains(':') -or [IO.Path]::GetFullPath($EtlPath) -cne $EtlPath -or
        [IO.Path]::GetExtension($EtlPath) -ine '.etl' -or $EtlPath.Length -ge 1024){throw 'Canonical local ETL path required.'}
    if($null -ne $NativeBoundary){
        $copy=@{}
        foreach($name in @('Start','Enumerate','Enable','Query','Stop')){
            if($NativeBoundary[$name] -isnot [scriptblock]){throw 'All five injected native boundaries required; no native fallback.'}
            $copy[$name]=$NativeBoundary[$name]
        }
        $NativeBoundary=$copy
    }
    Initialize-RemovalTraceNative
    if($null -eq (Get-Variable -Name RemovalNativeBridges -Scope Script -ErrorAction SilentlyContinue)){
        $script:RemovalNativeBridges=New-Object 'Runtime.CompilerServices.ConditionalWeakTable[object,object]'
    }
    $bridge=[Moiren.RemovalTraceNativeV1.Bridge]::new($Session,$EtlPath,$NativeBoundary)
    $script:RemovalNativeBridges.Add($bridge.Owner,$bridge)
    return $bridge.Owner
}

function New-RemovalNativeProperties($Owner,[switch]$Start) {
    $bytes=[Moiren.RemovalTraceQueryV1.Reader]::BufferBytes
    $p=[Runtime.InteropServices.Marshal]::AllocHGlobal($bytes)
    try {
        [Runtime.InteropServices.Marshal]::Copy((New-Object byte[] $bytes),0,$p,$bytes)
        $v=New-Object Moiren.RemovalTraceQueryV1.Properties
        $w=New-Object Moiren.RemovalTraceQueryV1.Wnode
        $w.BufferSize=$bytes;$w.Flags=0x20000;$w.Guid=$Owner.SessionGuid
        $v.LoggerNameOffset=[Moiren.RemovalTraceQueryV1.Reader]::PropertiesSize
        $v.LogFileNameOffset=$v.LoggerNameOffset+2048
        if($Start){
            $w.ClientContext=1;$v.LogFileMode=1;$v.MaximumFileSize=16
            $v.BufferSize=64;$v.MinimumBuffers=2;$v.MaximumBuffers=16;$v.FlushTimer=1
            foreach($entry in @(@{Offset=$v.LoggerNameOffset;Text=$Owner.Session},@{Offset=$v.LogFileNameOffset;Text=$Owner.EtlPath})){
                $text=[Text.Encoding]::Unicode.GetBytes($entry.Text+[char]0)
                [Runtime.InteropServices.Marshal]::Copy($text,0,[IntPtr]::Add($p,[int]$entry.Offset),$text.Length)
            }
        }
        $v.Wnode=$w;[Runtime.InteropServices.Marshal]::StructureToPtr($v,$p,$false)
        return $p
    } catch {[Runtime.InteropServices.Marshal]::FreeHGlobal($p);throw}
}

function Invoke-RemovalNativeBoundary($Owner,[string]$Operation,[object[]]$Values) {
    if($null -ne $Owner.Boundary){
        $result=&$Owner.Boundary[$Operation] @Values
        if($result -isnot [UInt32]){throw 'Native boundary must return exactly one typed UInt32 status; unknown is not zero.'}
        return $result
    }
    Assert-RemovalTraceNativeOwnerApproval
    switch($Operation){
        'Start'{return [Moiren.RemovalTraceNativeV1.Api]::Start($Values[0],$Values[1],$Values[2])}
        'Enumerate'{return [Moiren.RemovalTraceNativeV1.Api]::Enumerate($Values[0],$Values[1],$Values[2])}
        'Enable'{return [Moiren.RemovalTraceNativeV1.Api]::Enable($Values[0],$Values[1],$Values[2],$Values[3],$Values[4],$Values[5],$Values[6])}
        'Query'{return [Moiren.RemovalTraceNativeV1.Api]::Query($Values[0],$Values[1])}
        'Stop'{return [Moiren.RemovalTraceNativeV1.Api]::Stop($Values[0],$Values[1])}
        default{throw 'Unknown native trace operation.'}
    }
}

function Confirm-RemovalNativeProviderIdle($Owner) {
    $rows=[IntPtr]::Zero;$pointers=[IntPtr]::Zero;$count=[IntPtr]::Zero
    $bridge=$null
    if(-not $script:RemovalNativeBridges.TryGetValue($Owner,[ref]$bridge)){throw 'Owner has no retained native dispatcher capability.'}
    $bridge.BeginIdle()
    try {
        $cap=[Moiren.RemovalTraceQueryV1.Reader]::ProviderCap
        $size=[Moiren.RemovalTraceQueryV1.Reader]::GuidPropertiesSize
        $rows=[Runtime.InteropServices.Marshal]::AllocHGlobal($cap*$size)
        $pointers=[Runtime.InteropServices.Marshal]::AllocHGlobal($cap*[IntPtr]::Size)
        $count=[Runtime.InteropServices.Marshal]::AllocHGlobal(4)
        [Runtime.InteropServices.Marshal]::Copy((New-Object byte[] ($cap*$size)),0,$rows,$cap*$size)
        [Runtime.InteropServices.Marshal]::WriteInt32($count,0)
        for($i=0;$i -lt $cap;$i++){
            [Runtime.InteropServices.Marshal]::WriteIntPtr($pointers,$i*[IntPtr]::Size,[IntPtr]::Add($rows,$i*$size))
        }
        $status=Invoke-RemovalNativeBoundary $Owner 'Enumerate' @($pointers,[UInt32]$cap,$count)
        $bridge.CaptureIdle($status,$false)
        if($status -ne 0){throw 'Classic provider idle enumeration failed; never enable/steal.'}
        $n=[UInt32][Runtime.InteropServices.Marshal]::ReadInt32($count)
        if($n -gt $cap){throw 'Classic provider idle enumeration truncated.'}
        $matches=0;$idle=$false
        for($i=0;$i -lt $n;$i++){
            $row=[Runtime.InteropServices.Marshal]::PtrToStructure([IntPtr]::Add($rows,$i*$size),[type][Moiren.RemovalTraceQueryV1.GuidProperties])
            if($row.Guid -eq [Guid]'7dc75ba9-8274-4eae-a42d-b4c74e0209d2'){
                $matches++;$idle=$row.IsEnable -eq 0
            }
        }
        if($matches -ne 1 -or -not $idle){throw 'Classic provider missing/ambiguous/already enabled; never steal or disable foreign binding.'}
        # This observation is not an atomic reservation. A reviewed combined
        # preflight must exclude competing trace owners; no safety claim here.
        $bridge.CaptureIdle($status,$true)
    } finally {
        if($count -ne [IntPtr]::Zero){[Runtime.InteropServices.Marshal]::FreeHGlobal($count)}
        if($pointers -ne [IntPtr]::Zero){[Runtime.InteropServices.Marshal]::FreeHGlobal($pointers)}
        if($rows -ne [IntPtr]::Zero){[Runtime.InteropServices.Marshal]::FreeHGlobal($rows)}
    }
}

function Start-RemovalNativeTrace {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    $ErrorActionPreference='Stop'
    if($null -eq $Owner.Boundary){Assert-RemovalTraceNativeOwnerApproval}
    $bridge=$null
    if(-not $script:RemovalNativeBridges.TryGetValue($Owner,[ref]$bridge)){throw 'Owner has no retained native dispatcher capability.'}
    $bridge.BeginStart()
    $p=[IntPtr]::Zero;$handle=[IntPtr]::Zero;$status=$null
    try {
        $p=New-RemovalNativeProperties $Owner -Start
        $handle=[Runtime.InteropServices.Marshal]::AllocHGlobal(8)
        [Runtime.InteropServices.Marshal]::WriteInt64($handle,0)
        try {$status=Invoke-RemovalNativeBoundary $Owner 'Start' @($handle,$Owner.Session,$p)}
        finally {
            # Preserve even ambiguous/failed output before freeing the native
            # allocation. Only successful Start grants any cleanup authority.
            $raw=[Runtime.InteropServices.Marshal]::ReadInt64($handle)
            $id=[BitConverter]::ToUInt64([BitConverter]::GetBytes($raw),0)
            $bridge.CaptureStart($status,$id)
        }
        if(-not $Owner.TraceOwned){throw 'Start failed/ambiguous/invalid handle; never adopt an existing session.'}
        Confirm-RemovalNativeProviderIdle $Owner
        $bridge.BeginEnable()
        $enabled=Invoke-RemovalNativeBoundary $Owner 'Enable' @($Owner.Handle,[Guid]'7dc75ba9-8274-4eae-a42d-b4c74e0209d2',[UInt32]1,[byte]4,[UInt64]15,[UInt64]0,[UInt32]5000)
        $bridge.CaptureEnable($enabled)
        if($enabled -ne 0){throw 'Provider enable failed/unknown; exact created owner retained for cleanup.'}
    } catch {$Owner.Fail($_.Exception.Message)}
    finally {
        if($handle -ne [IntPtr]::Zero){[Runtime.InteropServices.Marshal]::FreeHGlobal($handle)}
        if($p -ne [IntPtr]::Zero){[Runtime.InteropServices.Marshal]::FreeHGlobal($p)}
    }
    return $Owner
}

function Assert-RemovalNativeSession($Owner,$Snapshot) {
    if($Snapshot.Handle -ne $Owner.Handle -or $Snapshot.SessionGuid -ne $Owner.SessionGuid -or
        $Snapshot.SessionName -cne $Owner.Session -or $Snapshot.LogFileName -cne $Owner.EtlPath -or
        $Snapshot.ClientContext -ne 1 -or $Snapshot.LogFileMode -ne 1 -or $Snapshot.MaximumFileSize -ne 16){
        throw 'Owned native handle/GUID/name/output/config changed; no by-name adoption.'
    }
}

function Stop-RemovalNativeTrace {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    $ErrorActionPreference='Stop'
    if($null -eq $Owner.Boundary){Assert-RemovalTraceNativeOwnerApproval}
    $bridge=$null
    if(-not $script:RemovalNativeBridges.TryGetValue($Owner,[ref]$bridge)){throw 'Owner has no retained native dispatcher capability.'}
    $bridge.BeginStopQuery()
    $p=[IntPtr]::Zero
    try {
        $p=New-RemovalNativeProperties $Owner
        $queryStatus=Invoke-RemovalNativeBoundary $Owner 'Query' @($Owner.Handle,$p)
        $bridge.CaptureStopQuery($queryStatus)
        if($queryStatus -ne 0){throw 'Exact pre-STOP query failed; retained owner unresolved, no blind STOP/retry.'}
        $before=[Moiren.RemovalTraceQueryV1.Reader]::DecodeSession($p,[Moiren.RemovalTraceQueryV1.Reader]::BufferBytes)
        Assert-RemovalNativeSession $Owner $before
        $bridge.CaptureStopQuerySnapshot($before)
        [Runtime.InteropServices.Marshal]::FreeHGlobal($p);$p=[IntPtr]::Zero
        $p=New-RemovalNativeProperties $Owner
        $bridge.BeginStop()
        $status=Invoke-RemovalNativeBoundary $Owner 'Stop' @($Owner.Handle,$p)
        $bridge.CaptureStop($status)
        if($status -eq 234){throw 'STOP returned MORE_DATA: stopped, final properties/counters unknown; never retry.'}
        if($status -eq 2402){throw 'STOP returned ACTIVE_CONNECTIONS: stopping pending, exact owner retained; never retry.'}
        if($status -ne 0){throw 'STOP failed/unknown; exact owner retained, never retry/name-adopt.'}
        $final=[Moiren.RemovalTraceQueryV1.Reader]::DecodeSession($p,[Moiren.RemovalTraceQueryV1.Reader]::BufferBytes)
        Assert-RemovalNativeSession $Owner $final
        foreach($field in @('EventsLost','LogBuffersLost','RealTimeBuffersLost')){
            if($final.$field -lt $before.$field){throw 'STOP returned decreasing loss counters; final evidence untrusted.'}
        }
        $bridge.CaptureFinal($final)
    } catch {$Owner.Fail($_.Exception.Message)}
    finally {if($p -ne [IntPtr]::Zero){[Runtime.InteropServices.Marshal]::FreeHGlobal($p)}}
    return $Owner.StopReceipt
}
