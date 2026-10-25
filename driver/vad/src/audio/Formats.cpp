// Copyright (c) Microsoft Corporation. All rights reserved.
// Portions Copyright (c) 2026 Moiren contributors.
// SPDX-License-Identifier: MS-PL
// Derived format construction: ACX Samples/Common/CircuitHelper.cpp.
#include "Private.h"
#include "Formats.tmh"

static ULONG ChannelMask(ULONG channels)
{
    return channels == 1 ? SPEAKER_FRONT_CENTER :
        SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT;
}

VAD_PAGE NTSTATUS VadAudioValidateFormat(VAD_AUDIO_DEVICE* device, ACXDATAFORMAT format,
    ULONG* blockAlign)
{
    PAGED_CODE();
    if (format == nullptr) {
        return STATUS_NO_MATCH;
    }
    const auto wave = static_cast<const WAVEFORMATEX*>(
        AcxDataFormatGetWaveFormatEx(format));
    const GUID sub = AcxDataFormatGetSubFormat(format);
    if (wave == nullptr ||
        wave->nChannels != device->Channels ||
        wave->nSamplesPerSec != MOIREN_VAD_SAMPLE_RATE) {
        return STATUS_NO_MATCH;
    }
    const bool pcm = IsEqualGUID(sub, KSDATAFORMAT_SUBTYPE_PCM) &&
        wave->wBitsPerSample == 16;
    const bool floating = IsEqualGUID(sub, KSDATAFORMAT_SUBTYPE_IEEE_FLOAT) &&
        wave->wBitsPerSample == 32;
    if (!pcm && !floating) {
        return STATUS_NO_MATCH;
    }
    const ULONG align = device->Channels * (wave->wBitsPerSample / 8);
    if (wave->nBlockAlign != align ||
        wave->nAvgBytesPerSec != MOIREN_VAD_SAMPLE_RATE * align) {
        return STATUS_NO_MATCH;
    }
    if (wave->wFormatTag == WAVE_FORMAT_EXTENSIBLE) {
        const auto ext = static_cast<const WAVEFORMATEXTENSIBLE*>(
            AcxDataFormatGetWaveFormatExtensible(format));
        if (wave->cbSize != sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX) ||
            ext == nullptr ||
            ext->Samples.wValidBitsPerSample != wave->wBitsPerSample ||
            ext->dwChannelMask != ChannelMask(device->Channels) ||
            !IsEqualGUID(ext->SubFormat, sub)) {
            return STATUS_NO_MATCH;
        }
    } else if (wave->cbSize != 0 ||
        wave->wFormatTag != (pcm ? WAVE_FORMAT_PCM : WAVE_FORMAT_IEEE_FLOAT)) {
        return STATUS_NO_MATCH;
    }
    *blockAlign = align;
    return STATUS_SUCCESS;
}

VAD_PAGE NTSTATUS VadAudioAddFormats(VAD_AUDIO_DEVICE* device, ACXPIN host)
{
    PAGED_CODE();
    const GUID* modes[] = {
        &AUDIO_SIGNALPROCESSINGMODE_RAW, &AUDIO_SIGNALPROCESSINGMODE_DEFAULT};
    for (const GUID* mode : modes) {
        ACXDATAFORMATLIST list = nullptr;
        const bool raw = IsEqualGUID(*mode, AUDIO_SIGNALPROCESSINGMODE_RAW);
        NTSTATUS status = STATUS_SUCCESS;
        if (raw) {
            // Every ACX pin already owns its RAW list. Replacing that reserved
            // association is invalid; populate the built-in list instead.
            list = AcxPinGetRawDataFormatList(host);
            if (list == nullptr) {
                status = STATUS_INSUFFICIENT_RESOURCES;
            }
        } else {
            ACX_DATAFORMAT_LIST_CONFIG listConfig;
            ACX_DATAFORMAT_LIST_CONFIG_INIT(&listConfig);
            WDF_OBJECT_ATTRIBUTES listAttributes;
            WDF_OBJECT_ATTRIBUTES_INIT(&listAttributes);
            listAttributes.ParentObject = host;
            status = AcxDataFormatListCreate(device->Device,
                &listAttributes, &listConfig, &list);
        }
        VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
            "Formats list mode=%u status=%!STATUS!",
            IsEqualGUID(*mode, AUDIO_SIGNALPROCESSINGMODE_RAW) ? 0u : 1u, status);
        if (!NT_SUCCESS(status)) {
            return status;
        }
        const ULONG bitDepths[] = {16, 32};
        for (ULONG bits : bitDepths) {
            KSDATAFORMAT_WAVEFORMATEXTENSIBLE ks = {};
            ks.DataFormat.FormatSize = sizeof(ks);
            ks.DataFormat.SampleSize = device->Channels * bits / 8;
            ks.DataFormat.MajorFormat = KSDATAFORMAT_TYPE_AUDIO;
            ks.DataFormat.SubFormat = bits == 16 ?
                KSDATAFORMAT_SUBTYPE_PCM : KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
            ks.DataFormat.Specifier = KSDATAFORMAT_SPECIFIER_WAVEFORMATEX;
            auto& wave = ks.WaveFormatExt;
            wave.Format.wFormatTag = WAVE_FORMAT_EXTENSIBLE;
            wave.Format.nChannels = static_cast<WORD>(device->Channels);
            wave.Format.nSamplesPerSec = MOIREN_VAD_SAMPLE_RATE;
            wave.Format.nBlockAlign = static_cast<WORD>(ks.DataFormat.SampleSize);
            wave.Format.nAvgBytesPerSec = MOIREN_VAD_SAMPLE_RATE *
                wave.Format.nBlockAlign;
            wave.Format.wBitsPerSample = static_cast<WORD>(bits);
            wave.Format.cbSize = sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX);
            wave.Samples.wValidBitsPerSample = static_cast<WORD>(bits);
            wave.dwChannelMask = ChannelMask(device->Channels);
            wave.SubFormat = ks.DataFormat.SubFormat;
            ACX_DATAFORMAT_CONFIG config;
            ACX_DATAFORMAT_CONFIG_INIT_KS(&config, &ks);
            WDF_OBJECT_ATTRIBUTES attributes;
            WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
            attributes.ParentObject = device->Circuit;
            ACXDATAFORMAT data = nullptr;
            status = AcxDataFormatCreate(device->Device, &attributes, &config, &data);
            VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
                "Formats create bits=%u status=%!STATUS!", bits, status);
            if (!NT_SUCCESS(status)) {
                return status;
            }
            status = AcxDataFormatListAddDataFormat(list, data);
            VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
                "Formats add bits=%u status=%!STATUS!", bits, status);
            if (!NT_SUCCESS(status)) {
                return status;
            }
        }
        status = raw ? STATUS_SUCCESS :
            AcxPinAssignModeDataFormatList(host, mode, list);
        VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
            "Formats assign mode=%u status=%!STATUS!",
            IsEqualGUID(*mode, AUDIO_SIGNALPROCESSINGMODE_RAW) ? 0u : 1u, status);
        if (!NT_SUCCESS(status)) {
            return status;
        }
    }
    return STATUS_SUCCESS;
}
