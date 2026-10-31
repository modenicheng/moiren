# Bounded provider READ capture plan

Date: 2026-10-31. This is a diagnostic implementation plan after the
[independent Process control](2026-10-31-ms-process-reference.md), not a
waveform, stable-buffer or physical-latency acceptance receipt.

Implementation and one bounded native run are now retained in the
[provider input receipt](2026-10-31-provider-input-capture.md). The checklist
below remains the original planned contract, not an assertion that every
test/acceptance item passed; the receipt explicitly lists missing automated
same-instance/error-path coverage and the all-zero waveform observation.

## Decision and boundaries

Observe the real provider's existing READ completion before decode/carrier
publication. Do not add another READ owner: it would consume packets belonging
to the running graph. Do not add another Process format variant, which would
repeat the unresolved observation. A new endpoint-loopback implementation
would add another client and conversion boundary before answering whether
Moiren already receives the submitted PCM.

Virtual speaker/Playback remains Moiren Input/Source; virtual MIC remains
Moiren Output/Sink. Kernel ABI, audio data, routing, transport admission,
buffer defaults, periods and recovery policy remain unchanged. The existing
holder's Gain0.5 fixture remains diagnostic, explicitly not exact S1.

## Implementation slice

- [ ] In an isolated worktree, fast-forward the reviewed diagnostic baseline;
  preserve unrelated work and use atomic local commits without pushing.
- [ ] Add an explicit preallocated raw-input recorder under
  `crates/moiren-windows-audio/src/vad/provider/`, bounded to480000frames,
  no more than4MiB raw PCM and a finite packet-record array. Omitted opt-in
  must allocate no artifact and perform no additional sample scan.
- [ ] Copy only after epoch/admission/order validation and before the existing
  `bridge.decode_packet(pcm)`. Preserve actual bytes, PCM format/channels,
  endpoint/native generation, stream/gate/reset identity, revision, packet
  sequence/QPC, stored offset/frame count and carrier-publication outcome.
  Pre-first-packet startup transitions are not captured generations. Once
  capture starts, seal on identity change or capacity; do not stitch epochs.
- [ ] Keep ownership on the actual IO worker. No packet-path filesystem IO,
  JSON, blocking handoff, growing storage or competing native request. Retain
  partial data on early error; empty capture is unknown, never confirmed
  silence. A diagnostic capacity boundary is not a dropped-audio counter.
- [ ] Wire a minimal explicit same-provider factory/handle through
  `crates/moiren-app/src/service/managed.rs`. The holder must not discover a
  second control owner. Add opt-in `--input-capture-frames` only to
  `crates/moiren-app/examples/vad_integration`; normal desktop defaults/help
  retain their existing behavior. Transfer the take-once artifact only after
  the relevant worker/service join; cancellation alone is insufficient.
- [ ] Write raw bytes plus bounded metadata into the holder's fresh output
  directory on the control side after join. Preserve error/incomplete
  observations and original process/report failure. Hash and inspect PCM on
  the development host; do not reinterpret a saved artifact as acceptance.

## Test and review gates

- [ ] Retain real pump/FakeIo RED before implementation, then GREEN for raw
  PCM16/Float32 identity, all-zero/nonfinite preservation, unchanged carrier,
  completion order, stale exclusion, reset/sealing, capacity/malformed packet,
  carrier refusal and early failure artifact retention.
- [ ] Test opt-in parsing, no-driver help/default behavior, same-provider
  ownership, take-once/post-join access and fresh-output persistence.
- [ ] Run focused tests, strict Clippy and formatting with shared caches and
  two jobs. Do not clear caches or run target builds during audio collection.
  Check the same modified portable surfaces on Linux before integration.
- [ ] Independently review actual packet hook and owner lifecycle, not only
  recorder helper tests. Integrate only the scoped reviewed commits.

## Native discriminator

- [ ] Deploy only the changed holder binary side-by-side and verify SHA-256;
  installed driver and source worker remain unchanged. Refuse preexisting
  audio/build owners or mismatched target/root/hash; preserve their processes.
- [ ] Use one existing holder and an exact identity-selected Shared Playback
  source. Retain its changing schedule, PID/creation, ready/Running/final
  receipts and bounded Stop/join. Do not start another Process recorder,
  physical render client or endpoint setting operation for this control.
- [ ] Verify source schedule overlap and inspect every saved packet's shape,
  offsets, identity, sequence and publication result. Match actual raw PCM
  against the intended waveform before calling it source recovery. Zero,
  only-LSB, empty and clipped/incomplete artifacts have different verdicts.
- [ ] Retain clean-or-failed teardown, driver/boot/root/process audit, archive
  memberwise length/hash verification and independently downloaded originals.

Normal-amplitude matching READ PCM would localize subsequent faults after
this boundary without proving physical output. Zero/only-LSB READ PCM would
place the unresolved observation at or before this boundary without proving
a sole driver/Windows/source cause. No data or identity-mixed capture remains
inconclusive. S1-S3 and calibrated physical p95<=10ms stay open until their
separate acceptance gates pass.
