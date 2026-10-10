# M0.6 Audio Host verification record

This record distinguishes implemented contracts, deterministic regression evidence and hardware evidence for the six M0.6 requirements. Original frontend development remains in the main checkout; backend work is on `feat/interactive-audio-host` in the isolated `audio-host` worktree.

## CI regression diagnosis

The failed run [37898563420](https://github.com/modenicheng/moiren/actions/runs/37898563420), commit `c581bbf`, failed two Compressor tests under Tree Borrows. The log reports assertion failures, rather than an undefined-behavior diagnostic:

- `compressor_hold_delays_release_but_never_attack_and_persists_between_windows`: sample examples differ by about `1e-16` between whole/split processing.
- `compressor_silence_remains_finite_and_matches_variable_blocks`: the sample `0.5194523896757088` is compared with `0.5194523896757085` using exact equality.

The user-referenced run [37913684994](https://github.com/modenicheng/moiren/actions/runs/37913684994), base commit `a1c07d6`, has also completed. Windows and Linux native jobs passed; Miri failed the same two assertions in its default-mode step (26 engine tests passed, two failed). Its downloaded failed-step log has no undefined-behavior diagnostic. The local full default/Tree Borrows results below validate this branch's corrections; they do not relabel that older remote run as green.

The Miri comparison checks every sample and exact vector shape, using a tolerance of `32 * f64::EPSILON * max(1, |actual|, |expected|)`. Final review also required an explicit finite check on both samples: tolerance arithmetic alone can accept infinity because both difference and tolerance become infinite. This correction has a dedicated nonfinite regression. Integration with the concurrently advanced `main` (`21cba63`) preserves exact chunk-output equality in native builds and uses this tight comparison under Miri. Existing independent envelope/hold/silence expectations remain in the tests. No DSP behavior or aliasing checks have been removed.

The same log shows 1,127 seconds for the full `u16` Bus port fixture and 3,271 seconds for graph integration tests under Tree Borrows. The native 65,536-port integration fixture remains intact. Miri now checks the actual production count guard at its exact lower/upper boundaries without constructing 65,536 ports, and keeps all small Bus edit tests. Deep-chain Miri tests use 32 nodes; native stress still uses 512 nodes. Both exercise iterative ordering, reconnection and cycle rejection.

The host compile-failure test also cloned and validated 100 growing graphs, taking more than 13 minutes under Tree Borrows before later tests could run. Miri now uses 16 edits and a 1,024-byte budget, while native tests retain 100 edits and a 4,096-byte budget. Both hit the real compiler budget failure, retain the prepared source backend, correct the graph, render successfully, and destroy the backend exactly once. The bounded fixture was checked with `--cfg=miri` as well as the interpreter. No buffer aliasing, runtime allocation, processor, parameter or plan-swap test is disabled by this adjustment.

Default Miri and Tree Borrows both completed the full core/engine/app suites successfully on nightly `1.101.0-nightly (a30aa9064 2026-10-08)`. After the final shutdown/finite-tolerance corrections, both modes also passed the 13 Compressor tests and focused full-reply/future-request shutdown and pending-publication receipt tests. The first old-size app runs were deliberately interrupted after the expensive fixture was identified and replaced; they are not counted as passes.

At capture-retirement correction `e1568ec`, locked Windows all-target workspace tests, Linux workspace tests, strict all-target Clippy and formatting passed again. The native backend, app and harness fixtures cover explicit producer completion before native cleanup, gating while cleanup is still pending, finite queued tails, preserved live-underrun history and AppliedLate confirmation. Reproduction commands are:

```text
cargo test --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
cargo +nightly miri test --locked -p moiren-windows-audio --test clock_bridge -- --skip adaptive_bridge_handles_both_drift_signs_and_packet_jitter
```

The focused Miri command passed all 14 remaining bridge tests in both default and `MIRIFLAGS=-Zmiri-tree-borrows` modes, including explicit finite finish, zero allocation/deallocation, cumulative live-underrun preservation and exact starvation characterization. The long drift test is exercised natively as described below. Independent review of `cc69781..e1568ec` approved spec compliance and code quality with no material findings; its one minor coverage suggestion concerns isolating the enable-after-retirement predicate with a still-owned unfinished native worker. The actual poll/cleanup/peer-silence regression and production guard are present.

Final main integration `599e108` changes only Compressor tests relative to the measured backend. Native `cargo test --locked -p moiren-engine compressor_` passed 17 processor/integration tests; `cargo +nightly miri test --locked -p moiren-engine --lib compressor_` passed all 13 processor tests in both interpreter modes, followed by strict engine Clippy, fmt and diff checks. A temporary one-ULP native chunk mutation failed exact equality and was restored before the passing checks. The complete production-code diff from the hardware-tested branch is empty, so the hardware record's `e1568ec` identity remains accurate.

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

Current-code evidence is from `e1568ec`: one 620.043-second hardware run and six fresh 30-second cold runs, with workspace builds/tests and Miri overlapping the pressure interval. Each host retained one native Render; parallel hosts are test pressure rather than a claim of multi-output graph support. The long run processed 29,760,480 DSP frames and submitted 29,761,536 frames including a 1,056-frame silent prime. Every run confirmed revisions 2 through 8, five normal Applied parameters, the deliberate StaleRevision race and retry, all six terminal parameter receipts, isolated target exit and successful native Stop. Cumulative live and finished input underruns across every observed source generation were zero; render bridge shortfall and empty padding were also zero.

In the current long run's 592.002-second steady window, both final input generations had zero new underrun, dropped frames, reset or discontinuity. Final physical capture retains one native discontinuity from startup/reconfiguration; its reset counter is zero. Final fill was 1,911 frames for both sources, with correction -8.85 ppm physical and -5.59 ppm process. These are servo diagnostics, not a calibrated physical clock-drift measurement.

The earlier baseline below is retained for comparison and is not substituted for current-code validation:

| Measurement | Recorded result |
| --- | --- |
| Continuous actual device run before silent-prime change | 620.056 seconds; 29,761,632 DSP/output frames |
| Confirmed plan revisions | 2 through 8, at actual render-thread block boundaries |
| Normal live parameter requests | Five Applied confirmations |
| Deliberate parameter/publication race | Sixth request has terminal StaleRevision; retry against confirmed revision Applied |
| Source exit | TargetExited observed; physical input and render kept running |
| Render bridge shortfall / empty padding | 0 / 0 |
| Source underrun in the pre-change baseline | 0 in that run and two further 30-second cold regressions |
| Final shutdown | Native Stop succeeded; renderer reclaimed on control |

Two additional 30-second cold scenarios ran during the long run. WSL native/Miri checks also ran concurrently. Both short scenarios passed all lifecycle assertions, seven plan confirmations, five normal Applied receipts and six terminal receipts, with zero source underrun/render bridge shortfall/empty-padding. The first exploratory 30-second run also completed these hardware actions; its harness incorrectly demanded Applied for the intentional retiring-revision race, and was corrected before the recorded passing runs.

The long run's final physical/source generation recorded two native discontinuities and one reset, plus initial/restart priming. The final process generation recorded no discontinuity/reset. In the 592-second steady interval after the scripted reconfiguration, both final inputs had zero new underrun or dropped frames; physical capture had one new reset and process capture had none. Startup, deliberate disable/backlog discard, process exit and explicit capture restart are not omitted from the raw aggregate counters; the JSON reports their priming/reset/fill diagnostics separately from the steady interval.

CLI hardware smoke also verified a three-second bounded run continuing after stdin EOF, and status/graph/explicit stop JSON commands. Invalid process preflight now emits exactly one structured `startup_failed` event and a nonzero exit before opening a valid device.

## Cold-start boundary

The previous Process Loopback record retains the unexplained 211-frame cold-child shortfall. These successful hardware runs do not establish its original scheduler/API cause or prove all devices immune to startup underrun.

An additional deterministic regression uses the real capture bridge, compiler, engine and demand renderer: a 4,096-frame pre-Start device fill consumes a live producer's 2,048-frame capture reserve before normal device cadence exists. It outputs 2,047 input frames, underruns, then re-primes. Native startup now submits an initialized silent device prime without consuming the engine/capture reservoir, then renders normal event demand after Start. The regression verifies the retained reserve, zero DSP timeline before Start, and complete first normal demand; 79 native backend tests and strict Clippy passed. This removes that reproducible startup burst and adds at most one native buffer of initial silence (22 ms for the measured device); it does not claim to fix arbitrary packet loss or the historical event's unproven cause. DSP processed frames now exclude the silent native prime; submitted frames include it.

The post-prime 620.055-second run processed 29,760,960 DSP frames and submitted 29,762,016 frames, including its 1,056-frame silent prime. Render shortfall and empty padding were zero, and all lifecycle/receipt assertions passed. However, the replaced process generation recorded 128 underrun frames at its 3,488th output frame while its producer was alive. Its subsequent final generation and steady interval had zero underrun. A post-shutdown-correction 30-second run repeated 128 startup underrun frames in the replacement generation, at output frame 2,048; the deliberately terminated old process also had 114 tail underrun frames after producer retirement. A further quiet 30-second comparison had zero underrun. These results are retained separately: a healthy final generation must not hide an earlier generation's failure.

Instrumentation measured 480-frame process packets/native capacity, below the unchanged 2,048-frame reserve. A deterministic regression reproduces both exact 128-frame prefixes by pausing publication after the initial reserve or after three further packets; this identifies ordinary reservoir depletion, without establishing why Windows publication paused in the historical runs. First-underrun packet count/age and bounded first-16 packet-interval diagnostics now make a recurrence attributable. At 48 kHz, the reserve covers about 42.7 ms; it cannot cover arbitrary capture scheduling gaps. Real shortfalls remain counted and trigger re-prime.

Six instrumented pressure trials found a separate shutdown boundary defect: the killed process's packet loop had ended, but native Stop/COM cleanup delayed the producer-finished marker, allowing its tail to be consumed as live. Correction `e1568ec` publishes producer completion before cleanup, closes the host gate while cleanup is pending, and prevents enable from reopening that producer. Finite consumers still drain queued audio and report tail underruns. The harness now latches cumulative live underruns every poll and retains removed/replaced generations, writes evidence before rejecting an unhealthy run, and accepts actual AppliedLate receipts while recording their IDs separately.

The six fresh 30-second cold scenarios on `e1568ec` all passed, including zero cumulative live/finished input underrun, zero render bridge shortfall/empty padding, seven plan confirmations, five Applied parameter confirmations, all six terminal receipts and successful Stop. They overlapped the new 620-second run; all children were newly created, without capture prewarm. These results demonstrate the measured regressions on the corrected code; they do not turn the historical live 128-frame observations into a claimed Windows-cause fix.

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
| Continuous single render / Gain/Pan | Current-code 620-second native session and five normal Applied confirmations | Passed |
| Real plan swap / control reclamation | Seven native confirmations; retained sink; zero-alloc swaps and caller-thread destruction tests | Passed |
| Physical and process inputs together | Real simultaneous microphone and independently owned process feeding stereo Bus | Passed |
| Source failure isolation and replacement | Native exit, replacement, stop/restart and remove/add without ending render | Passed |
| Cold start / drift / stop / reconfiguration | Six current-code cold runs, 620-second pressure, four two-hour simulated drift traces; startup/retirement regressions and Miri both modes passed | Passed for scoped regression validation |
| UI control contract | Domain API/CLI/parser/lifecycle tests; independent Task 2/3 review passed | Passed |

The fifth requirement is regression and pressure validation, not a guarantee against arbitrary operating-system capture delays. Historical producer-live 128-frame observations remain retained and unattributed; successful current runs do not prove their Windows cause fixed. The reproducible pre-Start reserve drain and late producer-retirement defects are separately fixed and tested.

Device disappearance, physical hotplug, system sleep/resume, exclusive-mode output, subjective end-to-end latency/audio quality, OBS and broad third-party application matrices were not exercised. Automatic device recovery is not implemented: failures are explicit, input failures are isolated, and restarting/replacing a selection is a deliberate control operation. See the separately measured [controlled Takeover experiment](2026-10-09-takeover.md) for the limits of session mute/volume routing. The M0.6 evidence above must not be used to claim those additional scenarios passed.
