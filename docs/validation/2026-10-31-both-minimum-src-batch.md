# Both-minimum PCM: frozen complete-support model diagnostic

## Result and limits

**All29 retained support windows have distinct model-consistent original code
identities; audio acceptance does not change.** The [both-minimum recording](2026-10-31-event-arm-both-minimum.md)
keeps its nominal14-unique/15-missing result,9686 native-QPC policy violations
and incomplete runtime quality. Without another native acquisition, the
unchanged [synthetic FIR/affine model](2026-10-31-indexed-src-synthetic.md)
classifies all29 frozen windows as29 distinct original source codes. This
supports waveform transformation as an explanation for the nominal identity
mismatches, rather than treating the15 missing classifications as proved lost
source markers. It does not reconstruct the actual SRC/servo history or prove
whole-waveform continuity, packet/runtime quality, source origin or physical
latency.

Evidence root:
`D:\coding\moiren-evidence\20261031-both-minimum-src-batch`.
Repository base is53117f5. The local evidence harness imports unchanged
repository algorithms; no production Rust, model algorithm, probe CLI, library
API, deployed executable, driver, buffer/default or quality gate changes.
No remote/native operation or physical rendering occurs in this diagnostic.

## Frozen complete set and explicit aggregate budget

Before model execution, `hypothesis-freeze.json` selects **all29 pre-existing
unlabelled support windows**, exactly matching the original nominal analysis
config, not an outcome-selected subset. Every window competes all29 exact
`SourceSchedule::prepare/indexed_marker` codes. Recorder support order is not
used to label source identities, fit an origin or reorder results.

The existing model grid remains quarter phases0/1/2/3,
scale-4000/-2000/0/+2000/+4000ppm, one/two nominal production FIR passes,
correlation0.7, margin0.05 and RMS0.0001. Its section-local means and two-frame
same-code coordinate grouping are unchanged; these are not identical to the
nominal CLI's matching semantics. No threshold, grid, window or model is
tuned after observing the batch outcome. The earlier
[stable-binding window0 diagnostic](2026-10-31-stable-binding-src-window0.md)
uses a different recording and remains a separate single-window result.

The complete batch is explicitly admitted at6937296480 conservatively charged
correlation samples under a separate7000000000 aggregate cap. Each of29 model
calls remains within the unchanged256000000 per-call cap:235894816 minimum,
241102752 maximum. All window shapes, ordered/nonoverlapping bounds and the
checked aggregate sum are validated before the first correlation search; the
entire PCM is also checked for nonfinite/clipped samples. This is an explicit
offline batch budget, not a claim that the whole analysis fits the original
256million cap or a wall-clock guarantee.

The local wrapper reconciles all per-window results globally. Duplicate
accepted source indices in different calls invalidate every corresponding
global identity/coordinate; unresolved/ambiguous results never acquire an
identity. Immutable individual model reports remain in the output. Original
indices are retained even when recorder windows are reversed or identities
are absent; there is no peak-order renumbering.

The seven new local bookkeeping tests cover inclusive aggregate admission,
per-call and aggregate refusal, later-window bounds/grid validation, original
indices, cross-call duplicate ambiguity, unresolved results and malformed
model outputs. The initial stub run exits101 with four expected positive-path
failures and three rejection-path passes; the latter are not claimed as RED.
After implementing only bookkeeping, all44 isolated tests pass, including
the unchanged19 model fixtures. The final executable is built and pinned
before a single finite local batch invocation. Marker/rate modules are copied
byte-for-byte for nested module resolution; the source/model/production FIR
are imported directly, with the existing test-style value-only aliases.
The inherited isolated package/executable name remains
`stable-binding-src-window0`; the explicit `frozen-batch` argv, new root,
frozen inputs and hashes identify this experiment, not the inherited name.

## Single batch outcome

The optimized harness runs once over14:58:00.601723-14:58:08.487129UTC and
exits0 without timeout. That exit means only the declared model identity set
is complete; wrapper `qualified=false` remains explicit.

