// SPDX-License-Identifier: MIT
#pragma once
#include "PlaybackCopyDiagnostic.h"
#if MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC
namespace vad {
enum class NativePlaybackPath : uint32_t { TimerCopy = 1, EventRead = 2 };
enum class NativePlaybackSeal : uint32_t { None, Capacity, IdentityChanged, NotAdmitted, LayoutChanged };
struct NativePlaybackLayout {
    NativePlaybackPath Path;
    uint32_t PacketCount, PacketSize, PacketFrames, AllocationBytes, MappedBytes, MappedFrames;
    uint32_t RtPacketSize[2], RtPacketOffset[2];
    bool Same(const NativePlaybackLayout& other) const
    {
        return Path == other.Path && PacketCount == other.PacketCount &&
            PacketSize == other.PacketSize && PacketFrames == other.PacketFrames &&
            AllocationBytes == other.AllocationBytes && MappedBytes == other.MappedBytes &&
            MappedFrames == other.MappedFrames && RtPacketSize[0] == other.RtPacketSize[0] &&
            RtPacketSize[1] == other.RtPacketSize[1] && RtPacketOffset[0] == other.RtPacketOffset[0] &&
            RtPacketOffset[1] == other.RtPacketOffset[1];
    }
};
struct NativePlaybackScan {
    uint64_t SegmentSequence, LogicalSequence, StartFrame, CallbackQpc, BeforeQpc, AfterQpc;
    uint32_t NativeOffset, DestinationOffset, Frames, Index, ConsumedFrames;
    uint32_t EosFlags, EosPacket, EosLength;
};
struct NativePlaybackPrefix {
    // Scan offsets point to this prefix's first selected frame; Frames and
    // ConsumedFrames still describe the original observed span. A decoder must
    // bound prefix offsets using Bytes, not offset + Frames * BlockAlign.
    NativePlaybackScan Scan;
    uint32_t FrameOffset, Bytes;
    unsigned char Native[16], Destination[16];
    bool Observed;
    uint32_t Word(bool destination, uint32_t index) const
    {
        const auto bytes = destination ? Destination : Native;
        uint32_t word = 0;
        for (uint32_t i = 0; i < 4; ++i) { word |= uint32_t{bytes[index * 4 + i]} << (i * 8); }
        return word;
    }
};

// Owned deterministic fixture ONLY: paired <=16B PCM witnesses are private
// opt-in evidence. State is under the existing Shared lock, reset only CREATE.
// Timer reads are AFTER memcpy / BEFORE clear; not a before-copy snapshot.
// Event reads sample the same live buffer ONCE, not an independently copied
// destination. Neither State nor Shared excludes concurrent mapped producers.
struct NativePlaybackDiagnostic {
    static constexpr uint32_t FrameCap = 480000;
    PlaybackCopyIdentity Identity;
    NativePlaybackLayout Layout;
    uint64_t Frequency;
    PlaybackCopyScan SelectingRead;
    uint32_t Revision, InspectedFrames, InspectedBytes;
    uint32_t NativeNonzeroBytes, DestinationNonzeroBytes, NativeNonzeroFrames;
    uint32_t DestinationNonzeroFrames, DifferentBytes, Calls, SequenceGaps;
    NativePlaybackScan FirstScan, LastScan;
    NativePlaybackPrefix First, FirstNonzero, FirstMismatch;
    NativePlaybackSeal Seal;
    bool Observed, InterimExported, SealedExported, CounterSaturated;

