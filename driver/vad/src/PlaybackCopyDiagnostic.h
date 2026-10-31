// SPDX-License-Identifier: MIT
#pragma once
#ifndef MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC
#define MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC 0
#endif
#if MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC
#include "Lifecycle.h"

namespace vad {
// Explicit diagnostic build for the owned deterministic fixture ONLY. Unlike
// ordinary diagnostics this retains three paired <=16-byte PCM prefixes, not
// arbitrary continuous audio. It cannot identify or authorize source content.
struct PlaybackCopyIdentity {
    uint64_t Endpoint, Session, Generation, Gate;
    uint32_t Format, Channels, Frames, BlockAlign;

    static PlaybackCopyIdentity From(const Slot& slot, uint32_t frames, uint32_t align)
    {
        return {slot.Id, slot.Session, slot.StreamGeneration, slot.AdmissionGeneration,
            slot.SampleFormat, slot.Channels, frames, align};
    }
    bool Same(const PlaybackCopyIdentity& other) const
    {
        return Endpoint == other.Endpoint && Session == other.Session &&
            Generation == other.Generation && Gate == other.Gate &&
            Format == other.Format && Channels == other.Channels &&
            Frames == other.Frames && BlockAlign == other.BlockAlign;
    }
    bool Valid() const
    {
        return Endpoint != 0 && Session != 0 && Generation != 0 && Gate != 0 &&
            (Format == MOIREN_VAD_FORMAT_PCM16 || Format == MOIREN_VAD_FORMAT_FLOAT32) &&
            (Channels == 1 || Channels == 2) && Frames != 0 &&
            Frames <= MOIREN_VAD_MAX_PACKET_FRAMES && BlockAlign <= 8 &&
            BlockAlign == Channels * (Format == MOIREN_VAD_FORMAT_PCM16 ? 2u : 4u);
    }
};

struct PlaybackCopyTime { uint64_t Qpc, Frequency; };
enum class PlaybackCopySeal : uint32_t { None, Capacity, IdentityChanged, NotAdmitted };
struct PlaybackCopyPrefix {
    uint64_t Sequence, CallbackQpc, BeforeQpc, AfterQpc;
    uint32_t FrameOffset, PacketByteOffset, MdlByteOffset, PacketInspectedFrames, Bytes;
    unsigned char Source[16], Destination[16];
    bool Observed;

    uint32_t Word(bool destination, uint32_t index) const
    {
        const auto bytes = destination ? Destination : Source;
        uint32_t word = 0;
        for (uint32_t i = 0; i < 4; ++i) {
            word |= static_cast<uint32_t>(bytes[index * 4 + i]) << (i * 8);
        }
        return word;
    }
};

// Shared->Lock protects this value. It is reset only when CREATE reserves the
// slot, never at stream acquire/resume: a new epoch cannot rescue old evidence.
// Enabled work is <=4800*8*2 volatile byte reads per callback, <=480000 frames
// lifetime. Its cost is unknown, not negligible or a latency measurement.
struct PlaybackCopyDiagnostic {
    static constexpr uint32_t FrameCap = 480000;
    PlaybackCopyIdentity Identity;
    uint64_t Frequency;
    uint32_t Revision, InspectedFrames, InspectedBytes;
    uint32_t SourceNonzeroBytes, DestinationNonzeroBytes;
    uint32_t SourceNonzeroFrames, DestinationNonzeroFrames, DifferentBytes;
    PlaybackCopyPrefix First, FirstNonzero, FirstMismatch;
    PlaybackCopySeal Seal;
    bool Observed, InterimExported, SealedExported;

