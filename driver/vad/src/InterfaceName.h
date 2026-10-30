// SPDX-License-Identifier: MIT
#pragma once
#include "../include/moiren_vad.h"

namespace vad {
// Registry REG_SZ uses UTF-16 bytes including the terminator. Keep the fixed
// slot bound here instead of relying on an unbounded scan at the DDI boundary.
inline uint32_t InterfaceNameBytes(const wchar_t (&name)[MOIREN_VAD_MAX_NAME_UNITS])
{
    static_assert(sizeof(wchar_t) == 2, "interface properties use UTF-16");
    for (uint32_t index = 0; index < MOIREN_VAD_MAX_NAME_UNITS; ++index) {
        if (name[index] == L'\0') {
            return index == 0 ? 0 : (index + 1) * sizeof(wchar_t);
        }
    }
    return 0;
}
}
