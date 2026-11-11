# Windows audio thread model

This is the canonical map of every OS thread that drives or observes audio in
Moiren, what each may do, and where its authority lives. Per-module realtime
contracts (`asio/callback.rs`, `vad/pcm.rs`, `vad/pcm_bridge.rs`, `servo.rs`,
`resample.rs`, `native.rs`, `bridge.rs`, …) stay in their own module headers;
this page links them instead of restating them. The workspace rule they all
implement is the one in `AGENTS.md`: prepare graphs, buffers and signal
bindings off the audio thread; realtime processing must avoid heap
allocation/deallocation and blocking; reclaim retired plans on the control side.

Thread names below are the literal `thread::Builder` names — they appear in
debuggers and ETW traces.

## Realtime audio threads

These are the only threads that touch native audio streams while a graph is
active. Each registers in the MMCSS `Audio` class for its lifetime
(`win32_raii::Mmcss`; registration failure is non-fatal and only costs
scheduling priority). Inside the active phase — after native start, before
stop — the rules for all of them are: no heap allocation or deallocation, no
locks or blocking calls other than the single documented wait, no COM/registry
work beyond buffer IO, no logging. Startup and teardown (negotiation, buffer
allocation, stream start/stop, diagnostics) run outside that phase on the same
thread and may allocate.

| Thread | Spawned in | Responsibility |
|---|---|---|
| `moiren-professional-master` | `engine_backend/owner.rs` `spawn` | Master native owner (WASAPI shared/exclusive or ASIO). Runs `Pump::cycle` — bridge ring refill, SRC, follower servo — and **`Engine::render` on this thread**; then `stream.wait`, the loop's only blocking call. |
| `moiren-professional-device` | `engine_backend/owner.rs` `spawn` | Additional non-master native owners. Same loop, never borrows `Engine`: the follower pump renders silence resampled and servo-paced to the device rate. |
| ASIO driver callback thread | the third-party ASIO driver | Not created by Moiren. The code that runs on it is `asio/callback.rs`. |
| `moiren-virtual-owner` | `virtual_owner/owner.rs` | Virtual transport owner for software-only graphs. Paces `Engine::render` against the virtual realtime clock (`virtual_owner/pacer.rs`, high-resolution waitable timer). |
| `moiren-vad-pcm` | `vad/provider.rs` `VadSession::spawn` | One worker per VAD device session. Overlapped PCM IO against the VAD driver plus fixed-size PCM conversion; feeds the RT boundary objects. |
| `moiren-shared-capture` | `capture/wasapi.rs` `start_owner` | Standalone WASAPI shared capture session worker: the CLI capture path and process-loopback capture (the `ProcessOwner` auxiliary in `engine_backend.rs`). Publishes clock-stamped frames to consumers. |

Notes per thread:

- **Master owner.** The master borrows `Engine` from `EngineSlot` for the
  active phase and returns it through a control-only handoff before any
  worker-local drop — the unwind boundary documented at the top of
  `engine_backend/owner.rs`. `NativeStream` (`native.rs`) has no `Send` bound:
  COM/driver objects stay on their opening thread, `wait` is the sole blocking
  operation, and transfers never wait for queue space. WASAPI shared mode is
  event-driven — `WasapiStream::wait` blocks on the audio engine's buffer
  event, then `read`/`write` drain or fill one period (`wasapi.rs`); the
  Windows audio engine owns the actual device interrupt, and our deadline is
  the negotiated period. Exclusive mode and ASIO pace device buffers directly
  from this thread.
- **ASIO callback.** The strictest rules in the crate live in the
  `asio/callback.rs` header: no driver calls, allocation, locks, conversion or
  reclamation on callbacks; SPSC rings in and out; a hand-declared `SetEvent`
  (avoiding `windows::core::Result` construction, which can allocate) wakes
  the owner; a global reader barrier closes the load/retire use-after-free
  race. `asio_rt_tests.rs` pins the zero-allocation behaviour (see
  "Shadow-compiled tests" below).
- **VAD PCM worker.** `vad/pcm.rs`: "Direct IO belongs to a PCM owner thread,
  never Engine render." `vad/pcm_bridge.rs`: "Fixed-size PCM conversion stays
  on transport workers, never Engine render." The worker writes into the
  virtual boundary (`virtual_owner/boundary.rs`) that Engine render reads.
