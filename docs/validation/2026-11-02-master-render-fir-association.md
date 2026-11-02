# Production master FIR explains the retained graph waveform

Date: 2026-11-02. Branch: `feat/minimal-io-latency`.
Base: `cdc5c4967b3b63f89fb58e0accf39199a6380c27`.

**The entire retained 11.991-second independent graph recording is bit-equal
to the production master-FIR projection of freshly reconstructed Process
output, using one frozen integer map. This closes this recording's unexplained
linear-replay residual, not native scheduling or physical qualification.**
No production audio algorithm is changed.

## Missing comparison stage

The [previous frozen-integer comparison](2026-11-02-frozen-integer-replay-association.md)
compared Process linear SRC output directly with the independent graph
recording. It omitted the actual render FIR after the unity Bus. In
`src/engine_backend/owner.rs`, every render Pump creates a `StereoResampler`,
including a48000-to-48000master. Only followers receive servo correction;
the master remains nominal step1/phase0. Equal rates do not make the production
32-tap, cutoff0.94 FIR an identity transform.

Previously, component0 at shift7328 had outer-training NRMSE0.090452 and failed
the unchanged0.05 limit. Including the production FIR and Float32 codec makes
the same components exactly compatible. There is no fractional mapping,
rate/phase/kernel fit, threshold change, identity-labelled input, or new native
recording. Prior nominal-code refusals remain historical facts; disagreement
with an incomplete waveform reference is not proof of lost audio.

## Reusable offline projector

`crates/moiren-windows-audio/examples/process_render_replay.rs` includes
unchanged production `resample.rs` and `codec.rs`; Python does not reimplement
their math. Inputs are stereo little-endian Float32 PCM, binary validity and
actual output generations, capped at2880000frames with cap+1 bounded reads.
It creates new output files and refuses overwrite. No native stream is opened.

Ordered f64 FIR accumulation precedes the actual Float32 codec. Output center
`n` depends on input `[n-15,n+16]`, using production initial zero history, not16
leading output-zero frames. Output length is `N-16`, without synthetic tail.
Initial15 outputs and every UNKNOWN or generation-crossing dependency remain
invalid. Process generation changes do not reset the master FIR.

The first behavior test fails against the identity stub:96frames instead of80.
Final14 tests pass, including production FIR/codec tests and new nonidentity,
no-tail, UNKNOWN support, generation, malformed-input and bounded-read controls.
Focused strict Clippy and release build pass. The initial chunks-exact Clippy
warning and subsequent array-reference compile error stay in the evidence;
final checks pass without lint suppression. Two read-only reviews confirm
the mapping and identify the unbounded-read risk, fixed before deployment.

```powershell
cargo build --locked --release -p moiren-windows-audio --example process_render_replay
process_render_replay.exe INPUT.f32le VALID.u8 GENERATION.u64le NEW_OUTPUT_DIRECTORY
```

This is a source-derived projection, not a captured native-write witness.
The summary always retains `qualified=false` and `native_submitted=false`.

## Remote projection and unchanged association

