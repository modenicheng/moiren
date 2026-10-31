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
    bool NonzeroCopy; // Selected under State/Shared, before notification.
};

struct CaptureNonzeroNotifyObservation {
    uint64_t EnterQpc;
    uint64_t ReturnQpc;
    uint32_t Status;
    uint32_t Index;
    bool Observed;
};

// The first actual MIC no-copy increment, not a PCM-content or client-delivery
// observation. Original transfer identity/state survive gate close and Stop.
struct CaptureNoCopyObservation {
    uint64_t EndpointId;
    uint64_t SessionEpoch;
    uint64_t StreamGeneration;
    uint64_t GateEpoch;
    uint64_t NativeSequence;
    uint64_t CallbackQpc;
    uint64_t Frequency; // Existing callback frequency; zero remains unknown.
    uint64_t SilenceBefore;
    uint64_t SilenceAfter;
    uint64_t PacketStartQpc;
    uint64_t EventGateEpoch; // May advance after transfer; never relabel GateEpoch.
    uint32_t State;
    uint32_t StreamState;
    uint32_t GateOpen;
    uint32_t AudioReady;
    uint32_t SampleFormat;
    uint32_t Channels;
    uint32_t Frames;
    uint32_t PacketCount;
    uint32_t Index;
    uint32_t PacketBytes;
    uint32_t NativePacketBytes;
    uint32_t PacketOffsetBytes;
    uint32_t NotifyStatus;
    bool Observed;
    bool EventObserved;
    bool NotifyObserved;
};
static_assert(sizeof(CaptureNoCopyObservation) <= 160, "fixed first no-copy witness");

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
    CaptureNonzeroNotifyObservation NonzeroNotify;
    CaptureNoCopyObservation FirstNoCopy;
    bool ReportCopy;
    bool ReportNonzero;
    bool ReportPosition;
    bool ReportPacket;
    bool ReportNonzeroPacket;
    bool ReportPacketRefusal;
    bool ReportNotifySuccess;
    bool ReportNotifyFailure;
    bool ReportNonzeroNotify;
    bool ReportNoCopy;
    bool ReportNoCopyEvent;
    bool ReportNoCopyNotify;

    void Reset(uint64_t generation) { *this = {}; Generation = generation; }
    void RecordNoCopy(CaptureNoCopyObservation observation)
    {
        if (observation.StreamGeneration != Generation || FirstNoCopy.Observed) { return; }
        observation.Observed = true;
        FirstNoCopy = observation;
        ReportNoCopy = true;
    }
    void RecordNoCopyEvent(uint64_t generation, const CaptureNotifyObservation& notification,
        uint32_t index, uint32_t packetBytes, uint32_t nativeBytes,
        uint32_t offsetBytes, uint64_t packetStartQpc)
    {
        auto& item = FirstNoCopy;
        if (generation != Generation || !item.Observed || item.EventObserved ||
            !notification.Observed || notification.PacketCount <= 1 ||
            notification.NativeSequence != item.NativeSequence ||
            notification.CompletionQpc != item.CallbackQpc ||
            index != notification.NativeSequence % notification.PacketCount) { return; }
        item.EventGateEpoch = notification.GateEpoch;
        item.PacketCount = notification.PacketCount;
        item.Index = index;
        item.PacketBytes = packetBytes;
        item.NativePacketBytes = nativeBytes;
        item.PacketOffsetBytes = offsetBytes;
        item.PacketStartQpc = packetStartQpc;
        item.EventObserved = true;
        ReportNoCopyEvent = true;
    }
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
    bool NeedsNonzeroNotify(const CaptureNotifyObservation& observation) const
    {
        return observation.Observed && !NonzeroNotify.Observed && FirstNonzero.Observed &&
            FirstNonzero.PacketCount > 1 && observation.PacketCount == FirstNonzero.PacketCount &&
            observation.GateEpoch == FirstNonzero.GateEpoch &&
            observation.NativeSequence == FirstNonzero.NativeSequence &&
            observation.CompletionQpc == FirstNonzero.Qpc;
    }
    void RecordNotify(uint64_t generation, CaptureNotifyObservation observation,
        uint64_t enterQpc, uint64_t returnQpc)
    {
        if (generation != Generation || !observation.Observed) { return; }
        auto& noCopy = FirstNoCopy;
        if (noCopy.EventObserved && !noCopy.NotifyObserved &&
            observation.GateEpoch == noCopy.EventGateEpoch &&
            observation.NativeSequence == noCopy.NativeSequence &&
            observation.CompletionQpc == noCopy.CallbackQpc &&
            observation.PacketCount == noCopy.PacketCount) {
            noCopy.NotifyStatus = observation.Status;
            noCopy.NotifyObserved = true;
            ReportNoCopyNotify = true;
        }
        if (observation.NonzeroCopy && NeedsNonzeroNotify(observation)) {
            // The copy witness already owns native/request/gate/copy QPC. Match
            // all identities before attaching the result of this exact call.
            NonzeroNotify = {enterQpc, returnQpc, observation.Status,
                static_cast<uint32_t>(observation.NativeSequence % observation.PacketCount), true};
            ReportNonzeroNotify = true;
        }
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
        ReportNonzeroNotify = false;
        ReportNoCopy = ReportNoCopyEvent = ReportNoCopyNotify = false;
        return snapshot;
    }
};
static_assert(sizeof(CaptureDiagnostic) <= 768, "fixed capture boundary diagnostic budget");
}
