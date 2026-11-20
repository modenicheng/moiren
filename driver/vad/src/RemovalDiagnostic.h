// SPDX-License-Identifier: MIT
#pragma once
#include "../include/moiren_vad.h"

namespace vad {
enum class RemovalStage : uint32_t {
    RemoveEnter, RemoveReturn, ReleaseEnter, ReleaseReturn, Cleanup, Destroy, Count
};
constexpr unsigned RemovalStageCount = static_cast<unsigned>(RemovalStage::Count);
constexpr uint32_t RemovalLossPending = uint32_t{1} << 31;
constexpr uint32_t RemovalStagesMask = (uint32_t{1} << RemovalStageCount) - 1;

struct RemovalObservation {
    uint64_t Sequence;
    uint64_t Qpc;
    uint32_t Status;
    bool HasStatus;
    bool NativeCall;
};

struct RemovalStageDiagnostic {
    uint32_t Count;
    RemovalObservation First;
    RemovalObservation Last;
    RemovalObservation FirstFailure;
};

// Shared->Lock orders these control/PnP observations. No heap, WDF references,
// PCM or inferred progress; an unobserved stage remains unknown, never success.
struct RemovalDiagnostic {
    uint64_t Runtime;
    uint64_t Session;
    uint64_t Sequence;
    uint64_t Frequency;
    uint64_t DiscardedRuntimes;
    uint64_t DiscardedRuntime;
    uint64_t DiscardedSession;
    uint64_t DiscardedSequence;
    uint32_t DiscardedStages;
    bool DiscardSaturated;
    RemovalStageDiagnostic Stages[RemovalStageCount];
    uint32_t Pending;
    bool Saturated;

    void Reset(uint64_t runtime, uint64_t session)
    {
        // CREATE must not delay slot reuse for diagnostics, but also must not
        // silently present an overwritten pending lifetime as absent evidence.
        auto discarded = DiscardedRuntimes;
        auto oldRuntime = DiscardedRuntime;
        auto oldSession = DiscardedSession;
        auto oldSequence = DiscardedSequence;
        auto oldStages = DiscardedStages;
        auto exhausted = DiscardSaturated;
        if ((Pending & RemovalStagesMask) != 0) {
            if (discarded == ~uint64_t{0}) { exhausted = true; }
            else { ++discarded; }
            oldRuntime = Runtime;
            oldSession = Session;
            oldSequence = Sequence;
            oldStages = Pending & RemovalStagesMask;
        }
        *this = {};
        DiscardedRuntimes = discarded;
        DiscardedRuntime = oldRuntime;
        DiscardedSession = oldSession;
        DiscardedSequence = oldSequence;
        DiscardedStages = oldStages;
        DiscardSaturated = exhausted;
        if (discarded != 0) { Pending = RemovalLossPending; }
        Runtime = runtime;
        Session = session;
    }

    void Record(uint64_t runtime, uint64_t session, RemovalStage stage,
        uint64_t qpc, bool hasStatus = false, uint32_t status = 0, bool nativeCall = false)
    {
        const auto index = static_cast<unsigned>(stage);
        if (Runtime == 0 || Runtime != runtime || Session != session || index >= RemovalStageCount) {
            return;
        }
        auto& item = Stages[index];
        Pending |= uint32_t{1} << index;
        if (Sequence == ~uint64_t{0} || item.Count == ~uint32_t{0}) {
            Saturated = true;
            return;
        }
        RemovalObservation observation = {++Sequence, qpc, status, hasStatus, nativeCall};
        if (item.Count++ == 0) {
            item.First = observation;
        }
        item.Last = observation;
        if (hasStatus && (status & 0x80000000u) != 0 && item.FirstFailure.Sequence == 0) {
            item.FirstFailure = observation;
        }
    }

    RemovalDiagnostic TakePending()
    {
        const auto result = *this;
        Pending = 0;
        return result;
    }
};
static_assert(sizeof(RemovalDiagnostic) <= 768, "fixed per-endpoint removal diagnostic budget");
}
