# M0.6 Audio Host verification record

This record distinguishes implemented contracts, deterministic regression evidence and hardware evidence for the six M0.6 requirements. Original frontend development remains in the main checkout; backend work is on `feat/interactive-audio-host` in the isolated `audio-host` worktree.

## CI regression diagnosis

The failed run [37898563420](https://github.com/modenicheng/moiren/actions/runs/37898563420), commit `c581bbf`, failed two Compressor tests under Tree Borrows. The log reports assertion failures, rather than an undefined-behavior diagnostic:

- `compressor_hold_delays_release_but_never_attack_and_persists_between_windows`: sample examples differ by about `1e-16` between whole/split processing.
- `compressor_silence_remains_finite_and_matches_variable_blocks`: the sample `0.5194523896757088` is compared with `0.5194523896757085` using exact equality.

The fix checks every sample and exact vector shape, using a tolerance of `32 * f64::EPSILON * max(1, |actual|, |expected|)`. Nonfinite differences still fail. Existing independent envelope/hold/silence expectations remain in the tests. No DSP behavior or aliasing checks have been removed.

The same log shows 1,127 seconds for the full `u16` Bus port fixture and 3,271 seconds for graph integration tests under Tree Borrows. The native 65,536-port integration fixture remains intact. Miri now checks the actual production count guard at its exact lower/upper boundaries without constructing 65,536 ports, and keeps all small Bus edit tests. Deep-chain Miri tests use 32 nodes; native stress still uses 512 nodes. Both exercise iterative ordering, reconnection and cycle rejection.

The host compile-failure test also cloned and validated 100 growing graphs, taking more than 13 minutes under Tree Borrows before later tests could run. Miri now uses 16 edits and a 1,024-byte budget, while native tests retain 100 edits and a 4,096-byte budget. Both hit the real compiler budget failure, retain the prepared source backend, correct the graph, render successfully, and destroy the backend exactly once. The bounded fixture was checked with `--cfg=miri` as well as the interpreter. No buffer aliasing, runtime allocation, processor, parameter or plan-swap test is disabled by this adjustment.

Default Miri core/engine completed successfully on nightly `1.101.0-nightly (a30aa9064 2026-10-08)`. Default app and the complete Tree Borrows suites are in progress; final results will be recorded after completion. The first old-size app runs were deliberately interrupted after the expensive fixture was identified and replaced; they are not counted as passes.

## Continuous backend ownership

`RenderOptions::continuous`, `CaptureOptions::continuous` and `ProcessLoopbackOptions::continuous` explicitly support running until stopped. Other durations still require the existing 1..600 second interval.

The render worker now returns its engine and output reader after its COM scope has ended. `join_with_renderer` returns these resources to control; the compatibility `join` and session Drop destroy them on the caller. Hardware-independent tests cover stopped and panicking native owner scopes, actual invalid-endpoint COM startup failure, and caller-thread processor destruction.

The render observer publishes coherent atomic timeline/counters/current-demand peak snapshots. Its startup acknowledgement changes only after native WASAPI Start succeeds; priming DSP alone does not prove a device is running. The render allocation test includes an attached observer and verifies zero allocations and deallocations. Attachment has exclusive publication ownership, preventing two renderers from interleaving writes into one observer.

## Native regression conditions and results

Reproduce only with explicitly selected, supported endpoints:

```powershell
cargo run --locked -p moiren-app -- host --list
cargo run --locked -p moiren-app --example host_regression -- --output '<48 kHz stereo f32 output ID>' --input '<44.1/48 kHz mono/stereo f32 input ID>' --generator-output '<isolated 48 kHz stereo f32 output ID>' --seconds 620
```

The helper creates and terminates only its own independent 440 Hz, amplitude 0.02 generators. It performs no default-device, endpoint-volume or unrelated-session changes. Initial capture strips use gain 0.05 before publication. The native case used a 48 kHz mono physical microphone and 48 kHz stereo output with 1,056 native buffer frames, plus process-tree capture of a newly spawned generator on an existing isolated endpoint. There is no audio prewarm. Aggregate evidence is in [audio-host results](2026-10-09-audio-host-results.json); no PCM, endpoint IDs, process IDs, full application paths or raw QPC values are committed.

The scenario changes independent Gain/Pan, inserts a compressor, disables/enables one source, terminates its process, replaces it with a freshly pinned process, stops/restarts physical capture, removes/adds process capture, publishes another compressor, rejects an invalid new input, and stops the whole host. The output sink ID and monotonic render timeline are checked throughout.

| Measurement | Recorded result |
| --- | --- |
| Continuous actual device run | 620.056 seconds; 29,761,632 DSP/output frames |
| Confirmed plan revisions | 2 through 8, at actual render-thread block boundaries |
| Normal live parameter requests | Five Applied confirmations |
| Deliberate parameter/publication race | Sixth request has terminal StaleRevision; retry against confirmed revision Applied |
| Source exit | TargetExited observed; physical input and render kept running |
| Render bridge shortfall / empty padding | 0 / 0 |
| Source underrun | 0 in the measured run and two further 30-second cold regressions |
| Final shutdown | Native Stop succeeded; renderer reclaimed on control |

Two additional 30-second cold scenarios ran during the long run. WSL native/Miri checks also ran concurrently. Both short scenarios passed all lifecycle assertions, seven plan confirmations, five normal Applied receipts and six terminal receipts, with zero source underrun/render bridge shortfall/empty-padding. The first exploratory 30-second run also completed these hardware actions; its harness incorrectly demanded Applied for the intentional retiring-revision race, and was corrected before the recorded passing runs.

The long run's final physical/source generation recorded two native discontinuities and one reset, plus initial/restart priming. The final process generation recorded no discontinuity/reset. In the 592-second steady interval after the scripted reconfiguration, both final inputs had zero new underrun, dropped frames or resets. Startup, deliberate disable/backlog discard, process exit and explicit capture restart are not omitted from the raw aggregate counters; the JSON reports their priming/reset/fill diagnostics separately from the steady interval.

CLI hardware smoke also verified a three-second bounded run continuing after stdin EOF, and status/graph/explicit stop JSON commands. Invalid process preflight now emits exactly one structured `startup_failed` event and a nonzero exit before opening a valid device.

## Cold-start boundary

The previous Process Loopback record retains the unexplained 211-frame cold-child shortfall. These successful hardware runs do not establish its original scheduler/API cause or prove all devices immune to startup underrun.

An additional deterministic regression uses the real capture bridge, compiler, engine and demand renderer: a 4,096-frame pre-Start device fill consumes a live producer's 2,048-frame capture reserve before normal device cadence exists. It outputs 2,047 input frames, underruns, then re-primes. Native startup is being changed to submit an initialized silent device prime without consuming the engine/capture reservoir, then render normal event demand after Start. This removes that reproducible startup burst; it does not claim to fix arbitrary packet loss or the historical event's unproven cause. Final native regression and post-change hardware checks are pending.

## Drift regression

```powershell
$env:MOIREN_CLOCK_SIM_SECONDS = '7200'
cargo test --locked -p moiren-windows-audio --release --test clock_bridge adaptive_bridge_handles_both_drift_signs_and_packet_jitter -- --nocapture
Remove-Item Env:MOIREN_CLOCK_SIM_SECONDS
```

All four two-hour simulated timelines passed: 44.1/48 kHz input at -1,000/+1,000 ppm, intermittent packets, variable consumption, zero underrun and zero dropped frames. Final correction ranged from -1,009.71 to +990.31 ppm and fill from 1,828 to 1,847 frames. The 620-second device measurement is a hardware test; these two-hour traces are deterministic simulations and are recorded as such.

## UI contract

The [app README](../../crates/moiren-app/README.md#windows-持续多源-host) documents `host::windows::HostSession`, `AudioHost`, JSONL commands, and source/revision/receipt semantics. The domain adapter owns preparation, polling and joins on a UI control worker. Normal UI methods use selections, source IDs, typed graph commands, parameter requests, plan results and data snapshots; they do not expose Engine or COM ownership.

| Domain | Public contract |
| --- | --- |
| Graph | desired/active GraphSnapshot, typed GraphCommand, transactional validation errors |
| Runtime | SessionOptions, start/stop, SessionStatus, monotonic timeline |
| Parameter | gain/pan/scheduled controls, request IDs and provisional/terminal receipts |
| Plan | compile/publish/cancel, desired/active revisions and applied/rejected events |
| Devices | explicit physical endpoint choices and pinned accessible process candidates |
| Diagnostics | current output peak/work counters, per-source bridge/fill/correction/XRUN/status/failure |

Portable tests execute real prepared multi-source graph renders, compressor insertion/settings/removal and plan reuse. Their allocation counters are exactly zero for render/swap. Native-owner tests establish COM-before-engine retirement and caller-thread destruction on stop, failure and panic. Source-gate/session tests verify safe initial gain, independent silence, failed preparation preserving peers, and future terminal receipt recovery at shutdown.

## Acceptance ledger

| Requirement | Current evidence | State |
| --- | --- | --- |
| Continuous single render / Gain/Pan | 620-second native session and five normal Applied confirmations | Passed |
| Real plan swap / control reclamation | Seven native confirmations; retained sink; zero-alloc swaps and caller-thread destruction tests | Passed |
| Physical and process inputs together | Real simultaneous microphone and independently owned process feeding stereo Bus | Passed |
| Source failure isolation and replacement | Native exit, replacement, stop/restart and remove/add without ending render | Passed |
| Cold start / drift / stop / reconfiguration | Two passing cold runs, four two-hour simulated drift traces, 620-second pressure; startup-burst fix/Miri final checks in progress | Pending final checks |
| UI control contract | Domain API/CLI/parser/lifecycle tests; independent Task 2/3 review passed | Passed |

Device disappearance, physical hotplug, system sleep/resume, exclusive-mode output, subjective end-to-end latency/audio quality, OBS and broad third-party application matrices were not exercised. Automatic device recovery is not implemented: failures are explicit, input failures are isolated, and restarting/replacing a selection is a deliberate control operation. See the separately measured [controlled Takeover experiment](2026-10-09-takeover.md) for the limits of session mute/volume routing. The M0.6 evidence above must not be used to claim those additional scenarios passed.
