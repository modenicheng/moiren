# Application Exclusive256: joined counters and isolated source controls

Date: 2026-10-31, Asia/Shanghai. Target: physical `LAPTOP-NP49RG6H`,
existing pinned SSH host key through `frp-rug.com:14857`. These controls
retain operational successes and failures, not completion of S1-S3,
continuous-waveform qualification, a new default or physical sub-10-ms output.

## Changes, verification and deployment

`6827f02` integrates independently developed `f8da640`: CLI monitor schema2
adds `final_after_owner_join`. The managed retirement path pins existing
native/professional retained counter handles before clearing live registries,
then snapshots them only after dropping/joining the corresponding host.
App generation, graph/host revision, native owner and resource identity remain
separate. Missing handles stay null, mismatched identities/Running observations
refuse publication, and `last_running_native` remains a Running snapshot.
Cleanup errors and original failures remain visible; no final counter is
fabricated from the last poll.

RED and retention-disabled mutation tests reproduce failure. Host library
tests pass502/2 ignored; headless GPUI tests595/3 ignored; scoped strict
Clippy/fmt/diff pass. Independent review finds no blocker. Remote release
library tests pass502/2 ignored, integration example110, strict Clippy and
app/example builds pass with jobs2 and shared caches, before audio starts.
Deployed app:33719808 bytes, SHA-256
`CEC7321075D3529EA2B0FBDC903F4AAA0C55BF7DAC36176CFDC5141380C3B288`.

`ac93295` integrates independently developed `a512913`: only the explicit
VAD integration example report gains snake_case direction and a detached
typed projection of the service's existing Windows name observation. Native
identity, QPC, exact MMDevice ID, actual name, error and nulls are retained.
No new resolver, native read, readiness/gate change or name fallback is added.
Projection/progress regressions reproduce RED; host/target example114 pass,
scoped strict Clippy/fmt/diff and independent review pass. Deployed holder:
7380992 bytes, SHA-256
`5C6351F0CA53A482E87E5929EBB23943933F3C87B3EBB69AB38750E6DD1F2DE6`.

App is under `D:\MoirenTest\20261031-6827f02\bin`; holder is side-by-side
under `20261031-ac93295\bin`. E2e source/recorder stays verified `0fd3fbf`,
SHA-256 `651B0596700791E3C0F3C68E35C52D2D9F0432196F50618C5E9FD8E445D1B94E`.
Driver stays diagnostic `1513274`, `oem200.inf`, SYS SHA-256
`E450E9589F3775254241D851116EF145B4D00DE007195DC6924BC6623E7F610F`.
No driver reinstall or signing occurs in these controls.

## Isolated source and exact direction

Virtual speaker/Playback is Moiren Input/Source; virtual microphone is
Moiren Output/Sink. The holder prepares its virtual Playback -> Gain0.5 ->
Microphone graph with no probe WASAPI clients or physical render. Its
180-second idle request is a source-endpoint fixture, not the measured graph
and not a successful 180-second run. The monitor itself is exactly
Process Loopback -> unity Bus -> pinned DAWN, with no Gain or extra DSP.

The source worker uses Shared only on exact owned virtual Playback
`{0.0.0.00000000}.{c42373a5-8f5d-4442-b9da-037595aa28df}`. Selection checks
stable endpoint `4d6f6972656e417070566164506c6179`, direction, current open
gate/active revision and the service's matching native identity tuple, not
display name or a catalog delta. Per-wake native identity is
`[10,680447008040,10]`. Worker ready/activate/running handshakes pin process
identity. The holder starts first; monitor's optional VAD discovery is not
needed by its Process-only graph. DAWN stays free of the source's Shared
client and is used only by monitor Exclusive render.

Both controls request Engine64/f32, DAWN Exclusive256, one-second measurement,
target4096/capacity16384/jitter96, then ten seconds of confirmed operation.
DAWN actual48 kHz stereo PCM32 period/capacity256 is observed in both.
Only the window control supplies all five thresholds: initial unserved16384,
prime-discarded4096, output frames96/reads2/span10000*100 ns. They are explicit
diagnostic limits, not default changes or evidence of stable delivery.

## Window refusal retained

`app-exclusive256-window-owned-source-1` measures1001 ms, then refuses
`native Process window Inconclusive: Process packet/read envelopes overlap or
tie; order is ambiguous`. No suffix is selected, no threshold loosened, no
measured owner confirmed and no recorder started. `run_complete=false`, active
confirmation fields null, monitor exit1; joined cleanup completes without
monitor cleanup_error/fault. This is not an Exclusive initialization failure.

