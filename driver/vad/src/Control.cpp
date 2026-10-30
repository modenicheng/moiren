// SPDX-License-Identifier: MIT
#include "Driver.h"
#include "Control.tmh"

VAD_PAGE static EVT_WDF_WORKITEM ControlWork;
static EVT_WDF_REQUEST_CANCEL CancelControl;

// Only the passive control path emits diagnostics. Audio callbacks retain two
// fixed first observations; no PCM, user names, paths, or per-packet log stream.
static void TracePcmObservation(const MOIREN_VAD_ENDPOINT_SNAPSHOT& snapshot,
    const vad::PcmDiagnostics& diagnostic, const vad::PcmTransferObservation& item,
    ULONG kind)
{
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_CONTROL,
        "PCM diagnostic endpoint=%I64u session=%I64u stream=%I64u run_gate=%I64u run_qpc=%I64u kind=%u outcome=%u gate=%I64u native=%I64u request=%I64u header_qpc=%I64u callback_qpc=%I64u check_qpc=%I64u period=%I64u frames=%u",
        snapshot.EndpointId, snapshot.SessionEpoch, diagnostic.Generation,
        diagnostic.RunGateEpoch, diagnostic.RunQpc, kind,
        static_cast<ULONG>(item.Outcome), item.GateEpoch, item.NativeSequence,
        item.RequestSequence, item.HeaderQpc, item.CallbackQpc, item.CheckQpc,
        item.PeriodTicks, item.Frames);
}

static void TracePendingPcmDiagnostics(VAD_SHARED* shared,
    const MOIREN_VAD_ENDPOINT_SNAPSHOT& snapshot)
{
    if (snapshot.Intent != MOIREN_VAD_INTENT_MICROPHONE ||
        !WPP_LEVEL_FLAGS_ENABLED(TRACE_LEVEL_INFORMATION, VAD_CONTROL)) {
        return;
    }
    vad::PcmDiagnostics diagnostic = {};
    KIRQL irql;
    KeAcquireSpinLock(&shared->Lock, &irql);
    for (ULONG i = 0; i < MOIREN_VAD_MAX_ENDPOINTS; ++i) {
        const auto& slot = shared->Slots[i];
        if (slot.Id == snapshot.EndpointId && slot.Session == snapshot.SessionEpoch &&
            slot.StreamGeneration == snapshot.StreamGeneration) {
            diagnostic = shared->PcmDiagnostics[i].TakeUnreported();
            break;
        }
    }
    KeReleaseSpinLock(&shared->Lock, irql);
    if (diagnostic.ReportFirst) {
        TracePcmObservation(snapshot, diagnostic, diagnostic.FirstTransfer, 1);
    }
    if (diagnostic.ReportRejected) {
        TracePcmObservation(snapshot, diagnostic, diagnostic.FirstRejected, 2);
    }
}

