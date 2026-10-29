// SPDX-License-Identifier: MIT
#include "../src/audio/TimerPcm.h"
#include <array>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

static void Check(bool condition, const char* message)
{
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        std::exit(1);
    }
}

template<unsigned Frames>
struct PacketFixture {
    vad::Slot Slot = {};
    vad::audio::TimerDrivenRing Ring = {};
    vad::audio::TimerPcmPacket Packet = {};
    std::array<unsigned char, 4096> Native = {};
    std::array<unsigned char, Frames * 4> Scratch = {};
    std::array<unsigned char, Frames * 4> Payload = {};
    std::array<unsigned char, Frames * 4> Transferred = {};
    unsigned Transfers = 0;
    bool PayloadAvailable = true;
    bool Capture;

    explicit PacketFixture(bool capture) : Capture(capture)
    {
        Slot.Id = 5;
        Slot.Session = 9;
        Slot.AdmissionGeneration = 11;
        Slot.StreamGeneration = 3;
        Slot.State = MOIREN_VAD_STATE_PRESENT;
        Slot.AudioReady = Slot.GateOpen = 1;
        Slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
        Slot.Intent = capture ? MOIREN_VAD_INTENT_MICROPHONE : MOIREN_VAD_INTENT_PLAYBACK;
        Slot.Channels = 2;
        Slot.SampleFormat = MOIREN_VAD_FORMAT_PCM16;
        Slot.PacketFrames = Frames;
        Check(vad::audio::InitializeTimerDrivenRing(Ring, Frames * 4, 4, 4096),
            "fixture uses a logical packet shorter than its page-backed ring");
        Native.fill(7);
        Payload.fill(5);
    }

    vad::audio::TimerDrivenSegment Segment(
        unsigned budget = vad::audio::DefaultTimerSegmentFrames)
    {
        vad::audio::TimerDrivenSegment segment = {};
        Check(vad::audio::NextTimerDrivenSegment(Ring, segment, budget), "segment advances");
        Check(vad::audio::TransferTimerSegment(Slot, 5, 3, Capture, Ring, segment, Packet,
            [&] { Scratch.fill(0); },
            [&](const vad::audio::TimerDrivenCopySpan& span) {
                auto ring = Native.data() + span.RingByteOffset;
                auto packet = Scratch.data() + span.PacketByteOffset;
                if (Capture) {
                    std::memcpy(ring, packet, span.ByteCount);
                } else {
                    std::memcpy(packet, ring, span.ByteCount);
                }
            },
            [&](const vad::audio::TimerDrivenCopySpan& span) {
                std::memset(Native.data() + span.RingByteOffset, 0, span.ByteCount);
            },
            [&] {
                ++Transfers;
                if (Capture) {
                    // Models the existing queue transfer contract: an empty
                    // or already-canceled queue leaves zeroed scratch.
                    if (PayloadAvailable) {
                        Scratch = Payload;
                    }
                } else {
                    Transferred = Scratch;
                }
            }), "production PCM helper accepts the fixture segment");
        return segment;
    }
};

using Fixture = PacketFixture<49>;

struct ElapsedFixture : PacketFixture<480> {
    static constexpr uint64_t Epoch = 10000000;
    vad::audio::Clock Clock = {};
    unsigned Spans = 0;
    unsigned PacketEnds = 0;

    explicit ElapsedFixture(bool capture) : PacketFixture<480>(capture)
    {
        Clock.Frequency = 10000000;
        Clock.EpochQpc = Epoch;
        Clock.Running = true;
    }

    void LoadPayload(unsigned seed)
    {
        for (unsigned i = 0; i < Payload.size(); ++i) {
            Payload[i] = static_cast<unsigned char>((seed + i * 17 + i / 7) % 251 + 1);
            if (!Capture) {
                Native[(Ring.RingFrameOffset * 4 + i) % Native.size()] = Payload[i];
            }
        }
    }

    vad::audio::TimerDrivenStep Tick(uint64_t now)
    {
        const auto before = Ring.PresentedFrames;
        const auto transfers = Transfers;
        const auto step = vad::audio::PlanTimerDrivenStep(Clock, Ring, now);
        Check(step.Valid && step.Due < 0 && step.Frames <= Ring.PacketFrames,
            "elapsed callback has a finite future deadline and one-packet budget");
        // Mirrors Streams.cpp's reset/dispatch glue, not WDF request completion.
        // Clock planning and every byte copy below use the production helpers.
        if (step.Rebased) {
            vad::InvalidatePcmEpoch(Slot);
            Packet = {};
            Scratch.fill(0);
            Native.fill(0);
        }
        Spans = PacketEnds = 0;
        unsigned remaining = step.Frames;
        for (; remaining != 0 && Spans < 3; ++Spans) {
            const auto segment = Segment(remaining);
            remaining -= segment.Frames;
            PacketEnds += segment.CompletesPacket ? 1u : 0u;
        }
        Check(remaining == 0 && PacketEnds <= 1 && Transfers - transfers <= 1 &&
            Ring.PresentedFrames == before + step.Frames,
            "elapsed callback copies only its bounded budget with at most one transfer");
        return step;
    }
};

