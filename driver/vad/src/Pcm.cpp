// SPDX-License-Identifier: MIT
#include "Driver.h"

struct VAD_PCM_QUEUE {
    VAD_SHARED* Shared;
};
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(VAD_PCM_QUEUE, VadPcmQueue)
static EVT_WDF_IO_QUEUE_IO_CANCELED_ON_QUEUE CancelQueued;
static EVT_WDF_OBJECT_CONTEXT_DESTROY DestroyQueue;

static void DropCountLocked(VAD_PCM_REQUEST* context)
{
    if (InterlockedExchange(&context->Counted, 0) != 0) {
        auto& slot = context->Shared->Slots[context->Index];
        NT_ASSERT(slot.Id == context->Header.EndpointId && slot.PendingRequests != 0);
        if (slot.Id == context->Header.EndpointId && slot.PendingRequests != 0) {
            --slot.PendingRequests;
        }
    }
}

static void DropCount(VAD_PCM_REQUEST* context)
{
    KIRQL irql;
    KeAcquireSpinLock(&context->Shared->Lock, &irql);
    DropCountLocked(context);
    KeReleaseSpinLock(&context->Shared->Lock, irql);
}

static ULONG FindIndex(VAD_SHARED* shared, uint64_t id)
{
    ULONG index = MOIREN_VAD_MAX_ENDPOINTS;
    KIRQL irql;
    KeAcquireSpinLock(&shared->Lock, &irql);
    for (ULONG i = 0; i < MOIREN_VAD_MAX_ENDPOINTS; ++i) {
        if (id != 0 && shared->Slots[i].Id == id) {
            index = i;
            break;
        }
    }
    KeReleaseSpinLock(&shared->Lock, irql);
    return index;
}

static NTSTATUS ReserveRequest(VAD_SHARED* shared, VAD_PCM_REQUEST* context,
    ULONG index, bool capture)
{
    KIRQL irql;
    KeAcquireSpinLock(&shared->Lock, &irql);
    auto& slot = shared->Slots[index];
    const auto& header = context->Header;
    NTSTATUS status = STATUS_SUCCESS;
    if (InterlockedCompareExchange(&shared->PcmOnline, 0, 0) == 0) {
        status = STATUS_DEVICE_REMOVED;
    } else if (!vad::CanQueuePcm(slot, header)) {
        status = STATUS_REVISION_MISMATCH;
    } else if (slot.SampleFormat != header.SampleFormat ||
        slot.PacketFrames != header.Frames || slot.Channels != header.Channels ||
        slot.Intent != (capture ? MOIREN_VAD_INTENT_MICROPHONE : MOIREN_VAD_INTENT_PLAYBACK)) {
        status = STATUS_INVALID_PARAMETER;
    } else if (slot.PendingRequests >= MOIREN_VAD_REQUEST_SLOTS) {
        status = STATUS_QUOTA_EXCEEDED;
    } else if (capture && !vad::ValidWriteSequence(slot, header)) {
        status = STATUS_INVALID_PARAMETER;
    } else {
        context->Shared = shared;
        context->Index = index;
        context->Counted = 1;
        ++slot.PendingRequests;
        if (capture) {
            slot.LastSequence = header.Sequence;
            slot.HasSequence = 1;
            // A safe reservation-side pre-forward boundary, NOT actual queue
            // insertion or proof that forward succeeds. Never touch a request
            // after successful WdfRequestForwardToIoQueue: WDF may complete it.
            shared->UnservedDiagnostics[index].RecordPreForward(
                header.StreamGeneration, {header.GateEpoch, header.Sequence,
                    header.Qpc, static_cast<uint64_t>(
                        KeQueryPerformanceCounter(nullptr).QuadPart)});
        }
    }
    KeReleaseSpinLock(&shared->Lock, irql);
    return status;
}

_Use_decl_annotations_
static void CancelQueued(WDFQUEUE queue, WDFREQUEST request)
{
    UNREFERENCED_PARAMETER(queue);
    auto context = VadPcmRequest(request);
    DropCount(context);
    WdfRequestComplete(request, STATUS_CANCELLED);
}

_Use_decl_annotations_
static void DestroyQueue(WDFOBJECT object)
{
    auto context = VadPcmQueue(object);
    if (context->Shared != nullptr) {
        VadSharedRelease(context->Shared);
    }
}

