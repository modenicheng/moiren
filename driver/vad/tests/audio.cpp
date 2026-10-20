// SPDX-License-Identifier: MIT
#include "../src/audio/Clock.h"
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
        {0, 1, 1920, 4, false, false, 0, 0, 0, R::Accepted},
        {0, 1, 1920, 4, false, false, 1, 0, 0, R::Overrun},
        {0, 1, 1920, 4, true, false, 0, 0, 0, R::Late},
        {0, 1, 1920, 4, true, false, 1, 0, 0, R::Overrun},
    };
    for (const auto& c : cases) {
        Check(vad::audio::ValidateRenderPacket(c.Completed, c.Count, c.Size,
            c.Align, c.Started, c.Ended, c.Packet, c.Flags, c.Length) == c.Expected,
            "render packet preroll/running/wrap/flag/EOS/allocation contract");
    }
    Check(vad::audio::ValidPacketShape(2, 96, 2) &&
        vad::audio::ValidPacketShape(1, 38400, 8) &&
        !vad::audio::ValidPacketShape(2, 94, 2) &&
        !vad::audio::ValidPacketShape(2, 38408, 8),
        "native packet allocation enforces one to 100 ms and one/two packets");
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
    DeadlineAndStall();
    PauseAndResume();
    FractionalDeadlines();
    RenderPackets();
    CapturePackets();
    std::puts("PASS: audio stall rebase, pause/resume, 3.6M rational deadlines, "
        "render flags/EOS/wrap, capture readiness/duplicates");
}
