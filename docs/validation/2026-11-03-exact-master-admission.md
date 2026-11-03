# Exact Master Wake Admission

Baseline: `13a55dc0768cd0b2a8e05756b90458a7e566a4fe`.
This continues S2/S3 implementation after exact-demand Pump rendering. Native
defaults, queues, device modes and driver are unchanged. No native acquisition
is repeated merely to recheck a preparation formula.

## Reproduced Overestimate

The preceding production change stops rendering unused Engine-block tails,
but preparation still advertised this old master-input envelope:

```text
ceil(native_demand * engine_rate / native_rate) + 64 + 2 * engine_block
```

That overestimate reaches Process input admission, physical capture reserves,
VAD provider preparation, published budgets and microphone reset reprime. It
can reject a smaller otherwise admitted target even though the real Pump no
longer performs those rounded Engine calls. Both Compatible provider precision
tests genuinely fail with704actual versus544initially expected. A separate
test fails `InvalidDeliveryBound` because the DTO assumes a whole wake cannot
be smaller than the Engine segment ceiling.

The first implementation conservatively retains a whole FIR32 beyond the
converted demand. Independent mathematical review supports the tighter
HALF16 bound over the actual professional domain. Final code and tests use16;
the earlier32-bound runs are retained with their original logs, not relabelled.

## Bound And Domain

For the nominal physical master, preparation now uses checked integer
arithmetic in the existing resampler module:

```text
B(D) = ceil(D * engine_rate / native_rate) + HALF
HALF = 16
```

The helper accepts only the four professional Engine rates, negotiated native
rates8000..384000Hz and positive native demands1..65536frames. Other inputs
refuse. The Engine ceiling is not a reserve term. A follower's corrected
ratio must not use this nominal-master bound. Zero native runtime demand still
does no work; a zero preparation envelope remains invalid.

Let `p` be local FIR position, `L` its retained length, and `r` the nominal
Engine/native ratio. Initially `p=L=16`. Successful output requires
`floor(p)+16<L`. With professional rates,0.11484375<=r<=12; after position
normalization16<=p<17, the discard cap does not bind, and `p-L<=0`. Pushes only
decrease this difference. This remains true across arbitrary reachable phase,
not just an integer-zero-phase wake.

Across repeated exact-demand Engine segments, supplied inputs and discarded
history telescope. For `n` outputs already produced and `D-n` remaining, the
total supplied plus next missing demand is bounded by:

```text
floor((p0-L0) + (D-1)*r + epsilon) + 17
```

Under the validated rate and65536-frame demand limits, a conservative
binary64 error bound is below3e-10frames, much smaller than the minimum ratio
step. BudgetingDsteps rather thanD-1 therefore leaves numerical margin and
gives `<=ceil(D*r)+16`. Equal-rate cold-start demand actually needsD+16, so the
uniform additive16 cannot be removed. This proof applies to the complete Pump
loop, not assumed equality with one bulk `input_frames_needed(D)` estimate.

The helper is not a physical latency estimator and does not reduce the FIR
filter or its retained state. At Engine48k/native48k/render512, prepared
whole-wake demand becomes528 instead of704. Separate Process delivery gap,
jitter, input interpolation pair, VAD drift allowance and input FIR reserve
remain fully charged. With declared gap480 and jitter96, the Process modeled
minimum is1106;1105 still refuses. Historical measured demand remains sticky
through the existing max/rebound logic, so old704evidence is not retroactively
rewritten as528.

## Short Physical Wakes And Recovery

`VirtualBufferOptions::prepare_budget` and microphone reprime now require a
positive whole burst, not `burst>=Engine ceiling`. The latter was only true
when the old Pump rounded up. Zero block, zero demand, invalid rate, capacity,
packet and evidence-integrity checks remain. A physical wake can legitimately
be80 or96frames while scratch is sized to Engine4096.

Pure-virtual scheduling still computes `block * admitted recovery cap` and
keeps its existing eight-period default and all capacity/target guards.
Microphone reprime still reconstructs and compares the complete public DTO
before use. At48k/Engine64/native512, Compatible modeled reset readiness becomes
1042frames instead of1218. This is modeled recovery, not measured residence.

Independent review finds one downstream fail-closed gap: a Target with
Engine4096/native64 has a whole wake80, corrected transfer81 and modeled
minimum689, yet boundary binding still sizes its drift transfer from4096.
Both precisions reproduce refusal at the otherwise admitted689target. Provider
binding now caps the Target segment ceiling at `min(block, whole_wake)` in both
directions. The public request/budget still retains Engine4096 and whole80;
688continues to refuse,689binds, and two complete wakes execute through the
compiled bindings under the allocation/deallocation guard with nonzero MIC
output. No native Start is involved.

The cap applies only to Target. An intermediate unconditional cap changes
Compatible's carrier-derived prime/servo reference; a new genuine RED records
playback8248versus the legacy10260. Final tests pin playback10260 and MIC6124
for Engine4096/carrier16384, and both precision variants pass after restoring
Compatible's original boundary geometry. Compatible's tighter provider/reprime
whole-wake budget remains intentional; its runtime servo target is not tuned.

