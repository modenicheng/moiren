// SPDX-License-Identifier: MIT
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include "../include/moiren_vad.h"

// Compile the real staging function; only registry/interface DDIs are stubs.
// This proves bounded name publication, not PnP or EndpointBuilder behavior.
using WCHAR = wchar_t;
using PCWSTR = const WCHAR*;
using ULONG = std::uint32_t;
using NTSTATUS = std::int32_t;
using HANDLE = void*;
using PHANDLE = HANDLE*;
using WDFDEVICE = void*;
using GUID = unsigned;
constexpr GUID TypedCategory = 0;
constexpr NTSTATUS STATUS_SUCCESS = 0;
constexpr NTSTATUS STATUS_INVALID_PARAMETER = -1;
constexpr NTSTATUS RegistryFailure = -7;
constexpr ULONG KEY_CREATE_SUB_KEY = 4;
constexpr ULONG KEY_SET_VALUE = 2;
constexpr ULONG REG_SZ = 1;
constexpr ULONG REG_DWORD = 4;
constexpr bool FALSE = false;
constexpr WCHAR VadAudioReference[] = L"MoirenEndpoint";
struct UNICODE_STRING { WCHAR* Buffer; };
#define NT_SUCCESS(status) ((status) >= 0)
static_assert(sizeof(WCHAR) == 2, "exercise Windows UTF-16 sizes");

static void Check(bool condition, const char* message)
{
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        std::exit(1);
    }
}

// Do not pass short wchar_t strings to a platform's wide libc ABI.
static bool SameUnits(PCWSTR left, PCWSTR right)
{
    while (*left == *right) {
        if (*left == L'\0') { return true; }
        ++left;
        ++right;
    }
    return false;
}

struct Fixture {
    ULONG ExpectedBytes = 0;
    unsigned Registered = 0;
    unsigned Names = 0;
    unsigned Handles = 0;
    unsigned Freed = 0;
    unsigned Disabled = 0;
    bool FailRegistry = false;
    PCWSTR ExpectedReference = VadAudioReference;
    WCHAR Link[2] = L"L";
} F;

void RtlInitUnicodeString(UNICODE_STRING* value, PCWSTR text)
{
    value->Buffer = const_cast<WCHAR*>(text);
}
WDFDEVICE WdfDeviceWdmGetDeviceObject(WDFDEVICE device) { return device; }
NTSTATUS IoRegisterDeviceInterface(WDFDEVICE, const GUID*, UNICODE_STRING* reference,
    UNICODE_STRING* link)
{
    Check(SameUnits(reference->Buffer, F.ExpectedReference),
        "staging preserves the exact supplied interface identity");
    ++F.Registered;
    link->Buffer = F.Link;
    return STATUS_SUCCESS;
}
NTSTATUS IoOpenDeviceInterfaceRegistryKey(UNICODE_STRING*, ULONG, PHANDLE result)
{
    if (F.FailRegistry) { return RegistryFailure; }
    ++F.Handles;
    *result = &F;
    return STATUS_SUCCESS;
}
NTSTATUS CreateKey(HANDLE, PCWSTR, PHANDLE result)
{
    ++F.Handles;
    *result = &F;
    return STATUS_SUCCESS;
}
NTSTATUS SetValue(HANDLE, PCWSTR key, ULONG type, const void* data, ULONG bytes)
{
    if (SameUnits(key, L"FriendlyName") ||
        SameUnits(key, L"{A45C254E-DF1C-4EFD-8020-67D146A850E0},14")) {
        Check(type == REG_SZ && bytes == F.ExpectedBytes,
            "both name properties use exact UTF-16 bytes including one terminator");
        Check(static_cast<PCWSTR>(data)[bytes / sizeof(WCHAR) - 1] == L'\0',
            "published name includes its terminator");
        ++F.Names;
    }
    return STATUS_SUCCESS;
}
void ZwClose(HANDLE) { Check(F.Handles > 0, "close owned handle"); --F.Handles; }
NTSTATUS IoSetDeviceInterfaceState(UNICODE_STRING*, bool enable)
{
    Check(!enable, "staging never enables an interface");
    ++F.Disabled;
    return STATUS_SUCCESS;
}
void RtlFreeUnicodeString(UNICODE_STRING* link)
{
    Check(link->Buffer == F.Link && F.Freed == 0, "release link exactly once");
    ++F.Freed;
    link->Buffer = nullptr;
}

