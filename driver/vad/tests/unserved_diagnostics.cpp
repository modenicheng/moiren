// SPDX-License-Identifier: MIT
#include <cstdio>
#include <cstdlib>

#if !__has_include("../src/UnservedDiagnostics.h")
int main()
{
    std::fputs("FAIL: independent first unserved reasons and safe pre-forward evidence are missing\n", stderr);
    return 1;
}
#else
#define VAD_DIAGNOSTICS_FIXTURE
#include "diagnostics.cpp"

using VAD_PCM_REQUEST = Request;
using WDFDEVICE = VAD_SHARED*;
using PMDL = void*;
constexpr KIRQL PASSIVE_LEVEL = 0;
constexpr NTSTATUS STATUS_DEVICE_REMOVED = -4;
constexpr NTSTATUS STATUS_INVALID_PARAMETER = -5;
constexpr NTSTATUS STATUS_QUOTA_EXCEEDED = -6;
constexpr NTSTATUS STATUS_INFO_LENGTH_MISMATCH = -7;
constexpr NTSTATUS STATUS_INSUFFICIENT_RESOURCES = -8;
constexpr NTSTATUS STATUS_NOT_FOUND = -9;
constexpr NTSTATUS STATUS_PENDING = 1;
constexpr unsigned NormalPagePriority = 0;
constexpr unsigned MdlMappingNoExecute = 0;
constexpr unsigned MdlMappingNoWrite = 0;
constexpr unsigned Executive = 0;
constexpr unsigned KernelMode = 0;
constexpr bool FALSE = false;
constexpr unsigned TRACE_LEVEL_INFORMATION = 4;
constexpr unsigned VAD_CONTROL = 1;
#define PAGED_CODE() ((void)0)
#define WPP_LEVEL_FLAGS_ENABLED(level, flags) TraceEnabled

static KIRQL CurrentIrql = PASSIVE_LEVEL;
static bool TraceEnabled = false;
static VAD_SHARED* ActiveShared = nullptr;
struct TraceRecord {
    const char* Format = nullptr;
    std::array<uint64_t, 14> Values = {};
    size_t Count = 0;
};
static std::array<TraceRecord, 6> TraceRecords = {};
static size_t TraceCount = 0;
static KIRQL KeGetCurrentIrql() { return CurrentIrql; }
static void KeAcquireSpinLock(SpinLock* lock, KIRQL* previous)
{
    Check(!lock->Held, "no lock recursion");
    lock->Held = true;
    *previous = CurrentIrql;
    CurrentIrql = 2;
}
static void KeReleaseSpinLock(SpinLock* lock, KIRQL previous)
{
    Check(lock->Held, "release owned shared lock");
    lock->Held = false;
    CurrentIrql = previous;
}
template<typename... Args>
static void VadTrace(unsigned, unsigned, const char* format, Args... args)
{
    Check(CurrentIrql == PASSIVE_LEVEL && !ActiveShared->Lock.Held,
        "WPP is passive after shared unlock");
    Check(sizeof...(Args) <= 14 && TraceCount < TraceRecords.size(),
        "bounded decodable first-only fragments");
    auto& record = TraceRecords[TraceCount++];
    record.Format = format;
    record.Count = sizeof...(Args);
    const std::array<uint64_t, sizeof...(Args)> values = {static_cast<uint64_t>(args)...};
    for (size_t i = 0; i < values.size(); ++i) { record.Values[i] = values[i]; }
}
static LONG InterlockedCompareExchange(LONG* value, LONG, LONG) { return *value; }
// Production uses VadRoot(device)->Shared; keep the root separate from shared.
struct Root { VAD_SHARED* Shared; };
static Root RootContext = {};
#define VadRoot(device) (RootContext.Shared = (device), &RootContext)
struct WDF_REQUEST_PARAMETERS {
    struct { struct { ULONG InputBufferLength; ULONG OutputBufferLength; }
        DeviceIoControl; } Parameters;
};
static MOIREN_VAD_PCM_HEADER SubmitHeader = {};
static unsigned char SubmitBuffer[960] = {};
static bool ForwardFails = false;
static void WDF_REQUEST_PARAMETERS_INIT(WDF_REQUEST_PARAMETERS*) {}
static void WdfRequestGetParameters(WDFREQUEST, WDF_REQUEST_PARAMETERS* parameters)
{
    parameters->Parameters.DeviceIoControl.InputBufferLength = sizeof(SubmitHeader);
    parameters->Parameters.DeviceIoControl.OutputBufferLength = sizeof(SubmitBuffer);
}
static NTSTATUS WdfRequestRetrieveInputBuffer(WDFREQUEST, size_t, void** input, void*)
{ *input = &SubmitHeader; return STATUS_SUCCESS; }
static NTSTATUS WdfRequestRetrieveOutputWdmMdl(WDFREQUEST, PMDL* mdl)
{ *mdl = SubmitBuffer; return STATUS_SUCCESS; }
static PVOID MmGetSystemAddressForMdlSafe(PMDL mdl, unsigned) { return mdl; }
static bool ExAcquireRundownProtection(unsigned*) { return true; }
static void ExReleaseRundownProtection(unsigned*) {}
static ULONG FindIndex(VAD_SHARED*, uint64_t) { return 0; }
static void KeWaitForSingleObject(unsigned*, unsigned, unsigned, bool, void*) {}
static void KeReleaseMutex(unsigned*, bool) {}
static void DropCount(Request*) {}
static NTSTATUS WdfRequestForwardToIoQueue(WDFREQUEST request, Queue* queue)
{
    Check(!ActiveShared->Lock.Held, "forward remains outside shared lock");
    Check(ActiveShared->UnservedDiagnostics[0].LatestWrite.Sequence == SubmitHeader.Sequence,
        "real submit captures WRITE boundary before ownership transfer");
    if (ForwardFails) { return STATUS_DEVICE_REMOVED; }
    queue->Requests[queue->Count++] = request;
    // A successful forward may complete immediately: poison the context to
    // catch accidental post-forward identity reads in the real submit body.
    request->Header = {};
    request->Shared = nullptr;
    return STATUS_SUCCESS;
}
#include "unserved-production.inc"

