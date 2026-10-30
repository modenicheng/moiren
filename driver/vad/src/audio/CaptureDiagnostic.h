// SPDX-License-Identifier: MIT
#pragma once
#include "../../include/moiren_vad.h"

namespace vad::audio {
struct CapturePrefix {
    uint32_t Bytes;
    bool SourceNonzero;
    bool DestinationNonzero;
    bool Equal;

    static CapturePrefix ReadBack(const void* source, const void* destination, uint32_t bytes)
    {
        CapturePrefix result = {};
        result.Bytes = bytes < 16 ? bytes : 16;
        result.Equal = true;
        // Observe actual mapped memory after the copy, not a compiler-inferred
        // memcpy result. No sample values, pointers or borrowed buffers escape.
        const auto from = static_cast<const volatile unsigned char*>(source);
        const auto to = static_cast<const volatile unsigned char*>(destination);
        for (uint32_t i = 0; i < result.Bytes; ++i) {
            const auto a = from[i];
            const auto b = to[i];
            result.SourceNonzero |= a != 0;
            result.DestinationNonzero |= b != 0;
            result.Equal &= a == b;
        }
        return result;
    }
};

struct CaptureCopyObservation {
    uint64_t GateEpoch;
    uint64_t NativeSequence;
    uint64_t RequestSequence;
    uint64_t Qpc;
    uint32_t PacketCount;
    uint32_t PacketBytes;
    uint32_t NativePacketBytes;
    uint32_t RingOffsetBytes;
    uint32_t PacketOffsetBytes;
    CapturePrefix Prefix;
    bool Observed;
};

struct CapturePositionObservation {
    uint64_t GateEpoch;
    uint64_t Frames;
    uint64_t Qpc;
    uint32_t PacketCount;
    bool Observed;
};

// Existing State/Shared locks protect these fixed first witnesses. No RT trace,
// allocation, full-packet scan, scheduling change or public ABI is involved.
struct CaptureDiagnostic {
    uint64_t Generation;
    uint64_t Stream;
    CaptureCopyObservation FirstCopy;
    CaptureCopyObservation FirstNonzero;
    CapturePositionObservation Position;
    bool ReportCopy;
    bool ReportNonzero;
    bool ReportPosition;

    void Reset(uint64_t generation) { *this = {}; Generation = generation; }
    bool NeedsCopy() const { return !FirstCopy.Observed || !FirstNonzero.Observed; }
    void RecordCopy(uint64_t generation, uint64_t stream, CaptureCopyObservation observation)
    {
        if (generation != Generation || observation.Prefix.Bytes == 0) { return; }
        observation.Observed = true;
        if (!FirstCopy.Observed) {
            Stream = stream;
            FirstCopy = observation;
            ReportCopy = true;
        }
        if (!FirstNonzero.Observed &&
            (observation.Prefix.SourceNonzero || observation.Prefix.DestinationNonzero)) {
            FirstNonzero = observation;
            ReportNonzero = true;
        }
    }
    void RecordPosition(uint64_t generation, uint64_t stream, CapturePositionObservation observation)
    {
        if (generation != Generation || stream != Stream || Position.Observed ||
            !FirstCopy.Observed || observation.PacketCount == 0 ||
            observation.GateEpoch != FirstCopy.GateEpoch) { return; }
        observation.Observed = true;
        Position = observation;
        ReportPosition = true;
    }
    // QUERY consumes pending publication, not historical witnesses. An absent
    // position is unknown/not observed, never evidence that ACX did not poll.
    CaptureDiagnostic TakeUnreported()
    {
        const auto snapshot = *this;
        ReportCopy = ReportNonzero = ReportPosition = false;
        return snapshot;
    }
};
static_assert(sizeof(CaptureDiagnostic) <= 256, "fixed capture boundary diagnostic budget");
}
