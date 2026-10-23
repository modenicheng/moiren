# Code-quality worklist: driver/vad/src + moiren-windows-audio (2026-10-24)

Produced by a read-only survey (duplicates with verified drift, comment
hygiene). Items marked **done** landed in the 2026-10-24 cleanup commit;
the rest are ranked for later passes. File:line refers to the surveyed
revision and may drift.

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
- **A1 (next, highest value)** WASAPI capture packet consumption exists in
  five copies whose byte-width calculation has already drifted apart
  (checked-mul frame_bytes vs `nBlockAlign` vs hardcoded stereo-f32 `*8`
  vs `codec.frame_bytes()`): capture/wasapi/packet.rs (production),
  probe/capture.rs, exclusive_probe/capture.rs, physical/stream/
  sampling.rs, professional/wasapi.rs. Extract a shared
  `BorrowedCapturePacket`-style helper (frame width + error mapping as
  parameters) so probes inherit the production overflow checks.
- **A3 (driver, needs WDK rebuild + host suites)** `Clock.h` mirrors
  `MOIREN_VAD_SAMPLE_RATE`/`MOIREN_VAD_MAX_PACKET_FRAMES` from
  include/moiren_vad.h; `Formats.cpp` hand-writes the IEEE_FLOAT GUID and
  format tag that the SDK exports; `Audio.h`/`Policy.cpp` duplicate the
  `L"MoirenEndpoint"` reference string. Single-source from the ABI header
  / SDK constants in one pass.
- **A4** process-loopback Initialize flag combination + SetEventHandle is
  copied in process_loopback/stream.rs, probe/capture.rs,
  exclusive_probe/capture.rs — extract into the process_loopback module.
- **A5** exclusive-mode negotiation trio (`supported`, exclusive
  Initialize, alignment-retry formula) duplicated between
  exclusive_probe/render.rs and professional/wasapi.rs; `frames_duration`
  already exists on one side only — promote to negotiation.rs.
- **A6/A7/A13** owner teardown/finish and startup-handshake skeletons are
  capture/render twins with light drift; worth a shared helper when either
  side is next touched, not as a standalone churn commit.
- **A8-A12, A15** small module-level twins (`api()`, `From<StageError>`,
  tri-handle wait, Client3 period query, OPEN_STAGES table, endpoint
  projections) — merge opportunistically.
- **A16 (driver)** six `ClearTimerPacket`+`ZeroPackets` quiesce micro-
  blocks in Streams.cpp with per-site field differences; extract named
  helper(s) so a future reset site cannot miss a field.

## Comments

- **done (B3)** physical/stream.rs referenced QQMusic and "this machine"
  in comments; rewritten product/hardware-agnostic.
- **B2** the WASAPI SILENT-flag contract is explained in three wordings;
  collapse into the A1 helper's doc comment when extracted.
- **B4** the OPEN_STAGES header comment is duplicated verbatim; collapses
  with A12.
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
