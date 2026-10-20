// SPDX-License-Identifier: MIT
#pragma once

// Independent of WDF so stream clock/packet arithmetic is tested off-target.
#if !defined(MOIREN_VAD_KERNEL)
#include <stdint.h>
#endif

namespace vad::audio {
constexpr uint64_t Rate = 48000;
constexpr uint64_t HnsPerSecond = 10000000;
constexpr uint32_t MaxPackets = 2;
constexpr uint32_t MinPacketFrames = 48;
constexpr uint32_t MaxPacketFrames = 4800;
constexpr uint32_t EndOfStream = 0x200; // KSSTREAM_HEADER_OPTIONSF_ENDOFSTREAM.
constexpr uint32_t DefaultTimerSegmentFrames = 48; // 1 ms at 48 kHz.

struct Clock {
    uint64_t Frequency;
    uint64_t EpochQpc;
    uint64_t BaseFrames;
    // A delayed DPC cannot present samples beyond the packet it is processing.
    // This ceiling lets stall recovery rebase without regressing a position
    // already returned to the OS.
    uint64_t LimitFrames;
    bool Running;
};

inline bool ValidFrequency(uint64_t frequency)
{
    // Covers QPC frequencies through 100 GHz, bounds intermediate products.
    return frequency >= Rate && frequency <= 100000000000ULL;
}

inline uint64_t TicksToFrames(uint64_t ticks, uint64_t frequency)
{
    return (ticks / frequency) * Rate + ((ticks % frequency) * Rate) / frequency;
}

inline uint64_t FramesToTicks(uint64_t frames, uint64_t frequency)
{
    return (frames / Rate) * frequency + ((frames % Rate) * frequency) / Rate;
}

inline uint64_t Position(const Clock& clock, uint64_t now)
{
    if (!clock.Running || now <= clock.EpochQpc) {
        return clock.BaseFrames;
    }
    const auto frames = clock.BaseFrames +
        TicksToFrames(now - clock.EpochQpc, clock.Frequency);
    return frames < clock.LimitFrames ? frames : clock.LimitFrames;
}

inline uint64_t FrameQpc(const Clock& clock, uint64_t frame)
{
    if (frame <= clock.BaseFrames) {
        return clock.EpochQpc;
    }
    const auto frames = frame - clock.BaseFrames;
    // Round UP: a notification must not precede its last sample's boundary.
    const auto whole = FramesToTicks(frames, clock.Frequency);
    const auto fraction = ((frames % Rate) * clock.Frequency) % Rate;
    return clock.EpochQpc + whole + (fraction != 0 ? 1 : 0);
}

inline long long RelativeDue(const Clock& clock, uint64_t frame, uint64_t now)
{
    const auto deadline = FrameQpc(clock, frame);
    if (deadline <= now) {
        return 0; // Expired, not a negative WDF relative due time.
    }
    const auto ticks = deadline - now;
    const auto hns = (ticks / clock.Frequency) * HnsPerSecond +
        ((ticks % clock.Frequency) * HnsPerSecond + clock.Frequency - 1) /
            clock.Frequency;
    return -static_cast<long long>(hns != 0 ? hns : 1);
}

inline void PauseClock(Clock& clock, uint64_t now)
{
    clock.BaseFrames = Position(clock, now);
    clock.Running = false;
}

inline void ResumeClock(Clock& clock, uint64_t now)
{
    clock.EpochQpc = now;
    clock.Running = true;
}

template<typename Stream, typename InvalidatePcm>
inline void ResetPacketClock(Stream& stream, InvalidatePcm invalidatePcm)
{
    // Release/Prepare can reuse the same stream and skip Started's resume path.
    // Fence its old PCM before restarting numbering or exposing purge cancellation.
    invalidatePcm();
    stream.CompletedPackets = 0;
    stream.DeliveredPackets = 0;
    stream.LastCaptureStart = 0;
    stream.HasEos = false;
    stream.Started = false;
    stream.Clock.BaseFrames = 0;
    stream.Clock.EpochQpc = 0;
    stream.Clock.LimitFrames = stream.PacketFrames;
}

struct PacketSchedule {
    long long Due;
    bool Rebased;
    uint64_t CompletedStartQpc;
};

inline PacketSchedule SchedulePacket(Clock& clock, uint64_t completed,
    uint32_t packetFrames, uint64_t now)
{
    const auto boundary = completed * packetFrames;
    const auto next = boundary + packetFrames;
    const auto start = FrameQpc(clock,
        completed != 0 ? boundary - packetFrames : 0);
    auto due = RelativeDue(clock, next, now);
    const bool rebase = due == 0;
    if (rebase) {
        const auto position = Position(clock, now); // Uses the previous ceiling.
        clock.BaseFrames = position > boundary ? position : boundary;
        clock.EpochQpc = now;
        due = RelativeDue(clock, next, now);
    }
    clock.LimitFrames = next;
    // Pause may have frozen exactly at the pending packet boundary. Allow one
    // immediate completion; after it the next deadline is a full packet away.
    // Rebase also discarded the completed buffer's old contents. Timestamp
    // that replacement packet in the new epoch, never before the stall.
    return {due == 0 ? -1 : due, rebase, rebase ? now : start};
}

inline bool ValidPacketShape(uint32_t count, uint32_t size, uint32_t align)
{
    return (count == 1 || count == MaxPackets) && align != 0 && size != 0 &&
        size % align == 0 && size / align >= MinPacketFrames &&
        size / align <= MaxPacketFrames;
}

// ACX timer-driven streams use one logical packet backed by a page-aligned
// mapping. Keep the logical packet size separate from the page-rounded backing
// size: the former is the PCM contract, while the latter is the physical ring
// span used by offset calculations.
inline bool RoundUpToPage(uint32_t bytes, uint32_t pageSize,
    uint32_t& roundedBytes)
{
    roundedBytes = 0;
    if (bytes == 0 || pageSize == 0) {
        return false;
    }
    const auto rounded = (static_cast<uint64_t>(bytes) + pageSize - 1) /
        pageSize * pageSize;
    if (rounded > 0xffffffffULL) {
        return false;
    }
    roundedBytes = static_cast<uint32_t>(rounded);
    return true;
}

inline uint32_t RtPacketOffset(uint32_t count, uint32_t index,
    uint32_t packetSize, uint32_t allocationBytes)
{
    // ACX joins the two event MDL mappings into a contiguous user ring. End
    // packet zero at the page boundary so packet one immediately follows it.
    // A single timer mapping instead starts at its page-aligned circular origin.
    return count == MaxPackets && index == 0 && allocationBytes >= packetSize ?
        allocationBytes - packetSize : 0;
}

struct TimerDrivenRing {
    uint32_t PacketSize;
    uint32_t MappedBytes;
    uint32_t PacketFrames;
    uint32_t MappedFrames;
    uint32_t SegmentFrames;
    uint32_t BlockAlign;
    uint32_t RingFrameOffset;
    uint64_t PresentedFrames;
    uint64_t SegmentSequence;
};

struct TimerDrivenSegment {
    uint64_t Sequence;
    uint64_t StartFrame;
    uint64_t PacketSequence;
    uint32_t PacketOffsetFrames;
    uint32_t RingFrameOffset;
    uint32_t Frames;
    uint32_t FirstFrames;
    uint32_t SecondFrames;
    bool CompletesPacket;
};

struct TimerDrivenCopySpan {
    uint32_t RingByteOffset;
    uint32_t PacketByteOffset;
    uint32_t ByteCount;
};

struct TimerDrivenCopyPlan {
    TimerDrivenCopySpan First;
    TimerDrivenCopySpan Second;
    uint32_t ByteCount;
};

inline bool ValidTimerDrivenPacketShape(uint32_t count, uint32_t size,
    uint32_t align, uint32_t pageSize)
{
    if (count != 1 || align == 0 || size == 0 || size % align != 0 ||
        size / align < MinPacketFrames || size / align > MaxPacketFrames) {
        return false;
    }
    uint32_t mappedBytes = 0;
    return RoundUpToPage(size, pageSize, mappedBytes) &&
        mappedBytes % align == 0;
}

inline bool IsTimerDriven(uint32_t count)
{
    return count == 1;
}

inline bool SupportsPacketNotifications(uint32_t count)
{
    return count == MaxPackets;
}

inline bool IsValidTimerDrivenRing(const TimerDrivenRing& ring)
{
    return ring.PacketSize != 0 && ring.MappedBytes >= ring.PacketSize &&
        ring.PacketFrames != 0 && ring.MappedFrames >= ring.PacketFrames &&
        ring.SegmentFrames != 0 && ring.SegmentFrames <= ring.PacketFrames &&
        ring.BlockAlign != 0 && ring.PacketSize % ring.BlockAlign == 0 &&
        ring.MappedBytes % ring.BlockAlign == 0 &&
        ring.PacketFrames == ring.PacketSize / ring.BlockAlign &&
        ring.MappedFrames == ring.MappedBytes / ring.BlockAlign &&
        ring.RingFrameOffset < ring.MappedFrames;
}

inline bool InitializeTimerDrivenRing(TimerDrivenRing& ring, uint32_t size,
    uint32_t align, uint32_t pageSize,
    uint32_t segmentFrames = DefaultTimerSegmentFrames)
{
    TimerDrivenRing candidate = {};
    if (!ValidTimerDrivenPacketShape(1, size, align, pageSize)) {
        return false;
    }
    uint32_t mappedBytes = 0;
    if (!RoundUpToPage(size, pageSize, mappedBytes)) {
        return false;
    }
    const auto packetFrames = size / align;
    const auto mappedFrames = mappedBytes / align;
    if (segmentFrames == 0) {
        segmentFrames = packetFrames < DefaultTimerSegmentFrames ?
            packetFrames : DefaultTimerSegmentFrames;
    }
    if (segmentFrames > packetFrames || segmentFrames > mappedFrames) {
        return false;
    }
    candidate.PacketSize = size;
    candidate.MappedBytes = mappedBytes;
    candidate.PacketFrames = packetFrames;
    candidate.MappedFrames = mappedFrames;
    candidate.SegmentFrames = segmentFrames;
    candidate.BlockAlign = align;
    ring = candidate;
    return true;
}

inline uint32_t NextTimerDrivenFrames(const TimerDrivenRing& ring)
{
    if (!IsValidTimerDrivenRing(ring)) {
        return 0;
    }
    const auto remaining = ring.PacketFrames - static_cast<uint32_t>(
        ring.PresentedFrames % ring.PacketFrames);
    const auto logical = ring.SegmentFrames < remaining ? ring.SegmentFrames : remaining;
    // Publish a complete native capture packet before any byte at its ring
    // beginning is overwritten by the next one. PCM packet boundaries differ.
    const auto native = ring.MappedFrames - ring.RingFrameOffset;
    return logical < native ? logical : native;
}

// Advance exactly one bounded segment. A delayed timer callback must not loop
// until wall-clock time is caught up; the caller can rebase the clock and
// discard stale PCM as one discontinuity instead.
inline bool NextTimerDrivenSegment(TimerDrivenRing& ring,
    TimerDrivenSegment& segment, uint32_t budget = DefaultTimerSegmentFrames)
{
    segment = {};
    const auto available = NextTimerDrivenFrames(ring);
    const auto frames = available < budget ? available : budget;
    if (frames == 0 ||
        ring.PresentedFrames > ~uint64_t{0} - frames ||
        ring.SegmentSequence == ~uint64_t{0}) {
        return false;
    }
    const auto packetOffset = static_cast<uint32_t>(
        ring.PresentedFrames % ring.PacketFrames);
    const auto remainingPacket = ring.PacketFrames - packetOffset;
    const auto firstFrames = ring.MappedFrames - ring.RingFrameOffset < frames ?
        ring.MappedFrames - ring.RingFrameOffset : frames;
    const auto secondFrames = frames - firstFrames;

    segment.Sequence = ring.SegmentSequence;
    segment.StartFrame = ring.PresentedFrames;
    segment.PacketSequence = ring.PresentedFrames / ring.PacketFrames;
    segment.PacketOffsetFrames = packetOffset;
    segment.RingFrameOffset = ring.RingFrameOffset;
    segment.Frames = frames;
    segment.FirstFrames = firstFrames;
    segment.SecondFrames = secondFrames;
    segment.CompletesPacket = frames == remainingPacket;

    // The first span reaches either the end of the mapping or the end of the
    // segment. With a wrap, the second span starts at ring frame zero.
    if (secondFrames != 0) {
        ring.RingFrameOffset = secondFrames;
    } else {
        const auto next = ring.RingFrameOffset + firstFrames;
        ring.RingFrameOffset = next == ring.MappedFrames ? 0 : next;
    }
    ring.PresentedFrames += frames;
    ++ring.SegmentSequence;
    return true;
}

// Produce at most two linear spans for either ring-to-packet or
// packet-to-ring copying. The actual copy stays outside this WDF-independent
// header so it can obey the caller's IRQL and address-space rules.
inline bool BuildTimerDrivenCopyPlan(const TimerDrivenRing& ring,
    const TimerDrivenSegment& segment, TimerDrivenCopyPlan& plan)
{
    plan = {};
    if (!IsValidTimerDrivenRing(ring) || segment.Frames == 0 ||
        segment.FirstFrames == 0 ||
        segment.RingFrameOffset >= ring.MappedFrames ||
        segment.FirstFrames > ring.MappedFrames - segment.RingFrameOffset ||
        segment.PacketOffsetFrames > ring.PacketFrames ||
        segment.Frames > ring.PacketFrames - segment.PacketOffsetFrames ||
        segment.Frames > ring.SegmentFrames) {
        return false;
    }
    const auto remainingRing = ring.MappedFrames - segment.RingFrameOffset;
    const auto firstFrames = remainingRing < segment.Frames ?
        remainingRing : segment.Frames;
    if (segment.FirstFrames != firstFrames ||
        segment.SecondFrames != segment.Frames - firstFrames) {
        return false;
    }
    const auto firstBytes = static_cast<uint64_t>(segment.FirstFrames) *
        ring.BlockAlign;
    const auto secondBytes = static_cast<uint64_t>(segment.SecondFrames) *
        ring.BlockAlign;
    const auto totalBytes = firstBytes + secondBytes;
    const auto packetOffsetBytes = static_cast<uint64_t>(
        segment.PacketOffsetFrames) * ring.BlockAlign;
    if (totalBytes > 0xffffffffULL || packetOffsetBytes > ring.PacketSize ||
        totalBytes > ring.PacketSize - packetOffsetBytes ||
        firstBytes > ring.MappedBytes || secondBytes > ring.MappedBytes) {
        return false;
    }
    plan.First = {segment.RingFrameOffset * ring.BlockAlign,
        static_cast<uint32_t>(packetOffsetBytes),
        static_cast<uint32_t>(firstBytes)};
    plan.Second = {0,
        static_cast<uint32_t>(packetOffsetBytes + firstBytes),
        static_cast<uint32_t>(secondBytes)};
    plan.ByteCount = static_cast<uint32_t>(totalBytes);
    return true;
}

struct TimerDrivenSchedule {
    long long Due;
    bool Rebased;
    bool Valid;
    uint64_t SegmentStartQpc;
};

inline TimerDrivenSchedule ScheduleTimerDrivenQuantum(Clock& clock,
    const TimerDrivenRing& ring, uint64_t now)
{
    if (!IsValidTimerDrivenRing(ring) || !clock.Running ||
        !ValidFrequency(clock.Frequency) ||
        ring.PresentedFrames > ~uint64_t{0} - DefaultTimerSegmentFrames) {
        return {0, false, false, 0};
    }
    const auto next = ring.PresentedFrames + DefaultTimerSegmentFrames;
    auto due = RelativeDue(clock, next, now);
    const bool rebased = due == 0;
    if (rebased) {
        clock.BaseFrames = ring.PresentedFrames;
        clock.EpochQpc = now;
        due = RelativeDue(clock, next, now);
    }
    clock.LimitFrames = next;
    return {due == 0 ? -1 : due, rebased, true,
        FrameQpc(clock, ring.PresentedFrames)};
}

inline uint64_t TimerDrivenPosition(const TimerDrivenRing& ring,
    const Clock& clock, uint64_t now)
{
    if (!IsValidTimerDrivenRing(ring) || !ValidFrequency(clock.Frequency)) {
        return ring.PresentedFrames;
    }
    const auto clockPosition = Position(clock, now);
    const auto frames = NextTimerDrivenFrames(ring);
    const auto ceiling = ring.PresentedFrames > ~uint64_t{0} - frames ?
        ~uint64_t{0} : ring.PresentedFrames + frames;
    const auto bounded = clockPosition < ceiling ? clockPosition : ceiling;
    return bounded > ring.PresentedFrames ? bounded : ring.PresentedFrames;
}

// Schedule only the next segment. On a long stall, rebase to the current ring
// frontier and schedule one fresh segment; never recursively catch up old time.
inline TimerDrivenSchedule ScheduleTimerDrivenSegment(Clock& clock,
    const TimerDrivenRing& ring, uint64_t now)
{
    const auto frames = NextTimerDrivenFrames(ring);
    if (frames == 0 || !clock.Running ||
        !ValidFrequency(clock.Frequency) ||
        ring.PresentedFrames > ~uint64_t{0} - frames) {
        return {0, false, false, 0};
    }
    const auto next = ring.PresentedFrames + frames;
    auto due = RelativeDue(clock, next, now);
    const bool rebased = due == 0;
    if (rebased) {
        clock.BaseFrames = ring.PresentedFrames;
        clock.EpochQpc = now;
        due = RelativeDue(clock, next, now);
    }
    clock.LimitFrames = next;
    return {due == 0 ? -1 : due, rebased, true,
        FrameQpc(clock, ring.PresentedFrames)};
}

enum class RenderPacketResult { Accepted, Invalid, Ended, Late, Overrun };

inline RenderPacketResult ValidateRenderPacket(uint64_t completed,
    uint32_t count, uint32_t size, uint32_t align, bool started, bool ended,
    uint32_t packet, uint32_t flags, uint32_t eosLength)
{
    // Packet sequencing is only used by the event-driven ping-pong path.
    if (count != MaxPackets || !ValidPacketShape(count, size, align)) {
        return RenderPacketResult::Invalid;
    }
    if (ended) {
        return RenderPacketResult::Ended;
    }
    if ((flags & ~EndOfStream) != 0 ||
        ((flags & EndOfStream) != 0 &&
            (eosLength > size || eosLength % align != 0))) {
        return RenderPacketResult::Invalid;
    }
    // Only a window of at most two packets is admissible. Unsigned modular
    // subtraction handles ULONG packet wrap without narrowing a signed value.
    const auto distance = static_cast<uint32_t>(
        packet - static_cast<uint32_t>(completed));
    if (distance >= 0x80000000u || (started && distance == 0)) {
        return RenderPacketResult::Late;
    }
    if (distance >= count) {
        return RenderPacketResult::Overrun;
    }
    return RenderPacketResult::Accepted;
}

inline bool TakeCapturePacket(uint64_t completed, uint64_t& delivered,
    uint32_t& packet)
{
    if (completed == 0 || completed == delivered) {
        return false;
    }
    // Keep only the latest completion; a slow reader never causes a burst.
    packet = static_cast<uint32_t>(completed - 1);
    delivered = completed;
    return true;
}
}
