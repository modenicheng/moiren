# Native source write witness plan

Date: 2026-10-31. Diagnostic continuation after the
[all-zero provider READ](2026-10-31-provider-input-capture.md), not an S1-S3
acceptance plan or a product-default change.

Implementation and one bounded native observation are now retained in the
[source write receipt](2026-10-31-source-write-witness.md). The original
checklist below is a planned contract, not a claim that all acceptance or
test coverage is complete; the receipt retains limitations and the zero READ.

The existing source counter commits generated nonzero f32 frames only after
encoding/copy/ReleaseBuffer succeed. It does not independently inspect the
granted native destination. Before another driver change, observe that missing
boundary once. Repeating an equivalent Process recorder would not answer it;
adding kernel instrumentation first would require a new installed driver.

## Contract

- Keep current holder `b0a0223`, installed `6d9e32f` driver, source20second
  schedule, exact Shared endpoint, format selection,12second observation,
  provider READ capture and cleanup policies unchanged.
- Add an explicit process-source-worker opt-in, disabled elsewhere/default.
  A one-shot fixed-size record copies at most16bytes of encoded data and the
  actual GetBuffer destination after memcpy and before ReleaseBuffer.
- Select the actual first known nonzero source frame within the current
  packet, not a silent preroll prefix. Record frame range, sample format,
  frame/byte count, flags, exact byte offset, QPC brackets and copied arrays.
  Preserve attempted/failed vs successfully released confirmation separately.
- Precompute bounds before acquiring the native buffer. No new fallible
  diagnostic path may bypass the matching ReleaseBuffer; no allocation,
  filesystem/serialization, extra client or wait inside the lease. The record
  owns bytes, never a retained native pointer. Keep the original audio error.
- A nonzero matching destination and successful Release witness plus zero
  source-active provider READ narrows the unresolved interval after release
  and before READ. It does not identify the Windows/driver cause or prove
  the whole waveform, continuity or physical latency.

## Execution and verification

- [ ] Isolated source helper/CLI/DTO implementation with genuine RED/GREEN;
  test exact current-frame offset, PCM16/Float32, silent preroll, bounds,
  disabled behavior and Release failure without lost original error.
- [ ] Independent source and frozen PS5.1 runner review; actual helper fixtures
  reject missing/unconfirmed/mismatched evidence without overwriting originals.
- [ ] Scoped tests/Clippy/fmt, build host only, deploy only the changed source
  worker side-by-side and pin its hash. No target build during acquisition.
- [ ] One bounded exact-identity run with the same captured READ boundary,
  source PID/creation, lifetime/active schedule, normal teardown and driver audit.
- [ ] Inspect actual bytes/timing, verify archives against independently
  downloaded originals, record truthful outcomes and batch commits/push.

Routing remains virtual speaker -> Moiren Input/Source and Moiren Output/Sink
-> virtual microphone. No physical audio client, speaker/default/privacy
change, buffer tuning or driver reinstall is part of this diagnostic.
