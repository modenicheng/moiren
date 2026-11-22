# Stable-binding PCM: one frozen SRC-model window

## Result and limits

**One nominally unresolved window has a model-consistent identity; acceptance
does not change.** The [stable-binding recording](2026-10-31-event-arm-stable-binding.md)
retains its original nominal13-unique/16-missing result and9611 native-QPC
policy violations. Without another native run, the existing finite
[synthetic FIR/affine model](2026-10-31-indexed-src-synthetic.md) resolves its
first unlabelled support window as source code0. This supports transformation
as a plausible explanation for this nominal mismatch, not proof of the actual
SRC history or absence of loss elsewhere. It is not a new complete-set,
continuity, packet-quality, origin or physical-latency result.

Evidence root:
`D:\coding\moiren-evidence\20261031-stable-binding-src-window0`.
Repository base is60696bd; no repository Rust, deployed binary, driver,
production transport, quality gate or CLI is changed by this investigation.

## Before execution

`hypothesis-freeze.json` selects support window0, stored start33319 and1211
frames, before model execution. It is selected by recorder support order only,
without assigning source identity. Every one of the29 exact prepared indexed-v2
codes competes. The single-window scope admits233290848 conservatively charged
correlation samples under the unchanged256000000 cap; the experiment does not
silently split all29 windows into unbounded aggregate work.

The existing fixed model grid is unchanged: quarter phases0/1/2/3,
scale-4000/-2000/0/+2000/+4000ppm and one/two nominal production FIR passes.
Correlation0.7, margin0.05 and RMS0.0001 stay fixed. The model uses section-local
means and a maximum two-frame grouping of accepted same-code hypotheses;
these are the existing synthetic model's semantics, not the nominal CLI's
identical-onset/matching semantics. A better model score is not a lowered
nominal threshold or permission to replace the original verdict.

The local evidence harness imports the unchanged repository `source.rs`,
`src_model.rs` and production `resample.rs`. The existing two value-only
dependencies are the same test-style finite conversion/stereo frame aliases.
Marker/rate modules are copied byte-for-byte solely to preserve nested module
resolution outside the workspace; their hashes match the frozen sources.
No resampler or codebook algorithm is rewritten. Native clients are never
constructed; only archived PCM and schedule bytes are read.

The first local fixture build fails module-path resolution, before any native
window evaluation. Moving only the byte-identical marker/rate modules into the
local module layout restores compilation. All19 existing model fixtures then
pass before the one frozen window evaluation. The final harness/executable,
dependency lock and hypothesis hashes are pinned in `pre-execution-pin.json`.

## Single offline evaluation

The local optimized harness evaluates the frozen window once and exits0:

- Model status `Unique`, exact `source_index=0`, score0.9649951231785059.
  The original nominal best for this window is0.5699170189192984, rejected.
- Accepted model-body interval is stored recorder frames33413-33415. It is
  not physical onset, a source-to-recorder origin or calibrated uncertainty.
- The model report retains `source_origin=unknown_not_estimated` and
  `packet_quality=not_evaluated_no_complete_set_claim`; wrapper `qualified=false`.
- No successful model parameters/servo history are exported as measured native
  facts. Linear affine interpolation after nominal FIR is not reconstruction
  of unknown cascaded SRC phases, rates or time-varying correction.
- Other windows are not evaluated and no complete-set result is inferred.
  There is no native rerun, grid/window widening, retry or fault suppression.

PCM11649024 bytes SHA256:
`61C730088143EFFC8317CA0FB1117896A072160F6CE451E61CF42C019F41221C`.
Frozen hypothesis4261 bytes SHA256:
`B8E22FA98FD91C99592EA16128C1E9CA137AC897A7C9A67C5BC52F32ABF8F75A`.
Model report746 bytes SHA256:
`196CE896614A356FA3CBAB724A86211641AE13E80410A819CDB4C9395DE62BE1`.
Execution receipt433 bytes SHA256:
`3343BFF8935AFD4A34E86A739061546210F306AFE553C01BA2B05B1154B4E8DB`.
All12 frozen source/evidence inputs and the two copied modules are reverified
after analysis. The predecessor's nominal config/report/PCM remain unchanged.

## Verification and remaining work

The local isolated harness passes all37 tests, including the19 existing model
fixtures. Its strict test Clippy fails on `chunks_exact_to_as_chunks` in an
unchanged imported test and the local reader; normal local build also retains
two unused-import warnings from imported test source. These failures/warnings
are preserved, not suppressed or called a pass. No source edit is justified
from that isolated lint outcome alone.

Fresh checks in the actual repository independently pass all19 model tests
(608 others filtered) and strict all-feature probe/test Clippy. The command
receipt named `root-clippy-red-receipt.json` actually records exit0; its filename
is an expected outcome, not evidence of a genuine RED. No new regression or
production fix is claimed. Earlier synthetic-model RED/GREEN history remains
separate.

This bounded diagnostic prevents treating one nominal mismatch as proven loss;
it does not certify every identity or waveform. Any broader analyzer extension
needs explicit aggregate work limits, original-code/ambiguity competition,
validated transformation coverage and preserved packet/runtime failures.
S1-S3, native verification of the final retired-query repair, a preceding-arm
fault witness, sustained lower-budget delivery and calibrated physical
acceptance remain open.
