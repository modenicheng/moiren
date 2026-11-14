// SPDX-License-Identifier: MIT
#define VAD_DIAGNOSTICS_FIXTURE
#include "diagnostics.cpp"
#include "../src/audio/TimerPcm.h"

using namespace vad::audio;
constexpr bool FALSE = false;
constexpr KIRQL PASSIVE_LEVEL = 0, DISPATCH_LEVEL = 2;
static KIRQL CurrentIrql = PASSIVE_LEVEL;
struct ChildContext { VAD_SHARED* Shared; ULONG Index = 0; uint64_t Id = 5; };
using WDFDEVICE = ChildContext*;
struct NativePacket { ULONG RtPacketSize = 4096, RtPacketOffset = 0; };
struct VAD_AUDIO_STREAM {
    WDFDEVICE Device;
    SpinLock State;
    uint64_t Generation = 3;
    void* Stream = reinterpret_cast<void*>(77);
    ULONG PacketCount = 1, PacketSize = 1920, PacketFrames = 480, BlockAlign = 4;
    ULONG AllocationBytes = 4096;
    bool Capture = false, HasEos = false;
    ULONG EosPacket = 0, EosLength = 0, RenderReadyMask = 3;
    void* Buffers[2] = {};
    NativePacket* Packets;
    void* TimerScratch;
    ULONG TimerScratchBytes = 1920;
    TimerDrivenRing TimerRing = {};
    TimerPcmPacket TimerPacket = {};
};
static ChildContext* VadChild(WDFDEVICE child) { return child; }
static bool ExAcquireRundownProtection(unsigned* rundown) { ++*rundown; return true; }
static void KeAcquireSpinLock(SpinLock* lock, KIRQL* previous)
{
    Check(!lock->Held, "nonrecursive existing lock");
    *previous = CurrentIrql; CurrentIrql = DISPATCH_LEVEL; lock->Held = true;
}
static void KeReleaseSpinLock(SpinLock* lock, KIRQL previous)
{
    Check(lock->Held, "owned lock release");
    lock->Held = false; CurrentIrql = previous;
}
static void ClearTimerPacket(VAD_AUDIO_STREAM* stream)
{
    RtlZeroMemory(stream->TimerScratch, stream->TimerScratchBytes);
    stream->TimerPacket = {};
}
#if MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC
static vad::PlaybackCopyTime TestClock()
{
    const auto qpc = KeQueryPerformanceCounter(nullptr);
    return {static_cast<uint64_t>(qpc.QuadPart), 48000};
}
#endif
static void ZeroTimerRingSpan(VAD_AUDIO_STREAM*, const TimerDrivenSegment&)
{
    Check(false, "fixture keeps PCM rundown available");
}
static CaptureNotifyObservation RecordEventCaptureCopy(VAD_AUDIO_STREAM*, ULONG,
    uint64_t, uint64_t, const VAD_PCM_BATCH&) { return {}; }

#if MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC
constexpr ULONG TRACE_LEVEL_INFORMATION = 4, VAD_CONTROL = 1;
static bool TraceEnabled = false;
static VAD_SHARED* TraceShared = nullptr;
static KIRQL KeGetCurrentIrql() { return CurrentIrql; }
struct NativeTrace { const char* Format; std::array<uint64_t, 14> Values; };
static std::array<NativeTrace, 52> Traces = {};
static uint32_t TraceCount = 0;
#define WPP_LEVEL_FLAGS_ENABLED(level, flags) TraceEnabled
template<class... Args>
static void VadTrace(ULONG, ULONG, const char* format, Args... args)
{
    Check(CurrentIrql == PASSIVE_LEVEL && TraceShared != nullptr && !TraceShared->Lock.Held,
        "actual passive exporter emits outside shared lock");
    Check(sizeof...(Args) <= 14 && TraceCount < Traces.size(), "fixed WPP fragment budget");
    auto& record = Traces[TraceCount++];
    record = {}; record.Format = format;
    const std::array<uint64_t, sizeof...(Args)> values = {static_cast<uint64_t>(args)...};
    for (size_t i = 0; i < values.size(); ++i) { record.Values[i] = values[i]; }
}
#endif
#include "native-playback-production.inc"
#if MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC
#include "native-playback-reserve.inc"
#endif