template<size_t Size>
static bool AllZero(const std::array<unsigned char, Size>& bytes)
{
    for (const auto value : bytes) {
        if (value != 0) {
            return false;
        }
    }
    return true;
}

static void PlaybackCompletesOnlyAfterAllSegments()
{
    Fixture f(false);
    f.Segment();
    Check(f.Transfers == 0 && f.Scratch.front() == 7 && f.Scratch.back() == 0,
        "playback retains partial PCM in preallocated scratch, not a WDF request");
    f.Segment();
    Check(f.Transfers == 1, "only a complete logical PCM packet is published");
    for (auto sample : f.Transferred) {
        Check(sample == 7, "short final segment is included in publication");
    }
    for (unsigned i = 0; i < 196; ++i) {
        Check(f.Native[i] == 0, "consumed native ring bytes are retired");
    }
}

static void CaptureRetrievesOnceAndSilencesAfterRevocation()
{
    Fixture f(true);
    f.Segment();
    Check(f.Transfers == 1 && f.Native[0] == 5 && f.Native[191] == 5,
        "capture copies one queued WRITE into scratch in the first callback");
    vad::CloseGate(f.Slot);
    f.Slot.GateOpen = 1;
    f.Segment();
    Check(f.Transfers == 1 && f.Native[192] == 0 && f.Native[195] == 0,
        "close/reopen epoch cannot leak the remaining old capture scratch");
    f.Segment();
    Check(f.Transfers == 2 && f.Native[196] == 5,
        "a new logical packet can start in the fresh epoch");
}

static void PlaybackNeverMixesGateEpochs()
{
    Fixture f(false);
    f.Segment();
    vad::InvalidatePcmEpoch(f.Slot);
    f.Segment();
    Check(f.Transfers == 0, "partial playback crossing a reset is discarded");
    f.Segment();
    f.Segment();
    Check(f.Transfers == 1, "fresh playback resumes only at the next packet boundary");
    for (auto sample : f.Transferred) {
        Check(sample == 7, "fresh packet contains no retired partial bytes");
    }
}

static void MidPacketOpenDoesNotAdmitPartialAudio()
{
    for (bool capture : {false, true}) {
        Fixture f(capture);
        f.Slot.GateOpen = 0;
        f.Segment();
        f.Slot.GateOpen = 1;
        f.Segment();
        Check(f.Transfers == 0, "opening a gate midway cannot admit a partial packet");
        f.Segment();
        f.Segment();
        Check(f.Transfers == 1, "next complete packet is admitted");
    }
}

static void StreamAndSessionReplacementFenceScratch()
{
    for (bool capture : {false, true}) {
        for (unsigned replacement = 0; replacement < 5; ++replacement) {
            Fixture f(capture);
            f.Segment();
            if (replacement == 0) { ++f.Slot.Session; }
            if (replacement == 1) { ++f.Slot.StreamGeneration; }
            if (replacement == 2) { f.Slot.State = MOIREN_VAD_STATE_REMOVING; }
            if (replacement == 3) { f.Slot.StreamState = MOIREN_VAD_STREAM_PAUSED; }
            if (replacement == 4) { f.Slot.AudioReady = 0; }
            f.Segment();
            Check(f.Transfers == (capture ? 1u : 0u) && f.Native[192] == 0,
                "replaced or paused resource cannot finish an old partial packet");
            f.Slot.Session = 9;
            f.Slot.StreamGeneration = 3;
            f.Slot.State = MOIREN_VAD_STATE_PRESENT;
            f.Slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
            f.Slot.AudioReady = 1;
            f.Segment();
            f.Segment();
            Check(f.Transfers == (capture ? 2u : 1u),
                "restored resource resumes only on a fresh complete packet");
        }
    }
}

