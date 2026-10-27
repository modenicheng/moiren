// SPDX-License-Identifier: MIT
#pragma once
#include "../include/moiren_vad.h"

namespace vad {
// Plain UTF-16 only: no indirect resource resolution, allocation or unbounded
// scan. The returned size includes the single terminating NUL.
inline uint32_t InterfaceNameBytes(const wchar_t (&name)[MOIREN_VAD_MAX_NAME_UNITS])
{
    static_assert(sizeof(wchar_t) == 2, "interface properties use UTF-16");
    for (uint32_t index = 0; index < MOIREN_VAD_MAX_NAME_UNITS; ++index) {
        if (name[index] == L'\0') {
            return index == 0 ? 0 : (index + 1) * 2;
        }
    }
    return 0;
}

inline bool InterfaceNameMatches(const wchar_t (&expected)[MOIREN_VAD_MAX_NAME_UNITS],
    const wchar_t (&actual)[MOIREN_VAD_MAX_NAME_UNITS], uint32_t actualBytes)
{
    const auto expectedBytes = InterfaceNameBytes(expected);
    if (expectedBytes == 0 || actualBytes != expectedBytes) {
        return false;
    }
    // Equality includes the final NUL and never reads past either fixed array.
    for (uint32_t index = 0; index < expectedBytes / 2; ++index) {
        if (expected[index] != actual[index]) {
            return false;
        }
    }
    return true;
}
}