struct NativeFixture {
    VAD_SHARED Shared = {};
    Queue Requests;
    ChildContext Child = {&Shared};
    std::array<NativePacket, 2> Packets;
    VAD_AUDIO_STREAM Stream = {};
    std::array<unsigned char, 4096> Native = {}, Second = {};
    std::array<unsigned char, 1920> Scratch = {};
    Request Read;
    std::array<unsigned char, sizeof(MOIREN_VAD_PCM_HEADER) + 1920> ReadMdl = {};
    NativeFixture()
    {
        auto& s = Shared.Slots[0];
        s.Id = 5; s.Session = 6; s.StreamGeneration = 3; s.AdmissionGeneration = 7;
        s.State = MOIREN_VAD_STATE_PRESENT; s.StreamState = MOIREN_VAD_STREAM_RUNNING;
        s.GateOpen = s.AudioReady = 1; s.Intent = MOIREN_VAD_INTENT_PLAYBACK;
        s.SampleFormat = MOIREN_VAD_FORMAT_PCM16; s.Channels = 2; s.PacketFrames = 480;
        Shared.PcmQueues[0] = &Requests;
        Stream.Device = &Child; Stream.Packets = Packets.data();
        Stream.Buffers[0] = Native.data(); Stream.Buffers[1] = Second.data();
        Stream.TimerScratch = Scratch.data();
        Check(InitializeTimerDrivenRing(Stream.TimerRing, 1920, 4, 4096), "real timer layout");
    }
    void Segment(ULONG frames = 48)
    {
        TimerDrivenSegment segment = {};
        Check(NextTimerDrivenSegment(Stream.TimerRing, segment, frames), "actual segment planner");
        VAD_PCM_BATCH batch = {};
        KIRQL previous;
        KeAcquireSpinLock(&Stream.State, &previous);
        TransferTimerSegment(&Stream, segment, 1000, &batch);
        KeReleaseSpinLock(&Stream.State, previous);
    }
#if MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC
    void SelectRead()
    {
        auto& read = Shared.PlaybackCopyDiagnostics[0];
        read.Identity = vad::PlaybackCopyIdentity::From(Shared.Slots[0], 480, 4);
        read.Observed = true; read.Revision = 1;
        read.FirstScan = {9, 900, 901, 902};
    }
    void QueueRead()
    {
        auto& s = Shared.Slots[0];
        auto& h = Read.Header;
        h = {}; h.EndpointId = h.EndpointGeneration = s.Id; h.SessionEpoch = s.Session;
        h.StreamGeneration = s.StreamGeneration; h.GateEpoch = s.AdmissionGeneration;
        h.SampleFormat = s.SampleFormat; h.Channels = s.Channels; h.Frames = s.PacketFrames;
        h.Qpc = 1000; Read.Buffer = ReadMdl.data();
        Requests.Next = 0; Requests.Count = 1; Requests.Requests[0] = &Read;
    }
#endif
};

