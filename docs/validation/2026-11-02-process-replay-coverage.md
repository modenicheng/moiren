# Frozen actual-replay coverage over the entire OBS return

## Scope and algorithm

This S1 increment extends [actual-replay association](2026-11-02-process-replay-obs.md).
It changes offline Python analysis only, not Rust, the driver, queue/defaults,
admission, SRC, recovery or first-packet policy. Native audio and devices are
not opened on the development host. No new live recording or stability scan is
needed: the retained native PCM smoke is reused on the remote test machine.

`scripts/process-replay-coverage.py` first invokes the existing complete native
audit, independent full-track decode/hash binding and fresh isolation control.
It retains every old fixed-window verdict. The earliest uniquely compatible
anchor per owner freezes both channels' recorder coordinate, rate, gain and DC.
Later anchors check that map at both window endpoints; disagreement exceeding
0.25 frame refuses the owner's map. They cannot introduce a coordinate jump or
new gain fit. Actual production SRC history and samples stay fixed throughout.

Every decoded return frame is in the denominator, including pre-route audio,
training, silence, UNKNOWN/priming and unsupported tails. Disjoint intervals
cover frame0 through the final frame without omissions. Each block is at most
64 frames, additionally split at training and half-second aggregate frontiers.
Both original interpolation neighbors must be valid. Native generation changes
are read from actual cursor spans and refuse crossing support, including a pair
straddling a generation edge. Full owner directory identity, not a reused native
generation number alone, distinguishes sessions. An owner without an anchor
is explicitly unanchored; silence does not manufacture one.

No parameters are fitted during scanning. Original NCC>=0.99, local NRMSE<=0.1
and half-second NRMSE<=0.05 bounds are unchanged. A half-second context that
cannot be evaluated refuses its otherwise compatible local blocks, rather than
bypassing the aggregate gate. The 0.25-frame anchor check is additional refusal
logic, not a relaxed waveform gate. Training frames are identified separately
and never reported as independent coverage. Silence cannot prove continuity.

Storage remains subject to the reused separate512MiB aggregate source and
derived budgets. Generation lookup uses binary search, not a scan of every
edge for each sample block. At4096 detailed category runs, remaining frames are
explicitly `analysis_cap_refused`, never omitted. Plots batch category intervals
instead of allocating one collection per block. Trace display bins are100ms;
the numeric scan still evaluates every block. All outputs are create-new and
must not overlap any source. A single bounded read supplies both parsed manifest
and hash; those exact bytes are persisted in the new output and used for the
association audit. Manifest identity is checked across audit and coverage;
numeric refusals survive post-scan hash or plot errors.

```powershell
python -I -B scripts/process-replay-coverage.py --manifest association-manifest.json --output-directory NEW
```

The manifest is the existing association schema. Exit0 means that the coverage
ledger was generated, not that its intervals all passed. All reports retain
`qualified:false`, `waveform_continuity_qualified:false`,
`physical_qualified:false`, `physical_latency_ms:null`. Owner lifetime,
Engine/render epochs, whole-run native quality and physical delay are not
established by a recorder-coordinate model.

## Software verification

Twenty-six regressions pass, including complete Rust-format native archive to
two-track MKV binding, fresh replay, full coverage and PNG. Known fractional
delay/drift, drops, repeats, eight-frame zero insertion, gain jumps, UNKNOWN,
generation crossings, later anchor conflicts, duplicate owners, silence,
nonfinite recording, last-frame accounting and unchanged gates are exercised.
Focused tests also retain RED then green evidence for aggregate gates, unknown
aggregate context, source/output overlap, detail caps and generation metadata.

Independent review found and repaired five actual defects before remote use:
manifest replacement between audit and scan, an aggregate-frontier leak inside
a local block, lost refusal receipts on post-scan hash failure, multiplicative
generation searches and unbounded per-interval plot collections. Each has a
retained failing regression followed by a passing result.
Follow-up review finds the parse-before-hash variant of manifest replacement:
equal later hashes alone cannot bind the earlier parsed dictionary. Two more
RED/green regressions require parsing and hashing the same captured bytes and
auditing an owned byte-identical manifest snapshot. This final input-snapshot
delta is locally verified after the remote run; it is not claimed exercised
remotely and does not change finite immutable evidence's numeric model.

Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
RED/green logs use `s1-coverage-*-1102.log`; related existing suites pass
association21/native32/linear16/isolated12/two-path14. Ruff check/format and
repository formatting checks pass. Rust/runtime files are not changed by this
increment. The final independently exported staged tree passes all121tests
(new26plus related95), Ruff check/format and repository fmt-check. Receipts are
`s1-coverage-final-scoped-intent-1102.json` and
`s1-coverage-final-scoped-checks-1102.log`. Final bounded follow-up review finds
no blocking issue in the manifest-snapshot repair or its documented remote delta.

## One remote offline coverage task

Target: `LAPTOP-NP49RG6H`. Original run:
`D:\MoirenTest\20261102-060101-native-pcm-smoke`.
The dedicated pinned environment from the association increment is reused with
`-I -B`, no shared or system site packages. Source tools are hash checked before
launch. No live audio owner, OBS recording, device operation or buffer scan is
started. A command-length failure occurs before launch; a bounded check confirms
no launch receipt, output directory or owned process before a short file-based
launcher starts the single task. That failure is not hidden or mistaken for an
analysis run.

