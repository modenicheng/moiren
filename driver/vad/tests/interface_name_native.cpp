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
constexpr ULONG LOCALE_NEUTRAL = 0;
constexpr ULONG PLUGPLAY_PROPERTY_PERSISTENT = 1;
constexpr ULONG KEY_CREATE_SUB_KEY = 4;
constexpr ULONG KEY_SET_VALUE = 2;
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
// The pre-rename fixed reference; staging must no longer publish it.
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
};
struct SharedState { int Lock = 0; Slot Slots[1]; };
struct Child;
struct Work { Child* Parent; bool Queued = false; };
struct VAD_CHILD {
    SharedState* Shared;
    ULONG Index = 0;
    Work* PolicyWork;
    NTSTATUS PolicyStatus = STATUS_UNSUCCESSFUL;
    UNICODE_STRING PolicyInterfaces[3];
    ULONG PolicyInterfaceCount = 0;
};
struct Child {
    SharedState Shared;
    Work Worker{this};
    VAD_CHILD Context{&Shared, 0, &Worker};
};
using WDFDEVICE = Child*;
using WDFWORKITEM = Work*;

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
#define UNREFERENCED_PARAMETER(value) (void)(value)

struct Fixture {
    unsigned Registered = 0;
    unsigned Freed = 0;
    unsigned Disabled = 0;
    unsigned SetCalls = 0;
    unsigned ReadCalls = 0;
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
} F;
static bool SystemThread = false;
static bool SharedLocked = false;
static void PreparePolicy(WDFWORKITEM work);

KIRQL KeGetCurrentIrql() { return PASSIVE_LEVEL; }
VAD_CHILD* VadChild(WDFDEVICE child) { return &child->Context; }
Child* WdfWorkItemGetParentObject(Work* work) { return work->Parent; }
Child* WdfDeviceWdmGetDeviceObject(Child* child) { return child; }
void KeAcquireSpinLock(int*, KIRQL* irql) { SharedLocked = true; *irql = PASSIVE_LEVEL; }
void KeReleaseSpinLock(int*, KIRQL) { SharedLocked = false; }
void WdfWorkItemEnqueue(Work* work) { work->Queued = true; }
void WdfWorkItemFlush(Work* work)
{
    Check(work->Queued, "flush must execute queued policy");
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
    // The registered reference must be derived from the current slot name so a
    // rename composes a fresh endpoint identity instead of returning to the
    // persisted name of the fixed-reference identity.
    WCHAR expected[vad::EndpointReferenceUnits] = {};
    vad::EndpointReference(child->Shared.Slots[0].Name, expected);
    Check(std::wcscmp(reference->Buffer, expected) == 0,
        "reference derived from the staged name");
    Check(std::wcscmp(reference->Buffer, VadAudioReference) != 0,
        "reference is not the fixed pre-rename constant");
    if (F.Registered + 1 == F.FailRegisterAt) { return ReadFailure; }
    Check(F.Registered < 3, "only three owned categories");
    const auto index = F.Registered++;
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

#include "interface-name-native-production.inc"

static void Release(Child& child)
{
    SystemThread = true;
    VadReleasePolicy(&child);
    VadReleasePolicy(&child);
    SystemThread = false;
    Check(F.Freed == F.Registered && F.Handles == 0, "no leaked or double-freed policy resources");
    Check(child.Context.PolicyInterfaceCount == 0, "release clears ownership count");
    for (const auto& link : child.Context.PolicyInterfaces) {
        Check(link.Buffer == nullptr, "released or failed context link cleared");
    }
}

static void SuccessfulTypedPublication()
{
    for (auto intent : {MOIREN_VAD_INTENT_PLAYBACK, MOIREN_VAD_INTENT_MICROPHONE}) {
        F = {};
        Child child;
        child.Shared.Slots[0].Intent = intent;
        Check(VadPreparePolicy(&child, intent) == STATUS_SUCCESS, "typed policy succeeds");
        Check(F.SetCalls == 3 && F.ReadCalls == 3,
            "every owned category must set and strictly read back typed FriendlyName");
        Check(F.NameTraceCalls == 3, "one typed diagnostic per category, not per read attempt");
        Check(F.PlainNames == 3 && F.EndpointNames == 3, "ordinary and EP0 names retained");
        Check(F.Disabled == 0 && F.Freed == 0 && child.Context.PolicyInterfaceCount == 3,
            "success transfers all three disabled links to ReleasePolicy");
        Check(VadPreparePolicy(&child, intent) == STATUS_SUCCESS && F.SetCalls == 3 &&
            F.NameTraceCalls == 3,
            "already staged policy must not repeat publication or diagnostic");
        Release(child);
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
        "a rename derives a fresh endpoint identity");
    const WCHAR empty[MOIREN_VAD_MAX_NAME_UNITS] = {};
    WCHAR fromEmpty[vad::EndpointReferenceUnits] = {};
    vad::EndpointReference(empty, fromEmpty);
    Check(std::wcslen(fromEmpty) == 31 && std::wcscmp(fromEmpty, other) != 0,
        "empty input stays bounded and never aliases another name");
}

int main()
{
    TerminalPolicyMatchesVirtualFunctionAndDefaultPreference();
    EndpointReferenceDerivation();
    SuccessfulTypedPublication();
    TypedFailureClosesPolicy();
    RejectReadbackMismatch();
    FixedNameBounds();
    ExistingFailureRollback();
    DisabledTraceDoesNotChoosePolicyOutcome();
    std::puts("PASS: production interface-name staging, strict readback, fail-closed rollback");
}