#if MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC
static void ActualFirstReadSelectsNextSpan()
{
    NativeFixture f;
    f.QueueRead();
    f.Native.fill(17);
    const auto before = FixtureQpcCalls;
    for (unsigned i = 0; i < 10; ++i) { f.Segment(); }
    Check(f.Shared.PlaybackCopyDiagnostics[0].Observed &&
        !f.Shared.NativePlaybackDiagnostics[0].Observed && FixtureQpcCalls == before + 2,
        "actual first accepted READ selects epoch, native waits until next span");
    f.Segment();
    const auto& d = f.Shared.NativePlaybackDiagnostics[0];
    Check(d.Observed && d.FirstScan.LogicalSequence == 1 && d.FirstScan.StartFrame == 480 &&
        d.FirstScan.NativeOffset == 1920 && d.FirstScan.DestinationOffset == 0 &&
        d.NativeNonzeroFrames == 48 && d.First.Word(false, 0) == 0x11111111 &&
        d.First.Word(true, 0) == 0x11111111 && f.Native[1920] == 0 &&
        d.SelectingRead.Sequence == 0,
        "real ring copied and witnessed before clear, correlated to selecting READ");
    Check(d.Layout.PacketCount == 1 && d.Layout.PacketSize == 1920 &&
        d.Layout.RtPacketSize[0] == 4096 && d.Layout.AllocationBytes == 4096 &&
        d.Layout.MappedBytes == 4096 && d.Layout.MappedFrames == 1024,
        "logical frames are distinct from actual advertised and mapped extents");
}

static void PassiveExportAndOriginalEpoch()
{
    NativeFixture f;
    f.SelectRead(); f.Native.fill(0x12);
    FixtureQpc = 1100; FixtureQpcStep = 3;
    f.Segment(); FixtureQpcStep = 0;
    auto& d = f.Shared.NativePlaybackDiagnostics[0];
    TraceShared = &f.Shared; TraceEnabled = false; TraceCount = 0;
    TracePendingNativePlaybackDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
    Check(!d.InterimExported && TraceCount == 0, "disabled WPP does not consume interim");
    TraceEnabled = true;
    TracePendingNativePlaybackDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
    Check(TraceCount == 26 && d.InterimExported && Traces[6].Values[11] == 1100 &&
        Traces[6].Values[12] == 1103 && Traces[1].Values[11] == 4096 &&
        Traces[13].Values[7] == 0x12121212 && Traces[14].Values[7] == 0x12121212,
        "actual exporter serializes layout, scan times and paired words");
    TracePendingNativePlaybackDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
    Check(TraceCount == 26, "no repeated interim group");
    auto& s = f.Shared.Slots[0];
    ++s.AdmissionGeneration; ++s.StreamGeneration;
    s.StreamState = MOIREN_VAD_STREAM_CLOSED; s.SampleFormat = s.PacketFrames = 0;
    TraceEnabled = false;
    TracePendingNativePlaybackDiagnostic(&f.Shared, vad::Snapshot(s));
    Check(d.Seal == vad::NativePlaybackSeal::IdentityChanged && !d.SealedExported,
        "QUERY seals changed stamp without a later copied packet even if trace disabled");
    TraceEnabled = true;
    TracePendingNativePlaybackDiagnostic(&f.Shared, vad::Snapshot(s));
    Check(TraceCount == 52 && d.SealedExported, "one complete historical sealed group");
    for (unsigned i = 0; i < TraceCount; ++i) {
        const auto& v = Traces[i].Values;
        Check(v[0] == 5 && v[1] == 6 && v[2] == 3 && v[3] == 7 &&
            v[4] == (i < 26 ? 1u : 2u) && v[5] == i % 26 && v[6] == 26,
            "all fragments repeat original identity/revision and exact fragment indexes");
    }
    TracePendingNativePlaybackDiagnostic(&f.Shared, vad::Snapshot(s));
    Check(TraceCount == 52, "no repeated final group");
}

static void ActualCreateResetOnly()
{
    NativeFixture f;
    f.SelectRead(); f.Segment();
    MOIREN_VAD_CREATE_REQUEST request = {};
    request.Intent = MOIREN_VAD_INTENT_PLAYBACK; request.Channels = 2;
    uint64_t nextId = 20;
    f.Shared.Slots[0].State = MOIREN_VAD_STATE_REMOVED;
    const auto index = ActualReserve(&f.Shared, request, 9, nextId);
    Check(index == 0 && !f.Shared.NativePlaybackDiagnostics[0].Observed,
        "actual CREATE resets fixed native record on slot reservation only");
}

