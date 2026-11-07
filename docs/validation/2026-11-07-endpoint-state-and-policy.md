# Exact endpoint policy and Windows state visibility

Recorded November7,2026, Asia/Shanghai. This continues the full S0-S5 goal.
Neither metadata reads nor software checks qualify exact Windows names,
migration, PCM, sustained audio or calibrated physical latency.

## Evidence-led next step

The preceding [failure snapshot](2026-11-07-rename-enumeration-evidence.md)
identifies the exact fixed-fixture Playback MMDevice as DISABLED while its
Microphone is ACTIVE. Playback already reports OUTPUT_UNDEFINED. This warrants
comparing persisted interface policy with the earlier seeded fixture that
initialized, not enabling the endpoint, rotating its stable ID or repeating the
same failed PCM acquisition.

Host evidence namespace: `D:\coding\moiren-evidence\20261107-endpoint-policy-2008`.
Remote namespace: `D:\MoirenTest\20261107-endpoint-policy-2008`. The standalone
remote computer is used directly; no VM or desktop input/unlock is requested.
The host compiles metadata helpers but never executes their native entrypoints.

## Guarded read-only policy observations

Six immutable inputs come from the previous exact topology receipts: fixed
Playback/Microphone plus before/after interfaces for the same seeded held-rename
fixture. They describe four unique raw PDO instance IDs and six exact AUDIO
links. The reader derives flow/REALTIME links from each exact recorded link,
confirms each against CM_Get_Device_Interface_ListW scoped to that instance,
then uses CM_Open_Device_Interface_KeyW with KEY_READ and OpenExisting only.
It opens EP\0 read-only and reads selected values, never enumerating or editing
unrelated registry state. It cannot initialize or Start an audio client.

The first helper has an incorrect flow-category GUID suffix, a3cc instead of
the source's a3b9. Its AUDIO and REALTIME reads succeed, but its empty flow list
is a helper error, not missing driver registration. A corrected V2 binary is
compiled, separately pinned and deployed to fresh paths. At20:15:35..43 its18 exact
Audio/Render-or-Capture/Realtime registrations all resolve and open successfully:

| Owned policy | Playback, fixed and seeded before/after | Microphone, fixed and seeded before/after |
| --- | --- | --- |
| Association | OUTPUT_UNDEFINED GUID | MICROPHONE GUID |
| NeverSetAsDefaultEndpoint | DWORD0x307 | DWORD0x307 |
| EnableEndpointByDefault | DWORD0x101 | Value absent, as designed |
| Interface FriendlyName and staged EP0 FriendlyName | Exact requested name for each recorded interface | Exact requested name for each recorded interface |

V2 helper is8704bytes, SHA-256
`BAF8FFB6710016E000C70BC6BE6D6302E4B51EAC53583CB2546D9D67540FC6CD`.
Its worker SHA-256 is
`6306EFC7FE47FAB68EBB0909E6B84F42C1A5B68837F2835194C6810F0CCED813`.
The worker's eight real invocation/refusal cases pass on each of PS5/7.

This proves persisted policy at observation time, after clients/children have
been cleaned up. It does not establish when EndpointBuilder consumed it, who
disabled the fixed endpoint, or whether an earlier OS state was migrated.
Identical staged0x101 on the fixed and seeded fixtures rules out a simple
currently missing enablement value; it does not prove equal Windows behavior.

## Raw PDO text is not the final endpoint name

A first PowerShell reader fails before the property loop because strict PS5
array projection cannot access Instance. Its corrected version reads DeviceDesc,
NAME, bus text and parent for the four exact instances, but accessing the Data
member of the FriendlyName result fails. That is a wrapper failure, not proof
that the property is absent; both receipts are retained.

At20:20:21..28 a separate native typed reader resolves those exact devnodes with
CM_LOCATE_DEVNODE_PHANTOM and reads fixed8192-byte buffers. It reports CONFIGRET,
DEVPROPTYPE, byte count and strictly validated UTF-16 instead of substituting a
description for a failed property. All four unique retained instances have:

