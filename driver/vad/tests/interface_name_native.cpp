// SPDX-License-Identifier: MIT
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cwchar>
#include <initializer_list>
#include <specstrings.h>
#include <guiddef.h>
#include <initguid.h>
using ULONG = unsigned long;
using CHAR = char;
using PCWSTR = const wchar_t*;
using PVOID = void*;
#include <devpkey.h>
#if !defined(_WIN32)
// The portable runner supplies the SDK-independent property-key fixture.
constexpr DEVPROPKEY DEVPKEY_DeviceInterface_ReferenceString =
    {{0x026e516e, 0xb814, 0x414b, {0x83, 0xcd, 0x85, 0x6d, 0x6f, 0xef, 0x48, 0x22}}, 5};
constexpr DEVPROPKEY DEVPKEY_Device_FriendlyName =
    {{0xa45c254e, 0xdf1c, 0x4efd, {0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50, 0xe0}}, 14};
#endif
#include "../include/moiren_vad.h"
#include "../src/EndpointReference.h"

// Extract real policy staging/rollback. This does not emulate PnP/ACX or prove
// EndpointBuilder consumes this particular interface's friendly name.
using NTSTATUS = std::int32_t;
using WCHAR = wchar_t;
using LCID = ULONG;
using KIRQL = unsigned;
using HANDLE = void*;
using PHANDLE = HANDLE*;
constexpr NTSTATUS STATUS_SUCCESS = 0;
constexpr NTSTATUS STATUS_UNSUCCESSFUL = -1;
constexpr NTSTATUS STATUS_INVALID_PARAMETER = -2;
constexpr NTSTATUS STATUS_INVALID_DEVICE_STATE = -3;
constexpr NTSTATUS STATUS_DATA_ERROR = -4;
constexpr NTSTATUS STATUS_BUFFER_TOO_SMALL = -5;
constexpr NTSTATUS STATUS_DEVICE_NOT_READY = -6;
constexpr NTSTATUS SetFailure = -71;
constexpr NTSTATUS ReadFailure = -72;
constexpr NTSTATUS RegistryFailure = -73;
constexpr NTSTATUS STATUS_OBJECT_NAME_NOT_FOUND = -74;
constexpr NTSTATUS IdentityFailure = -75;
constexpr NTSTATUS STATUS_NOT_FOUND = -76;
constexpr NTSTATUS RPC_NT_UUID_LOCAL_ONLY = 1;
constexpr ULONG LOCALE_NEUTRAL = 0;
constexpr ULONG PLUGPLAY_PROPERTY_PERSISTENT = 1;
constexpr ULONG KEY_CREATE_SUB_KEY = 4;
constexpr ULONG KEY_SET_VALUE = 2;
constexpr ULONG KEY_QUERY_VALUE = 1;
constexpr ULONG REG_BINARY = 3;
constexpr ULONG PLUGPLAY_REGKEY_DEVICE = 1;
constexpr ULONG DEVICE_INTERFACE_INCLUDE_NONACTIVE = 1;
constexpr auto WDF_NO_OBJECT_ATTRIBUTES = nullptr;
constexpr ULONG REG_SZ = 1;
constexpr ULONG REG_DWORD = 4;
constexpr ULONG OBJ_CASE_INSENSITIVE = 1;
constexpr ULONG OBJ_KERNEL_HANDLE = 2;
constexpr ULONG REG_OPTION_NON_VOLATILE = 0;
constexpr KIRQL PASSIVE_LEVEL = 0;
constexpr int TRACE_LEVEL_INFORMATION = 4;
constexpr int TRACE_LEVEL_ERROR = 2;
constexpr int VAD_PDO = 1;
constexpr bool FALSE = false;
// The historical fixed reference remains a valid installed identity to adopt.
constexpr WCHAR VadAudioReference[] = L"MoirenEndpoint";
static_assert(sizeof(WCHAR) == 2, "exercise real Windows UTF-16 byte lengths");
struct UNICODE_STRING {
    unsigned short Length = 0;
    unsigned short MaximumLength = 0;
    WCHAR* Buffer = nullptr;
};
struct OBJECT_ATTRIBUTES { PCWSTR Name = nullptr; HANDLE Parent = nullptr; };
struct RegistryKey { unsigned Interface = 0, Depth = 0; };
struct Slot {
    std::uint32_t Intent = MOIREN_VAD_INTENT_PLAYBACK;
    bool AllowDefault = false;
    WCHAR Name[MOIREN_VAD_MAX_NAME_UNITS] = L"Requested interface";
    MOIREN_VAD_ENDPOINT_ID StableId = {1, 2};
};
// Retained OS devnode storage is independent of any runtime Child/context.
struct IdentityStorage {
    unsigned char RootRecord[64] = {};
    ULONG RootBytes = 0, RootType = REG_BINARY;
    bool RootPresent = false;
    unsigned char Record[512] = {};
    ULONG Bytes = 0, Type = REG_BINARY;
    bool Present = false;
    WCHAR References[3][2][vad::EndpointReferenceUnits] = {};
    unsigned Count[3] = {1, 1, 1};
    IdentityStorage()
    {
        const WCHAR original[MOIREN_VAD_MAX_NAME_UNITS] = L"Requested interface";
        for (auto& category : References) { vad::EndpointReference(original, category[0]); }
    }
};
struct SharedState { int Lock = 0; Slot Slots[1]; };
struct Child;
struct Work { Child* Parent; bool Queued = false; };
struct VAD_CHILD {
    SharedState* Shared;
    ULONG Index = 0;
    Work* PolicyWork;
    NTSTATUS PolicyStatus = STATUS_UNSUCCESSFUL;
    UNICODE_STRING PolicyInterfaces[3] = {};
    ULONG PolicyInterfaceCount = 0;
    WCHAR PolicyReference[vad::EndpointReferenceUnits] = {};
    uint32_t PublicationMode = 2; // Fixture opts into reopen/adopt explicitly.
    MOIREN_VAD_ENDPOINT_ID InstallationId = {0x0123456789abcdefull, 0xfedcba9876543210ull};
};
struct Child {
    SharedState Shared;
    Work Worker{this};
    VAD_CHILD Context{&Shared, 0, &Worker};
    IdentityStorage Devnode;
    IdentityStorage* Store = &Devnode;
    WCHAR PdoName[MOIREN_VAD_MAX_NAME_UNITS] = {};
    ULONG PdoNameBytes = 0;
};
using WDFDEVICE = Child*;
using WDFWORKITEM = Work*;
using WDFKEY = IdentityStorage*;

static void Check(bool condition, const char* message)
{
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        std::exit(1);
    }
}
#define NT_SUCCESS(status) ((status) >= 0)
#define NT_ASSERT(condition) Check((condition), "kernel assertion")
#define PAGED_CODE() Check(KeGetCurrentIrql() == PASSIVE_LEVEL, "pageable policy at passive")
#define RtlCopyMemory std::memcpy
static size_t RtlCompareMemory(const void* left, const void* right, size_t bytes)
{
    const auto a = static_cast<const unsigned char*>(left);
    const auto b = static_cast<const unsigned char*>(right);
    size_t same = 0;
    while (same < bytes && a[same] == b[same]) { ++same; }
    return same;
}
#define UNREFERENCED_PARAMETER(value) (void)(value)
#define RTL_NUMBER_OF(value) (sizeof(value) / sizeof((value)[0]))

