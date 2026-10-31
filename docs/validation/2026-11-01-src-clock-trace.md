# Known time-varying production SRC centers

## Scope and result

The [frozen interval analysis](2026-11-01-indexed-spacing.md) rules out one
constant-affine map for the historical model coordinates. It does not
distinguish actual time-varying processing from coordinate-model bias. This
follow-up validates a narrower prerequisite on a fully known synthetic input:
two independently driven production servo/SRC stages can yield incompatible
constant-affine spacing while the unchanged identity model still identifies
each code and contains its known composed filter center.

That behavior is demonstrated for the fixed fixture below, not inferred for
the historical recording. No native waveform is reanalyzed. No production
servo, FIR, boundary, matcher, thresholds, buffer policy or default changes.
The added code is test-only and does not enable another acquisition command.

## Frozen fixture and independent checks

At base `1cacacf`, before implementation or evaluation, the hypothesis freezes:

- 144000 source frames, 48 kHz stereo, three original indexed-v2 codes of1024
  frames at24000/72000/120000, amplitude0.025; zeros between markers.
- Production `FollowerServo`, target1000 and smoothing0.01. Playback receives
  prescribed occupancy2000 for1500 output callbacks, then1000. Microphone
  receives occupancy2000 for3000 input callbacks, then1000. Both start at zero
  correction and use the production +/-2000 ppm authority.
- Production `StereoResampler` with4096 storage frames. Playback requests32
  outputs through `input_frames_needed`; microphone pushes32 input frames and
  drains all available output. Each stage carries its own continuous FIR state.
- Unchanged default model: one/two nominal FIR passes, frozen phase/rate grid,
  correlation0.7, margin0.05, RMS0.0001. Each window starts at the known composed
  center's floor minus64, spans1160 frames and competes against all three codes.
  These are synthetic known-center windows, not a newly fitted capture origin.

Occupancy is a prescribed arithmetic stimulus, not feedback from a live
carrier. This isolates servo/SRC behavior without inventing a native queue
history. The fixture models call cadence, not the complete route: it does not
execute VirtualPlaybackSource/VirtualMicrophoneSink, startup/recovery muting,
epoch handling, workers, driver queues, native clients or physical devices.
The final insufficient-input tail is not zero-padded into additional outputs.

For each output the fixture separately integrates the known source-center
step. A direct-index scalar oracle applies the32-tap/256-phase FIR definition
at those centers without using the production ring/discard implementation or
kernel builder. It validates sample placement against the streaming FIR; it
shares the intended filter definition and is not independent hardware timing.
An intentionally shifted one-frame center produces peak error above0.005 in
both stages, so the oracle cannot pass merely by comparing silence.

The fixed zero-correction controls agree exactly. The varying playback and
microphone maximum absolute sample errors are respectively
`3.725290298461914e-9` and `1.8277205526828766e-8`, below the pre-frozen2e-7
bound. All output centers are strictly increasing; every correction remains
within authority. A separate45-digit Decimal computation reproduces the
servo recurrence, supply/demand schedule and composed center inversion within
1e-5 frame, without calling the Rust FIR or fitting recorded coordinates.

## Coordinates and unchanged model

Here "coordinate" means the recorder index at the composed source filter
center, obtained by inverting each known monotonic center sequence. It is
neither physical onset nor latency, and is not an exact model of a cascade's
entire time-varying impulse response.

| Source start | Known varying center | Unchanged model interval | Model score |
| --- | --- | --- | --- |
| 24000 | 23916.87735246831 | [23916,23918] | 0.9608979079475121 |
| 72000 | 71766.76869355659 | [71766,71768] | 0.9536081686638263 |
| 120000 | 119712.37411027853 | [119712,119713] | 0.9256622614989115 |

All three exact original identities are Unique; each interval contains the
independently known center. For the on-target control the known centers are
exactly24000/72000/120000, all model scores1, and intervals[start-1,start+1].
Each three-window analysis charges31186080 correlation samples, below the
unchanged256million cap. These controls validate these inputs, not a universal
one/two-frame error guarantee for arbitrary histories, signals or cascades.

