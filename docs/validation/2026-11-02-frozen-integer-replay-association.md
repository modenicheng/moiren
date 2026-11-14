# Frozen integer association to current actual SRC replay

Date: 2026-11-02, Asia/Shanghai. This increment tests an explicit recorder
coordinate hypothesis against the retained
[clean native acquisition](2026-11-02-clean-process-native-boundaries.md).
No new native source, Moiren, recorder, OBS or device operation is started.
It adds a bounded offline helper and regressions, not a production audio fix.
Original nominal-template refusals and the seven delivery-gap violations /
one render wait timeout remain unchanged.

## Hypothesis and algorithm

`scripts/process-replay-frozen.py` tests whether an independently stored PCM
track differs from the exact production SRC replay by one fixed integer frame
offset and constant per-channel gain/DC. Both nominal rates are48kHz; that does
not establish integer alignment or equal clock rates. Fractional offsets and
drift may legitimately refuse this deliberately narrow hypothesis.

The existing half-second association fitter is not directly applicable:
its512-frame training patch and separated middle-half holdout can land in
different silent portions of a1024-frame-per-second marker schedule. This
helper instead consumes the already frozen, unlabelled support components,
approximately1184frames each. It never obtains identities from signal order,
strongest unaccepted nominal codes or source schedule timestamps.

For each window until an anchor exists:

- The first quarter searches all actual replay frames and all complete owner
  directories. UNKNOWN support is excluded. Coarse peak ratio must be>=1.8;
  its48-frame exclusion does not itself establish adjacent-lag uniqueness.
- Five adjacent integer positions, coarse lag+/-2, are compared using only
  both outer quarters. Per-channel gain/DC is fitted on those training samples.
  Exactly one integer lag must have training NCC>=0.99 and NRMSE<=0.05, and
  both channels must select the same lag.
- Every other owner's strongest coarse peak remains a competitor even when
  that owner is internally ambiguous or later training-refused. The selected
  owner must also beat every other owner by the1.8ratio in each channel.
- Whole-window validity and generation checks are structural requirements;
  no held-out amplitudes participate in selecting the anchor. An optional
  generation array is built from actual recorded output spans, never inferred
  from waveform similarity. Owner directory identity remains distinct from
  backend-local generation numbers.

The first training-compatible anchor freezes owner, integer shift, generation,
gain and DC. Its middle-half holdout excludes16-frame guards at both ends.
Held-out NCC>=0.99, NRMSE<=0.05 and maximum local NRMSE<=0.1 use the existing
association thresholds. Local errors are normalized by outer-training RMS.
An anchor's held-out failure cannot cause re-anchoring. Subsequent windows do
not search, fit or update parameters; UNKNOWN, outside support, generation
changes, silence and waveform residuals remain explicit failures.

Inputs are finite, unclipped floating stereo arrays; validity is binary.
Rate must be exactly48000; windows are unlabelled, ordered, complete and
nonoverlapping,256-8192frames, at most64. Owner count is1-8, input-array storage
is capped at512MiB, and aggregate searched positions at128Mi. These are offline
resource limits, not RT or physical guarantees. No physical latency, full-run
continuity or code identity qualification is returned by the helper.

## Software evidence

The missing helper first produces a genuine assertion RED. The initial ten
tests pass after implementation, covering fixed integer shift/gain/DC,
held-out corruption without re-anchoring, a deliberately false first anchor,
duplicate owners/repeated templates, UNKNOWN/generation edges, later drift,
fractional offsets, silence and malformed bounds/types/nonfinite input.

Independent review finds a cross-owner false acceptance: a unique owner could
win after a competing owner's repeated exact matches were discarded as
internally ambiguous. A new non-symmetric regression fails before the fix.
Preserving all owners' competing coarse peaks repairs it. A separate sparse
1024-frame marker/1184-frame-window case verifies the real signal geometry.
All12tests pass; Ruff check and format-check pass. Related independent suites
pass native32/association21/coverage26. No Rust/runtime source is changed.

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-frozen-replay-20261102-1650`.
RED/green logs and pinned deployed helper remain alongside the original outputs.
The original eighteen dirty-file changes and nine cache files are excluded from
this increment and retain their hashes.

## One remote offline execution

Remote output: `D:\MoirenTest\20261102-frozen-replay-1650\output`.
Original source remains
`D:\MoirenTest\20261102-clean-src-native-1620`.
One isolated `-I -B` execution starts17:08:15.761, PID24584/creation
134380840957778494, exits0 at17:08:39.237. Computation takes18.781s.
The held-process wrapper enforces a180s deadline and retains logs on refusal.

All620 original acquisition files and six additional prior-manifest/detector
artifacts are hash checked before and after. Actual native PCM/cursor replay
is freshly recomputed with the unchanged independent auditor; both owners
remain structurally complete. The generation map is constructed directly
from the freshly audited cursor spans, and its nonzero support must exactly
equal the replay validity mask. Original source and graph windows are read
from pinned detector output, with no omitted, shifted or identity-labelled
components.

Both twelve-window recordings fail to establish a training anchor. No held-out
association is therefore evaluated or accepted:

| Recording | First-quarter strongest NCC against measured-owner replay | Best integer outer-training NRMSE | Accepted anchors |
| --- | --- | --- | ---: |
| Independent source | 0.705836-0.998831 | 0.187813-0.864829 | 0 |
| Independent graph | 0.986917-0.999708 | 0.056817-0.139838 | 0 |

The baseline owner remains in the search rather than being excluded by expected
schedule identities. Graph first-quarter agreement is high, but both outer
quarters together exceed the unchanged0.05residual ceiling at every component.
For graph component0, the strongest training lag has shift7328, correlation
0.995901 and NRMSE0.090452. This shift is an **unaccepted training candidate**,
not a measured path delay or recorder alignment.

This result rejects a sufficient fit under the specified constant integer
mapping; it does not prove audio damage, loss, normal SRC behavior or a unique
cause. The independent source recorder captures unprocessed source, whereas
the replay includes production SRC, so that row is a comparison control,
not an expectation that both waveforms must match. Actual replay-to-recorder
association and physical qualification remain open. No threshold is relaxed,
no fractional/rate model is silently substituted and no native retry occurs.

## Artifacts and state

All19 compact artifacts are size/SHA256 verified. ZIP166316bytes, SHA256
`5DD86D41F67BD2A7F8450E447A7D01EA9A43D3B89F5CE4334781FD3D8B74EDEE`.
The reviewed host `training-refusals.png` renders only saved training metrics,
without a new search or fit; SHA256
`C895C122F8A20A40542D6D2B641E236AC3E6933EA2B78C241D4D6C21F15EBE8A`.
The remote `association-overview.png` separately preserves all zero-compatible
component rows rather than omitting refused windows.

The17:12:50 final audit re-verifies all626 input hashes, no native/analysis
owners, unchanged protected OBS/QQMusic creation identities and running SYS
hash. The unchanged original27 dirty/cache files remain intact and main stays
clean. No push or memory update occurs. First-packet work remains deferred;
original S1-S3 native/physical qualification is not declared complete.
