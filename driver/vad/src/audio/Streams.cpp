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

static PVOID TimerRingAddress(VAD_AUDIO_STREAM* stream, ULONG frame)
{
    auto offset = static_cast<SIZE_T>(frame) * stream->BlockAlign;
    return static_cast<PUCHAR>(stream->Buffers[0]) + offset;
}

static void ZeroPackets(VAD_AUDIO_STREAM* stream)
{
    if (stream->PacketCount == 1 && stream->Buffers[0] != nullptr) {
        RtlZeroMemory(stream->Buffers[0], stream->TimerRing.MappedBytes);
        return;
    }
    if (stream->Packets != nullptr) {
        for (ULONG i = 0; i < stream->PacketCount; ++i) {
            RtlZeroMemory(PacketAddress(stream, i), stream->PacketSize);
        }
    }
}

static void ZeroTimerRingSpan(VAD_AUDIO_STREAM* stream,
    const vad::audio::TimerDrivenSegment& segment)
{
    if (segment.FirstFrames != 0) {
        RtlZeroMemory(TimerRingAddress(stream, segment.RingFrameOffset),
            static_cast<SIZE_T>(segment.FirstFrames) * stream->BlockAlign);
    }
    if (segment.SecondFrames != 0) {
        RtlZeroMemory(TimerRingAddress(stream, 0),
            static_cast<SIZE_T>(segment.SecondFrames) * stream->BlockAlign);
    }
}

static void ClearTimerPacket(VAD_AUDIO_STREAM* stream)
{
    if (stream->TimerScratch != nullptr) {
        RtlZeroMemory(stream->TimerScratch, stream->TimerScratchBytes);
    }
    stream->TimerPacket = {};
}

// State is held. Erase every buffer byte and packet bookkeeping field a late
// reader could observe, so nothing from a previous admission, generation or
// epoch survives into the next one. The pause/resume pair (VadAudioStopLocked
// with reset=false and VadAudioStreamRun) instead preserves LastCaptureStart
// so a packet completed before the pause still reports its true capture time.
static void QuiesceStreamBuffers(VAD_AUDIO_STREAM* stream)
{
    ClearTimerPacket(stream);
    stream->RenderReadyMask = 0;
    stream->DeliveredPackets = stream->CompletedPackets;
    stream->LastCaptureStart = 0;
    ZeroPackets(stream);
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
    const auto scratch = stream->TimerScratch;
    stream->TimerScratch = nullptr;
    stream->TimerScratchBytes = 0;
    stream->TimerRing = {};
    stream->TimerPacket = {};
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
    if (scratch != nullptr) {
        ExFreePoolWithTag(scratch, VadAudioTag);
    }
}

_Use_decl_annotations_
void VadAudioStopLocked(VAD_AUDIO_STREAM* stream, bool reset)
{
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    KIRQL irql;
    KeAcquireSpinLock(&stream->State, &irql);
    if (stream->PacketCount == 1) {
        // Never expose timer-ring bytes which the DPC has not copied yet.
        stream->Clock.BaseFrames = stream->TimerRing.PresentedFrames;
        stream->Clock.Running = false;
    } else {
        vad::audio::PauseClock(stream->Clock,
            static_cast<uint64_t>(KeQueryPerformanceCounter(nullptr).QuadPart));
    }
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
        stream->TimerRing.RingFrameOffset = 0;
        stream->TimerRing.PresentedFrames = 0;
        stream->TimerRing.SegmentSequence = 0;
    }
    // Deliberately not QuiesceStreamBuffers: across a pause (reset=false) a
    // packet completed before the stop must still deliver its capture time, so
    // LastCaptureStart survives and the remaining fields are reset here.
    ClearTimerPacket(stream);
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
    if (stream->PacketCount == 1) {
        const auto schedule = vad::audio::ScheduleTimerDrivenQuantum(
            stream->Clock, stream->TimerRing, now);
        NT_ASSERT(schedule.Valid);
        if (schedule.Valid) {
            (void)WdfTimerStart(stream->Timer, schedule.Due);
        }
        return;
    }
    const auto schedule = vad::audio::SchedulePacket(stream->Clock,
        stream->CompletedPackets, stream->PacketFrames, now);
    (void)WdfTimerStart(stream->Timer, schedule.Due);
}