Remote output:
`D:\MoirenTest\20261102-coverage-output-20261102-070952820`.
Task PID17820 starts at2026-11-01T23:14:26.7369688Z; the final summary is written
at23:14:56.677Z, approximately30seconds later. Stdout reports
`coverage_accounted`, stderr is empty, and a fresh process query finds no owner.
The PowerShell process wrapper records `exit_code:null` despite completed=true;
an exit0 receipt is therefore **not** claimed or reconstructed. No rerun occurs
to obtain a more favorable process receipt.

The entire stereo return has3941376frames at48kHz,82.112seconds:

| Category | Frames | Seconds |
| --- | ---: | ---: |
| Independent fixed-map compatible | 1584128 | 33.002667 |
| Original anchor training | 48000 | 1.000000 |
| Waveform residual refused | 64 | 0.001333 |
| UNKNOWN replay | 3264 | 0.068000 |
| Uninformative signal | 862144 | 17.961333 |
| Outside replay support | 1443776 | 30.078667 |
| **Entire denominator** | **3941376** | **82.112000** |

All24category intervals are retained. Noise and sweep are covered without any
local refit. The later anchor disagreements are0.003396,0.007005 and0.009918
frames against the frozen first map. Owner1 remains unanchored because its
retained samples are all zero. Native generations are1 in both sessions;
that is not proof they are one owner or one rendered epoch.

Read-only evidence inventory separately establishes a predeclared source
changing-signal duration of34seconds:30seconds noise plus4seconds sweep.
The noise reference begins at1790857 and sweep at3326857 in the OBS reference
track; these are not the return's exact boundaries. Original four windows cover
only2seconds,5.8824% of that source duration. Do not divide the full-recording
category counts by that source-coordinate denominator or omit ramps and silence
to manufacture a percentage. Source whole-stimulus origin is1550857, not the
noise origin1790857.

### Narrow sweep-onset refusal, retained

One subsequent bounded read-only diagnostic examines the same frozen map;
it does not rerun association or change thresholds. The refused interval is
`[3329984,3330048)`,69.374667to69.376000seconds. Its actual RMS is5.5755e-8,
error RMS7.5663e-8 and peak absolute error2.5261e-7 of full scale. Following128
frames have actual RMS0.00122823 and error RMS1.9593e-7. A second plot shows
near-zero ramp onset and the following matching sweep, without gain/DC refit.

These values support a low-level onset/model-normalization limitation rather
than established audible audio destruction. They do **not** erase the64-frame
refusal, certify silence or demonstrate full-run continuity. No production fix,
lower amplitude threshold, more permissive gate or new stability run follows.
This issue need not block unrelated development. Startup behavior stays deferred
as requested by the user.

## Retained limits and artifacts

The native run still has8delivery-gap-bound violations. Its binary is the
earlier source-manifest build including preserved dirty sources, not an exact
clean-commit run of this increment. Player receipts namePID24944 while native
archives/cleanup identifyPID16768; waveform agreement alone does not resolve that
process-lifetime provenance discrepancy. A later [historical identity audit](2026-11-02-process-root-identity.md)
proves16768is the launcher parent of24944, consistent with intentional process-tree
capture. The child's exact old creation stamp and rendered epochs remain unknown.
Final queue and packet histories retain
only256events each. Sample-exact owner replacement and Engine/render epoch
boundaries in OBS remain unknown. None of these is relabeled a physical or
whole-native-quality PASS.

Compact files alone are downloaded to
`20261102-coverage-approved-20261102-070952820` under the evidence root.
Remote/local lengths and SHA256 are compared; both plots are visually reviewed:

- `summary.json`,113208bytes: `D3445A4A415828BBF7F5EB0078199E263AC5F3772349C5D2174FE61FB2262518`.
- `overview.png`,139592bytes: `E16B4FC10695D190A3998CDFC26475C3ABB20F6D94EDC37F851B98F06CE34527`.
- `sweep-onset.json`,1176bytes: `E1A6FFDE3196D952FD44662216743D524767459700BA94D00C95E9A2CCDCE958`.
- `sweep-onset.png`,146595bytes: `21F09D45B911F8557451900F7DBD48D5EAF23F3000463DB6001E673A5270E9BE`.

Executed coverage script: `24D2C29FB0B79641972C6793734DA86FA3978F627441DF2069835AF4AE2F8BD2`.
It precedes only the final manifest-snapshot integrity repair. No rerun is needed
to claim that repair's local regressions, and no remote exercise of it is claimed.
Launch/result/source pins are in `s1-coverage-remote-intent-1102.json`,
`s1-coverage-remote-result-1102.json` and `s1-coverage-remote-completion-1102.log`.
The read-only source/timing handoff is
`20261102-readonly-remote-evidence-handoff-20261101-230434560-a903e8e0.json`.
Protected user OBS12648/QQMusic15096 start identities remain unchanged.
Final read-only hashes of the original recording, both WAVs, control, stimulus,
old summary, monitor, capture result and source manifest remain unchanged in
`s1-coverage-remote-final-source-check-1102.json`.
No owned process remains. S1-S3 remain incomplete at their stated native,
provenance, route and physical boundaries; hardware unavailability does not
block the remaining independent software work or justify a smaller default.
