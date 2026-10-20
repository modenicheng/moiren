// Copyright (c) Microsoft Corporation. All rights reserved.
// Portions Copyright (c) 2026 Moiren contributors.
// SPDX-License-Identifier: MS-PL
// Packet lifecycle is based on ACX Samples/Common/StreamEngine.cpp, with
// bounded scheduling and closed-gate silence for Moiren.
#include "Private.h"

static_assert(vad::audio::EndOfStream == KSSTREAM_HEADER_OPTIONSF_ENDOFSTREAM);

class LifecycleGuard {
public:
    explicit LifecycleGuard(VAD_AUDIO_DEVICE* device) : Device(device)
    {
        VadAudioLock(Device);
    }
    ~LifecycleGuard() { VadAudioUnlock(Device); }
    LifecycleGuard(const LifecycleGuard&) = delete;
    LifecycleGuard& operator=(const LifecycleGuard&) = delete;
private:
    VAD_AUDIO_DEVICE* Device;
};

static VAD_PAGE void FreeStorage(PACX_RTPACKET packets, PVOID* buffers, ULONG count)
{
    PAGED_CODE();
    if (packets == nullptr) {
        return;
    }
    for (ULONG i = 0; i < count; ++i) {
        if (packets[i].RtPacketBuffer.u.MdlType.Mdl != nullptr) {
            auto mdl = packets[i].RtPacketBuffer.u.MdlType.Mdl;
            if (buffers[i] != nullptr) {
                MmUnmapLockedPages(buffers[i], mdl);
            }
            MmFreePagesFromMdl(mdl);
            ExFreePool(mdl);
        }
    }
    ExFreePoolWithTag(packets, VadAudioTag);
}

static PVOID PacketAddress(VAD_AUDIO_STREAM* stream, ULONG index)
{
    auto address = static_cast<PUCHAR>(stream->Buffers[index]);
    return index == 0 ? address + stream->Packets[0].RtPacketOffset : address;
}

static void ZeroPackets(VAD_AUDIO_STREAM* stream)
{
    if (stream->Packets != nullptr) {
        for (ULONG i = 0; i < stream->PacketCount; ++i) {
            RtlZeroMemory(PacketAddress(stream, i), stream->PacketSize);
        }
    }
}

// Lifecycle held; the caller has already stopped and drained the timer.
_IRQL_requires_(PASSIVE_LEVEL)
static void FreePacketStorage(VAD_AUDIO_STREAM* stream)
{
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    PVOID buffers[VadAudioPacketCount] = {};
    KIRQL irql;
    KeAcquireSpinLock(&stream->State, &irql);
    const auto packets = stream->Packets;
    const auto count = stream->PacketCount;
    for (ULONG i = 0; i < count; ++i) {
        buffers[i] = stream->Buffers[i];
        stream->Buffers[i] = nullptr;
    }
    stream->Packets = nullptr;
    stream->PacketCount = 0;
    stream->PacketSize = 0;
    stream->PacketFrames = 0;
    stream->AllocationBytes = 0;
    KeReleaseSpinLock(&stream->State, irql);
    VadAudioDescribeStream(stream->Device, stream->Generation,
        stream->BlockAlign, 0, MOIREN_VAD_STREAM_PAUSED);
    // Packet/position callbacks see the detached state; freeing MDLs never
    // happens under the spin lock or while a timer can still notify ACX.
    FreeStorage(packets, buffers, count);
}

_Use_decl_annotations_
void VadAudioStopLocked(VAD_AUDIO_STREAM* stream, bool reset)
{
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    KIRQL irql;
    KeAcquireSpinLock(&stream->State, &irql);
    vad::audio::PauseClock(stream->Clock,
        static_cast<uint64_t>(KeQueryPerformanceCounter(nullptr).QuadPart));
    stream->Running = FALSE;
    KeReleaseSpinLock(&stream->State, irql);
    // All callers hold Lifecycle, including FreeRtPackets and PnP power-down.
    // The DPC never acquires Lifecycle, so this also drains its ACX notification.
    if (stream->Timer != nullptr) {
        (void)WdfTimerStop(stream->Timer, TRUE);
    }
    KeAcquireSpinLock(&stream->State, &irql);
    if (reset) {
        // Shared by ReleaseHardware, FreeRtPackets and cleanup. The epoch must
        // change before the PCM purge below, even if polling misses this stop.
        vad::audio::ResetPacketClock(*stream, [&] {
            VadPcmDiscontinuity(stream->Device, stream->Generation, true);
        });
    }
    stream->RenderReadyMask = 0;
    stream->DeliveredPackets = stream->CompletedPackets;
    ZeroPackets(stream);
    KeReleaseSpinLock(&stream->State, irql);
    VadAudioDescribeStream(stream->Device, stream->Generation,
        stream->BlockAlign, stream->PacketFrames, MOIREN_VAD_STREAM_PAUSED);
    auto child = VadChild(stream->Device);
    VadPcmPurge(child->Shared, child->Index, true);
}

