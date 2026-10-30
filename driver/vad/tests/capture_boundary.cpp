// SPDX-License-Identifier: MIT
#include <cstdio>
#if !__has_include("../src/audio/CaptureDiagnostic.h")
int main()
{
    std::fputs("FAIL: successful capture copy has no bounded source/ring/position evidence\n", stderr);
    return 1;
}
#else
#define VAD_DIAGNOSTICS_FIXTURE
#include "diagnostics.cpp"
#include "../src/audio/TimerPcm.h"

using namespace vad::audio;
constexpr KIRQL PASSIVE_LEVEL = 0;
constexpr KIRQL DISPATCH_LEVEL = 2;
constexpr bool FALSE = false;
static KIRQL CurrentIrql = PASSIVE_LEVEL;
struct ChildContext { VAD_SHARED* Shared; ULONG Index = 0; uint64_t Id = 5; };
using WDFDEVICE = ChildContext*;
struct NativePacket { ULONG RtPacketSize = 4096; ULONG RtPacketOffset = 0; };
struct VAD_AUDIO_STREAM {
    WDFDEVICE Device;
    SpinLock State;
    uint64_t Generation = 3;
    void* Stream = reinterpret_cast<void*>(77);
    ULONG PacketCount = 1;
    ULONG PacketSize = 1920;
    ULONG PacketFrames = 480;
    ULONG BlockAlign = 4;
    bool Capture = true;
    void* Buffers[2] = {};
    NativePacket* Packets;
    void* TimerScratch;
    ULONG TimerScratchBytes = 1920;
    TimerDrivenRing TimerRing = {};
    TimerPcmPacket TimerPacket = {};
    Clock Clock = {48000, 1000, 0, 480, true};
};
using ACXSTREAM = VAD_AUDIO_STREAM*;
using PULONGLONG = uint64_t*;
using PUCHAR = unsigned char*;
static ChildContext* VadChild(WDFDEVICE child) { return child; }
static VAD_AUDIO_STREAM* VadAudioStream(ACXSTREAM stream) { return stream; }
static bool ExAcquireRundownProtection(unsigned* rundown) { ++*rundown; return true; }
static void KeAcquireSpinLock(SpinLock* lock, KIRQL* previous)
{
    Check(!lock->Held, "no recursive lock");
    *previous = CurrentIrql;
    CurrentIrql = DISPATCH_LEVEL;
    lock->Held = true;
}
static void KeReleaseSpinLock(SpinLock* lock, KIRQL previous)
{
    Check(lock->Held, "release owned lock");
    lock->Held = false;
    CurrentIrql = previous;
}
static void ClearTimerPacket(VAD_AUDIO_STREAM* stream)
{
    RtlZeroMemory(stream->TimerScratch, stream->TimerScratchBytes);
    stream->TimerPacket = {};
}
static void ZeroTimerRingSpan(VAD_AUDIO_STREAM*, const TimerDrivenSegment&)
{
    Check(false, "fixture keeps PCM rundown available");
}
static bool TraceEnabled = false;
static unsigned TraceEvents = 0;
static SpinLock* TraceLock = nullptr;
#define WPP_LEVEL_FLAGS_ENABLED(level, flags) TraceEnabled
constexpr unsigned TRACE_LEVEL_INFORMATION = 4;
constexpr unsigned VAD_CONTROL = 1;
static KIRQL KeGetCurrentIrql() { return CurrentIrql; }
template<typename... Args>
static void VadTrace(unsigned, unsigned, const char*, Args...)
{
    Check(CurrentIrql == PASSIVE_LEVEL, "WPP only at passive IRQL");
    Check(TraceLock != nullptr && !TraceLock->Held, "WPP outside shared lock");
    Check(sizeof...(Args) <= 14, "bounded decodable WPP fragment");
    ++TraceEvents;
}
#include "capture-boundary-production.inc"

struct Fixture {
    VAD_SHARED Shared = {};
    ChildContext Child = {&Shared};
    NativePacket Packet;
    VAD_AUDIO_STREAM Stream = {};
    std::array<unsigned char, 4096> Native = {};
    std::array<unsigned char, 1920> Scratch = {};
    std::array<unsigned char, 1920> Payload = {};
    Request Write;
    Queue Requests;
    VAD_PCM_BATCH Batch = {};

    Fixture()
    {
        auto& slot = Shared.Slots[0];
        slot.Id = 5; slot.Session = 6; slot.StreamGeneration = 3;
        slot.AdmissionGeneration = 7;
        slot.GateOpen = slot.AudioReady = 1;
        slot.State = MOIREN_VAD_STATE_PRESENT;
        slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
        slot.Intent = MOIREN_VAD_INTENT_MICROPHONE;
        slot.Channels = 2; slot.SampleFormat = MOIREN_VAD_FORMAT_PCM16;
        slot.PacketFrames = 480;
        Shared.CaptureDiagnostics[0].Reset(3);
        Shared.PcmQueues[0] = &Requests;
        auto& header = Write.Header;
        header.EndpointId = header.EndpointGeneration = 5;
        header.SessionEpoch = 6; header.StreamGeneration = 3; header.GateEpoch = 7;
        header.SampleFormat = MOIREN_VAD_FORMAT_PCM16;
        header.Channels = 2; header.Frames = 480; header.Qpc = 1000; header.Sequence = 1;
        Write.Buffer = Payload.data();
        Requests.Requests[0] = &Write; Requests.Count = 1;
        Stream.Device = &Child; Stream.Buffers[0] = Native.data();
        Stream.Packets = &Packet; Stream.TimerScratch = Scratch.data();
        Check(InitializeTimerDrivenRing(Stream.TimerRing, 1920, 4, 4096), "real ring shape");
    }

