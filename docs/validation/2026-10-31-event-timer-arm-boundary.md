# Event timer arm boundary diagnostic - 2026-10-31

## Purpose and evidence boundary

The [matched MIC startup trace](2026-10-31-mic-startup-clock-rebase.md) identifies
the first EventDeadline rebase and silence0->1 in the same callback. Callback
entry is already late; its State acquisition takes only one tick. That does
not distinguish a late preceding rearm/long previous callback from delayed
native timer dispatch. No production scheduling fix is justified yet.

This additive diagnostic records that missing preceding arm boundary. It
changes observation cost, not timer policy: two QPC calls are added around
each event-mode WdfTimerStart. Do not claim zero instrumentation perturbation.
No native installation/acquisition occurs for this slice. Installed driver610
and the frozen9968505/trace2 failure remain unchanged; historical traces do
not retroactively acquire the new fields. This paragraph describes the
original source-only slice. The subsequent, separately frozen
[611 target attempt](2026-10-31-event-arm-native-setup-failure.md) installs the
diagnostic but fails native setup before Start; it acquires no arm witness.

## Implementation and RT constraints

`audio/Streams.cpp::ArmEventTimer` wraps the existing event-mode arm at Run,
early callback and normal/rebased callback completion. It keeps the exact
signed relative due value, one WdfTimerStart and the existing position in the
State-protected processing order. Single-packet timer-mode arms are unchanged.
The48-byte per-stream `EventTimerArm` retains planning/before/after QPC, due
intent, target serviced deadline and the associated clock epoch. No heap,
new lock, wait, catch-up loop or RT log is added.

On the first EventDeadline rebase, the callback copies the PRECEDING arm and
its already sampled callback-entry/State QPC before replacing the arm with the
new epoch's request. Shared-lock recording and the first-observation latch
stay unchanged. The endpoint diagnostic grows216->280 bytes with a320-byte
compile-time cap; its lifetime/reset/one-shot publication are unchanged.

`Control.cpp::TracePendingClockDiagnostic` adds one passive WPP fragment only
for EventDeadline. `Clock dispatch boundary` has13 fields, including the same
endpoint/session/generation/stream/callback tuple. TimerStall still emits only
the original three fragments. WPP stays outside Shared->Lock at PASSIVE_LEVEL.
The ABI, generation/gate invalidation, counters, packet size, queue budgets,
native periods and defaults are not changed.

## Offline interpretation

`scripts/vad-clock-rebase.py` schema2 accepts an optional unique boundary for
each complete rebase/no-copy association. Legacy records explicitly retain
`dispatch_boundary.available=false`. A supplied but zero/foreign-epoch/
regressed clock or nonnegative due is kept raw with association-valid false;
no derived wait is fabricated. Duplicate and unmatched-identity fragments
refuse, not silently ignored. All existing quality/coverage flags stay false.

For valid clocks the report derives only State wait and callback entry versus
the serviced deadline. The arm's before/after interval and signed relative due
are actual call observations/requested intent, NOT kernel timer expiration.
Even a late entry after a prompt arm is not proof of a specific scheduler,
ISR/DPC, CPU/load or hardware cause. Old trace2 still has no preceding-arm
coverage;1.3007ms/4.3007ms remain diagnostic lateness, not physical audio delay.

## Verification

- Actual-source RED: after correcting extraction/stub fixture issues, the
  callback suite fails the missing preceding-arm/passive-fragment assertion
  in `target/clock-arm-red5.log`; no production code is changed until then.
- GREEN: extracted real Schedule, arm helper, callback, atomic invalidation
  and passive WPP retain distinct before/after stamps from the PRECEDING arm,
  the old epoch/deadline and unchanged relative due/start count. A stopped
  callback does not rearm or overwrite evidence; a single-packet callback
  retains exactly its original three QPC reads and does not touch event-arm
  state. Publication is one-shot.
- All17 driver host suites pass, including event/timer/native-playback and54
  pause-fence scenarios. The native-playback extraction anchor follows the
  new arm wrapper; it still excludes scheduling from its transfer slice.
- Offline suite grows11->15 tests after an optional-boundary RED with2
  failures/5 errors. Tests retain legacy absence, valid dispatch, unavailable
  clock and duplicate/foreign identity. Ruff passes.
- Default-off WDK build with `/analyze` and native INF verification pass.
  TracePDB produces all13 dispatch field placeholders/descriptors, including
  signed `due_hns`, without warnings/omissions. This is decode-format proof,
  not acquisition or scheduling qualification.
- Playback-copy-enabled WDK `/analyze`/INF and matching13-field TMF checks
  also pass; existing unserved callback/write_pre13/11-field TMFs remain
  warning-free. Old trace2 re-analysis creates `matched-rebase-v2.json` with
  the original same-callback association and explicitly unavailable dispatch
  boundary; the original v1 receipt is preserved. Ruff, rustfmt and diff
  whitespace checks pass. Neither build is deployed.

Logs are in `target/clock-arm-*`; host executable logs are also in
`driver/vad/build/tests`. The retained-QPC regression and library679/2 ignored,
probe627/0 ignored checks precede this additive C++ diagnostic; no Rust source
is modified by it. No driver is installed on the
development host or target for these builds.

Local analyzed SYS pins (not installed): default
`4B1F9BE5071103C2B10C835B94883DA49799C3660441D8D8DD18BC9D6AE4B559`;
playback-copy-enabled
`8EAA1BC3968693B0154E5EC8B3BFC8995FC4D4EFCDD47352C4E6639C4C76EE28`.
Matching PDB/TMF artifacts remain under their distinct existing build trees.

## Remaining work

Acquire this observation only in a separately frozen, archive-verified target
experiment with the unchanged exact route and existing quality gates. Native
acceptance, waveform identities/continuity, sustained lower-budget delivery,
matched VAD/Process and calibrated physical measurement remain open. Neither
this diagnostic nor API period limits establishes sub10ms impossibility or
the current device's best stable minimum.
