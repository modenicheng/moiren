// SPDX-License-Identifier: MIT
#define VAD_DIAGNOSTICS_FIXTURE
#include "diagnostics.cpp"

#if MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC
constexpr KIRQL PASSIVE_LEVEL = 0, DISPATCH_LEVEL = 2;
constexpr ULONG TRACE_LEVEL_INFORMATION = 4, VAD_CONTROL = 1;
static bool TraceEnabled = false;
static KIRQL CurrentIrql = PASSIVE_LEVEL;
static VAD_SHARED* TracedShared = nullptr;
struct PlaybackTrace {
    const char* Format = nullptr;
    std::array<uint64_t, 14> Values = {};
};
static std::array<PlaybackTrace, 30> Traces = {};
static uint32_t TraceCount = 0;
#define WPP_LEVEL_FLAGS_ENABLED(level, flags) TraceEnabled
static KIRQL KeGetCurrentIrql() { return CurrentIrql; }
static void KeAcquireSpinLock(SpinLock* lock, KIRQL* previous)
{
    Check(!lock->Held, "no recursive shared lock");
    *previous = CurrentIrql; CurrentIrql = DISPATCH_LEVEL; lock->Held = true;
}
static void KeReleaseSpinLock(SpinLock* lock, KIRQL previous)
{
    Check(lock->Held, "shared lock owned");
    lock->Held = false; CurrentIrql = previous;
}
template<class... Args>
static void VadTrace(ULONG, ULONG, const char* format, Args... args)
{
    Check(CurrentIrql == PASSIVE_LEVEL && TracedShared != nullptr &&
        !TracedShared->Lock.Held, "actual exporter emits only passive outside shared lock");
    Check(TraceCount < Traces.size() && sizeof...(Args) <= 14,
        "fixed decodable fragment count/size");
    auto& trace = Traces[TraceCount++];
    trace.Format = format;
    const std::array<uint64_t, sizeof...(Args)> values = {static_cast<uint64_t>(args)...};
    for (size_t i = 0; i < values.size(); ++i) { trace.Values[i] = values[i]; }
}
#include "playback-copy-production.inc"
#include "playback-copy-reserve.inc"
#endif

struct PlaybackFixture {
    VAD_SHARED Shared = {};
    Request Read;
    Queue Requests;
    VAD_PCM_BATCH Batch = {};
    std::array<unsigned char, MOIREN_VAD_MAX_PCM_BYTES> Source = {};
    std::array<unsigned char, sizeof(MOIREN_VAD_PCM_HEADER) + MOIREN_VAD_MAX_PCM_BYTES> Mdl = {};

    PlaybackFixture()
    {
        auto& slot = Shared.Slots[0];
        slot.Id = 5; slot.Session = 6; slot.StreamGeneration = 3;
        slot.AdmissionGeneration = 7;
        slot.State = MOIREN_VAD_STATE_PRESENT;
        slot.GateOpen = slot.AudioReady = 1;
        slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
        slot.Intent = MOIREN_VAD_INTENT_PLAYBACK;
        slot.SampleFormat = MOIREN_VAD_FORMAT_PCM16;
        slot.Channels = 2; slot.PacketFrames = 480;
        Shared.PcmQueues[0] = &Requests;
    }

    void Transfer(uint32_t frames = 480, uint64_t sequence = 9, uint32_t align = 4,
        uint64_t callbackQpc = 1000)
    {
        auto& slot = Shared.Slots[0];
        slot.PacketFrames = frames;
        Read.Header = {};
        auto& header = Read.Header;
        header.EndpointId = header.EndpointGeneration = slot.Id;
        header.SessionEpoch = slot.Session;
        header.StreamGeneration = slot.StreamGeneration;
        header.GateEpoch = slot.AdmissionGeneration;
        header.SampleFormat = slot.SampleFormat;
        header.Channels = slot.Channels; header.Frames = frames;
        header.Qpc = 1000;
        Read.Buffer = Mdl.data();
        Requests.Next = 0; Requests.Count = 1; Requests.Requests[0] = &Read;
        Batch = {}; Batch.Shared = &Shared;
        VadPcmTransferLocked(&Shared, 0, Source.data(), frames, align, sequence, callbackQpc, &Batch);
        Check(Batch.Completions.Count == 1 && Batch.Completions.Entries[0].Status == STATUS_SUCCESS,
            "original accepted READ completion remains successful");
        if (slot.Intent == MOIREN_VAD_INTENT_PLAYBACK) {
            Check(std::memcmp(Mdl.data() + sizeof(header), Source.data(), frames * align) == 0,
            "original READ payload is unchanged");
        }
    }
};

