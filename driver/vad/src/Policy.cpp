// SPDX-License-Identifier: MIT
#include "Driver.h"
#include "InterfaceName.h"
// Instantiate the SDK property key in this translation unit, not a private GUID.
#include <initguid.h>
#include <devpkey.h>
#include "Policy.tmh"

VAD_PAGE static EVT_WDF_WORKITEM PreparePolicy;

VAD_PAGE NTSTATUS VadInitializePolicy(WDFDEVICE child)
{
    PAGED_CODE();
    WDF_WORKITEM_CONFIG config;
    WDF_WORKITEM_CONFIG_INIT(&config, PreparePolicy);
    config.AutomaticSerialization = FALSE;
    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.ParentObject = child;
    return WdfWorkItemCreate(&config, &attributes, &VadChild(child)->PolicyWork);
}

static const GUID AudioCategory =
    {0x6994ad04, 0x93ef, 0x11d0, {0xa3, 0xcc, 0x00, 0xa0, 0xc9, 0x22, 0x31, 0x96}};
static const GUID RenderCategory =
    {0x65e8773e, 0x8f56, 0x11d0, {0xa3, 0xb9, 0x00, 0xa0, 0xc9, 0x22, 0x31, 0x96}};
static const GUID CaptureCategory =
    {0x65e8773d, 0x8f56, 0x11d0, {0xa3, 0xb9, 0x00, 0xa0, 0xc9, 0x22, 0x31, 0x96}};
static const GUID RealtimeCategory =
    {0xeb115ffc, 0x10c8, 0x4964, {0x83, 0x1d, 0x6d, 0xcb, 0x02, 0xe6, 0xf2, 0x3f}};

static void SnapshotPolicy(VAD_CHILD* context, uint32_t* intent,
    ULONG* neverDefault, WCHAR (&name)[MOIREN_VAD_MAX_NAME_UNITS])
{
    KIRQL irql;
    KeAcquireSpinLock(&context->Shared->Lock, &irql);
    const auto& slot = context->Shared->Slots[context->Index];
    *intent = slot.Intent;
    *neverDefault = slot.AllowDefault ? 0u : 0x307u;
    RtlCopyMemory(name, slot.Name, sizeof(name));
    KeReleaseSpinLock(&context->Shared->Lock, irql);
}

static NTSTATUS CreateKey(HANDLE parent, PCWSTR name, PHANDLE result)
{
    UNICODE_STRING path;
    RtlInitUnicodeString(&path, name);
    OBJECT_ATTRIBUTES attributes;
    InitializeObjectAttributes(&attributes, &path,
        OBJ_CASE_INSENSITIVE | OBJ_KERNEL_HANDLE, parent, nullptr);
    return ZwCreateKey(result, KEY_CREATE_SUB_KEY | KEY_SET_VALUE, &attributes,
        0, nullptr, REG_OPTION_NON_VOLATILE, nullptr);
}

static NTSTATUS SetValue(HANDLE key, PCWSTR name, ULONG type,
    const void* value, ULONG size)
{
    UNICODE_STRING path;
    RtlInitUnicodeString(&path, name);
    return ZwSetValueKey(key, &path, 0, type, const_cast<void*>(value), size);
}

static NTSTATUS StageInterfaceName(const GUID& category, UNICODE_STRING* link,
    const WCHAR (&name)[MOIREN_VAD_MAX_NAME_UNITS], ULONG nameBytes)
{
    // Only PreparePolicy's passive system worker calls this. Use the actual
    // owned link before ACX enables it; this is not a final MMDevice rename.
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    NTSTATUS status = IoSetDeviceInterfacePropertyData(link,
        &DEVPKEY_DeviceInterface_FriendlyName, LOCALE_NEUTRAL,
        PLUGPLAY_PROPERTY_PERSISTENT, DEVPROP_TYPE_STRING, nameBytes,
        const_cast<WCHAR*>(name));
    WCHAR readback[MOIREN_VAD_MAX_NAME_UNITS] = {};
    ULONG returnedBytes = 0;
    DEVPROPTYPE type = DEVPROP_TYPE_EMPTY;
    if (NT_SUCCESS(status)) {
        // Getter Flags is reserved (zero), not the setter's persistent flag.
        // Oversized/malformed results fail closed; never allocate or retry.
        status = IoGetDeviceInterfacePropertyData(link,
            &DEVPKEY_DeviceInterface_FriendlyName, LOCALE_NEUTRAL, 0,
            sizeof(readback), readback, &returnedBytes, &type);
    }
    const bool matches = NT_SUCCESS(status) && type == DEVPROP_TYPE_STRING &&
        vad::InterfaceNameMatches(name, readback, returnedBytes);
    if (NT_SUCCESS(status) && !matches) {
        status = STATUS_DATA_ERROR;
    }
    // One passive record per staged category; never log user-provided text.
    VadTrace(TRACE_LEVEL_INFORMATION, VAD_PDO,
        "Policy typed interface name category=%!GUID! bytes=%lu type=%lu returned=%lu match=%u status=%!STATUS!",
        &category, nameBytes, type, returnedBytes, matches ? 1u : 0u, status);
    return status;
}