Remote: `D:\MoirenTest\20261102-master-fir-1725`.
Original: `D:\MoirenTest\20261102-clean-src-native-1620`.
Local evidence:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-master-fir-20261102-1725`.

One isolated remote task runs17:36:51-17:37:13, exits0 and computes in15.110s,
with a180s held-process deadline. All620raw acquisition files and six prior
manifest/detector artifacts are hash checked before and after. The independent
auditor freshly reconstructs both Process owners; output-generation maps come
directly from recorded cursor spans.

| Owner | Process replay frames | FIR-computable frames | Compared prefix | Excluded computed tail |
| --- | ---: | ---: | ---: | ---: |
| baseline | 481024 | 481008 | 481008 | 0 |
| replacement | 961536 | 961520 | 961472 | 48 |

Replacement comparison is capped at recorded native-written count961472;
the48computed tail frames are not treated as submitted. Baseline has no such
receipt here and remains an explicitly unwitnessed projection competitor.
A count prefix is not per-submission provenance. The runner's
`comparison_sha256` hashes the full projected file, including the excluded48
frames; it is a file-integrity hash, not the compared-prefix hash.

Original12 unlabelled support windows and both owners feed the unchanged
frozen-integer helper. Component0 alone establishes replacement/shift7328/
generation1, with both channels gain1/DC0. The map freezes before held-out
evaluation; `training_attempts=1`, and the next11 components do not search or
refit. All12 pass; all24 channel held-out and full-window local NRMSEs are0.
The7328-frame offset maps recording origins; it is not measured path delay.

Projector executable SHA256:
`9C3685C3E92CA56FDA6325B114238BDA7331B073264B352A264B721A8FF1BD75`.
Production FIR SHA256:
`BC481E62D0D4A17894962ECB049A05962DBF7AC2B7632CDF6F9A4884C0F32991`.
Production codec SHA256:
`97CE31BFB5444DB768CA87CEBEA5DA51AF5372AE6BBE099D5056CAA9CC523898`.

## Entire retained recording, without another fit

A separate bounded remote audit runs17:41:25-17:41:27, exits0 and uses the
already-saved owner/shift/gain/DC. No search, interpolation or parameter update
occurs. Every recorder frame is checked, not just informative windows.

| Quantity | Result |
| --- | ---: |
| Recorded frames / duration | 575568 / 11.991s |
| Known-support frames | 575568 |
| UNKNOWN / outside-support frames | 0 / 0 |
| Bit-equal stereo frames | 575568 |
| Numerically mismatched frames | 0 |
| Outer-training frames | 592 |
| Nontraining known frames / mismatches | 574976 / 0 |
| Nonzero frames / silent frames | 12668 / 562900 |
| Maximum absolute PCM error | 0 |

Recorder `[0,575568)` maps to projected `[7328,582896)`, wholly inside the
replacement native-count prefix. Both channels match bitwise, but equal stereo
source samples are not independent replicates. Silence is explicitly counted,
not treated as informative marker evidence. This covers only the retained
interval, not startup, unrecorded portions, arbitrary content or other routes.

## Checks, artifacts and stopping boundary

During remote work, clean-export headless checks pass5zero-refill,
51analyzer/qualification and20measured desktop tests. The projector passes14
tests and strict focused Clippy. Full `cargo test --locked --workspace` passes
2179tests with8ignored; workspace all-target strict Clippy and formatting pass.
The release CLI smoke accepts96frames to80outputs/65known, refuses overwrite
without changing existing files, and refuses malformed mapping before creating
an output directory. These are not another native stability soak.

All22compact projection files are size/SHA256 verified; ZIP160399bytes, SHA256
`2A4F5A0A5CB5FF937BBC9AA7843F79FD87A99A1C06B5B7A7E32646156416AE36`.
All9whole-recording files are verified; ZIP198258bytes, SHA256
`BAA66D7D1EE19B6DE11E940BA0DFB1741E6C49E3EFFFCFDCE746C2E2F2520608`.
Reviewed plots: `archive/output/master-association.png` and
`coverage-archive/fixed-coverage/whole-recording.png`. The latter SHA256 is
`5637500E81C9C15618F1136E04662CD9EAF753F0D4F8C0079B29570FF8718432`.
Only compact JSON/logs/plots are downloaded, not the raw PCM archive.

Final remote audit re-verifies626original and8fixed-audit inputs, no remaining
analysis/native owners, unchanged protected OBS/QQMusic creation identities,
and unchanged running SYS hash. No native audio, UI, device or driver operation
occurs. Original27dirty/cache files are preserved; local commit only, no push
or memory update.

The original7delivery-gap bound violations and1render wait timeout remain;
waveform agreement does not erase them. Physical calibration, VAD waveform
and longer matched qualification remain open. This recording's unexplained
waveform no longer justifies another unchanged soak or speculative production
fix. First-packet work stays deferred; independent development remains unblocked.