static void TracePendingClockDiagnostic(VAD_SHARED* shared,
    const MOIREN_VAD_ENDPOINT_SNAPSHOT& snapshot)
{
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    if (!WPP_LEVEL_FLAGS_ENABLED(TRACE_LEVEL_INFORMATION, VAD_CONTROL)) {
        return;
    }
    vad::audio::ClockDiagnostic diagnostic = {};
    KIRQL irql;
    KeAcquireSpinLock(&shared->Lock, &irql);
    for (ULONG i = 0; i < MOIREN_VAD_MAX_ENDPOINTS; ++i) {
        const auto& slot = shared->Slots[i];
        if (slot.Id == snapshot.EndpointId && slot.Session == snapshot.SessionEpoch &&
            slot.StreamGeneration == snapshot.StreamGeneration) {
            diagnostic = shared->ClockDiagnostics[i].TakeUnreported();
            break;
        }
    }
    KeReleaseSpinLock(&shared->Lock, irql);
    if (!diagnostic.ReportFirst) {
        return;
    }
    const auto& item = diagnostic.First;
    const auto& before = diagnostic.EndpointBefore;
    const auto& after = diagnostic.EndpointAfter;
    // Fixed first observation only, emitted after releasing Shared->Lock.
    // QPC ticks/frequency are evidence, not wall time or physical audio latency.
    // Keep each WPP payload small enough for tracepdb/TMF decoding. Repeat the
    // immutable correlation tuple so interleaved endpoint fragments can be joined.
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_CONTROL,
        "Clock rebase cause endpoint=%I64u session=%I64u generation=%I64u stream=%I64u callback_qpc=%I64u reason=%u deadline_frame=%I64u deadline_qpc=%I64u late_ticks=%I64u frames=%u packets=%u completed=%I64u presented=%I64u",
        snapshot.EndpointId, snapshot.SessionEpoch, diagnostic.Generation,
        diagnostic.Stream, item.CallbackQpc, static_cast<ULONG>(item.Reason),
        item.DeadlineFrame, item.DeadlineQpc, item.LatenessTicks, item.PacketFrames,
        item.PacketCount, item.CompletedPackets, item.PresentedFrames);
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_CONTROL,
        "Clock rebase before endpoint=%I64u session=%I64u generation=%I64u stream=%I64u callback_qpc=%I64u slot_gen=%I64u gate=%I64u open=%u state=%u freq=%I64u epoch=%I64u base=%I64u limit=%I64u running=%u",
        snapshot.EndpointId, snapshot.SessionEpoch, diagnostic.Generation,
        diagnostic.Stream, item.CallbackQpc, before.Generation, before.GateEpoch,
        before.GateOpen, before.StreamState, item.Before.Frequency,
        item.Before.EpochQpc, item.Before.BaseFrames, item.Before.LimitFrames,
        static_cast<ULONG>(item.Before.Running));
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_CONTROL,
        "Clock rebase after endpoint=%I64u session=%I64u generation=%I64u stream=%I64u callback_qpc=%I64u slot_gen=%I64u gate=%I64u open=%u state=%u freq=%I64u epoch=%I64u base=%I64u limit=%I64u running=%u",
        snapshot.EndpointId, snapshot.SessionEpoch, diagnostic.Generation,
        diagnostic.Stream, item.CallbackQpc, after.Generation, after.GateEpoch,
        after.GateOpen, after.StreamState, item.After.Frequency,
        item.After.EpochQpc, item.After.BaseFrames, item.After.LimitFrames,
        static_cast<ULONG>(item.After.Running));
}