static void ActualEventSelectionAndReadOnlyLabel()
{
    for (ULONG index = 0; index < 2; ++index) {
        NativeFixture f;
        f.Stream.PacketCount = 2;
        f.Packets[0] = {1920, 2176}; f.Packets[1] = {1920, 0};
        f.Native.fill(0x23); f.Second.fill(0x34);
        f.QueueRead();
        VAD_PCM_BATCH batch = {};
        ActualEventTransfer(&f.Stream, index, 9, 1000, batch);
        Check(!f.Shared.NativePlaybackDiagnostics[0].Observed &&
            f.Shared.PlaybackCopyDiagnostics[0].Observed,
            "actual accepted event READ, not transient mode, selects native diagnostic");
        f.Stream.RenderReadyMask = 3;
        f.Native.fill(0x56); f.Second.fill(0x67);
        batch = {};
        ActualEventTransfer(&f.Stream, index, 10, 1000, batch);
        const auto& d = f.Shared.NativePlaybackDiagnostics[0];
        Check(d.Observed && d.Layout.Path == vad::NativePlaybackPath::EventRead &&
            d.Layout.PacketCount == 2 && d.Layout.MappedBytes == 0 && d.Layout.MappedFrames == 0 &&
            d.Layout.RtPacketSize[0] == 1920 && d.Layout.RtPacketOffset[0] == 2176 &&
            d.Layout.RtPacketOffset[1] == 0 && d.FirstScan.Index == index &&
            d.FirstScan.NativeOffset == (index == 0 ? 2176u : 0u) &&
            d.FirstScan.NativeOffset == d.FirstScan.DestinationOffset &&
            d.First.Word(false, 0) == (index == 0 ? 0x56565656u : 0x67676767u) &&
            d.First.Word(false, 0) == d.First.Word(true, 0) && d.DifferentBytes == 0 &&
            !d.FirstMismatch.Observed && d.SelectingRead.Sequence == 9,
            "real event observation reads actual descriptor-selected bytes, not a copy witness");
        TraceShared = &f.Shared; TraceCount = 0; TraceEnabled = true;
        TracePendingNativePlaybackDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
        Check(TraceCount == 26 && Traces[2].Values[11] == 0 && Traces[25].Values[7] == 1,
            "event export explicitly says paired_copy=0 and event_same_buffer=1");
    }
}

static void ActualEventEosTruncationIsExplicit()
{
    NativeFixture f;
    f.SelectRead();
    f.Stream.PacketCount = 2;
    f.Packets[0] = {1920, 2176}; f.Packets[1] = {1920, 0};
    f.Stream.HasEos = true; f.Stream.EosPacket = 19; f.Stream.EosLength = 0;
    f.Native.fill(0x45);
    VAD_PCM_BATCH batch = {};
    ActualEventTransfer(&f.Stream, 0, 19, 1000, batch);
    const auto& d = f.Shared.NativePlaybackDiagnostics[0];
    Check(d.Observed && d.NativeNonzeroBytes == 0 && !d.FirstNonzero.Observed,
        "actual event samples after deliberate EOS tail truncation, not before");
    TraceShared = &f.Shared; TraceEnabled = true; TraceCount = 0;
    TracePendingNativePlaybackDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
    Check(TraceCount == 26 && Traces[7].Values[11] == 3 &&
        Traces[7].Values[12] == 19 && Traces[7].Values[13] == 0,
        "event scan must retain EOS present/applied flags, packet and length explaining zero tail");
}