- FriendlyName: CONFIGRET37/0x25, CR_NO_SUCH_VALUE, type0, bytes0.
- DeviceDesc: type18 string, `Moiren virtual audio endpoint`.
- BusReportedDeviceDesc: type18 string, the last requested CREATE/rename text.
- Parent: type18 string, exactly `ROOT\MEDIA\0001`.

The repeated seeded before/after rows intentionally resolve the same raw
instance and therefore show its latest retained bus text, not historical text
at each earlier phase. These are phantom-allowed observations after removal,
not a live-device publication-timing result. Source explains the distinction:
the INF supplies the fixed Models description; WdfPdoInitAddDeviceText supplies
requested bus text; the bridge pin callback supplies endpoint description.
This identifies a plausible adapter-label source without proving a supported
way to remove Windows' composed suffix. No speculative setter, INF rewrite,
property-store write or raw-name gate relaxation follows.

Native helper is7168bytes, SHA-256
`21164E10C3551B9215F278DED3567F30AECB203716B7A51EA2A12A8087CC078F`.
Worker SHA-256 is
`B765AEB8EEDA6D30271C402C56B0517A1BC5DC30991819A7C227B0948B239676`;
eight invocation/refusal checks pass on each of PS5/7.

Every successful wrapper verifies the unchanged612 service-file/INF/PnP pins,
boot, six default roles, render controls, security and protected-process identity
before/after; terminal no-test-owner/no-present-child checks pass. No native
audio client, driver install/restart, desktop input, reboot, enablement, default,
volume or security write occurs. Metadata can affect observation timing but
does not prove uninterrupted external audio.

All61 terminal files, including failed/helper-correction receipts, independently
verify remote/local/ZIP lengths and SHA-256. Archive `policy-terminal.zip` is
79868bytes, SHA-256
`C21DA631AF6C4E94050D5CBC57188BA15AD02AD4E450B132CF6CCEEDF0288D66`.
No raw audio exists or is transferred.

## Product state visibility

The existing control-worker name observation now carries optional raw
IMMDevice::GetState bits and an independent state-read error. It reads only the
already selected exact root/stable-ID/flow MMDevice, after raw name readback.
A failed state read preserves the successful raw name and records unknown
state plus its actual HRESULT. A failed name/identity read never retains a
previously ACTIVE state. Existing one-second cache, observation QPC and native
identity/lifecycle filters remain in force. This is detached last-observed
metadata, never a client-admission or Engine-readiness authority.

The real Exports page and provider diagnostics display UNKNOWN, ACTIVE
explicitly labelled not proof of flow, DISABLED, NOTPRESENT or UNPLUGGED.
Unexpected/combined bits retain their hexadecimal raw value; no bit mask is
silently mapped to ACTIVE. Read failures remain explicit and do not hide the
raw Windows-name mismatch. Service and probe checkpoint serialization retain
both optional fields. No client is enabled, no default changes, and no native
streaming or name acceptance condition changes.

Behavioral RED receipts retain three backend observer assertion failures
against the old propagation and one real app presentation failure for its
missing Windows-state row. Initial GREEN checks cover raw bits, independent
errors, cache refresh, wrong/stale identity and lifecycle invalidation, missing
state and real headless row visibility at1440x1600/1024x720 with no dispatched
audio request. The full first workspace run exposes two ordered acceptance
oracle failures because the new row shifts existing name/lifecycle rows; the
literal expectations are extended without removing old assertions, and both
targeted tests pass. The first example suite's old complete-JSON expectation
likewise fails until the two explicit null fields are included. These are
retained test failures, not native-audio failures or evidence of qualification.

Final verification pins all17 changed source/test/localization files by length
and SHA-256 in `state-v2-manifest.json`, based on6401849. Windows completes
`cargo test --locked --workspace --features
moiren-app/gpui-test,moiren-app/vad-integration-probe` with2636 passed,
10 ignored and zero failed across73 harnesses. The opt-in integration example
passes155 tests; workspace/all-target strict Clippy, rustfmt and diff checks
all exit0. Linux checks use an archive of that exact base plus the pinned17
files: the example passes133, backend observer17, app library618 with2 ignored,
and headless acceptance7; example and workspace/all-target strict Clippy exit0.
The first Linux offline app attempt fails before tests because atomic0.5.3 is
not cached; the retained V2 receipts record the successful dependency fetch
and complete checks. No host native audio/UI/driver entrypoint is executed.