- **Shared capture.** Mmcss registration in `capture/wasapi/stream.rs`.
  Capture older than its deadline is discarded rather than burst-played
  (`bridge.rs` backlog policy), so a stalled worker degrades position, never
  the render schedule.

## Control, preparation and observation threads

These threads never touch an active native stream. All allocation,
negotiation, compilation and reclamation is theirs.

| Thread | Spawned in | Responsibility |
|---|---|---|
| `moiren-host-control` | `moiren-engine` `host/supervisor.rs` | Supervisor state machine: sequences preparation, activation and retirement; polls `EngineBackend::poll_resource`; drains the engine signal bus (`signals.pump()`); publishes host snapshots. |
| `moiren-host-prepare` | `moiren-engine` `host/preparation.rs` | Compiles graphs and runs backend prepare — negotiation, buffer allocation, signal-binding setup. Everything `AGENTS.md` requires to stay off the audio thread happens here. |
| `moiren-host-cleanup` | `moiren-engine` `host/retirement.rs` | Reclaims retired plans (drops processors, engines and streams) so RT threads never deallocate. |
| `app-service` | `moiren-app` `service/mod.rs` | Application service loop: dispatches requests, runs the managed driver's poll (which reads `EngineObserver` and drives `MeterBridge` levels and `SpectrumAnalysis`), drains the app-side signal bus (`pump_if_due`), publishes snapshots to the UI mailbox, parks with bounded timeouts. |
| `app-prepare` / `app-cleanup` | `moiren-app` `service/jobs.rs` | Single-slot job lanes for catalog and service jobs. |
| `moiren-asio-capabilities` | `asio/capabilities.rs`, one per query | Dedicated COM owner thread that initializes, queries and releases a registered ASIO driver for capability enumeration. Never sets a rate, creates buffers, starts IO or publishes callbacks. |

Not threads, despite the names: `MeterBridge` and `SpectrumAnalysis`
(`moiren-app` `service/managed/meters.rs`) are polled state machines that run
**on the `app-service` thread**; `MeterPollingTimer` (`moiren-engine`
`meter/polling.rs`) is a control-side `timeBeginPeriod(1)` request — raised
and released from control threads only, never the audio thread.

Probe-only threads (the `probe-tools` feature, example binaries only):
`physical.rs` and `exclusive_probe/render.rs` spawn Mmcss-registered workers
for the opt-in probes. They are excluded from library builds by the feature
gates in `lib.rs` and carry no production obligations.

## Crossing points — what runs inside `Engine::render`

`Engine::render` executes on whichever realtime owner is the master: the
professional master owner (graphs with physical or process IO) or the virtual
owner (software-only graphs). Everything it calls inherits that thread's
rules:

- `bridge.rs` — `RingSource`/`RingSink`/`Tone`/`ProcessSource`/
  `CaptureWriteClock` move complete stereo frames across bounded SPSC rings
  with QPC feed-forward; allocation and backlog reclamation stay on the
  control side of the ring.
- `resample.rs` / `servo.rs` — pure kernels: no allocation, trigonometry or
  reclamation while moving samples.
- `virtual_owner/boundary.rs` — RT sources/sinks consumed by render, fed by
  the VAD PCM worker and virtual sessions.
- Signal publication — `SignalLevelMeter` (`moiren-engine` `meter/signal.rs`)
  is a graph processor and publishes Latest/Stream blocks during render.
  Publishing is wait-free: a full topic drops and counts
  (`moiren-signal` `transport`), it never blocks or wakes the RT thread. The
  bus is drained only by control threads (`signals.pump()` /
  `pump_if_due`), so no publication-side wakeup ever originates on a realtime
  thread.

## Shadow-compiled tests

`src/asio_rt_tests.rs` and `src/engine_backend/recovery_host_tests.rs` are not
part of any `moiren-windows-audio` build target. They are compiled and run
exclusively by the rt-check allocation harness
(`tools/rt-check/tests/professional_rt.rs` mounts them with `#[path]`), which
also shadow-compiles the owner and bridge modules above to assert their
private realtime paths allocate nothing. Run them with:

```sh
cargo test --locked --manifest-path tools/rt-check/Cargo.toml
```