struct Fixture {
    unsigned Registered = 0;
    unsigned Freed = 0;
    unsigned Disabled = 0;
    unsigned SetCalls = 0;
    unsigned ReadCalls = 0;
    unsigned PdoSetCalls = 0, PdoReadCalls = 0;
    bool FailPdoSet = false, FailPdoRead = false;
    DEVPROPTYPE PdoReadType = DEVPROP_TYPE_STRING;
    int PdoReadLengthDelta = 0;
    bool PdoDifferentName = false, PdoMissingTerminator = false, PdoOversizeRead = false;
    bool PdoNameValidated = false;
    Child* PolicyChild = nullptr;
    unsigned RegistryCalls = 0;
    unsigned Handles = 0;
    unsigned TraceCalls = 0;
    unsigned NameTraceCalls = 0;
    unsigned FailSetAt = 0;
    unsigned FailReadAt = 0;
    unsigned FailRegistryAt = 0;
    unsigned FailRegisterAt = 0;
    DEVPROPTYPE ReadType = DEVPROP_TYPE_STRING;
    int ReadLengthDelta = 0;
    bool DifferentName = false;
    bool MissingTerminator = false;
    bool OversizeRead = false;
    bool TraceEnabled = true;
    bool ChangeNameAfterSnapshot = false;
    WCHAR SnapshotName[MOIREN_VAD_MAX_NAME_UNITS] = {};
    const WCHAR* References[3] = {};
    bool Live[3] = {};
    WCHAR Links[3][8] = {L"link0", L"link1", L"link2"};
    WCHAR Names[3][MOIREN_VAD_MAX_NAME_UNITS] = {};
    ULONG Bytes[3] = {};
    ULONG PlainNames = 0;
    ULONG EndpointNames = 0;
    WCHAR Associations[3][39] = {};
    ULONG AssociationCount = 0;
    ULONG NeverDefault[3] = {};
    ULONG NeverDefaultCount = 0;
    ULONG EnableByDefault[3] = {};
    ULONG EnableCount = 0;
    RegistryKey Keys[3][3] = {};
    unsigned IdentityOpens = 0, IdentityCloses = 0, IdentityWrites = 0;
    unsigned IdentityReads = 0, IdentityFlushes = 0, Enumerated = 0, EnumerationFreed = 0;
    unsigned FailIdentityAt = 0, IdentityOperations = 0;
    unsigned FailEnumerationAt = 0, FailReferenceAt = 0, ReferenceReads = 0;
    NTSTATUS EnumerationFailure = IdentityFailure;
    unsigned AliasCalls = 0, AliasFreed = 0, FailAliasAt = 0;
    bool CorruptIdentityReadback = false;
    bool CorruptRootReadback = false;
    NTSTATUS UuidStatus = STATUS_SUCCESS;
    unsigned UuidCalls = 0;
    bool WrongReferenceType = false, WrongReferenceBytes = false;
    bool RequireDurableIdentity = true, IdentityDurable = false;
    Child* DiscoveryChild = nullptr;
} F;
static bool SystemThread = false;
static bool SharedLocked = false;
static void PreparePolicy(WDFWORKITEM work);

KIRQL KeGetCurrentIrql() { return PASSIVE_LEVEL; }
VAD_CHILD* VadChild(WDFDEVICE child) { return &child->Context; }
Child* WdfWorkItemGetParentObject(Work* work) { return work->Parent; }
Child* WdfDeviceWdmGetDeviceObject(Child* child) { return child; }
void KeAcquireSpinLock(int*, KIRQL* irql) { SharedLocked = true; *irql = PASSIVE_LEVEL; }
void KeReleaseSpinLock(int* lock, KIRQL)
{
    Check(SharedLocked, "snapshot holds Shared lock");
    auto shared = reinterpret_cast<SharedState*>(lock);
    std::memcpy(F.SnapshotName, shared->Slots[0].Name, sizeof(F.SnapshotName));
    SharedLocked = false;
    if (F.ChangeNameAfterSnapshot) {
        const WCHAR changed[MOIREN_VAD_MAX_NAME_UNITS] = L"Changed after snapshot";
        std::memcpy(shared->Slots[0].Name, changed, sizeof(changed));
    }
}
void WdfWorkItemEnqueue(Work* work) { work->Queued = true; }
void WdfWorkItemFlush(Work* work)
{
    Check(work->Queued, "flush must execute queued policy");
    F.PolicyChild = work->Parent;
    F.PdoNameValidated = false;
    SystemThread = true;
    PreparePolicy(work);
    SystemThread = false;
    work->Queued = false;
}
template<typename... Args>
void VadTrace(int, int, const char* format, Args...)
{
    if (!F.TraceEnabled) { return; }
    ++F.TraceCalls;
    if (std::strstr(format, "Policy typed interface name") != nullptr) {
        ++F.NameTraceCalls;
        Check(sizeof...(Args) == 6 && std::strstr(format, "%ws") == nullptr &&
            std::strstr(format, "%ls") == nullptr && std::strstr(format, "%s") == nullptr,
            "six bounded fields only, never include user name text");
    }
}
void RtlInitUnicodeString(UNICODE_STRING* value, PCWSTR text)
{
    value->Buffer = const_cast<WCHAR*>(text);
    value->Length = static_cast<unsigned short>(std::wcslen(text) * sizeof(WCHAR));
    value->MaximumLength = value->Length + static_cast<unsigned short>(sizeof(WCHAR));
}
void InitializeObjectAttributes(OBJECT_ATTRIBUTES* attributes, UNICODE_STRING* name,
    ULONG, HANDLE parent, void*)
{
    attributes->Name = name->Buffer;
    attributes->Parent = parent;
}
static void CheckDdi()
{
    Check(SystemThread && !SharedLocked, "DDI in system worker outside Shared lock");
}

static void CheckPdoNameProperty(Child* child, const DEVPROPKEY* key, LCID locale)
{
    CheckDdi();
    Check(child != nullptr && child == F.PolicyChild,
        "PDO property uses the exact child retained by the flushed policy worker");
    Check(IsEqualGUID(key->fmtid, DEVPKEY_Device_FriendlyName.fmtid) &&
        key->pid == DEVPKEY_Device_FriendlyName.pid && locale == LOCALE_NEUTRAL,
        "only language-neutral child devnode FriendlyName, not interface/endpoint properties");
}

NTSTATUS IoSetDevicePropertyData(Child* child, const DEVPROPKEY* key,
    LCID locale, ULONG flags, DEVPROPTYPE type, ULONG bytes, void* data)
{
    CheckPdoNameProperty(child, key, locale);
    Check(flags == PLUGPLAY_PROPERTY_PERSISTENT && type == DEVPROP_TYPE_STRING,
        "PDO name is a persistent plain typed string");
    Check(bytes == (std::wcslen(F.SnapshotName) + 1) * sizeof(WCHAR) &&
        bytes <= sizeof(child->PdoName) && std::memcmp(data, F.SnapshotName, bytes) == 0,
        "PDO name uses the bounded locked snapshot including NUL, not the current slot");
    ++F.PdoSetCalls;
    if (F.FailPdoSet) { return SetFailure; }
    std::memcpy(child->PdoName, data, bytes);
    child->PdoNameBytes = bytes;
    return STATUS_SUCCESS;
}

NTSTATUS IoGetDevicePropertyData(Child* child, const DEVPROPKEY* key,
    LCID locale, ULONG flags, ULONG capacity, void* data, ULONG* required, DEVPROPTYPE* type)
{
    CheckPdoNameProperty(child, key, locale);
    Check(flags == 0 && capacity == sizeof(child->PdoName),
        "PDO getter uses reserved zero flags and one fixed-capacity buffer");
    ++F.PdoReadCalls;
    if (F.FailPdoRead) { return ReadFailure; }
    if (F.PdoOversizeRead) { *required = capacity + 2; return STATUS_BUFFER_TOO_SMALL; }
    std::memcpy(data, child->PdoName, child->PdoNameBytes);
    *required = static_cast<ULONG>(static_cast<int>(child->PdoNameBytes) + F.PdoReadLengthDelta);
    *type = F.PdoReadType;
    if (F.PdoDifferentName) { static_cast<WCHAR*>(data)[0] ^= 1; }
    if (F.PdoMissingTerminator) {
        static_cast<WCHAR*>(data)[child->PdoNameBytes / sizeof(WCHAR) - 1] = L'X';
    }
    F.PdoNameValidated = *type == DEVPROP_TYPE_STRING && *required == child->PdoNameBytes &&
        std::memcmp(data, F.SnapshotName, child->PdoNameBytes) == 0;
    return STATUS_SUCCESS;
}

