// Copyright (c) Microsoft Corporation. All rights reserved.
// Portions Copyright (c) 2026 Moiren contributors.
// SPDX-License-Identifier: MS-PL
// Derived ACX topology sequence: Samples/Common/{Render,Capture}Circuit.cpp.
#include "Private.h"
#include "Circuit.tmh"

static VAD_PAGE EVT_ACX_CIRCUIT_POWER_UP PowerUp;
static VAD_PAGE EVT_ACX_CIRCUIT_POWER_DOWN PowerDown;
static VAD_PAGE EVT_ACX_PIN_SET_DATAFORMAT SetFormat;
// ACX preallocates Name.Buffer/MaximumLength, unlike the typedef's _Out_ SAL.
_Function_class_(EVT_ACX_PIN_RETRIEVE_NAME)
_IRQL_requires_same_
_IRQL_requires_max_(PASSIVE_LEVEL)
static VAD_PAGE NTSTATUS PinName(_In_ ACXPIN pin, _Inout_ PUNICODE_STRING name);

static void CopyPinName(VAD_CHILD* child,
    WCHAR (&text)[MOIREN_VAD_MAX_NAME_UNITS])
{
    KIRQL irql;
    KeAcquireSpinLock(&child->Shared->Lock, &irql);
    RtlCopyMemory(text, child->Shared->Slots[child->Index].Name, sizeof(text));
    KeReleaseSpinLock(&child->Shared->Lock, irql);
}

_Use_decl_annotations_
static VAD_PAGE NTSTATUS PinName(ACXPIN pin, PUNICODE_STRING name)
{
    PAGED_CODE();
    const auto circuit = AcxPinGetCircuit(pin);
    const auto child = VadChild(AcxCircuitGetWdfDevice(circuit));
    WCHAR text[MOIREN_VAD_MAX_NAME_UNITS];
    CopyPinName(child, text);
    // ACX owns the output buffer; do not return a borrowed slot/stack string.
    return RtlUnicodeStringPrintf(name, L"%s", text);
}

_Use_decl_annotations_
static VAD_PAGE NTSTATUS SetFormat(ACXPIN pin, ACXDATAFORMAT format)
{
    PAGED_CODE();
    auto circuit = AcxPinGetCircuit(pin);
    auto device = VadAudioDevice(AcxCircuitGetWdfDevice(circuit));
    if (pin != device->HostPin) {
        return STATUS_INVALID_PARAMETER;
    }
    ULONG align = 0;
    return VadAudioValidateFormat(device, format, &align);
}

