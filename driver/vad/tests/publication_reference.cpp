// SPDX-License-Identifier: MIT
// Reuse the actual policy body/DDI fixture, not a second policy implementation.
#define VAD_PUBLICATION_REFERENCE_TEST
#include "interface_name_native.cpp"

constexpr NTSTATUS STATUS_INSUFFICIENT_RESOURCES = -7;
constexpr ULONG VadAudioHostPin = 0, VadAudioBridgePin = 1;
constexpr GUID KSCATEGORY_AUDIO =
    {0x6994ad04, 0x93ef, 0x11d0, {0xa3, 0xcc, 0x00, 0xa0, 0xc9, 0x22, 0x31, 0x96}};
constexpr bool TRUE = true;
enum { AcxCircuitTypeCapture, AcxCircuitTypeRender, AcxPinTypeSource, AcxPinTypeSink,
    AcxPinCommunicationSink, AcxPinCommunicationNone, WdfExecutionLevelPassive,
    AcxConnTypeOtherDigital, AcxGeoLocNotApplicable, AcxGenLocOther, AcxPortConnIntegratedDevice };
struct VAD_AUDIO_DEVICE {
    WDFDEVICE Device = nullptr;
    std::uint32_t Intent = MOIREN_VAD_INTENT_PLAYBACK;
    struct { WCHAR Uri[46] = {}; } Identity;
    void* Circuit = nullptr;
    void* HostPin = nullptr;
    bool Added = false, HardwareReady = false, Powered = false;
};
using ACXPIN = void*;
using ACXJACK = void*;
using PACXCIRCUIT_INIT = void*;
using WDFCMRESLIST = void*;
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
#define ACX_PIN_CALLBACKS_INIT(value) (*(value) = {})
#define WDF_OBJECT_ATTRIBUTES_INIT(value) (*(value) = {})
#define ACX_CIRCUIT_PNPPOWER_CALLBACKS_INIT(value) (*(value) = {})
#define ACX_JACK_CONFIG_INIT(value) (*(value) = {})
struct CircuitFixture {
    unsigned Allocated = 0, Added = 0, Pins = 0;
    NTSTATUS Failure = STATUS_SUCCESS;
    PCWSTR Reference = nullptr;
    bool Ready = false;
} C;
static VAD_AUDIO_DEVICE Audio;
VAD_AUDIO_DEVICE* VadAudioDevice(WDFDEVICE child)
{ Check(Audio.Device == child, "audio context belongs to the same child"); return &Audio; }
void VadAudioLock(VAD_AUDIO_DEVICE*) { Check(!SharedLocked, "no Shared lock at passive lifecycle"); }
void VadAudioUnlock(VAD_AUDIO_DEVICE*) {}
void VadAudioSetReady(WDFDEVICE, bool ready) { C.Ready = ready; }
void VadAudioPrepareFailed(WDFDEVICE, NTSTATUS status) { C.Failure = status; }
void WdfObjectDelete(void*) {}
PACXCIRCUIT_INIT AcxCircuitInitAllocate(WDFDEVICE) { ++C.Allocated; return &C; }
NTSTATUS AcxCircuitInitAssignComponentUri(void*, UNICODE_STRING*) { return STATUS_SUCCESS; }
NTSTATUS AcxCircuitInitAssignName(void*, UNICODE_STRING* name)
{ C.Reference = name->Buffer; return STATUS_SUCCESS; }
void AcxCircuitInitFree(void*) {}
void AcxCircuitInitSetCircuitType(void*, int) {}
void AcxCircuitInitSetAcxCircuitPnpPowerCallbacks(void*, ACX_CIRCUIT_PNPPOWER_CALLBACKS*) {}
NTSTATUS AcxCircuitInitAssignAcxCreateStreamCallback(void*, int) { return STATUS_SUCCESS; }
NTSTATUS AcxCircuitCreate(WDFDEVICE, WDF_OBJECT_ATTRIBUTES*, void** init, void** circuit)
{ *init = nullptr; *circuit = &C; return STATUS_SUCCESS; }
void ACX_PIN_CONFIG_INIT_ID(ACX_PIN_CONFIG* config, ULONG id) { *config = {}; config->Id = id; }
NTSTATUS AcxPinCreate(void*, WDF_OBJECT_ATTRIBUTES*, ACX_PIN_CONFIG*, ACXPIN* pin)
{ *pin = &C; return STATUS_SUCCESS; }
NTSTATUS AcxJackCreate(void*, WDF_OBJECT_ATTRIBUTES*, ACX_JACK_CONFIG*, ACXJACK* jack)
{ *jack = &C; return STATUS_SUCCESS; }
NTSTATUS AcxPinAddJacks(void*, ACXJACK*, unsigned) { return STATUS_SUCCESS; }
NTSTATUS VadAudioAddFormats(VAD_AUDIO_DEVICE*, ACXPIN) { return STATUS_SUCCESS; }
NTSTATUS AcxCircuitAddPins(void*, ACXPIN*, unsigned count) { C.Pins = count; return STATUS_SUCCESS; }
NTSTATUS AcxDeviceAddCircuit(WDFDEVICE, void*) { ++C.Added; return STATUS_SUCCESS; }