#if MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC
static auto Clock()
{
    return [] {
        const auto tick = KeQueryPerformanceCounter(nullptr);
        return vad::PlaybackCopyTime{static_cast<uint64_t>(tick.QuadPart), 48000};
    };
}

static void ActualScannedCoverageExport()
{
    PlaybackFixture f;
    FixtureQpc = 100; FixtureQpcStep = 3;
    f.Transfer(480, 9, 4, 90); // All zero: timing cannot depend on a nonzero prefix.
    f.Transfer(480, 11, 4, 96);
    FixtureQpcStep = 0;
    const auto& d = f.Shared.PlaybackCopyDiagnostics[0];
    Check(d.AcceptedCopied == 2 && d.SequenceGaps == 1 && !d.CoverageCounterSaturated &&
        d.FirstScan.Sequence == 9 && d.FirstScan.CallbackQpc == 90 &&
        d.FirstScan.BeforeQpc == 100 && d.FirstScan.AfterQpc == 103 &&
        d.LastScan.Sequence == 11 && d.LastScan.CallbackQpc == 96 &&
        d.LastScan.BeforeQpc == 106 && d.LastScan.AfterQpc == 109 &&
        d.First.BeforeQpc == 100 && !d.FirstNonzero.Observed,
        "first/last scanned calls are independent of frozen prefix and zero content");
    TraceEnabled = true; TraceCount = 0; TracedShared = &f.Shared;
    TracePendingPlaybackCopyDiagnostic(&f.Shared, vad::Snapshot(f.Shared.Slots[0]));
    bool first = false, last = false, counts = false;
    for (uint32_t i = 0; i < TraceCount; ++i) {
        first |= std::strstr(Traces[i].Format, "Playback copy coverage_first ") != nullptr;
        last |= std::strstr(Traces[i].Format, "Playback copy coverage_last ") != nullptr;
        counts |= std::strstr(Traces[i].Format, "Playback copy coverage_counts ") != nullptr;
    }
    Check(first && last && counts,
        "actual exporter must retain first/last scanned zero-copy times and sequence-gap counts");
    Check(TraceCount == 15 && Traces[12].Values[7] == 9 && Traces[12].Values[10] == 103 &&
        Traces[13].Values[7] == 11 && Traces[13].Values[10] == 109 &&
        Traces[14].Values[7] == 2 && Traces[14].Values[8] == 1 &&
        Traces[14].Values[9] == 0 && Traces[14].Values[10] == 0,
        "three coverage fragments serialize exact first/last/counts with no all-interval claim");
    for (uint32_t i = 0; i < TraceCount; ++i) {
        Check(Traces[i].Values[5] == i && Traces[i].Values[6] == 15,
            "new fragments use the same fixed complete fragment count");
    }
}

static void StopWithoutTransfer()
{
    for (uint32_t kind = 0; kind < 4; ++kind) {
        PlaybackFixture f;
        f.Transfer();
        auto& slot = f.Shared.Slots[0];
        if (kind == 0) { slot.StreamState = MOIREN_VAD_STREAM_PAUSED; }
        if (kind == 1) { slot.GateOpen = 0; }
        if (kind == 2) { slot.AudioReady = 0; }
        if (kind == 3) { slot.State = MOIREN_VAD_STATE_REMOVING; }
        TraceEnabled = false; TraceCount = 0; TracedShared = &f.Shared;
        TracePendingPlaybackCopyDiagnostic(&f.Shared, vad::Snapshot(slot));
        auto& d = f.Shared.PlaybackCopyDiagnostics[0];
        Check(d.Seal == vad::PlaybackCopySeal::NotAdmitted && !d.SealedExported && TraceCount == 0,
            "QUERY seals stopped state without another transfer, even when trace is disabled");
        TraceEnabled = true;
        TracePendingPlaybackCopyDiagnostic(&f.Shared, vad::Snapshot(slot));
        Check(TraceCount == 15 && d.SealedExported, "later trace emits retained stopped epoch only once");
    }
}

