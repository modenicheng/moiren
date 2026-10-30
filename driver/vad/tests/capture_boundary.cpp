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
constexpr NTSTATUS STATUS_DEVICE_NOT_READY = -4;
constexpr NTSTATUS STATUS_NOT_SUPPORTED = -5;
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
    uint64_t CompletedPackets = 0;
    uint64_t DeliveredPackets = 0;
    uint64_t LastCaptureStart = 0;
};
using ACXSTREAM = VAD_AUDIO_STREAM*;
using PULONGLONG = uint64_t*;
using PULONG = ULONG*;
using PBOOLEAN = bool*;
using PUCHAR = unsigned char*;
static ChildContext* VadChild(WDFDEVICE child) { return child; }
static VAD_AUDIO_STREAM* VadAudioStream(ACXSTREAM stream) { return stream; }
static bool VadAudioIsStreamCurrent(WDFDEVICE child, uint64_t generation)
{
    return child->Shared->Slots[child->Index].StreamGeneration == generation;
}
static NTSTATUS NotifyStatus = STATUS_SUCCESS;
static unsigned NotifyCalls = 0;
static uint64_t NotifiedPacket = 0, NotifiedQpc = 0;
static bool AdvanceGateDuringNotify = false;
static bool ReplaceGenerationDuringNotify = false;
static NTSTATUS AcxRtStreamNotifyPacketComplete(void* opaque, uint64_t packet, uint64_t qpc)
{
    auto stream = static_cast<VAD_AUDIO_STREAM*>(opaque);
    Check(!stream->State.Held && !stream->Device->Shared->Lock.Held,
        "real notification boundary holds neither State nor Shared");
    ++NotifyCalls; NotifiedPacket = packet; NotifiedQpc = qpc;
    if (AdvanceGateDuringNotify) { ++stream->Device->Shared->Slots[0].AdmissionGeneration; }
    if (ReplaceGenerationDuringNotify) {
        auto& shared = *stream->Device->Shared;
        shared.CaptureDiagnostics[0].Reset(++shared.Slots[0].StreamGeneration);
    }
    return NotifyStatus;
}
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
struct TraceRecord {
    const char* Format = nullptr;
    std::array<uint64_t, 14> Values = {};
};
static std::array<TraceRecord, 16> TraceRecords = {};
#define WPP_LEVEL_FLAGS_ENABLED(level, flags) TraceEnabled
constexpr unsigned TRACE_LEVEL_INFORMATION = 4;
constexpr unsigned VAD_CONTROL = 1;
static KIRQL KeGetCurrentIrql() { return CurrentIrql; }
template<typename... Args>
static void VadTrace(unsigned, unsigned, const char* format, Args... args)
{
    Check(CurrentIrql == PASSIVE_LEVEL, "WPP only at passive IRQL");
    Check(TraceLock != nullptr && !TraceLock->Held, "WPP outside shared lock");
    Check(sizeof...(Args) <= 14, "bounded decodable WPP fragment");
    Check(TraceEvents < TraceRecords.size(), "fixed number of first-witness WPP fragments");
    auto& record = TraceRecords[TraceEvents];
    record.Format = format;
    const std::array<uint64_t, sizeof...(Args)> values = {static_cast<uint64_t>(args)...};
    for (size_t i = 0; i < values.size(); ++i) { record.Values[i] = values[i]; }
    ++TraceEvents;
}
#include "capture-boundary-production.inc"

