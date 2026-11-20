// SPDX-License-Identifier: MIT
#pragma once
#include "EndpointReference.h"

namespace vad {
// Machine-local provenance in this exact PDO's hardware key. PnP scopes the
// key by root installation and the full StableId instance ID; it survives
// runtime remove/recreate. Never export this record as project settings.
struct PublicationIdentity {
    uint32_t Magic;
    uint32_t Size;
    uint32_t Version;
    uint32_t Intent;
    MOIREN_VAD_ENDPOINT_ID StableId;
    uint32_t BridgePin;
    uint32_t Origin;
    uint32_t ReferenceUnits;
    uint32_t Reserved;
    MOIREN_VAD_ENDPOINT_ID Installation;
    WCHAR Reference[EndpointReferenceUnits];
};
static_assert(sizeof(PublicationIdentity) == 256, "fixed identity schema uses UTF-16");
inline constexpr uint32_t PublicationIdentityMagic = 0x3149504d; // MPI1
inline constexpr uint32_t PublicationIdentityVersion = 1;
inline constexpr uint32_t PublicationIdentityAdopted = 1;
inline constexpr uint32_t PublicationIdentityFresh = 2;

inline void FreshPublicationReference(const MOIREN_VAD_ENDPOINT_ID& installation,
    const MOIREN_VAD_ENDPOINT_ID& stableId, uint32_t intent,
    WCHAR (&reference)[EndpointReferenceUnits])
{
    const WCHAR prefix[] = L"MoirenEndpoint-";
    const WCHAR digits[] = L"0123456789abcdef";
    uint32_t used = 0;
    for (; prefix[used] != L'\0'; ++used) { reference[used] = prefix[used]; }
    const uint64_t words[] = {installation.High, installation.Low, stableId.High, stableId.Low};
    for (uint32_t word = 0; word < 4; ++word) {
        if (word == 2) { reference[used++] = L'-'; }
        for (uint32_t digit = 16; digit != 0; --digit) {
            reference[used++] = digits[(words[word] >> ((digit - 1) * 4)) & 0xf];
        }
    }
    reference[used++] = L'-';
    reference[used++] = intent == MOIREN_VAD_INTENT_MICROPHONE ? L'm' : L'p';
    reference[used] = L'\0';
}

inline bool PublicationReferenceValid(const WCHAR (&reference)[EndpointReferenceUnits],
    ULONG bytes)
{
    if (bytes < 2 * sizeof(WCHAR) || bytes > sizeof(reference) || bytes % sizeof(WCHAR) != 0) {
        return false;
    }
    const auto units = bytes / sizeof(WCHAR);
    if (reference[units - 1] != L'\0') { return false; }
    for (ULONG index = 0; index + 1 < units; ++index) {
        if (reference[index] == L'\0' || reference[index] == L'\\' || reference[index] == L'/') {
            return false;
        }
    }
    return true;
}

inline bool PublicationIdentityValid(const PublicationIdentity& record,
    const MOIREN_VAD_ENDPOINT_ID& stableId, uint32_t intent)
{
    if (record.Magic != PublicationIdentityMagic || record.Size != sizeof(record) ||
        record.Version != PublicationIdentityVersion || record.Intent != intent ||
        record.StableId.High != stableId.High || record.StableId.Low != stableId.Low ||
        record.BridgePin != 1 ||
        (record.Origin != PublicationIdentityAdopted && record.Origin != PublicationIdentityFresh) ||
        record.Reserved != 0 || record.ReferenceUnits > EndpointReferenceUnits ||
        !PublicationReferenceValid(record.Reference, record.ReferenceUnits * sizeof(WCHAR))) {
        return false;
    }
    if ((record.Installation.High == 0 && record.Installation.Low == 0) ||
        (intent != MOIREN_VAD_INTENT_PLAYBACK && intent != MOIREN_VAD_INTENT_MICROPHONE)) {
        return false;
    }
    if (record.Origin == PublicationIdentityFresh) {
        WCHAR expected[EndpointReferenceUnits] = {};
        FreshPublicationReference(record.Installation, stableId, intent, expected);
        for (uint32_t index = 0; index < EndpointReferenceUnits; ++index) {
            if (record.Reference[index] != expected[index]) { return false; }
        }
    }
    // Canonical zero padding makes exact binary readback meaningful and rejects
    // hidden trailing data rather than silently repairing untrusted storage.
    for (uint32_t index = record.ReferenceUnits; index < EndpointReferenceUnits; ++index) {
        if (record.Reference[index] != L'\0') { return false; }
    }
    return true;
}

inline bool SamePublicationReference(const WCHAR (&left)[EndpointReferenceUnits],
    const WCHAR (&right)[EndpointReferenceUnits])
{
    for (uint32_t index = 0; index < EndpointReferenceUnits; ++index) {
        if (left[index] != right[index]) { return false; }
    }
    return true;
}

// v2 remains usable for untouched endpoints, but cannot rotate a reference
// already committed by a negotiated client. This read never adopts or mints.
// The hardware key itself scopes this compatibility lookup to the exact PDO.
inline NTSTATUS ReadCommittedPublicationReference(WDFDEVICE child,
    const MOIREN_VAD_ENDPOINT_ID& stableId, uint32_t intent,
    WCHAR (&reference)[EndpointReferenceUnits], bool& present)
{
    present = false;
    reference[0] = L'\0';
    WDFKEY key = nullptr;
    NTSTATUS status = WdfDeviceOpenRegistryKey(child, PLUGPLAY_REGKEY_DEVICE,
        KEY_QUERY_VALUE | KEY_SET_VALUE, WDF_NO_OBJECT_ATTRIBUTES, &key);
    if (status == STATUS_OBJECT_NAME_NOT_FOUND) { return STATUS_SUCCESS; }
    if (!NT_SUCCESS(status)) { return status; }
    UNICODE_STRING name;
    RtlInitUnicodeString(&name, L"MoirenPublicationIdentityV1");
    PublicationIdentity record = {};
    ULONG bytes = 0, type = 0;
    status = WdfRegistryQueryValue(key, &name, sizeof(record), &record, &bytes, &type);
    if (status == STATUS_OBJECT_NAME_NOT_FOUND) {
        status = STATUS_SUCCESS;
    } else if (NT_SUCCESS(status)) {
        if (bytes != sizeof(record) || type != REG_BINARY ||
            !PublicationIdentityValid(record, stableId, intent)) {
            status = STATUS_DATA_ERROR;
        } else {
            status = ZwFlushKey(WdfRegistryWdmGetHandle(key));
            if (NT_SUCCESS(status)) {
                present = true;
                RtlCopyMemory(reference, record.Reference, sizeof(reference));
            }
        }
    }
    WdfRegistryClose(key);
    return status;
}

// System-worker PASSIVE only. Enumeration owns paged pool; no opaque link is
// parsed, cached or used as identity. Inspect every category so orphan/partial
// history cannot be mistaken for a fresh endpoint. Multiple distinct refs are
// migration debt, not permission to choose by display text or enumeration order.
inline NTSTATUS DiscoverPublicationReference(WDFDEVICE child,
    const GUID* const (&categories)[3], WCHAR (&reference)[EndpointReferenceUnits],
    bool& empty)
{
    empty = false;
    bool found = false;
    ULONG complete = 0;
    for (const GUID* category : categories) {
        WCHAR* links = nullptr;
        NTSTATUS status = IoGetDeviceInterfaces(category, WdfDeviceWdmGetDeviceObject(child),
            DEVICE_INTERFACE_INCLUDE_NONACTIVE, &links);
        if (!NT_SUCCESS(status)) { return status; }
        bool categoryFound = false;
        ULONG inspected = 0;
        if (links == nullptr) { return STATUS_DATA_ERROR; }
        for (const WCHAR* link = links; *link != L'\0';) {
            // Limit property work over potentially extensive retained history.
            if (++inspected > 64) { status = STATUS_INVALID_DEVICE_STATE; break; }
            UNICODE_STRING symbolicLink;
            RtlInitUnicodeString(&symbolicLink, link);
            WCHAR candidate[EndpointReferenceUnits] = {};
            ULONG bytes = 0;
            DEVPROPTYPE type = DEVPROP_TYPE_EMPTY;
            status = IoGetDeviceInterfacePropertyData(&symbolicLink,
                &DEVPKEY_DeviceInterface_ReferenceString, LOCALE_NEUTRAL, 0,
                sizeof(candidate), candidate, &bytes, &type);
            if (!NT_SUCCESS(status)) { break; }
            if (type != DEVPROP_TYPE_STRING || !PublicationReferenceValid(candidate, bytes)) {
                status = STATUS_DATA_ERROR;
                break;
            }
            if (found && !SamePublicationReference(reference, candidate)) {
                status = STATUS_INVALID_DEVICE_STATE;
                break;
            }
            if (category == categories[0]) {
                // Alias lookup guarantees the same PDO and reference across
                // classes; unlike a symbolic-link parse it is a supported join.
                for (ULONG index = 1; index < 3; ++index) {
                    UNICODE_STRING alias = {};
                    status = IoGetDeviceInterfaceAlias(&symbolicLink, categories[index], &alias);
                    if (!NT_SUCCESS(status)) { break; }
                    WCHAR aliasReference[EndpointReferenceUnits] = {};
                    ULONG aliasBytes = 0;
                    DEVPROPTYPE aliasType = DEVPROP_TYPE_EMPTY;
                    status = IoGetDeviceInterfacePropertyData(&alias,
                        &DEVPKEY_DeviceInterface_ReferenceString, LOCALE_NEUTRAL, 0,
                        sizeof(aliasReference), aliasReference, &aliasBytes, &aliasType);
                    RtlFreeUnicodeString(&alias); // Not the enumeration's pool ownership.
                    if (!NT_SUCCESS(status)) { break; }
                    if (aliasType != DEVPROP_TYPE_STRING ||
                        !PublicationReferenceValid(aliasReference, aliasBytes)) {
                        status = STATUS_DATA_ERROR;
                        break;
                    }
                    if (!SamePublicationReference(candidate, aliasReference)) {
                        status = STATUS_INVALID_DEVICE_STATE;
                        break;
                    }
                }
                if (!NT_SUCCESS(status)) { break; }
            }
            RtlCopyMemory(reference, candidate, sizeof(reference));
            found = categoryFound = true;
            link += symbolicLink.Length / sizeof(WCHAR) + 1;
        }
        ExFreePool(links);
        if (!NT_SUCCESS(status)) { return status; }
        if (categoryFound) { ++complete; }
    }
    // Successful empty enumeration is a separate result, never an NTSTATUS
    // sentinel that could collide with a real DDI/property/alias failure.
    empty = complete == 0;
    return complete == 3 || empty ? STATUS_SUCCESS : STATUS_INVALID_DEVICE_STATE;
}

// All handle ownership is stack-local to the system worker. Never retain a
// borrowed root context/handle or close one in elevated final child destruction.
// No Admin/Lifecycle acquisition: PrepareHardware can flush while Admin is held.
// Fresh identity requires the negotiated EstablishNew management operation,
// with a durably read-back root token. Empty history is not authorization.
inline NTSTATUS ResolvePublicationReference(WDFDEVICE child,
    const MOIREN_VAD_ENDPOINT_ID& stableId, uint32_t intent,
    const GUID* const (&categories)[3], uint32_t mode,
    const MOIREN_VAD_ENDPOINT_ID& installation, WCHAR (&reference)[EndpointReferenceUnits])
{
    reference[0] = L'\0';
    if ((mode != MOIREN_VAD_IDENTITY_ESTABLISH_NEW &&
        mode != MOIREN_VAD_IDENTITY_REOPEN_OR_ADOPT) ||
        (installation.High == 0 && installation.Low == 0) ||
        (stableId.High == 0 && stableId.Low == 0) ||
        (intent != MOIREN_VAD_INTENT_PLAYBACK && intent != MOIREN_VAD_INTENT_MICROPHONE)) {
        return STATUS_INVALID_PARAMETER;
    }
    WDFKEY key = nullptr;
    NTSTATUS status = WdfDeviceOpenRegistryKey(child, PLUGPLAY_REGKEY_DEVICE,
        KEY_QUERY_VALUE | KEY_SET_VALUE, WDF_NO_OBJECT_ATTRIBUTES, &key);
    if (!NT_SUCCESS(status)) { return status; }
    UNICODE_STRING valueName;
    RtlInitUnicodeString(&valueName, L"MoirenPublicationIdentityV1");
    PublicationIdentity record = {};
    ULONG bytes = 0, type = 0;
    status = WdfRegistryQueryValue(key, &valueName, sizeof(record), &record, &bytes, &type);
    if (status == STATUS_OBJECT_NAME_NOT_FOUND) {
        record.Magic = PublicationIdentityMagic;
        record.Size = sizeof(record);
        record.Version = PublicationIdentityVersion;
        record.Intent = intent;
        record.StableId = stableId;
        record.Installation = installation;
        record.BridgePin = 1; // Actual external connector; EP\\0 is unrelated.
        record.Origin = PublicationIdentityAdopted;
        bool empty = false;
        status = DiscoverPublicationReference(child, categories, record.Reference, empty);
        if (NT_SUCCESS(status) && !empty && mode == MOIREN_VAD_IDENTITY_ESTABLISH_NEW) {
            status = STATUS_INVALID_DEVICE_STATE; // Never replace an adoptable legacy identity.
        } else if (NT_SUCCESS(status) && empty) {
            status = STATUS_INVALID_DEVICE_STATE;
            if (mode == MOIREN_VAD_IDENTITY_ESTABLISH_NEW) {
                record.Installation = installation;
                record.Origin = PublicationIdentityFresh;
                FreshPublicationReference(installation, stableId, intent, record.Reference);
                status = STATUS_SUCCESS;
            }
        }
        if (NT_SUCCESS(status)) {
            for (uint32_t index = 0; index < EndpointReferenceUnits; ++index) {
                if (record.Reference[index] == L'\0') {
                    record.ReferenceUnits = index + 1;
                    break;
                }
            }
            status = WdfRegistryAssignValue(key, &valueName, REG_BINARY,
                sizeof(record), &record);
        }
        if (NT_SUCCESS(status)) {
            // Visibility is not crash durability. Flush only at this passive
            // publication boundary; it may block on hive I/O, never on RT.
            status = ZwFlushKey(WdfRegistryWdmGetHandle(key));
        }
        if (NT_SUCCESS(status)) {
            PublicationIdentity readback = {};
            status = WdfRegistryQueryValue(key, &valueName, sizeof(readback),
                &readback, &bytes, &type);
            if (NT_SUCCESS(status) && (bytes != sizeof(readback) || type != REG_BINARY ||
                !PublicationIdentityValid(readback, stableId, intent) ||
                RtlCompareMemory(&record, &readback, sizeof(record)) != sizeof(record))) {
                status = STATUS_DATA_ERROR;
            }
        }
    } else if (NT_SUCCESS(status)) {
        if (bytes != sizeof(record) || type != REG_BINARY ||
            !PublicationIdentityValid(record, stableId, intent)) {
            status = STATUS_DATA_ERROR;
        } else if ((mode == MOIREN_VAD_IDENTITY_ESTABLISH_NEW &&
                record.Origin != PublicationIdentityFresh) ||
            record.Installation.High != installation.High ||
            record.Installation.Low != installation.Low) {
            status = STATUS_INVALID_DEVICE_STATE;
        } else {
            // A previous failed flush must not become authority merely because
            // its still-visible record is read on a retry in the same boot.
            status = ZwFlushKey(WdfRegistryWdmGetHandle(key));
        }
    }
    if (NT_SUCCESS(status)) { RtlCopyMemory(reference, record.Reference, sizeof(reference)); }
    WdfRegistryClose(key);
    return status;
}
}
