// Root setup adapted from Microsoft AudioCodec Device.cpp at the pinned commit.
// Copyright (c) Microsoft Corporation. All rights reserved.
// SPDX-License-Identifier: MS-PL
#include "Driver.h"
#include "Device.tmh"

VAD_PAGE static NTSTATUS InitializeRoot(WDFDEVICE device);

VAD_PAGE _Use_decl_annotations_
NTSTATUS VadAddDevice(WDFDRIVER driver, PWDFDEVICE_INIT init)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(driver);
    ACX_DEVICEINIT_CONFIG acxInit;
    ACX_DEVICEINIT_CONFIG_INIT(&acxInit);
    NTSTATUS status = AcxDeviceInitInitialize(init, &acxInit);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    // The root is solely a control bus. It never adds a static audio circuit.
    WdfDeviceInitSetDeviceType(init, FILE_DEVICE_UNKNOWN);
    WdfDeviceInitSetCharacteristics(init, FILE_DEVICE_SECURE_OPEN, TRUE);
    WdfDeviceInitSetExclusive(init, TRUE);
    WDF_FILEOBJECT_CONFIG files;
    WDF_FILEOBJECT_CONFIG_INIT(&files, VadFileCreate, WDF_NO_EVENT_CALLBACK,
        VadFileCleanup);
    WDF_OBJECT_ATTRIBUTES fileAttributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&fileAttributes);
    fileAttributes.ExecutionLevel = WdfExecutionLevelPassive;
    fileAttributes.SynchronizationScope = WdfSynchronizationScopeNone;
    WdfDeviceInitSetFileObjectConfig(init, &files, &fileAttributes);
    WDF_OBJECT_ATTRIBUTES requestAttributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&requestAttributes, VAD_PCM_REQUEST);
    WdfDeviceInitSetRequestAttributes(init, &requestAttributes);
    WDF_PNPPOWER_EVENT_CALLBACKS pnp;
    WDF_PNPPOWER_EVENT_CALLBACKS_INIT(&pnp);
    pnp.EvtDeviceSurpriseRemoval = VadSurpriseRemoval;
    pnp.EvtDeviceSelfManagedIoCleanup = VadStop;
    WdfDeviceInitSetPnpPowerEventCallbacks(init, &pnp);
    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, VAD_ROOT);
    attributes.ExecutionLevel = WdfExecutionLevelPassive;
    attributes.SynchronizationScope = WdfSynchronizationScopeNone;
    attributes.EvtCleanupCallback = VadRootCleanup;
    WDFDEVICE device = nullptr;
    status = WdfDeviceCreate(&init, &attributes, &device);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    status = InitializeRoot(device);
    if (!NT_SUCCESS(status)) {
        auto root = VadRoot(device);
        InterlockedExchange(&root->Online, 0);
        // EvtDeviceAdd failure lets WDF delete children before root cleanup.
        // Drain even a partially created PCM queue set while handles are valid.
        if (root->Shared != nullptr) {
            VadPcmShutdown(root->Shared);
        }
    }
    return status;
}

VAD_PAGE static NTSTATUS InitializeRoot(WDFDEVICE device)
{
    PAGED_CODE();
    auto root = VadRoot(device);
    root->Shared = static_cast<VAD_SHARED*>(ExAllocatePool2(
        POOL_FLAG_NON_PAGED, sizeof(VAD_SHARED), 'daVM'));
    if (root->Shared == nullptr) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    root->Shared->References = 1;
    KeInitializeSpinLock(&root->Shared->Lock);
    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.ParentObject = device;
    NTSTATUS status = WdfWaitLockCreate(&attributes, &root->Admin);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    ACX_DEVICE_CONFIG acx;
    ACX_DEVICE_CONFIG_INIT(&acx);
    status = AcxDeviceInitialize(device, &acx);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    status = VadControlInitialize(device);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    WDF_IO_QUEUE_CONFIG queue;
    // ACX may own its default queue. Use a dedicated device-control queue.
    WDF_IO_QUEUE_CONFIG_INIT(&queue, WdfIoQueueDispatchSequential);
    queue.PowerManaged = WdfFalse;
    queue.EvtIoDeviceControl = VadIoControl;
    WDF_OBJECT_ATTRIBUTES queueAttributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&queueAttributes);
    // EvtIo only publishes a cancelable single-slot handoff. Its precreated
    // worker takes Admin and performs the passive management/drain DDIs.
    queueAttributes.ExecutionLevel = WdfExecutionLevelPassive;
    WDFQUEUE controlQueue;
    status = WdfIoQueueCreate(device, &queue, &queueAttributes,
        &controlQueue);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    root->ControlQueue = controlQueue;
    status = VadPcmInitialize(device);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    status = WdfDeviceConfigureRequestDispatching(device, controlQueue,
        WdfRequestTypeDeviceControl);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    // Device ACL is installed by INF. WDF enables only this control interface.
    status = WdfDeviceCreateDeviceInterface(device, &VadControlInterface, nullptr);
    if (NT_SUCCESS(status)) {
        InterlockedExchange(&root->Online, 1);
    }
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_INIT,
        "Control interface creation status=%!STATUS!", status);
    return status;
}

