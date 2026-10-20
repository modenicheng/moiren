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
    return count >= 1 && count <= MaxPackets && align != 0 && size != 0 &&
        size % align == 0 && size / align >= MinPacketFrames &&
        size / align <= MaxPacketFrames;
}

enum class RenderPacketResult { Accepted, Invalid, Ended, Late, Overrun };

inline RenderPacketResult ValidateRenderPacket(uint64_t completed,
    uint32_t count, uint32_t size, uint32_t align, bool started, bool ended,
    uint32_t packet, uint32_t flags, uint32_t eosLength)
{
    if (!ValidPacketShape(count, size, align)) {
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