- All29 global statuses are `Unique`; the resulting original source indices
  are0 through28, each present once. That observed order is an output, not
  the input labeling rule. No cross-window duplicate is accepted.
- Model scores range0.89441793769113-0.9901615238035749. Each accepted
  model-body interval spans at most two stored recorder frames and lies
  inside its frozen window. The first is32843-32844; the last is
  1375999-1376001. These are not physical onset or calibrated uncertainty.
- All14 identities already accepted by the original nominal model agree in
  index; their nominal coordinates fall inside the new model intervals.
  The remaining15 now have model-consistent identities, not a rewritten
  nominal verdict or measured transformation history.
- Every individual report retains `source_origin=unknown_not_estimated` and
  `packet_quality=not_evaluated_no_complete_set_claim`. Global
  `model_identity_set_complete=true` is deliberately distinct from an
  accepted identity/continuity/quality result.
- The model is nominal FIR followed by affine interpolation, not an exact
  reconstruction of two independently varying SRC phases/rates or servo
  trajectories. No best-fitting parameters are exported as measured facts.

The original PCM, nominal config/report, packet metadata, native report and
collection receipt remain byte-identical. Stored coverage is complete, but
all9686 native-QPC diagnostic policy violations,14 unknown boundary
observations, initial prime32 and lifecycle counters remain visible.
`QualityComplete=false`, `identity_set_complete=false` in the nominal report,
and `qualified=false` are not overwritten. Source submission and positive
model correlations do not certify the intervals between markers or the
physical route.

## Provenance and verification

- PCM11647872 bytes, SHA256
  `93A50AF454280896A515FB988535DE43CB00C636DA5BE0FFF36E7916F6DD7F70`.
- Frozen hypothesis7696 bytes, SHA256
  `B5884A48E8220FDA5911D9A8BBFA6865BCF5C343BAC2A5268BF4945D36620F20`.
- Pre-execution pin receipt1604 bytes, SHA256
  `2B70FBCD61847AB58969BE2A2766030047121846D4685C797036B04741D1CD42`.
- Executable SHA256
  `A855CEE85717D156513A1B3DF116DBDA10583C0696D54B5E844B1D31EBEDF545`.
- Model report18920 bytes, SHA256
  `3EABB722944C1CF053278CDEB9B5A9745FE36AC4D6C8BB4400F9D6277B99AD77`.
- Execution receipt476 bytes, SHA256
  `374C7089B20D0B9662868C8986F09A7EC6277D7DE3B4BF32B511EC96AC265A4F`.

All14 frozen source/evidence inputs and all six pre-execution executable/
harness/config pins are rechecked after evaluation. Independent32-check
review recomputes total work and confirms global/per-window identities,
coordinates, nominal agreement and preservation of quality/packet refusal.
All44 isolated tests pass after the final harness is assembled. Its release
build retains two unused-import warnings from unchanged imported test source;
strict isolated test Clippy exits101 on the existing imported
`chunks_exact_to_as_chunks` lint. Neither is suppressed or called a pass.
Fresh actual-repository checks separately pass19 model tests (608 filtered)
and strict all-feature probe/test Clippy. These are host checks, not another
native observation or a new production fix.

## Consequence for S1-S3

This closes the narrower question of whether the retained nominal15-missing
classifications necessarily indicate absent marker codes: the fixed model
finds all29 original identities in those same frozen support windows. It does
not close the required changing-signal/clock-handling and continuous-waveform
acceptance gate. Packet-QPC provenance, transient mapping/servo behavior,
complete boundary quality, a matched preceding-arm/rebase witness, native
verification of the final retired-QUERY repair, sustained lower-budget
delivery, matched VAD/Process comparison and calibrated physical acceptance
remain open. S1-S3 are not complete; no device-floor or impossibility claim
follows.

The subsequent [exact interval-spacing diagnostic](2026-11-01-indexed-spacing.md)
uses these29 frozen model intervals without another correlation search. Their
406 pair constraints rule out a single positive constant-affine mapping, not
actual clock drift, SRC history or physical delay. All original refusals and
incomplete quality remain unchanged; time-varying mapping versus model-coordinate
bias needs separate validation.