// The interface registration is deliberately retained for the child
// lifetime. IoRegisterDeviceInterface documents that registrations are
// persistent devnode state and need not be manually deleted; only the opaque
// returned allocation is owned by this driver.
static NTSTATUS StageInterface(WDFDEVICE child, const GUID& category,
    PCWSTR association, ULONG associationBytes, ULONG neverDefault,
    const WCHAR (&name)[MOIREN_VAD_MAX_NAME_UNITS], UNICODE_STRING* link)
{
    *link = {};
    const ULONG nameBytes = vad::InterfaceNameBytes(name);
    if (nameBytes == 0) {
        return STATUS_INVALID_PARAMETER;
    }
    UNICODE_STRING reference;
    RtlInitUnicodeString(&reference, VadAudioReference);
    // Registration alone leaves the interface DISABLED. Never call
    // IoSetDeviceInterfaceState(TRUE) or let WDF automatically enable it.
    NTSTATUS status = IoRegisterDeviceInterface(
        WdfDeviceWdmGetDeviceObject(child), &category, &reference, link);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    HANDLE interfaceKey = nullptr;
    HANDLE endpointsKey = nullptr;
    HANDLE endpointKey = nullptr;
    status = IoOpenDeviceInterfaceRegistryKey(link,
        KEY_CREATE_SUB_KEY | KEY_SET_VALUE, &interfaceKey);
    static const WCHAR proxy[] = L"{17CCA71B-ECD7-11D0-B908-00A0C9223196}";
    if (NT_SUCCESS(status)) {
        // Dynamic raw PDOs have no per-endpoint AddInterface INF section.
        // Stage the same KSProxy metadata as the official ACX interfaces,
        // before ACX enables the interface for EndpointBuilder.
        status = SetValue(interfaceKey, L"CLSID", REG_SZ, proxy, sizeof(proxy));
    }
    if (NT_SUCCESS(status)) {
        status = SetValue(interfaceKey, L"FriendlyName", REG_SZ, name, nameBytes);
    }
    if (NT_SUCCESS(status)) {
        status = CreateKey(interfaceKey, L"EP", &endpointsKey);
    }
    if (NT_SUCCESS(status)) {
        status = CreateKey(endpointsKey, L"0", &endpointKey);
    }
    if (NT_SUCCESS(status)) {
        status = SetValue(endpointKey,
            L"{1DA5D803-D492-4EDD-8C23-E0C0FFEE7F0E},2", REG_SZ,
            association, associationBytes);
    }
    if (NT_SUCCESS(status)) {
        // All render/capture roles, including communications (0x307).
        // Association is mandatory in this same EP\\0 key.
        status = SetValue(endpointKey,
            L"{F3E80BEF-1723-4FF2-BCC4-7F83DC5E46D4},3", REG_DWORD,
            &neverDefault, sizeof(neverDefault));
    }
    if (NT_SUCCESS(status)) {
        const ULONG eventDriven = 1;
        status = SetValue(endpointKey,
            L"{1DA5D803-D492-4EDD-8C23-E0C0FFEE7F0E},7", REG_DWORD,
            &eventDriven, sizeof(eventDriven));
    }
    if (NT_SUCCESS(status)) {
        status = SetValue(endpointKey,
            L"{A45C254E-DF1C-4EFD-8020-67D146A850E0},14", REG_SZ, name,
            nameBytes);
    }
    if (endpointKey != nullptr) {
        ZwClose(endpointKey);
    }
    if (endpointsKey != nullptr) {
        ZwClose(endpointsKey);
    }
    if (interfaceKey != nullptr) {
        ZwClose(interfaceKey);
    }
    if (NT_SUCCESS(status)) {
        // Preserve ordinary FriendlyName/EP0 metadata. Typed publication and
        // its strict readback are the final gate, before ACX sees any link.
        status = StageInterfaceName(category, link, name, nameBytes);
    }
    if (!NT_SUCCESS(status)) {
        // The interface may have been registered before a registry write
        // failed. It is never enabled, but explicitly disable it here and
        // release the symbolic-link allocation.
        (void)IoSetDeviceInterfaceState(link, FALSE);
        RtlFreeUnicodeString(link);
        *link = {};
    }
    return status;
}