VAD_PAGE NTSTATUS VadAudioBuildCircuit(VAD_AUDIO_DEVICE* device)
{
    PAGED_CODE();
    const bool capture = device->Intent == MOIREN_VAD_INTENT_MICROPHONE;
    PACXCIRCUIT_INIT init = AcxCircuitInitAllocate(device->Device);
    if (init == nullptr) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    UNICODE_STRING uri;
    RtlInitUnicodeString(&uri, device->Identity.Uri);
    NTSTATUS status = AcxCircuitInitAssignComponentUri(init, &uri);
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
        "Circuit component URI status=%!STATUS!", status);
    if (!NT_SUCCESS(status)) {
        AcxCircuitInitFree(init);
        return status;
    }
    UNICODE_STRING name;
    RtlInitUnicodeString(&name, VadAudioReference);
    status = AcxCircuitInitAssignName(init, &name);
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
        "Circuit name status=%!STATUS!", status);
    if (!NT_SUCCESS(status)) {
        AcxCircuitInitFree(init);
        return status;
    }
    AcxCircuitInitSetCircuitType(init,
        capture ? AcxCircuitTypeCapture : AcxCircuitTypeRender);
    ACX_CIRCUIT_PNPPOWER_CALLBACKS power;
    ACX_CIRCUIT_PNPPOWER_CALLBACKS_INIT(&power);
    power.EvtAcxCircuitPowerUp = PowerUp;
    power.EvtAcxCircuitPowerDown = PowerDown;
    AcxCircuitInitSetAcxCircuitPnpPowerCallbacks(init, &power);
    status = AcxCircuitInitAssignAcxCreateStreamCallback(init, VadAudioCreateStream);
    if (!NT_SUCCESS(status)) {
        AcxCircuitInitFree(init);
        return status;
    }
    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.ExecutionLevel = WdfExecutionLevelPassive;
    status = AcxCircuitCreate(device->Device, &attributes, &init, &device->Circuit);
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
        "Circuit create status=%!STATUS!", status);
    if (!NT_SUCCESS(status)) {
        if (init != nullptr) {
            AcxCircuitInitFree(init);
        }
        return status;
    }
    ACXPIN pins[2] = {};
    ACX_PIN_CONFIG config;
    ACX_PIN_CALLBACKS callbacks;
    ACX_PIN_CALLBACKS_INIT(&callbacks);
    callbacks.EvtAcxPinSetDataFormat = SetFormat;
    ACX_PIN_CONFIG_INIT_ID(&config, VadAudioHostPin);
    config.Type = capture ? AcxPinTypeSource : AcxPinTypeSink;
    config.Communication = AcxPinCommunicationSink;
    config.Category = &KSCATEGORY_AUDIO;
    config.MaxStreams = 1;
    config.PinCallbacks = &callbacks;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.ParentObject = device->Circuit;
    status = AcxPinCreate(device->Circuit, &attributes, &config, &pins[0]);
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
        "Circuit host pin status=%!STATUS!", status);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    device->HostPin = pins[0];
    ACX_PIN_CONFIG_INIT_ID(&config, VadAudioBridgePin);
    config.Type = capture ? AcxPinTypeSink : AcxPinTypeSource;
    config.Communication = AcxPinCommunicationNone;
    config.Category = capture ? &KSNODETYPE_MICROPHONE : &KSNODETYPE_SPEAKER;
    ACX_PIN_CALLBACKS bridgeCallbacks;
    ACX_PIN_CALLBACKS_INIT(&bridgeCallbacks);
    bridgeCallbacks.EvtAcxPinRetrieveName = PinName;
    config.PinCallbacks = &bridgeCallbacks;
    status = AcxPinCreate(device->Circuit, &attributes, &config, &pins[1]);
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
        "Circuit bridge pin status=%!STATUS!", status);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    ACX_JACK_CONFIG jackConfig;
    ACX_JACK_CONFIG_INIT(&jackConfig);
    // Integrated endpoints have no jack-detection capability or presence
    // callback. ACX supplies the always-connected state for this description.
    // Jack speaker mapping is only for analog render connectors, not digital
    // render or capture. The actual PCM channel mask lives in each format.
    jackConfig.Description.ChannelMapping = 0;
    jackConfig.Description.ConnectionType = AcxConnTypeOtherDigital;
    jackConfig.Description.GeoLocation = AcxGeoLocNotApplicable;
    jackConfig.Description.GenLocation = AcxGenLocOther;
    jackConfig.Description.PortConnection = AcxPortConnIntegratedDevice;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.ParentObject = pins[1];
    ACXJACK jack = nullptr;
    status = AcxJackCreate(pins[1], &attributes, &jackConfig, &jack);
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
        "Circuit jack status=%!STATUS!", status);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    status = AcxPinAddJacks(pins[1], &jack, 1);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    status = VadAudioAddFormats(device, pins[0]);
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
        "Circuit formats status=%!STATUS!", status);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    status = AcxCircuitAddPins(device->Circuit, pins, RTL_NUMBER_OF(pins));
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
        "Circuit add pins status=%!STATUS!", status);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    // A single circuit has no element to connect; ACX routes the host pin to
    // the bridge pin in insertion order. Explicit self-connections are for
    // element graphs and would incorrectly identify a pin as both endpoints.
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
static VAD_PAGE NTSTATUS PowerUp(WDFDEVICE child, ACXCIRCUIT circuit,
    WDF_POWER_DEVICE_STATE previous)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(circuit);
    UNREFERENCED_PARAMETER(previous);
    auto device = VadAudioDevice(child);
    VadAudioLock(device);
    device->Powered = TRUE;
    VadAudioUnlock(device);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
static VAD_PAGE NTSTATUS PowerDown(WDFDEVICE child, ACXCIRCUIT circuit,
    WDF_POWER_DEVICE_STATE target)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(circuit);
    UNREFERENCED_PARAMETER(target);
    auto device = VadAudioDevice(child);
    VadAudioLock(device);
    device->Powered = FALSE;
    VadAudioStopDeviceLocked(device);
    VadAudioUnlock(device);
    return STATUS_SUCCESS;
}
