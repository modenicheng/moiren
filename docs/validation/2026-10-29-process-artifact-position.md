# Process PCM artifact position applicability

Date: 2026-10-29. This repair changes evidence semantics, not audio buffering,
delivery admission, physical latency or the qualification threshold.

## Native finding and reproduced cause

The archived `process3-owned-compatible` run at source `45a084e` completes
capture and cleanup. The real Process packets have repeated device position 0,
while the bridge intentionally does not treat that as a physical sample clock.
Its separate bounded PCM artifact recorder incorrectly enables position-gap
comparison unconditionally. It reports 534 gaps / 256,320 gap frames for 535
packets, although the bridge reports zero underruns/drops/discontinuities.
Neither counter set alone proves physical lossless output.

The regression invokes the actual CaptureReport artifact preparation, records
two equal-position packets and finalizes the report. Before the repair it fails
with one Process gap where zero position checks are applicable. A second actual
probe regression shows final admission accepting old metadata without explicit
position applicability. Both RED runs are executed on the separate physical
target using hardware-free tests, with unchanged installed driver and no audio
acquisition. Original failure logs remain retained.

## Contract

- CaptureReport schema 12 retains PCM artifact metadata schema 2. The metadata
  explicitly publishes `detect_position_gaps`; false is unchecked, not verified
  continuity. Missing or old metadata does not acquire a new success meaning.
- CaptureReport prepares Process artifacts without position correlation.
  Physical capture retains it when mix/client frame units match; a known
  converted-rate mismatch disables only that invalid comparison. The public
  standalone PcmRecorder constructor preserves its existing enabled default.
- Native discontinuity, timestamp-error, SILENT, dropped/overflow and packet/
  frame counters remain. Timestamp-error packets now have an explicit retained
  artifact count. Storage remains bounded and preallocated; callback work is
  scalar bookkeeping plus the existing bounded PCM copy, without resize/lock.
- The actual Process final collector requires schema 2 and explicit false
  applicability, rejects missing fields, and still checks zero gaps and all
  real native/overflow faults. Raw position 0 is not a source timestamp or
  a physical calibration. No positions are synthesized or replaced.
- Existing raw GetBufferSize evidence, unknown Process capacity/period,
  8-MiB packet safety limit, complete lease release and owner cleanup remain.

## Verification

Remote worktree `D:\MoirenBuild\s1-s3` is based on `45a084e` plus the exact
six-file repair patch. Its formatted binary patch SHA-256 matches the host:
`CA694E4F3340B8C978C5B08D097C0FB23C8B055C48C5486292A96C3F5C1F19D0`.
The clean remote main worktree is not modified. One explicit shared target,
jobs 3, no incremental and no dev/test debug symbols keep cache use bounded.

| Check | Result |
| --- | --- |
| Actual preparation regression before repair | Fails as expected: Process gap 1 versus 0 |
| Actual final collector regression before repair | Fails as expected: accepts metadata schema 1 |
| Actual report artifact regressions | 3 passed |
| Final collector schema/flags regression | 1 passed |
| Full Windows-audio package | 877 passed / 3 ignored |
| Full E2E example | 500 passed |
| Strict audio all-target Clippy, probe-tools/vad-probe | Passed |
| Workspace formatting and scoped diff | Passed |

Logs are under remote `D:\MoirenBuild\logs\process-artifact-{red-owned,
graph-red,green}` and host `target/physical-1921681297-20261029/`. An initial
PowerShell native-stderr redirection wrapper fails before executing the test
suite; it is not the RED result. The subsequent Process-owned wrapper retains
actual waited exit codes and test failures. Full-suite results above are from
the exact formatted patch, not from previous deployed binaries.

Native redeployment, repeated-zero-position confirmation in fresh captured
packets and independent recorder continuity remain separately required. The
previous recorder discontinuity is not erased by correcting artifact position
applicability. S1-S3, continuous replay, freshness composition and physical
sub-10-ms qualification are not completed by this repair.