Using outward integer intervals for the varying known centers gives disjoint
adjacent slope bounds:

```text
first spacing:  [0.9968541666666667, 0.9968958333333333]
second spacing: [0.9988541666666667, 0.9988958333333333]
```

Consequently no single constant affine map fits these three known-center
intervals. Both control bounds are exactly[1,1]. This is a constructive
synthetic example of time-varying mapping without any injected frame loss,
not an explanation of the historical non-affinity. The differing source and
output lengths reflect SRC stepping and withheld FIR tail, not measured loss.

Further negative controls erase one marker quarter, delete16 source samples
or insert16 zeros at the middle marker. Under the same varying two-stage
processing, the unchanged model refuses a complete Unique identity in each
case and leaves the identity/coordinate unknown. This is finite fault
coverage, not proof that all corruption is detectable or of continuous native
waveform quality. No identity is promoted to a known clock origin or delay;
`source_origin=unknown_not_estimated` and packet quality remains unevaluated.

## Verification and evidence

The new test module is
`examples/e2e_latency_probe/indexed_pcm/src_model/tests/clock_trace.rs`.
The existing example imports the production servo only under `cfg(test)`;
its five existing servo unit tests also run in that example binary.

The first two RED attempts expose a nested-module path error and an unused
constant; those compilation failures are retained, not counted as behavioral
RED. After correcting the test harness, all three initial tests fail at the
explicit missing fixture. The first implementation compile exposes unused
servo accessors in this isolated import; assertions use and check those
accessors without suppressing lints. The three tests then pass. Stronger
center-containment, shifted-center and waveform-fault controls bring the
final focused suite to five passing tests without changing the frozen input,
model or thresholds.

Evidence root is `D:\coding\moiren-evidence\20261101-src-clock-trace`.
Command receipts retain argv, working directory, UTC start/end, exit code,
stdout/stderr length/hash and all failed runs. `hypothesis-freeze.json` pins
the original production sources, model/test imports and unrelated `gate.log`.
`independent-result-audit.json` records30 checks, the Decimal center audit,
model work, original identities, center containment, scope and preserved
production/model/gate hashes. The final manifest covers the evidence files.

Fresh checks pass:

```text
cargo test --locked -p moiren-windows-audio --example e2e_latency_probe clock_trace -- --nocapture
    5 passed
cargo test --locked -p moiren-windows-audio --example e2e_latency_probe
    645 passed (includes the five imported servo unit tests)
cargo test --locked -p moiren-windows-audio
    952 passed / 3 ignored (default features; 32 suites including doc-tests)
cargo clippy --locked -p moiren-windows-audio --all-targets --all-features -- -D warnings
cargo check --locked -p moiren-windows-audio --all-targets --all-features --target x86_64-unknown-linux-gnu
cargo fmt --all -- --check
git diff --check
```

These are host software checks, not native execution, Linux execution, remote
CI, Miri, physical qualification or a sustained run. No app or workspace-wide
test result is claimed for this test-only example slice.

## Remaining S1-S3 work

The constructive control shows that time-varying processing can explain this
*class* of non-affinity while retaining recognizable identities. It does not
select that explanation for the historical capture or rule out its model bias.
Existing BoundaryProbe retains latest relaxed occupancy/correction snapshots,
not a per-callback correction/consumption/phase history. Sparse samples cannot
reconstruct the historical mapping. Do not synthesize that history or enlarge
model intervals to make it fit.

A future actual mapping experiment needs retained per-stage phase/consumption
provenance or a capture-aligned independent changing reference. Native
verification of recent tail-fault fixes, complete waveform/boundary quality,
matched VAD/Process comparison and calibrated physical acceptance remain open.
Original14/29 nominal refusal,9686 packet-policy violations,14 unknown
boundary observations and `quality_complete=false` are unchanged.
S1-S3 and the overarching goal remain incomplete. There is no native replay,
remote/audio process, driver/system-control write, speaker render, push or
subagent work; unrelated `gate.log` is preserved.