    void Segment(unsigned frames = 48)
    {
        Batch = {};
        TimerDrivenSegment segment = {};
        Check(NextTimerDrivenSegment(Stream.TimerRing, segment, frames), "real copy plan");
        KIRQL previous;
        KeAcquireSpinLock(&Stream.State, &previous);
        TransferTimerSegment(&Stream, segment, 1000, &Batch);
        KeReleaseSpinLock(&Stream.State, previous);
    }
};

static void RealTimerCopyWithoutRebase()
{
    Fixture f;
    f.Segment();
    auto& diagnostic = f.Shared.CaptureDiagnostics[0];
    Check(f.Batch.Completions.Count == 1 && f.Batch.Completions.Entries[0].Status == 0,
        "real PCM request copied successfully, not a stub");
    Check(diagnostic.FirstCopy.Observed && !diagnostic.FirstCopy.Prefix.SourceNonzero &&
        !diagnostic.FirstCopy.Prefix.DestinationNonzero && diagnostic.FirstCopy.Prefix.Equal,
        "successful zero copy is observed even without a clock rebase");
    Check(!diagnostic.FirstNonzero.Observed, "zero is not a nonzero witness");
    // Reach the next real request, rather than fabricating a nonzero scratch.
    for (unsigned i = 0; i < 9; ++i) { f.Segment(); }
    f.Payload.fill(17);
    f.Requests.Next = 0;
    f.Segment();
    Check(diagnostic.FirstNonzero.Observed && diagnostic.FirstNonzero.Prefix.SourceNonzero &&
        diagnostic.FirstNonzero.Prefix.DestinationNonzero && diagnostic.FirstNonzero.Prefix.Equal &&
        diagnostic.FirstNonzero.PacketOffsetBytes == 0 && diagnostic.FirstNonzero.PacketCount == 1 &&
        diagnostic.FirstNonzero.NativeSequence == 1 && !diagnostic.FirstCopy.Prefix.SourceNonzero,
        "actual Scratch-to-ring copy retains later nonzero prefix");
    uint64_t frames = 0, qpc = 0;
    Check(VadAudioGetPosition(&f.Stream, &frames, &qpc) == 0 && frames == 528 &&
        diagnostic.Position.Observed && diagnostic.Position.Frames == frames &&
        diagnostic.Position.PacketCount == 1, "real ACX position callback retains returned value");
}

static void RealWrapAndFailedWrite()
{
    Fixture f;
    f.Payload.fill(23);
    f.Stream.TimerRing.RingFrameOffset = 1022;
    f.Segment();
    f.Segment();
    const auto& copy = f.Shared.CaptureDiagnostics[0].FirstNonzero;
    Check(f.Native[4088] == 23 && f.Native[4095] == 23 && f.Native[0] == 23 &&
        copy.RingOffsetBytes == 4088 && copy.Prefix.Bytes == 8 && copy.Prefix.Equal,
        "real transfer and copy wrapper preserve prefix and bytes across ring wrap");
    Fixture rejected;
    rejected.Write.Header.Qpc = 1001;
    rejected.Payload.fill(45);
    rejected.Segment();
    Check(rejected.Batch.Completions.Entries[0].Status == STATUS_IO_TIMEOUT &&
        !rejected.Shared.CaptureDiagnostics[0].FirstCopy.Observed,
        "rejected WRITE cannot become a successful copy witness");
}

