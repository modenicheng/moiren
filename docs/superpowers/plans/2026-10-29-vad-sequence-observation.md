# Playback native sequence gap observation plan

Continues authorized S1-S3 boundary observability and correctness-mode work.
No native audio, VM, deployment, driver build/install or default changes.

## Contract

Playback READ returns the native packet sequence stamped by the driver;
microphone completion instead echoes the host submission sequence. Count only
forward holes between accepted playback stamps within one reset epoch. The
first observed prefix is unknown. Stale/regressed packets do not advance the
accepted frontier. An explicit new epoch establishes a new frontier even if
its sequence/QPC restarts. Retain existing stale packet handling and microphone
regression refusal. A forward hole alone does not reset or stop either path.

Publish saturating gap-observation and missing-sequence-slot counters separately
from driver/quota/carrier drops. Missing slots do not prove non-silent signal
loss and must not be converted to frames or latency. Additive telemetry schema3
keeps old JSON aliases/defaults. Show exact integers and scope in diagnostics.
Use typed order outcomes; do not mix user/control epochs with native timing.

## Tasks

- [x] Add actual worker regressions; observe missing-counter failures before
  implementation. Cover forward gaps, stale stamps, unknown prefix, epoch
  change/race and microphone submission provenance.
- [x] Add epoch-scoped typed order outcomes with arithmetic boundary tests;
  wire only accepted playback holes to independent saturating telemetry.
- [x] Add test-first telemetry JSON/saturation and truthful desktop diagnostics.
- [x] Run focused/full hardware-free affected tests, strict Clippy, all-target
  check, fmt and independent review. Record evidence and remaining gates.
- [x] Commit coherent slices, push and verify remote SHA; preserve gate.log.

Implementation `df6ea7e` and diagnostics `011289b` were independently pushed
and remote-hash verified. Full commands, retained failing runs and open gates:
[`2026-10-29-vad-sequence-observation.md`](../../validation/2026-10-29-vad-sequence-observation.md).

This fixes an observability gap, not playback continuity or physical latency.
The complete S1-S3 objective stays active and hardware acceptance stays deferred.