#include "publication-reference-production.inc"

static void Reset(Child& child)
{
    F = {};
    C = {};
    Audio = {};
    Audio.Device = &child;
    Audio.Intent = child.Shared.Slots[0].Intent;
}

#include "identity_control.inc"

static void PolicyReferenceSurvivesNameChangeBeforeCircuit()
{
    for (auto intent : {MOIREN_VAD_INTENT_PLAYBACK, MOIREN_VAD_INTENT_MICROPHONE}) {
        Child child;
        child.Shared.Slots[0].Intent = intent;
        Reset(child);
        Check(VadPreparePolicy(&child, intent) == STATUS_SUCCESS, "actual policy publishes");
        WCHAR retained[vad::EndpointReferenceUnits] = {};
        std::memcpy(retained, child.Context.PolicyReference, sizeof(retained));
        const WCHAR changed[MOIREN_VAD_MAX_NAME_UNITS] = L"Changed before circuit";
        std::memcpy(child.Shared.Slots[0].Name, changed, sizeof(changed));
        WCHAR other[vad::EndpointReferenceUnits] = {};
        vad::EndpointReference(changed, other);
        Check(std::wcscmp(retained, other) != 0, "changed slot has a different historical hash");
        Check(Prepare(&child, nullptr, nullptr) == STATUS_SUCCESS, "actual prepare builds and adds");
        Check(C.Allocated == 1 && C.Added == 1 && C.Pins == 2 && C.Ready,
            "normal ACX publication follows successful policy");
        Check(F.Registered == 3 && C.Reference == child.Context.PolicyReference &&
            std::wcscmp(C.Reference, retained) == 0,
            "ACX consumes exact retained Policy reference, never the changed live name");
        for (auto reference : F.References) {
            Check(reference == C.Reference, "Audio/flow/Realtime and ACX use identical storage");
        }
        Release(child);
        C = {};
        Check(VadAudioBuildCircuit(&Audio) == STATUS_INVALID_DEVICE_STATE && C.Allocated == 0,
            "released reference cannot authorize another ACX allocation");
    }
}

