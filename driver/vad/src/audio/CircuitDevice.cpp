// Copyright (c) Microsoft Corporation. All rights reserved.
// Portions Copyright (c) 2026 Moiren contributors.
// SPDX-License-Identifier: MS-PL
// Derived initialization sequence: SdcaVad/SdcaVXu/CircuitDevice.cpp.
#include "Private.h"
#include "CircuitDevice.tmh"

VAD_PAGE static EVT_WDF_DEVICE_PREPARE_HARDWARE Prepare;
VAD_PAGE static EVT_WDF_DEVICE_RELEASE_HARDWARE Release;

VAD_PAGE _Use_decl_annotations_
NTSTATUS VadAudioConfigureInit(PWDFDEVICE_INIT init)
{
    PAGED_CODE();
    ACX_DEVICEINIT_CONFIG config;
    ACX_DEVICEINIT_CONFIG_INIT(&config);
    config.Flags |= AcxDeviceInitConfigRawDevice;
    NTSTATUS status = AcxDeviceInitInitialize(init, &config);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    WDF_PNPPOWER_EVENT_CALLBACKS callbacks;
    WDF_PNPPOWER_EVENT_CALLBACKS_INIT(&callbacks);
    callbacks.EvtDevicePrepareHardware = Prepare;
    callbacks.EvtDeviceReleaseHardware = Release;
    WdfDeviceInitSetPnpPowerEventCallbacks(init, &callbacks);
    return STATUS_SUCCESS;
}

VAD_PAGE _Use_decl_annotations_
NTSTATUS VadAudioInitialize(WDFDEVICE child, uint32_t intent, uint32_t channels,
    const MOIREN_VAD_ENDPOINT_ID& stableId)
{
    PAGED_CODE();
    if (!vad::ValidEndpoint(intent, channels)) {
        return STATUS_INVALID_PARAMETER;
    }
    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, VAD_AUDIO_DEVICE);
    VAD_AUDIO_DEVICE* device = nullptr;
    NTSTATUS status = WdfObjectAllocateContext(child, &attributes,
        reinterpret_cast<PVOID*>(&device));
    if (!NT_SUCCESS(status)) {
        return status;
    }
    device->Device = child;
    device->Intent = intent;
    device->Channels = channels;
    device->Identity = vad::audio::CircuitInstanceIdentity(stableId, intent);
    KeInitializeMutex(&device->Lifecycle, 0);
    KeInitializeSpinLock(&device->State);
    ACX_DEVICE_CONFIG config;
    ACX_DEVICE_CONFIG_INIT(&config);
    status = AcxDeviceInitialize(child, &config);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    // Circuit lifetime follows Prepare/ReleaseHardware, not just PDO creation.
    return STATUS_SUCCESS;
}

VAD_PAGE static void DeleteUnpublishedCircuit(VAD_AUDIO_DEVICE* device)
{
    PAGED_CODE();
    VadAudioLock(device);
    NT_ASSERT(!device->Added);
    const auto circuit = device->Circuit;
    device->Circuit = nullptr;
    device->HostPin = nullptr;
    VadAudioUnlock(device);
    if (circuit != nullptr) {
        // Covers partial pin/format construction too. Never delete under
        // Lifecycle: ACX object cleanup may synchronously enter callbacks.
        WdfObjectDelete(circuit);
    }
}

VAD_PAGE _Use_decl_annotations_
static NTSTATUS Prepare(WDFDEVICE child, WDFCMRESLIST raw,
    WDFCMRESLIST translated)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(raw);
    UNREFERENCED_PARAMETER(translated);
    auto device = VadAudioDevice(child);
    VadAudioLock(device);
    const bool added = device->Added != FALSE;
    device->HardwareReady = added;
    VadAudioUnlock(device);
    if (added) {
        VadAudioSetReady(child, true);
        return STATUS_SUCCESS;
    }
    // The PDO has a devnode now. Stage all default-qualification properties
    // on a system worker before ACX publishes any audio interface.
    NTSTATUS status = VadPreparePolicy(child, device->Intent);
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
        "Audio prepare policy intent=%u status=%!STATUS!", device->Intent, status);
    if (NT_SUCCESS(status)) {
        status = VadAudioBuildCircuit(device);
        VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
            "Audio prepare build intent=%u status=%!STATUS!", device->Intent, status);
    }
    if (!NT_SUCCESS(status)) {
        DeleteUnpublishedCircuit(device);
        VadAudioPrepareFailed(child, status);
        return status;
    }
    // ACX may enter power callbacks synchronously. Never hold Lifecycle across
    // Add/RemoveCircuit (or root Add/RemoveCircuitDevice).
    status = AcxDeviceAddCircuit(child, device->Circuit);
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
        "Audio prepare publish intent=%u status=%!STATUS!", device->Intent, status);
    VadAudioLock(device);
    device->Added = NT_SUCCESS(status);
    device->HardwareReady = NT_SUCCESS(status);
    if (!NT_SUCCESS(status)) {
        device->HardwareReady = FALSE;
        device->Powered = FALSE;
    }
    VadAudioUnlock(device);
    if (NT_SUCCESS(status)) {
        VadAudioSetReady(child, true);
    } else {
        DeleteUnpublishedCircuit(device);
        VadAudioPrepareFailed(child, status);
    }
    return status;
}

VAD_PAGE _Use_decl_annotations_
static NTSTATUS Release(WDFDEVICE child, WDFCMRESLIST translated)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(translated);
    auto device = VadAudioDevice(child);
    VadAudioSetReady(child, false);
    VadAudioLock(device);
    device->HardwareReady = FALSE;
    device->Powered = FALSE;
    VadAudioStopDeviceLocked(device);
    const bool added = device->Added != FALSE;
    const auto circuit = device->Circuit;
    VadAudioUnlock(device);
    if (!added) {
        DeleteUnpublishedCircuit(device);
        return STATUS_SUCCESS;
    }
    NTSTATUS status = AcxDeviceRemoveCircuit(child, circuit);
    if (NT_SUCCESS(status)) {
        VadAudioLock(device);
        // RemoveCircuit deletes the circuit and its pins. A later Prepare
        // must build a complete fresh tree; neither handle can be reused.
        device->Added = FALSE;
        device->Circuit = nullptr;
        device->HostPin = nullptr;
        VadAudioUnlock(device);
    }
    return status;
}
