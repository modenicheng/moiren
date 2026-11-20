// SPDX-License-Identifier: MIT
#pragma once
#include "../include/moiren_vad.h"

namespace vad {
enum UnservedFlag : uint16_t {
    UnservedEmpty = 1, UnservedNoCurrent = 2, UnservedTooOld = 4,
    UnservedRetained = 8, UnservedReport = 16, UnservedHasWrite = 32
};

struct WriteBoundary {
    uint64_t GateEpoch;
    uint64_t Sequence;
    uint64_t HeaderQpc;
    uint64_t PreForwardQpc;
};

// Zero deadline explicitly means timer mode (no immutable event packet).
struct CallbackBoundary {
    uint64_t DpcQpc;
    uint64_t StateQpc;
    uint64_t DeadlineQpc;
    uint64_t Stream;
    uint64_t Frequency;
};

struct UnservedObservation {
    uint64_t DpcQpc;
    uint64_t StateQpc;
    uint64_t DeadlineQpc;
    uint64_t GateEpoch;
    uint64_t WriteSequence;
    uint64_t WriteHeaderQpc;
    uint64_t WritePreForwardQpc;
    int RetrieveStatus; // NTSTATUS bits; int is 32-bit on the driver/host targets.
    uint16_t Flags;
    uint16_t RetrieveCalls;
};

struct UnservedDiagnostics {
    uint64_t Generation;
    uint64_t Stream;
    uint64_t Frequency;
    WriteBoundary LatestWrite;
    UnservedObservation First[3];

    void Reset(uint64_t generation) { *this = {}; Generation = generation; }
    // Caller validates the full endpoint/session/stream/gate identity under
    // Shared->Lock while the request is still owned by the submitting caller.
    void RecordPreForward(uint64_t generation, const WriteBoundary& write)
    {
        if (generation == Generation) { LatestWrite = write; }
    }
    void RecordUnserved(uint64_t generation, uint64_t gate,
        const CallbackBoundary& callback,
        int retrieveStatus, uint16_t calls, uint16_t reasons)
    {
        if (generation != Generation) { return; }
        for (unsigned i = 0; i < 3; ++i) {
            auto& first = First[i];
            if ((reasons & (1u << i)) == 0 ||
                (first.Flags & UnservedRetained) != 0) { continue; }
            Stream = callback.Stream;
            Frequency = callback.Frequency;
            first.DpcQpc = callback.DpcQpc;
            first.StateQpc = callback.StateQpc;
            first.DeadlineQpc = callback.DeadlineQpc;
            first.GateEpoch = gate;
            first.RetrieveStatus = retrieveStatus;
            first.RetrieveCalls = calls;
            first.Flags = reasons | UnservedRetained | UnservedReport;
            // A retired gate's most recent write is NOT evidence for this one.
            if (LatestWrite.GateEpoch == gate && LatestWrite.PreForwardQpc != 0) {
                first.WriteSequence = LatestWrite.Sequence;
                first.WriteHeaderQpc = LatestWrite.HeaderQpc;
                first.WritePreForwardQpc = LatestWrite.PreForwardQpc;
                first.Flags |= UnservedHasWrite;
            }
        }
    }
    UnservedDiagnostics TakeUnreported()
    {
        const auto snapshot = *this;
        for (auto& first : First) { first.Flags &= ~UnservedReport; }
        return snapshot;
    }
};
static_assert(sizeof(UnservedDiagnostics) <= 256,
    "all additional persistent endpoint evidence must fit 256 bytes");
}
