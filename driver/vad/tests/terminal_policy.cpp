// SPDX-License-Identifier: MIT
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <initializer_list>
#include "../include/moiren_vad.h"

// No WDF/ACX or device access: capture the actual builder's publication boundary.
using NTSTATUS = std::int32_t;
using ULONG = std::uint32_t;
struct GUID { std::uint32_t Data1; std::uint16_t Data2, Data3; std::uint8_t Data4[8]; };
constexpr GUID KSCATEGORY_AUDIO =
    {0x6994ad04, 0x93ef, 0x11d0, {0xa3, 0xcc, 0x00, 0xa0, 0xc9, 0x22, 0x31, 0x96}};
[[maybe_unused]] constexpr GUID KSNODETYPE_MICROPHONE =
    {0xdff21be1, 0xf70f, 0x11d0, {0xb9, 0x17, 0x00, 0xa0, 0xc9, 0x22, 0x31, 0x96}};
[[maybe_unused]] constexpr GUID KSNODETYPE_SPEAKER =
    {0xdff21ce1, 0xf70f, 0x11d0, {0xb9, 0x17, 0x00, 0xa0, 0xc9, 0x22, 0x31, 0x96}};
constexpr NTSTATUS STATUS_SUCCESS = 0;
constexpr NTSTATUS STATUS_INSUFFICIENT_RESOURCES = -1;
constexpr int TRACE_LEVEL_INFORMATION = 4, VAD_PDO = 1;
constexpr ULONG VadAudioHostPin = 0, VadAudioBridgePin = 1;
enum { AcxCircuitTypeCapture, AcxCircuitTypeRender, AcxPinTypeSource, AcxPinTypeSink,
    AcxPinCommunicationSink, AcxPinCommunicationNone, WdfExecutionLevelPassive,
    AcxConnTypeOtherDigital, AcxGeoLocNotApplicable, AcxGenLocOther, AcxPortConnIntegratedDevice };
