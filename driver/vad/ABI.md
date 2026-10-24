# Control and PCM ABI v2

`include/moiren_vad.h` is authoritative; every field is fixed-width little-endian.
The control interface is `{4C7EF487-61A1-4A21-8D63-9D45115A7A5B}`. Discover its
present interface using SetupAPI, open GENERIC_READ|GENERIC_WRITE, sharing zero,
OVERLAPPED, with no inheritable security attributes. Keep one owning file object
for the session. A duplicated handle shares that object; its last handle cleanup
revokes gates, cancels PCM and requests asynchronous endpoint removal.

| IOCTL | Method | Input bytes | Second buffer / returned bytes |
| --- | --- | ---: | --- |
| `0x22E000` capabilities | BUFFERED | 8 | 64 |
| `0x22E004` create | BUFFERED | 168 | 128 |
| `0x22E008` query | BUFFERED | 32 | 128 |
| `0x22E00C` remove | BUFFERED | 32 | 128 |
| `0x22E010` gate | BUFFERED | 48 | 128 |
| `0x22E016` playback PCM | OUT_DIRECT | 80 | 80-byte header + PCM |
| `0x22E019` microphone PCM | IN_DIRECT | 80 | PCM input; completion bytes = consumed PCM bytes |

Size/version are u32 at 0/4; version is 2. Reserved fields must be zero. No native
pointers, arbitrary memory mapping or METHOD_NEITHER controls are accepted.
Capabilities are 64 bytes; snapshots 128. These exact response lengths differ.

CREATE: intent/channels at 8/12, 16 stable ID bytes at 16, local AllowDefault at
32, reserved u32 at 36, NUL-terminated UTF-16 name (64 units) at 40. Stable ID
bytes are preserved (Low then High u64, each LE). All 128-bit identities are
valid. Default eligibility comes only from local preference, never an imported
project permission. The instance ID is derived from stable identity, not a new
counter each app launch.

Endpoint requests: runtime EndpointId u64 at 8, SessionEpoch at 16 and
EndpointGeneration at 24. Gate adds canonical Open u32 at 32, reserved u32 at 36,
compared GateEpoch at 40. EndpointGeneration equals the runtime token for this
ABI. It is never a persistent ID. Stale session/generation requests fail.

Snapshot offsets: runtime ID 8; state/intent/channels/gate/policy/audio u32s at
16/20/24/28/32/36; session/endpoint/stream/gate u64s at 40/48/56/64; stable ID at
72; sample format/packet frames/async NTSTATUS/native stream state at 88/92/96/100;
dropped/silence counters at 104/112; reserved u64 at 120. States are Present=1,
Removing=2, Removed=3, Creating=4. Native stream states: Closed=0, Paused=1, Run=2.

CREATE can return Creating before PnP preparation. Query until Present with
prepared policy and ready audio before opening a gate; observe LastStatus on
asynchronous failure. Neither CREATE nor accepted REMOVE proves endpoint
visibility/removal. Only Removed reports final child destruction. Retained
Removed tombstones may be reused by CREATE; old tokens then return not found.
Native clients can hold a stream while the Engine gate is closed.

Capabilities preserve the first nine u32 fields of the provisional contract;
RequestSlots=4 at 36, SessionEpoch u64 at 40, MaxPacketFrames=4800 at 48, reserved
u32s at 52/56/60. Flags: raw PDO=1, policy=2, native audio=4, streams=8, PCM=16,
CREATE deferral=32.
RequestSlots is the maximum pending-request capacity, not a requirement to keep
four requests submitted or a guarantee of four periods of PCM freshness.
Formats: PCM16=1, Float32=2; target rate 48000, mono/stereo mask 6, intent mask 6,
max endpoints 64, max native streams 1. These advertise implemented interfaces,
not target qualification or a performance promise.

PCM buffered header: runtime ID/session/endpoint/stream/gate u64s at
8/16/24/32/40; sequence/QPC u64s at 48/56; format/frames/channels/reserved u32s at
64/68/72/76. Size=80 excludes payload. Frames must equal current native packet
frames; format and channels must match the snapshot. PCM16 is signed LE; Float32
is IEEE LE. Mono has one channel, stereo two interleaved channels.

Only the owning file object can submit PCM. Up to four pending requests per
endpoint bound pinned accepted payload to 153,600 bytes; the 64-endpoint cap
bounds the total. Empty capture is zero; full/late render is dropped. No extra
kernel PCM queue is added. WRITE sequence must increase within the gate epoch,
QPC must be nonzero/current, and payload older than one native period is dropped.
This age check applies when the native callback consumes WRITE, not just at
submission. Clients can preallocate four overlapped slots while pacing
microphone WRITE from completion events with only one request pending. Filling
all four with submission-time QPC values does not reserve future packet times
and does not extend the one-period age limit.

When `CREATE deferral=32` is negotiated, CREATE may return `STATUS_SUCCESS` and
the ordinary 128-byte response size with tag 5 at the snapshot State offset.
This CREATE-only response echoes the current session, requested stable ID,
intent and channel count; every runtime-key, generation, readiness and reserved
field is zero. It acknowledges that no child or runtime identity was created
because the same stable ID is still retiring from an earlier session. It does
not reserve quota or transfer ownership of the old PDO. The caller may retry
CREATE after this explicit response; timeout, malformed response and other IO
errors remain uncertain and never authorize retry. Older drivers do not
advertise this capability and must fail closed rather than interpret tag 5 as
an endpoint snapshot.
READ completion stamps the native packet sequence/QPC. WRITE never modifies its
IN_DIRECT payload; READ returns the header plus data. Map MDLs only on submission.

Closing a gate revokes queued data without destroying the native stream. Native
stream reopen, resume and scheduling discontinuity invalidate old epochs; owners
refresh snapshots and rebuild prepared transport state. Zero clients, zero PCM,
queue pressure and rejected stale work do not authorize physical-owner recovery.
