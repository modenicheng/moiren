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
