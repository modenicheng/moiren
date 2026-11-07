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

## First native indexed-v2 acquisition

The frozen integrated revision `74f173f` includes main through PR50 at
`c47035c`. Its release probe is7087616 bytes, SHA256
`855283181C2AB80B027E8F96351FEA5F8E35B2FF36DC840F7EC9AAF0CE470B89`.
The installed driver remains `2a5aee5` /609; no installation or default change
is part of this acquisition. The exact route is virtual speaker/Playback
(Moiren Input/Source) -> unity Bus -> virtual microphone/MIC (Output/Sink).
There is no physical render endpoint in this run.

The bounded runner keeps Engine32, target1024/capacity4800/jitter96,
declared packet/gap480, recovery cap8 and correctness3000ms. Source uses its
default period; MIC alone requests the queried minimum. Only source schedule
identity and version change from the preceding MIC-only control. The exact
schedule has29 original indices,1024-frame markers at amplitude0.025 and
1440000 source frames. Source schedule SHA256 is
`17BBB2605B59D9D3C9C82642364C609E4C598C9840EDD56B81A86AD58B71FECC`.

Independent review finds that PowerShell collection comparison can accept
empty/singleton array provenance. The runner first reproduces this RED, then
requires scalar strings for all four report/collection version and schedule-ID
fields. Exact PS5.1 provenance fixtures pass63/63 and retained period fixtures
42/42. The archived runner SHA256 is
`9D2905C3CF932638602FA61574E79544BB8815F7E38598AC8354A2C54FE2EE1D`.
This repairs evidence admission, not driver/runtime behavior or quality gates.

Actual collection runs UTC04:50:43.273219 through04:51:17.331900 on the physical
target. It negotiates Playback480/MIC144, submits the complete schedule, stores
10108 packets of144 frames (1455552 frames /11644416 PCM bytes), and completes
collection, native Stop/join and resource cleanup with exit0. Source submission
acknowledgment is not by itself proof that every marker was recorded.

Packet metadata has flags0, contiguous stored positions/sequences and no
reported dropped/gap/discontinuity/timestamp-error frames. The default native
delivery report has zero gap observations; its zero maxima are not a measured
zero-gap guarantee. A direct PCM scan finds no nonfinite/clipped samples,
62944 nonzero scalars and peak0.0493401997; stereo is bit-identical. Threshold
regions are morphology only, not decoded identities or source-clock alignment.
No whole-recording marker identity analysis is claimed by this receipt.

Quality explicitly remains incomplete. Playback retains32 startup-prime muted
frames, zero classified steady delta and15 unknown observations. Both active
provider snapshots have zero drop/silence/reset/recovery counters. The final
Removed MIC snapshot again has silence_packets1, while separate telemetry
silence remains0; both cleanup telemetries retain epoch_resets1/recovery_events1.
The silence increment's inclusion in retained PCM is unknown. Temporal proximity
to gate closure does not justify exempting it. `quality_passed=true` cannot
override `quality_complete=false`.

Before/after volume/mute/default roles and boot match; no owner or VAD child
remains. Archive `indexed-v2-1-evidence.zip` is1232058 bytes,22 payloads plus
manifest, SHA256
`45054FA026F26B5D15E55DB9E8780809B9B0E9CC9C9BAB09D5AB959D83869B60`.
Root verifies CRC, exact members and every original/extracted length/SHA256.
Independent audit passes64 checks with0 failed; audit JSON SHA256 is
`9759B50BF42F6A04F32A434F70A7C24C12486577120FF8BE4DEC3B5E3ACC1AF4`.
Host evidence root is `D:\coding\moiren-evidence\20261031-main74f-source-v2`.
The original artifacts and failed earlier controls remain retained. This is
operational acquisition evidence, not continuous waveform, calibrated physical
latency, a lower-default decision or S1-S3 completion.