static NTSTATUS IdentityOperation()
{
    CheckDdi();
    return ++F.IdentityOperations == F.FailIdentityAt ? IdentityFailure : STATUS_SUCCESS;
}
NTSTATUS WdfDeviceOpenRegistryKey(Child* child, ULONG keyType, ULONG access,
    decltype(WDF_NO_OBJECT_ATTRIBUTES), WDFKEY* key)
{
    Check(keyType == PLUGPLAY_REGKEY_DEVICE && access == (KEY_QUERY_VALUE | KEY_SET_VALUE),
        "identity uses this exact child hardware key with minimal value rights");
    const auto status = IdentityOperation();
    if (!NT_SUCCESS(status)) { return status; }
    *key = child->Store;
    ++F.IdentityOpens;
    return STATUS_SUCCESS;
}
void WdfRegistryClose(WDFKEY)
{ CheckDdi(); ++F.IdentityCloses; }
NTSTATUS WdfRegistryQueryValue(WDFKEY key, const UNICODE_STRING* name, ULONG capacity,
    void* data, ULONG* bytes, ULONG* type)
{
    const auto status = IdentityOperation();
    if (!NT_SUCCESS(status)) { return status; }
    ++F.IdentityReads;
    if (std::wcscmp(name->Buffer, L"MoirenRootPublicationV1") == 0) {
        if (!key->RootPresent) { return STATUS_OBJECT_NAME_NOT_FOUND; }
        *bytes = key->RootBytes;
        *type = key->RootType;
        if (capacity < key->RootBytes) { return STATUS_BUFFER_TOO_SMALL; }
        std::memcpy(data, key->RootRecord, key->RootBytes);
        if (F.CorruptRootReadback) { static_cast<unsigned char*>(data)[0] ^= 1; }
        return STATUS_SUCCESS;
    }
    if (!key->Present) { return STATUS_OBJECT_NAME_NOT_FOUND; }
    *bytes = key->Bytes;
    *type = key->Type;
    if (capacity < key->Bytes) { return STATUS_BUFFER_TOO_SMALL; }
    std::memcpy(data, key->Record, key->Bytes);
    if (F.CorruptIdentityReadback && F.IdentityReads > 1) {
        static_cast<unsigned char*>(data)[0] ^= 1;
    }
    return STATUS_SUCCESS;
}
NTSTATUS WdfRegistryAssignValue(WDFKEY key, const UNICODE_STRING* name, ULONG type,
    ULONG bytes, void* data)
{
    const auto status = IdentityOperation();
    if (!NT_SUCCESS(status)) { return status; }
    if (std::wcscmp(name->Buffer, L"MoirenRootPublicationV1") == 0) {
        Check(bytes <= sizeof(key->RootRecord), "bounded root token record");
        std::memcpy(key->RootRecord, data, bytes);
        key->RootBytes = bytes;
        key->RootType = type;
        key->RootPresent = true;
        ++F.IdentityWrites;
        return STATUS_SUCCESS;
    }
    Check(type == REG_BINARY && bytes <= sizeof(key->Record), "bounded binary identity record");
    std::memcpy(key->Record, data, bytes);
    key->Bytes = bytes;
    key->Type = type;
    key->Present = true;
    F.IdentityDurable = false;
    ++F.IdentityWrites;
    return STATUS_SUCCESS;
}
HANDLE WdfRegistryWdmGetHandle(WDFKEY key) { return key; }
NTSTATUS ZwFlushKey(HANDLE)
{
    const auto status = IdentityOperation();
    if (NT_SUCCESS(status)) { ++F.IdentityFlushes; F.IdentityDurable = true; }
    return status;
}
NTSTATUS ExUuidCreate(GUID* uuid)
{
    CheckDdi();
    ++F.UuidCalls;
    std::memset(uuid, 0x42, sizeof(*uuid));
    return F.UuidStatus;
}
NTSTATUS IoGetDeviceInterfaces(const GUID* category, Child* child, ULONG flags, WCHAR** result)
{
    CheckDdi();
    Check(child != nullptr && flags == DEVICE_INTERFACE_INCLUDE_NONACTIVE,
        "enumerate exact owned PDO including inactive registrations");
    const unsigned index = category->Data1 == 0x6994ad04 ? 0u :
        (category->Data1 == 0xeb115ffc ? 2u : 1u);
    F.DiscoveryChild = child;
    if (++F.Enumerated == F.FailEnumerationAt) { return F.EnumerationFailure; }
    auto text = static_cast<WCHAR*>(std::calloc(8, sizeof(WCHAR)));
    Check(text != nullptr && child->Store->Count[index] <= 2, "bounded discovery fixture");
    for (unsigned entry = 0; entry < child->Store->Count[index]; ++entry) {
        text[entry * 3] = static_cast<WCHAR>(L'a' + index);
        text[entry * 3 + 1] = static_cast<WCHAR>(L'0' + entry);
    }
    *result = text;
    return STATUS_SUCCESS;
}
void ExFreePool(void* value) { CheckDdi(); ++F.EnumerationFreed; std::free(value); }
NTSTATUS IoGetDeviceInterfaceAlias(UNICODE_STRING* link, const GUID* category,
    UNICODE_STRING* alias)
{
    CheckDdi();
    if (++F.AliasCalls == F.FailAliasAt) { return IdentityFailure; }
    const unsigned index = category->Data1 == 0xeb115ffc ? 2u : 1u;
    auto buffer = static_cast<WCHAR*>(std::calloc(3, sizeof(WCHAR)));
    Check(buffer != nullptr, "owned alias allocation");
    buffer[0] = static_cast<WCHAR>(L'a' + index);
    buffer[1] = link->Buffer[1];
    RtlInitUnicodeString(alias, buffer);
    return STATUS_SUCCESS;
}
static unsigned LinkIndex(const UNICODE_STRING* link)
{
    for (unsigned i = 0; i < 3; ++i) {
        if (link->Buffer == F.Links[i]) {
            Check(F.Live[i], "interface allocation still owned");
            return i;
        }
    }
    Check(false, "DDI uses exact owned symbolic link, not a reconstructed path");
    return 0;
}
NTSTATUS IoRegisterDeviceInterface(Child* child, const GUID*, UNICODE_STRING* reference,
    UNICODE_STRING* link)
{
    CheckDdi();
    Check(F.PdoNameValidated, "child PDO FriendlyName is verified before any interface registration");
    if (F.RequireDurableIdentity) {
        Check(F.IdentityDurable, "durable identity commit precedes every interface registration");
    }
    Check(reference->Buffer[0] != L'\0', "staged reference is nonempty");
    Check(reference->Buffer == child->Context.PolicyReference,
        "each category borrows the single child-owned Policy reference");
    if (F.Registered + 1 == F.FailRegisterAt) { return ReadFailure; }
    Check(F.Registered < 3, "only three owned categories");
    const auto index = F.Registered++;
    F.References[index] = reference->Buffer;
    F.Live[index] = true;
    RtlInitUnicodeString(link, F.Links[index]);
    return STATUS_SUCCESS;
}
NTSTATUS IoSetDeviceInterfaceState(UNICODE_STRING* link, bool enable)
{
    CheckDdi();
    (void)LinkIndex(link);
    Check(!enable, "policy never publicly enables an interface");
    ++F.Disabled;
    return STATUS_SUCCESS;
}
void RtlFreeUnicodeString(UNICODE_STRING* link)
{
    if (link->Buffer[0] >= L'a' && link->Buffer[0] <= L'c') {
        CheckDdi();
        ++F.AliasFreed;
        std::free(link->Buffer);
        *link = {};
        return;
    }
    const auto index = LinkIndex(link);
    F.Live[index] = false;
    ++F.Freed;
}
static void CheckProperty(const DEVPROPKEY* key, LCID locale)
{
    CheckDdi();
    Check(IsEqualGUID(key->fmtid, DEVPKEY_DeviceInterface_FriendlyName.fmtid) &&
        key->pid == DEVPKEY_DeviceInterface_FriendlyName.pid, "only interface FriendlyName key");
    Check(locale == LOCALE_NEUTRAL, "language-neutral property");
}
NTSTATUS IoSetDeviceInterfacePropertyData(UNICODE_STRING* link, const DEVPROPKEY* key,
    LCID locale, ULONG flags, DEVPROPTYPE type, ULONG bytes, void* data)
{
    CheckProperty(key, locale);
    const auto index = LinkIndex(link);
    Check(flags == PLUGPLAY_PROPERTY_PERSISTENT && type == DEVPROP_TYPE_STRING,
        "persistent plain typed string, not indirect name or endpoint key");
    Check(bytes > sizeof(WCHAR) && bytes <= sizeof(F.Names[index]) &&
        bytes % sizeof(WCHAR) == 0, "bounded exact UTF-16 size including terminator");
    Check(static_cast<WCHAR*>(data)[bytes / sizeof(WCHAR) - 1] == L'\0', "NUL included");
    ++F.SetCalls;
    if (F.SetCalls == F.FailSetAt) { return SetFailure; }
    std::memcpy(F.Names[index], data, bytes);
    F.Bytes[index] = bytes;
    return STATUS_SUCCESS;
}
NTSTATUS IoGetDeviceInterfacePropertyData(UNICODE_STRING* link, const DEVPROPKEY* key,
    LCID locale, ULONG flags, ULONG capacity, void* data, ULONG* required, DEVPROPTYPE* type)
{
    if (key->pid == 5) {
        CheckDdi();
        Check(flags == 0 && locale == LOCALE_NEUTRAL && F.DiscoveryChild != nullptr,
            "reference property getter uses reserved zero flags in the system worker");
        if (++F.ReferenceReads == F.FailReferenceAt) { return IdentityFailure; }
        // Only the DDI fixture decodes its synthetic opaque link identifiers.
        const auto category = static_cast<unsigned>(link->Buffer[0] - L'a');
        const auto entry = static_cast<unsigned>(link->Buffer[1] - L'0');
        Check(category < 3 && entry < 2, "owned discovery link");
        const auto reference = F.DiscoveryChild->Store->References[category][entry];
        *required = static_cast<ULONG>((std::wcslen(reference) + 1) * sizeof(WCHAR));
        *type = F.WrongReferenceType ? DEVPROP_TYPE_STRING_INDIRECT : DEVPROP_TYPE_STRING;
        if (*required > capacity) { return STATUS_BUFFER_TOO_SMALL; }
        std::memcpy(data, reference, *required);
        if (F.WrongReferenceBytes) { --*required; }
        return STATUS_SUCCESS;
    }
    CheckProperty(key, locale);
    const auto index = LinkIndex(link);
    Check(flags == 0 && capacity == sizeof(F.Names[index]), "one fixed-capacity read, no retry");
    ++F.ReadCalls;
    if (F.ReadCalls == F.FailReadAt) { return ReadFailure; }
    if (F.OversizeRead) {
        *required = capacity + 2;
        return STATUS_BUFFER_TOO_SMALL;
    }
    std::memcpy(data, F.Names[index], F.Bytes[index]);
    *required = static_cast<ULONG>(static_cast<int>(F.Bytes[index]) + F.ReadLengthDelta);
    *type = F.ReadType;
    if (F.DifferentName) { static_cast<WCHAR*>(data)[0] ^= 1; }
    if (F.MissingTerminator) { static_cast<WCHAR*>(data)[F.Bytes[index] / 2 - 1] = L'X'; }
    return STATUS_SUCCESS;
}
static NTSTATUS OpenKey(PHANDLE result, RegistryKey* key)
{
    CheckDdi();
    ++F.RegistryCalls;
    if (F.RegistryCalls == F.FailRegistryAt) { return RegistryFailure; }
    ++F.Handles;
    *result = key;
    return STATUS_SUCCESS;
}
NTSTATUS IoOpenDeviceInterfaceRegistryKey(UNICODE_STRING* link, ULONG, PHANDLE result)
{
    const auto index = LinkIndex(link);
    F.Keys[index][0] = {index, 0};
    return OpenKey(result, &F.Keys[index][0]);
}
NTSTATUS ZwCreateKey(PHANDLE result, ULONG, OBJECT_ATTRIBUTES* attributes,
    ULONG, void*, ULONG, void*)
{
    const auto parent = static_cast<const RegistryKey*>(attributes->Parent);
    Check(parent != nullptr && parent->Depth < 2 &&
        std::wcscmp(attributes->Name, parent->Depth == 0 ? L"EP" : L"0") == 0,
        "policy keys descend from the owned interface into the same EP\\0");
    auto& key = F.Keys[parent->Interface][parent->Depth + 1];
    key = {parent->Interface, parent->Depth + 1};
    return OpenKey(result, &key);
}
NTSTATUS ZwSetValueKey(HANDLE handle, UNICODE_STRING* name, ULONG, ULONG type,
    void* data, ULONG bytes)
{
    CheckDdi();
    ++F.RegistryCalls;
    if (F.RegistryCalls == F.FailRegistryAt) { return RegistryFailure; }
    if (name->Buffer[0] == L'{') {
        Check(static_cast<const RegistryKey*>(handle)->Depth == 2,
            "Association, enablement and exclusion stay together in the owned EP\\0");
    }
    if (std::wcscmp(name->Buffer, L"FriendlyName") == 0) { ++F.PlainNames; }
    if (std::wcscmp(name->Buffer, L"{A45C254E-DF1C-4EFD-8020-67D146A850E0},14") == 0) {
        ++F.EndpointNames;
    }
    if (std::wcscmp(name->Buffer, L"{1DA5D803-D492-4EDD-8C23-E0C0FFEE7F0E},2") == 0) {
        Check(type == REG_SZ && bytes == sizeof(F.Associations[0]) && F.AssociationCount < 3,
            "Association is a bounded GUID string including NUL");
        std::memcpy(F.Associations[F.AssociationCount++], data, bytes);
    }
    if (std::wcscmp(name->Buffer, L"{F3E80BEF-1723-4FF2-BCC4-7F83DC5E46D4},3") == 0) {
        Check(type == REG_DWORD && bytes == sizeof(ULONG) && F.NeverDefaultCount < 3,
            "default exclusion is a DWORD on each interface");
        F.NeverDefault[F.NeverDefaultCount++] = *static_cast<const ULONG*>(data);
    }
    if (std::wcscmp(name->Buffer, L"{F3E80BEF-1723-4FF2-BCC4-7F83DC5E46D4},4") == 0) {
        Check(type == REG_DWORD && bytes == sizeof(ULONG) && F.EnableCount < 3,
            "endpoint enablement is a DWORD on each interface");
        F.EnableByDefault[F.EnableCount++] = *static_cast<const ULONG*>(data);
    }
    return STATUS_SUCCESS;
}
void ZwClose(HANDLE)
{
    Check(F.Handles != 0, "close only owned handle");
    --F.Handles;
}

