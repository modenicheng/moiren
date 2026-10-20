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

struct Fixture {
    vad::Slot Slot = {};
    vad::audio::TimerDrivenRing Ring = {};
    vad::audio::TimerPcmPacket Packet = {};
    std::array<unsigned char, 4096> Native = {};
    std::array<unsigned char, 196> Scratch = {};
    std::array<unsigned char, 196> Transferred = {};
    unsigned Transfers = 0;
    bool PayloadAvailable = true;
    bool Capture;

    explicit Fixture(bool capture) : Capture(capture)
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
        Slot.PacketFrames = 49;
        Check(vad::audio::InitializeTimerDrivenRing(Ring, 196, 4, 4096),
            "fixture uses a logical packet shorter than its page-backed ring");
        Native.fill(7);
    }

    void Segment()
    {
        vad::audio::TimerDrivenSegment segment = {};
        Check(vad::audio::NextTimerDrivenSegment(Ring, segment), "segment advances");
        vad::audio::TransferTimerSegment(Slot, 5, 3, Capture, Ring, segment, Packet,
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
                        Scratch.fill(5);
                    }
                } else {
                    Transferred = Scratch;
                }
            });
    }
};

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


static void TimerJitterPreservesOneBoundedPacket()
{
    Fixture f(false);
    Check(vad::audio::InitializeTimerDrivenRing(f.Ring, 1920, 4, 4096),
        "jitter regression uses a ten millisecond logical packet");
    vad::audio::Clock clock = {};
    clock.Frequency = 10000000;
    clock.EpochQpc = 10000000;
    clock.Running = true;
    const auto step = vad::audio::PlanTimerDrivenStep(clock, f.Ring, 10050000);
    Check(step.Valid && !step.Rebased && step.Frames == 240 && step.Due < 0,
        "five millisecond scheduling jitter processes elapsed frames without reset");
    unsigned remaining = step.Frames;
    unsigned spans = 0;
    unsigned completed = 0;
    while (remaining != 0 && spans < 3) {
        vad::audio::TimerDrivenSegment segment = {};
        Check(vad::audio::NextTimerDrivenSegment(f.Ring, segment, remaining),
            "elapsed work uses a bounded logical/native span");
        vad::audio::TimerDrivenCopyPlan plan = {};
        Check(vad::audio::BuildTimerDrivenCopyPlan(f.Ring, segment, plan),
            "bounded elapsed span has a valid copy plan");
        remaining -= segment.Frames;
        completed += segment.CompletesPacket ? 1 : 0;
        ++spans;
    }
    Check(remaining == 0 && spans <= 3 && completed <= 1 &&
        f.Ring.PresentedFrames == 240, "one callback never loops over old periods");
    const auto next = vad::audio::PlanTimerDrivenStep(clock, f.Ring, 10100000);
    Check(next.Valid && !next.Rebased && next.Frames == 240,
        "a second late callback can complete the same logical packet");
    const auto stalled = vad::audio::PlanTimerDrivenStep(clock, f.Ring, 900000000);
    Check(stalled.Valid && stalled.Rebased && stalled.Frames == 0 &&
        stalled.Due == -10000 && f.Ring.PresentedFrames == 240,
        "a long stall discards old time without rendering a catch-up packet");
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
    TimerJitterPreservesOneBoundedPacket();
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
    std::puts("PASS: segmented PCM, epoch/session/stream fencing, mid-packet admission, capture silence");
}