static void RealRingWrapAndNonzeroTail()
{
    NativeFixture f;
    f.SelectRead();
    f.Stream.TimerRing.RingFrameOffset = 1022;
    f.Native[4095] = 0x49; f.Native[0] = 0x50;
    f.Segment(48); // Actual planner truncates at the ring boundary (two frames).
    f.Segment(46);
    const auto& d = f.Shared.NativePlaybackDiagnostics[0];
    Check(d.InspectedFrames == 48 && d.Calls == 2 && d.SequenceGaps == 0 &&
        d.FirstScan.NativeOffset == 4088 && d.FirstScan.DestinationOffset == 0 &&
        d.LastScan.NativeOffset == 0 && d.LastScan.DestinationOffset == 8 &&
        d.LastScan.StartFrame == 2 && d.NativeNonzeroBytes == 2 && d.DifferentBytes == 0 &&
        d.FirstNonzero.Scan.NativeOffset == 4092 && d.FirstNonzero.FrameOffset == 1 &&
        d.FirstNonzero.Native[3] == 0x49 && d.FirstNonzero.Bytes == 4 &&
        d.FirstNonzero.Scan.Frames == 2 && d.FirstNonzero.Scan.ConsumedFrames == 2 &&
        d.FirstNonzero.Scan.NativeOffset + d.FirstNonzero.Bytes == d.Layout.MappedBytes &&
        f.Native[4095] == 0 && f.Native[0] == 0 && f.Scratch[7] == 0x49 && f.Scratch[8] == 0x50,
        "prefix offsets shift to selected tail but original span lengths remain; bound prefix by Bytes");
}

static auto HostClock()
{
    return [] { return TestClock(); };
}

static void BoundsPartialCapAndNoMoreSamples()
{
    NativeFixture f;
    const auto identity = vad::PlaybackCopyIdentity::From(f.Shared.Slots[0], 480, 4);
    const auto layout = NativePlaybackLayoutFor(&f.Stream);
    vad::NativePlaybackDiagnostic d = {};
    std::array<unsigned char, 1920> a = {}, b = {};
    for (uint32_t i = 0; i < 999; ++i) {
        d.Observe(identity, layout, {i, i, i * 480ull, 1000, 0, 0, 0, 0, 480, 0, 0, 0, 0, 0},
            a.data(), b.data(), HostClock());
    }
    d.Observe(identity, layout, {999, 999, 479520, 1000, 0, 0, 0, 0, 479, 0, 0, 0, 0, 0},
        a.data(), b.data(), HostClock());
    Check(d.InspectedFrames == 479999 && d.Seal == vad::NativePlaybackSeal::None,
        "cap minus one remains open, zero frames are not skipped");
    a[3] = b[3] = 9; a[7] = 17; b[7] = 18;
    d.Observe(identity, layout, {1000, 999, 479999, 1000, 0, 0, 0, 0, 48, 0, 0, 0, 0, 0},
        a.data(), b.data(), HostClock());
    Check(d.InspectedFrames == 480000 && d.InspectedBytes == 1920000 &&
        d.Seal == vad::NativePlaybackSeal::Capacity && d.FirstNonzero.FrameOffset == 479999 &&
        d.FirstNonzero.Bytes == 4 && d.NativeNonzeroBytes == 1 && d.DifferentBytes == 0 &&
        d.LastScan.Frames == 1 && d.LastScan.ConsumedFrames == 48,
        "partial final call scans whole remaining frames only, retains actual consumed length");
    const auto before = FixtureQpcCalls;
    d.Observe(identity, layout, {}, nullptr, nullptr, HostClock());
    Check(d.InspectedFrames == 480000 && FixtureQpcCalls == before,
        "capacity exhausted adds no scans or clocks");
    // Saturation is explicit even though real cap/minimum-one-frame bounds
    // prevent these counters reaching UINT32_MAX during this experiment.
    d.Calls = ~uint32_t{0}; d.Increment(d.Calls);
    Check(d.CounterSaturated && d.Calls == ~uint32_t{0}, "coverage counter cannot wrap");
}

