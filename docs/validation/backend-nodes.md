# Backend node acceptance

This record describes the backend node implementation on the
`feat/backend-complete-local` integration branch. UI editing and new frontend
bindings are outside this change. The implementation worktree lives at
`.worktrees/backend-complete` inside the repository.

## Implemented graph nodes

| Node | Implemented behavior | Boundary |
| --- | --- | --- |
| Input / Output | Real prepared IO adapters, bounded audio bridges, backend-owned lifecycle | Device support depends on the backend |
| Gain | Linear gain, sample-accurate parameter values and ramps | Range 0 to 16 |
| Pan | Stereo balance, Mono and Swap | Requires stereo; not a mono-to-stereo panner |
| Bus | Explicit dynamic-input summing | Not a Windows endpoint |
| Compressor | Linked feedforward peak detector, ten live parameters, reduction signal | No external sidechain bus |
| EQ | Per-channel parametric peaking biquad, frequency/gain/Q | One band; no shelf or HP/LP bank |
| Gate | Linked peak gate with threshold, attack, release, hold and range | No external sidechain bus |
| Limiter | Linked sample-peak limiting with immediate attack and stateful release | No lookahead, oversampling or True Peak detection |
| Delay | Preallocated bounded history, delay/feedback/mix | Integer delay; at most 60 seconds and 64 MiB per instance |
| ChannelMapper | Explicit index mapping or signed weighted matrix; rectangular IO | No implicit normalization or surround semantic mapping |
| Recorder | Read-only audio tap, bounded f32 SPSC, start/stop, overflow counters, automatic IEEE-f32 WAV file worker | Whole-block drops under backpressure; RIFF size limit; stopping/joining stays off RT |
| Meter | Exact Sample Peak/RMS windows, reader and per-channel typed signal | Not BS.1770/LUFS or True Peak |
| SpectrumAnalyzer | Per-channel FFT magnitude, periodic Hann window, one-sided amplitude normalization | FFT sizes 16 to 8192, powers of two; hop 1 to FFT size |
| Rack | Real leaf-kernel expansion, up to 64 stable slots, multi-port streams and explicit nonbranching routes | No nested Rack, IO/observer slots, arbitrary internal graph or plugin multi-bus kernels |

Rack supports Gain, Pan, Compressor, EQ, Gate, Limiter, Delay and ChannelMapper
slots. Explicit routes assign each slot exactly once and cannot fan out or merge.
The slots vector determines global execution order. ChannelMapper is the explicit
width conversion boundary; a bypassed mapper preserves its incoming width.
Preparation rejects an incompatible final width. Unrouted outputs are silent.

Meter, Recorder and SpectrumAnalyzer use independent read-only observers and
preserve audio through the graph. A pass-through boundary alone is insufficient:
the compiler requires their real observer bindings.

## Control and realtime acceptance

- LogicalGraph remains editable control data; prepared buffers, kernels and
  signal permissions belong to an execution plan.
- Rack parameter intent uses stable node/slot/parameter identities. Saved values
  are domain-checked after expansion and applied before the first render block.
- Compatible updates remap stable slot IDs to plan-local ProcessorIds. Unchanged
  active slots retain DSP state and parameter ramp progress only when their
  actual input/output widths match. Bypass or static configuration changes do
  not reuse the old kernel.
- Meter/Recorder readers are managed by the host. Retained handles do not keep
  audio alive; generation fencing prevents access to a replacement owner.
- Runtime tests track allocation, reallocation and deallocation during rendering
  and plan transfer. Preparation, rejected candidates, retired state and file IO
  are reclaimed on the control side.

Meaningful regressions include saved Rack parameter values on the first render,
live values surviving recompilation, Delay history surviving ProcessorId
migration, changed IO/configuration/bypass preventing inappropriate reuse, and
Recorder queues surviving unrelated edits.

Structural edits can replace selected prepared parameter values while retaining
kernel history. Newer live intent invalidates a preparing or published initial
override so that a running ramp remains authoritative. Stale candidate rejection
does not block the replacement preparation. Spectrum FFT-size changes and level
port/width changes reserve separate topics; public subscriptions select the new
shape only after activation. Retired shapes are reclaimed after their leases.

Signal shape updates need enough topic and memory capacity for both active and
candidate shapes. Subsequent preparation waits for cleanup to make capacity
available, which can add update latency. Existing subscriptions to an old shape
close after retirement; consumers subscribe again to receive the new shape.

## Managed recording files