VAD_PAGE NTSTATUS VadPcmInitialize(WDFDEVICE device)
{
    PAGED_CODE();
    auto shared = VadRoot(device)->Shared;
    ExInitializeRundownProtection(&shared->PcmRundown);
    shared->PcmInitialized = TRUE;
    shared->PcmOnline = 1;
    for (ULONG i = 0; i < MOIREN_VAD_MAX_ENDPOINTS; ++i) {
        KeInitializeMutex(&shared->PcmDrain[i], 0);
        WDF_IO_QUEUE_CONFIG config;
        WDF_IO_QUEUE_CONFIG_INIT(&config, WdfIoQueueDispatchManual);
        config.PowerManaged = WdfFalse;
        config.EvtIoCanceledOnQueue = CancelQueued;
        WDF_OBJECT_ATTRIBUTES attributes;
        WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, VAD_PCM_QUEUE);
        attributes.EvtDestroyCallback = DestroyQueue;
        attributes.SynchronizationScope = WdfSynchronizationScopeNone;
        NTSTATUS status = WdfIoQueueCreate(device, &config, &attributes,
            &shared->PcmQueues[i]);
        if (!NT_SUCCESS(status)) {
            return status;
        }
        InterlockedIncrement(&shared->References);
        VadPcmQueue(shared->PcmQueues[i])->Shared = shared;
    }
    return STATUS_SUCCESS;
}

VAD_PAGE void VadPcmPurge(VAD_SHARED* shared, ULONG index, bool restart)
{
    PAGED_CODE();
    if (!shared->PcmInitialized || !ExAcquireRundownProtection(&shared->PcmRundown)) {
        return;
    }
    KeWaitForSingleObject(&shared->PcmDrain[index], Executive, KernelMode, FALSE, nullptr);
    const auto queue = shared->PcmQueues[index];
    if (queue != nullptr) {
        // Called only outside ALL EvtIo callbacks (including other queues).
        // Cancellation never enters Admin, Lifecycle, or PcmDrain.
        WdfIoQueuePurgeSynchronously(queue);
        if (restart && InterlockedCompareExchange(&shared->PcmOnline, 0, 0) != 0) {
            WdfIoQueueStart(queue);
        }
    }
    KeReleaseMutex(&shared->PcmDrain[index], FALSE);
    ExReleaseRundownProtection(&shared->PcmRundown);
}

VAD_PAGE void VadPcmShutdown(VAD_SHARED* shared)
{
    PAGED_CODE();
    if (!shared->PcmInitialized || InterlockedExchange(&shared->PcmOnline, 0) == 0) {
        return;
    }
    // All PCM queue users, including DPC transfers and child-release purges,
    // finish before the root can clean up its WDF queues.
    ExWaitForRundownProtectionRelease(&shared->PcmRundown);
    for (const auto queue : shared->PcmQueues) {
        if (queue != nullptr) {
            WdfIoQueuePurgeSynchronously(queue);
        }
    }
}

VAD_PAGE NTSTATUS VadPcmSubmit(WDFDEVICE device, WDFREQUEST request, ULONG code)
{
    PAGED_CODE();
    auto shared = VadRoot(device)->Shared;
    WDF_REQUEST_PARAMETERS parameters;
    WDF_REQUEST_PARAMETERS_INIT(&parameters);
    WdfRequestGetParameters(request, &parameters);
    if (parameters.Parameters.DeviceIoControl.InputBufferLength != sizeof(MOIREN_VAD_PCM_HEADER)) {
        return STATUS_INFO_LENGTH_MISMATCH;
    }
    PVOID input = nullptr;
    NTSTATUS status = WdfRequestRetrieveInputBuffer(request,
        sizeof(MOIREN_VAD_PCM_HEADER), &input, nullptr);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    auto context = VadPcmRequest(request);
    RtlCopyMemory(&context->Header, input, sizeof(context->Header));
    const auto& header = context->Header;
    if (!vad::ValidPcmShape(header)) {
        return STATUS_INVALID_PARAMETER;
    }
    const ULONG bytes = vad::PcmBytes(header);
    const bool capture = code == IOCTL_MOIREN_VAD_WRITE_PCM;
    const ULONG length = bytes + (capture ? 0 : sizeof(header));
    if (parameters.Parameters.DeviceIoControl.OutputBufferLength != length) {
        return STATUS_INFO_LENGTH_MISMATCH;
    }
    PMDL mdl = nullptr;
    status = WdfRequestRetrieveOutputWdmMdl(request, &mdl);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    // Map once on the passive submission path, never in the audio DPC.
    context->Buffer = MmGetSystemAddressForMdlSafe(mdl,
        NormalPagePriority | MdlMappingNoExecute | (capture ? MdlMappingNoWrite : 0));
    if (context->Buffer == nullptr) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    if (!ExAcquireRundownProtection(&shared->PcmRundown)) {
        return STATUS_DEVICE_REMOVED;
    }
    const ULONG index = FindIndex(shared, header.EndpointId);
    if (index == MOIREN_VAD_MAX_ENDPOINTS) {
        ExReleaseRundownProtection(&shared->PcmRundown);
        return STATUS_NOT_FOUND;
    }
    // Serialize forwarding against passive queue drains. A stream retirement
    // first closes its state; either this request is queued before the drain,
    // or admission sees the retired stream after it.
    KeWaitForSingleObject(&shared->PcmDrain[index], Executive, KernelMode, FALSE, nullptr);
    status = ReserveRequest(shared, context, index, capture);
    if (NT_SUCCESS(status)) {
        // WDF owns cancellation while queued. A DPC retrieves at most four
        // requests, finishes bounded copying, then completes every retrieved IRP.
        status = WdfRequestForwardToIoQueue(request, shared->PcmQueues[index]);
        if (!NT_SUCCESS(status)) {
            DropCount(context);
        } else {
            status = STATUS_PENDING;
        }
    }
    KeReleaseMutex(&shared->PcmDrain[index], FALSE);
    ExReleaseRundownProtection(&shared->PcmRundown);
    return status;
}

