// SPDX-License-Identifier: MIT
#pragma once
#include "../Driver.h"

// Must match Policy.cpp's interface reference string before publication.
inline constexpr WCHAR VadAudioReference[] = L"MoirenEndpoint";

// PASSIVE_LEVEL, before WdfDeviceCreate. Installs the audio PnP callbacks.
_IRQL_requires_(PASSIVE_LEVEL)
VAD_PAGE NTSTATUS VadAudioConfigureInit(PWDFDEVICE_INIT init);

// PASSIVE_LEVEL, after WdfDeviceCreate and before policy/publication.
// Uses an additional WDF context; never changes VAD_CHILD's context layout.
_IRQL_requires_(PASSIVE_LEVEL)
VAD_PAGE NTSTATUS VadAudioInitialize(WDFDEVICE child, uint32_t intent, uint32_t channels,
    const MOIREN_VAD_ENDPOINT_ID& stableId);

// PASSIVE_LEVEL after admission closes and before it reopens. Never call while
// Shared->Lock is held; the stream lifecycle owns the mapped packet storage.
_IRQL_requires_(PASSIVE_LEVEL)
void VadAudioClearGateBuffers(WDFDEVICE child);