static void TracePendingCaptureDiagnostic(VAD_SHARED* shared,
    const MOIREN_VAD_ENDPOINT_SNAPSHOT& snapshot)
{
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    if (snapshot.Intent != MOIREN_VAD_INTENT_MICROPHONE ||
        !WPP_LEVEL_FLAGS_ENABLED(TRACE_LEVEL_INFORMATION, VAD_CONTROL)) { return; }
    vad::audio::CaptureDiagnostic diagnostic = {};
    KIRQL irql;
    KeAcquireSpinLock(&shared->Lock, &irql);
    for (ULONG i = 0; i < MOIREN_VAD_MAX_ENDPOINTS; ++i) {
        const auto& slot = shared->Slots[i];
        if (slot.Id == snapshot.EndpointId && slot.Session == snapshot.SessionEpoch &&
            slot.StreamGeneration == snapshot.StreamGeneration) {
            diagnostic = shared->CaptureDiagnostics[i].TakeUnreported();
            break;
        }
    }
    KeReleaseSpinLock(&shared->Lock, irql);
    const vad::audio::CaptureCopyObservation* copies[] = {
        &diagnostic.FirstCopy, &diagnostic.FirstNonzero};
    const bool pending[] = {diagnostic.ReportCopy, diagnostic.ReportNonzero};
    for (ULONG i = 0; i < 2; ++i) {
        if (!pending[i]) { continue; }
        const auto& item = *copies[i];
        // Repeated identity/kind joins fixed fragments; booleans describe only
        // the inspected prefix, never the whole ring or downstream delivery.
        VadTrace(TRACE_LEVEL_INFORMATION, VAD_CONTROL,
            "Capture boundary shape endpoint=%I64u session=%I64u generation=%I64u stream=%I64u kind=%u gate=%I64u native=%I64u request=%I64u qpc=%I64u packets=%u packet_bytes=%u native_bytes=%u ring_offset=%u packet_offset=%u",
            snapshot.EndpointId, snapshot.SessionEpoch, diagnostic.Generation, diagnostic.Stream,
            i, item.GateEpoch, item.NativeSequence, item.RequestSequence, item.Qpc,
            item.PacketCount, item.PacketBytes, item.NativePacketBytes,
            item.RingOffsetBytes, item.PacketOffsetBytes);
        VadTrace(TRACE_LEVEL_INFORMATION, VAD_CONTROL,
            "Capture boundary prefix endpoint=%I64u session=%I64u generation=%I64u stream=%I64u kind=%u bytes=%u source_nonzero=%u destination_nonzero=%u equal=%u",
            snapshot.EndpointId, snapshot.SessionEpoch, diagnostic.Generation, diagnostic.Stream,
            i, item.Prefix.Bytes, static_cast<ULONG>(item.Prefix.SourceNonzero),
            static_cast<ULONG>(item.Prefix.DestinationNonzero), static_cast<ULONG>(item.Prefix.Equal));
    }
    if (diagnostic.ReportPosition) {
        const auto& item = diagnostic.Position;
        VadTrace(TRACE_LEVEL_INFORMATION, VAD_CONTROL,
            "Capture boundary position endpoint=%I64u session=%I64u generation=%I64u stream=%I64u gate=%I64u qpc=%I64u packets=%u returned_frames=%I64u",
            snapshot.EndpointId, snapshot.SessionEpoch, diagnostic.Generation, diagnostic.Stream,
            item.GateEpoch, item.Qpc, item.PacketCount, item.Frames);
    }
    const vad::audio::CapturePacketObservation* packets[] = {
        &diagnostic.FirstPacket, &diagnostic.FirstNonzeroPacket, &diagnostic.FirstPacketRefusal};
    const bool packetPending[] = {diagnostic.ReportPacket, diagnostic.ReportNonzeroPacket,
        diagnostic.ReportPacketRefusal};
    for (ULONG i = 0; i < 3; ++i) {
        if (!packetPending[i]) { continue; }
        const auto& item = *packets[i];
        // No callback record means unknown: ACX may consume the registered
        // packet counter without invoking GetCapturePacket at all.
        VadTrace(TRACE_LEVEL_INFORMATION, VAD_CONTROL,
            "Capture packet query endpoint=%I64u session=%I64u generation=%I64u kind=%u gate=%I64u status=%!STATUS! completed=%I64u delivered_before=%I64u delivered_after=%I64u packet=%u packet_qpc=%I64u",
            snapshot.EndpointId, snapshot.SessionEpoch, diagnostic.Generation, i,
            item.GateEpoch, static_cast<NTSTATUS>(item.Status), item.Completed,
            item.DeliveredBefore, item.DeliveredAfter, item.Packet, item.PacketStartQpc);
        VadTrace(TRACE_LEVEL_INFORMATION, VAD_CONTROL,
            "Capture packet readback endpoint=%I64u session=%I64u generation=%I64u kind=%u gate=%I64u completed=%I64u packet=%u packets=%u index=%u offset=%u bytes=%u nonzero=%u",
            snapshot.EndpointId, snapshot.SessionEpoch, diagnostic.Generation, i,
            item.GateEpoch, item.Completed, item.Packet, item.PacketCount, item.Index,
            item.OffsetBytes, item.Prefix.Bytes, static_cast<ULONG>(item.Prefix.Nonzero));
    }
    const vad::audio::CaptureNotifyObservation* notifications[] = {
        &diagnostic.FirstNotifySuccess, &diagnostic.FirstNotifyFailure};
    const bool notifyPending[] = {diagnostic.ReportNotifySuccess, diagnostic.ReportNotifyFailure};
    for (ULONG i = 0; i < 2; ++i) {
        if (!notifyPending[i]) { continue; }
        const auto& item = *notifications[i];
        VadTrace(TRACE_LEVEL_INFORMATION, VAD_CONTROL,
            "Capture packet notify endpoint=%I64u session=%I64u generation=%I64u kind=%u gate=%I64u native=%I64u completion_qpc=%I64u packets=%u status=%!STATUS!",
            snapshot.EndpointId, snapshot.SessionEpoch, diagnostic.Generation, i,
            item.GateEpoch, item.NativeSequence, item.CompletionQpc, item.PacketCount,
            static_cast<NTSTATUS>(item.Status));
    }
}

