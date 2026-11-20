# Actual Process interpolation observations

## Scope and implementation

This S1 slice records actual `ClockSource` interpolation, rather than fitting a
single affine clock to OBS PCM. It does not change audio samples, servo tuning,
queue defaults, admission, recovery, cancellation or Process archive schemas.
S1-S3 remain incomplete; physical qualification is still unavailable.

`BridgeObserver::interpolation_history` exposes an independent 256-event suffix.
Native Process delivery observations include the same diagnostic after owner
retirement/join. Each emitted span retains its nonempty read sequence, requested
output start, accepted native-input start, actual initial phase/step, buffer
generation, fade remaining and 1..32 emitted frames. A span ends at read, servo
or emitted-pair generation boundaries. Downsampling can skip directly between
two internally consistent generation pairs; telemetry splits there without
introducing an audio reset.

Requested-output frontiers include entirely priming reads and unfilled terminal
tails. Started/completed read counts identify an in-flight read. Empty/invalid
requests do not advance them. Accepted input indices exclude unpublished drops;
joining packet accepted frontiers to offered positions is separate work. Neither
timeline is a device clock, OBS frame offset or transport owner generation.

Replay must use the actual recurrence: subtract integer phase crossings before
each sample, interpolate its two finite f32 neighbors in f64, apply the recorded
32-frame recovery fade in f64, cast to f32, then add the recorded step. The
closed form `phase + k * step` is not bit-exact. A real JSON text roundtrip test
exposed one-ULP decimal parsing differences; exported `phase_bits` and
`step_bits` therefore contain IEEE-754 u64 bits as decimal **strings**, avoiding
both decimal-float rounding and JavaScript's 53-bit integer limit.

The ring is allocated during preparation and adds 18,488 bytes of scalar
storage to the existing byte-budget calculation. Its unique writer publishes
atomic payloads between SeqCst odd/even version markers; control snapshots
make one attempt and return unavailable on contention. No per-sample clock,
allocation, deallocation, logging, lock or retry is added. Invalid/exhausted
diagnostics become permanently saturated, never an audio cancellation request.
Writer closure is not native transport cleanup.

At 48k with unsplit reads, 256 full 32-frame spans cover about170.7ms. Read
segmentation can produce one span per output frame, reducing this to5.33ms.
Overwrites remain explicit. This is not a full-run archive: continuous polling,
retained native PCM and authenticated cross-boundary correspondence are not
implemented by this slice. Existing inventory replay/native-proof gates do not
consume these editable DTOs.

## Regression controls

The initial test fails because no actual observations are available, then
passes with the real source implementation. Independent changing stereo input
is reconstructed sample-for-sample from observed cursors across 31/32/33 and
irregular read splits, servo changes, supported unequal rates, startup trims,
backlog recovery fades, partial underruns, generation skips and extreme finite
PCM. Empty/invalid requests, terminal silence, exact JSON export, ring overwrite,
concurrent snapshots/writer closure, overflow and bounded import are covered.
The allocator guard verifies zero RT allocation/deallocation through overwrite
and underrun; injectable clock observations remain two per ordinary read.

Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
The initial RED receipt is `s1-interpolation-red-0545.log`; decimal-export RED
is `s1-interpolation-json-0653.log`, followed by
`s1-interpolation-json-green-0655.log`. Failures are preserved, not overwritten.

## Remote diagnostic smoke and limitations

Native/device work runs only on `LAPTOP-NP49RG6H` in isolated task-owned paths.
User OBS12648 and QQMusic15096 remain untouched. Source generation, two-track
OBS recording and offline analysis stay remote; only compact receipts and PNGs
are transferred. No development-host process/device is manipulated.

The first pre-bit-export smoke lives at
`D:\MoirenTest\20261102-s1-interpolation-0627`. Its35s measured owner records
52,446 spans, retains256, reports52,190 overwrites, requested1,681,536 frames
and26,274 completed reads; writer closure is true and saturation false.
Loss/underrun/reset counters are0, but two packet-gap-bound violations remain.
The monitor and source exit0; owned OBS requires forced termination after
recording stops. Thus capture completion does not mean clean task cleanup.

This first harness mistakenly lets the46s source outlive its35s owner. Its
fourth waveform window is after deliberate monitor shutdown and returns silence;
the first three also fail unchanged affine residual gates. These are not four
confirmed in-run drops or a latency result. The80.704s recording and original
refusals remain intact. Its visualization is
`s1-interpolation-analysis-overview-0648.png` in the host evidence root
(249,923 bytes, SHA256
`0ABEB01812D174BDA69B7FC0499BA1095F51C8B9EA76B0FC27B167E3B5362712`).

The fresh exact-bit build uses a60s owner to cover the whole46s source and lives
at `D:\MoirenTest\20261102-s1-interpolation-exact-0708`. Binary SHA256:
`58B08693AE82E21C993ACBDE755A94094A98CA8679387A77A8D8F7C6FF017B94`.
It is a source-manifest build including preserved dirty sources, not a clean
commit binary. Exact source pins are retained in
`s1-interpolation-source-manifest-exact-0708.json`. No target/default is reduced.

The exact-bit monitor completes60,002ms and all three owned children exit0
normally. The source submits2,208,000 frames with0 signal underflows. After join,
the source reports89,932 spans/89,676 overwrites, requested2,881,088 frames and
45,017 started/completed reads; saturation false, writer finished true. Native
capture/write totals are2,880,000 frames, with0 dropped/silent/nonfinite samples,
underruns, discontinuities, timestamp errors or resets. Eight delivery-gap-bound
violations remain (maximum11.1537ms). Startup3264 priming output frames and352
trimmed input frames remain separate. Operational completion is therefore not
a zero-quality-fault PASS, and no physical latency is inferred.

Remote analysis retains all four fixed noise windows:1 model-compatible,
3 residual refusals. All are now inside the running owner interval. The entire
fixed no-Moiren control has reference NCC1.0 and return RMS/peak0; this only
establishes that bounded pre-owner interval. The82.2186667s two-track recording
and rejected windows remain remote. The actual cursor graph covers the final
170.7ms suffix, not the earlier fitted waveform windows; it is not an explanation
of their residuals or an established continuous native-PCM mapping.

Visualization: `s1-interpolation-analysis-overview-exact-0715.png`,260,177 bytes,
SHA256 `1BE5A042AF72286938AF761BD84CC155583D0D23155473D9FF20CBD1C2CF95BA`.
Summary SHA256
`EC64527647B994F4768DD49C8A94F3344620696362BD323A4AD9365FA943B91D`;
exact history SHA256
`D8856C998E91AB80CE4CF5D4EA1024B5FB5BEEED8C9D5A9BF53514D8BB15ABE2`.
Only these compact outputs are downloaded; raw media stays on the target.

## Final software verification

- `cargo test --locked --workspace`:2113passed,8ignored across68targets.
- `cargo test --locked -p moiren-app --features gpui-test measured_ui_tests`:
  20passed.
- Strict workspace/all-target Clippy and workspace fmt check pass.
- An independent final static review finds no concrete correctness bug in the
  scoped changes. It does not substitute for measured RT overhead or native PCM.
- All18 preexisting dirty files retain their original SHA256; no unrelated
  changes are staged, no remote branch is pushed, and no memory is updated.

Final logs: `s1-interpolation-workspace-final-0710.log`,
`s1-interpolation-clippy-final-0712.log`, `s1-interpolation-fmt-final-0712.log`,
`s1-interpolation-ui-final-0712.log`. The original RED, intermediate lint failures
and unsuccessful status reads remain in the evidence root rather than being
rewritten as successful receipts.
