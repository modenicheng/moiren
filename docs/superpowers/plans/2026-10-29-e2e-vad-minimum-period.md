# E2E VAD minimum-period request

The S1-S3 goal remains active. This software slice lets the actual E2E
exclusive VAD source request its queried minimum period on future hardware.
It does not qualify the negotiated cadence, waveform continuity, or latency.

## Constraints

- Add an opt-in `--vad-minimum-period` switch to live VAD only.
- With the switch omitted, preserve the E2E opener's existing default request,
  accepted range of 1,000..=1,000,000 hns, single fresh-client alignment retry,
  production queues, graph admission, and legacy JSON field omission.
- With the switch present, select the queried minimum with the same explicit
  range. Refuse invalid minimum values without a fallback to default.
- Keep request evidence distinct from actual native buffer frames and provider
  packet observations. Evidence is `unqualified-request-only`.
- Retain raw default/minimum query, initial/final requests, and retry intent
  through initialization failures and worker cleanup.
- Keep identity, storage, cleanup, and native errors hard. Do not run native
  audio, access the VM, install a driver, or change production defaults.
- Preserve unrelated `gate.log`. Commit exact paths locally; batch pushes.

## Tasks

1. [x] Add portable period selection/evidence with regression tests covering
   legacy defaults, explicit minimum, invalid queries, and alignment evidence.
   Observe an expected failing test before implementation.
2. [x] Add CLI parsing, VAD-only validation, and opt-in manifest/report intent;
   test duplicate/foreign switches and unchanged omitted field behavior.
3. [x] Forward intent through both E2E playback preparation paths. Query and
   publish evidence before native initialization/retry, retaining partial
   outcomes. Preserve shared-source preparation behavior.
4. [x] Run focused and complete example tests, strict example Clippy, format
   and diff checks. Obtain an independent review, fix material findings, and
   record exact evidence and remaining native gates in a local commit.

Completed source: `1d3e691`. Separate baseline test lint cleanup: `b62de1e`.
Final complete independent review is clean after the alignment-query finding
was fixed with a genuine RED/GREEN regression. Host example337/focused15,
normal/test-profile strict Clippy, root fmt and diff pass. Linux cross-checks
compile with documented cfg warnings. Evidence:
`docs/validation/2026-10-29-e2e-vad-minimum-period.md`.
The overall S1-S3 goal is still active; no native/physical result is inferred.