static void EmptyCaptureQueueNeverReplaysPreviousScratch()
{
    Fixture f(true);
    f.Segment();
    f.Segment();
    f.PayloadAvailable = false;
    f.Segment();
    f.Segment();
    Check(f.Transfers == 2, "empty queue is attempted once per logical packet");
    for (unsigned i = 196; i < 392; ++i) {
        Check(f.Native[i] == 0, "empty or already-canceled capture queue produces silence");
    }
    // This is a byte-contract fake, not proof of WDF cancellation/completion.
}

static void CopyWrapPreservesChannelOrder()
{
    for (bool capture : {false, true}) {
        Fixture f(capture);
        f.Ring.RingFrameOffset = f.Ring.MappedFrames - 2;
        f.Segment();
        f.Segment();
        Check(f.Transfers == 1 && f.Ring.RingFrameOffset == 47,
            "wrapped logical PCM completes exactly once without consuming padding");
        if (capture) {
            Check(f.Native[4088] == 5 && f.Native[4095] == 5 &&
                f.Native[0] == 5 && f.Native[187] == 5 && f.Native[188] == 7,
                "capture wrap copies the tail then beginning, not extra ring bytes");
        } else {
            for (auto sample : f.Transferred) {
                Check(sample == 7, "playback wrap publishes both linear spans");
            }
        }
    }
}

static void TimerDiscontinuityDiscardsPartialBytes()
{
    for (bool capture : {false, true}) {
        Fixture f(capture);
        f.Segment();
        vad::InvalidatePcmEpoch(f.Slot);
        f.Packet = {}; // Production clears scratch after draining/stalling.
        f.Scratch.fill(0);
        f.Segment();
        Check(f.Transfers == (capture ? 1u : 0u),
            "reset does not reissue or finish a previously retired request");
        if (capture) {
            Check(f.Native[192] == 0, "remaining old capture packet becomes silence");
        }
    }
}

static void FormatsAndNativeWrapsPreserveEveryByte()
{
    for (unsigned align : {2u, 4u, 8u}) {
        for (unsigned frames : {48u, 49u, 480u, 1250u, 4800u}) {
            for (bool capture : {false, true}) {
                Fixture f(capture);
                Check(vad::audio::InitializeTimerDrivenRing(f.Ring, frames * align,
                    align, 4096), "format/packet matrix initializes");
                f.Slot.PacketFrames = frames;
                f.Slot.Channels = align == 2 ? 1 : 2;
                f.Slot.SampleFormat = align == 8 ? MOIREN_VAD_FORMAT_FLOAT32 :
                    MOIREN_VAD_FORMAT_PCM16;
                std::vector<unsigned char> native(f.Ring.MappedBytes);
                std::vector<unsigned char> scratch(f.Ring.PacketSize);
                unsigned transfers = 0;
                uint64_t expectedSequence = 0;
                uint64_t nativeWraps = 0;
                auto pattern = [](uint64_t offset) {
                    return static_cast<unsigned char>((offset * 17 + offset / 7) % 251 + 1);
                };
                while (transfers < 12) {
                    const auto before = f.Ring.PresentedFrames;
                    vad::audio::TimerDrivenSegment segment = {};
                    Check(vad::audio::NextTimerDrivenSegment(f.Ring, segment),
                        "matrix advances one bounded segment");
                    nativeWraps += f.Ring.PresentedFrames / f.Ring.MappedFrames -
                        before / f.Ring.MappedFrames;
                    if (!capture) {
                        for (unsigned i = 0; i < segment.Frames * align; ++i) {
                            native[(segment.RingFrameOffset * align + i) % native.size()] =
                                pattern(segment.StartFrame * align + i);
                        }
                    }
                    Check(vad::audio::TransferTimerSegment(f.Slot, 5, 3, capture,
                        f.Ring, segment, f.Packet,
                        [&] { std::memset(scratch.data(), 0, scratch.size()); },
                        [&](const vad::audio::TimerDrivenCopySpan& span) {
                            auto ring = native.data() + span.RingByteOffset;
                            auto packet = scratch.data() + span.PacketByteOffset;
                            std::memcpy(capture ? ring : packet,
                                capture ? packet : ring, span.ByteCount);
                        },
                        [&](const vad::audio::TimerDrivenCopySpan& span) {
                            std::memset(native.data() + span.RingByteOffset, 0, span.ByteCount);
                        },
                        [&] {
                            Check(segment.PacketSequence == expectedSequence++,
                                "one transfer per logical packet with monotonic sequence");
                            const auto start = segment.PacketSequence * frames * align;
                            for (unsigned i = 0; i < scratch.size(); ++i) {
                                if (capture) {
                                    scratch[i] = pattern(start + i);
                                } else {
                                    Check(scratch[i] == pattern(start + i),
                                        "playback preserves channel/sample bytes across both wraps");
                                }
                            }
                            ++transfers;
                        }), "production segment helper accepts matrix spans");
                    for (unsigned i = 0; i < segment.Frames * align; ++i) {
                        const auto value = native[(segment.RingFrameOffset * align + i) % native.size()];
                        Check(value == (capture ? pattern(segment.StartFrame * align + i) : 0),
                            "capture preserves sample order; playback retires consumed bytes");
                    }
                }
                Check(nativeWraps == f.Ring.PresentedFrames / f.Ring.MappedFrames,
                    "native packet count follows physical wraps, not logical PCM packets");
            }
        }
    }
}


