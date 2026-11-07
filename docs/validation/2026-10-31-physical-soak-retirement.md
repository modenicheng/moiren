# Physical DAWN retirement control, 2026-10-31

## Retained failure and scoped repair

`dawn-256-1` runs the exact Input -> unity Bus -> DAWN Output graph with
probe `8b574cc`, unchanged diagnostic driver `883a5be`/610, Engine64,
DAWN Exclusive256, input target1152/capacity4800/jitter96, declared packet/
gap480 and recovery4. The VAD source requests its queried minimum and
actually receives144 frames; DAWN actually receives256. The source retains
the same30-second indexed-v2 schedule with29 unique markers, followed by a
planned tail inside the35-second diagnostic soak. No PCM is collected.

The native probe exits1 with `SET_GATE GetOverlappedResultEx(VAD)` error
`0x8007051A` and retained `VAD final host did not join cleanly`. Controller
exit0 means supervision/terminal cleanup completes, not probe success. Source
Stop/join completes; final physical owner retains1681920 written frames,
6570 wakes and0 wait timeouts. Startup prime32 remains visible. Live provider
drop/silence/reset and render xrun/recovery/short-read counters are0, but these
do not waive the teardown error or prove waveform continuity. Report duration0
is an error-path assignment boundary, not evidence of zero native runtime.

The controller runs UTC06:40:10.5792558 through06:40:47.6806241. The source
reports35.004626 seconds. No timeout, forced termination, remaining owned
process/child, boot change, volume/mute/default-role write or driver change
occurs. The original failure remains archived with17 payloads plus manifest:
51943 bytes, SHA256
`435C019D52D8151B5934755ECFE6D98E49B03321ED45140A3A4059985308BB2F`.
Root verifies exact members, lengths/hashes and frozen provenance and compares
all17 extracted payloads with separately downloaded originals. Independent
acquired-evidence audit passes49 checks; integrity is not audio acceptance.

Probe-only `a20dfe2` integrates independently reviewed `a0630e0`. Physical
non-collection VAD retirement now observes while live, closes the original
Playback gate with its strict identity pin, invalidates admission, broadcasts
source Stop, joins the source, shuts down the host and retires the endpoint.
Each fallible/panicking stage still attempts its peers and retains original
errors. There is no stale retry, replacement-identity fallback or weakening of
provider/driver guards. Existing joins remain potentially blocking. MIC
collection and Process schedulers, production buffers and defaults are unchanged.

A real old-order regression exits101 before the repair. Author598 tests,
strict Clippy and formatting pass; independent frozen-source review passes16
checks with no blocker. Root integration passes620 tests, strict example/test
Clippy, formatting and a release build with2 jobs. The new binary is7240704
bytes, SHA256
`B0CB2EA508D99239379F60F2C576142219D957569A3581A8417084BAF141A684`.
It is staged side by side, leaving prior binaries, pins and failure evidence
unchanged; the following acquisition tests that scoped repair.

## Same-configuration native replay

The new side-by-side `dawn-256-1` runs UTC07:01:22.2866313 through
07:02:00.6413216 with the same source/driver/graph/budgets and actual256/144
physical/source frames. Its fresh build/native pins explicitly describe the
physical DAWN fixture; no earlier MIC pins are relabelled. Runner SHA256 is
`FBF7EA96A32921052033906F80C1D75B304CB17F178FFFB88DF9B6BD0DBEC345`.
Host runner/archive fixtures pass60 checks, independent runner fixtures23 and
PS5.1 parsing passes before execution. Remote binary/input/fixture bytes and
hashes match their frozen host copies.

Native error and final backend failure are null. Source Stop/release/join and
host Closed/Dormant complete without cleanup error; observed duration is
35.004354 seconds. Probe exit1 still means unqualified diagnostic output, not
another SET_GATE failure; controller exits0 with no timeout/forced termination.
All1440000 scheduled source frames are acknowledged. Joined DAWN retains
1682432 written frames,6572 event wakes,0 timeouts/drops/silent frames. Input
boundary xrun/short-read/recovery-prime counters stay0; initial-prime muted32
remains retained. Live provider reset/recovery/drop/silence are0; terminal
Removed/Closed retains separate reset1/recovery1 and silence0. None proves
source identity or uninterrupted physical output in the absence of PCM.