VAD_PAGE void VadAudioStopDeviceLocked(VAD_AUDIO_DEVICE* device)
{
    PAGED_CODE();
    if (device->Stream != nullptr) {
        VadAudioStopLocked(VadAudioStream(device->Stream), false);
    }
}

static void Schedule(VAD_AUDIO_STREAM* stream, uint64_t now)
{
    if (stream->Timer == nullptr || !stream->Running) {
        return;
    }
    const auto schedule = vad::audio::SchedulePacket(stream->Clock,
        stream->CompletedPackets, stream->PacketFrames, now);
    (void)WdfTimerStart(stream->Timer, schedule.Due);
}

_Use_decl_annotations_
VOID VadAudioTimer(WDFTIMER timer)
{
    VAD_PCM_BATCH batch = {};
    auto timerContext = VadAudioTimerContext(timer);
    auto stream = timerContext->Stream;
    if (stream == nullptr) {
        return;
    }
    KIRQL irql;
    KeAcquireSpinLock(&stream->State, &irql);
    if (!stream->Running || stream->Packets == nullptr || stream->PacketCount == 0) {
        KeReleaseSpinLock(&stream->State, irql);
        return;
    }
    const auto now = static_cast<uint64_t>(KeQueryPerformanceCounter(nullptr).QuadPart);
    if (!VadAudioIsStreamCurrent(stream->Device, stream->Generation)) {
        vad::audio::PauseClock(stream->Clock, now);
        stream->Running = FALSE;
        ZeroPackets(stream);
        stream->DeliveredPackets = stream->CompletedPackets;
        KeReleaseSpinLock(&stream->State, irql);
        return;
    }
    // Do not complete early even if a timer implementation wakes prematurely.
    if (vad::audio::RelativeDue(stream->Clock,
        (stream->CompletedPackets + 1) * stream->PacketFrames, now) < 0) {
        Schedule(stream, now);
        KeReleaseSpinLock(&stream->State, irql);
        return;
    }
    const ULONG index = static_cast<ULONG>(stream->CompletedPackets % stream->PacketCount);
    const auto packet = stream->CompletedPackets;
    ++stream->CompletedPackets;
    const auto schedule = vad::audio::SchedulePacket(stream->Clock,
        stream->CompletedPackets, stream->PacketFrames, now);
    stream->LastCaptureStart = schedule.CompletedStartQpc;
    if (schedule.Rebased) {
        // Invalidate queued old-epoch PCM before it can reach a resumed packet.
        VadPcmDiscontinuity(stream->Device, stream->Generation);
        stream->RenderReadyMask = 0;
        ZeroPackets(stream);
    }
    auto buffer = PacketAddress(stream, index);
    if (stream->Capture) {
        RtlZeroMemory(buffer, stream->PacketSize);
        VadPcmTransfer(stream->Device, stream->Generation, buffer,
            stream->PacketFrames, stream->BlockAlign, packet, now, &batch);
    } else {
        const ULONG ready = 1u << index;
        if ((stream->RenderReadyMask & ready) != 0) {
            if (stream->HasEos && stream->EosPacket == static_cast<ULONG>(packet)) {
                RtlZeroMemory(static_cast<PUCHAR>(buffer) + stream->EosLength,
                    stream->PacketSize - stream->EosLength);
            }
            VadPcmTransfer(stream->Device, stream->Generation, buffer,
                stream->PacketFrames, stream->BlockAlign, packet, now, &batch);
        }
        // Discard after the bounded copy. A reused/underflow packet is silence.
        RtlZeroMemory(buffer, stream->PacketSize);
        stream->RenderReadyMask &= ~ready;
    }
    (void)WdfTimerStart(stream->Timer, schedule.Due);
    KeReleaseSpinLock(&stream->State, irql);
    VadPcmComplete(&batch);
    (void)AcxRtStreamNotifyPacketComplete(stream->Stream, packet, now);
}