static void TracePendingUnservedDiagnostics(VAD_SHARED* shared,
    const MOIREN_VAD_ENDPOINT_SNAPSHOT& snapshot)
{
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    if (snapshot.Intent != MOIREN_VAD_INTENT_MICROPHONE ||
        !WPP_LEVEL_FLAGS_ENABLED(TRACE_LEVEL_INFORMATION, VAD_CONTROL)) {
        return;
    }
    vad::UnservedDiagnostics diagnostic = {};
    KIRQL irql;
    KeAcquireSpinLock(&shared->Lock, &irql);
    for (ULONG i = 0; i < MOIREN_VAD_MAX_ENDPOINTS; ++i) {
        const auto& slot = shared->Slots[i];
        if (slot.Id == snapshot.EndpointId && slot.Session == snapshot.SessionEpoch &&
            slot.StreamGeneration == snapshot.StreamGeneration) {
            diagnostic = shared->UnservedDiagnostics[i].TakeUnreported();
            break;
        }
    }
    KeReleaseSpinLock(&shared->Lock, irql);
    for (ULONG i = 0; i < 3; ++i) {
        const auto& item = diagnostic.First[i];
        if ((item.Flags & vad::UnservedReport) == 0) { continue; }
        // First unserved only, never a per-packet trace. Repeat correlation so
        // interleaved fragments remain joinable. Deadline=0 means timer mode.
        VadTrace(TRACE_LEVEL_INFORMATION, VAD_CONTROL,
            "Mic unserved callback endpoint=%I64u session=%I64u generation=%I64u stream=%I64u state_qpc=%I64u kind=%u gate=%I64u flags=%u dpc_qpc=%I64u deadline_qpc=%I64u retrieve=%!STATUS! calls=%u freq=%I64u",
            snapshot.EndpointId, snapshot.SessionEpoch, diagnostic.Generation,
            diagnostic.Stream, item.StateQpc, i, item.GateEpoch,
            static_cast<ULONG>(item.Flags), item.DpcQpc, item.DeadlineQpc,
            static_cast<NTSTATUS>(item.RetrieveStatus),
            static_cast<ULONG>(item.RetrieveCalls), diagnostic.Frequency);
        // Pre-forward is a safe admission-attempt boundary, not actual enqueue
        // time; there is deliberately no post-forward request/context access.
        VadTrace(TRACE_LEVEL_INFORMATION, VAD_CONTROL,
            "Mic unserved write_pre endpoint=%I64u session=%I64u generation=%I64u stream=%I64u state_qpc=%I64u kind=%u gate=%I64u has_write=%u request=%I64u header_qpc=%I64u pre_forward_qpc=%I64u",
            snapshot.EndpointId, snapshot.SessionEpoch, diagnostic.Generation,
            diagnostic.Stream, item.StateQpc, i, item.GateEpoch,
            static_cast<ULONG>((item.Flags & vad::UnservedHasWrite) != 0),
            item.WriteSequence, item.WriteHeaderQpc, item.WritePreForwardQpc);
    }
}

