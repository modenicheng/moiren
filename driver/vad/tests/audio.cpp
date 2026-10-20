// SPDX-License-Identifier: MIT
#include "../src/audio/Clock.h"
#include "../src/Lifecycle.h"
#include <cstdio>
#include <cstdlib>

static void Check(bool condition, const char* message)
{
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        std::exit(1);
    }
}

static vad::audio::Clock RunningClock(uint64_t frequency = 10000000,
    uint32_t frames = 480)
{
    vad::audio::Clock clock = {};
    clock.Frequency = frequency;
    clock.EpochQpc = 10000000;
    clock.LimitFrames = frames;
    clock.Running = true;
    return clock;
}

struct PacketClockState {
    vad::audio::Clock Clock;
    uint64_t CompletedPackets = 7;
    uint64_t DeliveredPackets = 6;
    uint64_t LastCaptureStart = 10100000;
    uint32_t PacketFrames = 480;
    bool HasEos = true;
    bool Started = true;
};

static PacketClockState StoppedPacketClock()
{
    PacketClockState stream = {};
    stream.Clock = RunningClock();
    stream.Clock.BaseFrames = stream.CompletedPackets * stream.PacketFrames;
    stream.Clock.LimitFrames = stream.Clock.BaseFrames + stream.PacketFrames;
    stream.Clock.Running = false;
    return stream;
}

static vad::Slot RunningPcmSlot()
{
    vad::Slot slot = {};
    slot.Id = 5;
    slot.Session = 9;
    slot.State = MOIREN_VAD_STATE_PRESENT;
    slot.Intent = MOIREN_VAD_INTENT_MICROPHONE;
    slot.Channels = 2;
    slot.PolicyPrepared = 1;
    slot.AudioReady = 1;
    slot.GateOpen = 1;
    slot.AdmissionGeneration = 42;
    slot.StreamGeneration = 6;
    slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
    slot.SampleFormat = MOIREN_VAD_FORMAT_FLOAT32;
    slot.PacketFrames = 480;
    slot.LastSequence = 91;
    slot.HasSequence = 1;
    return slot;
}

static MOIREN_VAD_PCM_HEADER PcmStamp(const vad::Slot& slot)
{
    return {sizeof(MOIREN_VAD_PCM_HEADER), MOIREN_VAD_ABI_VERSION,
        slot.Id, slot.Session, slot.Id, slot.StreamGeneration,
        slot.AdmissionGeneration, 0, 10100000,
        MOIREN_VAD_FORMAT_FLOAT32, 480, slot.Channels, 0};
}

static void PacketClockResetFencesSameStreamBeforeCounterRestart()
{
    auto slot = RunningPcmSlot();
    const auto stale = PcmStamp(slot);
    Check(vad::SameStream(slot, stale) && !vad::ValidWriteSequence(slot, stale),
        "sequence zero cannot rewind an admitted epoch with existing writes");
    // Release, packet free and cleanup all use this production reset helper.
    // The observer samples only before reset and after the same-object RUN.
    for (unsigned reset = 0; reset < 3; ++reset) {
        auto stream = StoppedPacketClock();
        const auto previous = PcmStamp(slot);
        unsigned invalidations = 0;
        vad::audio::ResetPacketClock(stream, [&] {
            Check(stream.CompletedPackets == 7 && stream.Clock.BaseFrames == 3360,
                "PCM invalidation precedes resetting native packet numbering");
            vad::InvalidatePcmEpoch(slot, true);
            ++invalidations;
        });
        Check(invalidations == 1 && !vad::SameStream(slot, previous),
            "packet clock reset must invalidate PCM before restarting numbering");
        Check(slot.AdmissionGeneration == previous.GateEpoch + 1 &&
            slot.StreamGeneration == stale.StreamGeneration && slot.GateOpen == 1,
            "same-object reset advances the gate epoch without replacing the stream");
        Check(stream.CompletedPackets == 0 && stream.DeliveredPackets == 0 &&
            stream.LastCaptureStart == 0 && !stream.HasEos && !stream.Started &&
            stream.Clock.BaseFrames == 0 && stream.Clock.EpochQpc == 0 &&
            stream.Clock.LimitFrames == 480 && !stream.Clock.Running,
            "packet counts, timestamps, preroll and presentation restart together");
        const auto fresh = PcmStamp(slot);
        Check(slot.StreamState == MOIREN_VAD_STREAM_PAUSED &&
            !vad::SameStream(slot, fresh),
            "new-epoch PCM cannot enter between clock reset and the pending purge");
        slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
        Check(vad::SameStream(slot, fresh) && vad::ValidWriteSequence(slot, fresh),
            "sequence zero is admitted only with the new PCM epoch");
        Check(!vad::SameStream(slot, stale),
            "old PCM stamps stay invalid across repeated resets");
        slot.LastSequence = fresh.Sequence;
        slot.HasSequence = 1;
        Check(!vad::ValidWriteSequence(slot, fresh),
            "duplicate sequence zero is still rejected within the new epoch");
    }
}