static void EventCopyResetAndPassivePublication()
{
    Fixture f;
    f.Stream.PacketCount = 2;
    f.Packet.RtPacketSize = 1920;
    f.Packet.RtPacketOffset = 2176;
    f.Payload.fill(31);
    // The real transfer produces the exact post-copy prefix used by the event
    // wrapper. Neither the memcpy nor successful request classification is stubbed.
    f.Batch.Shared = &f.Shared;
    f.Stream.Packets = &f.Packet;
    KIRQL previous;
    KeAcquireSpinLock(&f.Shared.Lock, &previous);
    VadPcmTransferLocked(&f.Shared, 0, f.Native.data() + 2176, 480, 4, 9, 1000, &f.Batch);
    KeReleaseSpinLock(&f.Shared.Lock, previous);
    RecordEventCaptureCopy(&f.Stream, 0, 9, 1000, f.Batch);
    auto& diagnostic = f.Shared.CaptureDiagnostics[0];
    Check(diagnostic.FirstCopy.PacketCount == 2 && diagnostic.FirstCopy.NativeSequence == 9 &&
        diagnostic.FirstCopy.RequestSequence == 1 && diagnostic.FirstCopy.RingOffsetBytes == 2176 &&
        diagnostic.FirstCopy.Prefix.SourceNonzero && diagnostic.FirstCopy.Prefix.DestinationNonzero &&
        diagnostic.FirstCopy.Prefix.Equal && f.Native[2176] == 31 && f.Native[0] == 0,
        "event evidence comes from actual successful direct copy at packet offset");
    uint64_t frames, qpc;
    VadAudioGetPosition(&f.Stream, &frames, &qpc);
    auto snapshot = vad::Snapshot(f.Shared.Slots[0]);
    TraceLock = &f.Shared.Lock;
    TraceEvents = 0; TraceEnabled = false;
    TracePendingCaptureDiagnostic(&f.Shared, snapshot);
    Check(TraceEvents == 0 && diagnostic.ReportCopy, "disabled WPP does not consume evidence");
    TraceEnabled = true;
    TracePendingCaptureDiagnostic(&f.Shared, snapshot);
    Check(TraceEvents == 5 && !diagnostic.ReportCopy && !diagnostic.ReportPosition,
        "one passive drain emits two bounded copy witnesses and actual position");
    TracePendingCaptureDiagnostic(&f.Shared, snapshot);
    Check(TraceEvents == 5, "repeated QUERY cannot re-emit first witnesses");
    RecordEventCaptureCopy(&f.Stream, 0, 99, 1100, f.Batch);
    Check(!diagnostic.ReportCopy && !diagnostic.ReportNonzero &&
        diagnostic.FirstCopy.NativeSequence == 9 && diagnostic.FirstNonzero.NativeSequence == 9,
        "later copies cannot overwrite or rearm the two fixed witnesses");
    ++snapshot.StreamGeneration;
    diagnostic.Reset(snapshot.StreamGeneration);
    RecordEventCaptureCopy(&f.Stream, 0, 9, 1000, f.Batch);
    VadAudioGetPosition(&f.Stream, &frames, &qpc);
    Check(!diagnostic.FirstCopy.Observed && !diagnostic.Position.Observed,
        "retired generation cannot populate reset evidence");
    Check(!diagnostic.FirstNonzero.Observed && !diagnostic.ReportCopy &&
        !diagnostic.ReportPosition, "reset clears every pending and historical witness");
    ++f.Shared.Slots[0].StreamGeneration;
    f.Stream.Generation = snapshot.StreamGeneration;
    f.Batch.CaptureGateEpoch = ++f.Shared.Slots[0].AdmissionGeneration;
    RecordEventCaptureCopy(&f.Stream, 0, 10, 1001, f.Batch);
    Check(diagnostic.FirstCopy.Observed && diagnostic.FirstCopy.GateEpoch == 8 &&
        diagnostic.FirstCopy.NativeSequence == 10,
        "new generation retains its own gate and copy, not the old record");
    const auto mismatch = CapturePrefix::ReadBack(f.Payload.data(), f.Scratch.data(), 1920);
    Check(mismatch.Bytes == 16 && mismatch.SourceNonzero && !mismatch.DestinationNonzero &&
        !mismatch.Equal, "readback diagnoses mismatched zero destination without scanning whole packet");
    const auto noBytes = CapturePrefix::ReadBack(nullptr, nullptr, 0);
    Check(noBytes.Bytes == 0, "empty prefix never touches a pointer");
    f.Payload.fill(0);
    f.Payload[16] = 31;
    const auto bounded = CapturePrefix::ReadBack(f.Payload.data(), f.Scratch.data(), 1920);
    Check(bounded.Bytes == 16 && !bounded.SourceNonzero && bounded.Equal,
        "nonzero outside fixed prefix does not become a whole-packet claim");
}

static void PositionRequiresActualAllocationAndCopy()
{
    Fixture f;
    uint64_t frames, qpc;
    f.Stream.Packets = nullptr;
    f.Stream.PacketCount = 0;
    VadAudioGetPosition(&f.Stream, &frames, &qpc);
    auto& diagnostic = f.Shared.CaptureDiagnostics[0];
    Check(!diagnostic.Position.Observed, "pre-allocation query cannot consume first position witness");
    f.Stream.Packets = &f.Packet;
    f.Stream.PacketCount = 1;
    VadAudioGetPosition(&f.Stream, &frames, &qpc);
    Check(!diagnostic.Position.Observed, "pre-copy query cannot be paired with a later payload");
    f.Payload.fill(37);
    f.Segment();
    ++f.Shared.Slots[0].AdmissionGeneration;
    VadAudioGetPosition(&f.Stream, &frames, &qpc);
    Check(!diagnostic.Position.Observed, "different gate cannot masquerade as copy's returned position");
}

int main()
{
    RealTimerCopyWithoutRebase();
    RealWrapAndFailedWrite();
    EventCopyResetAndPassivePublication();
    PositionRequiresActualAllocationAndCopy();
    std::printf("PASS: real PCM-to-Scratch-to-ring, zero/nonzero, no-rebase, wrap, "
        "generation reset, event copy and passive position publication; record=%zu bytes\n",
        sizeof(CaptureDiagnostic));
}
#endif