Final baseline identity: app1/graph2/host1/native-owner1/resource0. Native DAWN
written49152 frames,192 wakes,0 timeouts; native dropped/silent0. Process
captured/written48000; bridge output46080, priming3136, prime-discarded352.
These counters do not make the refused window admissible.

## Separate per-wake replacement

`app-exclusive256-perwake-owned-source-1` explicitly requests per-wake mode,
without a window policy. This is a separate acceptance surface, not an automatic
fallback or a replacement verdict for the preceding failed window control.
Measurement1007 ms, app generation1 ->2, confirmed graph revision2 and
native owner1 in each separate host. The measured profile retains its origin
app1/native1/graph2, source PID7412/creation134378666991801170, packet480,
gap561 and target4096/capacity16384/jitter96. Active audio confirms
Engine64/Exclusive256. Ten seconds complete; monitor exit0, run/cleanup true,
failure=null. Numeric native owner1 alone never identifies both hosts globally.

Final joined identity: app2/graph2/host1/native-owner1/resource0, stopped,
cleanup_error/fault=null. DAWN written480512 frames,1877 wakes,0 timeouts,
native/transport dropped/silent0. Process captured/written480000, bridge
output475648, priming4928, prime-discarded224. Dropped/silent/underrun/reset,
nonfinite/discontinuity/timestamp-error counters are0. Final captured480000
exceeds last Running snapshot434400, verifying that the export is not merely
that earlier sample relabelled as final.

Delivery nevertheless reports one gap-bound violation across1000 packets /
999 intervals. Maximum117568*100 ns =11.7568 ms exceeds selected561 frames /
11.6875 ms. Zero native wait timeouts cannot erase that violation or qualify
stable delivery. Source ends after542400 acknowledged nonzero-source frames
plus1056 preroll, successful Stop/join; its60-second schedule is intentionally
not completed by this short control.

## All-zero recording and separate holder failure

An independent six-second `record-process` pins only app PID4656 /
creation134378666993411325. It starts after2.5 seconds of process lifetime,
not an exact replacement-owner confirmation. It records287520 stereo f32
frames /2300160 bytes,599 packets. Raw sample inspection finds every byte
zero, all samples finite, peak/RMS0. Metadata silent_frames0 means no SILENT
flags; it does not establish nonzero audio. Recorder timeout_wakes1, failures
empty, Stop succeeds; exit1 is its always-unqualified contract. It proves
neither the app's continuous waveform nor actual DAWN audibility, and does
not establish a sole source/session/Exclusive-loopback cause.

Both holders fail independently: phase becomes Starting with no active
revision while exports are still Present/open. Window holder ends after9058 ms,
per-wake after16625 ms. The normal five-second gate re-admission allowance
requires the owner to remain Running/confirmed and therefore does not admit
this state. Service join and control release are true, top-level cleanup_error
null, but final `exit_service_drained` graph_error explicitly retains host
cleanup_error=Cancelled. Do not describe that as clean holder teardown.
Source-stop token emission was not timestamped, so stop/failure ordering and
whether this is an expected transition or a transport fault remain unresolved.
The probe exits at first lost confirmation; no permanent non-recovery is proven.

Do not repeat the same headless permutations or lower another target to hide
these failures. The next useful discriminator is a bounded source-stop/host
state transition record, plus independent nonzero-source/output evidence under
an actual interactive session when available. Session0/no logged-on user is
only a confound; no automatic login/privacy change is performed. Window order
ambiguity must stay inconclusive, not be solved by sorting overlapping events.

## Retention and remaining gates

The runner is explicitly collector-only; script exit0 does not imply acceptance.
Owned processes and children are gone, only root PnP remains OK. No forced
termination, timeout of an owned process, settings write, reboot or cache clean
is recorded. All volume/mute/default roles stay unchanged. Boot remains
`2026-10-30T09:50:49.659845+08:00`, D: free242069159936 bytes. No target
artifact is deleted; current free space does not justify another cache clean.

`app-exclusive-joined-20261031.zip`:14814956 bytes,52 manifest files /53
entries,46165434 payload bytes excluding archive manifest, SHA-256
`098327A6612A2E8BD54CA59D31D07EE138A528463163AD76B5ECDBEDBBF620EE`.
Host verification checks receipt hash/length, CRC, exact entry set, every file
length/hash and total. Holder binary/deployment and build logs remain separate
host/target artifacts, not falsely claimed as contents of this run archive.

S3 gains actual Exclusive256 per-wake replacement and honest post-join counters.
Window admission, nonzero continuous output and holder transition remain open.
S1 calibrated physical measurement, S2 zero-fault lower-budget qualification,
and the complete stable sub-10-ms objective remain unachieved. No smaller
default, waveform pass or VAD/Process latency winner is promoted.
