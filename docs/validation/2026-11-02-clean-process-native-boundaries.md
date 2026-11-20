# Current clean Process native boundaries and actual SRC archive

Date: 2026-11-02, Asia/Shanghai. Source: `9b1af6c` on
`feat/minimal-io-latency`, built from an independent `git archive` excluding
the original eighteen dirty-file changes and nine cache files. This is one
finite acquisition and two offline analyses, not S1-S3 completion, an audio
quality PASS, a calibrated latency result or a lower-default promotion.

## Acquisition and isolation

Remote target: `LAPTOP-NP49RG6H`, SSH session0 elevated. No native audio client
or GUI runs on the development host. Host headless regressions run concurrently.
The reviewed runner reuses the successful
[measured-content lifecycle](2026-11-02-measured-process-content.md), adding
only the current application pin and opt-in native PCM/cursor archive.
The PS5 non-audio selfcheck passes230checks; its only child is probe `--help`.

Remote root: `D:\MoirenTest\20261102-clean-src-native-1620`.
Host evidence:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-clean-src-native-20261102-1620`.
Application SHA256:
`8776B5EFDFF383619F4D201F52329FF47499A2DA2538647099B31CEA9BFBC06D`.
Probe SHA256:
`AB03BE0D3FA9C6230B31E70E33C19EF535382A214E2CF48CEB5370038F1B1685`.
Loaded SYS SHA256:
`4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.

The graph remains Process capture -> unity Bus -> DAWN PRO2. Source is the
pinned deterministic indexed-v2 worker playing to separate Voicemeeter In2:
48kHz/stereo,1024-frame codes, amplitude0.025,40s/1920000frames/39codes.
Engine64/F32, Shared requested336/actualcapacity738, render512/wake64/callback64
remain fixed. After10002ms Compatible baseline, owner2 runs20001ms at measured
target1536/capacity16384/jitter96. Observed admission uses packet480/gap526,
prepared demand704. Both owners retain
`--cursor-trace-directory NEW --cursor-trace-pcm-frames 65536`.

The run starts16:20:51.255 and finishes16:21:33.934. Source PID14588/creation
134380812524472955 is pinned. Baseline cleanup/reap precedes owner2 start.
Two exact-process independent recorders run12s after replacement; they retain
575520 source and575568 graph frames. Recorder launch is not a readiness
protocol; these are independent stored coordinates covering only a subwindow.
Their designed exit1 is operational completion, never qualification. Source
and monitor exit0; source submits the complete schedule and stops/join normally.
All held owners exit without forced termination.

## Native PCM and actual interpolation replay

The unchanged independent `process-native-pcm.py` runs remotely in the pinned
isolated Python environment with `-I -B`, exits0 at16:23:09, and audits both
owners as structurally complete. It uses actual f64 phase/step recurrence,
generation, accepted neighbors, recovery fade and f32 cast. No synthetic FIR,
constant-phase hypothesis or production SRC fit substitutes for the archive.

| Owner | Native packets | Offered = accepted frames | Cursor events | Reconstructed frames | UNKNOWN / priming |
| --- | ---: | ---: | ---: | ---: | ---: |
| Compatible baseline | 1000 | 480000 | 14930 | 477760 | 3264 |
| Measured1536 | 2002 | 960960 | 29978 | 959296 | 2240 |

Both archives close without diagnostic loss, IO/cap/sequence refusal, native
drop tails, nonfinite normalization, native silent flags, discontinuity or
timestamp-error packets. UNKNOWN storage is not certified silent audio.
Structural completeness does not mean every requested frame was reconstructed.
Owner directories remain distinct despite backend-local generation1 reuse.

The measured bridge retains zero drops, silent frames, underruns, resets,
backlog/age/ingress-elapsed recoveries, with2240priming and384prime-discarded
frames reported separately. There are **7 delivery-gap bound violations**,
maximum11.1377ms, and **1 render wait timeout**. Those observations prevent
a zero-fault or whole-run quality PASS. They do not establish an audible
failure or a new production defect; no audio behavior is modified from them.

## Boundary identity diagnostic

One remote offline run16:32:48.909-16:32:54.176 invokes the original pinned
`analyze-indexed-pcm`. Every eligible component competes against all39 source
codes with unchanged correlation0.7, onset/identity margin0.05 and RMS0.0001,
including quarter gates. Work584183808 is pre-admitted under the aggregate
2500000000 and per-child256000000 caps. Identity is never assigned from signal
order, source schedule position or a strongest rejected candidate.

