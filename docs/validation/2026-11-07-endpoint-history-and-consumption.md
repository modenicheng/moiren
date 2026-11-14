# Endpoint history and persisted policy consumption

Recorded November 7, 2026, Asia/Shanghai, against clean source `0fdc2c6`.
This continues the full S0-S5 goal, not a replacement metadata-only goal.
The standalone remote computer is used directly. No VM, desktop input/unlock,
audio client Start, device enablement or driver change is required by these reads.

## Question and provenance

The [preceding policy/state receipt](2026-11-07-endpoint-state-and-policy.md)
finds the exact fixed Playback MMDevice DISABLED during live publication, while
both fixed and seeded Playback interfaces already stage EnableEndpointByDefault
`0x101`. The next discriminant is the retained device history and the endpoint's
own property store, not another unchanged acquisition or a new stable ID.

Host evidence: `D:\coding\moiren-evidence\20261107-endpoint-history-2109`.
Remote evidence: `D:\MoirenTest\20261107-endpoint-history-2109`.
Both readers retain the exact six-item topology-derived manifest with SHA-256
`75F5807B31FB2A7DFC1A34F9415FFCD083C05798D50E8009E2AF27F4470D05C4`.
Its repeated seeded before/after entries describe four unique raw PDOs but six
different recorded endpoint/interface identities. No name-based ownership
matching or guessed substitute endpoint is permitted.

## Typed raw PDO history

At21:26:30..37 the guarded native reader uses CM_LOCATE_DEVNODE_PHANTOM and
CM_Get_DevNode_PropertyW with bounded buffers. It preserves CONFIGRET, property
type, byte count and validated values for every requested property. The host
only compiles it and runs12 pure decoder checks, without native entrypoints.
Eight actual worker invocation/refusal cases pass on each of PowerShell5/7.

All four unique raw PDOs resolve, retain ConfigFlags0, driver3.1.1.612/oem206.inf
and parent ROOT\MEDIA\0001. FriendlyName, DevNodeStatus and ProblemCode return
CONFIGRET37/CR_NO_SUCH_VALUE; missing status/problem is not a healthy zero.
DeviceDesc remains the fixed INF text, while BusReportedDeviceDesc contains
the last requested name. The stored dates, shown here in UTC, distinguish:

| Raw PDO | FirstInstallDate | InstallDate |
| --- | --- | --- |
| Fixed Playback | 2026-10-25T06:50:45.0891452Z | 2026-11-07T10:20:48.3320000Z |
| Fixed Microphone | 2026-10-25T06:50:45.1302076Z | 2026-11-07T10:20:47.9150000Z |
| Seeded Playback | 2026-11-07T10:25:30.4294375Z | Same |
| Seeded Microphone | 2026-11-07T10:25:30.4701114Z | Same |

The fixed pair's latest arrival/removal agrees with the preceding20:52..20:53
native run. Seeded before/after entries resolve the same raw PDO, so their
identical latest dates are not separate historical phase observations.
These are device-instance dates, not MMDevice first-creation dates, policy
consumption timestamps or records of who disabled a device.

Reader:10240bytes, SHA-256
`ACD7F6CDB9A76CC62E9CAE3782B97FF434911600E422A5674B01B41E145762DC`.
Worker SHA-256:
`BEF890A0256B48B8D67DB669980BF61C37F8A03E89014D06EE571F6D18B35B12`.
Its13 terminal files independently verify lengths and SHA-256 in the18141-byte
archive, SHA-256
`A72A05F4259FF31AA933079D136CFE9F43B5B526FCA6054861FFF5CA3F2D2916`.

## PnP inventory is not MMDevice existence

At21:32 the read-only AudioEndpoint inventory has102 entries. Exact recorded-ID
mapping finds five corresponding SWD\MMDEVAPI instances; the fixed Playback ID
has no match. The five matches retain ConfigFlags0 and latest history; removed
DevNodeStatus/ProblemCode are Empty, not successful zero values. Helper failures
from an incorrect PowerShell array projection are retained separately and never
used as a device-state verdict.

The subsequent exact GetDevice call succeeds for that same fixed Playback ID.
Therefore its absent PnP inventory entry is not evidence that the MMDevice was
deleted. Neither PnP Unknown nor a missing inventory row is substituted for
IMMDevice::GetState.

## Endpoint property-store divergence

At21:38:11..18 a separate x64 COM reader calls GetDevice on each of the six
pinned opaque IDs, checks the returned ID exactly, reads GetState and opens
the property store with STGM_READ only. It never calls Activate, an audio
client, a setter or Commit. Every GetDevice/GetId/GetState/OpenPropertyStore
and requested GetValue succeeds; each PROPVARIANT is cleared and each counted
COM reference is released. Missing properties remain S_OK/VT_EMPTY.