static void ActualExportAndOldIdentity()
{
    PlaybackFixture f;
    f.Source[0] = 0x12; f.Source[1] = 0x34; f.Source[2] = 0x56; f.Source[3] = 0x78;
    f.Transfer();
    auto& diagnostic = f.Shared.PlaybackCopyDiagnostics[0];
    auto& slot = f.Shared.Slots[0];
    TracedShared = &f.Shared; TraceEnabled = false; TraceCount = 0;
    TracePendingPlaybackCopyDiagnostic(&f.Shared, vad::Snapshot(slot));
    Check(!diagnostic.InterimExported && TraceCount == 0, "trace-disabled does not consume export");
    TraceEnabled = true;
    TracePendingPlaybackCopyDiagnostic(&f.Shared, vad::Snapshot(slot));
    Check(TraceCount == 15 && diagnostic.InterimExported, "exactly one interim fragment group");
    Check(Traces[4].Values[9] == 0x78563412 && Traces[5].Values[8] == 0x78563412,
        "actual exporter serializes little-endian paired source/destination prefix words");
    TracePendingPlaybackCopyDiagnostic(&f.Shared, vad::Snapshot(slot));
    Check(TraceCount == 15, "interim query never repeats");
    slot.StreamState = MOIREN_VAD_STREAM_CLOSED;
    slot.SampleFormat = slot.PacketFrames = 0;
    ++slot.StreamGeneration; ++slot.AdmissionGeneration;
    TracePendingPlaybackCopyDiagnostic(&f.Shared, vad::Snapshot(slot));
    Check(TraceCount == 30 && diagnostic.SealedExported, "one sealed group after Stop/shape clear");
    for (uint32_t i = 0; i < TraceCount; ++i) {
        const auto& values = Traces[i].Values;
        Check(values[0] == 5 && values[1] == 6 && values[2] == 3 && values[3] == 7 &&
            values[4] == (i < 15 ? 1u : 2u) && values[5] == i % 15 && values[6] == 15,
            "all fragments repeat original epoch, revision and exact fragment count/index");
    }
    TracePendingPlaybackCopyDiagnostic(&f.Shared, vad::Snapshot(slot));
    Check(TraceCount == 30, "sealed query never repeats");
    slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
    slot.SampleFormat = MOIREN_VAD_FORMAT_PCM16;
    f.Transfer();
    Check(diagnostic.InspectedFrames == 480 && diagnostic.Identity.Generation == 3,
        "new stream acquire does not rearm or relabel historical evidence");
}

