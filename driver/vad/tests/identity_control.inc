// SPDX-License-Identifier: MIT
// The full production Dispatch consumes the real shared buffered request.
// Stub only WDF request/PDO creation edges, not validation or mode selection.
constexpr NTSTATUS STATUS_ACCESS_DENIED = -90, STATUS_INVALID_DEVICE_REQUEST = -91;
constexpr NTSTATUS STATUS_INFO_LENGTH_MISMATCH = -92, STATUS_REVISION_MISMATCH = -93;
struct WDF_REQUEST_PARAMETERS {
    struct { struct { ULONG IoControlCode = 0; size_t InputBufferLength = 0; }
        DeviceIoControl; } Parameters;
};
#define WDF_REQUEST_PARAMETERS_INIT(value) (*(value) = {})
struct IdentityRequest {
    unsigned char Buffered[176] = {};
    ULONG Code = 0;
    size_t InputBytes = 0, OutputBytes = 128;
    void* Owner = nullptr;
};
using WDFREQUEST = IdentityRequest*;
struct ControlRoot {
    SharedState* Shared = nullptr;
    void* Owner = nullptr;
    long Online = 1;
    uint64_t Session = 1;
} Control;
static Child* CreatedChild = nullptr;
static unsigned CreateCalls = 0;
static MOIREN_VAD_CREATE_REQUEST Normalized = {};
ControlRoot* VadRoot(WDFDEVICE) { return &Control; }
long InterlockedCompareExchange(long* value, long, long) { return *value; }
void* WdfRequestGetFileObject(WDFREQUEST request) { return request->Owner; }
void WdfRequestGetParameters(WDFREQUEST request, WDF_REQUEST_PARAMETERS* parameters)
{
    parameters->Parameters.DeviceIoControl.IoControlCode = request->Code;
    parameters->Parameters.DeviceIoControl.InputBufferLength = request->InputBytes;
}
NTSTATUS WdfRequestRetrieveInputBuffer(WDFREQUEST request, size_t bytes, void** buffer, void*)
{
    if (request->InputBytes < bytes) { return STATUS_BUFFER_TOO_SMALL; }
    *buffer = request->Buffered;
    return STATUS_SUCCESS;
}
NTSTATUS WdfRequestRetrieveOutputBuffer(WDFREQUEST request, size_t bytes, void** buffer, void*)
{
    if (request->OutputBytes < bytes) { return STATUS_BUFFER_TOO_SMALL; }
    *buffer = request->Buffered; // Actual METHOD_BUFFERED aliasing, not separate data.
    return STATUS_SUCCESS;
}
namespace vad {
inline bool ValidEndpoint(uint32_t intent, uint32_t channels)
{ return (intent == 1 || intent == 2) && (channels == 1 || channels == 2); }
inline bool SameIdentity(const ::Slot&, const MOIREN_VAD_ENDPOINT_REQUEST&)
{ Check(false, "query branch is not part of create-only fixture"); return false; }
inline MOIREN_VAD_ENDPOINT_SNAPSHOT Snapshot(const ::Slot&)
{ Check(false, "query branch is not part of create-only fixture"); return {}; }
}
NTSTATUS VadPcmSubmit(WDFDEVICE, WDFREQUEST, ULONG) { return STATUS_INVALID_DEVICE_REQUEST; }
NTSTATUS VadRemoveChild(WDFDEVICE, uint64_t, MOIREN_VAD_ENDPOINT_SNAPSHOT*)
{ return STATUS_INVALID_DEVICE_REQUEST; }
NTSTATUS VadSetGate(WDFDEVICE, const MOIREN_VAD_GATE_REQUEST&, MOIREN_VAD_ENDPOINT_SNAPSHOT*)
{ return STATUS_INVALID_DEVICE_REQUEST; }
void TracePendingRemovalDiagnostic(SharedState*, const MOIREN_VAD_ENDPOINT_SNAPSHOT&) {}
void TracePendingPcmDiagnostics(SharedState*, const MOIREN_VAD_ENDPOINT_SNAPSHOT&) {}
void TracePendingClockDiagnostic(SharedState*, const MOIREN_VAD_ENDPOINT_SNAPSHOT&) {}
void TracePendingCaptureDiagnostic(SharedState*, const MOIREN_VAD_ENDPOINT_SNAPSHOT&) {}
void TracePendingUnservedDiagnostics(SharedState*, const MOIREN_VAD_ENDPOINT_SNAPSHOT&) {}
NTSTATUS VadCreateChild(WDFDEVICE, const MOIREN_VAD_CREATE_REQUEST& request,
    MOIREN_VAD_ENDPOINT_SNAPSHOT* snapshot, uint32_t mode,
    const MOIREN_VAD_ENDPOINT_ID* installation)
{
    ++CreateCalls;
    Normalized = request;
    Check(request.Size == sizeof(request) && request.Version == MOIREN_VAD_ABI_VERSION,
        "actual control dispatch normalizes a separate identity header before child creation");
    auto child = CreatedChild;
    child->Context.PublicationMode = mode;
    if (installation != nullptr) { child->Context.InstallationId = *installation; }
    child->Shared.Slots[0].Intent = request.Intent;
    child->Shared.Slots[0].StableId = request.StableId;
    child->Shared.Slots[0].AllowDefault = request.AllowDefault != 0;
    std::memcpy(child->Shared.Slots[0].Name, request.Name, sizeof(request.Name));
    // Return only the creating snapshot, as real ACX enumeration is async.
    *snapshot = {};
    snapshot->Size = sizeof(*snapshot);
    snapshot->Version = MOIREN_VAD_ABI_VERSION;
    snapshot->State = MOIREN_VAD_STATE_CREATING;
    snapshot->StableId = request.StableId;
    snapshot->EndpointId = 77;
    return STATUS_SUCCESS;
}
#include "identity-control-production.inc"