#include "../src/PublicationIdentity.h"
#include "../src/RootPublicationIdentity.h"
#include "interface-name-native-production.inc"

static void Release(Child& child)
{
    SystemThread = true;
    VadReleasePolicy(&child);
    VadReleasePolicy(&child);
    SystemThread = false;
    Check(F.Freed == F.Registered && F.Handles == 0, "no leaked or double-freed policy resources");
    Check(child.Context.PolicyInterfaceCount == 0, "release clears ownership count");
    Check(child.Context.PolicyReference[0] == L'\0', "release invalidates the runtime reference");
    for (const auto& link : child.Context.PolicyInterfaces) {
        Check(link.Buffer == nullptr, "released or failed context link cleared");
    }
}

#if defined(VAD_PUBLICATION_REFERENCE_TEST)
// Defined by the existing composed fixture/extract after its ACX types exist.
static void Reset(Child& child);
static NTSTATUS Prepare(WDFDEVICE child, void* raw, void* translated);
static void CheckPdoNameCircuitUnpublished();
#endif

static void PdoNameFailureRefusesPreparation()
{
    for (bool cached : {false, true}) {
        for (unsigned mode = 0; mode < 12; ++mode) {
            Child child;
#if defined(VAD_PUBLICATION_REFERENCE_TEST)
            Reset(child);
#else
            F = {};
#endif
            if (cached) {
                Check(VadPreparePolicy(&child, 0) == STATUS_SUCCESS, "seed three-interface policy");
            }
            WCHAR retained[vad::EndpointReferenceUnits] = {};
            std::memcpy(retained, child.Context.PolicyReference, sizeof(retained));
            const auto before = child.Devnode;
            const auto setBefore = F.PdoSetCalls, readBefore = F.PdoReadCalls;
            const auto identityWrites = F.IdentityWrites;
            const WCHAR changed[MOIREN_VAD_MAX_NAME_UNITS] = L"Renamed PDO \x540d\x79f0";
            std::memcpy(child.Shared.Slots[0].Name, changed, sizeof(changed));
            switch (mode) {
            case 0: F.FailPdoSet = true; break;
            case 1: F.FailPdoRead = true; break;
            case 2: F.PdoReadType = DEVPROP_TYPE_STRING_INDIRECT; break;
            case 3: F.PdoReadLengthDelta = -2; break;
            case 4: F.PdoReadLengthDelta = 2; break;
            case 5: F.PdoReadLengthDelta = -1; break;
            case 6: F.PdoDifferentName = true; break;
            case 7: F.PdoMissingTerminator = true; break;
            case 8: F.PdoOversizeRead = true; break;
            case 9: F.PdoReadLengthDelta = 200; break;
            case 10: child.Shared.Slots[0].Name[0] = L'\0'; break;
            case 11: for (auto& unit : child.Shared.Slots[0].Name) { unit = L'X'; } break;
            }
            const auto expected = mode == 0 ? SetFailure : mode == 1 ? ReadFailure :
                mode == 8 ? STATUS_BUFFER_TOO_SMALL : mode >= 10 ? STATUS_INVALID_PARAMETER :
                STATUS_DATA_ERROR;
#if defined(VAD_PUBLICATION_REFERENCE_TEST)
            const auto status = Prepare(&child, nullptr, nullptr);
            CheckPdoNameCircuitUnpublished();
#else
            const auto status = VadPreparePolicy(&child, 0);
#endif
            Check(status == expected && child.Context.PolicyStatus == expected,
                "initial and cached PDO-name failures refuse preparation with the original status");
            Check(F.PdoSetCalls == setBefore + (mode >= 10 ? 0u : 1u) &&
                F.PdoReadCalls == readBefore + (mode == 0 || mode >= 10 ? 0u : 1u),
                "invalid names never reach DDIs; failures do not retry or read after a failed set");
            Check(F.Registered == (cached ? 3u : 0u) && F.IdentityWrites == identityWrites &&
                std::wcscmp(child.Context.PolicyReference, retained) == 0 &&
                std::memcmp(&before, &child.Devnode, sizeof(before)) == 0,
                "PDO-name failure cannot change retained publication identity or register categories");
            Release(child);
        }
    }
}

