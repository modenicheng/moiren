# Mutable VAD names and immutable Windows publication identity

Date: 2026-11-08, Asia/Shanghai. This is a source/contract audit, not a native
rename result. The full [S4 requirement](../vad-low-latency-goals-2026-10-28.md)
still requires both accepted Windows names and preserved identity/settings.

## Confirmed implementation conflict

`driver/vad/src/EndpointReference.h::EndpointReference` hashes the requested
display name. `Policy.cpp` uses the result for Audio/flow/Realtime interface
registration; `audio/CircuitDevice.cpp` passes it to the circuit name assigned
in `audio/Circuit.cpp`. Keeping the PDO instance/component URI alone does not
keep this separate identity input unchanged.

The production-path interface test currently requires different reference
strings after a name edit. The managed service fake tests stable project ID
and graph binding, not retained Windows endpoint ID or settings. These checks
do not establish the S4 identity-preservation requirement.

Microsoft's default endpoint-selection documentation identifies reference
strings and connector pin IDs as endpoint-identity inputs and requires their
stability for the driver installation. Changing them risks creating endpoints
without their previous settings. This supports a compatibility finding, not
a diagnosis of why the fixed Playback fixture was previously disabled.

Primary contract:
`https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/default-audio-endpoint-selection`

The Core Audio device-property contract says clients should read rather than
set basic endpoint properties. The friendly-name guidance describes user edits
through Mmsys.cpl while distinguishing the persistent endpoint description from
KS names. These sources do not establish a supported programmatic publisher
for this driver's retained exact endpoint. This remains an open implementation
question, not proof that the requested feature is impossible.

`https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-properties`

`https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/friendly-names-for-audio-endpoint-devices`

## Required implementation boundary

Separate immutable Windows publication identity from mutable requested display
text. Preserve the existing reference when adopting an installed endpoint;
flipping all old endpoints to a new fixed reference would itself change an
identity input. A migration must account for existing per-endpoint state,
siblings and removal/recreation rather than manufacturing a fresh endpoint.

Policy registration and ACX circuit naming must use the same immutable value.
Production-path tests must exercise a changed requested name while requiring
the reference to remain unchanged. Requested/provider/observed Windows names,
generation checks, retained old audio and explicit pending/failure state remain
separate. The final Windows name still needs a supported publication/update
mechanism and native readback; stable reference alone does not implement it.

Interface FriendlyName staging already says it is not a final MMDevice rename.
The retained native observations show this distinction matters. No suffix
stripping, endpoint-store reset, identity rotation or automatic rename cycle
is introduced by this audit. Refusing unsupported edits can protect an active
resource, but it is not the requested final rename feature or S4 completion.

Driver/native follow-up still awaits the user's reboot and fresh613 provenance.
Source/design work on publication and migration can continue independently.

## Property-provider and adoption discriminator

The installed Windows SDK10.0.28000.0 defines the final property-store key
`PKEY_Device_FriendlyName` in `um/FunctionDiscoveryKeys_devpkey.h:62` and the
devnode key `DEVPKEY_Device_FriendlyName` in `shared/devpkey.h:54` as the same
GUID/PID: `{A45C254E-DF1C-4EFD-8020-67D146A850E0},14`. This key equivalence
does not establish shared storage or propagation between a raw PDO, the owned
SWD/MMDEVAPI devnode and the MMDevice property-store provider.

Microsoft documents devnode FriendlyName as writable by installation
applications/installers through SetupDiSetDeviceProperty. That is a possible
setter boundary, not a documented final-MMDevice update contract or evidence
that an ordinary application should set Core Audio's basic endpoint properties.
The next supported-publication question is whether an exact owned endpoint
devnode update is supported and propagates to final raw MMDevice readback with
identity/settings retained. No such write is performed by this investigation.

`https://learn.microsoft.com/en-us/windows-hardware/drivers/install/devpkey-device-friendlyname`

Interface FriendlyName is instead `{026E516E-B814-414B-83CD-856D6FEF4822},2`.
The SDK also defines `DEVPKEY_DeviceInterface_ReferenceString` with PID5 under
that interface GUID. For installed-reference discovery, IoGetDeviceInterfaces
can narrow enumeration to the exact owned PDO and include nonactive interfaces;
typed property reads can then retrieve reference strings without parsing opaque
symbolic links. The DDI requires a system thread at PASSIVE_LEVEL and allocation
cleanup. Existing policy system-worker placement supplies the correct kind of
context, not a completed adoption implementation.

`https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-iogetdeviceinterfaces`

Adoption must establish one historical reference consistently across
Audio/flow/Realtime and the connector pin ID. Multiple retained references
cannot be resolved by enumeration order, newest timestamp or display text.
Preserve the installed identity instead of globally switching constants or
deleting endpoint history. This discovery path narrows implementation choices;
neither it nor a refusal-only fallback completes the requested naming feature.

## Supported PDO setter and concrete source boundary

A further primary-source check finds an explicit driver runtime setter: the
Microsoft co-installer-removal guidance permits DEVPKEY_Device_FriendlyName
during Start IRP/PrepareHardware via IoSetDevicePropertyData or WdfAssignProperty.
This is stronger than the earlier installation-application setter hypothesis,
but its documented object is a device/PDO. It does not establish propagation
into an existing SWD/MMDEVAPI devnode or the final MMDevice property store.
No property write or new kernel package follows this source check.

`https://learn.microsoft.com/en-us/windows-hardware/drivers/develop/removing-coinstallers#setting-device-friendly-names`

`https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-iosetdevicepropertydata`

Current source review identifies the minimal ordering boundary before any
publication: PreparePolicy's system worker resolves/adopts and durably records
the immutable reference, before StageInterface and subsequent BuildCircuit/
AddCircuit. Audio initialization currently computes Reference earlier than the
policy worker; remove that independent mutable-name authority when wiring the
shared adopted value. VAD_AUDIO_DEVICE.Reference can hold the runtime value,
but Slot Reserve clears memory and neither it nor PolicyInterfaces is durable
publication provenance. There is no existing persisted reference slot to reuse.

The persistent record needs schema/root-installation scope/StableId/intent and
exact adopted reference/source version; commit/readback must precede interface
registration. An absent record is not proof of a never-published endpoint.
Historical adoption needs complete typed Audio/flow/Realtime reference data
plus exact MMDevice/connector association where multiple candidates exist.
Preserve bridge pin1 and host pin0; EP\0 is a property-subkey index, not bridge
pin1. Do not choose by time, order, requested text, or a raw symbolic-link parse.

WdfDeviceOpenRegistryKey is a documented PASSIVE_LEVEL device-key access path;
Microsoft also requires validating persistent data as untrusted. Do not open
hardcoded Enum/DeviceClasses/Class registry paths directly or treat a project's
portable settings/default-role authorization as this machine-local identity
record. These constraints define implementation work, not a completed adoption
algorithm or proof that supported final-MMDevice publication exists.

`https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdfdevice/nf-wdfdevice-wdfdeviceopenregistrykey`

`https://learn.microsoft.com/en-us/windows-hardware/drivers/install/registry-trees-and-keys`