static void ActualControlNegotiatesAndSelectsPublicationMode()
{
    for (ULONG code : {IOCTL_MOIREN_VAD_CREATE, IOCTL_MOIREN_VAD_CREATE_IDENTITY}) {
        for (uint32_t mode : {MOIREN_VAD_IDENTITY_ESTABLISH_NEW, MOIREN_VAD_IDENTITY_REOPEN_OR_ADOPT}) {
            Child root, child;
            Reset(child);
            if (mode == MOIREN_VAD_IDENTITY_ESTABLISH_NEW) {
                for (auto& count : child.Store->Count) { count = 0; }
            }
            Control = {&root.Shared, &root, 1, 1};
            CreatedChild = &child;
            CreateCalls = 0;
            IdentityRequest io;
            io.Code = code;
            io.Owner = &root;
            MOIREN_VAD_CREATE_IDENTITY_REQUEST request = {};
            request.Size = code == IOCTL_MOIREN_VAD_CREATE ? sizeof(MOIREN_VAD_CREATE_REQUEST) : sizeof(request);
            request.Version = code == IOCTL_MOIREN_VAD_CREATE ? MOIREN_VAD_ABI_VERSION : MOIREN_VAD_IDENTITY_VERSION;
            request.Intent = MOIREN_VAD_INTENT_PLAYBACK;
            request.Channels = 2;
            request.StableId = {1, 2};
            request.Name[0] = L'N';
            request.Mode = mode;
            io.InputBytes = request.Size;
            std::memcpy(io.Buffered, &request, sizeof(request));
            size_t information = 0;
            SystemThread = true;
            const auto status = Dispatch(&root, &io, &information);
            SystemThread = false;
            Check(status == STATUS_SUCCESS && CreateCalls == 1 && information == 128,
                "real Dispatch accepts legacy/new request without buffered input corruption");
            Check(Normalized.StableId.Low == 1 && Normalized.StableId.High == 2 && Normalized.Name[0] == L'N',
                "normalization preserves exact ID and mutable text");
            Check(child.Context.PublicationMode == (code == IOCTL_MOIREN_VAD_CREATE ? 0 : mode),
                "only opt-in IOCTL conveys explicit establishment/reopen mode");
            F.RequireDurableIdentity = code != IOCTL_MOIREN_VAD_CREATE;
            Check(VadPreparePolicy(&child, 0) == STATUS_SUCCESS && F.Registered == 3,
                "actual dispatched decision can publish all categories");
            Check(VadAudioBuildCircuit(&Audio) == STATUS_SUCCESS && C.Reference == child.Context.PolicyReference,
                "actual dispatched decision reaches ACX through the single policy reference");
            Release(child);
        }
    }
    Child root;
    Control = {&root.Shared, &root, 1, 1};
    for (ULONG code : {IOCTL_MOIREN_VAD_CAPABILITIES, IOCTL_MOIREN_VAD_IDENTITY_CAPABILITIES}) {
        F = {};
        CreateCalls = 0;
        IdentityRequest io;
        io.Owner = &root;
        io.Code = code;
        io.InputBytes = sizeof(MOIREN_VAD_HEADER);
        const MOIREN_VAD_HEADER header = {sizeof(header), code == IOCTL_MOIREN_VAD_CAPABILITIES ? 2u : 1u};
        std::memcpy(io.Buffered, &header, sizeof(header));
        size_t information = 0;
        Check(Dispatch(&root, &io, &information) == STATUS_SUCCESS && CreateCalls == 0 && F.IdentityOperations == 0,
            "capability discovery neither creates identity nor opens storage");
        if (code == IOCTL_MOIREN_VAD_CAPABILITIES) {
            MOIREN_VAD_CAPABILITIES caps = {};
            std::memcpy(&caps, io.Buffered, sizeof(caps));
            Check(caps.Version == 2 && caps.Flags == 0x3f && caps.Reserved[0] == 0 &&
                caps.Reserved[1] == 0 && caps.Reserved[2] == 0 && information == sizeof(caps),
                "pre-change v2 decoder can still connect: flags/reserved never silently change");
        } else {
            MOIREN_VAD_IDENTITY_CAPABILITIES caps = {};
            std::memcpy(&caps, io.Buffered, sizeof(caps));
            Check(caps.Version == 1 && caps.Flags == MOIREN_VAD_CAP_PUBLICATION_IDENTITY &&
                caps.Reserved == 0 && information == sizeof(caps), "identity capability is explicit/versioned");
        }
    }
}