VAD_PAGE NTSTATUS VadControlInitialize(WDFDEVICE device)
{
    PAGED_CODE();
    auto root = VadRoot(device);
    KeInitializeSpinLock(&root->ControlLock);
    WDF_WORKITEM_CONFIG config;
    WDF_WORKITEM_CONFIG_INIT(&config, ControlWork);
    config.AutomaticSerialization = FALSE;
    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.ParentObject = device;
    return WdfWorkItemCreate(&config, &attributes, &root->ControlWork);
}

static NTSTATUS Dispatch(WDFDEVICE device, WDFREQUEST request, size_t* information)
{
    auto root = VadRoot(device);
    if (InterlockedCompareExchange(&root->Online, 0, 0) == 0 ||
        root->Owner == nullptr || WdfRequestGetFileObject(request) != root->Owner) {
        return STATUS_ACCESS_DENIED;
    }
    WDF_REQUEST_PARAMETERS parameters;
    WDF_REQUEST_PARAMETERS_INIT(&parameters);
    WdfRequestGetParameters(request, &parameters);
    const ULONG code = parameters.Parameters.DeviceIoControl.IoControlCode;
    if (code == IOCTL_MOIREN_VAD_READ_PCM || code == IOCTL_MOIREN_VAD_WRITE_PCM) {
        return VadPcmSubmit(device, request, code);
    }
    size_t expected;
    switch (code) {
    case IOCTL_MOIREN_VAD_CAPABILITIES: expected = sizeof(MOIREN_VAD_HEADER); break;
    case IOCTL_MOIREN_VAD_CREATE: expected = sizeof(MOIREN_VAD_CREATE_REQUEST); break;
    case IOCTL_MOIREN_VAD_QUERY:
    case IOCTL_MOIREN_VAD_REMOVE: expected = sizeof(MOIREN_VAD_ENDPOINT_REQUEST); break;
    case IOCTL_MOIREN_VAD_SET_GATE: expected = sizeof(MOIREN_VAD_GATE_REQUEST); break;
    default: return STATUS_INVALID_DEVICE_REQUEST;
    }
    if (parameters.Parameters.DeviceIoControl.InputBufferLength != expected) {
        return STATUS_INFO_LENGTH_MISMATCH;
    }
    void* input = nullptr;
    NTSTATUS status = WdfRequestRetrieveInputBuffer(request, expected, &input, nullptr);
    if (!NT_SUCCESS(status)) { return status; }
    // METHOD_BUFFERED shares input/output storage; snapshot all input first.
    union {
        MOIREN_VAD_HEADER Header;
        MOIREN_VAD_CREATE_REQUEST Create;
        MOIREN_VAD_ENDPOINT_REQUEST Endpoint;
        MOIREN_VAD_GATE_REQUEST Gate;
    } local = {};
    RtlCopyMemory(&local, input, expected);
    if (local.Header.Size != expected || local.Header.Version != MOIREN_VAD_ABI_VERSION) {
        return STATUS_REVISION_MISMATCH;
    }
    void* output = nullptr;
    const auto outputSize = code == IOCTL_MOIREN_VAD_CAPABILITIES ?
        sizeof(MOIREN_VAD_CAPABILITIES) : sizeof(MOIREN_VAD_ENDPOINT_SNAPSHOT);
    status = WdfRequestRetrieveOutputBuffer(request, outputSize, &output, nullptr);
    if (!NT_SUCCESS(status)) { return status; }
    auto shared = root->Shared;
    if (code == IOCTL_MOIREN_VAD_CAPABILITIES) {
        const MOIREN_VAD_CAPABILITIES capabilities = {
            sizeof(MOIREN_VAD_CAPABILITIES), MOIREN_VAD_ABI_VERSION,
            MOIREN_VAD_CAP_RAW_PDO | MOIREN_VAD_CAP_POLICY_STAGED |
                MOIREN_VAD_CAP_AUDIO | MOIREN_VAD_CAP_STREAMS | MOIREN_VAD_CAP_PCM |
                MOIREN_VAD_CAP_CREATE_DEFERRED,
            MOIREN_VAD_MAX_ENDPOINTS, MOIREN_VAD_MAX_STREAMS_PER_PIN,
            MOIREN_VAD_SAMPLE_RATE, MOIREN_VAD_FORMAT_PCM16 | MOIREN_VAD_FORMAT_FLOAT32,
            6, 6, MOIREN_VAD_REQUEST_SLOTS, root->Session,
            MOIREN_VAD_MAX_PACKET_FRAMES, {0, 0, 0}};
        RtlCopyMemory(output, &capabilities, sizeof(capabilities));
        *information = sizeof(capabilities);
        return STATUS_SUCCESS;
    }
    MOIREN_VAD_ENDPOINT_SNAPSHOT snapshot = {};
    if (code == IOCTL_MOIREN_VAD_CREATE) {
        const auto& create = local.Create;
        bool terminated = false;
        for (const auto unit : create.Name) {
            if (unit == 0) { terminated = true; break; }
            if (unit < 0x20) { return STATUS_INVALID_PARAMETER; }
        }
        if (!vad::ValidEndpoint(create.Intent, create.Channels) ||
            create.AllowDefault > 1 || create.Reserved != 0 ||
            create.Name[0] == 0 || !terminated) {
            return STATUS_INVALID_PARAMETER;
        }
        status = VadCreateChild(device, create, &snapshot);
    } else {
        if (local.Endpoint.SessionEpoch != root->Session ||
            local.Endpoint.EndpointId == 0 ||
            local.Endpoint.EndpointGeneration != local.Endpoint.EndpointId) {
            return STATUS_REVISION_MISMATCH;
        }
        if (code == IOCTL_MOIREN_VAD_REMOVE) {
            status = VadRemoveChild(device, local.Endpoint.EndpointId, &snapshot);
        } else if (code == IOCTL_MOIREN_VAD_SET_GATE) {
            if (local.Gate.Reserved != 0 || local.Gate.Open > 1) {
                return STATUS_INVALID_PARAMETER;
            }
            status = VadSetGate(device, local.Gate, &snapshot);
        } else {
            status = STATUS_NOT_FOUND;
            KIRQL irql;
            KeAcquireSpinLock(&shared->Lock, &irql);
            for (const auto& slot : shared->Slots) {
                if (vad::SameIdentity(slot, local.Endpoint)) {
                    snapshot = vad::Snapshot(slot);
                    status = STATUS_SUCCESS;
                    break;
                }
            }
            KeReleaseSpinLock(&shared->Lock, irql);
        }
    }
    if (NT_SUCCESS(status)) {
        // Gate close stops further admitted transfers before passive logging;
        // successful probes need no extra QUERY in their measured interval.
        if (code == IOCTL_MOIREN_VAD_QUERY ||
            (code == IOCTL_MOIREN_VAD_SET_GATE && local.Gate.Open == 0)) {
            TracePendingPcmDiagnostics(shared, snapshot);
            TracePendingClockDiagnostic(shared, snapshot);
            TracePendingCaptureDiagnostic(shared, snapshot);
            TracePendingUnservedDiagnostics(shared, snapshot);
        }
        RtlCopyMemory(output, &snapshot, sizeof(snapshot));
        *information = sizeof(snapshot);
    }
    return status;
}

