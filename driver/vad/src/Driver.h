// SPDX-License-Identifier: MIT
#pragma once
#include <ntddk.h>
#include <ntstrsafe.h>
#include <wdf.h>
#include <acx.h>
#include "Trace.h"
#include "Lifecycle.h"
#include "Ownership.h"
#include "PcmDiagnostics.h"
#include "audio/ClockDiagnostic.h"

extern const GUID VadControlInterface;

// Published audio identity: the ACX circuit name and the registered device
// interface reference string are one value shared by Policy and the audio
// circuit, so they cannot drift apart.
inline constexpr WCHAR VadAudioReference[] = L"MoirenEndpoint";

struct VAD_SHARED {
    // Root cleanup can precede the final destruction of a referenced PDO.
    // This fixed allocation survives independently of cleaned-up WDF objects.
    KSPIN_LOCK Lock;
    volatile LONG References;
    vad::Slot Slots[MOIREN_VAD_MAX_ENDPOINTS];
    vad::PcmDiagnostics PcmDiagnostics[MOIREN_VAD_MAX_ENDPOINTS];
    vad::audio::ClockDiagnostic ClockDiagnostics[MOIREN_VAD_MAX_ENDPOINTS];
    WDFDEVICE Children[MOIREN_VAD_MAX_ENDPOINTS]; // Owns one WDF reference.
    WDFQUEUE PcmQueues[MOIREN_VAD_MAX_ENDPOINTS];
    EX_RUNDOWN_REF PcmRundown;
    volatile LONG PcmOnline;
    BOOLEAN PcmInitialized;
    KMUTEX PcmDrain[MOIREN_VAD_MAX_ENDPOINTS];
};

struct VAD_ROOT {
    // Admin serializes create / cleanup / removal at PASSIVE_LEVEL.
    WDFWAITLOCK Admin; // A fast mutex would raise IRQL above registry DDI limits.
    VAD_SHARED* Shared;
    WDFFILEOBJECT Owner;
    uint64_t Session;
    uint64_t NextId;
    volatile LONG Online;
    volatile LONG Releasing;
    WDFQUEUE ControlQueue;
    WDFWORKITEM ControlWork;
    KSPIN_LOCK ControlLock; // Protects handoff/cancellation, never Admin or DDIs.
    vad::ControlHandoff<WDFREQUEST> ControlHandoff;
};
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(VAD_ROOT, VadRoot)

struct VAD_CHILD {
    VAD_SHARED* Shared; // One allocation reference until final child destruction.
    uint64_t Id;
    ULONG Index;
    volatile LONG ActiveStreams;
    volatile LONG CircuitDeviceAdded;
    WDFWORKITEM PolicyWork;
    NTSTATUS PolicyStatus;
    // IoRegisterDeviceInterface returns an owned symbolic-link allocation.
    // Keep each link with the PDO so partial policy setup and final cleanup
    // can release it without touching the root device.
    UNICODE_STRING PolicyInterfaces[3];
    ULONG PolicyInterfaceCount;
};
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(VAD_CHILD, VadChild)

extern "C" DRIVER_INITIALIZE DriverEntry;
#define VAD_PAGE __declspec(code_seg("PAGE"))
VAD_PAGE EVT_WDF_DRIVER_DEVICE_ADD VadAddDevice;
VAD_PAGE EVT_WDF_DRIVER_UNLOAD VadUnload;
VAD_PAGE EVT_WDF_DEVICE_FILE_CREATE VadFileCreate;
VAD_PAGE EVT_WDF_FILE_CLEANUP VadFileCleanup;
EVT_WDF_IO_QUEUE_IO_DEVICE_CONTROL VadIoControl;
VAD_PAGE NTSTATUS VadControlInitialize(WDFDEVICE device);
EVT_WDF_DEVICE_SURPRISE_REMOVAL VadSurpriseRemoval;
VAD_PAGE EVT_WDF_DEVICE_SELF_MANAGED_IO_CLEANUP VadStop;
VAD_PAGE void VadStopCore(WDFDEVICE device);
EVT_WDF_OBJECT_CONTEXT_CLEANUP VadChildCleanup;
EVT_WDF_OBJECT_CONTEXT_DESTROY VadChildDestroy;
EVT_WDF_OBJECT_CONTEXT_CLEANUP VadRootCleanup;
void VadSharedRelease(VAD_SHARED* shared);