    void Reset() { *this = {}; }
    void Increment(uint32_t& value)
    {
        if (value == ~uint32_t{0}) { CounterSaturated = true; }
        else { ++value; }
    }
    void SealCurrent(const PlaybackCopyIdentity& current, bool admitted)
    {
        if (Observed && Seal == NativePlaybackSeal::None) {
            if (!Identity.Same(current)) { Seal = NativePlaybackSeal::IdentityChanged; }
            else if (!admitted) { Seal = NativePlaybackSeal::NotAdmitted; }
            if (Seal != NativePlaybackSeal::None) { ++Revision; }
        }
    }
    template<class Clock>
    void Observe(const PlaybackCopyIdentity& identity, const NativePlaybackLayout& layout,
        NativePlaybackScan scan, const void* native, const void* destination, Clock clock)
    {
        SealCurrent(identity, true);
        if (Seal != NativePlaybackSeal::None || !identity.Valid()) { return; }
        if (Observed && !Layout.Same(layout)) {
            Seal = NativePlaybackSeal::LayoutChanged; ++Revision; return;
        }
        const bool timer = layout.Path == NativePlaybackPath::TimerCopy;
        const uint64_t bytes = static_cast<uint64_t>(scan.Frames) * identity.BlockAlign;
        const uint32_t extent = timer ? layout.MappedBytes : layout.AllocationBytes;
        if ((!timer && layout.Path != NativePlaybackPath::EventRead) ||
            layout.PacketFrames != identity.Frames ||
            layout.PacketSize != identity.Frames * identity.BlockAlign ||
            scan.Frames == 0 || scan.Frames > MOIREN_VAD_MAX_PACKET_FRAMES ||
            layout.PacketCount != (timer ? 1u : 2u) || scan.Index >= layout.PacketCount ||
            scan.NativeOffset % identity.BlockAlign != 0 ||
            scan.DestinationOffset % identity.BlockAlign != 0 ||
            scan.NativeOffset > extent || bytes > extent - scan.NativeOffset ||
            (timer && (scan.DestinationOffset > layout.PacketSize ||
                bytes > layout.PacketSize - scan.DestinationOffset)) ||
            native == nullptr || destination == nullptr) { return; }
        if (!Observed) { Identity = identity; Layout = layout; Observed = true; }
        const auto before = clock();
        Frequency = before.Frequency;
        scan.BeforeQpc = before.Qpc;
        scan.ConsumedFrames = scan.Frames;
        const auto remaining = FrameCap - InspectedFrames;
        if (scan.Frames > remaining) { scan.Frames = remaining; }
        const auto from = static_cast<const volatile unsigned char*>(native);
        const auto to = static_cast<const volatile unsigned char*>(destination);
        NativePlaybackPrefix* prefixes[] = {&First, &FirstNonzero, &FirstMismatch};
        bool started[3] = {};
        for (uint32_t frame = 0; frame < scan.Frames; ++frame) {
            unsigned char a[8] = {}, b[8] = {};
            bool nz = false, dz = false, mismatch = false;
            const auto offset = frame * identity.BlockAlign;
            for (uint32_t byte = 0; byte < identity.BlockAlign; ++byte) {
                a[byte] = from[offset + byte];
                b[byte] = timer ? to[offset + byte] : a[byte];
                NativeNonzeroBytes += a[byte] != 0; DestinationNonzeroBytes += b[byte] != 0;
                DifferentBytes += a[byte] != b[byte];
                nz |= a[byte] != 0; dz |= b[byte] != 0; mismatch |= a[byte] != b[byte];
            }
            NativeNonzeroFrames += nz; DestinationNonzeroFrames += dz;
            const bool choose[] = {true, nz || dz, mismatch};
            for (uint32_t kind = 0; kind < 3; ++kind) {
                auto& p = *prefixes[kind];
                if (!p.Observed && choose[kind]) {
                    p.Scan = scan; p.FrameOffset = InspectedFrames + frame;
                    p.Scan.NativeOffset += offset; p.Scan.DestinationOffset += offset;
                    p.Observed = started[kind] = true;
                }
                if (started[kind] && p.Bytes + identity.BlockAlign <= 16) {
                    for (uint32_t byte = 0; byte < identity.BlockAlign; ++byte) {
                        p.Native[p.Bytes] = a[byte]; p.Destination[p.Bytes++] = b[byte];
                    }
                }
            }
        }
        scan.AfterQpc = clock().Qpc;
        if (Calls == 0) { FirstScan = scan; }
        else {
            const bool adjacent = timer ?
                LastScan.StartFrame <= ~uint64_t{0} - LastScan.Frames &&
                    scan.StartFrame == LastScan.StartFrame + LastScan.Frames :
                LastScan.LogicalSequence != ~uint64_t{0} &&
                    scan.LogicalSequence == LastScan.LogicalSequence + 1;
            if (!adjacent) { Increment(SequenceGaps); }
        }
        LastScan = scan; Increment(Calls);
        for (uint32_t kind = 0; kind < 3; ++kind) {
            if (started[kind]) { prefixes[kind]->Scan.AfterQpc = scan.AfterQpc; }
        }
        InspectedFrames += scan.Frames; InspectedBytes += scan.Frames * identity.BlockAlign;
        if (InspectedFrames == FrameCap) { Seal = NativePlaybackSeal::Capacity; }
        ++Revision;
    }
    bool Take(NativePlaybackDiagnostic& result)
    {
        if (!Observed) { return false; }
        auto& emitted = Seal == NativePlaybackSeal::None ? InterimExported : SealedExported;
        if (emitted) { return false; }
        emitted = true; result = *this; return true;
    }
};
static_assert(sizeof(NativePlaybackDiagnostic) <= 1024, "Fixed native per-slot diagnostic bound");
}
#endif
