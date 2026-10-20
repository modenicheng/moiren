// Copyright (c) Microsoft Corporation. All rights reserved.
// Portions Copyright (c) 2026 Moiren contributors.
// SPDX-License-Identifier: MS-PL
// Derived ACX topology sequence: Samples/Common/{Render,Capture}Circuit.cpp.
#include "Private.h"

static VAD_PAGE EVT_ACX_CIRCUIT_POWER_UP PowerUp;
static VAD_PAGE EVT_ACX_CIRCUIT_POWER_DOWN PowerDown;
static VAD_PAGE EVT_ACX_PIN_SET_DATAFORMAT SetFormat;
static VAD_PAGE EVT_ACX_JACK_RETRIEVE_PRESENCE_STATE JackPresent;

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

_Use_decl_annotations_
static VAD_PAGE NTSTATUS JackPresent(ACXJACK jack, PBOOLEAN connected)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(jack);
    *connected = TRUE; // Integrated virtual endpoint, no presence detection.
    return STATUS_SUCCESS;
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
    if (!NT_SUCCESS(status)) {
        AcxCircuitInitFree(init);
        return status;
    }
    UNICODE_STRING name;
    RtlInitUnicodeString(&name, VadAudioReference);
    status = AcxCircuitInitAssignName(init, &name);
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
    if (!NT_SUCCESS(status)) {
        return status;
    }
    device->HostPin = pins[0];
    ACX_PIN_CONFIG_INIT_ID(&config, VadAudioBridgePin);
    config.Type = capture ? AcxPinTypeSink : AcxPinTypeSource;
    config.Communication = AcxPinCommunicationNone;
    config.Category = capture ? &KSNODETYPE_MICROPHONE : &KSNODETYPE_SPEAKER;
    status = AcxPinCreate(device->Circuit, &attributes, &config, &pins[1]);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    ACX_JACK_CONFIG jackConfig;
    ACX_JACK_CONFIG_INIT(&jackConfig);
    ACX_JACK_CALLBACKS jackCallbacks;
    ACX_JACK_CALLBACKS_INIT(&jackCallbacks);
    jackCallbacks.EvtAcxJackRetrievePresenceState = JackPresent;
    jackConfig.Callbacks = &jackCallbacks;
    jackConfig.Description.ChannelMapping = device->Channels == 1 ?
        SPEAKER_FRONT_CENTER : SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT;
    jackConfig.Description.ConnectionType = AcxConnTypeUnknown;
    jackConfig.Description.GeoLocation = AcxGeoLocNotApplicable;
    jackConfig.Description.GenLocation = AcxGenLocOther;
    jackConfig.Description.PortConnection = AcxPortConnIntegratedDevice;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.ParentObject = pins[1];
    ACXJACK jack = nullptr;
    status = AcxJackCreate(pins[1], &attributes, &jackConfig, &jack);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    status = AcxPinAddJacks(pins[1], &jack, 1);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    status = VadAudioAddFormats(device, pins[0]);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    status = AcxCircuitAddPins(device->Circuit, pins, RTL_NUMBER_OF(pins));
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