// WDF deletes children before the stream's cleanup. Its timer cleanup is called
// only after all timer callbacks have returned. Clear the borrowed handle here
// so stream cleanup (and PnP) never stops an already deleted timer.
_Use_decl_annotations_
VAD_PAGE VOID VadAudioTimerCleanup(WDFOBJECT object)
{
    PAGED_CODE();
    auto timer = VadAudioTimerContext((WDFTIMER)object);
    auto context = timer->Stream;
    if (context == nullptr) { // Explicit deletion detached it under Lifecycle.
        return;
    }
    LifecycleGuard lifecycle(VadAudioDevice(context->Device));
    if (context->Timer == (WDFTIMER)object) {
        context->Timer = nullptr;
        context->Prepared = FALSE;
        VadAudioStopLocked(context, false);
    }
    timer->Stream = nullptr;
}

static VAD_PAGE void DeleteTimerLocked(VAD_AUDIO_STREAM* stream)
{
    PAGED_CODE();
    if (stream->Timer != nullptr) {
        auto timer = stream->Timer;
        VadAudioTimerContext(timer)->Stream = nullptr;
        stream->Timer = nullptr;
        WdfObjectDelete(timer);
    }
    stream->Prepared = FALSE;
}

static VAD_PAGE bool StreamCurrent(VAD_AUDIO_STREAM* stream)
{
    PAGED_CODE();
    const auto device = VadAudioDevice(stream->Device);
    return device->HardwareReady && device->Added && stream->AdmissionOwned &&
        VadAudioIsStreamCurrent(stream->Device, stream->Generation);
}