These checks support the software slice, not native publication, waveform,
exact-name or physical acceptance. The separate native observation below
retains its own failed acceptance verdict.

## New-source native observation, not an audio pass

The reviewed slice is committed clean asd004809. Its optimized integration
example builds on the host without execution, then is copied immutably:
7915008bytes, SHA-256
`693B54879B8089DEF7D312188EEB25F65B63E5B6506DFBAA368CAF6F0C9A5C58`.
Source tree is81d1cf05c53027d0e4480e140b431408f39da1e9. The finite worker
changes only the previous PCM worker's paths, mode/binary pins and recorded
source provenance. Eight actual invocation/refusal cases pass on each PS5/7.
Worker SHA-256 is
`4CF73E75C0B6111C05E9928C9AC4752F54856FCAD9936EAB4858A4D131042923`.
Fresh remote paths and free space are checked before upload; strict known-host
SSH invokes exactly `--exercise --output-dir=... --pcm --timeout-seconds=90`.
No rename seed, round trip, enablement or silent fallback is added.

At20:52:50..20:53:14 on the standalone remote target, the unchanged612 driver
is Running/PnP OK/problem0 with the same SYS/INF pins. The new product fields
observe both exact endpoint identities as state4/NOTPRESENT during initial
Apply. Their later four-second no-probe-client hold observes Microphone
state1/ACTIVE and Playback state2/DISABLED on those same MMDevice IDs and native
identities. All recorded name/state reads have no error. The independent
failure-only ALL-state collection completes97 Playback/53 Microphone rows,
without truncation/error, and its one exact match per flow agrees with the
product's later state/name/ID. Product propagation and observed state change
are demonstrated; the cause and entire state history are not established.

The run still fails `owned MMDevices did not arrive; topology errors: []`
before any probe Start, with zero acquired signal frames and PCM unverified.
Idle Engine timeline advances699 times over4003ms, with zero epoch changes;
that is not proof of PCM flow or no native activity. Raw names still retain
the adapter suffix, and Playback's raw name remains the generic speaker text
at this observation. No exact-name condition is relaxed.

Report status is failed, native exit1, cleanup_error null, service joined and
control session released. The wrapper's only audit error is the expected
failed report-acceptance condition; boot/driver, final no-owner/no-present-child,
six default roles, render controls, security and protected-process identity
audits pass. This does not prove uninterrupted external audio. The17 terminal
files independently verify remote/local/archive lengths and SHA-256;
`native-state-pcm.zip` is223708bytes, SHA-256
`C84829A174396D9B905053314DC58EB9D71343B33923563FB37E4C0B45379A3A`.
No raw audio is transferred. Do not repeat the unchanged acquisition: state
consumption/migration and Windows-name publication remain separate unresolved
questions, as do the complete S1/S4/S5 native/physical acceptance gates.

## Publication boundary

Reviewed diagnostics through6401849 are pushed at20:18 and remote hash/PR26
head are independently verified. PR26 stays Draft/open and must not be merged
wholesale. Matching CI42371855897 completes all20 jobs successfully; original
watcher54337 exits0. This result covers6401849, not the laterd004809 source
slice, which requires its own exact-head CI after the next publication.

## Official interpretation references

- Microsoft, PKEY_AudioDevice_EnableEndpointByDefault:
  https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/pkey-audiodevice-enableendpointbydefault
- Microsoft, DEVPKEY_Device_DeviceDesc:
  https://learn.microsoft.com/en-us/windows-hardware/drivers/install/devpkey-device-devicedesc
- Microsoft, Device Properties:
  https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-properties
- Microsoft, CM_Get_DevNode_PropertyW:
  https://learn.microsoft.com/en-us/windows/win32/api/cfgmgr32/nf-cfgmgr32-cm_get_devnode_propertyw
