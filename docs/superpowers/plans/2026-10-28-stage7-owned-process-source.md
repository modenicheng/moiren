# Stage 7: owned Process source and matched independent collection

Date: 2026-10-28. Base: bb7ea620924829657b9298b024e7e484b861bce3.
Worktree: `.worktrees/minimal-io-latency`, branch `feat/minimal-io-latency`.
This is an implementation slice of the existing minimal-IO specification,
not a replacement acceptance definition.

## Storage constraint

- Build only on the host. The 64 GB-class guest includes Windows; do not
  create guest build/cache trees or leave old probe versions there.
- Stage one bounded run at a time. Archive failed runs too, verify lengths
  and SHA-256 on host and guest, then remove only explicit owned artifacts.
- Preserve Windows, DriverStore, `C:\DriverTest`, installed debugging tools,
  current signed driver and rollback packages. Reject reparse cleanup paths.
- The 2026-10-28 01:04 +08:00 read-only inventory found the test root empty,
  no probe processes or dynamic VAD children, bus OK, and 32,225,071,104 free
  bytes. There were no obsolete owned files to delete.

## Missing capability

The Process probe currently captures an externally supplied PID. Only VAD
can generate the accepted known source schedule and supervise an independent
MeasurementSession. An external PID and imported PCM do not establish an
owned matched-condition source/recorder run.

## A. Bounded owned source precursor

1. Reuse the existing validated SourceSchedule, PreparedSource and SourceCursor.
   Never create a second generator, infer recorder windows from source starts,
   or call successful ReleaseBuffer physical playback.
2. Extend the example-local native playback owner with an explicit shared-mode
   MMDevice target. Preserve the VAD exclusive path and stable/root matching.
   Shared source uses exact 48 kHz stereo Float32/PCM16, event callbacks and
   current-padding-sized available packets; reject unsupported formats rather
   than enabling undocumented automatic conversion. Preallocate all samples
   and encoding storage; no native reconstruction or filesystem IO in callback.
3. Add a child mode `process-source-worker`, not an in-process source (Process
   Loopback rejects the probe/ancestor identity). Require an explicit endpoint,
   source file, new private artifact directory and finite lifetime <=600 s.
4. Publish bounded, new immutable ready/running/submitted/final JSON receipts.
   Parent activation and stop are distinct one-shot tokens. No periodic PCM
   files, progress-file rewrite loop, unbounded stdout capture or detached task.
   Main child control may poll at bounded intervals; native playback remains on
   its existing owner. Source completion means submitted only; continue zeros
   until stop/deadline. Retain failed preparation and native Stop/join outcomes.
5. Provide an owned parent session to spawn/pin the real child identity, wait
   for ready/Start/submission, request stop and boundedly join/force reap on
   fallback. Unexpected exit, missing/malformed/oversized receipts, event
   timeout and failed Stop must remain failures, including final late faults.
   No shell/string-command child launch or writes outside its run directory.
6. Watch regression tests fail then pass for parsing/path collision, strict
   byte/duration caps, activation ordering, early exit, final late timeout,
   padding-limited chunks and failed ReleaseBuffer accounting. Tests must not
   open real devices or launch native hardware from the normal test suite.

## B. Actual Process collection integration

After A passes independent review, wire the owned child into the existing
`Input -> Bus -> Output` Process path with independently owned measurement:

- Explicit source output, independent measurement input and finite budgets.
- Observe recorder and graph Running before source activation; preserve real
  PID plus creation time, source endpoint, exact schedule, negotiated formats,
  timing and final source/capture/render/bridge/measurement/load evidence.
- Reuse MeasurementSession and controlled_load::Supervisor. Do not replace
  the accepted VAD collector with a speculative generic supervisor.
- Stop every native owner and child before any join or evidence disk wait.
  Preserve partial preparation/run/final failures and reconcile final health.
- Keep external-PID Process mode compatible. New collection options are explicit
  and mutually exclusive with imported measurement and diagnostic soak.
- Collect raw independent PCM/packet metadata; do not synthesize dual-channel
  reference wiring, calibration, measured delay or qualified status.

### Dependency ruling: preparation failures must stop peers before joining

Prepare the recorder first, the identity-ready owned source second, and the
gated Process graph last. A full graph-first reorder is invalid: Process
capture requires the actual child PID and creation time, and recorder-native
constructors can join internally before returning an error.

