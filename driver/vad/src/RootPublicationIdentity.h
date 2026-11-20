// SPDX-License-Identifier: MIT
#pragma once

namespace vad {
struct RootPublicationIdentity {
    uint32_t Magic;
    uint32_t Size;
    uint32_t Version;
    uint32_t Reserved;
    MOIREN_VAD_ENDPOINT_ID Token;
};
static_assert(sizeof(RootPublicationIdentity) == 32, "fixed machine-local root schema");

inline bool RootPublicationIdentityValid(const RootPublicationIdentity& record)
{
    return record.Magic == 0x3152504d && record.Size == sizeof(record) &&
        record.Version == 1 && record.Reserved == 0 &&
        (record.Token.High != 0 || record.Token.Low != 0);
}

// Called by the passive system control worker under root Admin. No key or
// borrowed root pointer escapes; children copy only the committed token before
// enumeration. Creating this root scope is NOT evidence that any child is new.
inline NTSTATUS ResolveRootPublicationIdentity(WDFDEVICE root,
    MOIREN_VAD_ENDPOINT_ID& token)
{
    token = {};
    WDFKEY key = nullptr;
    NTSTATUS status = WdfDeviceOpenRegistryKey(root, PLUGPLAY_REGKEY_DEVICE,
        KEY_QUERY_VALUE | KEY_SET_VALUE, WDF_NO_OBJECT_ATTRIBUTES, &key);
    if (!NT_SUCCESS(status)) { return status; }
    UNICODE_STRING name;
    RtlInitUnicodeString(&name, L"MoirenRootPublicationV1");
    RootPublicationIdentity record = {};
    ULONG bytes = 0, type = 0;
    status = WdfRegistryQueryValue(key, &name, sizeof(record), &record, &bytes, &type);
    if (status == STATUS_OBJECT_NAME_NOT_FOUND) {
        GUID uuid = {};
        status = ExUuidCreate(&uuid);
        // This identity is machine-local. UUID_LOCAL_ONLY provides sufficient
        // uniqueness; STATUS_RETRY and every other result propagate unchanged.
        if (status == STATUS_SUCCESS || status == RPC_NT_UUID_LOCAL_ONLY) {
            record.Magic = 0x3152504d; // MPR1
            record.Size = sizeof(record);
            record.Version = 1;
            RtlCopyMemory(&record.Token, &uuid, sizeof(record.Token));
            status = RootPublicationIdentityValid(record) ?
                WdfRegistryAssignValue(key, &name, REG_BINARY, sizeof(record), &record) :
                STATUS_DATA_ERROR;
        }
    } else if (NT_SUCCESS(status) && (bytes != sizeof(record) || type != REG_BINARY ||
        !RootPublicationIdentityValid(record))) {
        status = STATUS_DATA_ERROR;
    }
    if (NT_SUCCESS(status)) { status = ZwFlushKey(WdfRegistryWdmGetHandle(key)); }
    if (NT_SUCCESS(status)) {
        RootPublicationIdentity readback = {};
        status = WdfRegistryQueryValue(key, &name, sizeof(readback), &readback, &bytes, &type);
        if (NT_SUCCESS(status) && (bytes != sizeof(readback) || type != REG_BINARY ||
            !RootPublicationIdentityValid(readback) ||
            RtlCompareMemory(&record, &readback, sizeof(record)) != sizeof(record))) {
            status = STATUS_DATA_ERROR;
        }
    }
    if (NT_SUCCESS(status)) { token = record.Token; }
    WdfRegistryClose(key);
    return status;
}
}
