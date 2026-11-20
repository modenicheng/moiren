# Remote OBS / QQMusic application capture checkpoint

Date: 2026-11-01. Worktree source: `0b69e11` (no production-code changes).
The user requires application/native tests on the existing remote Windows
target, not on the development host. Lack of simultaneous physical recording
hardware defers calibrated physical latency measurement; it does not block
application capture/coexistence work.

**A fresh remote 15-second selected-process capture completes while actual OBS
records the same QQMusic application. Both retained recordings contain finite,
nonzero audio. This is a bounded coexistence smoke, not complete application,
whole-waveform, VAD or physical-latency acceptance.** Startup silence remains
explicitly measured rather than hidden by aggregate nonzero counts.

## Scope correction and environment

Before the remote-only correction, a development-host five-second passive
QQMusic capture yielded zero packets, and a task-owned isolated portable OBS
instance was started. These are not remote acceptance evidence. On the user's
correction, the exact task-owned local OBS PID / creation time / executable
were checked and that instance was stopped. Original local OBS configuration
hashes match; the pre-existing local QQMusic process is preserved. No further
local application, capture or audio test is launched.

The actual tests below run on `LAPTOP-NP49RG6H`, console session1, through
Interactive / Limited scheduled tasks. Installed OBS31.0.3 is copied to a
new task-owned portable directory; original scenes/profiles are not edited.
Existing QQMusic21.45 plays a generated120-second stereo16-bit48kHz WAV with
changing envelope and distinct channel frequencies. No login, online music
download, driver install, default-device, master-volume, mute, privacy or
power-policy write is performed. QQMusic retains its configured output:
the captured session is Active on `DAWN PRO2`, volume1, not muted. This
application playback can use its existing output; it is not a physical
reference acquisition or calibrated output measurement.

OBS has exactly one `wasapi_process_output_capture` input, matching the test
QQMusic title/class/executable; monitoring remains `OBS_MONITORING_TYPE_NONE`.
No desktop/microphone capture source, streaming or virtual camera is enabled.
The task-local authenticated WebSocket uses IPv4 port4457. The source process
PID and exact decimal FILETIME are pinned independently for the Moiren probe.

Probe: the already-built current-code7446528-byte `e2e_latency_probe.exe`,
SHA256 `9462CF10B053B89B11160C7E7EFB403AF8AA9F9547D3433242D00282BD9EADDE`,
tested source `497957e` including SRC fix `353f0db`.
`record-process` records native PCM without constructing a graph, renderer,
MIC export or UI. Its exit1 intentionally means **unqualified**, not an
automatic operational failure. `w00_probe.exe` is used only for passive
endpoint/session/settings snapshots; SHA256
`84344CF8655CE5BC53CF6B72F730B70801D6E80B03D6D584804952FD5EB5165F`.

## Fresh remote result

The final single-task round starts OBS, waits for its authenticated API, then
starts fresh QQMusic and performs recording/capture without SSH scheduling
gaps between playback and collection. The controller exits0; probe exit1
retains `qualified=false` by design.

| Surface | Observed result |
| --- | --- |
| Process capture | Native status completed; recording/artifact complete; Stop succeeds; no failure strings |
| Native PCM | 1497 packets,718560 frames,5748480 bytes,48kHz stereo Float32 |
| Stored artifact | Zero recorded drop/overflow/discontinuity/timestamp-error events; position-gap detection disabled |
| PCM readback | 1437120 finite samples,1262177 nonzero; peak0.01224065, RMS0.00506864 |
| Startup PCM | First86465 stereo frames are exactly zero:1801.35ms from this recording's origin |
| Later PCM | Longest subsequent all-channel exact-zero run is one frame; this deterministic signal has regular zero crossings |
| Delivery | Maximum observed packet480 frames; maximum host packet gap32.5955ms; no declared cadence bound was supplied |
| OBS | Actual recording active during capture, Start/Stop acknowledged, final `outputActive=false`; MKV closes and decodes successfully |
| OBS decoded PCM | 724992 frames,1198208 nonzero samples,all finite; peak0.01234257, RMS0.00489275 |
| OBS startup | First125888 stereo frames are exactly zero:2622.67ms from its own recording origin |
| OBS performance | Snapshot reports0 output/render skipped video frames; this is not an audio continuity counter |