Native `packets.jsonl` is explicitly projected to the detector's legacy fields:
`packet_frames -> frames`, `stored_before_frames -> stored_start_frame`, with
original sequence/flags/device-position/QPC unchanged. It is not a fabricated
schema3 capture-history header or a valid physical clock. Both raw native and
accepted arrays are hash-equal before this projection is used. Raw metadata
and all detector outputs/packet issues remain retained.

| Boundary, per channel | Components | Resolved once | Accepted zero-based identities |
| --- | ---: | ---: | --- |
| Native baseline | 10 | 10 | 0-9 |
| Native measured1536 | 20 | 20 | 10-29 |
| Independent source recorder | 12 | 12 | 11-22 |
| Independent graph recorder | 12 | 6 | 17-22 |

Both channels agree, but the source submits the same sample on each channel,
so they are not independent measurements. Every accepted native/source
confidence is1.0. Graph components0-5 remain `missing` under the nominal
template: strongest candidates11-15 fail quarter gates; candidate16 has
confidence0.692404 and onset margin0.043497, so its quarter test short-circuits
and is not independently observed. These are **unaccepted candidates**,
not established identities or missing-content counts. Graph components6-11
resolve17-22, confidence0.825301-0.969940. All components are analyzed, no
cross-batch duplicates occur, and packet diagnostic issue lists are empty.

Review identifies that the script's `unique_windows` field counts batch-local
uniques before cross-batch exclusion. The script/output are preserved without
rerunning. `postflight.json` independently asserts zero duplicates and equality
with `resolved_once_identity_set` for all eight channel rows. This dataset's
counts are unaffected; future reuse must rename that field or exclude
duplicates before treating it as a resolved-once count.

This evidence places intact submitted marker identity at current native
capture ingress, while nominal-template disagreements remain downstream.
It does **not** attribute the disagreements to loss, normal SRC, phase, Engine,
render or the independent recorder. The closed actual replay is available for
that next correspondence test. Marker-to-SRC sample linkage and independent
recorder-to-replay mapping remain explicitly `not_established`. No phase/gain/
rate fit, physical delay percentile or new native retry is performed here.

## Verification and artifacts

The exact clean export passes the requested5zero-refill,51marker/rate/
qualification and20measuredUI tests. Python native replay32, section11 and
planner9 tests also pass. The first native-Python test invocation with `-I`
cannot import NumPy from the host user site; that dependency failure is retained.
Its corrected finite headless run uses the existing NumPy2.4.1 user environment
without `-I`. Remote analysis uses the separately pinned isolated environment.
These focused checks do not replace a fresh integration gate for future code
changes; no production source is changed by this receipt.

The compact ZIP contains87 size/SHA256-verified files,930011bytes, SHA256
`B017183FF72B29B062368E241E2567F4398A5A7001CA7D346ECE782ECC0A2B7B`.
Raw35,662,216bytes/620files remain remote; the16:39:37 final read-only audit
rehashes all620, confirms no native/analysis owner, unchanged protected
OBS/QQMusic creation identities, running SYS and application pins. Acquisition
before/after catalogs confirm unchanged defaults and render controls.

Reviewed visualizations:

- Remote `boundaries/boundary-identities.png`, SHA256
  `6BC88D314037CF7E4B8E5C5F6669AE480317E98A666A9E01D29D4376E037FCA3`.
- Remote `pcm-audit/owner-0002-generation-1/overview.png`, SHA256
  `9B746EF425B9E77763007787E7D97F42B867842D239EB0EE4285F19801CAAEE0`.
- Host `boundary-results-readable-v2.png` re-renders only the verified compact
  summary, without detector/native rerun. It merges identical channels and
  marks rejected candidate labels with `?`, SHA256
  `18587CA2779D9C7283EEBBA3D0077BE3BA297B532ABBBCF113A70E827DC42072`.

All27 original dirty/cache hashes and their original status are preserved;
main remains clean. No push or memory update occurs. Independent development
can proceed without another unchanged soak; this finite archive supports later
offline correspondence work. First-packet work stays deferred, Compatible
defaults stay unchanged, and native/physical qualification remains open.
