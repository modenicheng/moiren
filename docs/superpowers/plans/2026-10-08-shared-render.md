# First WASAPI Shared Render Implementation Plan

> Execution record: implemented directly on main under the user's existing authorization; independent requesting-code-review review and recheck completed.

**Goal:** Run a compiler-prepared tone graph through one explicitly selected WASAPI Shared output.

**Architecture:** Cross-platform DemandRenderer converts bounded Engine renders through a same-owner output bridge. Windows owner prepares native 48 kHz stereo f32, fills staging before buffer leases and supports independent cancellation. The app prepares the tone graph and reports after stop/release.

**Tech Stack:** Rust 2024, existing Compiler/Engine/rtrb, windows 0.62.2, serde; no additional third-party dependencies.

## Global Constraints

- 用户授权直接在 main 工作，不创建新开发分支/worktree。
- 单 Graph RT owner；固定 48000 Hz、2 channels、32-bit float。
- RT 成功循环不分配/释放或格式化日志；COM 与 kernel handles 在 owner 完整释放。
- 显式 endpoint；运行时间 1..600 秒；不改系统默认设备或其他 session 设置。
- 用户 Compressor 草稿按用户明确选择协调，其他改动不包含该草稿。

### Task 1: Demand and tone path

**Files:** Create `crates/moiren-windows-audio/src/render.rs`, `tests/render.rs`, `crates/moiren-app/src/tone.rs`, `crates/moiren-app/tests/tone.rs`; modify both lib.rs exports and Windows audio Cargo.toml.

**Interfaces:**

```rust
DemandRenderer::new(engine: Engine<f32>, output: AudioReader<f32>) -> Result<Self, RenderError>;
DemandRenderer::render_interleaved(&mut self, output: &mut [f32]) -> Result<DemandReport, RenderError>;
writable_frames(capacity: u32, padding: u32) -> Result<u32, RenderError>;
prepare_tone(config: ToneConfig) -> Result<ToneSession, ToneError>;
```

- [x] Add tests expecting silence/no timeline advance for zero demand; 19 frames at max block 8 require 3 blocks and contiguous signal. Run `cargo test --locked -p moiren-windows-audio --test render` and observe missing API before implementation.
- [x] Implement demand validation, complete output initialization, block splitting and same-owner bridge reads. Export EngineConfig through an Engine getter to validate the sample rate and max block.
- [x] Add tone graph/sample tests before implementing sine source and compiler preparation. Use expected samples `0.05 * sin(2π * 440 * frame / 48000)` and current stereo balance; compare across `[3, 1, 8, 7]` demands.
- [x] Add counting allocator test around render calls only; require 0 allocations and 0 deallocations.

### Task 2: WASAPI owner

**Files:** Create `crates/moiren-windows-audio/src/render/wasapi.rs`; reuse existing owner helpers without changing W00 semantics.

**Interfaces:**

```rust
start_render(options: RenderOptions, renderer: DemandRenderer) -> Result<RenderSession, RenderError>;
RenderSession::request_stop(&self) -> Result<(), RenderError>;
RenderSession::join(self) -> Result<RenderReport, RenderError>;
```

- [x] Add unit tests for malformed native format and stop wake with an unsignaled fake audio event; no endpoint activation in cargo test.
- [x] Implement pinned endpoint lookup/flow check, native mix validation, EVENTCALLBACK/NOPERSIST client initialization, capacity/latency/period inspection, optional own-session ducking preference and MMCSS.
- [x] Implement prefill and variable padding demand with preallocated staging, paired GetBuffer/ReleaseBuffer and RAII cancellation on abandoned lease.
- [x] Implement stop/audio wait set, bounded duration, scalar report and cleanup on start/stream/stop failure; serialize diagnostics only after cleanup.
- [x] Run Windows all-targets check/test/strict Clippy; pure helpers remain buildable without Windows.

### Task 3: CLI, docs and hardware slice

**Files:** Modify `crates/moiren-app/src/main.rs`, `crates/moiren-app/Cargo.toml`, both crate READMEs, root README, PRD and Windows integration record.

**Interfaces:** CLI consumes ToneSession and moves only its Engine/output to DemandRenderer; ControlPort and logical bindings remain on the app/control side.

- [x] Add `render --list` and explicit endpoint/time/frequency/gain/pan parsing, preserving the offline CLI. Reject missing/duplicate/unknown arguments and non-Windows render with clear reasons.
- [x] Run workspace tests, check, Clippy, fmt, offline/logical_graph/app examples and `git diff --check`; record exact counts and any user-edit constraints.
- [x] Once the user selects a device, run the bounded 10-second 440 Hz/gain 0.05 smoke and inspect stage/format/submission/shortfall/stop fields; request the user's listening observation without treating silence or API success as audible proof.
- [x] Record implemented scope and remaining capture/clock/swap tasks; commit only this slice and explicitly authorized integration changes on main.

## Implementation and verification results

- Compiler commit `2fb754e` fast-forward merged into local main; this slice developed directly there, without another branch/worktree.
- User chose to disable the incomplete Compressor module while retaining its draft; user-authored comment/draft edits remain outside this feature commit.
- Windows owner is the only Graph executor. Native 48 kHz/stereo/f32, staging <= 8 MiB, default Engine max block 256; unsupported formats fail before Start. No format conversion or cross-clock bridge.
- Windows local workspace: 109 tests passed, including one compile-fail doctest; strict Clippy, all-targets check, fmt, offline/app/logical_graph and diff whitespace checks passed. Linux/Miri and remote CI were not run locally.
- Pure demand tests observed expected missing APIs before implementation. Added no-allocation/free checks, first/later bridge failure accounting, prior Engine work exclusion, finite tone samples/control and CLI rejection tests. Fake COM tests verify full PCM copy, empty-demand skip and zero-frame cancellation; separate stop event works without audio wake.
- Independent review found and fixed render list's dependency on unrelated capture/default lookup and lost DSP counts on failed demand. Counter updates now precede bridge transfer; owner reports session timeline delta. Recheck approved with no blocking findings.
- Explicitly selected FreeDSP headphones; 10 s / 440 Hz / gain 0.05 / pan 0. Submitted and processed 481,536 frames (including 1,056 prime), 2,007 DSP blocks, 1,001 audio wakes; bridge shortfall/empty padding/timeout all 0, Stop succeeded, no stream API failure. User replied “听到了”.
- Before/after: 8 endpoints and 13 original sessions compared; no observed defaults/volume/mute differences. Two protected-process identity queries returned 0x80070005 in both snapshots, while volume/mute was readable; errors retained. [Experiment record](../../experiments/windows/2026-10-08-shared-render.md) contains limits and scalar JSON.
- Next slice: single physical capture with bounded Clock Bridge/SRC, then process loopback; Plan Swap/lifecycle and GUI follow. W06 complete stress/restart/recovery acceptance and M0.5 as a whole remain open. CLI Ctrl+C graceful stopping is not yet implemented.