Both recordings' middle channel0 spectrum includes the generated431/839Hz
components. The original PCM and decoded OBS audio remain retained. The
different recording origins, startup/activation, OBS buffering and lossy AAC
encoding prevent interpreting their offsets or frame counts as route latency.
No full value/identity alignment, duplicate/missing-frame proof or independent
pre-route clock reference is established. All-zero startup PCM can arrive
without WASAPI's SILENT flag: aggregate flag0 is not proof of nonzero audio.

The unconsumed compatibility bridge intentionally drops the captured stream;
its counters are separate from the native artifact tap and do not mean the
stored PCM lost those frames. Similarly, delivery `gap_bound_violations=0`
without a declared bound does not certify a cadence/latency gate. The observed
32.5955ms gap remains visible, not relabeled as good servicing or lost audio.

## Failed setup attempts retained

The initial remote five-second capture completed499 packets/239520 frames,
but PCM was all zero. Long scheduled-task startup delay and later inspection
made this run unsuitable for non-silent coexistence acceptance. The cause is
not established by this attempt alone.

Earlier helper failures remain archived: PowerShell5.1's parsed numeric
FILETIME became exponent text; the corrected path preserves exact decimal
text. OBS lists a QQMusic shadow window as well as its titled main window;
helpers that demanded exactly one executable/class match refused rather than
selecting arbitrarily. A later direct capture correctly rejected the retired
first QQMusic PID with OpenProcess `0x8007001F`. Its approximately one-second
OBS file is retained as an incomplete attempt, not combined with the final
15-second result. The second round resolves scheduling by keeping readiness,
fresh playback and dual acquisition in one bounded interactive task; it does
not weaken a quality threshold or bypass process identity checks.

## Retirement, settings and archive

The native capture and OBS recording stop normally. The task-owned QQMusic and
OBS **applications are then forcibly terminated** by their bounded controller;
application graceful-exit acceptance is not established. Final inspection
finds no OBS/QQMusic/probe processes or WebSocket listener. The four exact
terminal task definitions are compared with their exported XML and removed
only after archive download and independent payload verification. Unrelated
tasks and all acquired evidence remain.

All726 original remote OBS files independently match their pre-test hashes.
The first PowerShell postflight checker incorrectly counted727 differences
because its top-level JSON array remained nested inside `@()`; Python's
element-wise keyed comparison and fresh current-file hashes verify no actual
change. The failed receipt and corrected receipt are both preserved, without
overwriting user configuration to make the check pass. Six default roles and
12 render master volume/mute states match before/after, including the final
round's own adjacent snapshots. Driver611 / `oem205.inf`, SYS SHA256
`4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`,
and boot time are unchanged; no raw VAD children remain.

Evidence root:
`D:\coding\moiren-evidence\20261101-app-route-followup-104222`.
Target root:
`D:\MoirenTest\20261101-app-route-followup-104222`.
Archive:81 payloads plus manifest,25815579 bytes, SHA256
`90D659F58D6F65509FDEBD1F71224E3ECF2E72400400E795773BAD581470E98A`.
Manifest SHA256:
`033E79D593C371E4EA445EF6E24FB1D90C443F3BB7103595531E9E03BE4160AB`.
The archive and every extracted payload are independently length/hash checked.
Local helper/SSH failures (including command-line length, text encoding and
the postflight array checker) remain separate receipts; they are not new
native audio failures. No push occurs and unrelated `gate.log` is untouched.

## Remaining application work and native failure

This smoke covers **QQMusic + actual OBS + production Process Loopback
capture**, not the Moiren desktop graph or VAD MIC route. Next application
coverage must separately exercise graph start/stop, OBS consuming the exported
MIC, process lifecycle/isolation, operational recovery and sustained playback.
Missing physical recording hardware is not a prerequisite for those tests.
Do not auto-route QQMusic, replace drivers or change device defaults to avoid
a refused configuration.

The [current VAD retirement failure](2026-11-01-current-vad-native-regression.md)
is still open. Its retained log shows explicit Playback SET_GATE(close)
publishing `admitted=true`, the worker observing gate epoch1->2, then slot0
`0x8007051A`. Source inspection finds ordinary close does not revoke session
admission first; playback refill only checks reset epoch before another READ.
Driver gate-close publication precedes buffer clearing/purge, and both request
reservation and transfer reject stale stamps. This narrows a **gate-close /
pending-PCM interleaving hypothesis**, not a proven kernel sequence or final
fix. A deterministic interleaving regression is needed before a root repair;
retain real terminal mismatch errors rather than treating them as cancellation.
No production audio/error policy changes in this checkpoint. Calibrated
physical p95, sub-10ms qualification and S1-S3 remain unmeasured/incomplete.
