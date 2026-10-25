# Code-quality worklist: driver/vad/src + moiren-windows-audio (2026-10-24)

Produced by a read-only survey (duplicates with verified drift, comment
hygiene). Items marked **done** landed in the 2026-10-24 cleanup commit
or the 2026-10-25 quality pass (dated entries); the rest are ranked for
later passes. File:line refers to the surveyed revision and may drift.

## Duplicates

- **done (A2)** MMCSS registration was inlined at five sites
  (capture/wasapi/stream.rs, render/wasapi/stream.rs, physical/stream.rs,
  probe/capture.rs, exclusive_probe/render.rs) with three reporting
  styles. All now call `Mmcss::audio_detailed()`; reports keep their own
  consumption of the failure HRESULT.
- **done (A14/B1)** The engine 48 kHz family had four names plus bare
  literals (`render::SAMPLE_RATE`, `clock_bridge::OUTPUT_SAMPLE_RATE`,
  `virtual_owner::boundary::NATIVE_SAMPLE_RATE`, and literals in
  preparation/process_loopback/exclusive_probe/clock_bridge ingress).
  Single-sourced as `crate::NATIVE_SAMPLE_RATE`; `stats::SILENT` and
  siblings stay literal (windows crate is target-gated) but are pinned to
  the SDK contract values by a cfg(windows) test.
  The VAD ABI mirror (`vad/protocol.rs`) is deliberately separate: it is
  pinned to the kernel header by the abi_conformance test. Do not merge.
- **done (A1)** WASAPI capture packet consumption existed in
  five copies whose byte-width calculation has already drifted apart
  (checked-mul frame_bytes vs `nBlockAlign` vs hardcoded stereo-f32 `*8`
  vs `codec.frame_bytes()`): capture/wasapi/packet.rs (production),
  probe/capture.rs, exclusive_probe/capture.rs, physical/stream/
  sampling.rs, professional/wasapi.rs. All now call
  `capture_packet::borrow_capture_packet`, which checks frame capacity,
  byte-length overflow, null data, SILENT, and zero-frame behavior using
  each format's frame width. Callers keep their own error mapping, packet
  lease, and `ReleaseBuffer` ordering.
- **done (2026-10-25, A3)** `Clock.h` now includes `include/moiren_vad.h`
  (still WDF-independent) and derives `Rate`/`MaxPacketFrames` from
  `MOIREN_VAD_SAMPLE_RATE`/`MOIREN_VAD_MAX_PACKET_FRAMES`.
  `Formats.cpp` uses the SDK's `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT` and
  `WAVE_FORMAT_IEEE_FLOAT`; reaching them needs `mmreg.h` (ksmedia.h
  gates the IEEE constants on `_INC_MMREG` and the kernel include graph
  never pulls it in), so `audio/Private.h` and `Guids.cpp` include
  `mmreg.h` after `ks.h` with `NOBITMAP` to skip its wingdi block.
  `VadAudioReference` moved to `Driver.h` as the one published circuit
  name / interface reference string; `Policy.cpp` now uses it.
  Validated by the portable host suites, the WDK build with code
  analysis (0 warnings) and InfVerif.
- **done (A4)** Process Loopback Shared initialization now gets its
  Loopback/EventCallback/AutoConvertPCM/default-quality-SRC flag combination
  from `process_loopback_stream_flags()` in the process_loopback module. The
  production owner, probe capture, and exclusive probe capture all use it;
  each caller still owns its event handle, COM client, stage reporting, and
  error mapping.
- **done (A5)** exclusive-mode negotiation now shares the raw
  `exclusive_format_supported`, event-callback `initialize_exclusive`, and
  `exclusive_frames_duration` helpers in `negotiation.rs`. The professional
  backend keeps its `BackendError` stage mapping and period policy; the
  exclusive probe keeps its Windows error/reporting behavior and client
  reactivation after `BUFFER_SIZE_NOT_ALIGNED`.
- **A6/A7/A13** owner teardown/finish and startup-handshake skeletons are
  capture/render twins with light drift; worth a shared helper when either
  side is next touched, not as a standalone churn commit.
- **done (A12/B4)** The capture and render `OPEN_STAGES` tables now come
  from direction-specific `SharedOpenStages` constructors in
  `negotiation.rs`; observable stage labels remain unchanged and the
  duplicated header comment is gone.
- **done (A11)** Capture, render and activation-gate waits now share the
  owner-level `wait_for_handles` kernel. Each caller retains its own wake
  or activation mapping and observable error stage.
- **done (2026-10-25, A10)** the Client3 current-period query is one
  `owner::current_shared_mode_engine_period` primitive returning the
  period plus the TaskMemory-owned engine format; shared_stream's
  `current_period`/`record_current_period` and the physical probe use it.
- **done (2026-10-25, A15, partial)** `negotiation::endpoint_data_flow`
  (probe-tools gated, like its only callers) merges the cast+GetDataFlow
  projection at physical/stream.rs and exclusive_probe/render.rs. The
  negotiation/professional sites keep their own cast and query calls
  because their per-stage labels (`IMMEndpoint` vs `GetDataFlow`) are
  observable reporting, not drift.
- **A8/A9 — evaluated 2026-10-25, intentionally left.** capture/render
  `api()` are already one-line delegations to `negotiation::stage`, and
  the per-type `From<StageError>` impls are inherent to each error enum;
  merging them would only add macro indirection. `GateError` is a public
  enum and cannot reference the `pub(crate)` StageError.
- **done (2026-10-25, A16)** Streams.cpp now has a `QuiesceStreamBuffers`
  helper (ClearTimerPacket + RenderReadyMask + DeliveredPackets +
  LastCaptureStart + ZeroPackets) used by the three TimerDrivenTick exits,
  the stale-generation timer exit and VadAudioClearGateBuffers. The
  pause/resume pair (VadAudioStopLocked with reset=false and
  VadAudioStreamRun) deliberately preserves LastCaptureStart so a packet
  completed before a pause still reports its capture time; both sites say
  so in comments.

## Comments

- **done (B3)** physical/stream.rs referenced QQMusic and "this machine"
  in comments; rewritten product/hardware-agnostic.
- **done (2026-10-25, B2)** The WASAPI SILENT-flag contract is documented
  once on `capture_packet::borrow_capture_packet`; `stats::analyze_f32`,
  `clock_bridge` ingress and the exclusive-probe render fill now
  reference it instead of re-explaining it.
- **done (B4)** The duplicated OPEN_STAGES header comment was removed when
  the direction-specific constructors landed with A12.
- No commented-out code blocks, no mixed-language, no stale comments
  contradicting code were found in either tree.

## Stability-audit follow-ups (not defects)

From the 2026-10-24 parallel audits (see docs/vad-validation-2026-10-21.md):
kernel core and PCM path audited sound; suggested target evidence for a
later pass: root surprise-removal while an owner is active, Verifier
stress across owner-close × child-cleanup × DPC transfer, power
transitions, and watching `pcm_retire_park_full` / quota-deferral debug
events during soaks. The user-mode final drains (PcmOwner::Drop, Control
Handle::drop) remain INFINITE by design plus unbounded join — a
shutdown-resilience decision to revisit if hangs are ever observed.