void VadPcmDiscontinuity(WDFDEVICE child, uint64_t generation, bool pauseStream)
{
    auto context = VadChild(child);
    auto shared = context->Shared;
    KIRQL irql;
    KeAcquireSpinLock(&shared->Lock, &irql);
    auto& slot = shared->Slots[context->Index];
    if (slot.Id == context->Id && slot.StreamGeneration == generation) {
        vad::InvalidatePcmEpoch(slot, pauseStream);
    }
    KeReleaseSpinLock(&shared->Lock, irql);
}

void VadPcmTransfer(WDFDEVICE child, uint64_t generation, PVOID buffer,
    ULONG frames, ULONG blockAlign, uint64_t sequence, uint64_t qpc,
    VAD_PCM_BATCH* batch)
{
    NT_ASSERT(batch->Shared == nullptr && batch->Completions.Count == 0);
    auto childContext = VadChild(child);
    auto shared = childContext->Shared;
    if (!ExAcquireRundownProtection(&shared->PcmRundown)) {
        return;
    }
    batch->Shared = shared;
    KIRQL irql;
    KeAcquireSpinLock(&shared->Lock, &irql);
    auto& slot = shared->Slots[childContext->Index];
    if (slot.Id == childContext->Id && slot.StreamGeneration == generation) {
        VadPcmTransferLocked(shared, childContext->Index, buffer,
            frames, blockAlign, sequence, qpc, batch);
    }
    KeReleaseSpinLock(&shared->Lock, irql);
}