static void PacketClockResetAdvancesClosedEpochAndFailsClosedOnOverflow()
{
    const uint64_t epochs[] = {42, ~uint64_t{0}};
    for (const auto epoch : epochs) {
        for (uint32_t open = 0; open <= 1; ++open) {
            auto slot = RunningPcmSlot();
            slot.GateOpen = open;
            slot.AdmissionGeneration = epoch;
            const auto stale = PcmStamp(slot);
            auto stream = StoppedPacketClock();
            vad::audio::ResetPacketClock(stream, [&] {
                vad::InvalidatePcmEpoch(slot, true);
            });
            Check(!slot.HasSequence && !vad::SameStream(slot, stale),
                "reset clears sequence history and rejects every old stamp");
            if (epoch == ~uint64_t{0}) {
                Check(slot.AdmissionGeneration == epoch && slot.GateOpen == 0 &&
                    !vad::SameStream(slot, PcmStamp(slot)),
                    "epoch exhaustion closes admission instead of wrapping");
            } else {
                Check(slot.AdmissionGeneration == epoch + 1 && slot.GateOpen == open,
                    "every reset advances the epoch even when the gate was closed");
            }
        }
    }
}

static void DeadlineAndStall()
{
    auto clock = RunningClock();
    Check(vad::audio::RelativeDue(clock, 480, 10100000) == 0,
        "expired deadline must be distinguishable from a future delay");
    Check(vad::audio::RelativeDue(clock, 480, 10000000) == -100000,
        "future deadline is a negative 100-ns relative interval");
    Check(vad::audio::Position(clock, 700000000) == 480,
        "stalled presentation stops at the packet boundary");
    const auto next = vad::audio::SchedulePacket(clock, 1, 480, 700000000);
    Check(next.Rebased && next.Due == -100000,
        "long stall rebases to one full future packet, no immediate catch-up");
    Check(next.CompletedStartQpc == 700000000,
        "a rebased capture completion cannot retain its pre-stall timestamp");
    Check(vad::audio::Position(clock, 700000000) == 480,
        "stall rebase preserves the position already visible to the OS");
    for (uint64_t completed = 2; completed < 100; ++completed) {
        const auto now = 700000000 + (completed - 1) * 100000;
        const auto schedule = vad::audio::SchedulePacket(clock, completed, 480, now);
        Check(!schedule.Rebased && schedule.Due == -100000,
            "stall recovery stays at one packet per nominal period");
        Check(schedule.CompletedStartQpc ==
            700000000 + (completed - 2) * 100000,
            "capture timestamps continue from the rebased packet clock");
    }
}