VAD_PAGE void VadReleasePolicy(WDFDEVICE child)
{
    PAGED_CODE();
    auto context = VadChild(child);
    for (ULONG i = 0; i < context->PolicyInterfaceCount; ++i) {
        // Copy into a known-initialized local before clearing the context.
        // This also makes the ownership transfer explicit to static analysis.
        UNICODE_STRING link = {};
        link = context->PolicyInterfaces[i];
        context->PolicyInterfaces[i] = {};
        if (link.Buffer != nullptr) {
            RtlFreeUnicodeString(&link);
        }
    }
    context->PolicyInterfaceCount = 0;
}

VAD_PAGE _Use_decl_annotations_
static void PreparePolicy(WDFWORKITEM work)
{
    PAGED_CODE();
    // WDF work items run in a system worker thread. PASSIVE_LEVEL alone,
    // including a passive I/O queue callback, does not satisfy this DDI.
    NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
    WDFDEVICE child = static_cast<WDFDEVICE>(WdfWorkItemGetParentObject(work));
    auto context = VadChild(child);
    uint32_t intent = 0;
    ULONG neverDefault = 0;
    WCHAR name[MOIREN_VAD_MAX_NAME_UNITS];
    SnapshotPolicy(context, &intent, &neverDefault, name);
    static const WCHAR speaker[] = L"{DFF21CE1-F70F-11D0-B917-00A0C9223196}";
    static const WCHAR microphone[] = L"{DFF21BE1-F70F-11D0-B917-00A0C9223196}";
    const bool capture = intent == MOIREN_VAD_INTENT_MICROPHONE;
    const PCWSTR association = capture ? microphone : speaker;
    const ULONG associationBytes = capture ? sizeof(microphone) : sizeof(speaker);
    const GUID* categories[] = {
        &AudioCategory, capture ? &CaptureCategory : &RenderCategory,
        &RealtimeCategory};
    if (context->PolicyInterfaceCount != 0) {
        context->PolicyStatus = context->PolicyInterfaceCount == 3 ?
            STATUS_SUCCESS : STATUS_INVALID_DEVICE_STATE;
        return;
    }
    for (const GUID* category : categories) {
        NTSTATUS status = StageInterface(child, *category, association,
            associationBytes, neverDefault, name,
            &context->PolicyInterfaces[context->PolicyInterfaceCount]);
        if (!NT_SUCCESS(status)) {
            VadTrace(TRACE_LEVEL_ERROR, VAD_PDO,
                "Policy staging failed intent=%u status=%!STATUS!", intent, status);
            VadReleasePolicy(child);
            context->PolicyStatus = status;
            return;
        }
        ++context->PolicyInterfaceCount;
    }
    context->PolicyStatus = STATUS_SUCCESS;
}

VAD_PAGE NTSTATUS VadPreparePolicy(WDFDEVICE child, uint32_t intent)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(intent);
    auto context = VadChild(child);
    context->PolicyStatus = STATUS_DEVICE_NOT_READY;
    WdfWorkItemEnqueue(context->PolicyWork);
    // PrepareHardware is passive and PnP-serialized. The worker never enters
    // ACX or takes Lifecycle/Admin, so waiting cannot cycle back to this call.
    WdfWorkItemFlush(context->PolicyWork);
    return context->PolicyStatus;
}