void VadPcmTransferLocked(VAD_SHARED* shared, ULONG index, PVOID buffer,
    ULONG frames, ULONG blockAlign, uint64_t sequence, uint64_t qpc,
    VAD_PCM_BATCH* batch)
{
    NT_ASSERT(batch->Shared == shared && batch->Completions.Count == 0);
    auto& slot = shared->Slots[index];
    bool copied = false;
    const bool capture = slot.Intent == MOIREN_VAD_INTENT_MICROPHONE;
    auto& diagnostic = shared->PcmDiagnostics[index];
    // Stack-only accumulation: a rejected WRITE is not an unserved callback
    // if a later request in the unchanged bounded loop supplies this packet.
    uint16_t unservedReasons = 0;
    uint16_t retrieveCalls = 0;
    NTSTATUS retrieveStatus = STATUS_SUCCESS;
    const ULONG bytes = frames * blockAlign;
    if (capture) {
        RtlZeroMemory(buffer, bytes);
    }
    if (slot.State == MOIREN_VAD_STATE_PRESENT && slot.AudioReady != 0) {
        // A native packet can satisfy only one request. Stale requests at the
        // head are completed with the epoch mismatch, but once the first
        // current request is found, leave remaining current requests queued
        // for later native packets.
        for (ULONG count = 0; count < MOIREN_VAD_REQUEST_SLOTS && !copied; ++count) {
            WDFREQUEST request = nullptr;
            retrieveStatus = WdfIoQueueRetrieveNextRequest(shared->PcmQueues[index],
                &request);
            ++retrieveCalls;
            if (!NT_SUCCESS(retrieveStatus)) {
                unservedReasons |= vad::UnservedEmpty;
                if (capture && slot.GateOpen != 0 &&
                    slot.StreamState == MOIREN_VAD_STREAM_RUNNING) {
                    diagnostic.Record({vad::PcmWriteOutcome::Empty,
                        slot.AdmissionGeneration, sequence, 0, 0, qpc, 0, 0, frames});
                }
                break;
            }
            auto context = VadPcmRequest(request);
            DropCountLocked(context);
            const bool appended = batch->Completions.Append(
                {request, STATUS_REVISION_MISMATCH, 0});
            NT_ASSERT(appended);
            UNREFERENCED_PARAMETER(appended);
            auto& completion = batch->Completions.Entries[count];
            auto& header = context->Header;
            if (!vad::SameStream(slot, header) ||
                slot.SampleFormat != header.SampleFormat || header.Frames != frames ||
                header.Channels != slot.Channels || bytes > MOIREN_VAD_MAX_PCM_BYTES) {
                unservedReasons |= vad::UnservedNoCurrent;
                continue;
            }
            if (capture) {
                LARGE_INTEGER frequency;
                const auto checked = KeQueryPerformanceCounter(&frequency);
                const uint64_t period = static_cast<uint64_t>(frequency.QuadPart) *
                    frames / MOIREN_VAD_SAMPLE_RATE;
                const auto outcome = vad::ClassifyPcmWrite(header.Qpc, qpc, period);
                diagnostic.Record({outcome, slot.AdmissionGeneration, sequence,
                    header.Sequence, header.Qpc, qpc,
                    static_cast<uint64_t>(checked.QuadPart), period, frames});
                if (outcome != vad::PcmWriteOutcome::Success) {
                    if (outcome == vad::PcmWriteOutcome::TooOld) {
                        unservedReasons |= vad::UnservedTooOld;
                    }
                    ++slot.DroppedPackets;
                    completion.Status = STATUS_IO_TIMEOUT;
                    continue;
                }
                RtlCopyMemory(buffer, context->Buffer, bytes);
                batch->CaptureCopied = true;
                batch->CaptureGateEpoch = slot.AdmissionGeneration;
                batch->CaptureRequestSequence = header.Sequence;
                if (shared->CaptureDiagnostics[index].NeedsCopy()) {
                    batch->CapturePrefix = vad::audio::CapturePrefix::ReadBack(
                        context->Buffer, buffer, bytes);
                }
                completion.Information = bytes;
            } else {
                header.Sequence = sequence;
                header.Qpc = qpc;
                RtlCopyMemory(context->Buffer, &header, sizeof(header));
                RtlCopyMemory(static_cast<PUCHAR>(context->Buffer) + sizeof(header), buffer, bytes);
                completion.Information = sizeof(header) + bytes;
            }
            completion.Status = STATUS_SUCCESS;
            copied = true;
        }
        if (!copied) {
            if (capture) {
                if (slot.GateOpen != 0 &&
                    slot.StreamState == MOIREN_VAD_STREAM_RUNNING) {
                    // Four retired requests can exhaust the bounded loop
                    // without reaching the empty-queue or freshness branch.
                    diagnostic.Record({vad::PcmWriteOutcome::NoCurrentRequest,
                        slot.AdmissionGeneration, sequence, 0, 0, qpc, 0, 0, frames});
                    shared->UnservedDiagnostics[index].RecordUnserved(
                        slot.StreamGeneration, slot.AdmissionGeneration,
                        batch->Callback, retrieveStatus, retrieveCalls,
                        unservedReasons != 0 ? unservedReasons : vad::UnservedNoCurrent);
                }
                ++slot.SilencePackets;
            } else {
                ++slot.DroppedPackets;
            }
        }
    }
}

void VadPcmComplete(VAD_PCM_BATCH* batch)
{
    if (batch->Shared == nullptr) {
        return;
    }
    const auto shared = batch->Shared;
    batch->Completions.Drain([](const VAD_PCM_COMPLETION& completion) {
        WdfRequestCompleteWithInformation(completion.Request,
            completion.Status, completion.Information);
    });
    batch->Shared = nullptr;
    // A purge/root shutdown cannot outlive retrieved requests or their MDLs.
    ExReleaseRundownProtection(&shared->PcmRundown);
}
