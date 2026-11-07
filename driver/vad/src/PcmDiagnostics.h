// SPDX-License-Identifier: MIT
#pragma once
#include "../include/moiren_vad.h"

namespace vad {
enum class PcmWriteOutcome : uint32_t {
    Unobserved, Empty, Success, NewerThanCallback, TooOld, NoCurrentRequest
};

inline PcmWriteOutcome ClassifyPcmWrite(uint64_t headerQpc, uint64_t callbackQpc,
    uint64_t periodTicks)
{
    if (headerQpc > callbackQpc) {
        return PcmWriteOutcome::NewerThanCallback;
    }
    if (callbackQpc - headerQpc > periodTicks) {
        return PcmWriteOutcome::TooOld;
    }
    return PcmWriteOutcome::Success;
}

struct PcmTransferObservation {
    PcmWriteOutcome Outcome;
    uint64_t GateEpoch;
    uint64_t NativeSequence;
    uint64_t RequestSequence;
    uint64_t HeaderQpc;
    uint64_t CallbackQpc;
    uint64_t CheckQpc;
    uint64_t PeriodTicks;
    uint32_t Frames = 0;
};

struct PcmDiagnostics {
    uint64_t Generation;
    uint64_t RunGateEpoch;
    uint64_t RunQpc;
    PcmTransferObservation FirstTransfer;
    PcmTransferObservation FirstRejected;
    bool ReportFirst;
    bool ReportRejected;

    void Reset(uint64_t generation) { *this = {}; Generation = generation; }
    void RecordRun(uint64_t gate, uint64_t qpc)
    {
        if (RunQpc == 0) {
            RunGateEpoch = gate;
            RunQpc = qpc;
        }
    }
    void Record(const PcmTransferObservation& observation)
    {
        if (FirstTransfer.Outcome == PcmWriteOutcome::Unobserved) {
            FirstTransfer = observation;
            ReportFirst = true;
        }
        if (FirstRejected.Outcome == PcmWriteOutcome::Unobserved &&
            (observation.Outcome == PcmWriteOutcome::NewerThanCallback ||
                observation.Outcome == PcmWriteOutcome::TooOld)) {
            FirstRejected = observation;
            ReportRejected = true;
        }
    }
    // Called only by the passive control query while Shared->Lock is held.
    // RT retains the fixed records, never formats or emits a trace event.
    PcmDiagnostics TakeUnreported()
    {
        const auto snapshot = *this;
        ReportFirst = ReportRejected = false;
        return snapshot;
    }
};
static_assert(sizeof(PcmDiagnostics) <= 256, "fixed per-endpoint diagnostic budget");
}
