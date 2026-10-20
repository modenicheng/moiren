// WPP scaffold adapted from Microsoft ACX Samples/Shared/Trace.h and
// Common/Trace_macros.h. Copyright (c) Microsoft Corporation.
// SPDX-License-Identifier: MS-PL
#pragma once
#include <WppRecorder.h>
#include <evntrace.h>

// Fixed in-flight recorder allocation; never log PCM, paths or user buffers.
#define WPP_TOTAL_BUFFER_SIZE PAGE_SIZE
#define WPP_ERROR_PARTITION_SIZE (WPP_TOTAL_BUFFER_SIZE / 4)

// {7DC75BA9-8274-4EAE-A42D-B4C74E0209D2}
#define WPP_CONTROL_GUIDS \
    WPP_DEFINE_CONTROL_GUID(MoirenVadTrace, \
        (7DC75BA9,8274,4EAE,A42D,B4C74E0209D2), \
        WPP_DEFINE_BIT(VAD_INIT) \
        WPP_DEFINE_BIT(VAD_SESSION) \
        WPP_DEFINE_BIT(VAD_PDO) \
        WPP_DEFINE_BIT(VAD_CONTROL))

#define WPP_LEVEL_FLAGS_LOGGER(level, flags) WPP_LEVEL_LOGGER(flags)
#define WPP_LEVEL_FLAGS_ENABLED(level, flags) \
    (WPP_LEVEL_ENABLED(flags) && WPP_CONTROL(WPP_BIT_ ## flags).Level >= level)

// begin_wpp config
// FUNC VadTrace(LEVEL, FLAGS, MSG, ...);
// end_wpp
