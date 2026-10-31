# Opt-in indexed source markers, 2026-10-31

## Scope and provenance

`ec2bbea` integrates independently developed `15934d2`. Only the latency probe's
`source.rs` and `live.rs` change. Existing schedules continue to select
`moiren-marker-xorshift32-v1`; their repeated fixed-seed marker bytes are
unchanged. The explicit version `moiren-marker-xorshift32-indexed-v2` selects a
different deterministic code for each original schedule index.

The prepared mono codebook is one continuous xorshift32 stream. Silence and
native packet boundaries do not consume generator state. Schedule index `i`
always selects segment `i`, including when an earlier marker is absent from a
recording. This defines source identity, not an algorithm for recovering that
identity after SRC, loss or drift.

Preparation checks sample/byte multiplication and reserves fallibly before
opening any native owner. The independent codebook cap is 4 MiB. A 29-marker,
1024-frame plan occupies 118784 bytes before stereo expansion into existing
native buffers. Failed ReleaseBuffer submissions do not advance acknowledged
source progress or marker identity.

## Verification

The author retains genuine unsupported-version RED, then GREEN evidence in
`.worktrees/marker-identity-v2/.tmp/marker-v2-evidence`. The author reports full
probe 553/553, strict example/tests Clippy, formatting and Linux-target tests
compilation. The root independently reviews the two-file diff and runs, after
integration, the seven version-focused tests and all 553 probe tests: all pass.
Root logs are retained under
`D:\coding\moiren-evidence\20261031-app-e1a7c2f\integrated-marker-v2-*.log`.

Tests cover fixed v1 golden bytes, exact v2 code segments, chunk/silence
independence, original indices, byte-cap boundaries, version/schema refusal,
failed native-release progress and exact dry-run schedule/provenance retention.

## Limits

No driver, production buffer/default, native period, recorder lifecycle or
analyzer changes occur. No new native recording is collected by this slice.
The source origin remains the first successfully released post-Start payload;
the recorder origin is not inferred or manufactured. Identical stereo channels
are not an independent reference. Neither detected-region counts nor sparse
markers certify continuous audio through the silent intervals. SRC identity
discrimination, trustworthy recorder mapping, continuous waveform quality and
calibrated physical latency qualification remain open. S1-S3 is not complete.
