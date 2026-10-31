// SPDX-License-Identifier: MIT
#pragma once
#include "../include/moiren_vad.h"

namespace vad {
// "MoirenEndpoint-" + 16 hex digits + NUL.
inline constexpr uint32_t EndpointReferenceUnits = 32;

// Derive the audio interface reference string (and the matching ACX circuit
// name) from the requested endpoint name. The audio service composes and
// persists the MMDevice friendly name per endpoint identity, and the device
// interface reference string participates in that identity: with a fixed
// reference, a same-ID replacement endpoint returns to the previously
// composed name and never consumes the properties staged before enable.
// A name-derived reference gives each requested name a fresh endpoint
// identity that is composed from the newly staged properties. The hash keeps
// the reference bounded and charset-safe; display text always comes from the
// staged properties, never from this string.
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