static void OneMillisecondQuantumDoesNotScheduleShortPacketTails()
{
    vad::audio::TimerDrivenRing ring = {};
    Check(vad::audio::InitializeTimerDrivenRing(ring, 196, 4, 4096),
        "49-frame packet sets up short-tail timer regression");
    vad::audio::Clock clock = {};
    clock.Frequency = 10000000;
    clock.EpochQpc = 10000000;
    clock.Running = true;
    for (unsigned tick = 0; tick < 200; ++tick) {
        unsigned remaining = 48;
        unsigned segments = 0;
        unsigned logicalCompletions = 0;
        while (remaining != 0) {
            vad::audio::TimerDrivenSegment segment = {};
            Check(vad::audio::NextTimerDrivenSegment(ring, segment, remaining),
                "quantum consumes the remaining budget, not another full segment");
            remaining -= segment.Frames;
            ++segments;
            logicalCompletions += segment.CompletesPacket ? 1 : 0;
        }
        Check(segments <= 3 && logicalCompletions <= 1 &&
            ring.PresentedFrames == static_cast<uint64_t>(tick + 1) * 48,
            "quantum is bounded by 48 frames, three spans and one PCM packet");
        const auto schedule = vad::audio::ScheduleTimerDrivenQuantum(clock, ring,
            10000000 + static_cast<uint64_t>(tick + 1) * 10000);
        Check(schedule.Valid && !schedule.Rebased && schedule.Due == -10000,
            "49-frame packet tails never ask Windows for a 20us callback");
    }
    const auto late = vad::audio::ScheduleTimerDrivenQuantum(clock, ring, 900000000);
    Check(late.Valid && late.Rebased && late.Due == -10000,
        "a genuine stall rebases one fixed quantum rather than catching up");
}