_Use_decl_annotations_
VAD_PAGE NTSTATUS VadAudioStreamPrepare(ACXSTREAM stream)
{
    PAGED_CODE();
    auto context = VadAudioStream(stream);
    LifecycleGuard lifecycle(VadAudioDevice(context->Device));
    if (!StreamCurrent(context) || context->Packets == nullptr) {
        return STATUS_DEVICE_NOT_READY;
    }
    if (context->Prepared) {
        return STATUS_SUCCESS;
    }
    WDF_TIMER_CONFIG timerConfig;
    WDF_TIMER_CONFIG_INIT(&timerConfig, VadAudioTimer);
    timerConfig.AutomaticSerialization = FALSE;
    timerConfig.UseHighResolutionTimer = WdfTrue;
    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, VAD_AUDIO_TIMER);
    attributes.ParentObject = stream;
    attributes.ExecutionLevel = WdfExecutionLevelDispatch;
    attributes.EvtCleanupCallback = VadAudioTimerCleanup;
    NTSTATUS status = WdfTimerCreate(&timerConfig, &attributes, &context->Timer);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    VadAudioTimerContext(context->Timer)->Stream = context;
    context->Prepared = TRUE;
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
VAD_PAGE NTSTATUS VadAudioStreamRelease(ACXSTREAM stream)
{
    PAGED_CODE();
    auto context = VadAudioStream(stream);
    LifecycleGuard lifecycle(VadAudioDevice(context->Device));
    VadAudioStopLocked(context, true);
    DeleteTimerLocked(context);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS VadAudioStreamRun(ACXSTREAM stream)
{
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    auto context = VadAudioStream(stream);
    auto device = VadAudioDevice(context->Device);
    LifecycleGuard lifecycle(device);
    if (!StreamCurrent(context) || !device->Powered || !context->Prepared ||
        context->Timer == nullptr || context->Packets == nullptr ||
        context->PacketCount == 0) {
        return STATUS_INVALID_DEVICE_STATE;
    }
    KIRQL irql;
    KeAcquireSpinLock(&context->State, &irql);
    if (context->Running) {
        KeReleaseSpinLock(&context->State, irql);
        return STATUS_SUCCESS;
    }
    const auto now = static_cast<uint64_t>(KeQueryPerformanceCounter(nullptr).QuadPart);
    const bool resume = context->Started != FALSE;
    vad::audio::ResumeClock(context->Clock, now);
    context->Running = TRUE;
    context->Started = TRUE;
    context->DeliveredPackets = context->CompletedPackets;
    if (resume) {
        VadPcmDiscontinuity(context->Device, context->Generation);
        context->RenderReadyMask = 0;
        ZeroPackets(context); // Discard stale pre-pause PCM, retain first preroll.
    }
    VadAudioDescribeStream(context->Device, context->Generation,
        context->BlockAlign, context->PacketFrames, MOIREN_VAD_STREAM_RUNNING);
    Schedule(context, now);
    KeReleaseSpinLock(&context->State, irql);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
VAD_PAGE NTSTATUS VadAudioStreamPause(ACXSTREAM stream)
{
    PAGED_CODE();
    auto context = VadAudioStream(stream);
    LifecycleGuard lifecycle(VadAudioDevice(context->Device));
    VadAudioStopLocked(context, false);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
VAD_PAGE NTSTATUS VadAudioGetLatency(ACXSTREAM stream, PULONG fifo, PULONG delay)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(stream);
    *fifo = 0;
    *delay = 0;
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS VadAudioSetRenderPacket(ACXSTREAM stream, ULONG packet, ULONG flags,
    ULONG eosLength)
{
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    auto context = VadAudioStream(stream);
    KIRQL irql;
    KeAcquireSpinLock(&context->State, &irql);
    NTSTATUS status;
    if (context->Capture || context->Packets == nullptr || context->PacketCount == 0 ||
        !VadAudioIsStreamCurrent(context->Device, context->Generation)) {
        status = STATUS_INVALID_DEVICE_STATE;
    } else {
        const auto result = vad::audio::ValidateRenderPacket(
            context->CompletedPackets, context->PacketCount, context->PacketSize,
            context->BlockAlign, context->Started != FALSE,
            context->HasEos != FALSE, packet, flags, eosLength);
        switch (result) {
        case vad::audio::RenderPacketResult::Accepted:
            status = STATUS_SUCCESS;
            break;
        case vad::audio::RenderPacketResult::Invalid:
            status = STATUS_INVALID_PARAMETER;
            break;
        case vad::audio::RenderPacketResult::Ended:
            status = STATUS_INVALID_DEVICE_STATE;
            break;
        case vad::audio::RenderPacketResult::Late:
            status = STATUS_DATA_LATE_ERROR;
            break;
        default:
            status = STATUS_DATA_OVERRUN;
            break;
        }
    }
    if (NT_SUCCESS(status)) {
        context->RenderReadyMask |= 1u << (packet % context->PacketCount);
        if ((flags & KSSTREAM_HEADER_OPTIONSF_ENDOFSTREAM) != 0) {
            context->HasEos = TRUE;
            context->EosPacket = packet;
            context->EosLength = eosLength;
        }
    }
    KeReleaseSpinLock(&context->State, irql);
    return status;
}

_Use_decl_annotations_
NTSTATUS VadAudioGetCurrentPacket(ACXSTREAM stream, PULONG current)
{
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    auto context = VadAudioStream(stream);
    KIRQL irql;
    KeAcquireSpinLock(&context->State, &irql);
    *current = static_cast<ULONG>(context->CompletedPackets);
    KeReleaseSpinLock(&context->State, irql);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS VadAudioGetCapturePacket(ACXSTREAM stream, PULONG last,
    PULONGLONG qpc, PBOOLEAN more)
{
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    auto context = VadAudioStream(stream);
    *last = 0;
    *qpc = 0;
    *more = FALSE;
    KIRQL irql;
    KeAcquireSpinLock(&context->State, &irql);
    uint32_t packet = 0;
    NTSTATUS status = STATUS_DEVICE_NOT_READY;
    if (context->Capture && context->Packets != nullptr &&
        VadAudioIsStreamCurrent(context->Device, context->Generation) &&
        vad::audio::TakeCapturePacket(context->CompletedPackets,
            context->DeliveredPackets, packet)) {
        *last = packet;
        *qpc = context->LastCaptureStart;
        status = STATUS_SUCCESS;
    }
    KeReleaseSpinLock(&context->State, irql);
    return status;
}

_Use_decl_annotations_
NTSTATUS VadAudioGetPosition(ACXSTREAM stream, PULONGLONG blocks,
    PULONGLONG qpc)
{
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    auto context = VadAudioStream(stream);
    KIRQL irql;
    KeAcquireSpinLock(&context->State, &irql);
    const auto now = static_cast<uint64_t>(KeQueryPerformanceCounter(nullptr).QuadPart);
    const auto frames = vad::audio::Position(context->Clock, now);
    *blocks = frames;
    *qpc = now;
    KeReleaseSpinLock(&context->State, irql);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
VAD_PAGE VOID VadAudioStreamCleanup(WDFOBJECT object)
{
    PAGED_CODE();
    auto context = VadAudioStream((ACXSTREAM)object);
    // ACX may clean up a failed AcxRtStreamCreate before it returns to us.
    if (context->Device == nullptr) {
        return;
    }
    auto device = VadAudioDevice(context->Device);
    LifecycleGuard lifecycle(device);
    VadAudioStopLocked(context, true);
    DeleteTimerLocked(context);
    // ACX normally calls FreeRtPackets first. Keep cleanup idempotent for a
    // failed stream setup or an ACX teardown path that has no packet callback.
    FreePacketStorage(context);
    if (device->Stream == context->Stream) {
        device->Stream = nullptr;
    }
    if (context->AdmissionOwned) {
        context->AdmissionOwned = FALSE;
        VadAudioReleaseStream(context->Device);
    }
}

_Use_decl_annotations_
VOID VadAudioStreamDestroy(WDFOBJECT object)
{
    UNREFERENCED_PARAMETER(object);
}

_Use_decl_annotations_
VAD_PAGE NTSTATUS VadAudioCreateStream(WDFDEVICE child, ACXCIRCUIT circuit, ACXPIN pin,
    PACXSTREAM_INIT streamInit, ACXDATAFORMAT format, const GUID* mode,
    ACXOBJECTBAG arguments)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(arguments);
    if (mode == nullptr || (!IsEqualGUID(*mode, AUDIO_SIGNALPROCESSINGMODE_RAW) &&
        !IsEqualGUID(*mode, AUDIO_SIGNALPROCESSINGMODE_DEFAULT))) {
        return STATUS_NOT_SUPPORTED;
    }
    auto device = VadAudioDevice(child);
    LifecycleGuard lifecycle(device);
    if (!device->HardwareReady || !device->Added) {
        return STATUS_DEVICE_NOT_READY;
    }
    if (pin != device->HostPin || circuit != device->Circuit) {
        return STATUS_INVALID_PARAMETER;
    }
    ULONG blockAlign = 0;
    NTSTATUS status = VadAudioValidateFormat(device, format, &blockAlign);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    LARGE_INTEGER frequency;
    KeQueryPerformanceCounter(&frequency);
    if (frequency.QuadPart <= 0 ||
        !vad::audio::ValidFrequency(static_cast<uint64_t>(frequency.QuadPart))) {
        return STATUS_NOT_SUPPORTED;
    }
    uint64_t generation = 0;
    status = VadAudioAcquireStream(child, &generation);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, VAD_AUDIO_STREAM);
    attributes.EvtCleanupCallback = VadAudioStreamCleanup;
    attributes.EvtDestroyCallback = VadAudioStreamDestroy;
    attributes.ExecutionLevel = WdfExecutionLevelPassive;
    ACX_STREAM_CALLBACKS callbacks;
    ACX_STREAM_CALLBACKS_INIT(&callbacks);
    callbacks.EvtAcxStreamPrepareHardware = VadAudioStreamPrepare;
    callbacks.EvtAcxStreamReleaseHardware = VadAudioStreamRelease;
    callbacks.EvtAcxStreamRun = VadAudioStreamRun;
    callbacks.EvtAcxStreamPause = VadAudioStreamPause;
    status = AcxStreamInitAssignAcxStreamCallbacks(streamInit, &callbacks);
    if (!NT_SUCCESS(status)) {
        VadAudioReleaseStream(child);
        return status;
    }
    ACX_RT_STREAM_CALLBACKS rt;
    ACX_RT_STREAM_CALLBACKS_INIT(&rt);
    rt.EvtAcxStreamGetHwLatency = VadAudioGetLatency;
    rt.EvtAcxStreamAllocateRtPackets = VadAudioAllocatePackets;
    rt.EvtAcxStreamFreeRtPackets = VadAudioFreePackets;
    rt.EvtAcxStreamGetCurrentPacket = VadAudioGetCurrentPacket;
    rt.EvtAcxStreamGetPresentationPosition = VadAudioGetPosition;
    if (device->Intent == MOIREN_VAD_INTENT_PLAYBACK) {
        rt.EvtAcxStreamSetRenderPacket = VadAudioSetRenderPacket;
    } else {
        rt.EvtAcxStreamGetCapturePacket = VadAudioGetCapturePacket;
    }
    status = AcxStreamInitAssignAcxRtStreamCallbacks(streamInit, &rt);
    if (!NT_SUCCESS(status)) {
        VadAudioReleaseStream(child);
        return status;
    }
    AcxStreamInitSetAcxRtStreamSupportsNotifications(streamInit);
    ACXSTREAM stream = nullptr;
    status = AcxRtStreamCreate(child, circuit, &attributes, &streamInit, &stream);
    if (!NT_SUCCESS(status)) {
        VadAudioReleaseStream(child);
        return status;
    }
    auto context = VadAudioStream(stream);
    context->Stream = stream;
    context->Generation = generation;
    context->BlockAlign = blockAlign;
    context->Capture = device->Intent == MOIREN_VAD_INTENT_MICROPHONE;
    context->AdmissionOwned = TRUE;
    KeInitializeSpinLock(&context->State);
    context->Clock.Frequency = frequency.QuadPart;
    context->Clock.Running = false;
    context->Device = child;
    device->Stream = stream;
    VadAudioDescribeStream(child, generation, blockAlign, 0,
        MOIREN_VAD_STREAM_PAUSED);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS VadAudioAllocatePackets(ACXSTREAM stream, ULONG count, ULONG packetSize,
    PACX_RTPACKET* output)
{
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    auto context = VadAudioStream(stream);
    LifecycleGuard lifecycle(VadAudioDevice(context->Device));
    *output = nullptr;
    if (!vad::audio::ValidPacketShape(count, packetSize, context->BlockAlign) ||
        packetSize > MAXULONG - (PAGE_SIZE - 1)) {
        return STATUS_INVALID_PARAMETER;
    }
    if (!StreamCurrent(context) || context->Packets != nullptr || context->Running) {
        return STATUS_INVALID_DEVICE_STATE;
    }
    const ULONG frames = packetSize / context->BlockAlign;
    const ULONG pages = (packetSize + PAGE_SIZE - 1) / PAGE_SIZE;
    const ULONG allocation = pages * PAGE_SIZE;
    // The admitted event-driven path always owns exactly two packet descriptors.
    const SIZE_T bytes = VadAudioPacketCount * sizeof(ACX_RTPACKET);
    auto packets = static_cast<PACX_RTPACKET>(ExAllocatePool2(
        POOL_FLAG_NON_PAGED, bytes, VadAudioTag));
    if (packets == nullptr) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    RtlZeroMemory(packets, bytes);
    PVOID buffers[VadAudioPacketCount] = {};
    for (ULONG i = 0; i < VadAudioPacketCount && i < count; ++i) {
        PHYSICAL_ADDRESS low = {};
        PHYSICAL_ADDRESS high;
        high.QuadPart = MAXLONGLONG;
        auto mdl = MmAllocatePagesForMdlEx(low, high, low, allocation,
            MmCached, MM_ALLOCATE_FULLY_REQUIRED);
        if (mdl == nullptr) {
            FreeStorage(packets, buffers, count);
            return STATUS_INSUFFICIENT_RESOURCES;
        }
        ACX_RTPACKET_INIT(&packets[i]);
        WDF_MEMORY_DESCRIPTOR_INIT_MDL(&packets[i].RtPacketBuffer, mdl,
            MmGetMdlByteCount(mdl));
        auto buffer = MmGetSystemAddressForMdlSafe(mdl,
            NormalPagePriority | MdlMappingNoExecute);
        if (buffer == nullptr) {
            FreeStorage(packets, buffers, count);
            return STATUS_INSUFFICIENT_RESOURCES;
        }
        packets[i].RtPacketSize = packetSize;
        packets[i].RtPacketOffset = 0;
        buffers[i] = buffer;
        RtlZeroMemory(buffer, allocation);
    }
    // Publish only fully allocated storage. The packet callbacks take State
    // rather than Lifecycle and must never observe partial MDL construction.
    KIRQL irql;
    KeAcquireSpinLock(&context->State, &irql);
    context->Packets = packets;
    context->AllocationBytes = allocation;
    context->PacketSize = packetSize;
    context->PacketFrames = frames;
    context->PacketCount = count;
    context->Clock.LimitFrames = frames;
    for (ULONG i = 0; i < count; ++i) {
        context->Buffers[i] = buffers[i];
    }
    KeReleaseSpinLock(&context->State, irql);
    VadAudioDescribeStream(context->Device, context->Generation,
        context->BlockAlign, frames, MOIREN_VAD_STREAM_PAUSED);
    *output = packets;
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
VAD_PAGE VOID VadAudioFreePackets(ACXSTREAM stream, PACX_RTPACKET packets, ULONG count)
{
    PAGED_CODE();
    auto context = VadAudioStream(stream);
    LifecycleGuard lifecycle(VadAudioDevice(context->Device));
    UNREFERENCED_PARAMETER(count);
    if (packets != nullptr && packets == context->Packets) {
        VadAudioStopLocked(context, true);
        FreePacketStorage(context);
    }
}