static void RecreatedChildUsesAdoptedReferenceInAcx()
{
    for (auto intent : {MOIREN_VAD_INTENT_PLAYBACK, MOIREN_VAD_INTENT_MICROPHONE}) {
        IdentityStorage devnode;
        const WCHAR legacy[vad::EndpointReferenceUnits] = L"MoirenEndpoint";
        for (auto& category : devnode.References) {
            std::memcpy(category[0], legacy, sizeof(legacy));
        }
        Child original;
        original.Store = &devnode;
        original.Shared.Slots[0].Intent = intent;
        Reset(original);
        Check(Prepare(&original, nullptr, nullptr) == STATUS_SUCCESS &&
            std::wcscmp(C.Reference, legacy) == 0 && F.IdentityWrites == 1,
            "actual Prepare adopts, persists and publishes the original short reference into ACX");
        Release(original);
        Child renamed;
        renamed.Store = &devnode;
        renamed.Shared.Slots[0].Intent = intent;
        const WCHAR changed[MOIREN_VAD_MAX_NAME_UNITS] = L"Renamed after runtime destruction";
        std::memcpy(renamed.Shared.Slots[0].Name, changed, sizeof(changed));
        Reset(renamed);
        Check(Prepare(&renamed, nullptr, nullptr) == STATUS_SUCCESS && F.IdentityWrites == 0 &&
            F.Enumerated == 0 && C.Reference == renamed.Context.PolicyReference &&
            std::wcscmp(C.Reference, legacy) == 0 && F.IdentityDurable && C.Added == 1,
            "new runtime and new name keep the exact persisted interface/ACX identity");
        for (unsigned category = 0; category < 3; ++category) {
            Check(F.References[category] == C.Reference &&
                std::wcscmp(F.Names[category], changed) == 0,
                "all three categories publish mutable names using the retained exact ACX reference");
        }
        Release(renamed);
    }
}

static void PolicyFailureNeverBuildsOrAddsCircuit()
{
    for (unsigned category = 1; category <= 3; ++category) {
        for (unsigned boundary = 0; boundary < 4; ++boundary) {
            Child child;
            Reset(child);
            if (boundary == 0) { F.FailSetAt = category; }
            if (boundary == 1) { F.FailReadAt = category; }
            if (boundary == 2) { F.FailRegisterAt = category; }
            if (boundary == 3) { F.FailRegistryAt = 1 + (category - 1) * 10; }
            const auto expected = boundary == 0 ? SetFailure :
                (boundary == 3 ? RegistryFailure : ReadFailure);
            Check(Prepare(&child, nullptr, nullptr) == expected && C.Failure == expected,
                "actual Prepare preserves original policy failure");
            Check(child.Context.PolicyReference[0] == L'\0' &&
                child.Context.PolicyInterfaceCount == 0 &&
                C.Allocated == 0 && C.Added == 0 && !C.Ready,
                "partial policy clears reference and cannot build/add any circuit");
            Check(VadAudioBuildCircuit(&Audio) == STATUS_INVALID_DEVICE_STATE && C.Allocated == 0,
                "direct builder also refuses the failed reference before allocation");
            Release(child);
        }
    }
}

int main()
{
    FreshReservationRetryKeepsExactReference();
    LegacyRecreationCannotRotateCommittedIdentity();
    ActualControlNegotiatesAndSelectsPublicationMode();
    ActualControlRejectsMalformedIdentityBeforeStorageOrChild();
    ExplicitFreshNeverReplacesInstalledIdentityOrMasksDiscoveryFailure();
    RootTokenIsDurableAndNeverRotatesOnRetry();
    LegacyCreateDoesNotRequireIdentityProvisioning();
    ExplicitFreshAuthorityCreatesAndRetainsNewIdentity();
    AdoptedReferencePersistsAcrossRuntimeRename();
    AdoptionFailuresNeverPublishOrLeak();
    MissingAndAmbiguousHistoryCannotMintIdentity();
    PersistedRecordValidatesOwnershipAndSchema();
    FlushFailureRetryAndStageFailureRetainIdentity();
    TerminalPolicyMatchesVirtualFunctionAndDefaultPreference();
    EndpointReferenceDerivation();
    SuccessfulTypedPublication();
    LockedSnapshotOwnsPublicationReference();
    TypedFailureClosesPolicy();
    RejectReadbackMismatch();
    FixedNameBounds();
    ExistingFailureRollback();
    DisabledTraceDoesNotChoosePolicyOutcome();
    PolicyReferenceSurvivesNameChangeBeforeCircuit();
    PolicyFailureNeverBuildsOrAddsCircuit();
    RecreatedChildUsesAdoptedReferenceInAcx();
    std::puts("PASS: composed production Policy/Prepare/BuildCircuit reference authority and fail-closed gate");
}
