// SPDX-License-Identifier: MIT
// Pure bounded state transitions, also exercised by the host tests.
#pragma once
#include "../include/moiren_vad.h"

namespace vad {
constexpr uint32_t Creating = MOIREN_VAD_STATE_CREATING;

struct Slot {
    uint64_t Id;
    uint64_t Session;
    uint32_t State;
    uint32_t Intent;
    uint32_t Channels;
    uint32_t PolicyPrepared;
    uint32_t GateOpen;
    uint32_t AudioReady;
    uint64_t AdmissionGeneration;
    uint64_t StreamGeneration;
    MOIREN_VAD_ENDPOINT_ID StableId;
    uint32_t SampleFormat;
    uint32_t PacketFrames;
    uint32_t LastStatus;
    uint32_t StreamState;
    uint64_t DroppedPackets;
    uint64_t SilencePackets;
    uint64_t LastSequence;
    uint32_t HasSequence;
    uint32_t PendingRequests;
    uint32_t AllowDefault;
    uint16_t Name[MOIREN_VAD_MAX_NAME_UNITS];
};

inline bool ValidEndpoint(uint32_t intent, uint32_t channels)
{
    return (intent == MOIREN_VAD_INTENT_PLAYBACK ||
            intent == MOIREN_VAD_INTENT_MICROPHONE) &&
        (channels == 1 || channels == 2);
}

inline bool Reusable(const Slot& slot)
{
    return slot.State == 0 || slot.State == MOIREN_VAD_STATE_REMOVED;
}

// Call before Reserve. This acknowledges only this request's lack of effects;
// neither the old slot nor its runtime identity is adopted by the new owner.
inline bool TryDeferCreate(const Slot& slot, uint64_t session,
    const MOIREN_VAD_CREATE_REQUEST& request, MOIREN_VAD_CREATE_DEFERRED& reply)
{
    if (slot.Session == 0 || session == 0 || slot.Session >= session ||
        slot.State != MOIREN_VAD_STATE_REMOVING ||
        slot.GateOpen != 0 || slot.AudioReady != 0 ||
        slot.StableId.Low != request.StableId.Low ||
        slot.StableId.High != request.StableId.High) {
        return false;
    }
    reply = {};
    reply.Size = sizeof(reply);
    reply.Version = MOIREN_VAD_ABI_VERSION;
    reply.Kind = MOIREN_VAD_CREATE_DEFERRED_KIND;
    reply.Intent = request.Intent;
    reply.Channels = request.Channels;
    reply.SessionEpoch = session;
    reply.StableId = request.StableId;
    return true;
}

// Returns an index, -1 for quota, or -2 for exhausted runtime IDs.
inline int Reserve(Slot (&slots)[MOIREN_VAD_MAX_ENDPOINTS], uint64_t session,
    uint64_t& nextId, uint32_t intent, uint32_t channels)
{
    if (nextId == ~uint64_t{0}) {
        return -2;
    }
    for (uint32_t i = 0; i < MOIREN_VAD_MAX_ENDPOINTS; ++i) {
        if (Reusable(slots[i])) {
            slots[i] = {};
            slots[i].Id = ++nextId;
            slots[i].Session = session;
            slots[i].State = Creating;
            slots[i].Intent = intent;
            slots[i].Channels = channels;
            slots[i].AdmissionGeneration = 1;
            return static_cast<int>(i);
        }
    }
    return -1;
}

inline bool FinishLifetime(Slot& slot, uint64_t id)
{
    if (slot.Id != id) {
        return false;
    }
    slot.GateOpen = 0;
    slot.AudioReady = 0;
    slot.StreamState = MOIREN_VAD_STREAM_CLOSED;
    slot.SampleFormat = 0;
    slot.PacketFrames = 0;
    slot.HasSequence = 0;
    slot.State = MOIREN_VAD_STATE_REMOVED;
    return true;
}

inline void CloseGate(Slot& slot)
{
    if (slot.GateOpen != 0) {
        slot.GateOpen = 0;
        if (slot.AdmissionGeneration != ~uint64_t{0}) {
            ++slot.AdmissionGeneration;
        }
        slot.HasSequence = 0;
    }
}

inline void InvalidatePcmEpoch(Slot& slot, bool pauseStream = false)
{
    // A reset must advance even a closed gate; exhaustion cannot reuse a stamp.
    if (slot.AdmissionGeneration == ~uint64_t{0}) {
        slot.GateOpen = 0;
    } else {
        ++slot.AdmissionGeneration;
    }
    slot.HasSequence = 0;
    if (pauseStream) {
        // Publish with the epoch under Shared->Lock, before a reset can purge.
        slot.StreamState = MOIREN_VAD_STREAM_PAUSED;
    }
}

inline bool Revoke(Slot& slot)
{
    if (slot.State != Creating && slot.State != MOIREN_VAD_STATE_PRESENT &&
        slot.State != MOIREN_VAD_STATE_REMOVING) {
        return false;
    }
    CloseGate(slot);
    slot.AudioReady = 0;
    slot.State = MOIREN_VAD_STATE_REMOVING;
    return true;
}

inline MOIREN_VAD_ENDPOINT_SNAPSHOT Snapshot(const Slot& slot)
{
    return {sizeof(MOIREN_VAD_ENDPOINT_SNAPSHOT), MOIREN_VAD_ABI_VERSION,
        slot.Id, slot.State, slot.Intent, slot.Channels, slot.GateOpen,
        slot.PolicyPrepared, slot.AudioReady, slot.Session, slot.Id,
        slot.StreamGeneration, slot.AdmissionGeneration, slot.StableId,
        slot.SampleFormat, slot.PacketFrames, slot.LastStatus, slot.StreamState,
        slot.DroppedPackets, slot.SilencePackets, 0};
}

inline bool SameIdentity(const Slot& slot, const MOIREN_VAD_ENDPOINT_REQUEST& request)
{
    return slot.Id != 0 && slot.Id == request.EndpointId &&
        slot.Session == request.SessionEpoch && slot.Id == request.EndpointGeneration;
}

inline bool SameStream(const Slot& slot, const MOIREN_VAD_PCM_HEADER& request)
{
    return slot.Id == request.EndpointId && slot.Session == request.SessionEpoch &&
        slot.Id == request.EndpointGeneration &&
        slot.StreamGeneration == request.StreamGeneration &&
        slot.AdmissionGeneration == request.GateEpoch && slot.GateOpen != 0 &&
        slot.State == MOIREN_VAD_STATE_PRESENT && slot.AudioReady != 0 &&
        slot.StreamState == MOIREN_VAD_STREAM_RUNNING;
}

inline bool CanCommitGateOpen(const Slot& slot, const MOIREN_VAD_GATE_REQUEST& request)
{
    return SameIdentity(slot, request.Endpoint) &&
        slot.AdmissionGeneration == request.GateEpoch &&
        slot.AdmissionGeneration != ~uint64_t{0} &&
        slot.State == MOIREN_VAD_STATE_PRESENT &&
        slot.PolicyPrepared != 0 && slot.AudioReady != 0;
}

inline bool ValidWriteSequence(const Slot& slot, const MOIREN_VAD_PCM_HEADER& request)
{
    return request.Qpc != 0 && (!slot.HasSequence || request.Sequence > slot.LastSequence);
}

inline bool ValidPcmShape(const MOIREN_VAD_PCM_HEADER& header)
{
    return header.Size == sizeof(header) && header.Version == MOIREN_VAD_ABI_VERSION &&
        header.Reserved == 0 && header.Frames != 0 &&
        header.Frames <= MOIREN_VAD_MAX_PACKET_FRAMES &&
        (header.Channels == 1 || header.Channels == 2) &&
        (header.SampleFormat == MOIREN_VAD_FORMAT_PCM16 ||
            header.SampleFormat == MOIREN_VAD_FORMAT_FLOAT32);
}

// Call only after ValidPcmShape; all factors are bounded before multiplication.
inline uint32_t PcmBytes(const MOIREN_VAD_PCM_HEADER& header)
{
    return header.Frames * header.Channels *
        (header.SampleFormat == MOIREN_VAD_FORMAT_PCM16 ? 2u : 4u);
}
}
