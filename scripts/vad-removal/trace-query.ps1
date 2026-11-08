#requires -Version 5.1
# Definitions only. No ETW API is called by dot-source or Initialize.
# Microsoft contracts (legacy enumeration includes RegisterTraceGuids/WPP):
# https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-enumeratetraceguids
# https://learn.microsoft.com/en-us/windows/win32/api/evntrace/ns-evntrace-trace_guid_properties
# https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-controltracew
# Complete means bounded readback/association, not atomic provider state,
# ownership creation, final STOP evidence, zero loss, or native qualification.
Set-StrictMode -Version 2

function Initialize-RemovalTraceQuery {
    if('Moiren.RemovalTraceQueryV1.Reader' -as [type]){return}
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

namespace Moiren.RemovalTraceQueryV1 {
    // Windows SDK evntrace.h/wmistr.h, default native packing. BOOLEAN is one byte.
    [StructLayout(LayoutKind.Sequential)]
    public struct Wnode {
        public uint BufferSize, ProviderId;
        public ulong HistoricalContext, TimeStamp;
        public Guid Guid;
        public uint ClientContext, Flags;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct Properties {
        public Wnode Wnode;
        public uint BufferSize, MinimumBuffers, MaximumBuffers, MaximumFileSize;
        public uint LogFileMode, FlushTimer, EnableFlags;
        public int AgeLimit;
        public uint NumberOfBuffers, FreeBuffers, EventsLost, BuffersWritten;
        public uint LogBuffersLost, RealTimeBuffersLost;
        public IntPtr LoggerThreadId;
        public uint LogFileNameOffset, LoggerNameOffset;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct GuidProperties {
        public Guid Guid;
        public uint GuidType, LoggerId, EnableLevel, EnableFlags;
        public byte IsEnable;
    }
    public sealed class Session {
        public string SessionName, LogFileName;
        public Guid SessionGuid;
        public ulong Handle;
        public uint ClientContext, LogFileMode, MaximumFileSize;
        public uint EventsLost, LogBuffersLost, RealTimeBuffersLost;
    }
    public sealed class Provider {
        public Guid Guid;
        public bool Enabled;
        public uint LoggerId, Level, EnableFlags;
        // Modern keyword fields are not evidence for classic WPP.
        public ulong? MatchAnyKeyword, MatchAllKeyword;
        public string Kind = "ClassicWpp";
    }
    public sealed class Snapshot {
        public uint SchemaVersion = 1;
        public bool Complete;
        public uint? QueryStatus, ProviderQueryStatus, AssociationQueryStatus, FinalQueryStatus;
        public string SessionName, LogFileName, Error;
        public Guid SessionGuid;
        public ulong Handle;
        public uint? LoggerId, ClientContext, LogFileMode, MaximumFileSize;
        public uint? EventsLost, LogBuffersLost, RealTimeBuffersLost;
        public Provider Provider;
    }
    public static class Reader {
        public const int NameChars = 1024;
        public const int ProviderCap = 4096;
        public static readonly int PropertiesSize = Marshal.SizeOf(typeof(Properties));
        public static readonly int GuidPropertiesSize = Marshal.SizeOf(typeof(GuidProperties));
        public static readonly int BufferBytes = PropertiesSize + NameChars * 4;

        [DllImport("advapi32.dll", EntryPoint="ControlTraceW", CharSet=CharSet.Unicode, ExactSpelling=true)]
        private static extern uint ControlTrace(ulong handle, string name, IntPtr properties, uint code);
        [DllImport("advapi32.dll", ExactSpelling=true)]
        private static extern uint EnumerateTraceGuids(IntPtr properties, uint count, out uint returned);

        private static IntPtr Allocate(int bytes) {
            IntPtr p = Marshal.AllocHGlobal(bytes);
            try { Marshal.Copy(new byte[bytes], 0, p, bytes); return p; }
            catch { Marshal.FreeHGlobal(p); throw; }
        }
        private static string ReadString(IntPtr p, int bytes, uint offset) {
            if(offset < PropertiesSize || (offset & 1) != 0 || offset > bytes - 2)
                throw new InvalidOperationException("Invalid returned ETW string offset.");
            int room = Math.Min(NameChars, (bytes - (int)offset) / 2);
            int chars = 0;
            while(chars < room && Marshal.ReadInt16(p, (int)offset + chars * 2) != 0) chars++;
            if(chars == 0 || chars == room) throw new InvalidOperationException("Empty/unterminated ETW string.");
            byte[] data = new byte[chars * 2];
            Marshal.Copy(IntPtr.Add(p, (int)offset), data, 0, data.Length);
            return new UnicodeEncoding(false, false, true).GetString(data);
        }
        // Also usable by the future STOP owner on its still-allocated returned
        // properties buffer. This does not call ETW or attest provider state.
        public static Session DecodeSession(IntPtr p, int bytes) {
            if(p == IntPtr.Zero || bytes < PropertiesSize || bytes > BufferBytes)
                throw new InvalidOperationException("Invalid ETW properties allocation.");
            Properties v = (Properties)Marshal.PtrToStructure(p, typeof(Properties));
            if(v.Wnode.BufferSize < PropertiesSize || v.Wnode.BufferSize > bytes)
                throw new InvalidOperationException("ETW returned properties length exceeds allocation.");
            if(v.Wnode.Guid == Guid.Empty || v.Wnode.HistoricalContext == 0 ||
                v.Wnode.HistoricalContext == ulong.MaxValue)
                throw new InvalidOperationException("ETW returned no session identity.");
            Session s = new Session();
            s.SessionName = ReadString(p, (int)v.Wnode.BufferSize, v.LoggerNameOffset);
            s.LogFileName = ReadString(p, (int)v.Wnode.BufferSize, v.LogFileNameOffset);
            s.SessionGuid = v.Wnode.Guid; s.Handle = v.Wnode.HistoricalContext;
            s.ClientContext = v.Wnode.ClientContext; s.LogFileMode = v.LogFileMode;
            s.MaximumFileSize = v.MaximumFileSize; s.EventsLost = v.EventsLost;
            s.LogBuffersLost = v.LogBuffersLost; s.RealTimeBuffersLost = v.RealTimeBuffersLost;
            return s;
        }
        private static Session ReadSession(ulong handle, Guid sessionGuid, Func<ulong,IntPtr,uint> query, Action<uint> record) {
            IntPtr p = Allocate(BufferBytes);
            try {
                Properties v = new Properties();
                v.Wnode.BufferSize = (uint)BufferBytes; v.Wnode.Flags = 0x20000;
                // ControlTrace requires the owned GUID as an input field even
                // for QUERY. It is not independent evidence by itself. The
                // returned handle/names/config/counters start zero and are read
                // from the API result; ownership remains the StartTrace owner.
                v.Wnode.Guid = sessionGuid;
                v.LoggerNameOffset = (uint)PropertiesSize;
                v.LogFileNameOffset = (uint)(PropertiesSize + NameChars * 2);
                Marshal.StructureToPtr(v, p, false);
                uint status = query(handle, p);
                record(status);
                return status == 0 ? DecodeSession(p, BufferBytes) : null;
            } finally { Marshal.FreeHGlobal(p); }
        }
        private static Provider ReadProvider(Guid expected, Func<IntPtr,uint,IntPtr,uint> enumerate,
            Action<uint> record) {
            IntPtr rows = IntPtr.Zero, pointers = IntPtr.Zero, count = IntPtr.Zero;
            try {
                rows = Allocate(checked(ProviderCap * GuidPropertiesSize));
                pointers = Allocate(checked(ProviderCap * IntPtr.Size));
                count = Allocate(4);
                for(int i=0;i<ProviderCap;i++)
                    Marshal.WriteIntPtr(pointers, i * IntPtr.Size, IntPtr.Add(rows, i * GuidPropertiesSize));
                uint status = enumerate(pointers, ProviderCap, count);
                record(status);
                uint returned = unchecked((uint)Marshal.ReadInt32(count));
                if(status != 0) return null;
                if(returned > ProviderCap) throw new InvalidOperationException("ETW provider enumeration truncated.");
                GuidProperties found = new GuidProperties(); int matches = 0;
                for(int i=0;i<(int)returned;i++) {
                    // Read only the owned slab; never follow returned pointers.
                    GuidProperties v = (GuidProperties)Marshal.PtrToStructure(
                        IntPtr.Add(rows, i * GuidPropertiesSize), typeof(GuidProperties));
                    if(v.Guid == expected) { found = v; matches++; }
                }
                if(matches != 1 || found.IsEnable != 1 || found.LoggerId == 0 || found.EnableLevel > 255)
                    throw new InvalidOperationException("Classic WPP binding missing/ambiguous/disabled.");
                Provider provider = new Provider();
                provider.Guid = found.Guid; provider.Enabled = true; provider.LoggerId = found.LoggerId;
                provider.Level = found.EnableLevel; provider.EnableFlags = found.EnableFlags;
                return provider;
            } finally {
                if(count != IntPtr.Zero) Marshal.FreeHGlobal(count);
                if(pointers != IntPtr.Zero) Marshal.FreeHGlobal(pointers);
                if(rows != IntPtr.Zero) Marshal.FreeHGlobal(rows);
            }
        }
        private static bool SameSession(Session a, Session b) {
            return a.Handle == b.Handle && a.SessionGuid == b.SessionGuid &&
                String.Equals(a.SessionName,b.SessionName,StringComparison.Ordinal) &&
                String.Equals(a.LogFileName,b.LogFileName,StringComparison.Ordinal) &&
                a.ClientContext == b.ClientContext && a.LogFileMode == b.LogFileMode &&
                a.MaximumFileSize == b.MaximumFileSize;
        }
        private static void CopySession(Snapshot r, Session s) {
            r.SessionName=s.SessionName; r.SessionGuid=s.SessionGuid; r.Handle=s.Handle;
            r.LogFileName=s.LogFileName; r.ClientContext=s.ClientContext; r.LogFileMode=s.LogFileMode;
            r.MaximumFileSize=s.MaximumFileSize; r.EventsLost=s.EventsLost;
            r.LogBuffersLost=s.LogBuffersLost; r.RealTimeBuffersLost=s.RealTimeBuffersLost;
        }
        public static Snapshot Query(string name, Guid guid, ulong handle, Guid provider) {
            return QueryWith(name,guid,handle,provider,
                delegate(ulong id,IntPtr p) { return ControlTrace(id,null,p,0); },
                delegate(IntPtr p,uint n,IntPtr count) {
                    uint returned; uint status=EnumerateTraceGuids(p,n,out returned);
                    Marshal.WriteInt32(count,unchecked((int)returned)); return status;
                });
        }
        // Mock delegates execute the exact allocation/parse/association path;
        // tests must supply both, so no mock can fall through to a native call.
        public static Snapshot QueryWith(string name, Guid guid, ulong handle, Guid provider,
            Func<ulong,IntPtr,uint> query, Func<IntPtr,uint,IntPtr,uint> enumerate) {
            if(String.IsNullOrEmpty(name) || name.Length >= NameChars || guid == Guid.Empty ||
                provider == Guid.Empty || handle == 0 || handle == ulong.MaxValue || query == null || enumerate == null)
                throw new ArgumentException("Exact owned ETW identity and both query boundaries required.");
            Snapshot r = new Snapshot();
            try {
                Session first = ReadSession(handle,guid,query,delegate(uint status) { r.QueryStatus=status; });
                if(first == null) return r;
                CopySession(r,first);
                if(first.Handle != handle || first.SessionGuid != guid ||
                    !String.Equals(first.SessionName,name,StringComparison.Ordinal))
                    throw new InvalidOperationException("Owned ETW handle/GUID/name differs.");
                Provider p = ReadProvider(provider,enumerate,delegate(uint status) { r.ProviderQueryStatus=status; });
                if(p == null) return r;
                r.Provider=p; r.LoggerId=p.LoggerId;
                Session associated = ReadSession(p.LoggerId,guid,query,delegate(uint status) { r.AssociationQueryStatus=status; });
                if(associated == null) return r;
                if(!SameSession(first,associated)) throw new InvalidOperationException("Classic WPP logger belongs to another session.");
                Session last = ReadSession(handle,guid,query,delegate(uint status) { r.FinalQueryStatus=status; });
                if(last == null) return r;
                if(!SameSession(first,last)) throw new InvalidOperationException("Owned ETW identity changed during readback.");
                if(last.EventsLost < first.EventsLost || last.LogBuffersLost < first.LogBuffersLost ||
                    last.RealTimeBuffersLost < first.RealTimeBuffersLost)
                    throw new InvalidOperationException("ETW loss counters decreased during readback.");
                CopySession(r,last); r.Complete=true;
            } catch(Exception error) { r.Error=error.GetType().Name+": "+error.Message; }
            return r;
        }
    }
}
'@
}

function Get-RemovalTraceQuerySnapshot {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory=$true)][Alias('Session')][ValidatePattern('\AMoirenRemoval613[A-Za-z0-9_-]{1,64}\z')][string]$SessionName,
        [Parameter(Mandatory=$true)][Guid]$SessionGuid,
        [Parameter(Mandatory=$true)][UInt64]$Handle,
        [Guid]$ProviderGuid='7DC75BA9-8274-4EAE-A42D-B4C74E0209D2',
        [hashtable]$QueryBoundary
    )
    Initialize-RemovalTraceQuery
    if($null -ne $QueryBoundary){
        if($QueryBoundary.Query -isnot [scriptblock] -or $QueryBoundary.Enumerate -isnot [scriptblock]){
            throw 'Both mocked query boundaries are required; no native fallback.'
        }
        return [Moiren.RemovalTraceQueryV1.Reader]::QueryWith($SessionName,$SessionGuid,$Handle,$ProviderGuid,
            [Func[UInt64,IntPtr,UInt32]]$QueryBoundary.Query,[Func[IntPtr,UInt32,IntPtr,UInt32]]$QueryBoundary.Enumerate)
    }
    # Actual read-only native path. No StartTrace/Enable/Stop/CloseTrace import.
    # Caller must place synchronous API reads inside its owned bounded collector.
    # A successful query never grants ownership: Handle/SessionGuid must already
    # belong to the caller's retained successful StartTrace result. No fallback
    # to querying by name, modern empty results, or guessed handle bitfields.
    [Moiren.RemovalTraceQueryV1.Reader]::Query($SessionName,$SessionGuid,$Handle,$ProviderGuid)
}