static void MismatchAndInvalidExtent()
{
    NativeFixture f;
    const auto id = vad::PlaybackCopyIdentity::From(f.Shared.Slots[0], 480, 4);
    const auto l = NativePlaybackLayoutFor(&f.Stream);
    unsigned char a[4] = {0, 0, 0, 9}, b[4] = {0, 0, 0, 8};
    vad::NativePlaybackDiagnostic d = {};
    d.Observe(id, l, {0, 0, 0, 1000, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0}, a, b, HostClock());
    Check(d.DifferentBytes == 1 && d.FirstMismatch.Observed &&
        d.FirstMismatch.Native[3] == 9 && d.FirstMismatch.Destination[3] == 8,
        "paired scan preserves actual mismatch without rewriting PCM");
    const auto before = FixtureQpcCalls;
    for (unsigned kind = 0; kind < 5; ++kind) {
        vad::NativePlaybackDiagnostic bad = {};
        auto s = vad::NativePlaybackScan{0, 0, 0, 1000, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0};
        if (kind == 0) { s.Frames = 4801; }
        if (kind == 1) { s.NativeOffset = 4096; }
        if (kind == 2) { s.DestinationOffset = 1920; }
        if (kind == 3) { s.Index = 1; }
        if (kind == 4) { s.NativeOffset = 1; }
        bad.Observe(id, l, s, a, b, HostClock());
        Check(!bad.Observed, "invalid consumed extent never binds or reads");
    }
    Check(FixtureQpcCalls == before, "invalid bounds add no clock calls");
    auto changed = l;
    changed.RtPacketOffset[0] = 4;
    d.Observe(id, changed, {1, 0, 1, 1000, 0, 0, 4, 0, 1, 0, 0, 0, 0, 0}, a, b, HostClock());
    Check(d.Seal == vad::NativePlaybackSeal::LayoutChanged && d.InspectedFrames == 1 &&
        d.Layout.RtPacketOffset[0] == 0, "changed layout seals instead of relabeling stored addresses");
}

static void EpochAdmissionMicAndNoRearm()
{
    for (unsigned kind = 0; kind < 4; ++kind) {
        NativeFixture f;
        f.SelectRead(); f.Segment();
        auto& slot = f.Shared.Slots[0];
        if (kind == 0) { ++slot.AdmissionGeneration; }
        if (kind == 1) { slot.StreamState = MOIREN_VAD_STREAM_PAUSED; }
        if (kind == 2) { slot.GateOpen = 0; }
        if (kind == 3) { slot.AudioReady = 0; }
        TraceShared = &f.Shared; TraceEnabled = false;
        TracePendingNativePlaybackDiagnostic(&f.Shared, vad::Snapshot(slot));
        auto& d = f.Shared.NativePlaybackDiagnostics[0];
        Check(d.Seal != vad::NativePlaybackSeal::None && d.Identity.Gate == 7,
            "QUERY seals gate/stop/admission without relabeling original epoch");
        slot.AdmissionGeneration = 7; slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
        slot.GateOpen = slot.AudioReady = 1;
        f.SelectRead(); f.Segment();
        Check(d.InspectedFrames == 48, "resume/current READ cannot rearm sealed native record");
    }
    NativeFixture transient;
    transient.SelectRead();
    ++transient.Shared.PlaybackCopyDiagnostics[0].Identity.Generation;
    const auto before = FixtureQpcCalls;
    transient.Segment();
    Check(!transient.Shared.NativePlaybackDiagnostics[0].Observed && FixtureQpcCalls == before,
        "historical READ from another stream does not select current stream");
    NativeFixture mic;
    mic.SelectRead(); mic.Stream.Capture = true;
    mic.Shared.Slots[0].Intent = MOIREN_VAD_INTENT_MICROPHONE;
    mic.Segment();
    Check(!mic.Shared.NativePlaybackDiagnostics[0].Observed,
        "actual microphone transfer never enters native Playback diagnostic");
}

