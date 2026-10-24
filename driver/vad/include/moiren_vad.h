// SPDX-License-Identifier: MIT
// Fixed-width little-endian C ABI. No native pointers or handles.
#pragma once
#include <stddef.h>
#if defined(MOIREN_VAD_KERNEL)
#include <ntdef.h>
typedef UINT16 uint16_t;
typedef UINT32 uint32_t;
typedef UINT64 uint64_t;
#else
#include <stdint.h>
#endif

#define MOIREN_VAD_ABI_VERSION 2u
#define MOIREN_VAD_MAX_ENDPOINTS 64u
#define MOIREN_VAD_MAX_STREAMS_PER_PIN 1u
#define MOIREN_VAD_SAMPLE_RATE 48000u
#define MOIREN_VAD_REQUEST_SLOTS 4u
#define MOIREN_VAD_MAX_PACKET_FRAMES 4800u
#define MOIREN_VAD_MAX_NAME_UNITS 64u
#define MOIREN_VAD_MAX_PCM_BYTES (MOIREN_VAD_MAX_PACKET_FRAMES * 2u * 4u)
#define MOIREN_VAD_CONTROL_INTERFACE_GUID \
    {0x4c7ef487, 0x61a1, 0x4a21, {0x8d, 0x63, 0x9d, 0x45, 0x11, 0x5a, 0x7a, 0x5b}}

// FILE_DEVICE_UNKNOWN, FILE_READ_DATA | FILE_WRITE_DATA, BUFFERED control.
#define IOCTL_MOIREN_VAD_CAPABILITIES 0x0022e000u
#define IOCTL_MOIREN_VAD_CREATE       0x0022e004u
#define IOCTL_MOIREN_VAD_QUERY        0x0022e008u
#define IOCTL_MOIREN_VAD_REMOVE       0x0022e00cu
#define IOCTL_MOIREN_VAD_SET_GATE     0x0022e010u
// OUT_DIRECT render PCM (kernel -> user), IN_DIRECT capture PCM (user -> kernel).
#define IOCTL_MOIREN_VAD_READ_PCM     0x0022e016u
#define IOCTL_MOIREN_VAD_WRITE_PCM    0x0022e019u

#define MOIREN_VAD_INTENT_PLAYBACK   1u
#define MOIREN_VAD_INTENT_MICROPHONE 2u
#define MOIREN_VAD_STATE_PRESENT    1u
#define MOIREN_VAD_STATE_REMOVING   2u
#define MOIREN_VAD_STATE_REMOVED    3u
#define MOIREN_VAD_STATE_CREATING   4u
#define MOIREN_VAD_CAP_RAW_PDO       0x00000001u
#define MOIREN_VAD_CAP_POLICY_STAGED 0x00000002u
#define MOIREN_VAD_CAP_AUDIO         0x00000004u
#define MOIREN_VAD_CAP_STREAMS       0x00000008u
#define MOIREN_VAD_CAP_PCM           0x00000010u
#define MOIREN_VAD_CAP_CREATE_DEFERRED 0x00000020u
#define MOIREN_VAD_CREATE_DEFERRED_KIND 5u // CREATE reply tag, never a PDO state.
#define MOIREN_VAD_FORMAT_PCM16      0x00000001u
#define MOIREN_VAD_FORMAT_FLOAT32    0x00000002u
#define MOIREN_VAD_STREAM_CLOSED     0u
#define MOIREN_VAD_STREAM_PAUSED     1u
#define MOIREN_VAD_STREAM_RUNNING    2u

typedef struct MOIREN_VAD_HEADER {
    uint32_t Size;
    uint32_t Version;
} MOIREN_VAD_HEADER;

// Independent of NodeId, MMDevice identifiers and runtime/session tokens.
typedef struct MOIREN_VAD_ENDPOINT_ID {
    uint64_t Low;
    uint64_t High;
} MOIREN_VAD_ENDPOINT_ID;

typedef struct MOIREN_VAD_CREATE_REQUEST {
    uint32_t Size;
    uint32_t Version;
    uint32_t Intent;
    uint32_t Channels;
    MOIREN_VAD_ENDPOINT_ID StableId;
    uint32_t AllowDefault; // Local preference, never project authorization.
    uint32_t Reserved;
    uint16_t Name[MOIREN_VAD_MAX_NAME_UNITS]; // NUL-terminated UTF-16.
} MOIREN_VAD_CREATE_REQUEST;

