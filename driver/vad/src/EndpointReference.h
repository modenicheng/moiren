// SPDX-License-Identifier: MIT
#pragma once
#include "../include/moiren_vad.h"

namespace vad {
// "MoirenEndpoint-" + 16 hex digits + NUL.
inline constexpr uint32_t EndpointReferenceUnits = 32;

// Current implementation hashes mutable display text into the interface and
// ACX circuit reference. That reference is an input to Windows endpoint
// identity: changing it can lose retained endpoint settings. This is not a
// qualified rename contract. Separate immutable publication identity from the
// requested name, with explicit adoption of existing references; simply
// switching installed endpoints to a new constant would also change identity.
// See docs/validation/2026-11-08-vad-name-identity-contract.md. The hash below
// remains the historical derivation until publication/migration is implemented.
inline void EndpointReference(const wchar_t (&name)[MOIREN_VAD_MAX_NAME_UNITS],
    wchar_t (&reference)[EndpointReferenceUnits])
{
    static_assert(sizeof(wchar_t) == 2, "interface references use UTF-16");
    uint64_t hash = 0xcbf29ce484222325ull; // FNV-1a 64 over the UTF-16 units
    for (uint32_t index = 0; index < MOIREN_VAD_MAX_NAME_UNITS; ++index) {
        if (name[index] == L'\0') {
            break;
        }
        hash ^= static_cast<uint16_t>(name[index]);
        hash *= 0x100000001b3ull;
    }
    static const wchar_t prefix[] = L"MoirenEndpoint-";
    uint32_t used = 0;
    for (; prefix[used] != L'\0'; ++used) {
        reference[used] = prefix[used];
    }
    static const wchar_t digits[] = L"0123456789abcdef";
    for (int digit = 15; digit >= 0; --digit) {
        reference[used++] = digits[hash & 0xf];
        hash >>= 4;
    }
    reference[used] = L'\0';
}
}