static void SuccessfulTypedPublication()
{
    PdoNameFailureRefusesPreparation();
    for (auto intent : {MOIREN_VAD_INTENT_PLAYBACK, MOIREN_VAD_INTENT_MICROPHONE}) {
        F = {};
        Child child;
        child.Shared.Slots[0].Intent = intent;
        Check(VadPreparePolicy(&child, intent) == STATUS_SUCCESS, "typed policy succeeds");
        WCHAR expected[vad::EndpointReferenceUnits] = {};
        vad::EndpointReference(child.Shared.Slots[0].Name, expected);
        Check(std::wcscmp(child.Context.PolicyReference, expected) == 0,
            "Policy retains its exact historical publication reference for ACX");
        Check(F.SetCalls == 3 && F.ReadCalls == 3,
            "every owned category must set and strictly read back typed FriendlyName");
        Check(F.PdoSetCalls == 1 && F.PdoReadCalls == 1 && F.PdoNameValidated &&
            std::wcscmp(child.PdoName, F.SnapshotName) == 0,
            "first preparation sets and verifies this child PDO's exact requested name");
        Check(F.NameTraceCalls == 3, "one typed diagnostic per category, not per read attempt");
        Check(F.PlainNames == 3 && F.EndpointNames == 3, "ordinary and EP0 names retained");
        Check(F.Disabled == 0 && F.Freed == 0 && child.Context.PolicyInterfaceCount == 3,
            "success transfers all three disabled links to ReleasePolicy");
        const auto before = child.Devnode;
        const WCHAR changed[MOIREN_VAD_MAX_NAME_UNITS] = L"Changed cached PDO name";
        std::memcpy(child.Shared.Slots[0].Name, changed, sizeof(changed));
        Check(VadPreparePolicy(&child, intent) == STATUS_SUCCESS && F.SetCalls == 3 &&
            F.NameTraceCalls == 3 && F.PdoSetCalls == 2 && F.PdoReadCalls == 2 &&
            std::wcscmp(child.PdoName, changed) == 0 &&
            std::wcscmp(child.Context.PolicyReference, expected) == 0 &&
            std::memcmp(&before, &child.Devnode, sizeof(before)) == 0,
            "three-interface fast path restages PDO name without changing durable reference or categories");
        Release(child);
    }
}

static void AdoptedReferencePersistsAcrossRuntimeRename()
{
    IdentityStorage devnode;
    for (auto& category : devnode.References) {
        const WCHAR legacy[vad::EndpointReferenceUnits] = L"MoirenEndpoint";
        std::memcpy(category[0], legacy, sizeof(legacy));
    }
    F = {};
    Child original;
    original.Store = &devnode;
    Check(VadPreparePolicy(&original, 0) == STATUS_SUCCESS, "adopt unique installed reference");
    Check(F.IdentityWrites == 1 && F.IdentityFlushes == 1 && F.IdentityReads == 2 && devnode.Present,
        "adoption must persist, flush and read back before publication");
    Check(F.AliasCalls == 2 && F.AliasFreed == 2,
        "adoption verifies and frees flow/Realtime aliases of the owned Audio reference");
    Check(std::wcscmp(original.Context.PolicyReference, L"MoirenEndpoint") == 0,
        "preserve the short legacy reference byte-for-byte, not the requested-name hash");
    Release(original);
    F = {};
    Child renamed;
    renamed.Store = &devnode;
    const WCHAR changed[MOIREN_VAD_MAX_NAME_UNITS] = L"New requested name after recreation";
    std::memcpy(renamed.Shared.Slots[0].Name, changed, sizeof(changed));
    Check(VadPreparePolicy(&renamed, 0) == STATUS_SUCCESS, "recreated PDO reads its durable identity");
    Check(F.IdentityWrites == 0 && F.Enumerated == 0 &&
        std::wcscmp(renamed.Context.PolicyReference, L"MoirenEndpoint") == 0,
        "rename must reuse persisted authority without regenerating from display text");
    Release(renamed);
}

static void AdoptionFailuresNeverPublishOrLeak()
{
    for (unsigned alias = 1; alias <= 2; ++alias) {
        F = {};
        Child child;
        F.FailAliasAt = alias;
        Check(VadPreparePolicy(&child, 0) == IdentityFailure && F.Registered == 0 &&
            F.IdentityWrites == 0 && F.AliasFreed == alias - 1 &&
            F.EnumerationFreed == F.Enumerated && F.IdentityOpens == F.IdentityCloses,
            "missing/failed category alias cannot authorize adoption; ownership is released");
        Release(child);
    }
    for (unsigned boundary = 1; boundary <= 5; ++boundary) {
        F = {};
        Child child;
        F.FailIdentityAt = boundary; // open/query/write/flush/readback
        Check(VadPreparePolicy(&child, 0) == IdentityFailure,
            "preserve exact identity I/O failure, never treat it as absence");
        Check(child.Context.PolicyReference[0] == L'\0' && F.Registered == 0 &&
            F.IdentityOpens == F.IdentityCloses && F.Enumerated == F.EnumerationFreed,
            "every identity failure closes owned storage and precedes all publication");
        Release(child);
    }
    for (unsigned category = 0; category < 3; ++category) {
        for (bool referenceRead : {false, true}) {
            F = {};
            Child child;
            F.FailEnumerationAt = referenceRead ? 0 : category + 1;
            F.FailReferenceAt = referenceRead ? category + 1 : 0;
            Check(VadPreparePolicy(&child, 0) == IdentityFailure && F.Registered == 0,
                "discovery/property errors propagate before any registry write/publication");
            Check(F.IdentityWrites == 0 && F.IdentityOpens == F.IdentityCloses &&
                F.EnumerationFreed == F.Enumerated - (referenceRead ? 0u : 1u),
                "only successful OS enumeration allocations are released");
            Release(child);
        }
    }
    for (unsigned mode = 0; mode < 3; ++mode) {
        F = {};
        Child child;
        F.CorruptIdentityReadback = mode == 0;
        F.WrongReferenceType = mode == 1;
        F.WrongReferenceBytes = mode == 2;
        Check(VadPreparePolicy(&child, 0) == STATUS_DATA_ERROR && F.Registered == 0,
            "typed/malformed discovery and mismatched durable readback fail closed");
        Check(F.IdentityOpens == F.IdentityCloses && F.Enumerated == F.EnumerationFreed,
            "mismatch paths release all passive ownership");
        Release(child);
    }
}