static void ActualCapAndLogicalBoundary()
{
    NativeFixture edge;
    edge.SelectRead();
    edge.Stream.TimerRing.PresentedFrames = 479; // Real assembled span, one frame remains.
    edge.Stream.TimerRing.RingFrameOffset = 0;
    edge.Stream.TimerPacket = {6, 7, true, false, 0};
    edge.Segment(48);
    Check(edge.Shared.NativePlaybackDiagnostics[0].FirstScan.Frames == 1,
        "actual logical packet boundary limits consumption to remaining whole frame");
    NativeFixture f;
    f.SelectRead();
    // A 480-frame logical packet is assembled from real bounded timer spans.
    f.Native.fill(1);
    for (unsigned i = 0; i < 10050; ++i) { f.Segment(); }
    const auto& d = f.Shared.NativePlaybackDiagnostics[0];
    Check(d.InspectedFrames <= 480000 && d.SequenceGaps == 0,
        "real ring/native-boundary splits preserve contiguous scan accounting");
    for (unsigned i = 0; i < 11000 && d.Seal == vad::NativePlaybackSeal::None; ++i) { f.Segment(); }
    const auto before = FixtureQpcCalls;
    f.Segment();
    Check(d.InspectedFrames == 480000 && d.Seal == vad::NativePlaybackSeal::Capacity &&
        FixtureQpcCalls == before, "actual exhausted timer hook keeps copy/clear but no diagnostic clocks");
}
#endif

int main()
{
    NativeFixture f;
    f.Native.fill(17);
    const auto before = FixtureQpcCalls;
    f.Segment();
    Check(f.Scratch[0] == 17 && f.Native[0] == 0, "real timer copies before destructive clearing");
#if MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC
#if __has_include("../src/NativePlaybackDiagnostic.h")
    const auto& d = f.Shared.NativePlaybackDiagnostics[0];
    Check(!d.Observed && FixtureQpcCalls == before,
        "unselected transient stream must not bind native evidence before accepted READ");
    f.SelectRead();
    f.Segment();
    Check(d.Observed && d.InspectedFrames == 48 && d.NativeNonzeroFrames == 48 &&
        d.DestinationNonzeroFrames == 48 && d.DifferentBytes == 0,
        "actual ring-to-Scratch hook witnesses nonzero bytes before zero");
#else
    Check(false, "actual ring-to-Scratch pre-clear witness is missing");
#endif
#else
    Check(FixtureQpcCalls == before, "default-off actual timer adds no QPC");
#endif
    (void)before;
    // Keep the extracted event routine exercised even in the default-off build.
    f.Stream.PacketCount = 2; f.Packets[0].RtPacketSize = 1920;
    f.Packets[0].RtPacketOffset = 2176;
    f.Native.fill(21);
    VAD_PCM_BATCH batch = {};
    ActualEventTransfer(&f.Stream, 0, 0, 1001, batch);
    Check(f.Native[2176] == 0 && f.Native[2175] == 21, "real event uses descriptor offset and clears only packet");
#if MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC
    ActualFirstReadSelectsNextSpan();
    PassiveExportAndOriginalEpoch();
    ActualCreateResetOnly();
    ActualEventSelectionAndReadOnlyLabel();
    ActualEventEosTruncationIsExplicit();
    RealRingWrapAndNonzeroTail();
    BoundsPartialCapAndNoMoreSamples();
    MismatchAndInvalidExtent();
    EpochAdmissionMicAndNoRearm();
    ActualCapAndLogicalBoundary();
    Check(sizeof(vad::NativePlaybackDiagnostic) <= 1024, "native state within one KiB per slot");
    std::printf("PASS: 10 native playback groups; per-slot=%zu bytes; 26 fragments per group\n",
        sizeof(vad::NativePlaybackDiagnostic));
#endif
    std::puts("PASS: actual timer/event native playback boundary");
}
