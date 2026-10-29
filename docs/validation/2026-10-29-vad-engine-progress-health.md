# VAD probe lifecycle versus progress observation

Date: 2026-10-29. Source repair: `f72e03c`. Target:
`LAPTOP-NP49RG6H` / `192.168.12.97`, separate from the deferred VM.
This changes probe control-side validation only, not Engine RT publication,
the driver, native audio periods, transport buffers or production defaults.

## Reproduction and bounded repair

The original [packet/block matrix](2026-10-29-vad-period-block-matrix.md)
preserves a failed `default-b128` acquisition. Its same first-failure snapshot
has executionRunning, active/plan revision1, owner/resource1/0, no fault or
cleanup error, but `timeline=null`. The probe incorrectly requires an available
positive timeline on every poll to classify execution asRunning.

The virtual owner's one-attempt seqlock observation may legitimately be
unavailable; the supervisor does not equate that with stopped execution.
The archive proves the false-stop conversion, not which internal seqlock or
identity-filter branch caused that particular null.

The repair keeps two distinct facts:

- Execution lifecycle is confirmed independently of optional timeline data.
  Initial preparation still requires a positive, plan-matched timeline and
  returns that exact accepted HostSnapshot, without a second read.
- One control-side guard pins desired/active/plan revisions, owner/resource
  generations including legitimate0, and timeline epoch. Real lifecycle/fault,
  identity, epoch and frame-regression failures remain hard errors.
- Unknown or unchanged timelines do not advance progress. A five-second
  no-progress deadline is extended only by the existing correctness policy's
  configured0..3000ms grace. The post-observation elapsed time is checked before
  renewal; a newly advanced frame at/after expiry cannot erase it.
- Crossing the base deadline remains a sticky quality failure even if progress
  returns during correctness grace. Grace is diagnostic control tolerance,
  not permitted audio delay or a buffer/default change.
- Raw unavailable timelines remain null. Separately labelled historical last
  advancement and sticky first error survive final serialization. Failing raw
  reports/health are retained before returning errors, including retirement's
  pre-stop observation. The same guard spans admission, recorder confirmation
  and collection; post-shutdown Closed validation is unchanged.

No recorder first-packet discontinuity, provider drop/silence, xrun or other
quality failure is exempted. Synchronous native QUERY/IO/join operations may
block; this control deadline does not become an OS watchdog guarantee.

## Verification

Verification runs on the separate physical build target with no concurrent
native audio. All use the same `D:\MoirenBuild\target`, Cargo home and D: temp
paths. Build outputs remain cached; no `cargo clean` occurs.

- RED: extract the existing production predicate unchanged, feed a confirmed
  Running snapshot with transient null timeline through Health validation.
  One regression fails at the intended assertion, exit101;500 tests filtered.
- First GREEN:8 targeted lifecycle/progress/policy tests pass. Complete initial
  probe suite:508 passed. Audio library/integration suite:899 passed,3 ignored.
- Independent review finds two evidence-retention issues: final observation
  must not replace a prior tracker failure with a fabricated successful result;
  retirement must latch raw failure before final observation overwrites it.
  Both are repaired, with a sticky-first-failure regression added.
- Final source:509 probe tests pass, zero ignored; workspace rustfmt check and
  strict audio all-target Clippy with `probe-tools,vad-probe` pass. Final local
  source hashes match the verified remote inputs. The unchanged audio library/
  integration suite is not rerun after the example-only review repair.
- Final independent source review confirms both findings closed and reports
  no further Important/Critical issue. Source review is not native acceptance.

Logs retain RED/GREEN, full and final-review commands, exit codes, source
patch/hashes, disk checkpoints and expected failure separately. The remote
tested patch is preserved in a named stash before adopting the exact committed
source for a clean release build. The user's remote main stays untouched.

## Deployment and single native retest

Clean committed release build succeeds, exit0. Only the probe is updated in
`D:\MoirenTest\20261029-0118a54\bin`; the old5382369 probe and its manifest
are preserved in an explicit rollback directory. New binary:6,970,368 bytes,
SHA-256 `3CEF3ABFFE43E6C8681176ED9628B840B87973B22D8542800835C5954E5997A4`.
Applicatione195bfe and installed driverSYS hashes remain unchanged; there is
no driver reinstall, application rebuild or system audio-setting change.

Only the original failed shape is retested in a new artifact directory:
`vad-engine-health2-default-b128-t1024`. Exact settings remain480-frame native
playback/microphone packets,128-frame Engine block, target1024/capacity4800,
jitter96, declared packet/gap480, recovery cap1, correctness grace3000ms,
30-second ceiling and the same240,000-frame schedule.

- Exit0 without watchdog; collection, source schedule and cleanup complete.
  Source acknowledges240,000 frames. The recorder retains532 packets /
  255,360 frames /2,042,880 PCM bytes; source submission is not marker detection.
- Both running provider snapshots have zero drop/silence counts. The control
  tracker retains no deadline overrun or first failure. Its unknown count is0:
  the original contention condition does not recur in this bounded native run.
  Deterministic RED/GREEN proves classification handling; this run is not a
  native reproduction of the transient-null branch.
- The recorder still has exactly one discontinuity, first packet flags1,
  position480, stored start0. Later adjacency mismatches, gaps, drops, overflow,
  timestamp errors, silent frames, missing/incomplete sink packets and nonfinite
  samples remain0. Strict quality staysfalse; no startup exemption is added.
- Both exports confirmRemoved; Engine confirmsClosed/Dormant without terminal
  fault or cleanup error. Post-stop histories remain distinct from running
  observations. The older failed acquisition is not overwritten or relabeled.

Archive: `evidence-vad-engine-health-f72e03c.zip`,378,628 bytes,
SHA-256 `BF85FC3DE37894B161C6EFD18192FF33D943F96C1FA766FD0A7CB10DEB52F616`.
All77 member files /78 entries, lengths and SHA-256 hashes are verified on the
host. The archive includes original PCM/packet/report files, exact manifests,
settings, validation stages, source hashes/patches, runner/deployment scripts,
cache policy and idle target state. Host semantic assertions separately verify
the run, quality failures, binary identity and final-source equivalence.

Final archive checkpoint: no build/audio process active; shared target retains
915,203,047 bytes (~0.85 GiB), D free271,478,968,320 bytes (~253 GiB),
C free5,341,106,176 bytes. No cleanup is performed after tests, deployment or
archiving. Storage guards remain inspection/pause thresholds, not auto-clean.

The probe's false-stop defect is repaired and the bounded retest completes.
Continuous clean waveform and calibrated physical latency remain unqualified.
This is not a VAD/Process path comparison, stable sub-10-ms output, a lower
default decision, or completion of S1-S3. Physical follow-up still needs the
intended usable output and independently captured common reference/calibration;
the muted output is not changed to force a pass.
