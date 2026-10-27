// SPDX-License-Identifier: MIT
#pragma once
#include "Clock.h"
#include "../Lifecycle.h"

namespace vad::audio {
enum class ClockRebaseReason : uint32_t { Unobserved, EventDeadline, TimerStall };

struct ClockEndpointState {
    uint64_t Generation;
    uint64_t GateEpoch;
    uint32_t GateOpen;
    uint32_t StreamState;
};

inline ClockEndpointState ObserveClockEndpoint(const vad::Slot& slot)
{
    return {slot.StreamGeneration, slot.AdmissionGeneration,
        slot.GateOpen, slot.StreamState};
}

struct ClockRebaseObservation {
    Clock Before;
    Clock After;
    uint64_t CallbackQpc;
    uint64_t DeadlineFrame;
    uint64_t DeadlineQpc;
    uint64_t LatenessTicks;
    uint64_t CompletedPackets;
    uint64_t PresentedFrames;
    uint32_t PacketFrames;
    uint32_t PacketCount;
    ClockRebaseReason Reason;
};

inline ClockRebaseObservation ObserveClockRebase(ClockRebaseReason reason,
    const Clock& before, const Clock& after, uint64_t callbackQpc,
    uint32_t packetFrames, uint32_t packetCount, uint64_t completedPackets,
    uint64_t presentedFrames)
{
    // The event scheduler tests its NEXT packet deadline after completion.
    // The timer planner instead rebases only when more than one logical packet
    // remains unprocessed. Both deadlines must use the old clock association.
    const auto frame = reason == ClockRebaseReason::EventDeadline ?
        (completedPackets + 1) * packetFrames : presentedFrames + packetFrames;
    const auto deadline = FrameQpc(before, frame);
    return {before, after, callbackQpc, frame, deadline,
        callbackQpc > deadline ? callbackQpc - deadline : 0,
        completedPackets, presentedFrames, packetFrames, packetCount, reason};
}

struct ClockDiagnostic {
    uint64_t Generation;
    uint64_t Stream;
    ClockEndpointState EndpointBefore;
    ClockEndpointState EndpointAfter;
    ClockRebaseObservation First;
    bool ReportFirst;

    void Reset(uint64_t generation) { *this = {}; Generation = generation; }
    void Record(uint64_t stream, const ClockEndpointState& before,
        const ClockEndpointState& after, const ClockRebaseObservation& observation)
    {
        if (First.Reason == ClockRebaseReason::Unobserved &&
            observation.Reason != ClockRebaseReason::Unobserved &&
            before.Generation == Generation && after.Generation == Generation) {
            Stream = stream;
            EndpointBefore = before;
            EndpointAfter = after;
            First = observation;
            ReportFirst = true;
        }
    }
    // Shared->Lock protects recording and this passive QUERY/gate-close drain.
    // Draining never erases the historical first observation or rearms it.
    ClockDiagnostic TakeUnreported()
    {
        const auto snapshot = *this;
        ReportFirst = false;
        return snapshot;
    }
};
static_assert(sizeof(ClockDiagnostic) <= 256, "fixed per-endpoint clock diagnostic budget");
}
