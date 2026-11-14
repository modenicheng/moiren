# Measured Process frozen-component identities

Date: 2026-11-02, Asia/Shanghai. Baseline: `c6a34a3`. This closes a bounded
diagnostic gap in the [post-replacement content acquisition](2026-11-02-measured-process-content.md):
the first673-frame graph component previously prevented the unchanged detector
from analyzing the other eleven complete components. The old whole-channel
admission refusal remains unchanged. This result does not accept the capture,
prove whole-waveform continuity or measure physical latency.

## Scoped planner and regression

`scripts/indexed-support-plan.py` is a standard-library-only work planner; it
opens no audio or PCM, invokes no detector and produces no PASS. Every original
component retains its index and exact recorder-coordinate window. Input schema,
integer types, count, order and nonoverlap are validated before local refusals;
bools, floats, duplicate JSON fields and unknown fields refuse. Short, oversized
and per-child-work-cap refusals stay explicit components. Eligible windows are
grouped with the unchanged Cartesian charge:
`(window_frames - marker_frames + 1) * marker_frames * code_count`.
Aggregate overflow refuses the entire dispatch plan, retaining all components;
it cannot silently choose an affordable suffix. CLI input is bounded to1MiB
and output uses exclusive creation. Both `qualified` and `partial_can_pass`
are always false, even when all windows are eligible. Pending planned work
is not described as completed detector execution.

The initial focused regression genuinely fails because the planner is absent.
After implementation,9 tests pass, including mixed673+eleven eligible windows,
all-refused cases, exhaustive original-index accounting, exact work/batch/cap
boundaries, ordering/overlap, strict types/schema, input immutability and output
overwrite refusal. The parent independently reruns all9 tests successfully
with `python -I -B scripts/tests/indexed-support-plan.tests.py`.
All16 related indexed-spacing regressions also pass. These are25 software
tests, not native tests; no Rust source changes require a workspace rerun here.

## One closed remote analysis

Native source material is exactly the earlier40s schedule and12s source/graph
recordings: Process measured1536/capacity16384/jitter96 -> unity Bus -> DAWN
PRO2,48kHz/F32/Engine64,Shared336/capacity738/render512/wake64/callback64.
No new source, Moiren, recorder, OBS or audio/device process is started.
All14 original raw file sizes/hashes and the original summary hash are verified
before and after analysis; raw PCM and large logs remain remote.

The isolated offline run uses the pinned existing Python environment,
`-I -B`, single-thread numerical libraries and an output-local plotting cache.
Runtime planner, analyzer, intent and probe hashes are checked. All four
channel plans are budgeted before detector launch; then nonzero support is
recomputed from unchanged PCM using the exact old split/padding rule and must
equal every frozen original coordinate. Static plans and detector execution
records are retained separately. Refused components never receive invented
scores or identities. Only eligible components are submitted to the unchanged
Rust detector, searching all39 original codes at .7NCC/.05margin/.0001RMS.

One task runs15:19:32.5628601-15:19:38.4353672, PID16912/
creation134380775725810070, normally exits0 with no forced cleanup. Analyzer
computation is2.141s and charges exactly264,775,680 correlation samples,
within the2,000,000,000 aggregate and256,000,000 per-child bounds.
Its original300-second lifetime guard is not extended. All four raw detector
commands exit1 under the unqualified/incomplete-codebook contract; those
verdicts are retained. The wrapper completes its diagnostic, not qualification.

## Results

Both stored channels agree within each recording. Channel agreement is not an
independent reference: the source intentionally sends the same sample on both.

| Capture, per channel | All frozen components | Detector result | Remaining refusal |
| --- | --- | --- | --- |
| Source output |12, each1152frames |12unique identities11-22 |None within those windows; full39-code schedule is not covered |
| Graph output |12: one673, eleven1183/1184frames |11analyzed:9unique identities12 and14-21 |Component0NOT ANALYZED; components1and3fail section agreement |

Graph component0 remains exactly start1344/length673, refused `too_short`, with
no padding extension, omitted record or inferred identity. Component1 is
start48871/length1183; its strongest candidate is code11 at48950, NCC
0.7111240782583097. Component3 is start144943/length1184; its strongest
candidate is code13 at145023, NCC0.8652437157700476. Both have
`sections_match=false`, `valid_onsets=0`, raw status`missing` and null accepted
identity/position. These candidate numbers are not accepted identities, proof
of missing samples or audible corruption. Their exact cause stays unresolved;
no new fitting, rate search, threshold relaxation or first-packet investigation
is justified by this result.

Each of the nine accepted graph identities has one accepted source-window
partner on the corresponding channel, established by all-codebook competition,
not ordering. Source and graph recorder frame-zero origins remain independent
and unknown, so subtracting their positions would not establish route delay.
Codes outside a recording subwindow remain outside-or-unresolved. Neither
recording covers the complete39-code source schedule, and no partial PASS is
reported. Source12/12 and graph9/11 analyzed plus1unanalysed do not mean that
the complete native waveform is clean.

The retained packet histories cover1198 source and1712 graph packets with no
artifact overflow/drop/discontinuity/timestamp-error reported. Device-position
gap detection is disabled, every packet's position provenance remains unknown
for this analysis and native epoch is not recorded. Coverage of retained
records is not independently established audio continuity or hardware time.
The earlier measured-owner render wait timeout remains unlocated in run versus
teardown. Original Process18/19, VAD17/19 and whole-channel refusals stay intact.

## Visualization, pins and closure

Host evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-component-plan-20261102-1515`.
Remote diagnostic root: `D:\MoirenTest\20261102-component-diagnostic-1515`.
The reviewed `archive\content-identities-rms.png` shows all original indices,
NCC with detector refusals, accepted identity indices, independent-origin RMS
and every frozen support length. Component0 is shaded **NOT ANALYZED**, not
plotted as a zero correlation result. The lower bars separately show the673
length against the unchanged1024-frame marker requirement. The original
whole-channel **NOT ANALYZED** plot is retained in its own evidence directory.

Analyzer SHA256: `8198B81E178A7AAB64E9EAC4D8525FC8A712840BCC771B314E10A02A93B94B2C`.
Planner SHA256: `EDBB83005FA585E1AA0EB4D125413BA6AC51C3DB47E49E9EAF08C5BF0765F7AA`.
Pinned Rust probe SHA256:
`AB03BE0D3FA9C6230B31E70E33C19EF535382A214E2CF48CEB5370038F1B1685`.
Unchanged original summary SHA256:
`68672CB9B7D764D90349F1B0E0B5DF96F5152258771F2B44045F2666DDEA5957`.
Reviewed plot SHA256:
`2BEF7ED3FCFFCEA4A5322B2139753EA8A4F5B27348A27126D6F114FD9EBDCB67`.
All24 compact downloaded artifacts are byte/hash verified, archive274910bytes,
SHA256 `C5F4D76C3EF5E633156479DDB1DFE0CA563C45A10DBC2C5BECCF5C7E5A09FA65`.
The archive flattens output filenames; the manifest retains original paths.

The15:21:48.8461116 read-only final audit confirms all14raw files and original
summary unchanged, no native/analysis owners and unchanged protected OBS12648/
creation1793505454949 and QQMusic15096/creation1793504036422. This increment
changes no audio/RT/default/driver/admission behavior, uses no new native soak
and does not reopen the deferred first-packet issue. It improves visibility
of already recorded steady content; sustained quality, matched route comparison
and calibrated physical latency remain unqualified. No push or memory update.
