// SPDX-License-Identifier: MIT
#pragma once
#include "../include/moiren_vad.h"

namespace vad {
struct TerminalPolicy {
    GUID Category;
    wchar_t Association[39];
    uint32_t EnableByDefault;
};

inline constexpr TerminalPolicy MakeTerminalPolicy(bool capture)
{
    // The playback bridge is a software destination, not a physical speaker.
    // OUTPUT_UNDEFINED is UnknownFormFactor and requires explicit render enablement.
    TerminalPolicy policy = {
        {capture ? 0xdff21be1u : 0xdff21ce0u, 0xf70f, 0x11d0,
            {0xb9, 0x17, 0x00, 0xa0, 0xc9, 0x22, 0x31, 0x96}},
        {}, capture ? 0u : 0x101u}; // FLAG_ENABLE | FLOW_MASK_RENDER, not default selection.
    // Derive the registry Association from the same GUID ACX publishes, so the
    // enable/default-exclusion properties cannot target a different terminal.
    constexpr wchar_t digits[] = L"0123456789ABCDEF";
    uint32_t used = 0;
    const auto hex = [&](uint64_t value, uint32_t count) constexpr {
        for (uint32_t index = count; index != 0; --index) {
            policy.Association[used++] = digits[(value >> ((index - 1) * 4)) & 0xf];
        }
    };
    policy.Association[used++] = L'{';
    hex(policy.Category.Data1, 8);
    policy.Association[used++] = L'-';
    hex(policy.Category.Data2, 4);
    policy.Association[used++] = L'-';
    hex(policy.Category.Data3, 4);
    policy.Association[used++] = L'-';
    for (uint32_t index = 0; index < 8; ++index) {
        if (index == 2) { policy.Association[used++] = L'-'; }
        hex(policy.Category.Data4[index], 2);
    }
    policy.Association[used] = L'}';
    return policy;
}

inline constexpr auto PlaybackTerminal = MakeTerminalPolicy(false);
inline constexpr auto MicrophoneTerminal = MakeTerminalPolicy(true);

inline constexpr const TerminalPolicy& EndpointTerminalPolicy(uint32_t intent)
{
    return intent == MOIREN_VAD_INTENT_MICROPHONE ? MicrophoneTerminal : PlaybackTerminal;
}
}