static void LimitsAndNonzeroTail()
{
    PlaybackFixture f;
    auto identity = vad::PlaybackCopyIdentity::From(f.Shared.Slots[0], 1, 4);
    vad::PlaybackCopyDiagnostic diagnostic = {};
    unsigned char a[4] = {}, b[4] = {};
    for (uint32_t i = 0; i < vad::PlaybackCopyDiagnostic::FrameCap - 1; ++i) {
        diagnostic.Observe(identity, a, b, i, 1000, Clock());
    }
    Check(diagnostic.InspectedFrames == 479999 && diagnostic.Seal == vad::PlaybackCopySeal::None,
        "cap minus one remains unsealed");
    a[3] = b[3] = 9;
    diagnostic.Observe(identity, a, b, 479999, 1000, Clock());
    Check(diagnostic.InspectedFrames == 480000 && diagnostic.Seal == vad::PlaybackCopySeal::Capacity &&
        diagnostic.FirstNonzero.FrameOffset == 479999 && diagnostic.FirstNonzero.Bytes == 4 &&
        diagnostic.SourceNonzeroBytes == 1 && diagnostic.SourceNonzeroFrames == 1,
        "exact cap records final nonzero frame, no leading-zero skip");
    const auto qpc = FixtureQpcCalls;
    diagnostic.Observe(identity, a, b, 480000, 1000, Clock());
    Check(diagnostic.InspectedFrames == 480000 && FixtureQpcCalls == qpc &&
        diagnostic.AcceptedCopied == 480000 && diagnostic.SequenceGaps == 0 &&
        diagnostic.LastScan.Sequence == 479999,
        "cap plus one adds no readback or QPC");

    // Real production branch: 100*4799 leaves exactly 100 whole frames.
    PlaybackFixture partial;
    partial.Source.fill(1);
    for (uint32_t i = 0; i < 101; ++i) { partial.Transfer(4799, i); }
    const auto& actual = partial.Shared.PlaybackCopyDiagnostics[0];
    Check(actual.InspectedFrames == 480000 && actual.InspectedBytes == 1920000 &&
        actual.SourceNonzeroBytes == 1920000 && actual.DestinationNonzeroFrames == 480000 &&
        actual.AcceptedCopied == 101 && actual.FirstScan.Sequence == 0 &&
        actual.LastScan.Sequence == 100 && actual.SequenceGaps == 0,
        "actual READ final packet is frame-aligned partial coverage only");
    const auto partialQpc = FixtureQpcCalls;
    partial.Transfer(4799, 101);
    Check(actual.AcceptedCopied == 101 && actual.LastScan.Sequence == 100 &&
        FixtureQpcCalls == partialQpc, "cap-exhausted copy never advances retained coverage or QPC");
}

static void CoverageShapesAndOverflow()
{
    for (const uint32_t frames : {48u, 480u}) {
        PlaybackFixture f;
        f.Transfer(frames, 0); f.Transfer(frames, 1); f.Transfer(frames, 1);
        const auto& d = f.Shared.PlaybackCopyDiagnostics[0];
        Check(d.InspectedFrames == 3 * frames && d.AcceptedCopied == 3 && d.SequenceGaps == 1 &&
            d.FirstScan.Sequence == 0 && d.LastScan.Sequence == 1 &&
            d.SourceNonzeroFrames == 0 && d.DestinationNonzeroFrames == 0,
            "separate fixed 48/480-frame zero epochs retain duplicate sequence as gap, not silence");
    }
    PlaybackFixture f;
    f.Transfer(480, ~uint64_t{0}); f.Transfer(480, 0);
    auto& d = f.Shared.PlaybackCopyDiagnostics[0];
    Check(d.SequenceGaps == 1 && d.AcceptedCopied == 2,
        "maximum previous sequence cannot overflow into false consecutive wrap");
    d.AcceptedCopied = d.SequenceGaps = ~uint32_t{0};
    f.Transfer(480, 2);
    Check(d.AcceptedCopied == ~uint32_t{0} && d.SequenceGaps == ~uint32_t{0} &&
        d.CoverageCounterSaturated && d.LastScan.Sequence == 2 && d.FirstScan.Sequence == ~uint64_t{0},
        "coverage counters saturate explicitly without wrap or overwriting first scan");
}

