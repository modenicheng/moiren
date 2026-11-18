# moiren-windows-audio

Crate guide. Read [../../AGENTS.md](../../AGENTS.md) first; this file adds local detail and never overrides the root rules.

## Role

The professional Windows audio backend and its probes: WASAPI Shared/Exclusive and ASIO, engine-backed capture/render (`engine_backend`), process loopback, VAD support, clock adaptation, and hardware observation. The canonical boundary and thread model live in [README.md](README.md) and [docs/platform/windows-audio-threads.md](../../docs/platform/windows-audio-threads.md) — read the thread map before touching any callback path.

Formal native scope today: WASAPI Shared, stereo IO, 48 kHz output. Mono/stereo PCM/float formats are converted by Windows, while multichannel and unusual channel masks are rejected; intermediate DSP widths are independent of that native limit.

## Layout

- `src/engine_backend.rs` plus `src/engine_backend/`, `src/wasapi.rs`, `src/capture.rs`, `src/negotiation.rs`, `src/format.rs`, `src/catalog.rs`, `src/native.rs`: `WindowsBackend`/`ProfessionalBackend`, native owners, endpoint catalog, format negotiation, and engine-backed capture/render.
- `src/asio.rs`, `src/asio_rt_tests.rs`: ASIO capability probing and callback ownership. The `asio_rt_tests.rs` and `engine_backend/recovery_host_tests.rs` files are compiled only by `tools/rt-check`, not by this crate's test target.
- `src/clock.rs`, `src/clock_bridge.rs`, `src/bridge.rs`, `src/servo.rs`, `tests/clock_*.rs`: master-clock selection, bounded clock bridges, drift/servo behavior.
- `src/vad/`, `src/virtual_owner/`, `src/process_loopback.rs`, `src/session.rs`: VAD integration, virtual-device ownership, app loopback, and the activation gate.
- `src/probe.rs`, `src/exclusive_probe.rs`, `src/physical.rs`, `src/swdevice.rs`: example-only COM/WASAPI/SWDevice probe tooling, gated by `probe-tools` and never part of default library builds.
- `src/latency.rs`, `src/stats.rs`, `src/capture_packet.rs`, `src/codec.rs`, `src/resample.rs`: bounded latency telemetry, statistics, packet metadata, format conversion, and windowed-sinc SRC.

## Contracts

- Real-time callbacks do not allocate, lock, block, rebuild COM, or call into control-side code. COM objects stay on their native owner threads; preparation/negotiation/destruction stay off the audio thread. Document non-obvious unsafe and callback lifetimes.
- Keep the native interface contract narrow and explicit: do not silently widen channel counts/masks or promise end-to-end latency, XRUN-free operation, or multi-device compensation that is not measured.
- Probes must never change the system default device or volume, and their results are limited to the tested machine/app/format/duration. Hardware acceptance evidence belongs in `docs/validation/` or `docs/experiments/`, separate from portable tests.
- The C++ ACX/KMDF VAD driver is a separate project under [driver/vad](../../driver/vad); do not assume a driver is installed, and keep the Rust ABI side (`src/vad/`, [driver/vad/ABI.md](../../driver/vad/ABI.md)) in sync.

## Verification

```sh
cargo test --locked -p moiren-windows-audio
cargo test --locked -p moiren-windows-audio --features vad-probe --example vad_lifecycle --example vad_native --example vad_active_lifecycle
cargo build --locked -p moiren-windows-audio --examples --features probe-tools
```

Default tests are portable: they use locally implemented COM objects, kernel events, and pure statistics — they never open a real audio device. Device-touching runs are opt-in examples (`w00_*`, `exclusive_probe`, ASIO acceptance), and `tests/asio_acceptance.rs` is gated to Windows x64 hardware. CI's Miri matrix runs only `--lib vad::` here; native RT contracts are shadow-compiled by `tools/rt-check`.

## Docs

- [README.md](README.md) is canonical for backend behavior and hardware evidence.
- [docs/platform/windows-audio-threads.md](../../docs/platform/windows-audio-threads.md), [docs/validation/backend-nodes.md](../../docs/validation/backend-nodes.md), [docs/validation/asio-v1.md](../../docs/validation/asio-v1.md), [docs/designs/14-managed-routing-low-latency.md](../../docs/designs/14-managed-routing-low-latency.md), [driver/README.md](../../driver/README.md).