_Use_decl_annotations_
void VadIoControl(WDFQUEUE queue, WDFREQUEST request, size_t outputLength,
    size_t inputLength, ULONG code)
{
    UNREFERENCED_PARAMETER(outputLength);
    UNREFERENCED_PARAMETER(inputLength);
    UNREFERENCED_PARAMETER(code);
    auto root = VadRoot(WdfIoQueueGetDevice(queue));
    KIRQL irql;
    KeAcquireSpinLock(&root->ControlLock, &irql);
    // The sequential queue owns all undispatched requests. Only its single
    // delivered request can be handed off; no per-request work item or queue.
    NTSTATUS status = STATUS_DEVICE_BUSY;
    if (root->ControlHandoff.Publish(request)) {
        status = WdfRequestMarkCancelableEx(request, CancelControl);
        if (NT_SUCCESS(status)) {
            // Enqueue while still holding the handoff lock: cancellation must
            // not complete this request before the work item is scheduled.
            WdfWorkItemEnqueue(root->ControlWork);
        } else {
            (void)root->ControlHandoff.Take();
        }
    }
    KeReleaseSpinLock(&root->ControlLock, irql);
    if (!NT_SUCCESS(status)) {
        WdfRequestComplete(request, status);
    }
}

_Use_decl_annotations_
static void CancelControl(WDFREQUEST request)
{
    auto root = VadRoot(WdfIoQueueGetDevice(WdfRequestGetIoQueue(request)));
    KIRQL irql;
    KeAcquireSpinLock(&root->ControlLock, &irql);
    // ClaimControl may already have detached a canceled request. Matching by
    // handle also prevents that late cancel from clearing the next handoff.
    (void)root->ControlHandoff.Cancel(request);
    KeReleaseSpinLock(&root->ControlLock, irql);
    WdfRequestComplete(request, STATUS_CANCELLED);
}

