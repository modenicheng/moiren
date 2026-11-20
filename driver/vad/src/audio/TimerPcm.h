// SPDX-License-Identifier: MIT
#pragma once
#include "Clock.h"
#include "../Lifecycle.h"

namespace vad::audio {
struct TimerPcmPacket {
    uint64_t Session;
    uint64_t GateEpoch;
    bool Active;
    // Diagnostic provenance only; never changes the copy/admission decision.
    bool CaptureCopied;
    uint64_t RequestSequence;
};

// The caller holds the endpoint state lock across validation and copying. Only
// PCM bytes survive between callbacks: a WDF request/MDL never does.
template<typename Clear, typename Copy, typename Zero, typename Transfer>
inline bool TransferTimerSegment(const vad::Slot& slot, uint64_t endpoint,
    uint64_t generation, bool capture, const TimerDrivenRing& ring,
    const TimerDrivenSegment& segment, TimerPcmPacket& packet,
    Clear clear, Copy copy, Zero zero, Transfer transfer)
{
    TimerDrivenCopyPlan plan = {};
    if (!BuildTimerDrivenCopyPlan(ring, segment, plan)) {
        return false;
    }
    const bool ready = slot.Id == endpoint && slot.StreamGeneration == generation &&
        slot.State == MOIREN_VAD_STATE_PRESENT && slot.AudioReady != 0 &&
        slot.GateOpen != 0 && slot.StreamState == MOIREN_VAD_STREAM_RUNNING;
    if (segment.PacketOffsetFrames == 0) {
        clear();
        packet.Session = slot.Session;
        packet.GateEpoch = slot.AdmissionGeneration;
        packet.Active = ready;
        if (capture && packet.Active) {
            transfer();
        }
    } else if (!ready || packet.Session != slot.Session ||
        packet.GateEpoch != slot.AdmissionGeneration) {
        // Once invalidated, this logical packet cannot be resumed by opening a
        // gate in its middle. A new epoch starts at the next packet boundary.
        clear();
        packet.Active = false;
    }
    if (packet.Active) {
        copy(plan.First);
        if (plan.Second.ByteCount != 0) {
            copy(plan.Second);
        }
    }
    if (!capture || !packet.Active) {
        zero(plan.First);
        if (plan.Second.ByteCount != 0) {
            zero(plan.Second);
        }
    }
    if (segment.CompletesPacket && !capture && packet.Active) {
        transfer();
    }
    return true;
}
}
