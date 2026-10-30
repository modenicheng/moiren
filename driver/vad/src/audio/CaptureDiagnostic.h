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

struct CaptureReadPrefix {
    uint32_t Bytes;
    bool Nonzero;

    static CaptureReadPrefix Read(const void* buffer, uint32_t bytes)
    {
        CaptureReadPrefix result = {};
        result.Bytes = bytes < 16 ? bytes : 16;
        const auto from = static_cast<const volatile unsigned char*>(buffer);
        for (uint32_t i = 0; i < result.Bytes; ++i) {
            result.Nonzero |= from[i] != 0;
        }
        return result;
    }
};

struct CapturePacketObservation {
    uint64_t GateEpoch; // Gate at callback readback, not proof of the payload's origin.
    uint64_t Completed;
    uint64_t DeliveredBefore;
    uint64_t DeliveredAfter;
    uint64_t PacketStartQpc;
    uint32_t Status;
    uint32_t Packet;
    uint32_t PacketCount;
    uint32_t Index;
    uint32_t OffsetBytes;
    CaptureReadPrefix Prefix;
    bool Observed;
};

struct CaptureNotifyObservation {
    uint64_t GateEpoch; // Captured under State/Shared before completion/notification.
    uint64_t NativeSequence;
    uint64_t CompletionQpc;
    uint32_t PacketCount;
    uint32_t Status;
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
    CapturePacketObservation FirstPacket;
    CapturePacketObservation FirstNonzeroPacket;
    CapturePacketObservation FirstPacketRefusal;
    CaptureNotifyObservation FirstNotifySuccess;
    CaptureNotifyObservation FirstNotifyFailure;
    bool ReportCopy;
    bool ReportNonzero;
    bool ReportPosition;
    bool ReportPacket;
    bool ReportNonzeroPacket;
    bool ReportPacketRefusal;
    bool ReportNotifySuccess;
    bool ReportNotifyFailure;

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
    bool NeedsPacket(uint32_t status) const
    {
        return (status & 0x80000000u) == 0 ?
            !FirstPacket.Observed || !FirstNonzeroPacket.Observed : !FirstPacketRefusal.Observed;
    }
    void RecordPacket(uint64_t generation, CapturePacketObservation observation)
    {
        if (generation != Generation) { return; }
        observation.Observed = true;
        if ((observation.Status & 0x80000000u) != 0) {
            if (!FirstPacketRefusal.Observed) {
                FirstPacketRefusal = observation;
                ReportPacketRefusal = true;
            }
            return;
        }
        if (!FirstPacket.Observed) {
            FirstPacket = observation;
            ReportPacket = true;
        }
        if (!FirstNonzeroPacket.Observed && observation.Prefix.Nonzero) {
            FirstNonzeroPacket = observation;
            ReportNonzeroPacket = true;
        }
    }
    void RecordNotify(uint64_t generation, CaptureNotifyObservation observation)
    {
        if (generation != Generation || !observation.Observed) { return; }
        if ((observation.Status & 0x80000000u) == 0) {
            if (!FirstNotifySuccess.Observed) {
                FirstNotifySuccess = observation;
                ReportNotifySuccess = true;
            }
        } else if (!FirstNotifyFailure.Observed) {
            FirstNotifyFailure = observation;
            ReportNotifyFailure = true;
        }
    }
    // QUERY consumes pending publication, not historical witnesses. Missing
    // position/query evidence is unknown, not proof of no delivery: ACX clients
    // may read the packet count/QPC register updated by NotifyPacketComplete.
    CaptureDiagnostic TakeUnreported()
    {
        const auto snapshot = *this;
        ReportCopy = ReportNonzero = ReportPosition = false;
        ReportPacket = ReportNonzeroPacket = ReportPacketRefusal = false;
        ReportNotifySuccess = ReportNotifyFailure = false;
        return snapshot;
    }
};
static_assert(sizeof(CaptureDiagnostic) <= 512, "fixed capture boundary diagnostic budget");
}