static void ActualControlRejectsMalformedIdentityBeforeStorageOrChild()
{
    for (unsigned fault = 0; fault < 14; ++fault) {
        F = {};
        Child root;
        Control = {&root.Shared, &root, 1, 1};
        CreateCalls = 0;
        IdentityRequest io;
        io.Code = IOCTL_MOIREN_VAD_CREATE_IDENTITY;
        io.Owner = &root;
        io.InputBytes = sizeof(MOIREN_VAD_CREATE_IDENTITY_REQUEST);
        MOIREN_VAD_CREATE_IDENTITY_REQUEST request = {};
        request.Size = sizeof(request);
        request.Version = MOIREN_VAD_IDENTITY_VERSION;
        request.Intent = MOIREN_VAD_INTENT_PLAYBACK;
        request.Channels = 2;
        request.StableId = {1, 2};
        request.Name[0] = L'N';
        request.Mode = MOIREN_VAD_IDENTITY_ESTABLISH_NEW;
        NTSTATUS expected = STATUS_INVALID_PARAMETER;
        switch (fault) {
        case 0: --io.InputBytes; expected = STATUS_INFO_LENGTH_MISMATCH; break;
        case 1: --request.Size; expected = STATUS_REVISION_MISMATCH; break;
        case 2: request.Version = MOIREN_VAD_ABI_VERSION; expected = STATUS_REVISION_MISMATCH; break;
        case 3: request.Mode = 0; break;
        case 4: request.Mode = 3; break;
        case 5: request.Reserved1 = 1; break;
        case 6: request.Reserved = 1; break;
        case 7: request.StableId = {}; break;
        case 8: io.OutputBytes = 127; expected = STATUS_BUFFER_TOO_SMALL; break;
        case 9: Control.Owner = nullptr; expected = STATUS_ACCESS_DENIED; break;
        case 10: io.Owner = &io; expected = STATUS_ACCESS_DENIED; break;
        case 11: Control.Online = 0; expected = STATUS_ACCESS_DENIED; break;
        case 12: request.Intent = 0; break;
        case 13: request.Name[0] = 0; break;
        }
        std::memcpy(io.Buffered, &request, sizeof(request));
        size_t information = 0;
        SystemThread = true;
        const auto status = Dispatch(&root, &io, &information);
        SystemThread = false;
        Check(status == expected && information == 0 && CreateCalls == 0 &&
            F.IdentityOperations == 0 && F.UuidCalls == 0 && F.Registered == 0,
            "malformed/unauthorized identity wire cannot reserve root state or create a PDO");
    }
}