typedef struct MOIREN_VAD_ENDPOINT_REQUEST {
    uint32_t Size;
    uint32_t Version;
    uint64_t EndpointId; // Opaque runtime token, not StableId.
    uint64_t SessionEpoch;
    uint64_t EndpointGeneration;
} MOIREN_VAD_ENDPOINT_REQUEST;

typedef struct MOIREN_VAD_GATE_REQUEST {
    MOIREN_VAD_ENDPOINT_REQUEST Endpoint;
    uint32_t Open;
    uint32_t Reserved;
    uint64_t GateEpoch; // Must match the current snapshot, including close.
} MOIREN_VAD_GATE_REQUEST;

typedef struct MOIREN_VAD_CAPABILITIES {
    uint32_t Size;
    uint32_t Version;
    uint32_t Flags;
    uint32_t MaxEndpoints;
    uint32_t MaxStreamsPerPin;
    uint32_t TargetSampleRate;
    uint32_t SupportedSampleFormats;
    uint32_t ChannelMask;
    uint32_t IntentMask;
    uint32_t RequestSlots; // Maximum pending requests, not required concurrency.
    uint64_t SessionEpoch;
    uint32_t MaxPacketFrames;
    uint32_t Reserved[3];
} MOIREN_VAD_CAPABILITIES;

typedef struct MOIREN_VAD_ENDPOINT_SNAPSHOT {
    uint32_t Size;
    uint32_t Version;
    uint64_t EndpointId;
    uint32_t State;
    uint32_t Intent;
    uint32_t Channels;
    uint32_t GateOpen;
    uint32_t PolicyPrepared;
    uint32_t AudioReady;
    uint64_t SessionEpoch;
    uint64_t EndpointGeneration;
    uint64_t StreamGeneration;
    uint64_t GateEpoch;
    MOIREN_VAD_ENDPOINT_ID StableId;
    uint32_t SampleFormat; // Zero until a native stream opens.
    uint32_t PacketFrames; // Zero until native packet allocation.
    uint32_t LastStatus; // NTSTATUS bits from asynchronous PnP preparation.
    uint32_t StreamState;
    uint64_t DroppedPackets;
    uint64_t SilencePackets;
    uint64_t Reserved;
} MOIREN_VAD_ENDPOINT_SNAPSHOT;

// CREATE-only alternative to the 128-byte endpoint snapshot, returned with
// STATUS_SUCCESS only for a pre-reservation old-session revoked-ID collision.
// This request created no endpoint. SessionEpoch is the current owner session;
// EndpointId and all Reserved fields are zero, so this reply conveys no key.
typedef struct MOIREN_VAD_CREATE_DEFERRED {
    uint32_t Size;
    uint32_t Version;
    uint64_t EndpointId;
    uint32_t Kind; // MOIREN_VAD_CREATE_DEFERRED_KIND at the snapshot State offset.
    uint32_t Intent;
    uint32_t Channels;
    uint32_t Reserved0[3]; // No gate, policy or audio readiness.
    uint64_t SessionEpoch;
    uint64_t Reserved1[3]; // No endpoint, stream or gate generation.
    MOIREN_VAD_ENDPOINT_ID StableId;
    uint64_t Reserved2[5]; // No stream shape, status, counters or extension data.
} MOIREN_VAD_CREATE_DEFERRED;

// Buffered header separate from the MDL payload. READ output is this header
// followed by PCM; WRITE's direct buffer contains PCM only. Frames must equal
// current PacketFrames. Up to four pending requests, no additional kernel PCM ring.
// WRITE freshness is checked at consumption against one native period; the
// capacity does not authorize queueing four periods of microphone payload.
typedef struct MOIREN_VAD_PCM_HEADER {
    uint32_t Size; // 80, excludes PCM bytes.
    uint32_t Version;
    uint64_t EndpointId;
    uint64_t SessionEpoch;
    uint64_t EndpointGeneration;
    uint64_t StreamGeneration;
    uint64_t GateEpoch;
    uint64_t Sequence;
    uint64_t Qpc;
    uint32_t SampleFormat;
    uint32_t Frames;
    uint32_t Channels;
    uint32_t Reserved;
} MOIREN_VAD_PCM_HEADER;