All six retained endpoints report state4/NOTPRESENT while the test has no
present raw child. This is a later idle observation, not a contradiction of
the fixed Playback's earlier live state2/DISABLED, and not an enablement pass.

| Endpoint property | Fixed Playback | Seeded Playback, before and after |
| --- | --- | --- |
| FormFactor | UI4 10 / UnknownFormFactor | UI4 10 / UnknownFormFactor |
| Association | OUTPUT_UNDEFINED GUID | Same |
| NeverSetAsDefaultEndpoint | UI4 775 / 0x307 | Same |
| EnableEndpointByDefault | S_OK, VT_EMPTY | UI4 257 / 0x101 |
| Supports_EventDriven_Mode | UI4 1 | Same |
| DeviceDesc | Localized generic speaker text | The respective requested name |
| DeviceInterface_FriendlyName | `8- Moiren virtual audio endpoint` | `69- Moiren virtual audio endpoint` |

All Microphone endpoint stores retain their respective requested DeviceDesc,
their composed adapter labels and no EnableEndpointByDefault value, as designed
for that flow. All final FriendlyName strings still include an adapter suffix.
The endpoint store's DeviceInterface_FriendlyName is an adapter label; it must
not be conflated with the independently verified exact typed name on the KS
interface. No string is normalized or stripped before acceptance.

This proves a persisted endpoint-property difference despite the identical
currently staged Playback interface policy. It narrows investigation to
historical consumption/retention/migration, but does not establish which event
caused that difference or whether a user disabled the endpoint. In particular,
absence of a creation-default property is not proof of the current user setting.
No automatic enable, endpoint deletion, identity rotation, property-store write
or threshold relaxation follows.

Independent review first identifies a possible stdout Unicode-loss path. Before
deployment the reader is changed to all-ASCII escaped JSON and verified through
actual Start-Process redirection on PowerShell5 and7, preserving Chinese text
and a surrogate pair exactly. Seven pure layout/serialization checks and eight
worker invocation/refusal checks per shell pass; none proves COM success by
itself. The separate native receipt above supplies that evidence.

Reader:10752bytes, SHA-256
`4808005E177B39067B5A77034B3EBFBAC1B34123B6445978293BCF2B2C2B4692`.
Worker SHA-256:
`6703FF683A816D886D6B42649F1CC5CDDD2FA1858220F9240B6BBC7E430F1220`.
All13 terminal files verify remote/local/ZIP lengths and hashes; the17093-byte
archive SHA-256 is
`D0124E626BD1BE4C81C6622A9C5C6865BF7A0E1B994B30BB43C6E626AC0AC2A3`.

## Safety, publication and remaining work

Both guarded native reads verify unchanged612 loaded-service-file/INF/PnP pins,
boot, six default roles, render controls, security and protected-process identity.
Their terminal no-owner/no-present-child audits pass. They do not prove
uninterrupted external audio. All29 pinned original dirty-worktree draft/cache
files remain unchanged. No host native audio/UI/driver operation occurs.

Published `0fdc2c6` completes exact-head Runtime foundation run42374879801 with
all20 jobs successful; original watcher99946 exits0. The earlier d004809 product
slice is covered by this published head. This closes its software CI, not any
Windows-name, migration, PCM or physical gate. PR26 remains Draft/open; the
accumulated branch must not be merged wholesale.

The next source review must address a separate naming/identity design risk:
EndpointReference.h derives the KS reference from the display name, and both
Policy.cpp and CircuitDevice.cpp use it. Recorded seeded before/after endpoint
IDs differ while the raw PDO/project identity is retained. Microsoft's default
selection and migration guidance warns that changing reference strings creates
new endpoints without the previous endpoint's user settings. A stable project
ID alone is not evidence of preserving Windows endpoint settings. Do not rotate
references again merely to refresh names, or revert to a fixed reference without
also solving supported name publication and compatibility. The full S4 exact
name/default-role/held-client/repetition/race matrix remains open, as do matched
PCM/waveforms, native budgets, desktop input, sustained quality and calibrated
p95/p99 below10ms or the evidenced target-device fallback.

## Official interpretation references

- Microsoft, Default Audio Endpoint Selection Starting in Windows 10:
  https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/default-audio-endpoint-selection
- Microsoft, Operating System Upgrades:
  https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/operating-system-upgrades
- Microsoft, PKEY_AudioDevice_EnableEndpointByDefault:
  https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/pkey-audiodevice-enableendpointbydefault
- Microsoft, Device Properties:
  https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-properties
- Microsoft, IMMDevice::OpenPropertyStore:
  https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immdevice-openpropertystore