static void MissingAndAmbiguousHistoryCannotMintIdentity()
{
    for (unsigned mode = 0; mode < 6; ++mode) {
        F = {};
        Child child;
        if (mode < 3) { child.Store->Count[mode] = 0; }
        if (mode == 3) {
            for (auto& count : child.Store->Count) { count = 0; }
        }
        if (mode == 4) {
            const WCHAR other[vad::EndpointReferenceUnits] = L"MoirenEndpoint-aaaaaaaaaaaaaaaa";
            std::memcpy(child.Store->References[1][0], other, sizeof(other));
        }
        if (mode == 5) {
            const WCHAR other[vad::EndpointReferenceUnits] = L"MoirenEndpoint-bbbbbbbbbbbbbbbb";
            for (unsigned category = 0; category < 3; ++category) {
                child.Store->Count[category] = 2;
                std::memcpy(child.Store->References[category][1], other, sizeof(other));
            }
        }
        Check(VadPreparePolicy(&child, 0) == STATUS_INVALID_DEVICE_STATE && F.Registered == 0 &&
            F.IdentityWrites == 0 && !child.Store->Present,
            "missing/partial/conflicting/multiple triples cannot mint a fresh identity");
        Check(F.IdentityOpens == F.IdentityCloses && F.Enumerated == F.EnumerationFreed,
            "ambiguous history retains no transient ownership");
        Release(child);
    }
}

static void PersistedRecordValidatesOwnershipAndSchema()
{
    F = {};
    Child seed;
    Check(VadPreparePolicy(&seed, 0) == STATUS_SUCCESS, "seed valid adoption record");
    const auto stored = seed.Devnode;
    Release(seed);
    for (unsigned mode = 0; mode < 19; ++mode) {
        F = {};
        Child child;
        child.Devnode = stored;
        vad::PublicationIdentity record = {};
        std::memcpy(&record, child.Store->Record, sizeof(record));
        switch (mode) {
        case 0: record.Magic ^= 1; break;
        case 1: --record.Size; break;
        case 2: ++record.Version; break;
        case 3: ++record.StableId.Low; break;
        case 4: ++record.StableId.High; break;
        case 5: record.Intent = MOIREN_VAD_INTENT_MICROPHONE; break;
        case 6: record.BridgePin = 0; break;
        case 7: record.Origin = 0; break;
        case 8: record.Reserved = 1; break;
        case 9: record.ReferenceUnits = 0; break;
        case 10: record.ReferenceUnits = vad::EndpointReferenceUnits + 1; break;
        case 11: record.Reference[0] = L'\0'; break;
        case 12: record.Reference[1] = L'\\'; break;
        case 13: record.Reference[record.ReferenceUnits - 1] = L'X'; break;
        case 14: child.Store->Type = REG_SZ; break;
        case 15: --child.Store->Bytes; break;
        case 16: ++child.Store->Bytes; break;
        case 17: ++record.Installation.Low; break;
        case 18: ++record.Installation.High; break;
        }
        std::memcpy(child.Store->Record, &record, sizeof(record));
        const auto before = child.Devnode;
        const auto status = VadPreparePolicy(&child, 0);
        const auto expectedStatus = mode == 16 ? STATUS_BUFFER_TOO_SMALL :
            (mode >= 17 ? STATUS_INVALID_DEVICE_STATE : STATUS_DATA_ERROR);
        Check(status == expectedStatus &&
            F.Registered == 0 && F.Enumerated == 0 && F.IdentityWrites == 0,
            "corruption/ownership mismatch never falls back to display-name hash or history");
        Check(std::memcmp(before.Record, child.Store->Record, sizeof(before.Record)) == 0 &&
            F.IdentityOpens == F.IdentityCloses,
            "corrupt identity is preserved for explicit repair, not rewritten");
        Release(child);
    }
    F = {};
    Child authoritative;
    authoritative.Devnode = stored;
    // Stale history must not override an already proven exact devnode record.
    for (auto& count : authoritative.Store->Count) { count = 2; }
    Check(VadPreparePolicy(&authoritative, 0) == STATUS_SUCCESS && F.Enumerated == 0 &&
        F.IdentityWrites == 0 && F.IdentityFlushes == 1,
        "valid persisted authority wins over unrelated stale historical registrations");
    Release(authoritative);
}

static void FlushFailureRetryAndStageFailureRetainIdentity()
{
    for (bool failFlush : {false, true}) {
        F = {};
        Child child;
        F.FailIdentityAt = failFlush ? 4 : 0;
        F.FailSetAt = failFlush ? 0 : 2;
        Check(VadPreparePolicy(&child, 0) == (failFlush ? IdentityFailure : SetFailure),
            "first failure remains explicit");
        Check(child.Store->Present && child.Context.PolicyReference[0] == L'\0',
            "durable reservation survives failed staging/flush while runtime reference is invalid");
        const auto stored = child.Devnode;
        Release(child);
        F = {};
        Check(VadPreparePolicy(&child, 0) == STATUS_SUCCESS && F.IdentityWrites == 0 &&
            F.IdentityFlushes == 1 && F.Enumerated == 0,
            "retry validates and durably flushes existing reservation before publishing");
        Check(std::memcmp(stored.Record, child.Store->Record, sizeof(stored.Record)) == 0,
            "retry never rotates a partially published identity");
        Release(child);
    }
}

static void ExplicitFreshAuthorityCreatesAndRetainsNewIdentity()
{
    F = {};
    Child child;
    for (auto& count : child.Store->Count) { count = 0; }
    child.Context.PublicationMode = 1; // Explicit negotiated EstablishNew.
    Check(VadPreparePolicy(&child, 0) == STATUS_SUCCESS && F.IdentityWrites == 1 &&
        F.IdentityFlushes >= 1 && F.Registered == 3,
        "negotiated EstablishNew enables first publication without an external provisioner");
    Check(std::wcscmp(child.Context.PolicyReference,
        L"MoirenEndpoint-fedcba98765432100123456789abcdef-00000000000000020000000000000001-p") == 0,
        "fresh reference keeps full installation/StableId bits plus direction, not a name hash");
    WCHAR retained[vad::EndpointReferenceUnits] = {};
    std::memcpy(retained, child.Context.PolicyReference, sizeof(retained));
    const auto stored = child.Devnode;
    Release(child);
    F = {};
    Child renamed;
    renamed.Devnode = stored;
    const WCHAR changed[MOIREN_VAD_MAX_NAME_UNITS] = L"Fresh endpoint renamed later";
    std::memcpy(renamed.Shared.Slots[0].Name, changed, sizeof(changed));
    Check(VadPreparePolicy(&renamed, 0) == STATUS_SUCCESS && F.IdentityWrites == 0 &&
        std::wcscmp(renamed.Context.PolicyReference, retained) == 0,
        "fresh identity becomes immutable durable authority across recreation/name changes");
    Release(renamed);
}

static void LegacyCreateDoesNotRequireIdentityProvisioning()
{
    F = {};
    F.RequireDurableIdentity = false;
    Child child;
    child.Context.PublicationMode = 0;
    for (auto& count : child.Store->Count) { count = 0; }
    Check(VadPreparePolicy(&child, 0) == STATUS_SUCCESS && F.Registered == 3 &&
        F.IdentityWrites == 0 && F.Enumerated == 0,
        "legacy v2 CREATE keeps working without any identity/fresh provisioning");
    WCHAR expected[vad::EndpointReferenceUnits] = {};
    vad::EndpointReference(child.Shared.Slots[0].Name, expected);
    Check(std::wcscmp(expected, child.Context.PolicyReference) == 0,
        "legacy compatibility retains exactly the locked-name reference");
    Release(child);
}

