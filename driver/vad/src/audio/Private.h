// SPDX-License-Identifier: MIT
#pragma once
#include "Audio.h"
#include "Clock.h"
#include "CircuitIdentity.h"
#include <windef.h>
#include <mmsystem.h>
#include <ks.h>
#include <ksmedia.h>

constexpr ULONG VadAudioHostPin = 0;
constexpr ULONG VadAudioBridgePin = 1;
constexpr ULONG VadAudioPacketCount = vad::audio::MaxPackets;
constexpr ULONG VadAudioTag = 'aVoM';

struct VAD_AUDIO_DEVICE {
    WDFDEVICE Device;
    // Embedded so WDF sibling destruction order cannot delete the lock before
    // a late stream cleanup. Only passive callbacks wait on it.
    KMUTEX Lifecycle;
    KSPIN_LOCK State;
    ACXCIRCUIT Circuit;
    ACXPIN HostPin;
    ACXSTREAM Stream; // Borrowed; cleared by stream cleanup under Lifecycle.
    vad::audio::CircuitIdentity Identity; // Immutable across Prepare/Release.
    uint32_t Intent;
    uint32_t Channels;
    BOOLEAN Added;
    BOOLEAN HardwareReady;
    BOOLEAN Powered;
};
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(VAD_AUDIO_DEVICE, VadAudioDevice)

struct VAD_AUDIO_STREAM {
    WDFDEVICE Device;
    ACXSTREAM Stream;
    WDFTIMER Timer;
    KSPIN_LOCK State; // DPC and passive position/packet snapshots.
    PACX_RTPACKET Packets;
    PVOID Buffers[VadAudioPacketCount];
    ULONG AllocationBytes;
    ULONG PacketCount;
    ULONG PacketSize;
    ULONG PacketFrames;
    ULONG BlockAlign;
    uint64_t Generation;
    vad::audio::Clock Clock;
    uint64_t CompletedPackets;
    uint64_t DeliveredPackets;
    uint64_t LastCaptureStart;
    ULONG RenderReadyMask;
    ULONG EosPacket;
    ULONG EosLength;
    BOOLEAN HasEos;
    BOOLEAN Started;
    BOOLEAN Capture;
    BOOLEAN Prepared;
    BOOLEAN AdmissionOwned;
    BOOLEAN Running;
};
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(VAD_AUDIO_STREAM, VadAudioStream)

struct VAD_AUDIO_TIMER {
    VAD_AUDIO_STREAM* Stream;
};
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(VAD_AUDIO_TIMER, VadAudioTimerContext)

inline void VadAudioLock(VAD_AUDIO_DEVICE* device)
{
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    KeWaitForSingleObject(&device->Lifecycle, Executive, KernelMode, FALSE, nullptr);
}

inline void VadAudioUnlock(VAD_AUDIO_DEVICE* device)
{
    KeReleaseMutex(&device->Lifecycle, FALSE);
}

// These helpers require Lifecycle. They never take it recursively.
_IRQL_requires_(PASSIVE_LEVEL)
void VadAudioStopLocked(VAD_AUDIO_STREAM* stream, bool reset);
VAD_PAGE void VadAudioStopDeviceLocked(VAD_AUDIO_DEVICE* device);
VAD_PAGE NTSTATUS VadAudioBuildCircuit(VAD_AUDIO_DEVICE* device);
VAD_PAGE NTSTATUS VadAudioValidateFormat(VAD_AUDIO_DEVICE* device, ACXDATAFORMAT format,
    ULONG* blockAlign);
VAD_PAGE NTSTATUS VadAudioAddFormats(VAD_AUDIO_DEVICE* device, ACXPIN host);

VAD_PAGE EVT_ACX_CIRCUIT_CREATE_STREAM VadAudioCreateStream;
VAD_PAGE EVT_WDF_OBJECT_CONTEXT_CLEANUP VadAudioStreamCleanup;
EVT_WDF_OBJECT_CONTEXT_DESTROY VadAudioStreamDestroy;
EVT_WDF_TIMER VadAudioTimer;
VAD_PAGE EVT_WDF_OBJECT_CONTEXT_CLEANUP VadAudioTimerCleanup;
EVT_ACX_STREAM_ALLOCATE_RTPACKETS VadAudioAllocatePackets;
VAD_PAGE EVT_ACX_STREAM_FREE_RTPACKETS VadAudioFreePackets;
VAD_PAGE EVT_ACX_STREAM_PREPARE_HARDWARE VadAudioStreamPrepare;
VAD_PAGE EVT_ACX_STREAM_RELEASE_HARDWARE VadAudioStreamRelease;
EVT_ACX_STREAM_RUN VadAudioStreamRun;
VAD_PAGE EVT_ACX_STREAM_PAUSE VadAudioStreamPause;
VAD_PAGE EVT_ACX_STREAM_GET_HW_LATENCY VadAudioGetLatency;
EVT_ACX_STREAM_SET_RENDER_PACKET VadAudioSetRenderPacket;
EVT_ACX_STREAM_GET_CURRENT_PACKET VadAudioGetCurrentPacket;
EVT_ACX_STREAM_GET_CAPTURE_PACKET VadAudioGetCapturePacket;
EVT_ACX_STREAM_GET_PRESENTATION_POSITION VadAudioGetPosition;