static void PauseAndResume()
{
    auto clock = RunningClock();
    Check(vad::audio::Position(clock, 10050000) == 240,
        "live position includes a partial packet");
    vad::audio::PauseClock(clock, 10050000);
    Check(vad::audio::Position(clock, 90000000) == 240,
        "pause snapshots and freezes the partial packet position");
    vad::audio::ResumeClock(clock, 90000000);
    const auto schedule = vad::audio::SchedulePacket(clock, 0, 480, 90000000);
    Check(!schedule.Rebased && schedule.Due == -50000,
        "resume schedules the remaining half packet");
    Check(vad::audio::Position(clock, 90000000) == 240,
        "resume does not round presentation back to the last completion");
    Check(vad::audio::Position(clock, 90025000) == 360,
        "presentation continues forward from the pause snapshot");
    vad::audio::PauseClock(clock, 90100000);
    Check(clock.BaseFrames == 480, "pause at a stalled boundary is bounded");
    vad::audio::ResumeClock(clock, 100000000);
    Check(vad::audio::SchedulePacket(clock, 0, 480, 100000000).Due == -1,
        "resume at the pending boundary permits a single immediate completion");
    Check(vad::audio::SchedulePacket(clock, 1, 480, 100000000).Due == -100000,
        "the completion after resume cannot start a catch-up burst");
    Check(vad::audio::Position(clock, 100000000) == 480,
        "pending boundary completion never regresses presentation");
}

static void FractionalDeadlines()
{
    // An independent integer-ratio oracle: the product fits in uint64_t here.
    // A rounded per-packet increment would drift on this frequency/shape.
    auto clock = RunningClock(10000003, 49);
    for (uint64_t completed = 1; completed <= 3600000; ++completed) {
        const auto boundary = (completed * 49 * 10000003 + 47999) / 48000;
        const auto qpc = 10000000 + boundary;
        const auto schedule = vad::audio::SchedulePacket(clock, completed, 49, qpc);
        Check(!schedule.Rebased, "ordinary fractional periods do not rebase");
        Check(vad::audio::FrameQpc(clock, (completed + 1) * 49) ==
            10000000 + ((completed + 1) * 49 * 10000003 + 47999) / 48000,
            "rational deadlines do not accumulate per-packet rounding drift");
        Check(schedule.Due < -10000,
            "a normal packet always schedules a future period");
    }
    Check(vad::audio::ValidFrequency(48000) &&
        vad::audio::ValidFrequency(100000000000ULL) &&
        !vad::audio::ValidFrequency(0) &&
        !vad::audio::ValidFrequency(100000000001ULL),
        "QPC frequency validation bounds the arithmetic");
    clock = RunningClock(100000000000ULL, 48);
    Check(vad::audio::RelativeDue(clock, 48, 10000000) == -10000,
        "maximum QPC frequency retains a one-ms interval");
}

static void NativeAllocationRejectsTimerDrivenPackets()
{
    const uint32_t aligns[] = {2, 4, 8};
    const uint32_t frames[] = {48, 480, 4800};
    for (const auto align : aligns) {
        for (const auto frameCount : frames) {
            const auto size = frameCount * align;
            Check(!vad::audio::ValidPacketShape(1, size, align),
                "single-packet timer-driven allocation must fail closed");
            Check(vad::audio::ValidPacketShape(2, size, align),
                "two-packet event-driven allocation remains supported");
        }
    }
    Check(!vad::audio::ValidPacketShape(1, 4096, 4),
        "page alignment does not admit an unsupported timer-driven stream");
    Check(vad::audio::ValidPacketShape(2, 4096, 4),
        "page-aligned two-packet streams remain supported");
}

