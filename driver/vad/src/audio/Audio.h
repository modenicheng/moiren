// SPDX-License-Identifier: MIT
#pragma once
#include "../Driver.h"

// PASSIVE_LEVEL, before WdfDeviceCreate. Installs the audio PnP callbacks.
_IRQL_requires_(PASSIVE_LEVEL)
VAD_PAGE NTSTATUS VadAudioConfigureInit(PWDFDEVICE_INIT init);

// PASSIVE_LEVEL, after WdfDeviceCreate and before policy/publication.
// Uses an additional WDF context; never changes VAD_CHILD's context layout.
_IRQL_requires_(PASSIVE_LEVEL)
VAD_PAGE NTSTATUS VadAudioInitialize(WDFDEVICE child, uint32_t intent, uint32_t channels,
    const MOIREN_VAD_ENDPOINT_ID& stableId,
    const WCHAR (&name)[MOIREN_VAD_MAX_NAME_UNITS]);

// PASSIVE_LEVEL after admission closes and before it reopens. Never call while
// Shared->Lock is held; the stream lifecycle owns the mapped packet storage.
_IRQL_requires_(PASSIVE_LEVEL)
void VadAudioClearGateBuffers(WDFDEVICE child);

// Caller holds stream State, never Shared->Lock. Retains a first rebase while
// performing the same PCM epoch invalidation under the existing shared lock.
void VadAudioRebaseDiscontinuity(WDFDEVICE child, uint64_t generation,
    uint64_t stream, const vad::audio::ClockRebaseObservation& observation);