The new17-payload archive is51554 bytes, SHA256
`CA8264DF741AB3AB32C430104D340C4D9F7E98DDC99EA49CA770ECC978E77842`.
Root verifies CRC, exact members, every length/hash, frozen local provenance
and extracted bytes. Previous failed evidence remains untouched. Owned audio
processes/children are absent; boot, driver610 and preexisting render/default
controls are unchanged. Independent acquired-evidence audit passes59 checks.
This single replay supports the corrected cleanup path, not sustained stability
or complete concurrency correctness; there are no per-stage teardown QPC
windows in this archive to independently prove exact close/Stop chronology.

## Exact128 refusal

A second invocation changes only the requested DAWN period to128. The same
binary refuses before physical Running: WASAPI reports a144-frame minimum
at48000 Hz, while128 frames is smaller. The error suggests the next application
preset256; that suggestion is not evidence that the hardware cannot accept144.
The probe currently permits only the power-of-two preset list, a separate
software restriction that must not be mistaken for a physical minimum.

Controller UTC07:02:38.6275401 through07:03:00.1886413, controller exit0,
probe exit1, no timeout/forced termination. Source never starts and its worker
joins; native Stop success is unknown/not attempted, not fabricated. Final
diagnostics retain missing admitted generation. Root PnP, driver, boot and
render/default controls remain unchanged; no owned audio processes/children
remain. Root verifies the17-payload archive,23657 bytes, SHA256
`0D04EDC0D4155B5F72EB39D485DB60D659ED7E5FA560A5CC16AEC1BEA9DF10F0`.
Independent acquired-evidence audit passes50 checks. A fresh post-controls
read-only audit confirms no audio/build owner, unchanged boot, Running610,
unchanged default/render controls and239005523968 free bytes on D:. No cache
clean or evidence deletion is needed.
This is a specific current-mode/rate native refusal, not evidence that physical
sub10ms is impossible or that256 frames is the hardware's exact minimum.

## Current admission model, not physical lower bound

At48000 Hz and full-capacity Exclusive physical output, define actual native
period/capacity P and Engine block B. Whole-wake demand D=P+64+2B; drift
transfer X=ceil(D*1000/998); input low-water L=X+32. With declared gap480,
jitter96 and FIR32, reserve=max(480+96,32)=576 and modeled target minimum
M=L+576. Current source anchors are `engine_backend.rs` whole-wake preparation,
`virtual_owner/budget.rs` `prepare`/`prepare_budget`, and
`latency/transport.rs` `prepare_transport_input`.

| Engine B | Candidate P | D | X | L | Minimum target M |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 32 | 128 | 256 | 257 | 289 | 865 |
| 64 | 128 | 320 | 321 | 353 | 929 |
| 32 | 256 | 384 | 385 | 417 | 993 |
| 64 | 256 | 448 | 449 | 481 | 1057 |

The128 rows remain hypothetical and are refused by the current native control.
Targets are integer frames, not necessarily multiples of B, native144 or a
power of two. Capacity also requires target < capacity-480 and
target+480+L <= capacity. Actual144-frame VAD packets do not authorize lowering
the declared480-frame delivery gap. Physical whole-wake demand is not truncated
by the virtual recovery cap. No queue target, period or modeled minimum is a
calibrated end-to-end result or physical theoretical bound.

## Evidence boundary

Early gate close may expose trailing starvation or silence; any such counters
remain failures/observations rather than a new quality exemption. The failed
control and deterministic scheduler test do not establish the exact native
interleaving or prove complete kernel concurrency correctness.

No calibrated physical delay, source-waveform continuity or current-device
theoretical minimum follows from native periods, queue admission or counters.
The conditional user fallback applies only after demonstrated sub10ms
impossibility under declared current-device conditions. Neither that
impossibility nor a physical lower bound is established. S1-S3 remain open.
