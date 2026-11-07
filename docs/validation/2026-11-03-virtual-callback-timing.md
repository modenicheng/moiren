# Opt-In Pure-Virtual Engine Callback Timing

Date: 2026-11-03, Asia/Shanghai. Integration baseline: `b6f48c1`.

## Original S1 Gap And Scope

Original S1 calls for bounded observations at actual boundaries, including
callback time. The pure-virtual owner advances the Engine without a physical
render master. Existing native Write envelopes therefore cannot establish its
Engine callback service time. This increment instruments that existing call;
it does not change pacing, transport budgets, recovery, queue defaults or
Engine epoch behavior.

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-virtual-callback-20261103-1800`.
Only compilation, pure and headless checks run on the development host. Native
audio, desktop UI, OBS, device and driver activity remain remote-only. Closed
waveform analysis may run remotely in parallel with independent development;
shared native resources remain serialized. First-packet investigation stays
deferred, and missing physical equipment does not block software work.

## Implementation Contract

`ProfessionalBackend::with_virtual_callback_timing` is opt-in and defaults to
false. An enabled pure-virtual preparation attaches fixed atomic observations
and a separate prepared QPC clock to its existing immutable `VirtualClockProbe`.
Timing never adds calls to `VirtualRealtimeClock::now_ticks` or initializes an
additional clock on the disabled path. The existing clock snapshot and
`publish_with_clock` interfaces retain their meanings.

The measured interval surrounds `Engine::render`, not native presentation,
waiting, waveform transport or control-side publication. Latest evidence records
attempt sequence, requested frames, Engine epoch/frame before and after,
success/error/unfinished outcome, optional QPC endpoints and optional duration.
Fixed cumulative counters retain attempts, completions, errors, valid timings,
missing/regressed clocks, maximum duration and saturation. Missing or contended
evidence remains unknown; it is not a zero-duration sample.

An unfinished attempt is published before entering the Engine. A panic must not
masquerade as a completed previous callback or fabricate post-render coordinates.
Streaming performs no heap allocation/deallocation, blocking or publication
retry. Probe ownership is retained and reclaimed on control after join.

`NativeObservationSnapshot.virtual_callback` reads the same owner-local probe;
a retained old-owner handle cannot be redirected to its replacement. Both app
precision factories select the hook through the existing explicit native-render
evidence option. Ordinary application defaults and saved settings are unchanged.
Callback service time is not physical end-to-end delay or a sustained-quality
verdict.

## Verification

The corrected API RED fails only on the missing callback builder. The subsequent
behavioral RED compiles, then fails because the actual owner publishes no callback
sample. Earlier fixture/preparation failures are retained separately, not counted
as clean behavioral regressions. Initial actual-owner and helper checks pass.
Subsequent test-only syntax failures and an incomplete concurrent fixture overlay
are retained in their own logs; they are not product regressions or passing runs.

The main clean-source callback selection passes25tests with1unrelated ignored
fixture (`callback-main-v3.log`). It includes both-precision actual-owner Engine
boundaries, Engine-error publication, disabled-path zero clock reads, invalid-clock
counters and allocation/deallocation guards. A real owner-loop panic leaves an
unfinished sample with no invented completion. Portable snapshot round trips,
publication contention/exhaustion, sticky counter saturation and retained
old-owner projection also pass. These are fake-clock/portable checks, not device
observations. App acceptance independently passes5snapshot-publication tests,
including callback-only dedup and explicit unfinished/null serialization.

The integration source is a clean archive of `b6f48c1` plus an explicit intended
overlay allowlist, excluding the27preexisting dirty/cache paths. Their hashes
match before integration.

The first feature integration detects an omitted callback dependency in the
existing shadow-compiled clock-observation integration test. Its failed log is
preserved; the test now includes the production callback module and checks that
enabled callback publication leaves clock accounting independent without heap
work. A stale test-fixture call is also repaired. An initial backend-acceptance
fixture incorrectly expected preparation to publish an owner; it is corrected to
retain only after the existing activation path, without changing production.

The actual ProfessionalBackend acceptance passes1test covering both precisions,
real disabled defaults and explicit enablement (`backend-integration-main-v6.log`).
It uses fake provider/native/clock interfaces, opens zero native devices and proves
that the prepared registry's retained probe receives the owner loop's final
Engine coordinates after join. It is not a native hardware test.

Feature-workspace integration passes2476tests with10ignored across71harnesses;
strict feature all-target Clippy and workspace fmt exit0. These original results
retain overlay-v3 identity
`5E697CD63E97582F2D9F8AA01F6EB22032D76ED9B00BD0873AD0011FAA498679`.
The subsequent standalone RT harness fails to compile because its shadow clock
module expects a sibling named `callback_timing`, not the differently named root
module plus an alias nested elsewhere. Renaming only that shadow module and
its type re-export fixes the dependency at its actual scope; production is
unchanged and the failed RT receipt/log remain retained.

The next attempt passes260RT tests and strict RT Clippy, then its fmt check
requires alphabetical placement of the renamed module. That separate fmt
failure remains retained; only module ordering changes for the final attempt.

The24final code overlays are frozen in `code-overlays-v5.json`, SHA256
`B8A60BCDDA02F394E7BCEA7E6BC96AFB003E914476E6B7F4F16B445ADA6AE1AC`.
All23root-workspace overlays remain byte-identical to v3. Since `tools/rt-check`
is a separate workspace excluded from root members, its one-file correction
does not invalidate the root results; original receipts are not relabeled as v5.
Corrected independent RT tests pass260with0ignored; strict RT Clippy and RT fmt
exit0. Ordinary release finishes at19:23:33on2026-11-03with exit0 under the
same v5 overlay identity. The previously pending release entry is superseded
by `release-integration-v5-result.json` and its unchanged build log. No new
native acquisition, physical measurement, lower default or original S1-S3
completion is claimed.

Independent read-only static review of the final24-file overlay finds no blocking
issue in default-off behavior, render-only envelopes, panic/error publication,
bounded atomics, retained lifetime or cfg wiring. It executes no native test and
does not substitute for integration. No Linux or Miri run is claimed.
Read-only follow-up review finds no blocker in the standalone module correction
or preserving unchanged root-workspace receipts under their original identity.

## 2026-11-04 Integration Closure

Readback verifies all24current worktree code files against the final v5 manifest.
The unchanged earlier test logs contain2476passed/10ignored across71workspace
harnesses and260passed/0ignored in the independent RT harness. All seven final
workspace/RT/lint/fmt/release receipts report exit0, retaining each receipt's
original source identity. The closed integration source differs from current
HEAD outside the overlay only in a subsequent waveform documentation receipt.
The verified aggregate is stored beside the original evidence as
`verified-integration-v5-summary.json`.

Fresh isolated-source checks on2026-11-04pass26audio callback tests with1ignored,
the actual app callback-publication/dedup regression,260independent RT tests
and workspace fmt. The initial app filter selects a different native-observation
test, and a subsequent overly narrow module filter selects zero tests; neither
is used as callback-publication evidence. The exact named regression then
passes. Logs remain in
`D:\coding\moiren-evidence\20261104-pipeline-resume`.

This closes the software integration pending entry. It does not validate the
unrelated dirty files, native audio quality, calibrated physical delay or the
remaining S1-S5 requirements.