struct VAD_AUDIO_DEVICE {
    std::uint32_t Intent;
    void* Device = nullptr;
    struct { wchar_t Uri[46] = {}; } Identity;
    wchar_t Reference[32] = {};
    void* Circuit = nullptr;
    void* HostPin = nullptr;
};
using ACXPIN = void*;
using ACXJACK = void*;
using PACXCIRCUIT_INIT = void*;
struct UNICODE_STRING { const wchar_t* Buffer; };
struct ACX_PIN_CALLBACKS { int EvtAcxPinSetDataFormat = 0, EvtAcxPinRetrieveName = 0; };
struct ACX_PIN_CONFIG {
    ULONG Id = 0;
    int Type = 0, Communication = 0;
    const GUID* Category = nullptr;
    ULONG MaxStreams = 0;
    ACX_PIN_CALLBACKS* PinCallbacks = nullptr;
};
struct WDF_OBJECT_ATTRIBUTES { int ExecutionLevel = 0; void* ParentObject = nullptr; };
struct ACX_CIRCUIT_PNPPOWER_CALLBACKS { int EvtAcxCircuitPowerUp = 0, EvtAcxCircuitPowerDown = 0; };
struct ACX_JACK_CONFIG {
    struct { ULONG ChannelMapping = 0; int ConnectionType = 0, GeoLocation = 0,
        GenLocation = 0, PortConnection = 0; } Description;
};
constexpr int PowerUp = 1, PowerDown = 2, SetFormat = 3, PinName = 4, VadAudioCreateStream = 5;
#define NT_SUCCESS(status) ((status) >= 0)
#define PAGED_CODE() ((void)0)
#define RTL_NUMBER_OF(value) (sizeof(value) / sizeof((value)[0]))
#define ACX_PIN_CALLBACKS_INIT(value) (*(value) = {})
#define WDF_OBJECT_ATTRIBUTES_INIT(value) (*(value) = {})
#define ACX_CIRCUIT_PNPPOWER_CALLBACKS_INIT(value) (*(value) = {})
#define ACX_JACK_CONFIG_INIT(value) (*(value) = {})
struct {
    unsigned PinCount = 0;
    GUID Categories[2] = {};
    ULONG Ids[2] = {};
    int Types[2] = {}, Communications[2] = {}, NameCallback = 0, CircuitType = -1;
    unsigned Formats = 0, Published = 0;
} F;
static void Check(bool condition, const char* message)
{
    if (!condition) { std::fprintf(stderr, "FAIL: %s\n", message); std::exit(1); }
}
template<typename... Args> void VadTrace(Args...) {}
void RtlInitUnicodeString(UNICODE_STRING* value, const wchar_t* text) { value->Buffer = text; }
void ACX_PIN_CONFIG_INIT_ID(ACX_PIN_CONFIG* value, ULONG id) { *value = {}; value->Id = id; }
PACXCIRCUIT_INIT AcxCircuitInitAllocate(void*) { return &F; }
NTSTATUS AcxCircuitInitAssignComponentUri(void*, UNICODE_STRING*) { return STATUS_SUCCESS; }
NTSTATUS AcxCircuitInitAssignName(void*, UNICODE_STRING*) { return STATUS_SUCCESS; }
void AcxCircuitInitFree(void*) {}
void AcxCircuitInitSetCircuitType(void*, int type) { F.CircuitType = type; }
void AcxCircuitInitSetAcxCircuitPnpPowerCallbacks(void*, ACX_CIRCUIT_PNPPOWER_CALLBACKS*) {}
NTSTATUS AcxCircuitInitAssignAcxCreateStreamCallback(void*, int) { return STATUS_SUCCESS; }
NTSTATUS AcxCircuitCreate(void*, WDF_OBJECT_ATTRIBUTES*, void** init, void** circuit)
{ *init = nullptr; *circuit = &F; return STATUS_SUCCESS; }
NTSTATUS AcxPinCreate(void*, WDF_OBJECT_ATTRIBUTES*, ACX_PIN_CONFIG* config, ACXPIN* pin)
{
    Check(F.PinCount < 2 && config->Category != nullptr, "builder emits two categorized pins");
    const auto index = F.PinCount++;
    F.Categories[index] = *config->Category;
    F.Ids[index] = config->Id;
    F.Types[index] = config->Type;
    F.Communications[index] = config->Communication;
    if (index == 1) { F.NameCallback = config->PinCallbacks->EvtAcxPinRetrieveName; }
    *pin = &F.Categories[index];
    return STATUS_SUCCESS;
}
NTSTATUS AcxJackCreate(void*, WDF_OBJECT_ATTRIBUTES*, ACX_JACK_CONFIG*, ACXJACK* jack)
{ *jack = &F; return STATUS_SUCCESS; }
NTSTATUS AcxPinAddJacks(void*, ACXJACK*, unsigned) { return STATUS_SUCCESS; }
NTSTATUS VadAudioAddFormats(VAD_AUDIO_DEVICE*, ACXPIN) { ++F.Formats; return STATUS_SUCCESS; }
NTSTATUS AcxCircuitAddPins(void*, ACXPIN*, unsigned count)
{ F.Published = count; return STATUS_SUCCESS; }

#include "terminal-policy-production.inc"

int main()
{
    for (auto intent : {MOIREN_VAD_INTENT_PLAYBACK, MOIREN_VAD_INTENT_MICROPHONE}) {
        F = {};
        VAD_AUDIO_DEVICE device{};
        device.Intent = intent;
        Check(VadAudioBuildCircuit(&device) == STATUS_SUCCESS, "production circuit builds");
        const bool capture = intent == MOIREN_VAD_INTENT_MICROPHONE;
        const GUID expected = {capture ? 0xdff21be1u : 0xdff21ce0u, 0xf70f, 0x11d0,
            {0xb9, 0x17, 0x00, 0xa0, 0xc9, 0x22, 0x31, 0x96}};
        Check(std::memcmp(&F.Categories[1], &expected, sizeof(GUID)) == 0,
            "ACX virtual playback publishes OUTPUT_UNDEFINED, not SPEAKER; MIC is unchanged");
        Check(std::memcmp(&F.Categories[0], &KSCATEGORY_AUDIO, sizeof(GUID)) == 0 &&
            F.Ids[0] == 0 && F.Ids[1] == 1 && F.Published == 2 && F.Formats == 1,
            "host category, pin IDs, format admission and two-pin publication are unchanged");
        Check(F.Types[0] == (capture ? AcxPinTypeSource : AcxPinTypeSink) &&
            F.Types[1] == (capture ? AcxPinTypeSink : AcxPinTypeSource) &&
            F.Communications[0] == AcxPinCommunicationSink &&
            F.Communications[1] == AcxPinCommunicationNone && F.NameCallback == PinName &&
            F.CircuitType == (capture ? AcxCircuitTypeCapture : AcxCircuitTypeRender),
            "dataflow, communication, retrieve-name callback and circuit direction are unchanged");
    }
    std::puts("PASS: actual ACX terminal categories, pin IDs, direction and publication contract");
}