template<unsigned Frames>
static void ShortPacketElapsedJitterProgress()
{
    for (bool capture : {false, true}) {
        PacketFixture<Frames> f(capture);
        vad::audio::Clock clock = {10000000, 10000000, 0, Frames, true};
        unsigned completions = 0;
        unsigned totalTransfers = 0;
        uint64_t expectedElapsed = 0;
        for (unsigned tick = 1; tick <= 3000; ++tick) {
            const auto wake = 10000000 + static_cast<uint64_t>(tick) * 11000;
            auto now = wake;
            unsigned callbacks = 0;
            do {
                const auto before = f.Ring.PresentedFrames;
                const auto transfers = f.Transfers;
                const auto step = vad::audio::PlanTimerDrivenStep(clock, f.Ring, now);
                Check(step.Valid && !step.Rebased && step.Frames != 0 &&
                    step.Frames <= Frames && step.Due < 0,
                    "1.1ms short-packet callback and timely debt service progress without rebasing");
                unsigned remaining = step.Frames;
                unsigned spans = 0;
                while (remaining != 0 && spans < 3) {
                    vad::audio::TimerDrivenSegment segment = {};
                    Check(vad::audio::NextTimerDrivenSegment(f.Ring, segment, remaining),
                        "short callback consumes bounded elapsed frames across packet tails");
                    const auto pattern = [](uint64_t frame, unsigned byte) {
                        return static_cast<unsigned char>((frame * 17 + byte) % 251 + 1);
                    };
                    if (!capture) {
                        for (unsigned frame = 0; frame < segment.Frames; ++frame) {
                            for (unsigned byte = 0; byte < 4; ++byte) {
                                f.Native[((segment.RingFrameOffset + frame) * 4 + byte) % f.Native.size()] =
                                    pattern(segment.StartFrame + frame, byte);
                            }
                        }
                    }
                    Check(vad::audio::TransferTimerSegment(f.Slot, 5, 3, capture,
                        f.Ring, segment, f.Packet, [&] { f.Scratch.fill(0); },
                        [&](const vad::audio::TimerDrivenCopySpan& span) {
                            auto ring = f.Native.data() + span.RingByteOffset;
                            auto packet = f.Scratch.data() + span.PacketByteOffset;
                            std::memcpy(capture ? ring : packet, capture ? packet : ring, span.ByteCount);
                        },
                        [&](const vad::audio::TimerDrivenCopySpan& span) {
                            std::memset(f.Native.data() + span.RingByteOffset, 0, span.ByteCount);
                        },
                        [&] {
                            ++f.Transfers;
                            for (unsigned frame = 0; frame < Frames; ++frame) {
                                for (unsigned byte = 0; byte < 4; ++byte) {
                                    const auto value = pattern(segment.PacketSequence * Frames + frame, byte);
                                    if (capture) { f.Scratch[frame * 4 + byte] = value; }
                                    else { Check(f.Scratch[frame * 4 + byte] == value,
                                        "jittered playback retains complete patterned packets in order"); }
                                }
                            }
                        }), "short jitter segment uses production PCM epoch/copy logic");
                    for (unsigned frame = 0; frame < segment.Frames; ++frame) {
                        for (unsigned byte = 0; byte < 4; ++byte) {
                            Check(f.Native[((segment.RingFrameOffset + frame) * 4 + byte) % f.Native.size()] ==
                                (capture ? pattern(segment.StartFrame + frame, byte) : 0),
                                "jittered capture preserves every byte; playback clears retired bytes");
                        }
                    }
                    remaining -= segment.Frames;
                    completions += segment.CompletesPacket ? 1u : 0u;
                    ++spans;
                }
                expectedElapsed = vad::audio::TicksToFrames(now - 10000000, 10000000);
                Check(remaining == 0 && f.Transfers - transfers <= 1 &&
                    f.Ring.PresentedFrames == before + step.Frames &&
                    f.Ring.PresentedFrames <= expectedElapsed &&
                    clock.BaseFrames == 0 && clock.EpochQpc == 10000000 &&
                    f.Slot.AdmissionGeneration == 11,
                    "short jitter preserves elapsed time and epoch within one transfer/three spans");
                totalTransfers += f.Transfers - transfers;
                ++callbacks;
                if (f.Ring.PresentedFrames < expectedElapsed) {
                    Check(step.Due == -1 && callbacks == 1,
                        "bounded residual debt requests a minimal due time, not another full quantum");
                    now += 100; // Timely service is an explicit fixture, not an OS guarantee.
                }
            } while (f.Ring.PresentedFrames < expectedElapsed);
            Check(callbacks <= 2 && f.Ring.PresentedFrames == expectedElapsed,
                "one timely extra callback drains this bounded jitter fixture");
            const auto duplicate = vad::audio::PlanTimerDrivenStep(clock, f.Ring, now);
            Check(duplicate.Valid && !duplicate.Rebased && duplicate.Frames == 0,
                "duplicate QPC never replays short packets");
        }
        Check(completions == f.Ring.PresentedFrames / Frames &&
            totalTransfers == completions + (capture && f.Ring.PresentedFrames % Frames != 0 ? 1u : 0u),
            "all complete short packets transfer exactly once");
    }
}

template<unsigned Frames>
static void ShortPacketPersistentOverloadIsCountedNotStarved()
{
    vad::audio::TimerDrivenRing ring = {};
    Check(vad::audio::InitializeTimerDrivenRing(ring, Frames * 4, 4, 4096), "overload ring");
    vad::audio::Clock clock = {10000000, 10000000, 0, Frames, true};
    unsigned rebases = 0;
    unsigned progressing = 0;
    for (unsigned tick = 1; tick <= 3000; ++tick) {
        const auto step = vad::audio::PlanTimerDrivenStep(clock, ring,
            10000000 + static_cast<uint64_t>(tick) * 11000);
        Check(step.Valid && step.Frames <= Frames && step.Due < 0,
            "persistent overload retains one-packet callback bound");
        if (step.Rebased) {
            ++rebases;
            Check(step.Frames == 0, "overload drops time association rather than replaying old PCM");
        } else {
            unsigned remaining = step.Frames;
            for (unsigned spans = 0; remaining != 0 && spans < 3; ++spans) {
                vad::audio::TimerDrivenSegment segment = {};
                Check(vad::audio::NextTimerDrivenSegment(ring, segment, remaining), "overload segment");
                remaining -= segment.Frames;
            }
            Check(remaining == 0 && step.Frames != 0, "nonstalled overloaded callback still progresses");
            ++progressing;
        }
    }
    Check(rebases > 0 && progressing > 2000 && ring.PresentedFrames > 100000,
        "fixed slow callbacks produce counted overload and real progress, never permanent zero starvation");
}