static void LegacyRecreationCannotRotateCommittedIdentity()
{
    for (bool fresh : {false, true}) {
        F = {};
        Child original;
        if (fresh) {
            original.Context.PublicationMode = MOIREN_VAD_IDENTITY_ESTABLISH_NEW;
            for (auto& count : original.Store->Count) { count = 0; }
        }
        Check(VadPreparePolicy(&original, 0) == STATUS_SUCCESS, "seed negotiated exact identity");
        WCHAR retained[vad::EndpointReferenceUnits] = {};
        std::memcpy(retained, original.Context.PolicyReference, sizeof(retained));
        const auto stored = original.Devnode;
        Release(original);
        for (bool corrupt : {false, true}) {
            F = {};
            Child legacy;
            legacy.Devnode = stored;
            legacy.Context.PublicationMode = 0;
            const WCHAR renamed[MOIREN_VAD_MAX_NAME_UNITS] = L"Changed through old v2 client";
            std::memcpy(legacy.Shared.Slots[0].Name, renamed, sizeof(renamed));
            if (corrupt) { legacy.Devnode.Record[0] ^= 1; }
            const auto status = VadPreparePolicy(&legacy, 0);
            Check(status == (corrupt ? STATUS_DATA_ERROR : STATUS_SUCCESS) &&
                F.IdentityWrites == 0 && F.Enumerated == 0,
                "legacy downgrade must read committed identity, never replace or adopt history");
            Check(corrupt ? F.Registered == 0 && legacy.Context.PolicyReference[0] == L'\0' :
                F.Registered == 3 && std::wcscmp(retained, legacy.Context.PolicyReference) == 0,
                "mixed-version recreation cannot publish another endpoint or bypass corrupt state");
            Release(legacy);
        }
    }
}

static void RootTokenIsDurableAndNeverRotatesOnRetry()
{
    F = {};
    Child root;
    MOIREN_VAD_ENDPOINT_ID first = {};
    SystemThread = true;
    Check(vad::ResolveRootPublicationIdentity(&root, first) == STATUS_SUCCESS,
        "root token is created by the actual management operation");
    SystemThread = false;
    Check(first.Low != 0 && first.High != 0 && F.UuidCalls == 1 &&
        F.IdentityWrites == 1 && F.IdentityReads == 2 && F.IdentityFlushes == 1 &&
        F.IdentityOpens == F.IdentityCloses, "root is durably flushed/read back with local key ownership");
    auto stored = root.Devnode;
    for (bool corrupt : {false, true}) {
        F = {};
        Child recreated;
        recreated.Devnode = stored;
        F.CorruptRootReadback = corrupt;
        MOIREN_VAD_ENDPOINT_ID token = {};
        SystemThread = true;
        const auto status = vad::ResolveRootPublicationIdentity(&recreated, token);
        SystemThread = false;
        Check(status == (corrupt ? STATUS_DATA_ERROR : STATUS_SUCCESS) && F.UuidCalls == 0 &&
            F.IdentityWrites == 0 && F.IdentityOpens == F.IdentityCloses,
            "valid root is reused; corrupt state never authorizes minting a replacement");
        Check(corrupt ? token.Low == 0 && token.High == 0 :
            token.Low == first.Low && token.High == first.High, "no failed root authority escapes");
    }
    for (unsigned boundary = 1; boundary <= 5; ++boundary) {
        F = {};
        Child failed;
        F.FailIdentityAt = boundary;
        MOIREN_VAD_ENDPOINT_ID token = {};
        SystemThread = true;
        const auto status = vad::ResolveRootPublicationIdentity(&failed, token);
        SystemThread = false;
        Check(status == IdentityFailure && token.Low == 0 && token.High == 0 &&
            F.IdentityOpens == F.IdentityCloses, "root I/O failure is not freshness and leaks no authority");
        if (failed.Devnode.RootPresent) {
            const auto retained = failed.Devnode;
            F = {};
            SystemThread = true;
            Check(vad::ResolveRootPublicationIdentity(&failed, token) == STATUS_SUCCESS,
                "retry finishes the exact reserved root token");
            SystemThread = false;
            Check(F.UuidCalls == 0 && F.IdentityWrites == 0 &&
                std::memcmp(retained.RootRecord, failed.Devnode.RootRecord, sizeof(retained.RootRecord)) == 0,
                "failed flush/readback never rotates reserved root identity");
        }
    }
    F = {};
    F.UuidStatus = IdentityFailure;
    Child failed;
    MOIREN_VAD_ENDPOINT_ID token = {};
    SystemThread = true;
    Check(vad::ResolveRootPublicationIdentity(&failed, token) == IdentityFailure &&
        F.IdentityWrites == 0, "UUID failure propagates without reserving guessed bytes");
    SystemThread = false;
}

static void FreshReservationRetryKeepsExactReference()
{
    for (unsigned boundary : {4u, 5u}) { // child write succeeded; flush/readback fails
        F = {};
        Child child;
        child.Context.PublicationMode = MOIREN_VAD_IDENTITY_ESTABLISH_NEW;
        for (auto& count : child.Store->Count) { count = 0; }
        F.FailIdentityAt = boundary;
        Check(VadPreparePolicy(&child, 0) == IdentityFailure && child.Store->Present &&
            F.Registered == 0 && child.Context.PolicyReference[0] == L'\0',
            "failed fresh commit cannot publish or discard its reserved identity");
        const auto reserved = child.Devnode;
        Release(child);
        F = {};
        Check(VadPreparePolicy(&child, 0) == STATUS_SUCCESS && F.IdentityWrites == 0 &&
            F.Enumerated == 0 && F.IdentityFlushes == 1 &&
            std::memcmp(reserved.Record, child.Store->Record, sizeof(reserved.Record)) == 0,
            "EstablishNew retry reuses the exact durable reservation, never allocates another");
        Release(child);
    }
    for (NTSTATUS uuidStatus : {RPC_NT_UUID_LOCAL_ONLY, IdentityFailure}) {
        F = {};
        F.UuidStatus = uuidStatus;
        Child root;
        MOIREN_VAD_ENDPOINT_ID token = {};
        SystemThread = true;
        const auto status = vad::ResolveRootPublicationIdentity(&root, token);
        SystemThread = false;
        Check(uuidStatus == RPC_NT_UUID_LOCAL_ONLY ?
            status == STATUS_SUCCESS && F.IdentityWrites == 1 && token.Low != 0 :
            status == IdentityFailure && F.IdentityWrites == 0 && token.Low == 0,
            "supported machine-local UUID succeeds; generation failure never becomes fresh authority");
    }
}

static void ExplicitFreshNeverReplacesInstalledIdentityOrMasksDiscoveryFailure()
{
    for (unsigned mode = 0; mode < 4; ++mode) {
        F = {};
        Child child;
        if (mode >= 2) {
            Check(VadPreparePolicy(&child, 0) == STATUS_SUCCESS, "seed exact adopted record");
            Release(child);
            F = {};
        }
        child.Context.PublicationMode = MOIREN_VAD_IDENTITY_ESTABLISH_NEW;
        if (mode == 1) {
            for (auto& count : child.Store->Count) { count = 0; }
            F.FailEnumerationAt = 1;
            F.EnumerationFailure = STATUS_NOT_FOUND;
        }
        if (mode == 3) { ++child.Context.InstallationId.High; }
        const auto before = child.Devnode;
        const auto status = VadPreparePolicy(&child, 0);
        Check(status == (mode == 1 ? STATUS_NOT_FOUND : STATUS_INVALID_DEVICE_STATE) &&
            F.Registered == 0 && F.IdentityWrites == 0 && child.Context.PolicyReference[0] == L'\0',
            "EstablishNew rejects legacy identity and all discovery errors, even NOT_FOUND");
        Check(std::memcmp(before.Record, child.Store->Record, sizeof(before.Record)) == 0,
            "conflict never overwrites installed settings authority");
        Release(child);
    }
}

static void LockedSnapshotOwnsPublicationReference()
{
    F = {};
    Child child;
    F.ChangeNameAfterSnapshot = true;
    Check(VadPreparePolicy(&child, 0) == STATUS_SUCCESS, "locked snapshot stages");
    WCHAR expected[vad::EndpointReferenceUnits] = {};
    vad::EndpointReference(F.SnapshotName, expected);
    Check(std::wcscmp(F.SnapshotName, child.Shared.Slots[0].Name) != 0 &&
        std::wcscmp(child.Context.PolicyReference, expected) == 0,
        "post-snapshot mutation cannot replace the staged publication reference");
    for (unsigned index = 0; index < 3; ++index) {
        Check(F.References[index] == child.Context.PolicyReference &&
            std::wcscmp(F.Names[index], F.SnapshotName) == 0,
            "Audio/flow/Realtime share the same reference and requested-name snapshot");
    }
    Check(VadPreparePolicy(&child, 0) == STATUS_SUCCESS && F.Registered == 3 &&
        std::wcscmp(child.Context.PolicyReference, expected) == 0,
        "retained policy does not rederive a reference from the changed live name");
    Release(child);

    for (bool unterminated : {false, true}) {
        F = {};
        Child missing;
        missing.Context.PolicyInterfaceCount = 3;
        if (unterminated) {
            for (auto& unit : missing.Context.PolicyReference) { unit = L'X'; }
        }
        Check(VadPreparePolicy(&missing, 0) == STATUS_INVALID_DEVICE_STATE && F.Registered == 0,
            "cached interface count cannot authorize an empty or unterminated reference");
        Release(missing);
    }
}