// Keep the temporary DISPATCH_LEVEL claim outside the pageable worker body.
// CancelControl uses the same lock, so it cannot complete a request between
// inspecting Pending and unmarking it (an object reference alone is insufficient).
static NTSTATUS ClaimControl(VAD_ROOT* root, WDFREQUEST* result)
{
    *result = nullptr;
    KIRQL irql;
    KeAcquireSpinLock(&root->ControlLock, &irql);
    const auto request = root->ControlHandoff.Pending;
    if (request == nullptr) {
        KeReleaseSpinLock(&root->ControlLock, irql);
        return STATUS_NO_MORE_ENTRIES;
    }
    NTSTATUS status = WdfRequestUnmarkCancelable(request);
    // Detach even on STATUS_CANCELLED: an overlapping work-item invocation
    // must not attempt a second unmark while CancelControl is waiting here.
    (void)root->ControlHandoff.Take();
    if (status != STATUS_CANCELLED) {
        *result = request;
    }
    KeReleaseSpinLock(&root->ControlLock, irql);
    return status;
}

VAD_PAGE _Use_decl_annotations_
static void ControlWork(WDFWORKITEM work)
{
    PAGED_CODE();
    WDFDEVICE device = static_cast<WDFDEVICE>(WdfWorkItemGetParentObject(work));
    auto root = VadRoot(device);
    WDFREQUEST request = nullptr;
    NTSTATUS status = ClaimControl(root, &request);
    if (request == nullptr) {
        return; // Absent/canceled: only CancelControl may touch the old request.
    }
    if (!NT_SUCCESS(status)) {
        WdfRequestComplete(request, status);
        return;
    }
    // All synchronous PCM drains and ACX management now execute outside EvtIo.
    // Unmarking also makes a subsequent PCM queue forward legal. Once claimed,
    // this request is completed here or exclusively owned by the PCM queue.
    WdfWaitLockAcquire(root->Admin, nullptr);
    size_t information = 0;
    status = WdfRequestIsCanceled(request) ? STATUS_CANCELLED :
        Dispatch(device, request, &information);
    WdfWaitLockRelease(root->Admin);
    if (status == STATUS_PENDING) { return; } // Manual PCM queue owns the request.
    VadTrace(NT_SUCCESS(status) ? TRACE_LEVEL_VERBOSE : TRACE_LEVEL_WARNING,
        VAD_CONTROL, "Control worker status=%!STATUS! bytes=%Iu", status, information);
    // Completion/forward can dispatch the next sequential request and enqueue
    // this same precreated work item. No request/root state is touched afterward.
    WdfRequestCompleteWithInformation(request, status, information);
}
