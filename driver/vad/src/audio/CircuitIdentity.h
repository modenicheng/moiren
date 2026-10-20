// SPDX-License-Identifier: MIT
#pragma once
#include "../../include/moiren_vad.h"

namespace vad::audio {
struct CircuitIdentity {
    wchar_t Uri[46];
};

inline constexpr CircuitIdentity CircuitInstanceIdentity(const MOIREN_VAD_ENDPOINT_ID& endpoint,
    uint32_t intent)
{
    // ACX accepts a component URI as the instance identity. Keep every stable
    // bit and the direction without a lossy GUID hash or runtime/session token.
    CircuitIdentity identity = {L"moiren:vad:p:"};
    identity.Uri[11] = intent == MOIREN_VAD_INTENT_MICROPHONE ? L'm' : L'p';
    constexpr wchar_t digits[] = L"0123456789abcdef";
    for (uint32_t index = 0; index < 32; ++index) {
        const uint64_t word = index < 16 ? endpoint.High : endpoint.Low;
        const uint32_t shift = (15 - index % 16) * 4;
        identity.Uri[13 + index] = digits[(word >> shift) & 0xf];
    }
    return identity;
}
}