// The native suite verifies the real property publication. This stub keeps
// the portable test at the bounded REG_SZ staging boundary after integration.
static NTSTATUS StageInterfaceName(const GUID&, UNICODE_STRING*,
    const WCHAR (&)[MOIREN_VAD_MAX_NAME_UNITS], ULONG) { return STATUS_SUCCESS; }

#include "interface-name-production.inc"

static NTSTATUS StageForName(const WCHAR (&name)[MOIREN_VAD_MAX_NAME_UNITS],
    UNICODE_STRING* link)
{
#ifdef MOIREN_INTERFACE_NAME_TYPED_STAGE
    WCHAR reference[vad::EndpointReferenceUnits] = {};
    vad::EndpointReference(name, reference);
    F.ExpectedReference = reference;
    return StageInterface(nullptr, TypedCategory, L"association", 24, 0, name, reference, link);
#else
    return StageInterface(nullptr, TypedCategory, L"association", 24, 0, name, link);
#endif
}

static void InvalidName(bool terminated)
{
    F = {};
    WCHAR name[MOIREN_VAD_MAX_NAME_UNITS];
    for (auto& unit : name) { unit = L'X'; }
    if (terminated) { name[0] = L'\0'; }
    F.ExpectedBytes = terminated ? sizeof(WCHAR) : sizeof(name);
    UNICODE_STRING link = {};
    Check(StageForName(name, &link) ==
        STATUS_INVALID_PARAMETER, "reject empty or unterminated name before registration");
    Check(F.Registered == 0 && F.Names == 0 && F.Handles == 0 && link.Buffer == nullptr,
        "invalid names never reach native DDIs or retain link ownership");
}

static void ValidName(const WCHAR (&name)[MOIREN_VAD_MAX_NAME_UNITS], ULONG bytes)
{
    F = {};
    F.ExpectedBytes = bytes;
    UNICODE_STRING link = {};
    Check(StageForName(name, &link) == STATUS_SUCCESS,
        "valid bounded name stages successfully");
    Check(F.Registered == 1 && F.Names == 2 && F.Handles == 0 && F.Freed == 0,
        "both existing properties staged; success retains only the link");
    RtlFreeUnicodeString(&link);
}

int main()
{
    InvalidName(true);
    InvalidName(false);
    WCHAR maximum[MOIREN_VAD_MAX_NAME_UNITS];
    for (auto& unit : maximum) { unit = L'X'; }
    maximum[MOIREN_VAD_MAX_NAME_UNITS - 1] = L'\0';
    ValidName(maximum, sizeof(maximum));
    const WCHAR unicode[MOIREN_VAD_MAX_NAME_UNITS] = {0x4e2d, 0xd83c, 0xdfb5, 0};
    ValidName(unicode, 8);
    const WCHAR trailing[MOIREN_VAD_MAX_NAME_UNITS] = {L'A', 0, L'X'};
    ValidName(trailing, 4);
    const WCHAR single[MOIREN_VAD_MAX_NAME_UNITS] = L"A";
    ValidName(single, 4);
    F = {};
    F.FailRegistry = true;
    UNICODE_STRING link = {};
    Check(StageForName(single, &link) == RegistryFailure,
        "original registry failure is preserved");
    Check(F.Registered == 1 && F.Disabled == 1 && F.Freed == 1 && F.Handles == 0,
        "existing failure rollback still disables and frees the owned link");
    std::puts("PASS: 7 bounded interface-name staging cases");
}