static void RenderPackets()
{
    using R = vad::audio::RenderPacketResult;
    struct Case {
        uint64_t Completed;
        uint32_t Count;
        uint32_t Size;
        uint32_t Align;
        bool Started;
        bool Ended;
        uint32_t Packet;
        uint32_t Flags;
        uint32_t Length;
        R Expected;
    };
    const Case cases[] = {
        {0, 2, 1920, 4, false, false, 0, 0, 9999, R::Accepted},
        {0, 2, 1920, 4, false, false, 1, 0, 0, R::Accepted},
        {0, 2, 1920, 4, false, false, 2, 0, 0, R::Overrun},
        {0, 2, 1920, 4, false, false, 0xffffffffu, 0, 0, R::Late},
        {0, 2, 1920, 4, true, false, 0, 0, 0, R::Late},
        {0, 2, 1920, 4, true, false, 1, 0, 0, R::Accepted},
        {0, 2, 1920, 4, true, false, 2, 0, 0, R::Overrun},
        {0xffffffffULL, 2, 1920, 4, true, false, 0, 0, 0, R::Accepted},
        {0xffffffffULL, 2, 1920, 4, true, false, 0xffffffffu, 0, 0, R::Late},
        {0xffffffffULL, 2, 1920, 4, true, false, 1, 0, 0, R::Overrun},
        {0xffffffffULL, 2, 1920, 4, true, false, 0xfffffffeu, 0, 0, R::Late},
        {0x100000000ULL, 2, 1920, 4, true, false, 1, 0, 0, R::Accepted},
        {0x100000000ULL, 2, 1920, 4, true, false, 0, 0, 0, R::Late},
        {0, 2, 1920, 4, false, false, 0, 0x200, 0, R::Accepted},
        {0, 2, 1920, 4, false, false, 0, 0x200, 1920, R::Accepted},
        {0, 2, 1920, 4, false, false, 0, 0x200, 1924, R::Invalid},
        {0, 2, 1920, 4, false, false, 0, 0x200, 1919, R::Invalid},
        {0, 2, 1920, 4, false, false, 0, 1, 0, R::Invalid},
        {0, 2, 1920, 4, false, true, 1, 0, 0, R::Ended},
        {0, 0, 1920, 4, false, false, 0, 0, 0, R::Invalid},
        {0, 3, 1920, 4, false, false, 0, 0, 0, R::Invalid},
        {0, 2, 0, 4, false, false, 0, 0, 0, R::Invalid},
        {0, 2, 1919, 4, false, false, 0, 0, 0, R::Invalid},
        {0, 2, 1920, 0, false, false, 0, 0, 0, R::Invalid},
        {0, 1, 1920, 4, false, false, 0, 0, 0, R::Invalid},
        {0, 1, 1920, 4, false, false, 1, 0, 0, R::Invalid},
        {0, 1, 1920, 4, true, false, 0, 0, 0, R::Invalid},
        {0, 1, 1920, 4, true, false, 1, 0, 0, R::Invalid},
    };
    for (const auto& c : cases) {
        Check(vad::audio::ValidateRenderPacket(c.Completed, c.Count, c.Size,
            c.Align, c.Started, c.Ended, c.Packet, c.Flags, c.Length) == c.Expected,
            "render packet preroll/running/wrap/flag/EOS/allocation contract");
    }
    Check(vad::audio::ValidPacketShape(2, 96, 2) &&
        vad::audio::ValidPacketShape(2, 38400, 8) &&
        !vad::audio::ValidPacketShape(2, 94, 2) &&
        !vad::audio::ValidPacketShape(2, 38408, 8),
        "native packet allocation enforces one to 100 ms and two packets");
}

static void CapturePackets()
{
    uint64_t delivered = 0;
    uint32_t packet = 123;
    Check(!vad::audio::TakeCapturePacket(0, delivered, packet),
        "capture is not ready before the first completion");
    Check(vad::audio::TakeCapturePacket(1, delivered, packet) && packet == 0,
        "the first actual capture completion is packet zero");
    Check(!vad::audio::TakeCapturePacket(1, delivered, packet),
        "a duplicate capture query reports no new packet");
    Check(vad::audio::TakeCapturePacket(8, delivered, packet) && packet == 7,
        "slow capture reader receives latest completion without catch-up");
    Check(!vad::audio::TakeCapturePacket(8, delivered, packet),
        "slow capture does not return the latest packet twice");
    Check(vad::audio::TakeCapturePacket(0x100000000ULL, delivered, packet) &&
        packet == 0xffffffffu,
        "capture packet index reaches ULONG maximum");
    Check(vad::audio::TakeCapturePacket(0x100000001ULL, delivered, packet) &&
        packet == 0,
        "capture wraps packet index while duplicate detection remains 64-bit");
}

int main()
{
    PacketClockResetFencesSameStreamBeforeCounterRestart();
    PacketClockResetAdvancesClosedEpochAndFailsClosedOnOverflow();
    DeadlineAndStall();
    PauseAndResume();
    FractionalDeadlines();
    NativeAllocationRejectsTimerDrivenPackets();
    RenderPackets();
    CapturePackets();
    std::puts("PASS: packet reset epoch/sequence/overflow, audio stall rebase, "
        "pause/resume, 3.6M rational deadlines, "
        "two-packet admission, render flags/EOS/wrap, capture readiness/duplicates");
}