// State is held. The batch contains detached post-copy evidence, never an MDL.
static void RecordEventCaptureCopy(VAD_AUDIO_STREAM* stream, ULONG index,
    uint64_t sequence, uint64_t qpc, const VAD_PCM_BATCH& batch)
{
    if (!batch.CaptureCopied || batch.CapturePrefix.Bytes == 0) { return; }
    auto child = VadChild(stream->Device);
    auto shared = child->Shared;
    KIRQL irql;
    KeAcquireSpinLock(&shared->Lock, &irql);
    const auto& slot = shared->Slots[child->Index];
    if (slot.Id == child->Id && slot.StreamGeneration == stream->Generation) {
        shared->CaptureDiagnostics[child->Index].RecordCopy(stream->Generation,
            reinterpret_cast<uint64_t>(stream->Stream),
            {batch.CaptureGateEpoch, sequence, batch.CaptureRequestSequence, qpc,
                stream->PacketCount, stream->PacketSize, stream->Packets[index].RtPacketSize,
                stream->Packets[index].RtPacketOffset, 0, batch.CapturePrefix, true});
    }
    KeReleaseSpinLock(&shared->Lock, irql);
}

// State is held. The endpoint lock spans epoch validation and each bounded
// copy; gate closure therefore cannot acknowledge revocation while stale
// scratch is being copied. Requests retrieved here complete in this DPC.
static void TransferTimerSegment(VAD_AUDIO_STREAM* stream,
    const vad::audio::TimerDrivenSegment& segment, uint64_t now,
    VAD_PCM_BATCH* batch)
{
    auto child = VadChild(stream->Device);
    auto shared = child->Shared;
    if (batch->Shared == nullptr &&
        !ExAcquireRundownProtection(&shared->PcmRundown)) {
        ClearTimerPacket(stream);
        ZeroTimerRingSpan(stream, segment);
        return;
    }
    batch->Shared = shared;
    KIRQL irql;
    KeAcquireSpinLock(&shared->Lock, &irql);
    const auto& slot = shared->Slots[child->Index];
    const bool copied = vad::audio::TransferTimerSegment(slot, child->Id,
        stream->Generation, stream->Capture != FALSE, stream->TimerRing,
        segment, stream->TimerPacket,
        [&] { RtlZeroMemory(stream->TimerScratch, stream->TimerScratchBytes); },
        [&](const vad::audio::TimerDrivenCopySpan& span) {
            auto native = static_cast<PUCHAR>(stream->Buffers[0]) + span.RingByteOffset;
            auto scratch = static_cast<PUCHAR>(stream->TimerScratch) + span.PacketByteOffset;
            if (stream->Capture) {
                RtlCopyMemory(native, scratch, span.ByteCount);
                auto& diagnostic = shared->CaptureDiagnostics[child->Index];
                if (stream->TimerPacket.CaptureCopied && diagnostic.NeedsCopy()) {
                    diagnostic.RecordCopy(stream->Generation,
                        reinterpret_cast<uint64_t>(stream->Stream),
                        {stream->TimerPacket.GateEpoch, segment.PacketSequence,
                            stream->TimerPacket.RequestSequence, now, stream->PacketCount,
                            stream->PacketSize, stream->Packets[0].RtPacketSize,
                            span.RingByteOffset, span.PacketByteOffset,
                            vad::audio::CapturePrefix::ReadBack(scratch, native, span.ByteCount), true});
                }
            } else {
                RtlCopyMemory(scratch, native, span.ByteCount);
            }
        },
        [&](const vad::audio::TimerDrivenCopySpan& span) {
            RtlZeroMemory(static_cast<PUCHAR>(stream->Buffers[0]) +
                span.RingByteOffset, span.ByteCount);
        },
        [&] {
            VadPcmTransferLocked(shared, child->Index, stream->TimerScratch,
                stream->PacketFrames, stream->BlockAlign,
                segment.PacketSequence, now, batch);
            if (stream->Capture) {
                stream->TimerPacket.CaptureCopied = batch->CaptureCopied;
                stream->TimerPacket.RequestSequence = batch->CaptureRequestSequence;
            }
        });
    NT_ASSERT(copied);
    UNREFERENCED_PARAMETER(copied);
    KeReleaseSpinLock(&shared->Lock, irql);
}