static void TimerElapsedDebtThresholdIsExplicit()
{
    for (const auto frames : {48u, 49u, 95u, 96u, 480u, 512u, 1250u, 4800u}) {
        for (const auto align : {2u, 4u, 8u}) {
            vad::audio::TimerDrivenRing ring = {};
            Check(vad::audio::InitializeTimerDrivenRing(ring, frames * align, align, 4096),
                "debt threshold format");
            const auto threshold = frames < 96 ? frames + 48 : frames;
            vad::audio::Clock clock = {10000000, 10000000, 0, frames, true};
            const auto at = vad::audio::FrameQpc(clock, threshold);
            const auto admitted = vad::audio::PlanTimerDrivenStep(clock, ring, at);
            Check(admitted.Valid && !admitted.Rebased && admitted.Frames == frames &&
                (frames < 96 ? admitted.Due == -1 : admitted.Due < -1) &&
                clock.EpochQpc == 10000000,
                "only sub-two-quantum packets receive bounded residual debt; legacy threshold stays inclusive");
            auto remaining = admitted.Frames;
            while (remaining != 0) {
                vad::audio::TimerDrivenSegment segment = {};
                Check(vad::audio::NextTimerDrivenSegment(ring, segment, remaining), "threshold advance");
                remaining -= segment.Frames;
            }
            const auto residual = vad::audio::PlanTimerDrivenStep(clock, ring, at);
            Check(residual.Valid && !residual.Rebased && residual.Frames == (frames < 96 ? 48u : 0u),
                "same-QPC debt callback consumes only admitted elapsed remainder");

            ring = {};
            Check(vad::audio::InitializeTimerDrivenRing(ring, frames * align, align, 4096), "stall ring");
            clock = {10000000, 10000000, 0, frames, true};
            const auto tooLate = vad::audio::FrameQpc(clock, threshold + 1);
            const auto stalled = vad::audio::PlanTimerDrivenStep(clock, ring, tooLate);
            Check(stalled.Valid && stalled.Rebased && stalled.Frames == 0 &&
                stalled.Due == -10000 && ring.PresentedFrames == 0 &&
                clock.BaseFrames == 0 && clock.EpochQpc == tooLate,
                "one frame over bounded debt discards without old-frame catch-up");
            const auto duplicate = vad::audio::PlanTimerDrivenStep(clock, ring, tooLate);
            Check(duplicate.Valid && !duplicate.Rebased && duplicate.Frames == 0,
                "same-QPC stalled retry cannot repeatedly invalidate the epoch");
        }
    }

    vad::audio::TimerDrivenRing narrow = {};
    Check(vad::audio::InitializeTimerDrivenRing(narrow, 48 * 8, 8, 128), "narrow mapped ring");
    Check(narrow.MappedFrames == 48, "narrow fixture has no unused native history");
    vad::audio::Clock clock = {10000000, 10000000, 0, 48, true};
    const auto step = vad::audio::PlanTimerDrivenStep(clock, narrow,
        vad::audio::FrameQpc(clock, 49));
    Check(step.Valid && step.Rebased && step.Frames == 0,
        "extra pending allowance can never exceed available native mapping history");
}


static void TimerJitterPreservesOneBoundedPacket()
{
    for (bool capture : {false, true}) {
        ElapsedFixture f(capture);
        f.LoadPayload(19);
        const auto native = f.Native;
        const auto epoch = f.Slot.AdmissionGeneration;
        const auto first = f.Tick(ElapsedFixture::Epoch + 50000);
        Check(!first.Rebased && first.Frames == 240 && first.Due == -10000 &&
            f.Ring.PresentedFrames == 240 && f.Packet.Active && f.PacketEnds == 0 &&
            f.Transfers == (capture ? 1u : 0u),
            "first five milliseconds retain a partial packet without playback publication");
        for (unsigned i = 0; i < f.Scratch.size(); ++i) {
            Check(f.Scratch[i] == (capture || i < 960 ? f.Payload[i] : 0),
                "partial scratch contains exactly the copied bytes or one capture payload");
        }
        for (unsigned i = 0; i < f.Native.size(); ++i) {
            const auto expected = i < 960 ? (capture ? f.Payload[i] : 0) : native[i];
            Check(f.Native[i] == expected, "first callback leaves all future ring bytes untouched");
        }
        const auto second = f.Tick(ElapsedFixture::Epoch + 100000);
        Check(!second.Rebased && second.Frames == 240 && f.PacketEnds == 1 &&
            f.Ring.PresentedFrames == 480 && f.Transfers == 1 &&
            f.Slot.AdmissionGeneration == epoch && f.Clock.EpochQpc == ElapsedFixture::Epoch &&
            f.Scratch == f.Payload,
            "two five millisecond callbacks finish one intact 480-frame payload in one epoch");
        if (!capture) {
            Check(f.Transferred == f.Payload, "playback publishes all 1920 patterned bytes exactly once");
        }
        for (unsigned i = 0; i < f.Native.size(); ++i) {
            const auto expected = i < f.Payload.size() ?
                (capture ? f.Payload[i] : 0) : native[i];
            Check(f.Native[i] == expected, "complete payload preserves capture bytes and padding guards");
        }
        const auto duplicate = f.Tick(ElapsedFixture::Epoch + 100000);
        Check(!duplicate.Rebased && duplicate.Frames == 0 && f.Spans == 0 &&
            f.Transfers == 1 && f.Ring.PresentedFrames == 480,
            "same-QPC callback cannot repeat the payload transfer or advance position");
    }
}

