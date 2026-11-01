# Process admission budget diagnostics - 2026-11-01

## Why this advances S3

The [native desktop check](2026-11-01-process-low-target-desktop.md) refuses a
1024-frame target but exposes only a generic insufficient-budget message.
The user cannot tell whether the next useful change concerns whole-wake demand
or Process delivery. Add the actual modeled frame terms to that existing visible
refusal instead of repeatedly trying smaller targets or weakening admission.

This slice changes diagnosis, not acceptance policy. Compatible/defaults,
all-phase reserves,whole-wake ownership,recovery and error types are preserved.
No startup flag investigation or new native timing trace is introduced.

## Shared calculation

`latency::transport::transport_input_budget` computes the same input-rate reserve
used by `prepare_transport_input`: upward-converted complete consumer demand,
delivery gap,jitter and at least2frames of interpolation/FIR lookahead.
Both native preparation and diagnostics use this calculation. Checked frame
conversion and addition retain the previous overflow refusals. Capacity remains
independent storage and does not reduce or inflate the operating minimum.

`MeasuredProcessDelivery::budget_for_consumer` reuses admission's consumer
envelope,including historical prepared/observed whole-wake maxima and upward
rate rebinding. Process creation identity and owner generation must match;
the diagnostic result does not authorize a target or publish an active owner.

The app enriches only `InsufficientInputBudget`. Other errors retain their
original messages. The existing desktop feedback label displays the result,
including candidate target,the numeric sum,input-rate units and the distinction
between storage,modeled reserve and measured latency. No RT logging,formatting,
allocation or queue resize is added; this path is preparation/control-side.

## Regression evidence

Host evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.

The first new service regression compiles and fails because the old error lacks
`target 209 frames`,not due to a compile error:
`s3-budget-detail-red-2224.log`. The unchanged209target is below the existing
210-frame minimum. After the change the same test passes:
`s3-budget-detail-green-2226.log`. Target210still admits; increasing capacity
does not change the floor.

Fresh focused checks pass115tests:

- Transport budgets6and virtual budgets21:
  `s3-budget-transport-2228.log`.
- Measured Process evidence26:
  `s3-budget-measured-2228.log`.
- App/service/headless desktop measured subset62:
  `s3-budget-ui-2228.log`.

New controls cover exact reserve/one-frame-below boundaries at multiple sample
rates and capacities,historical whole-wake demand with double upward rounding,
identity/generation fences,and an actual headless settings click whose refusal
keeps the editable draft and previously running owner without dispatching Apply.
The tests establish budget/diagnostic agreement,not future delivery or physical
latency. Seven changed source files are pinned in
`s3-budget-details-2231\source-manifest.json`.

## Native verification

Remote preflight22:30:14+08:00 confirms no owned audio test processes and
unchanged original OBS/QQMusic identities. New isolated deployment directory:
`D:\MoirenTest\20261101-s3-budget-details-2231`.
No audio starts in this preflight. Release build,strict Clippy and native
feedback verification subsequently complete successfully.

App33860608bytes,SHA256
`FCF01755B7122ACB82FDC32011D5D7672505348A75F2891AF0358A22AE40D485`.
Remote executable:
`D:\MoirenTest\20261101-s3-budget-details-2231\bin\moiren-app.exe`.
The retained source-worker executable is separately pinned1956636with SHA256
`3FB5330D790767F68B0114162F137E685AE472D17717CF5C4967D8EFA81A2E28`;
its revision field is corrected in the new runner rather than copied from app.
Release receipt:`s3-budget-release-2232.log`;strict workspace/all-target Clippy
exit0:`s3-budget-clippy-2232.log`.

Actual limited-interactive desktop route remains Process -> unity Bus -> DAWN
PRO2,48kHz/F32,Engine256,Shared1056-frame output,Compatible baseline. Candidate
target1024/capacity16384/jitter96is refused. Native visible terms at the retained
observation are:

| Term | 48kHz input frames |
| --- | ---: |
| Complete consumer demand |1632 |
| Observed delivery-gap reserve |534 |
| Requested jitter |96 |
| Interpolation lookahead |2 |
| Modeled minimum |2264 |

The runner parses the visible label and verifies2264=1632+534+96+2,exact
candidate values and target below minimum. Both requested/active remain
Compatible,owner1is retained during refusal and Apply has no InvokePattern.
Raw UIA IsEnabled remains incorrectly true,as already documented separately.
No smaller candidate is tried. Source has a direct output path too,so this
mixed render is not used as clean waveform/latency evidence.

Native Stop/Quit,app/source/task exit0,Completed=true,no forced termination and
zero cleanup errors. Final22:39:27+08:00audit confirms no test owners,unchanged
original OBS/QQMusic identities,defaults/render controls and unchanged Running
611driver/SYS hash. No host audio/UI or existing host process is manipulated.

Remote run:`runs\desktop-process-target1024-details`. Downloaded result and
screenshot are independently size/hash verified in the new host evidence folder:

- `result.json`:217432bytes,SHA256
  `D6BEC9EC977D029746F9FA84712C329351144AF9666894AD40EDCCD61B0A37F9`.
- `budget-refusal.png`:297573bytes,SHA256
  `EBE6DAC47202BD76C3E9CAD02170E69906B3AC8B8E1C01AA7A2E0AD37DB14D8F`.

The image is visually reviewed: all numeric terms and both Compatible summaries
are visible,the draft selections remain1024/16384/96and the main route is Running.

## Interpretation and next boundary

The current configuration cannot admit1024even before adding its observed
delivery gap: prepared whole demand1632already exceeds the candidate. Source
inspection matches1632=1056native render demand+64SRC/transfer allowance+
2*256graph-block reserve. This is a preparation bound,not1632measured frames of
residence or a theoretical physical floor. Shrinking spare input capacity cannot
fix it,and reducing graph block alone with the same1056render demand remains
insufficient. Any next smaller-target experiment must establish a smaller actual
output demand and remeasure Process delivery before admission. No default or
existing safety margin is changed by this finding.

## Integration verification and development priority

The existing `cargo test --locked --workspace -j 1` invocation completes with
exit 0: 2079 passed, 0 failed and 8 ignored across 67 harness summaries,
including documentation tests. Empty harnesses are not additional test coverage.
Receipt: `s3-budget-workspace-2239.log`. Final `cargo fmt --all -- --check`
and `git diff --check` also exit 0. All seven source hashes still match the
native deployment's `source-manifest.json` after these checks.

The user's approximately ten-minute real-machine use without severe trouble
is sufficient context to continue feature development with Compatible defaults.
An isolated initial discontinuity remains deferred. The retained genuine
`0x8007051A` and epoch/rebase observations are open follow-up issues; a repeatable
failure during ordinary use, continuing waveform damage or failed shutdown would
raise their priority again. No further identical startup/timer acquisition is a
prerequisite for the next software slice. Calibrated physical acceptance remains
separate and deferred for unavailable equipment.

The next bounded step is to determine a supported smaller actual render demand
on the remote endpoint, then remeasure Process delivery under that configuration.
Keep all current whole-wake admission terms and refusal behavior intact; do not
blindly scan 1024/512/256 against evidence from a different output configuration.
