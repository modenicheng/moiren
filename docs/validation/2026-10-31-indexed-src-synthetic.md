# Synthetic indexed SRC model, not native qualification

`e2b38f5` integrates independently reviewed `d913330`. Four example-only test
files add a finite offline model and synthetic fixtures; the normal probe CLI,
library API, production resampler, driver and deployed binaries are unchanged.
The source codebook uses exact `SourceSchedule::prepare/indexed_marker` bytes,
also checked against continuous indexed-v2 generation and `SourceCursor`.
The test binary imports the actual production FIR with equivalent value-only
dependencies, rather than copying its algorithm or exposing a production API.

The model applies one or two nominal FIR passes followed by bounded affine
interpolation. Its fixed grid has quarter-frame phases and scale values
-4000/-2000/0/+2000/+4000ppm. This is an approximation, not reconstruction of
unknown independent cascade phases or servo histories. All original codes
compete; four sections share one phase/rate/filter/polarity hypothesis.
Correlation0.7, margin0.05 and RMS0.0001 are unchanged. Section-local mean
subtraction differs from the nominal matcher's whole-window mean and must not
be presented as identical matching semantics.

Work admission precedes search, with checked finite code/window/length bounds
and at most256million conservatively charged correlation samples. This is not
a wall-clock guarantee. Rejected-section and subthreshold competitors still
participate in margin checks; competing identities, dispersed coordinates and
resolved duplicate identities refuse uniqueness. Accepted intervals are stored
recorder coordinates, not source origin, physical onset or calibrated timing
uncertainty. Missing/off-grid/time-varying cases have no universal recovery
promise. No complete-set, packet-quality or continuity claim is emitted.

Evidence is retained in
`D:\coding\moiren-evidence\20261031-indexed-src-model`. Author behavioral,
duplicate and work-budget RED logs remain distinct from final GREEN. Root
independently reruns17 focused tests,605 full example tests and strict Clippy
on the frozen author tree, then verifies all four reviewed hashes after
integration and reruns605 full example tests successfully. Author also retains
formatting and Linux-target test compilation; compilation is not Linux test
execution. Independent source review finds no blocking issue and flags the
missing explicit polarity fixtures for a separate test-only follow-up.

The synthetic production-SRC fixtures recover exact nonzero source indices
where the nominal model refuses. They do not analyze or fit retained native
PCM. The existing14/29 native nominal result, all native quality faults and
source-clock unknowns remain unchanged. S1-S3, calibrated physical sub10ms,
and evidence for a current-device theoretical lower bound remain open.
