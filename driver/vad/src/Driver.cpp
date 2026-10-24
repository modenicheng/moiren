// Scaffold adapted from Microsoft AudioCodec Driver.cpp at the pinned commit.
// Copyright (c) Microsoft Corporation. All rights reserved.
// SPDX-License-Identifier: MS-PL
#include "Driver.h"
#include "Driver.tmh"

const GUID VadControlInterface = MOIREN_VAD_CONTROL_INTERFACE_GUID;

_Use_decl_annotations_
NTSTATUS DriverEntry(PDRIVER_OBJECT driverObject, PUNICODE_STRING registryPath)
{
    WDF_DRIVER_CONFIG config;
    WDF_DRIVER_CONFIG_INIT(&config, VadAddDevice);
    config.EvtDriverUnload = VadUnload;
    WPP_INIT_TRACING(driverObject, registryPath);
    WDFDRIVER driver = nullptr;
    NTSTATUS status = WdfDriverCreate(driverObject, registryPath,
        WDF_NO_OBJECT_ATTRIBUTES, &config, &driver);
    if (!NT_SUCCESS(status)) {
        WPP_CLEANUP(driverObject);
        return status;
    }
    ACX_DRIVER_CONFIG acx;
    ACX_DRIVER_CONFIG_INIT(&acx);
    status = AcxDriverInitialize(driver, &acx);
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_INIT,
        "ACX driver initialization status=%!STATUS!", status);
    if (!NT_SUCCESS(status)) {
        WPP_CLEANUP(driverObject);
    }
    return status;
}

VAD_PAGE _Use_decl_annotations_
void VadUnload(WDFDRIVER driver)
{
    PAGED_CODE();
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_INIT, "Driver unload");
    WPP_CLEANUP(WdfDriverWdmGetDriverObject(driver));
}