// State is held. One callback consumes at most one logical packet of elapsed
// time, split only at logical/native boundaries. Short packets admit one timer
// quantum of residual debt and redue without relabeling PCM; longer stalls
// discard time. Ordinary jitter must not repeatedly erase incomplete PCM.
_IRQL_requires_(DISPATCH_LEVEL)
static void TimerDrivenTick(VAD_AUDIO_STREAM* stream, uint64_t now,
    VAD_PCM_BATCH* batch)
{
    const auto clockBefore = stream->Clock;
    const auto schedule = vad::audio::PlanTimerDrivenStep(
        stream->Clock, stream->TimerRing, now);
    if (!schedule.Valid) {
        stream->Running = FALSE;
        QuiesceStreamBuffers(stream);
        return;
    }
    if (schedule.Rebased) {
        const auto observation = vad::audio::ObserveClockRebase(
            vad::audio::ClockRebaseReason::TimerStall, clockBefore,
            stream->Clock, now, stream->PacketFrames, stream->PacketCount,
            stream->CompletedPackets, stream->TimerRing.PresentedFrames,
            vad::audio::TimerDrivenPendingLimit(stream->TimerRing));
        VadAudioRebaseDiscontinuity(stream->Device, stream->Generation,
            reinterpret_cast<uint64_t>(stream->Stream), observation);
        QuiesceStreamBuffers(stream);
    }
    ULONG remaining = schedule.Frames;
    // A budget no larger than one packet crosses at most one logical and one
    // native boundary. Three spans and one PCM completion are absolute bounds.
    for (unsigned spans = 0; remaining != 0 && spans < 3; ++spans) {
        vad::audio::TimerDrivenSegment segment = {};
        if (!vad::audio::NextTimerDrivenSegment(stream->TimerRing, segment, remaining)) {
            stream->Running = FALSE;
            QuiesceStreamBuffers(stream);
            return;
        }
        remaining -= segment.Frames;
        TransferTimerSegment(stream, segment, now, batch);
    }
    NT_ASSERT(remaining == 0);
    // Timer clients consume a rolling position, not an immutable native packet.
    // Publishing packet-complete here would advertise bytes already reused by
    // this interval, so packet queries and notifications are event-mode only.
    (void)WdfTimerStart(stream->Timer, schedule.Due);
}