The managed Recorder reader can lease its sole SPSC consumer to a WAV worker.
The claim happens before opening a path, so a busy or stale handle cannot
truncate another recording's destination. Writer creation and file IO hold no
host directory or reader slot lock. An unrelated compatible edit retains the
lease; Recorder replacement/removal or owner shutdown revokes it. A revoked
worker stops capture, waits for any accepted in-flight block, drains and
finalizes independently of host shutdown. Reader ownership returns only to the
same endpoint, not a replacement queue with the same node ID.

`RecordingSession::stop` requests shutdown. `finish` joins and reports final
written-frame counts, transport statistics and IO/finalization errors. Status
exposes progress and terminal errors; terminal publication follows reader lease
release. Dropping a session also stops and joins, but does not report errors.
The graph's processing sample rate must be passed to the writer, rather than
the physical device mix rate. Buffered frames predating the file claim are
included. Slow storage still causes bounded whole-block drops, reported by the
Recorder counters, and can delay file-worker shutdown without blocking audio
or the host's coordinator. Recording session operations belong off RT.

## Verification

Run from the integration worktree:

```powershell
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets --target-dir target-focused
cargo test --locked --workspace --target-dir target-focused
cargo clippy --locked --workspace --all-targets --target-dir target-focused -- -D warnings
```

Tests do not open audio devices by default. Hardware tests remain separate and
ignored. A successful portable suite does not qualify a specific audio device,
latency profile or driver for continuous realtime use.

Local verification on 2026-10-17 passed formatting, all-target workspace checking
and strict all-target workspace Clippy. The final workspace rerun passed 952
tests, with three hardware tests ignored and three long clock tests filtered.
Those three tests had passed earlier in this same run; no Windows clock
algorithm changed afterward. This rerun includes managed recording and Windows
format/period admission changes, with the log at
`target-focused/backend-recording-workspace.log`. The filtered tests are:

- `clock_control_prepared_low_inventory_handles_rate_phase_order_and_lateness`
- `clock_faults_joint_queue_handles_bursts_stalls_and_pause_recovery`
- `clock_startup_joint_queue_matrix_handles_staggered_owners`

The final engine suites include 111 library tests, 40 compiler tests, 74 host
tests and 60 runtime tests. Render allocation regressions check allocation,
reallocation and deallocation separately. No hardware tests ran in this delivery.
The recording regressions cover an explicitly gated in-flight block, data and
finalization failures, source revocation, exclusive claims, file completion and
host shutdown while a writer is blocked. Two real-filesystem host tests are
ignored under Miri isolation; the in-memory concurrency tests remain enabled.
Miri itself was not executed locally for this increment.

## Remaining platform and product work

The managed Windows backend already provides WASAPI Shared input/render,
process-tree PassiveTap capture, multiple IO nodes, a master output, bounded
follower/capture clock bridges, compatible updates, cancellation, resource
waiting and owner replacement. Managed native IO remains f32 stereo at a 48 kHz
processing rate. This limitation does not restrict intermediate portable DSP
ports to stereo. Conventional mono/stereo capture and front-stereo render
PCM/float mix formats can use Windows shared conversion while retaining their
channel count. Surround capture, nonstereo render and nonconventional explicit
speaker masks are rejected before initialization, rather than being implicitly
mapped or downmixed outside the graph. Catalog admission uses the same portable
format policy. Device initialization determines actual format support.
An explicit `period_frames` request must include `hardware_sample_rate`, so
endpoint frame units are never silently interpreted at an assumed 48 kHz.
Omitting the frame request retains the configured latency-profile policy.

The following roadmap work is still outstanding:

- Production Exclusive, ASIO, native multichannel IO and virtual audio drivers.
- Application Redirect/Takeover; passive process capture does not suppress the
  application's original playback.
- CLAP/VST3 hosting, plugin multi-bus/sidechain kernels and crash isolation.
- Professional SRC, output latency compensation and physical end-to-end latency
  measurement. Current cross-clock interpolation is a functional baseline.
- Standard LUFS/True Peak, a multiband EQ/filter bank and advanced analysis.
- Named Pipe/remote control.
- Broader hardware qualification: concurrent physical/process inputs, device
  removal, sleep/resume, pressure tests and long-running driver combinations.

Historical hardware evidence in the managed-routing and low-latency validation
records includes clock recovery and dropped frames. It does not establish
continuous XRUN-free operation. Existing exclusive probes in other worktrees
are experiments and are not part of this backend node delivery.