// Caller holds Admin, never Shared->Lock, across PnP/registry DDIs.
_IRQL_requires_(PASSIVE_LEVEL)
NTSTATUS VadCreateChild(WDFDEVICE root, const MOIREN_VAD_CREATE_REQUEST& request,
    MOIREN_VAD_ENDPOINT_SNAPSHOT* snapshot);
_IRQL_requires_(PASSIVE_LEVEL)
NTSTATUS VadRemoveChild(WDFDEVICE root, uint64_t id,
    MOIREN_VAD_ENDPOINT_SNAPSHOT* snapshot);
_IRQL_requires_(PASSIVE_LEVEL)
// Attempts all retained children and returns the first submission failure.
NTSTATUS VadRevokeSession(WDFDEVICE root);
VAD_PAGE NTSTATUS VadPreparePolicy(WDFDEVICE child, uint32_t intent);
VAD_PAGE void VadReleasePolicy(WDFDEVICE child);
VAD_PAGE NTSTATUS VadInitializePolicy(WDFDEVICE child);
void VadAudioSetReady(WDFDEVICE child, bool ready);
NTSTATUS VadSetGate(WDFDEVICE root, const MOIREN_VAD_GATE_REQUEST& request,
    MOIREN_VAD_ENDPOINT_SNAPSHOT* snapshot);

// Native stream lifetime is independent of the Engine PCM gate.
NTSTATUS VadAudioAcquireStream(WDFDEVICE child, uint64_t* generation);
bool VadAudioIsStreamCurrent(WDFDEVICE child, uint64_t generation);
void VadAudioReleaseStream(WDFDEVICE child);
void VadAudioDescribeStream(WDFDEVICE child, uint64_t generation,
    ULONG blockAlign, ULONG frames, ULONG state);
void VadAudioRecordRun(WDFDEVICE child, uint64_t generation, uint64_t qpc);
void VadAudioPrepareFailed(WDFDEVICE child, NTSTATUS status);

struct VAD_PCM_REQUEST {
    MOIREN_VAD_PCM_HEADER Header;
    PVOID Buffer;
    VAD_SHARED* Shared; // Root/queue keep this alive through request completion.
    ULONG Index;
    volatile LONG Counted;
};
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(VAD_PCM_REQUEST, VadPcmRequest)
VAD_PAGE NTSTATUS VadPcmInitialize(WDFDEVICE device);
VAD_PAGE void VadPcmShutdown(VAD_SHARED* shared);
VAD_PAGE NTSTATUS VadPcmSubmit(WDFDEVICE device, WDFREQUEST request, ULONG code);
VAD_PAGE void VadPcmPurge(VAD_SHARED* shared, ULONG index, bool restart);
struct VAD_PCM_COMPLETION {
    WDFREQUEST Request;
    NTSTATUS Status;
    ULONG Information;
};
struct VAD_PCM_BATCH {
    vad::CompletionBatch<VAD_PCM_COMPLETION, MOIREN_VAD_REQUEST_SLOTS> Completions;
    VAD_SHARED* Shared; // Owns PCM rundown until completion, not just until copy.
};
// DPC-safe bounded packet copy. Caller must finish the batch after releasing
// ALL spin locks. Neither copying nor collecting completes an IRP.
void VadPcmTransfer(WDFDEVICE child, uint64_t generation, PVOID buffer,
    ULONG frames, ULONG blockAlign, uint64_t sequence, uint64_t qpc,
    VAD_PCM_BATCH* batch);
void VadPcmComplete(VAD_PCM_BATCH* batch);
void VadPcmDiscontinuity(WDFDEVICE child, uint64_t generation, bool pauseStream = false);
// Caller holds Shared->Lock and a PCM rundown reference. Retrieved requests
// always enter the batch and complete after all spin locks are released.
void VadPcmTransferLocked(VAD_SHARED* shared, ULONG index, PVOID buffer,
    ULONG frames, ULONG blockAlign, uint64_t sequence, uint64_t qpc,
    VAD_PCM_BATCH* batch);