#ifdef __cplusplus
static_assert(sizeof(MOIREN_VAD_HEADER) == 8);
static_assert(sizeof(MOIREN_VAD_ENDPOINT_ID) == 16);
static_assert(sizeof(MOIREN_VAD_CREATE_REQUEST) == 168);
static_assert(sizeof(MOIREN_VAD_ENDPOINT_REQUEST) == 32);
static_assert(sizeof(MOIREN_VAD_GATE_REQUEST) == 48);
static_assert(sizeof(MOIREN_VAD_CAPABILITIES) == 64);
static_assert(sizeof(MOIREN_VAD_ENDPOINT_SNAPSHOT) == 128);
static_assert(sizeof(MOIREN_VAD_CREATE_DEFERRED) == 128);
static_assert(sizeof(MOIREN_VAD_PCM_HEADER) == 80);
#define VAD_OFFSET(type, field, offset) static_assert(offsetof(type, field) == offset)
VAD_OFFSET(MOIREN_VAD_CREATE_REQUEST, StableId, 16);
VAD_OFFSET(MOIREN_VAD_CREATE_REQUEST, AllowDefault, 32);
VAD_OFFSET(MOIREN_VAD_CREATE_REQUEST, Name, 40);
VAD_OFFSET(MOIREN_VAD_ENDPOINT_REQUEST, SessionEpoch, 16);
VAD_OFFSET(MOIREN_VAD_ENDPOINT_REQUEST, EndpointGeneration, 24);
VAD_OFFSET(MOIREN_VAD_GATE_REQUEST, Open, 32);
VAD_OFFSET(MOIREN_VAD_GATE_REQUEST, GateEpoch, 40);
VAD_OFFSET(MOIREN_VAD_CAPABILITIES, SessionEpoch, 40);
VAD_OFFSET(MOIREN_VAD_CAPABILITIES, MaxPacketFrames, 48);
VAD_OFFSET(MOIREN_VAD_ENDPOINT_SNAPSHOT, SessionEpoch, 40);
VAD_OFFSET(MOIREN_VAD_ENDPOINT_SNAPSHOT, EndpointGeneration, 48);
VAD_OFFSET(MOIREN_VAD_ENDPOINT_SNAPSHOT, StreamGeneration, 56);
VAD_OFFSET(MOIREN_VAD_ENDPOINT_SNAPSHOT, GateEpoch, 64);
VAD_OFFSET(MOIREN_VAD_ENDPOINT_SNAPSHOT, StableId, 72);
VAD_OFFSET(MOIREN_VAD_ENDPOINT_SNAPSHOT, SampleFormat, 88);
VAD_OFFSET(MOIREN_VAD_ENDPOINT_SNAPSHOT, DroppedPackets, 104);
VAD_OFFSET(MOIREN_VAD_CREATE_DEFERRED, Size, 0);
VAD_OFFSET(MOIREN_VAD_CREATE_DEFERRED, Version, 4);
VAD_OFFSET(MOIREN_VAD_CREATE_DEFERRED, EndpointId, 8);
VAD_OFFSET(MOIREN_VAD_CREATE_DEFERRED, Kind, 16);
VAD_OFFSET(MOIREN_VAD_CREATE_DEFERRED, Intent, 20);
VAD_OFFSET(MOIREN_VAD_CREATE_DEFERRED, Channels, 24);
VAD_OFFSET(MOIREN_VAD_CREATE_DEFERRED, Reserved0, 28);
VAD_OFFSET(MOIREN_VAD_CREATE_DEFERRED, SessionEpoch, 40);
VAD_OFFSET(MOIREN_VAD_CREATE_DEFERRED, Reserved1, 48);
VAD_OFFSET(MOIREN_VAD_CREATE_DEFERRED, StableId, 72);
VAD_OFFSET(MOIREN_VAD_CREATE_DEFERRED, Reserved2, 88);
VAD_OFFSET(MOIREN_VAD_PCM_HEADER, Sequence, 48);
VAD_OFFSET(MOIREN_VAD_PCM_HEADER, Qpc, 56);
VAD_OFFSET(MOIREN_VAD_PCM_HEADER, SampleFormat, 64);
VAD_OFFSET(MOIREN_VAD_PCM_HEADER, Frames, 68);
VAD_OFFSET(MOIREN_VAD_PCM_HEADER, Channels, 72);
#undef VAD_OFFSET
#endif