static void TerminalPolicyMatchesVirtualFunctionAndDefaultPreference()
{
    for (auto intent : {MOIREN_VAD_INTENT_PLAYBACK, MOIREN_VAD_INTENT_MICROPHONE}) {
        for (bool allowDefault : {false, true}) {
            F = {};
            Child child;
            child.Shared.Slots[0].Intent = intent;
            child.Shared.Slots[0].AllowDefault = allowDefault;
            Check(VadPreparePolicy(&child, intent) == STATUS_SUCCESS, "terminal policy stages");
            const bool capture = intent == MOIREN_VAD_INTENT_MICROPHONE;
            Check(F.AssociationCount == 3 && F.NeverDefaultCount == 3,
                "every owned interface receives Association and default exclusion");
            for (unsigned index = 0; index < 3; ++index) {
                Check(std::wcscmp(F.Associations[index], capture ?
                    L"{DFF21BE1-F70F-11D0-B917-00A0C9223196}" :
                    L"{DFF21CE0-F70F-11D0-B917-00A0C9223196}") == 0,
                    "virtual playback Association is OUTPUT_UNDEFINED; microphone is unchanged");
                Check(F.NeverDefault[index] == (allowDefault ? 0u : 0x307u),
                    "enablement never overrides local all-role default exclusion/opt-in");
            }
            Check(F.EnableCount == (capture ? 0u : 3u),
                "only virtual render receives explicit endpoint enablement");
            for (unsigned index = 0; index < F.EnableCount; ++index) {
                Check(F.EnableByDefault[index] == 0x101u,
                    "enable mask is FLAG_ENABLE plus FLOW_MASK_RENDER, not default selection");
            }
            Release(child);
        }
    }
}

static void TypedFailureClosesPolicy()
{
    for (unsigned index = 1; index <= 3; ++index) {
        for (bool failRead : {false, true}) {
            F = {};
            Child child;
            F.FailSetAt = failRead ? 0 : index;
            F.FailReadAt = failRead ? index : 0;
            Check(VadPreparePolicy(&child, 0) == (failRead ? ReadFailure : SetFailure),
                "propagate original typed DDI failure, do not publish ACX");
            Check(F.SetCalls == index && F.ReadCalls == (failRead ? index : index - 1),
                "fail once, never retry or read after failed set");
            Check(F.Registered == index && F.Disabled == 1, "stop before later categories");
            Check(F.NameTraceCalls == index, "failed typed operation still emits one bounded record");
            Release(child);
        }
    }
}

static void RejectReadbackMismatch()
{
    for (unsigned mode = 0; mode < 8; ++mode) {
        F = {};
        Child child;
        switch (mode) {
        case 0: F.ReadType = DEVPROP_TYPE_STRING_INDIRECT; break;
        case 1: F.ReadLengthDelta = -2; break;
        case 2: F.ReadLengthDelta = 2; break;
        case 3: F.ReadLengthDelta = -1; break;
        case 4: F.ReadLengthDelta = 200; break;
        case 5: F.DifferentName = true; break;
        case 6: F.MissingTerminator = true; break;
        case 7: F.OversizeRead = true; break;
        }
        const auto status = VadPreparePolicy(&child, 0);
        Check(status == (mode == 7 ? STATUS_BUFFER_TOO_SMALL : STATUS_DATA_ERROR),
            "reject type, exact bytes, content, terminator, and oversize mismatch");
        Check(F.SetCalls == 1 && F.ReadCalls == 1 && F.Disabled == 1,
            "mismatch closes first staged interface without retry");
        Release(child);
    }
}

static void FixedNameBounds()
{
    for (unsigned mode = 0; mode < 3; ++mode) {
        F = {};
        Child child;
        auto& name = child.Shared.Slots[0].Name;
        for (auto& unit : name) { unit = L'X'; }
        if (mode == 0) { name[0] = L'\0'; }
        if (mode == 2) { name[MOIREN_VAD_MAX_NAME_UNITS - 1] = L'\0'; }
        const auto status = VadPreparePolicy(&child, 0);
        Check(status == (mode == 2 ? STATUS_SUCCESS : STATUS_INVALID_PARAMETER),
            "reject empty/unterminated name, accept exactly maximum bounded name");
        if (mode != 2) {
            Check(F.Registered == 0 && F.SetCalls == 0, "invalid name never reaches DDI");
        } else {
            Check(F.Bytes[0] == sizeof(name), "maximum name includes its final NUL");
        }
        Release(child);
    }
}

static void ExistingFailureRollback()
{
    for (auto intent : {MOIREN_VAD_INTENT_PLAYBACK, MOIREN_VAD_INTENT_MICROPHONE}) {
        const unsigned registryCalls = intent == MOIREN_VAD_INTENT_PLAYBACK ? 30 : 27;
        for (unsigned index = 1; index <= registryCalls; ++index) {
            F = {};
            Child child;
            child.Shared.Slots[0].Intent = intent;
            F.FailRegistryAt = index;
            Check(VadPreparePolicy(&child, intent) == RegistryFailure,
                "every registry write, including render enablement, fails closed");
            Release(child);
        }
    }
    for (unsigned index = 1; index <= 3; ++index) {
        F = {};
        Child child;
        F.FailRegisterAt = index;
        Check(VadPreparePolicy(&child, 0) == ReadFailure, "registration failure still propagates");
        Release(child);
    }
}

static void DisabledTraceDoesNotChoosePolicyOutcome()
{
    for (bool failRead : {false, true}) {
        F = {};
        Child child;
        F.TraceEnabled = false;
        F.FailReadAt = failRead ? 2 : 0;
        Check(VadPreparePolicy(&child, 0) == (failRead ? ReadFailure : STATUS_SUCCESS),
            "WPP disabled cannot skip typed validation or change fail-closed result");
        Check(F.TraceCalls == 0 && F.ReadCalls == (failRead ? 2u : 3u),
            "typed DDI evidence evaluated independently of passive trace enablement");
        Release(child);
    }
}

static void EndpointReferenceDerivation()
{
    const WCHAR name[MOIREN_VAD_MAX_NAME_UNITS] = L"Requested interface";
    WCHAR reference[vad::EndpointReferenceUnits] = {};
    vad::EndpointReference(name, reference);
    Check(std::wcslen(reference) == 31, "prefix plus 16 hex digits, NUL terminated");
    Check(std::wcsncmp(reference, L"MoirenEndpoint-", 15) == 0,
        "reference keeps the Moiren endpoint prefix");
    WCHAR again[vad::EndpointReferenceUnits] = {};
    vad::EndpointReference(name, again);
    Check(std::wcscmp(reference, again) == 0, "derivation is deterministic");
    const WCHAR renamed[MOIREN_VAD_MAX_NAME_UNITS] = L"Renamed interface";
    WCHAR other[vad::EndpointReferenceUnits] = {};
    vad::EndpointReference(renamed, other);
    Check(std::wcscmp(reference, other) != 0,
        "historical derivation changes with display text and must not drive adopted identity");
    const WCHAR empty[MOIREN_VAD_MAX_NAME_UNITS] = {};
    WCHAR fromEmpty[vad::EndpointReferenceUnits] = {};
    vad::EndpointReference(empty, fromEmpty);
    Check(std::wcslen(fromEmpty) == 31 && std::wcscmp(fromEmpty, other) != 0,
        "empty input stays bounded and never aliases another name");
}

#ifndef VAD_PUBLICATION_REFERENCE_TEST
int main()
{
    FreshReservationRetryKeepsExactReference();
    LegacyRecreationCannotRotateCommittedIdentity();
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
    std::puts("PASS: production interface-name staging, strict readback, fail-closed rollback");
    return 0;
}
#endif