static void PrefixMismatchAndValidation()
{
    PlaybackFixture f;
    auto identity = vad::PlaybackCopyIdentity::From(f.Shared.Slots[0], 480, 4);
    vad::PlaybackCopyDiagnostic d = {};
    f.Source[1919] = 5; f.Mdl[sizeof(MOIREN_VAD_PCM_HEADER) + 1919] = 7;
    FixtureQpc = 100; FixtureQpcStep = 3;
    d.Observe(identity, f.Source.data(), f.Mdl.data() + sizeof(MOIREN_VAD_PCM_HEADER), 33, 90, Clock());
    Check(d.DifferentBytes == 1 && d.SourceNonzeroBytes == 1 && d.DestinationNonzeroBytes == 1 &&
        d.FirstMismatch.PacketByteOffset == 1916 && d.FirstMismatch.MdlByteOffset ==
        sizeof(MOIREN_VAD_PCM_HEADER) + 1916 && d.FirstMismatch.Bytes == 4 &&
        d.FirstMismatch.Source[3] == 5 && d.FirstMismatch.Destination[3] == 7 &&
        d.FirstMismatch.Sequence == 33 && d.FirstMismatch.BeforeQpc == 100 && d.FirstMismatch.AfterQpc == 103,
        "mismatch tail retains exact paired frame prefix/offset/sequence/QPC bracket");
    FixtureQpcStep = 0;
    for (uint32_t kind = 0; kind < 6; ++kind) {
        auto bad = identity;
        if (kind == 0) { bad.Frames = 0; }
        if (kind == 1) { bad.Frames = 4801; }
        if (kind == 2) { bad.BlockAlign = 9; }
        if (kind == 3) { bad.BlockAlign = 2; }
        if (kind == 4) { bad.Format = 0; }
        if (kind == 5) { bad.Channels = 3; }
        vad::PlaybackCopyDiagnostic invalid = {};
        const auto qpc = FixtureQpcCalls;
        invalid.Observe(bad, nullptr, nullptr, 0, 0, Clock());
        Check(!invalid.Observed && FixtureQpcCalls == qpc, "invalid bounds do not dereference or sample QPC");
    }
}

static void EpochAndMic()
{
    for (uint32_t kind = 0; kind < 8; ++kind) {
        PlaybackFixture f;
        f.Transfer();
        auto changed = vad::PlaybackCopyIdentity::From(f.Shared.Slots[0], 480, 4);
        if (kind == 0) { ++changed.Endpoint; }
        if (kind == 1) { ++changed.Session; }
        if (kind == 2) { ++changed.Generation; }
        if (kind == 3) { ++changed.Gate; }
        if (kind == 4) { changed.Format = MOIREN_VAD_FORMAT_FLOAT32; }
        if (kind == 5) { changed.Channels = 1; }
        if (kind == 6) { changed.Frames = 144; }
        if (kind == 7) { changed.BlockAlign = 8; }
        auto& d = f.Shared.PlaybackCopyDiagnostics[0];
        const auto qpc = FixtureQpcCalls;
        d.Observe(changed, nullptr, nullptr, 10, 1000, Clock());
        Check(d.Seal == vad::PlaybackCopySeal::IdentityChanged && d.InspectedFrames == 480 &&
            FixtureQpcCalls == qpc, "first identity/shape change seals without scanning replacement");
    }
    PlaybackFixture mic;
    mic.Shared.Slots[0].Intent = MOIREN_VAD_INTENT_MICROPHONE;
    mic.Transfer();
    Check(!mic.Shared.PlaybackCopyDiagnostics[0].Observed,
        "accepted MIC WRITE never arms Playback diagnostic");
}

static void ActualReserveOnlyReset()
{
    PlaybackFixture f;
    f.Transfer();
    auto& d = f.Shared.PlaybackCopyDiagnostics[0];
    auto& slot = f.Shared.Slots[0];
    slot.State = MOIREN_VAD_STATE_REMOVED;
    MOIREN_VAD_CREATE_REQUEST request = {};
    request.Intent = MOIREN_VAD_INTENT_PLAYBACK; request.Channels = 2;
    uint64_t nextId = 5;
    KIRQL previous;
    KeAcquireSpinLock(&f.Shared.Lock, &previous);
    const auto index = ReservePlaybackFixture(&f.Shared, request, 9, nextId);
    KeReleaseSpinLock(&f.Shared.Lock, previous);
    Check(index == 0 && slot.Id == 6 && slot.Session == 9 && !d.Observed &&
        d.InspectedFrames == 0 && !d.InterimExported && !d.SealedExported,
        "actual successful CREATE reservation alone clears historical diagnostic");
    slot.State = MOIREN_VAD_STATE_PRESENT; slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
    slot.AudioReady = slot.GateOpen = 1;
    slot.StreamGeneration = 1; slot.SampleFormat = MOIREN_VAD_FORMAT_PCM16;
    f.Transfer();
    Check(d.Observed && d.Identity.Endpoint == 6 && d.Identity.Session == 9 && d.InspectedFrames == 480,
        "new reserved endpoint can bind its own first accepted transfer");
    nextId = ~uint64_t{0};
    KeAcquireSpinLock(&f.Shared.Lock, &previous);
    const auto denied = ReservePlaybackFixture(&f.Shared, request, 10, nextId);
    KeReleaseSpinLock(&f.Shared.Lock, previous);
    Check(denied == -2 && d.InspectedFrames == 480, "failed reservation never resets evidence");
}