static void PureFirstReasons()
{
    vad::UnservedDiagnostics diagnostic = {};
    diagnostic.Reset(3);
    const vad::WriteBoundary write = {11, 7, 90, 95};
    diagnostic.RecordPreForward(3, write);
    const vad::CallbackBoundary callback = {100, 105, 99, 77, 10000000};
    diagnostic.RecordUnserved(3, 11, callback, -3, 1,
        vad::UnservedEmpty);
    auto first = diagnostic.TakeUnreported();
    Check(first.First[0].Flags & vad::UnservedReport,
        "first empty is retained independently of the existing first transfer");
    Check(first.First[0].WriteSequence == 7 &&
        first.First[0].WriteHeaderQpc == 90 &&
        first.First[0].WritePreForwardQpc == 95 &&
        first.First[0].RetrieveStatus == -3,
        "empty snapshot retains latest safe pre-forward identity and actual retrieve status");
    diagnostic.RecordUnserved(3, 11, callback, 0, 4,
        vad::UnservedNoCurrent | vad::UnservedTooOld);
    auto later = diagnostic.TakeUnreported();
    Check(!(later.First[0].Flags & vad::UnservedReport) &&
        (later.First[1].Flags & vad::UnservedReport) &&
        (later.First[2].Flags & vad::UnservedReport),
        "later no-current and too-old reasons retain their own first callback");
    Check(sizeof(diagnostic) <= 256, "additional per-endpoint budget");

    diagnostic.RecordPreForward(2, {11, 999, 999, 999});
    Check(diagnostic.LatestWrite.Sequence == 7, "retired generation cannot replace current write");
    diagnostic.Reset(4);
    diagnostic.RecordPreForward(4, {11, 8, 120, 125});
    diagnostic.RecordUnserved(4, 12, callback, -3, 1, vad::UnservedEmpty);
    Check(!(diagnostic.First[0].Flags & vad::UnservedHasWrite) &&
        diagnostic.First[0].WritePreForwardQpc == 0,
        "retired gate write is not relabeled as current evidence");
    diagnostic.Reset(5);
    diagnostic.RecordUnserved(4, 12, callback, -3, 1, vad::UnservedEmpty);
    Check(diagnostic.First[0].Flags == 0, "retired callback cannot claim new generation");
}