struct Fixture {
    VAD_SHARED Shared = {};
    ChildContext Child = {&Shared};
    std::array<NativePacket, 2> Packets;
    NativePacket& Packet = Packets[0];
    VAD_AUDIO_STREAM Stream = {};
    std::array<unsigned char, 4096> Native = {};
    std::array<unsigned char, 4096> NativeSecond = {};
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
        Stream.Device = &Child; Stream.Stream = &Stream; Stream.Buffers[0] = Native.data();
        Stream.Buffers[1] = NativeSecond.data();
        Stream.Packets = Packets.data(); Stream.TimerScratch = Scratch.data();
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

static void RealCaptureQueryPublishesBoundary()
{
    Fixture f;
    f.Stream.PacketCount = 2;
    f.Packet.RtPacketSize = 1920;
    f.Packet.RtPacketOffset = 2176;
    f.Stream.CompletedPackets = 1;
    f.Stream.LastCaptureStart = 900;
    Check(PacketAddress(&f.Stream, 0) == f.Native.data() + 2176,
        "actual query packet address follows the ACX MDL offset");
    ULONG packet = 99;
    uint64_t qpc = 99;
    bool more = true;
    Check(VadAudioGetCapturePacket(&f.Stream, &packet, &qpc, &more) == STATUS_SUCCESS &&
        packet == 0 && qpc == 900 && !more && f.Stream.DeliveredPackets == 1,
        "actual callback preserves packet, timestamp and delivery bookkeeping");
    Check(VadAudioGetCapturePacket(&f.Stream, &packet, &qpc, &more) == STATUS_DEVICE_NOT_READY &&
        packet == 0 && qpc == 0 && !more && f.Stream.DeliveredPackets == 1,
        "actual repeated query still refuses without fabricating another packet");
    TraceLock = &f.Shared.Lock;
    TraceEvents = 0; TraceEnabled = true;
    TracePendingCaptureDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
    Check(TraceEvents == 4,
        "actual GetCapturePacket success and refusal produce bounded passive evidence");
    Check(std::strstr(TraceRecords[0].Format, "Capture packet query") != nullptr &&
        std::strstr(TraceRecords[0].Format, "stream=") == nullptr &&
        TraceRecords[0].Values[4] == 7 && TraceRecords[0].Values[5] == 0 &&
        TraceRecords[0].Values[6] == 1 && TraceRecords[0].Values[7] == 0 &&
        TraceRecords[0].Values[8] == 1 && TraceRecords[0].Values[9] == 0 &&
        TraceRecords[0].Values[10] == 900 &&
        std::strstr(TraceRecords[1].Format, "Capture packet readback") != nullptr &&
        TraceRecords[1].Values[8] == 0 && TraceRecords[1].Values[9] == 2176 &&
        TraceRecords[1].Values[10] == 16 && TraceRecords[1].Values[11] == 0 &&
        TraceRecords[2].Values[3] == 2 &&
        static_cast<uint32_t>(TraceRecords[2].Values[5]) == static_cast<uint32_t>(STATUS_DEVICE_NOT_READY),
        "passive query/readback fragments preserve status, counters, offset and no new address identity");
    const auto& diagnostic = f.Shared.CaptureDiagnostics[0];
    Check(diagnostic.FirstPacket.Observed && !diagnostic.FirstPacket.Prefix.Nonzero &&
        diagnostic.FirstPacket.Prefix.Bytes == 16 && diagnostic.FirstPacket.OffsetBytes == 2176 &&
        diagnostic.FirstPacket.Index == 0 && diagnostic.FirstPacket.GateEpoch == 7 &&
        diagnostic.FirstPacket.Completed == 1 && diagnostic.FirstPacket.DeliveredBefore == 0 &&
        diagnostic.FirstPacket.DeliveredAfter == 1 && diagnostic.FirstPacket.PacketStartQpc == 900,
        "first successful zero callback retains exact identity, offsets and before/after counters");
    Check(diagnostic.FirstPacketRefusal.Observed &&
        diagnostic.FirstPacketRefusal.Status == static_cast<uint32_t>(STATUS_DEVICE_NOT_READY) &&
        diagnostic.FirstPacketRefusal.DeliveredBefore == 1 &&
        diagnostic.FirstPacketRefusal.DeliveredAfter == 1 &&
        diagnostic.FirstPacketRefusal.Prefix.Bytes == 0,
        "actual repeated-query refusal never reads or claims returned PCM");

    // A later real successful WRITE populates buffer1, not a synthetic prefix.
    f.Payload.fill(41);
    f.Batch.Shared = &f.Shared;
    KIRQL previous;
    KeAcquireSpinLock(&f.Shared.Lock, &previous);
    VadPcmTransferLocked(&f.Shared, 0, PacketAddress(&f.Stream, 1), 480, 4, 1, 1000, &f.Batch);
    KeReleaseSpinLock(&f.Shared.Lock, previous);
    f.Stream.CompletedPackets = 2;
    f.Stream.LastCaptureStart = 950;
    Check(VadAudioGetCapturePacket(&f.Stream, &packet, &qpc, &more) == STATUS_SUCCESS && packet == 1 &&
        diagnostic.FirstNonzeroPacket.Observed && diagnostic.FirstNonzeroPacket.Packet == 1 &&
        diagnostic.FirstNonzeroPacket.Index == 1 && diagnostic.FirstNonzeroPacket.OffsetBytes == 0 &&
        diagnostic.FirstNonzeroPacket.GateEpoch == 7 && diagnostic.FirstNonzeroPacket.Prefix.Bytes == 16 &&
        diagnostic.FirstNonzeroPacket.Prefix.Nonzero && diagnostic.FirstPacket.Packet == 0 &&
        !diagnostic.FirstPacket.Prefix.Nonzero,
        "later actual nonzero packet1 is retained independently from first zero packet0");
    TracePendingCaptureDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
    Check(TraceEvents == 6 && !diagnostic.ReportPacket && !diagnostic.ReportNonzeroPacket &&
        !diagnostic.ReportPacketRefusal, "each fixed callback witness emits only once");
    f.Stream.CompletedPackets = 3;
    VadAudioGetCapturePacket(&f.Stream, &packet, &qpc, &more);
    TracePendingCaptureDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
    Check(TraceEvents == 6 && diagnostic.FirstNonzeroPacket.Packet == 1,
        "later query cannot overwrite or rearm published witnesses");
    Check(CaptureReadPrefix::Read(nullptr, 0).Bytes == 0, "zero length readback never dereferences");
    f.NativeSecond.fill(0); f.NativeSecond[16] = 1;
    Check(!CaptureReadPrefix::Read(f.NativeSecond.data(), 1920).Nonzero,
        "query nonzero is restricted to the first sixteen bytes");
}

static void QueryWrapResetGateAndRefusal()
{
    Fixture f;
    f.Stream.PacketCount = 2;
    f.Packet.RtPacketSize = 1920;
    f.Packet.RtPacketOffset = 2176;
    f.Native[2176] = 51;
    f.Stream.CompletedPackets = 0x100000001ULL;
    f.Stream.DeliveredPackets = 0xffffffffULL;
    ULONG packet;
    uint64_t qpc;
    bool more;
    auto& diagnostic = f.Shared.CaptureDiagnostics[0];
    VadAudioGetCapturePacket(&f.Stream, &packet, &qpc, &more);
    Check(packet == 0 && diagnostic.FirstPacket.Packet == 0 && diagnostic.FirstPacket.Index == 0 &&
        diagnostic.FirstPacket.Completed == 0x100000001ULL &&
        diagnostic.FirstPacket.DeliveredBefore == 0xffffffffULL &&
        diagnostic.FirstPacket.DeliveredAfter == 0x100000001ULL &&
        diagnostic.FirstPacket.Prefix.Nonzero,
        "actual ULONG packet wrap retains full counters and correct buffer0 offset");
    diagnostic.Reset(4);
    VadAudioGetCapturePacket(&f.Stream, &packet, &qpc, &more);
    Check(!diagnostic.FirstPacketRefusal.Observed && !diagnostic.FirstPacket.Observed,
        "retired callback cannot populate new generation diagnostics");
    f.Shared.Slots[0].StreamGeneration = f.Stream.Generation = 4;
    ++f.Shared.Slots[0].AdmissionGeneration;
    f.Stream.CompletedPackets = 0x100000002ULL;
    VadAudioGetCapturePacket(&f.Stream, &packet, &qpc, &more);
    Check(diagnostic.FirstPacket.Observed && diagnostic.FirstPacket.GateEpoch == 8 &&
        diagnostic.FirstPacket.Packet == 1 && diagnostic.FirstPacket.Index == 1 &&
        !diagnostic.FirstNonzeroPacket.Observed,
        "new generation records its own query gate without fabricating copy provenance");
    diagnostic.Reset(4);
    f.Stream.PacketCount = 1;
    const auto delivered = f.Stream.DeliveredPackets;
    Check(VadAudioGetCapturePacket(&f.Stream, &packet, &qpc, &more) == STATUS_NOT_SUPPORTED &&
        diagnostic.FirstPacketRefusal.Status == static_cast<uint32_t>(STATUS_NOT_SUPPORTED) &&
        diagnostic.FirstPacketRefusal.Prefix.Bytes == 0 && f.Stream.DeliveredPackets == delivered,
        "timer path preserves unsupported status and never claims a packet read");
    diagnostic.Reset(4);
    f.Stream.PacketCount = 2; f.Stream.Packets = nullptr;
    Check(VadAudioGetCapturePacket(&f.Stream, &packet, &qpc, &more) == STATUS_DEVICE_NOT_READY &&
        diagnostic.FirstPacketRefusal.Observed && diagnostic.FirstPacketRefusal.Prefix.Bytes == 0,
        "pre-allocation refusal retains status without touching a nonexistent native buffer");
}

static void RealNotifyResultAndGateIdentity()
{
    Fixture f;
    f.Stream.PacketCount = 2;
    f.Packet.RtPacketSize = 1920;
    f.Packet.RtPacketOffset = 2176;
    auto& diagnostic = f.Shared.CaptureDiagnostics[0];
    KIRQL previous;
    KeAcquireSpinLock(&f.Stream.State, &previous);
    const auto first = RecordEventCaptureCopy(&f.Stream, 0, 0, 1000, f.Batch);
    KeReleaseSpinLock(&f.Stream.State, previous);
    Check(first.Observed && first.GateEpoch == 7 && !diagnostic.FirstCopy.Observed,
        "notify identity exists even for a zero/unserved packet with no successful WRITE");
    NotifyStatus = STATUS_SUCCESS; NotifyCalls = 0;
    AdvanceGateDuringNotify = true;
    NotifyPacketComplete(&f.Stream, 0, 1000, first);
    AdvanceGateDuringNotify = false;
    Check(NotifyCalls == 1 && NotifiedPacket == 0 && NotifiedQpc == 1000 &&
        diagnostic.FirstNotifySuccess.Observed && diagnostic.FirstNotifySuccess.Status == 0 &&
        diagnostic.FirstNotifySuccess.GateEpoch == 7 && f.Shared.Slots[0].AdmissionGeneration == 8,
        "actual successful notify records pre-notify gate rather than relabeling it after transition");
    auto next = first;
    next.GateEpoch = 8; next.NativeSequence = 1; next.CompletionQpc = 1100;
    NotifyStatus = STATUS_DEVICE_NOT_READY;
    NotifyPacketComplete(&f.Stream, 1, 1100, next);
    Check(NotifyCalls == 2 && NotifiedPacket == 1 && NotifiedQpc == 1100 &&
        diagnostic.FirstNotifyFailure.Status == static_cast<uint32_t>(STATUS_DEVICE_NOT_READY) &&
        diagnostic.FirstNotifyFailure.GateEpoch == 8 && diagnostic.FirstNotifyFailure.NativeSequence == 1 &&
        diagnostic.FirstNotifySuccess.NativeSequence == 0,
        "actual negative notify result remains separate from the first successful notification");
    TraceLock = &f.Shared.Lock; TraceEvents = 0; TraceEnabled = false;
    TracePendingCaptureDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
    Check(TraceEvents == 0 && diagnostic.ReportNotifySuccess && diagnostic.ReportNotifyFailure,
        "disabled WPP retains both actual notify results");
    TraceEnabled = true;
    TracePendingCaptureDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
    Check(TraceEvents == 2 && !diagnostic.ReportNotifySuccess && !diagnostic.ReportNotifyFailure,
        "passive publication drains two fixed notify results");
    Check(std::strstr(TraceRecords[0].Format, "Capture packet notify") != nullptr &&
        std::strstr(TraceRecords[0].Format, "stream=") == nullptr &&
        TraceRecords[0].Values[3] == 0 && TraceRecords[0].Values[4] == 7 &&
        TraceRecords[0].Values[5] == 0 && TraceRecords[0].Values[6] == 1000 &&
        TraceRecords[0].Values[7] == 2 && TraceRecords[0].Values[8] == 0 &&
        TraceRecords[1].Values[3] == 1 && TraceRecords[1].Values[4] == 8 &&
        TraceRecords[1].Values[5] == 1 && TraceRecords[1].Values[6] == 1100 &&
        static_cast<uint32_t>(TraceRecords[1].Values[8]) == static_cast<uint32_t>(STATUS_DEVICE_NOT_READY),
        "passive notify output preserves both actual statuses and their distinct packet/gate identities");
    NotifyPacketComplete(&f.Stream, 1, 1100, next);
    TracePendingCaptureDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
    Check(NotifyCalls == 3 && TraceEvents == 2,
        "diagnostic dedup never suppresses a real notification or rearms first evidence");
    diagnostic.Reset(4);
    NotifyPacketComplete(&f.Stream, 1, 1100, next);
    Check(!diagnostic.FirstNotifyFailure.Observed && !diagnostic.FirstNotifySuccess.Observed,
        "generation reset refuses a late old notification result");
    NotifyPacketComplete(&f.Stream, 2, 1200, {});
    Check(NotifyCalls == 5 && !diagnostic.FirstNotifyFailure.Observed,
        "unobserved/render notification still calls ACX exactly once without capture evidence");
    NotifyStatus = STATUS_SUCCESS;
}

static void FirstNonzeroCopyHasItsOwnNotifyResult(NTSTATUS result)
{
    Fixture f;
    f.Stream.PacketCount = 2;
    f.Packet.RtPacketSize = 1920;
    f.Packet.RtPacketOffset = 2176;
    KIRQL previous;
    KeAcquireSpinLock(&f.Stream.State, &previous);
    const auto empty = RecordEventCaptureCopy(&f.Stream, 0, 0, 900, f.Batch);
    KeReleaseSpinLock(&f.Stream.State, previous);
    NotifyStatus = STATUS_SUCCESS;
    NotifyPacketComplete(&f.Stream, 0, 900, empty);
    TraceLock = &f.Shared.Lock; TraceEvents = 0; TraceEnabled = true;
    TracePendingCaptureDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
    Check(TraceEvents == 1, "first successful zero packet notification is already published");

    // Use the actual WRITE transfer and actual event wrapper. The second
    // successful notify must not be hidden by first-success deduplication.
    f.Payload.fill(43);
    f.Batch.Shared = &f.Shared;
    KeAcquireSpinLock(&f.Stream.State, &previous);
    KIRQL sharedPrevious;
    KeAcquireSpinLock(&f.Shared.Lock, &sharedPrevious);
    VadPcmTransferLocked(&f.Shared, 0, PacketAddress(&f.Stream, 1), 480, 4, 1, 1000, &f.Batch);
    KeReleaseSpinLock(&f.Shared.Lock, sharedPrevious);
    const auto nonzero = RecordEventCaptureCopy(&f.Stream, 1, 1, 1000, f.Batch);
    KeReleaseSpinLock(&f.Stream.State, previous);
    Check(f.Batch.Completions.Count == 1 && f.Batch.Completions.Entries[0].Status == 0 &&
        nonzero.NonzeroCopy && f.NativeSecond[0] == 43,
        "real successful WRITE selects exactly its first nonzero event packet");
    NotifyStatus = result;
    FixtureQpc = 2000; FixtureQpcStep = 7; FixtureQpcCalls = 0;
    AdvanceGateDuringNotify = true;
    NotifyPacketComplete(&f.Stream, 1, 1000, nonzero);
    AdvanceGateDuringNotify = false;
    const auto& diagnostic = f.Shared.CaptureDiagnostics[0];
    Check(FixtureQpcCalls == 2 && diagnostic.NonzeroNotify.Observed &&
        diagnostic.NonzeroNotify.EnterQpc == 2000 && diagnostic.NonzeroNotify.ReturnQpc == 2007 &&
        diagnostic.NonzeroNotify.Status == static_cast<uint32_t>(result) &&
        diagnostic.NonzeroNotify.Index == 1 && diagnostic.FirstNonzero.NativeSequence == 1 &&
        diagnostic.FirstNonzero.RequestSequence == 1 && diagnostic.FirstNonzero.Qpc == 1000 &&
        diagnostic.FirstNonzero.GateEpoch == 7 && f.Shared.Slots[0].AdmissionGeneration == 8 &&
        diagnostic.FirstNotifySuccess.NativeSequence == 0,
        "same-call QPC and actual result attach to nonzero packet, never earlier success or newer gate");
    TraceEvents = 0;
    TracePendingCaptureDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
    bool matched = false;
    for (unsigned i = 0; i < TraceEvents; ++i) {
        const auto& trace = TraceRecords[i];
        if (std::strstr(trace.Format, "Capture nonzero notify") == nullptr) { continue; }
        matched = true;
        Check(std::strstr(trace.Format, "stream=") == nullptr && trace.Values[2] == 3 &&
            trace.Values[3] == 7 && trace.Values[4] == 1 && trace.Values[5] == 1 &&
            trace.Values[6] == 1 && trace.Values[7] == 1000 && trace.Values[8] == 2000 &&
            trace.Values[9] == 2007 && static_cast<uint32_t>(trace.Values[10]) ==
                static_cast<uint32_t>(result),
            "passive output has exact native/request/index/copy/call identity and actual success or failure");
    }
    Check(matched, "first nonzero native1/request1 needs its own actual notify result after native0 success");
    TraceEvents = 0;
    TracePendingCaptureDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
    Check(TraceEvents == 0 && !diagnostic.ReportNonzeroNotify,
        "one passive drain consumes pending publication, not the retained nonzero notify witness");
    KeAcquireSpinLock(&f.Stream.State, &previous);
    const auto later = RecordEventCaptureCopy(&f.Stream, 0, 2, 1100, f.Batch);
    KeReleaseSpinLock(&f.Stream.State, previous);
    Check(!later.NonzeroCopy, "later packet cannot request additional nonzero-call QPC samples");
    FixtureQpcCalls = 0;
    NotifyPacketComplete(&f.Stream, 2, 1100, later);
    Check(FixtureQpcCalls == 0 && !diagnostic.ReportNonzeroNotify &&
        diagnostic.NonzeroNotify.EnterQpc == 2000 && diagnostic.NonzeroNotify.Status ==
            static_cast<uint32_t>(result), "later notify neither resamples nor replaces the first nonzero result");
    FixtureQpc = 1001; FixtureQpcStep = 0; NotifyStatus = STATUS_SUCCESS;
}

static void NonzeroNotifyDoesNotCrossIdentityOrGeneration()
{
    Fixture f;
    auto& diagnostic = f.Shared.CaptureDiagnostics[0];
    CaptureCopyObservation copy = {7, 1, 2, 1000, 2, 1920, 1920, 0, 0,
        {16, true, true, true}, true};
    diagnostic.RecordCopy(3, 77, copy);
    CaptureNotifyObservation notification = {7, 1, 1000, 2, 0, true, true};
    for (unsigned field = 0; field < 5; ++field) {
        auto mismatch = notification;
        switch (field) {
        case 0: ++mismatch.GateEpoch; break;
        case 1: ++mismatch.NativeSequence; break;
        case 2: ++mismatch.CompletionQpc; break;
        case 3: mismatch.PacketCount = 1; break;
        default: mismatch.NonzeroCopy = false; break;
        }
        diagnostic.RecordNotify(3, mismatch, 2000, 2001);
        Check(!diagnostic.NonzeroNotify.Observed,
            "gate/sequence/copy-time/shape or unselected notification cannot attach another call's result");
    }
    diagnostic.RecordNotify(2, notification, 2000, 2001);
    Check(!diagnostic.NonzeroNotify.Observed, "retired generation never completes a current copy witness");
    ReplaceGenerationDuringNotify = true;
    NotifyPacketComplete(&f.Stream, 1, 1000, notification);
    ReplaceGenerationDuringNotify = false;
    Check(diagnostic.Generation == 4 && !diagnostic.FirstNonzero.Observed &&
        !diagnostic.NonzeroNotify.Observed && !diagnostic.ReportNonzeroNotify,
        "generation reset during actual notification cannot publish an old result into new diagnostics");
    notification.NonzeroCopy = false;
    diagnostic.RecordNotify(4, notification, 2000, 2001);
    Check(!diagnostic.NonzeroNotify.Observed, "notification before any copy cannot invent nonzero association");
}

int main()
{
    RealTimerCopyWithoutRebase();
    RealWrapAndFailedWrite();
    EventCopyResetAndPassivePublication();
    PositionRequiresActualAllocationAndCopy();
    RealCaptureQueryPublishesBoundary();
    QueryWrapResetGateAndRefusal();
    RealNotifyResultAndGateIdentity();
    FirstNonzeroCopyHasItsOwnNotifyResult(STATUS_SUCCESS);
    FirstNonzeroCopyHasItsOwnNotifyResult(STATUS_DEVICE_NOT_READY);
    NonzeroNotifyDoesNotCrossIdentityOrGeneration();
    std::printf("PASS: real PCM-to-Scratch-to-ring, zero/nonzero, no-rebase, wrap, "
        "generation reset, event copy, actual query/notify results, same-copy notify success/failure QPC "
        "and passive publication; record=%zu bytes\n",
        sizeof(CaptureDiagnostic));
}
#endif