static void AllPcmShapesAndStale()
{
    for (const auto format : {MOIREN_VAD_FORMAT_PCM16, MOIREN_VAD_FORMAT_FLOAT32}) {
        for (uint32_t channels = 1; channels <= 2; ++channels) {
            PlaybackFixture f;
            auto& slot = f.Shared.Slots[0];
            slot.SampleFormat = format; slot.Channels = channels;
            const uint32_t align = channels * (format == MOIREN_VAD_FORMAT_PCM16 ? 2 : 4);
            f.Source[4800 * align - 1] = 128;
            f.Transfer(4800, 0, align);
            const auto& d = f.Shared.PlaybackCopyDiagnostics[0];
            Check(d.InspectedBytes == 4800 * align && d.SourceNonzeroBytes == 1 &&
                d.SourceNonzeroFrames == 1 && d.DestinationNonzeroFrames == 1 &&
                d.FirstNonzero.FrameOffset == 4799 && d.FirstNonzero.Bytes == align &&
                d.FirstNonzero.Source[align - 1] == 128 && d.FirstNonzero.Destination[align - 1] == 128,
                "actual maximum PCM16/f32 mono/stereo READ records full tail without decoding");
        }
    }
    PlaybackFixture f;
    f.Transfer();
    f.Shared.PlaybackCopyDiagnostics[0].Reset(); // Test precondition, not production rearm.
    f.Requests.Next = 0; --f.Read.Header.GateEpoch;
    f.Batch = {}; f.Batch.Shared = &f.Shared;
    const auto qpc = FixtureQpcCalls;
    VadPcmTransferLocked(&f.Shared, 0, f.Source.data(), 480, 4, 10, 1000, &f.Batch);
    Check(!f.Shared.PlaybackCopyDiagnostics[0].Observed && FixtureQpcCalls == qpc &&
        f.Batch.Completions.Entries[0].Status == STATUS_REVISION_MISMATCH,
        "stale READ remains rejected and never binds or scans diagnostic");
}
#endif

int main()
{
    PlaybackFixture fixture;
    fixture.Source[1919] = 37;
    const auto qpcBefore = FixtureQpcCalls;
    fixture.Transfer();
#if MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC
#if __has_include("../src/PlaybackCopyDiagnostic.h")
    const auto& diagnostic = fixture.Shared.PlaybackCopyDiagnostics[0];
    Check(diagnostic.InspectedFrames == 480 && diagnostic.SourceNonzeroBytes == 1 &&
        diagnostic.DestinationNonzeroBytes == 1 && diagnostic.DifferentBytes == 0,
        "actual accepted READ records post-copy source and MDL including nonzero tail");
    ActualScannedCoverageExport();
    CoverageShapesAndOverflow();
    StopWithoutTransfer();
    ActualExportAndOldIdentity();
    LimitsAndNonzeroTail();
    PrefixMismatchAndValidation();
    EpochAndMic();
    ActualReserveOnlyReset();
    AllPcmShapesAndStale();
#else
    Check(false, "accepted Playback READ has no actual post-copy source/MDL witness");
#endif
#else
    Check(FixtureQpcCalls == qpcBefore, "default-off actual transfer adds no QPC");
#endif
    (void)qpcBefore;
#if MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC
    std::printf("PASS: 10 playback-copy groups (real READ/export/reserve, limits, shapes, epoch, MIC, first/last coverage and saturation); per-slot=%zu bytes\n",
        sizeof(vad::PlaybackCopyDiagnostic));
#else
    std::puts("PASS: default-off actual Playback READ unchanged, no added QPC/state/scan");
#endif
}