static void ProductionUnserved()
{
    VAD_SHARED shared = {};
    auto& slot = shared.Slots[0];
    slot.Id = 5;
    slot.Session = 6;
    slot.StreamGeneration = 3;
    slot.AdmissionGeneration = 7;
    slot.GateOpen = slot.AudioReady = 1;
    slot.State = MOIREN_VAD_STATE_PRESENT;
    slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
    slot.Intent = MOIREN_VAD_INTENT_MICROPHONE;
    slot.SampleFormat = MOIREN_VAD_FORMAT_PCM16;
    slot.Channels = 1;
    slot.PacketFrames = 480;
    Queue queue = {};
    shared.PcmQueues[0] = &queue;
    shared.UnservedDiagnostics[0].Reset(3);
    shared.UnservedDiagnostics[0].RecordPreForward(3, {7, 20, 900, 950});
    unsigned char pcm[960] = {};
    VAD_PCM_BATCH batch = {};
    batch.Shared = &shared;
    batch.Callback = {1001, 1004, 1000, 77, 48000};
    VadPcmTransferLocked(&shared, 0, pcm, 480, 2, 5, 1004, &batch);
    auto empty = shared.UnservedDiagnostics[0].TakeUnreported();
    Check((empty.First[0].Flags & vad::UnservedReport) &&
        empty.First[0].RetrieveStatus == -3 && empty.First[0].RetrieveCalls == 1 &&
        empty.First[0].DpcQpc == 1001 && empty.First[0].StateQpc == 1004 &&
        empty.First[0].DeadlineQpc == 1000 &&
        empty.First[0].WritePreForwardQpc == 950,
        "actual empty callback retains DPC/State/deadline/retrieve and WRITE boundary");
    Check(!(empty.First[1].Flags & vad::UnservedReport),
        "empty is not conflated with exhausted no-current budget");

    std::array<Request, 4> requests = {};
    for (ULONG i = 0; i < 4; ++i) {
        auto& request = requests[i];
        request.Header.EndpointId = request.Header.EndpointGeneration = 5;
        request.Header.SessionEpoch = 6;
        request.Header.StreamGeneration = 3;
        request.Header.GateEpoch = 6;
        request.Header.SampleFormat = MOIREN_VAD_FORMAT_PCM16;
        request.Header.Channels = 1;
        request.Header.Frames = 480;
        request.Buffer = pcm;
        queue.Requests[i] = &request;
    }
    queue.Count = 4;
    queue.Next = 0;
    batch.Completions = {};
    VadPcmTransferLocked(&shared, 0, pcm, 480, 2, 6, 1004, &batch);
    const auto retired = shared.UnservedDiagnostics[0].TakeUnreported();
    Check(retired.First[1].RetrieveStatus == STATUS_SUCCESS &&
        retired.First[1].RetrieveCalls == 4 &&
        (retired.First[1].Flags & vad::UnservedReport) &&
        !(retired.First[0].Flags & vad::UnservedReport),
        "actual four retired requests keep independent NoCurrent, last successful retrieve");
    for (auto& request : requests) {
        request.Header.GateEpoch = 7;
        request.Header.Qpc = 1; // Older than the unchanged one-period boundary.
    }
    queue.Next = 0;
    batch.Completions = {};
    VadPcmTransferLocked(&shared, 0, pcm, 480, 2, 7, 1004, &batch);
    const auto old = shared.UnservedDiagnostics[0].TakeUnreported();
    Check((old.First[2].Flags & vad::UnservedReport) &&
        old.First[2].RetrieveCalls == 4 && slot.DroppedPackets == 4 &&
        slot.SilencePackets == 3 && batch.Completions.Count == 4,
        "actual TooOld unserved callback has independent first evidence without changing completions");

    shared.UnservedDiagnostics[0].Reset(3);
    requests[1].Header.Qpc = 1000;
    queue.Next = 0;
    batch.Completions = {};
    VadPcmTransferLocked(&shared, 0, pcm, 480, 2, 8, 1004, &batch);
    Check(shared.UnservedDiagnostics[0].First[2].Flags == 0 &&
        batch.Completions.Count == 2 &&
        batch.Completions.Entries[1].Status == STATUS_SUCCESS && slot.SilencePackets == 3,
        "TooOld followed by a fresh request is served and must NOT retain unserved evidence");

    shared.UnservedDiagnostics[0].RecordUnserved(3, 7, batch.Callback,
        -3, 1, vad::UnservedEmpty | vad::UnservedNoCurrent | vad::UnservedTooOld);
    ActiveShared = &shared;
    const auto snapshot = vad::Snapshot(slot);
    TracePendingUnservedDiagnostics(&shared, snapshot);
    Check(TraceCount == 0 && (shared.UnservedDiagnostics[0].First[0].Flags & vad::UnservedReport),
        "disabled WPP does not consume evidence");
    TraceEnabled = true;
    auto mismatched = snapshot;
    ++mismatched.StreamGeneration;
    TracePendingUnservedDiagnostics(&shared, mismatched);
    Check(TraceCount == 0, "passive query cannot misattribute a retired generation");
    TracePendingUnservedDiagnostics(&shared, snapshot);
    Check(TraceCount == 6, "actual passive emitter publishes two fragments for each first reason");
    for (size_t i = 0; i < TraceCount; ++i) {
        const auto& trace = TraceRecords[i];
        Check(trace.Values[0] == 5 && trace.Values[1] == 6 && trace.Values[2] == 3 &&
            trace.Values[3] == 77 && trace.Values[4] == 1004 && trace.Values[5] == i / 2,
            "all fragments correlate endpoint/session/generation/stream/State QPC/reason");
        Check(trace.Count == (i % 2 == 0 ? 13u : 11u), "bounded WPP arguments");
    }
    TracePendingUnservedDiagnostics(&shared, snapshot);
    Check(TraceCount == 6, "passive drain never rearms first evidence");

    shared.UnservedDiagnostics[0].Reset(3);
    SubmitHeader = requests[0].Header;
    SubmitHeader.Size = sizeof(SubmitHeader);
    SubmitHeader.Version = MOIREN_VAD_ABI_VERSION;
    SubmitHeader.Sequence = 42;
    SubmitHeader.Qpc = 990;
    Request submit = {};
    queue = {};
    Check(VadPcmSubmit(&shared, &submit, IOCTL_MOIREN_VAD_WRITE_PCM) == STATUS_PENDING &&
        submit.Header.EndpointId == 0 &&
        shared.UnservedDiagnostics[0].LatestWrite.Sequence == 42 &&
        shared.UnservedDiagnostics[0].LatestWrite.PreForwardQpc == 1001,
        "actual submit records safe boundary and tolerates immediate WDF ownership loss");
    ++SubmitHeader.Sequence;
    ForwardFails = true;
    Check(VadPcmSubmit(&shared, &submit, IOCTL_MOIREN_VAD_WRITE_PCM) == STATUS_DEVICE_REMOVED &&
        shared.UnservedDiagnostics[0].LatestWrite.Sequence == 43,
        "pre-forward evidence is explicitly an attempt, even when forward fails");
    ++SubmitHeader.Sequence;
    --SubmitHeader.GateEpoch;
    Check(VadPcmSubmit(&shared, &submit, IOCTL_MOIREN_VAD_WRITE_PCM) == STATUS_REVISION_MISMATCH &&
        shared.UnservedDiagnostics[0].LatestWrite.Sequence == 43,
        "real admission rejects retired gate without contaminating latest WRITE");
}

int main()
{
    PureFirstReasons();
    ProductionUnserved();
    std::printf("PASS: independent first unserved reasons; endpoint record=%zu bytes\n",
        sizeof(vad::UnservedDiagnostics));
}
#endif