## Tests And Preserved Failures

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s2-exact-demand-budget-20261103-1421`.
Tests run from a clean13a55dcarchive plus only the intended code overlays.
Host execution is compilation and pure/headless tests only.

- Genuine provider RED: both precisions704versus544 before production edits.
- Genuine short-wake RED:96burst under Engine128refuses with the original DTO.
- Provider/diagnostic/reprime equality covers both precisions, Engine ceilings
 64/256/4096, native64/128/512 and equal/up/downsampled configurations.
- Actual compiled IndexedSource -> unity Bus -> RingSink -> Pump tests retain
 exact timeline/output equality, add the new envelope assertion, and preserve
 zero/partial-write and oversized-demand behavior.
- Fractional-phase FIR tests cover all professional Engine rates, supported
 native rates plus8000/384000extremes, repeated mixed demands and65536-frame
 full-limit output, with exact equal-rate cold/steady counts and reset checks.
- Process admission checks1105refusal/1106acceptance; existing whole-wake,
 historical evidence, identity and evaluated-envelope guards stay active.
- Virtual budget/reprime tests cover a whole96-frame wake under Engine
 128/512/4096; zero burst/block still refuse.
- Integrated Target edges check1137refusal/1138acceptance and688refusal/
 689acceptance in both precisions, including actual compiled graph transfers
 and control-side cleanup. Compatible probe geometry is independently pinned.

The first full-library attempt passes850but fails one old measured-preparation
fixture: after the tighter bound its target256 now fits a native128@96k wake,
so the fixture falls through to `OpenProcess` for synthetic PID42 and receives
`0x80070057`. This is a test-isolation failure, not an audio-device run or
production diagnosis. The fake native period becomes512, making the whole
Engine demand272 and preserving refusal before any real Process capture.
Its assertions still require no native Start and exact parked-owner cleanup.
With that repair, audio library851/5ignored and standaloneRT255pass; final
complete-source integration is recorded separately after closure.

The first new Target fixture uses capacity4800 against a fake provider that
always returns16384; the exact carrier-capacity check refuses. The fixture is
corrected to16384 without changing production equality or the shared fake.
The first short-wake reproduction attempt has a fixture `usize`/`u32` compile
error; the subsequent typed run is the genuine behavioral RED. An additional
runtime assertion first requests more PCM than the fake microphone produced,
creating a test-consumer xrun. Final verification drains exactly the published
available prefix, preserving zero-xrun and nonzero-output assertions. These
original failure logs remain, distinct from the successful corrected runs.

The first complete clean-source feature-workspace run exits0 with2434passed/
10ignored across71harnesses; it predates the new Target runtime tests and
provider repair. Its strict feature-on all-target Clippy exits0. Intermediate
255-test RT runs also pass; none substitutes for the final pinned source run.

Final code is pinned by `final-code-pins-v4.json` over13intended code/test paths.
The separate final-source audio package run exits0 with1169passed/6ignored
across36harnesses, including library853/5ignored and virtual-budget24. Final
standalone RT255, strict all-target audio and RT Clippy, and workspace/RT fmt
all exit0. The reviewed Target-only fence and Compatible probe pins are present
in this source, unlike the earlier integration iterations. Final feature-workspace
and release results are recorded below after their original handles finish.

The independent bounded review closes its P2 after inspecting the Target-only
provider repair and tests. It finds no remaining code/RT blocker in this delta;
the reviewer does not rerun tests or touch native/network resources.

No lower target is selected, no old failed capture is reclassified, no physical
qualification is claimed, and deferred first-packet work remains untouched.

## Final Integration And Preservation

At15:10:09, final pinned feature-workspace integration exits0 with2436passed/
10ignored/0failed across71harnesses, including zero-refill5, measuredUI20 and
marker/rate/qualification19/8/24. Feature-on strict all-target workspace Clippy
and workspace fmt exit0. Final RT255 and audio1169results above use the same
13code pins. The intermediate2436workspace run is kept separately and is not
misrepresented as the final Target-only source.

The ordinary release build exits0 at15:10:59. Binary:
`D:\coding\moiren-evidence\cargo-target-minimal-latency\release\moiren-app.exe`,
34249216bytes, SHA256
`D6965020D88086AF86E816A2BD48AB73BA78076ECC91E1F1BEB84C20B40D2AF7`.
It is built only; no host-native execution, remote deployment or new driver
package is performed for this preparation-only question.

All27preexisting dirty/cache file hashes remain unchanged; unrelated
main-worktree `docs/audit/2026-11-03/` is preserved. Only the13listed code/test
paths, this receipt and the existing S1-S3 ledger belong to the local increment.
No push is performed. S1-S3 native/physical acceptance remains incomplete;
unavailable physical equipment and deferred first-packet work do not block
independent development.