    void Reset() { *this = {}; }
    void SealChanged(const PlaybackCopyIdentity& identity)
    {
        if (Observed && Seal == PlaybackCopySeal::None && !Identity.Same(identity)) {
            Seal = PlaybackCopySeal::IdentityChanged;
            ++Revision;
        }
    }
    template<class Clock>
    void Observe(const PlaybackCopyIdentity& identity, const void* source,
        const void* destination, uint64_t sequence, uint64_t callbackQpc, Clock clock)
    {
        SealChanged(identity);
        if (Seal != PlaybackCopySeal::None || !identity.Valid()) { return; }
        if (!Observed) { Identity = identity; Observed = true; }
        const auto before = clock();
        Frequency = before.Frequency;
        const uint32_t remaining = FrameCap - InspectedFrames;
        const uint32_t frames = identity.Frames < remaining ? identity.Frames : remaining;
        const auto from = static_cast<const volatile unsigned char*>(source);
        const auto to = static_cast<const volatile unsigned char*>(destination);
        PlaybackCopyPrefix* prefixes[] = {&First, &FirstNonzero, &FirstMismatch};
        bool started[3] = {};
        for (uint32_t frame = 0; frame < frames; ++frame) {
            unsigned char a[8] = {}, b[8] = {};
            bool sourceNonzero = false, destinationNonzero = false, different = false;
            const uint32_t offset = frame * identity.BlockAlign;
            // Actual volatile paired readback AFTER RtlCopyMemory. A producer
            // can still change source concurrently; this is NOT before-copy
            // source evidence, native-ring ingress, or whole-waveform proof.
            for (uint32_t byte = 0; byte < identity.BlockAlign; ++byte) {
                a[byte] = from[offset + byte]; b[byte] = to[offset + byte];
                SourceNonzeroBytes += a[byte] != 0;
                DestinationNonzeroBytes += b[byte] != 0;
                DifferentBytes += a[byte] != b[byte];
                sourceNonzero |= a[byte] != 0;
                destinationNonzero |= b[byte] != 0;
                different |= a[byte] != b[byte];
            }
            SourceNonzeroFrames += sourceNonzero;
            DestinationNonzeroFrames += destinationNonzero;
            const bool choose[] = {true, sourceNonzero || destinationNonzero, different};
            for (uint32_t kind = 0; kind < 3; ++kind) {
                auto& item = *prefixes[kind];
                if (!item.Observed && choose[kind]) {
                    item.Sequence = sequence; item.CallbackQpc = callbackQpc;
                    item.BeforeQpc = before.Qpc; item.FrameOffset = InspectedFrames + frame;
                    item.PacketByteOffset = offset;
                    item.MdlByteOffset = sizeof(MOIREN_VAD_PCM_HEADER) + offset;
                    item.PacketInspectedFrames = frames;
                    item.Observed = started[kind] = true;
                }
                if (started[kind] && item.Bytes + identity.BlockAlign <= 16) {
                    for (uint32_t byte = 0; byte < identity.BlockAlign; ++byte) {
                        item.Source[item.Bytes] = a[byte];
                        item.Destination[item.Bytes++] = b[byte];
                    }
                }
            }
        }
        const auto after = clock();
        for (uint32_t kind = 0; kind < 3; ++kind) {
            if (started[kind]) { prefixes[kind]->AfterQpc = after.Qpc; }
        }
        InspectedFrames += frames;
        InspectedBytes += frames * identity.BlockAlign;
        if (InspectedFrames == FrameCap) { Seal = PlaybackCopySeal::Capacity; }
        ++Revision;
    }
    void SealCurrent(const PlaybackCopyIdentity& current, bool admitted)
    {
        SealChanged(current);
        if (Observed && Seal == PlaybackCopySeal::None && !admitted) {
            Seal = PlaybackCopySeal::NotAdmitted;
            ++Revision;
        }
    }
    bool Take(PlaybackCopyDiagnostic& result)
    {
        if (!Observed) { return false; }
        auto& emitted = Seal == PlaybackCopySeal::None ? InterimExported : SealedExported;
        if (emitted) { return false; }
        emitted = true;
        result = *this;
        return true;
    }
};
static_assert(sizeof(PlaybackCopyDiagnostic) <= 1024, "Fixed per-slot diagnostic bound");
}
#endif
