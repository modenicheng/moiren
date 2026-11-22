# Explicit measured Process selection in the application

This software slice continues S3 without opening native audio, running the VM,
installing a driver, deploying a package or changing production defaults.
Physical matched-route timing and stable sub-10-ms qualification stay deferred
under the user's 2026-10-29 direction. This does not complete S1-S3.

Implementation commits `380e2490dd9d5ea476b5008ab7504256ee9aff29` (managed
service) and `356669ee709029211ae79e7565ac06e3b5b2052d` (actual CLI) were
separately committed, pushed to `origin/feat/minimal-io-latency` and checked
against `git ls-remote` before the next commit. The worktree and unrelated
`gate.log` are preserved.

## Application contract

`ApplyMeasuredProcessBridge` explicitly selects the application's own current
native Process observations. It requires a settled Running graph, exact source
node and PID/creation time, originating app generation, native owner generation
and graph revision. Pending audio/transport/profile changes, pending Apply,
parameter confirmation, missing observations and stale origins are refused
before retirement. The actual host is reread before accepting the selection.

The full typed evidence is ephemeral and cannot be loaded as trusted proof from
a saved project/settings file. Desired and confirmed-active summaries are
separate. Apply captures an immutable profile; a later transport draft cannot
rewrite the pending replacement. Both f32 and f64 production factories pass
the typed evidence to `ProfessionalBackend::with_measured_process_bridge`.
The actual backend still revalidates source, storage and negotiated complete
consumer demand. There is no fallback to a different target or live SPSC resize.

Graph revision changes after selection are refused before retiring audio;
clear the profile or select fresh observations. Successful document replacement,
normal transport settings and Stop clear desired measured intent. A transport
draft does not alter confirmed-active intent until another Apply confirms.
Preparation refusal preserves the desired summary without claiming it active.

Pending measured replacement requires a clean predecessor teardown. Non-Closed
host state, cleanup errors or final faults cancel it before the next factory can
open a host. Declared replacement preserves the existing modeled recovery path
for `Closed + ResourceUnavailable + no fault`. Other failures cancel queued
replacement; after Closed, a later explicit declared Start/Apply may retry while
retaining the original receipt. Measured intent must be cleared first. An
incomplete/non-Closed cleanup never admits a new host merely because of retry.

## Real CLI entry point

Add these flags to `moiren-app monitor --process PID --output ENDPOINT`:

```text
--measure-seconds 1 --target-frames 256 --capacity-frames 2048 --jitter-frames 0
```

This is an example request, not a promised admissible preset. Measured mode
requires all three budget fields and no declared packet/gap bounds. The shape
is bounded before preparation: measurement 1..60 seconds, capacity 4..262144,
2 <= target < capacity, and jitter < capacity. Sample-age enforcement remains
unsupported and is refused, not silently discarded.

The CLI first runs the unchanged Compatible baseline, collects one stable
origin, explicitly requests the measured replacement and waits for a different
app generation with the exact active profile. Only then does `--seconds` start
timing the requested run. Each preparation phase has one fixed startup deadline;
collection has its requested duration plus one fixed observation grace. Late
matching snapshots cannot bypass an expired phase. Polls never renew deadlines.
These control-side bounds are not an OS watchdog for blocking native calls or
joins, and are separate from the probe's correctness/strict timing policy.

Report schema v1 gains additive requested target/measurement fields, originating
measurement generations/duration, confirmed replacement/native generations and
active measured summary. The requested capacity/target/jitter survive refusal.
During the timed run the confirmed graph, audio/transport settings, measured
selection and native owner must continue matching. Cleanup errors are retained
separately from run completion. A joined thread alone does not prove successful
owner teardown. Missing snapshots do not disable the run deadline: it requests
cleanup without claiming run completion when confirmation is unavailable. The
report always keeps `qualified: false`.

The service command and actual CLI are wired; this is not a new desktop UI
control for selecting measured profiles. Existing desktop declared settings are
unchanged. Per-wake admission is not finite-window queue replay, source sample
age, sustainable queue balance, physical latency or uninterrupted output proof.

## Verification record

Focused test-first logs retain the initial missing implementation, insufficient
target, late graph edit, missing requested-target report and unbounded collection
failures followed by passing regressions. Tests inject observations and prepared
backends; none open a native audio stream.

Final hardware-free checks:

- App library with `gpui-test`: 449 passed, 3 ignored, `--test-threads=4`.
  Both precision factories, typed evidence, captured Apply state, refused
  settings, stale origin, graph edits, cleanup failure/retry and monitor phase
  deadlines are covered without native devices.
- Strict all-target app Clippy with `gpui-test`, all-target app check,
  workspace formatting and diff whitespace checks pass.
- Real-binary CLI integration: 2 passed, using the explicit
  `x86_64-pc-windows-msvc` output rather than replacing another session's shared
  debug executable. These tests exercise help, not live Process audio.
- No Linux/Ubuntu or Miri execution is claimed for this slice. Non-Windows
  fields/imports are cfg-gated and statically reviewed; the previously documented
  missing Linux GLib/sysroot setup is unchanged. CI remains required before
  integration.

The first full app run found a real cleanup-gate regression: 445 passed,
1 failed, 3 ignored. An overbroad rule blocked
`apply_replaces_waiting_owner_after_export_pcm_drains_with_present_pdo` on its
modeled `ResourceUnavailable` disconnect. The fix narrows the typed declared
recovery exception and adds explicit Closed retry regressions; the existing test
was not weakened. The complete final run includes it and passes. The initial
cleanup regressions and new monitor deadline API also have retained failing
runs before their successful full verification. Final bounded read-only review
found no remaining Important/Critical cleanup findings.

Logs are under `target/s1-s3-20261028/s3-app-measured-*.log`; earlier failures
are retained. The final full run is `s3-app-measured-full-app-final.log`, strict
lint/check logs end in `-final.log`, and real CLI results are in
`s3-app-measured-real-cli-final.log`.

Remaining work: finite-window queue feasibility with truthful inventory and
availability observations; measurable Process sample age; native negotiated
lower-period qualification; suitable-hardware route comparison and sustained
sub-10-ms acceptance. These stay OPEN rather than becoming VM timing blockers.