Before B integration, add only three control-side returning/abort entry points:
Process capture preparation, native render preparation and renderer attachment.
Existing APIs delegate with a no-op callback and preserve their signatures.
On failure, request local/common stop, invoke the caller's peer-stop callback
once on the calling control thread, then join/recover the local owner. Callback
panic or stop failure cannot skip local cleanup. Retain the original error,
available partial report, independent stop/join errors and callback outcome;
render attachment must also return its renderer. Success never invokes abort.
No callback enters a native/RT worker or changes timing/gate/driver policy.
Exercise the actual owner guards with hardware-free failure/unwind regressions.

Separately split the existing controlled-load supervisor's stop request from
its join. Stop-token failure must not prevent reaping or erase the actual
child outcome. B must broadcast all native/child/load stops before any join.
Neither prerequisite completes collection or relaxes the final physical gates.

## Verification and delivery

- Focused RED/GREEN tests, complete probe example tests, strict example/test
  Clippy, rustfmt and Linux example check where applicable.
- Independent spec/quality review before atomic source commit and prompt push.
- Only a short explicit host diagnostic on a selected non-default virtual
  endpoint after review; no speaker/default/microphone changes by assumption.
- Subsequent complete integration checks and bounded native diagnosis must
  still precede physical tests and any long run.

## Unchanged final gates

Exact 48 kHz `Input -> Bus -> Output`; calibrated physical p95 plus uncertainty
<=10 ms; report p50/p95/p99/max; zero unexpected steady-state quality faults;
same-condition VAD/Process comparison; qualified 30-minute candidate and two
independent two-hour repetitions. Compatible remains default. No VM/internal
timestamp, source receipt, passing software test or this precursor completes
Stage 7.

## Checkpoint, 2026-10-28

- A accepted and pushed as `6464581`: exact eight-file frozen snapshot,
  independent spec/quality PASS, parent 159 example / 18 owned / two shared
  tests and selected strict Windows lint PASS. Actual test-profile strict
  lint gaps remain explicitly recorded in the validation document.
- Signal-only controlled-load stop prerequisite accepted/pushed as `2a0561d`.
- The nine-file production preparation-abort prerequisite was independently
  reviewed and accepted/pushed as `6660ee1`. Parent abort tests (24), strict
  all-target crate lint and scoped formatting pass; the owner also records
  105 WASAPI tests and the 86-test RT harness passing. Its first full parallel
  library attempt failed (509 pass, one failure, two ignored) at the existing
  virtual-owner transient Running wait. The same failure reproduces in pristine
  `bdfe6e3` and the candidate under a bounded child-only single-CPU experiment;
  keep the failure and its triage rather than claiming a full parallel pass.
  The final serial retry now terminates successfully: 510 library and 221
  integration tests pass, five ignored across 28 suites. The earlier parallel
  failure remains recorded; serial success does not rewrite that outcome.
- B integration is dispatched against `6660ee1`, using the exact four-file
  brief and the accepted source/load/library interfaces. Removing only newly
  unfulfilled example-local dead-code expectations is permitted as consumption
  hygiene, not a behavioral change to A. No collection, physical qualification
  or default promotion follows from these prerequisites.
- Read-only VM inventory at 01:55 +08:00 finds the test root empty, no test
  processes, bus OK and 32,221,892,608 bytes free; no old owned files exist
  to remove. Keep builds and caches on the host.
- B's first independent review requests three repairs despite 176 passing
  example tests: enforce first positive acknowledgement deadlines, reconcile
  final stopped bridge evidence, and independently attempt graph PCM/metadata
  persistence. Keep B commit/native execution on hold for genuine regression
  RED/GREEN and independent re-review; initial software checks are not approval.
- At 02:32 +08:00 the pinned-host-key guest inventory again finds the test root
  empty, no probes or present dynamic Moiren children, bus OK and 32,220,827,648
  bytes free. No old owned files can be removed. No guest build/cache or audio
  run was introduced by this read-only check; the storage policy is unchanged.
- B repairs now pass independent spec/quality re-review and were atomically
  committed/pushed as `4811362`. Parent freshly reruns all 189 example tests
  and ordinary/actual-test-profile Windows and Linux strict lint successfully.
  Genuine regression failures and their fixes remain retained. This resolves
  the B source-review hold, not any native/physical gate.
- A clean `4811362` Git archive builds the release example successfully on the
  host. Complete accepted-pin workspace/app/RT/driver-host verification is
  running in that isolated snapshot; no result is claimed while commands run.
  A separate host smoke helper has its own provenance/partial-evidence review
  findings under repair and must pass re-review before native execution.