static void ElapsedHalfPacketRejectsGateAndStreamReplacement()
{
    for (bool capture : {false, true}) {
        for (bool replaceStream : {false, true}) {
            ElapsedFixture f(capture);
            f.LoadPayload(31);
            f.Tick(ElapsedFixture::Epoch + 50000);
            Check(!AllZero(f.Scratch), "replacement regression starts with real partial PCM");
            const auto transfers = f.Transfers;
            if (replaceStream) {
                ++f.Slot.StreamGeneration;
            } else {
                vad::CloseGate(f.Slot);
                f.Slot.GateOpen = 1;
            }
            const auto second = f.Tick(ElapsedFixture::Epoch + 100000);
            Check(!second.Rebased && second.Frames == 240 && f.PacketEnds == 1 &&
                !f.Packet.Active && AllZero(f.Scratch) && f.Transfers == transfers,
                "mid-packet gate epoch or stream replacement clears scratch without an old transfer");
            for (unsigned i = 960; i < 1920; ++i) {
                Check(f.Native[i] == 0, "retired packet tail never reaches either native direction");
            }
            if (!capture) {
                Check(AllZero(f.Transferred), "discarded playback cannot publish a mixed-epoch payload");
            }
        }
    }
}

static void ElapsedLongStallDiscardsWithoutCompletionOrCatchUp()
{
    for (bool capture : {false, true}) {
        ElapsedFixture f(capture);
        f.LoadPayload(43);
        f.Tick(ElapsedFixture::Epoch + 50000);
        const auto transfers = f.Transfers;
        const auto sequence = f.Ring.SegmentSequence;
        const auto epoch = f.Slot.AdmissionGeneration;
        const auto stalledAt = ElapsedFixture::Epoch + 50000000;
        const auto stalled = f.Tick(stalledAt);
        Check(stalled.Rebased && stalled.Frames == 0 && stalled.Due == -10000 &&
            f.Ring.PresentedFrames == 240 && f.Ring.RingFrameOffset == 240 &&
            f.Ring.SegmentSequence == sequence && f.Spans == 0 && f.PacketEnds == 0 &&
            f.Transfers == transfers && !f.Packet.Active && AllZero(f.Scratch) &&
            AllZero(f.Native) && f.Slot.AdmissionGeneration == epoch + 1 &&
            f.Clock.BaseFrames == 240 && f.Clock.EpochQpc == stalledAt,
            "long stall invalidates partial bytes once without position jumps or packet transfers");
        const auto duplicate = f.Tick(stalledAt);
        Check(!duplicate.Rebased && duplicate.Frames == 0 && f.Transfers == transfers &&
            f.Slot.AdmissionGeneration == epoch + 1 && f.Spans == 0,
            "same-QPC retry cannot drain old periods or repeatedly invalidate the epoch");
        const auto tail = f.Tick(stalledAt + 50000);
        Check(!tail.Rebased && tail.Frames == 240 && f.Ring.PresentedFrames == 480 &&
            f.PacketEnds == 1 && f.Transfers == transfers && AllZero(f.Scratch) &&
            AllZero(f.Native), "first fresh interval silently retires the discarded old packet tail");
        f.LoadPayload(97);
        const auto freshFirst = f.Tick(stalledAt + 100000);
        const auto freshSecond = f.Tick(stalledAt + 150000);
        Check(!freshFirst.Rebased && !freshSecond.Rebased && freshFirst.Frames == 240 &&
            freshSecond.Frames == 240 && f.Ring.PresentedFrames == 960 &&
            f.Transfers == transfers + 1 && f.Scratch == f.Payload &&
            f.Slot.AdmissionGeneration == epoch + 1,
            "only a complete post-stall packet resumes transport on the fresh timeline");
        if (!capture) {
            Check(f.Transferred == f.Payload, "post-stall playback publishes only the new payload");
        }
        for (unsigned i = 0; i < f.Native.size(); ++i) {
            const auto expected = capture && i >= 1920 && i < 3840 ?
                f.Payload[i - 1920] : 0;
            Check(f.Native[i] == expected, "old ring PCM stays zero outside the fresh capture interval");
        }
    }
}