VAD_PAGE _Use_decl_annotations_
void VadFileCreate(WDFDEVICE device, WDFREQUEST request, WDFFILEOBJECT file)
{
    PAGED_CODE();
    auto root = VadRoot(device);
    WdfWaitLockAcquire(root->Admin, nullptr);
    NTSTATUS status = STATUS_SUCCESS;
    const auto name = WdfFileObjectGetFileName(file);
    if (name != nullptr && name->Length != 0) {
        status = STATUS_OBJECT_NAME_NOT_FOUND;
    } else if (InterlockedCompareExchange(&root->Online, 0, 0) == 0) {
        status = STATUS_DEVICE_REMOVED;
    } else if (InterlockedCompareExchange(&root->Releasing, 0, 0) != 0) {
        // Keep the short cleanup window exclusive. Otherwise a new owner
        // could open while the old owner's queued requests are being purged.
        status = STATUS_DEVICE_BUSY;
    } else if (root->Owner != nullptr) {
        status = STATUS_SHARING_VIOLATION;
    } else if (root->Session == MAXULONGLONG) {
        status = STATUS_INTEGER_OVERFLOW;
    }
    // Refused opens never sweep. Retry departed-owner failures before assigning
    // a new session, without waiting for accepted removals to reach destruction.
    status = vad::ClaimOwner(status, root->Owner, root->Session, file,
        [device] { return VadRevokeSession(device); },
        [] { return static_cast<uint64_t>(KeQueryPerformanceCounter(nullptr).QuadPart); });
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_SESSION,
        "Owner open session=%I64u status=%!STATUS!", root->Session, status);
    WdfWaitLockRelease(root->Admin);
    WdfRequestComplete(request, status);
}

VAD_PAGE _Use_decl_annotations_
void VadFileCleanup(WDFFILEOBJECT file)
{
    PAGED_CODE();
    WDFDEVICE device = WdfFileObjectGetDevice(file);
    auto root = VadRoot(device);
    bool revoke = false;
    WdfWaitLockAcquire(root->Admin, nullptr);
    if (root->Owner == file) {
        InterlockedExchange(&root->Releasing, 1);
        // Revoke admission first, before any asynchronous PnP operation.
        root->Owner = nullptr;
        VadTrace(TRACE_LEVEL_INFORMATION, VAD_SESSION,
            "Owner cleanup revoke session=%I64u", root->Session);
        (void)VadRevokeSession(device);
        revoke = true;
    }
    WdfWaitLockRelease(root->Admin);
    if (revoke && root->ControlQueue != nullptr) {
        // Requests already queued for this owner must not execute after its
        // file cleanup has revoked admission.
        WdfIoQueuePurgeSynchronously(root->ControlQueue);
        WdfWorkItemFlush(root->ControlWork);
        // Purge stops dispatch. Resume while Releasing still excludes a new
        // owner; no request can cross from the departed session into the next.
        if (InterlockedCompareExchange(&root->Online, 0, 0) != 0) {
            WdfIoQueueStart(root->ControlQueue);
        }
    }
    if (revoke) {
        InterlockedExchange(&root->Releasing, 0);
    }
}

_Use_decl_annotations_
void VadSurpriseRemoval(WDFDEVICE device)
{
    // Surprise removal is not serialized with the power-down callbacks and
    // may run above PASSIVE_LEVEL. Only latch the state here; self-managed
    // cleanup performs the lock-taking revocation and queue purge later.
    auto root = VadRoot(device);
    InterlockedExchange(&root->Online, 0);
}

VAD_PAGE _Use_decl_annotations_
void VadStop(WDFDEVICE device)
{
    PAGED_CODE();
    VadStopCore(device);
}

VAD_PAGE void VadStopCore(WDFDEVICE device)
{
    PAGED_CODE();
    auto root = VadRoot(device);
    WdfWaitLockAcquire(root->Admin, nullptr);
    InterlockedExchange(&root->Online, 0);
    root->Owner = nullptr;
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_SESSION,
        "Root stop revoke session=%I64u", root->Session);
    (void)VadRevokeSession(device);
    WdfWaitLockRelease(root->Admin);
    VadPcmShutdown(root->Shared);
    if (root->ControlQueue != nullptr) {
        // Requests not yet dispatched are still owned by WDF; purge them after
        // admission is revoked so no new work can enter.
        WdfIoQueuePurgeSynchronously(root->ControlQueue);
        WdfWorkItemFlush(root->ControlWork);
    }
}

void VadSharedRelease(VAD_SHARED* shared)
{
    if (InterlockedDecrement(&shared->References) == 0) {
        ExFreePoolWithTag(shared, 'daVM');
    }
}

_Use_decl_annotations_
void VadRootCleanup(WDFOBJECT object)
{
    auto root = VadRoot(object);
    if (root->Shared != nullptr) {
        // Normal self-managed cleanup, or EvtDeviceAdd failure unwind, already
        // drained PCM. Child queues may be deleted now: never touch them here.
        NT_ASSERT(!root->Shared->PcmInitialized || root->Shared->PcmOnline == 0);
        VadSharedRelease(root->Shared);
        root->Shared = nullptr;
    }
}
