# Bounded indexed-v2 identity diagnostic

## Implementation and verification

`910cfb9` integrates independent author commit `49c5888`. This is an offline
probe-only slice: separate `analyze-indexed-pcm` command, bounded module and
tests, read-only prepared-codebook/packet-evidence accessors. Existing v1
generator bytes, paired-channel analyzer, driver, production transport and
native collection behavior remain unchanged.

The caller supplies the exact indexed-v2 source schedule and explicit unlabelled
recorder-coordinate windows. Every original schedule code competes in each
window; detected-peak order never assigns identity. The detector does not fit
source origin, resample a codebook or manufacture an independent reference.
Only complete48kHz stereo Float32 stored-PCM frames and explicit frame-zero
coordinates are admitted. Input/schema/frame/window/metadata/packet/search-work
caps are checked; JSON output is create-new, never overwrites an input.

Each accepted candidate must pass global correlation, four-section coverage,
onset and codeword competition. Weaker valid codewords/onsets remain ambiguity,
even when their score is substantially below the strongest peak. Missing,
duplicate, reordered and ambiguous identities, invalid PCM and packet faults
remain explicit. Relative spacing is recorder-coordinate evidence, not physical
delay or calibrated clock drift. Whole-recording absence/continuity is outside
the supplied-window scope. All output remains `qualified=false`.

Author retains genuine initial detector, CLI/competition and weaker-duplicate
RED cases before GREEN. Author focused21/full574, strict all-feature example
Clippy, formatting and Linux-target test compilation pass. At integrated910cfb9,
root independently runs focused21/full576, strict example Clippy, formatting and
Linux-target example/tests compilation: all pass. The differing full counts
belong to different base revisions. A cross-target check is not executed Linux
runtime verification. Evidence root is
`D:\coding\moiren-evidence\20261031-indexed-marker-analysis`.

## Retained native recording: partial identity, no pass

The input is the independently verified
[74f indexed-v2 recording](2026-10-31-indexed-marker-source.md#first-native-indexed-v2-acquisition),
not a new native run. Its PCM SHA256 is
`89FA05D51A36743616356692FA847968FEEADDB0C0509B4A46914309FAAC3B06`.
The29 unlabelled windows derive solely from nonzero PCM support, separated by
more than32 zero frames and padded64 frames on each side. Each window competes
against all29 exact1024-frame codes. This support heuristic is not source
identity, a whole-recording detector or an origin mapping. Root's separate
threshold-morphology windows are not silently substituted.

Explicit thresholds stay correlation0.7, peak margin0.05 and RMS0.0001. Work
admission is163832832 correlation samples under the256000000 cap. Config SHA256
is `278B90FAFCA667C428FBF171DB6E8A027441A8F1C3B3EF75943DC7FC22671391`.
No threshold is lowered to improve the result.

Author and root-integrated CLI runs produce exactly equal parsed JSON and exit1:
14 unique identities and15 missing, no duplicate. Eleven missing candidates
fail global correlation; four exceed0.7 but fail section coverage. Fifteen
missing classifications do not prove15 dropped markers: filtering, SRC or
other transformation remains unsupported by this exact nominal-template model.
Six adjacent resolved identity pairs yield relative spacing only. Source origin
and native epoch remain unknown, and no latency distribution is produced.

Stored frame/sequence coverage is complete, but9762 packet intervals violate the
existing nominal-rate diagnostic policy (one-frame plus1000ppm tolerance).
These are not relabelled as9762 lost frames, recorder gaps or calibrated drift.
The report retains the policy issue and `identity_set_complete=false`. Clean
native packet flags and earlier operational exit0 cannot override this result,
the recorded tail silence or explicit incomplete native quality.

Integrated diagnostic JSON SHA256 is
`104627BEAEF5C0ECD4B9EE86FDEE95D11053A16920F9A9A3374FD0CA5296A352`.
Author final receipt SHA256 is
`A134C600C249B766AEDF0B514789BED49D5DD57A37A56993093FAD633DC3A28C`.
All originals and negative results remain retained. No driver/default update,
lower-budget promotion, continuous-waveform or physical latency pass follows.

## Separate decoded-driver evidence

Root decodes the original indexed-v2 ETL using verified matching2a5aee5 PDB/TMF:
159 processed events, zero reported loss/format errors/unknown formats. The
separate independent addendum passes27 checks; its JSON SHA256 is
`D0ACEF18C8180385B4DC855EE2B5E832BCE7871F7032B6DE575B85D70BCF199A`.
The original64-check acquisition audit remains unchanged.

The exported bounded native/read diagnostics cover480000 frames/1000calls with
zero different copied bytes and matching endpoint/session/generation/gate.
They do not provide whole-run paired sample alignment. No rebase or active-MIC
unserved messages are exported in this decode; bounded/reset/passive-drained
instrumentation cannot establish that no such event ever occurred. Cleanup
removes both endpoints and retains MIC silence1. Its inclusion in recorder PCM
is still unknown. S1-S3 and calibrated stable sub-10-ms qualification stay open.