static void TimerElapsedBudgetsRespectAllBoundaries()
{
    const uint32_t sizes[] = {48u, 49u, 480u, 1250u, 4800u};
    for (const auto frames : sizes) {
        for (const auto align : {2u, 4u, 8u}) {
            vad::audio::TimerDrivenRing ring = {};
            Check(vad::audio::InitializeTimerDrivenRing(ring, frames * align,
                align, 4096), "elapsed boundary shape is supported");
            for (unsigned pass = 0; pass < 40; ++pass) {
                unsigned remaining = frames;
                unsigned spans = 0;
                unsigned completions = 0;
                while (remaining != 0 && spans < 3) {
                    vad::audio::TimerDrivenSegment segment = {};
                    Check(vad::audio::NextTimerDrivenSegment(ring, segment, remaining),
                        "packet budget advances even at a logical/native wrap");
                    vad::audio::TimerDrivenCopyPlan plan = {};
                    Check(vad::audio::BuildTimerDrivenCopyPlan(ring, segment, plan),
                        "elapsed copy stays inside its MDL and scratch packet");
                    remaining -= segment.Frames;
                    completions += segment.CompletesPacket ? 1 : 0;
                    ++spans;
                }
                Check(remaining == 0 && completions == 1 && spans <= 3,
                    "maximum packet budget has at most three spans and one completion");
                vad::audio::TimerDrivenSegment prefix = {};
                Check(vad::audio::NextTimerDrivenSegment(ring, prefix, 1),
                    "shift the next budget across a different native boundary");
            }
            vad::audio::Clock clock = {};
            clock.Frequency = 10000000;
            clock.Running = true;
            clock.BaseFrames = ring.PresentedFrames;
            clock.EpochQpc = 10000000;
            const auto early = vad::audio::PlanTimerDrivenStep(clock, ring, 9999999);
            Check(early.Valid && !early.Rebased && early.Frames == 0 && early.Due < 0,
                "early callback cannot publish any unelapsed native frame");
            ring.PresentedFrames = ~uint64_t{0} - 1;
            Check(!vad::audio::PlanTimerDrivenStep(clock, ring, 10000000).Valid,
                "position exhaustion rejects before scheduling arithmetic wraps");
        }
    }
}

int main()
{
    ShortPacketElapsedJitterProgress<48>();
    ShortPacketElapsedJitterProgress<49>();
    ShortPacketPersistentOverloadIsCountedNotStarved<48>();
    ShortPacketPersistentOverloadIsCountedNotStarved<49>();
    TimerElapsedDebtThresholdIsExplicit();
    TimerJitterPreservesOneBoundedPacket();
    ElapsedHalfPacketRejectsGateAndStreamReplacement();
    ElapsedLongStallDiscardsWithoutCompletionOrCatchUp();
    TimerElapsedBudgetsRespectAllBoundaries();
    Check(!vad::audio::SupportsPacketNotifications(1) &&
        vad::audio::SupportsPacketNotifications(2) &&
        !vad::audio::SupportsPacketNotifications(0),
        "single-ring timer streams expose position, never overwritten whole packets");
    PlaybackCompletesOnlyAfterAllSegments();
    CaptureRetrievesOnceAndSilencesAfterRevocation();
    PlaybackNeverMixesGateEpochs();
    MidPacketOpenDoesNotAdmitPartialAudio();
    StreamAndSessionReplacementFenceScratch();
    EmptyCaptureQueueNeverReplaysPreviousScratch();
    CopyWrapPreservesChannelOrder();
    TimerDiscontinuityDiscardsPartialBytes();
    FormatsAndNativeWrapsPreserveEveryByte();
    OneMillisecondQuantumDoesNotScheduleShortPacketTails();
    std::puts("PASS: segmented PCM, elapsed payload/gate/stream/stall regressions, "
        "epoch/session/stream fencing, mid-packet admission, capture silence");
}