_Use_decl_annotations_
VOID VadAudioTimer(WDFTIMER timer)
{
    const auto dpcQpc = static_cast<uint64_t>(KeQueryPerformanceCounter(nullptr).QuadPart);
    VAD_PCM_BATCH batch = {};
    auto timerContext = VadAudioTimerContext(timer);
    auto stream = timerContext->Stream;
    if (stream == nullptr) {
        return;
    }
    KIRQL irql;
    KeAcquireSpinLock(&stream->State, &irql);
    const auto stateQpc = static_cast<uint64_t>(KeQueryPerformanceCounter(nullptr).QuadPart);
    if (!stream->Running || stream->Packets == nullptr || stream->PacketCount == 0) {
        KeReleaseSpinLock(&stream->State, irql);
        return;
    }
    const auto now = static_cast<uint64_t>(KeQueryPerformanceCounter(nullptr).QuadPart);
    batch.Callback = {dpcQpc, stateQpc,
        stream->PacketCount == 1 ? 0 : vad::audio::FrameQpc(stream->Clock,
            (stream->CompletedPackets + 1) * stream->PacketFrames),
        reinterpret_cast<uint64_t>(stream->Stream), stream->Clock.Frequency};
    if (!VadAudioIsStreamCurrent(stream->Device, stream->Generation)) {
        vad::audio::PauseClock(stream->Clock, now);
        stream->Running = FALSE;
        QuiesceStreamBuffers(stream);
        KeReleaseSpinLock(&stream->State, irql);
        return;
    }
    if (stream->PacketCount == 1) {
        TimerDrivenTick(stream, now, &batch);
        KeReleaseSpinLock(&stream->State, irql);
        VadPcmComplete(&batch);
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
    const auto clockBefore = stream->Clock;
    const auto schedule = vad::audio::SchedulePacket(stream->Clock,
        stream->CompletedPackets, stream->PacketFrames, now);
    stream->LastCaptureStart = schedule.CompletedStartQpc;
    if (schedule.Rebased) {
        // Invalidate queued old-epoch PCM before it can reach a resumed packet.
        const auto observation = vad::audio::ObserveClockRebase(
            vad::audio::ClockRebaseReason::EventDeadline, clockBefore,
            stream->Clock, now, stream->PacketFrames, stream->PacketCount,
            stream->CompletedPackets, stream->TimerRing.PresentedFrames);
        VadAudioRebaseDiscontinuity(stream->Device, stream->Generation,
            reinterpret_cast<uint64_t>(stream->Stream), observation);
        stream->RenderReadyMask = 0;
        ZeroPackets(stream);
    }
    auto buffer = PacketAddress(stream, index);
    if (stream->Capture) {
        RtlZeroMemory(buffer, stream->PacketSize);
        VadPcmTransfer(stream->Device, stream->Generation, buffer,
            stream->PacketFrames, stream->BlockAlign, packet, now, &batch);
        RecordEventCaptureCopy(stream, index, packet, now, batch);
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

// Gate callers have dropped Shared->Lock. Lifecycle keeps a borrowed stream
// alive while State excludes the DPC. No native packet or scratch from a
// previous admission may survive a close/open transaction.
_Use_decl_annotations_
void VadAudioClearGateBuffers(WDFDEVICE child)
{
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    auto device = VadAudioDevice(child);
    LifecycleGuard lifecycle(device);
    if (device->Stream == nullptr) {
        return;
    }
    auto stream = VadAudioStream(device->Stream);
    KIRQL irql;
    KeAcquireSpinLock(&stream->State, &irql);
    QuiesceStreamBuffers(stream);
    KeReleaseSpinLock(&stream->State, irql);
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
        // Like VadAudioStopLocked above, LastCaptureStart is preserved so a
        // pre-pause completion still reports its capture time.
        context->RenderReadyMask = 0;
        ZeroPackets(context); // Discard stale pre-pause PCM, retain first preroll.
        ClearTimerPacket(context);
    }
    VadAudioDescribeStream(context->Device, context->Generation,
        context->BlockAlign, context->PacketFrames, MOIREN_VAD_STREAM_RUNNING);
    VadAudioRecordRun(context->Device, context->Generation, now);
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
    } else if (!vad::audio::SupportsPacketNotifications(context->PacketCount)) {
        status = STATUS_NOT_SUPPORTED;
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
    if (context->PacketCount != 0 &&
        !vad::audio::SupportsPacketNotifications(context->PacketCount)) {
        *current = 0;
        KeReleaseSpinLock(&context->State, irql);
        return STATUS_NOT_SUPPORTED;
    }
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
    if (context->PacketCount != 0 &&
        !vad::audio::SupportsPacketNotifications(context->PacketCount)) {
        status = STATUS_NOT_SUPPORTED;
    } else if (context->Capture && context->Packets != nullptr &&
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
    const auto frames = context->PacketCount == 1 ?
        context->TimerRing.PresentedFrames : vad::audio::Position(context->Clock, now);
    *blocks = frames;
    *qpc = now;
    if (context->Capture && context->Packets != nullptr && context->PacketCount != 0) {
        auto child = VadChild(context->Device);
        KIRQL sharedIrql;
        KeAcquireSpinLock(&child->Shared->Lock, &sharedIrql);
        const auto& slot = child->Shared->Slots[child->Index];
        auto& diagnostic = child->Shared->CaptureDiagnostics[child->Index];
        if (slot.Id == child->Id && slot.StreamGeneration == context->Generation) {
            diagnostic.RecordPosition(context->Generation,
                reinterpret_cast<uint64_t>(context->Stream),
                {slot.AdmissionGeneration, frames, now, context->PacketCount, true});
        }
        KeReleaseSpinLock(&child->Shared->Lock, sharedIrql);
    }
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
    vad::audio::TimerDrivenRing timerRing = {};
    if (count == 1 && !vad::audio::InitializeTimerDrivenRing(timerRing,
        packetSize, context->BlockAlign, PAGE_SIZE)) {
        return STATUS_INVALID_PARAMETER;
    }
    // Fixed descriptor capacity also makes the allocation bound visible to the
    // WDK analyzer; only `count` initialized descriptors are returned to ACX.
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
        // ACX double-maps timer packets into user space. Both ends of the
        // descriptor must be page aligned, not just the underlying MDL.
        packets[i].RtPacketSize = count == 1 ? allocation : packetSize;
        packets[i].RtPacketOffset = vad::audio::RtPacketOffset(
            count, i, packetSize, allocation);
        buffers[i] = buffer;
        RtlZeroMemory(buffer, allocation);
    }
    PVOID scratch = nullptr;
    if (count == 1) {
        scratch = ExAllocatePool2(POOL_FLAG_NON_PAGED, packetSize, VadAudioTag);
        if (scratch == nullptr) {
            FreeStorage(packets, buffers, count);
            return STATUS_INSUFFICIENT_RESOURCES;
        }
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
    context->TimerRing = timerRing;
    context->TimerScratch = scratch;
    context->TimerScratchBytes = count == 1 ? packetSize : 0;
    context->TimerPacket = {};
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
