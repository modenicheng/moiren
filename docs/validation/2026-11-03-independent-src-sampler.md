# Independent SRC History Sampler

Date: 2026-11-03, Asia/Shanghai. Baseline: `afce0a2` plus four explicit
example-only source overlays retained with the binary.

## Problem And Change

The previous target1536 capture retained callback1 and444, then489 through6067.
Exactly44 interior scalar checkpoints were missing. With a256-entry history,
the next successful snapshot after444 had advanced to744:300callbacks of logical
progress, or400ms at Engine64/48k. This is not a measured400ms control stall or
evidence of missing audio. The writer stream was complete and unretained0.
The exact blocking control operation remains unidentified.

Source inspection establishes a separate, reproducible acquisition defect:
history polling depended on the main control loop, including synchronous native
queries and source-read confirmation. A regression advancing600callbacks while
that caller never polls fails on the old implementation. The example now owns
a separate control sampler, immediately polling then waiting up to10ms between
attempts. The existing bounded archive/writer remains its sole mutable owner.
Normal cleanup joins the actual audio host before stopping/joining the sampler
and finalizing the archive. Early Drop joins without inventing a terminal receipt;
panic remains sticky and reports incomplete/unqualified evidence.

This changes the probe only. No production RT path, history capacity, queue cap,
buffer/default, admission, quality threshold or driver changes.10ms is a polling
target, not a scheduling guarantee. Late scheduling or contention can still
produce a gap and must refuse completeness. The original444->489 fixture still
reports44missing; sampler join cannot replace boundary writer closure.

## Software Verification

An exact `git archive afce0a2` with only the four intended overlays excludes all
27 preserved dirty/cache files. The regression first fails at
`history_complete=false`, then passes using a bounded acknowledgement of actual
background collection, without the caller driving poll. Additional cases cover
early Drop, open writers, repeated finish, sticky sampler panic and unchanged
interior-overwrite refusal. Focused tests pass18/1ignored; the full probe suite
passes738/1ignored. Strict probe Clippy and release build exit0.

The requested zero-refill/analyzer/headless measured-UI checks pass5/51/20.
Host execution is compilation and pure/headless tests only, with no host-native
audio/UI or existing-process manipulation. Windows feature-workspace tests pass
2360/9ignored/0failed across71harnesses. Feature-on all-target workspace check,
strict Clippy and rustfmt check exit0. The separate example suite is not part of
that2360count. All1400archive entries are accounted for, non-overlaid bytes match
the baseline, the four overlays match the binary's source pins and all27original
dirty/cache files remain hash-identical. No Linux, Miri or remote CI is implied.

## One Remote Native Acquisition

Remote root: `D:\MoirenTest\20261103-src-sampler-1000` on
`LAPTOP-NP49RG6H`. Lifecycle is task: cooperative125s case checks, finite held
process joins and scheduled outer420s limit, not a hard125s watchdog.
The new probe is7629312bytes, SHA256
`853EF19F21D9184D0C97BEDC4E6505549E272F6E2B3650C2AD0555152450EA5D`.
`source-overlays.json` pins the four exact source files; the isolated source tree
is retained locally. Independent source and runner reviews find no blocker.

The same admitted graph/settings are used once: VAD Playback -> unity Bus ->
DAWN physical Shared336 output; F32/48k/Engine64; source Shared480/capacity1056;
render target512/wake64/callback64/capacity738; virtual target1536/capacity4800,
jitter96, declared packet480/gap512, recovery cap8. Actual whole-wake704,
transfer706 and modeled floor1346 remain published. The source submits288000
contiguous indexed-v2 changing frames at amplitude0.005, without scheduled gaps.
Submission completion is not playback or capture proof.

The case runs10:12:09.935-10:12:50.126. Task Ready0 and acquisition complete;
graph exits normally1 with an unqualified diagnostic report, isolated OBS exits0,
neither requires forced cleanup. No native retry follows.

The finalized trace retains all6041callbacks, missing0/unretained0, closed writer,
no refusal, stream/history complete.26unavailable polls remain explicit and are
covered by later successful snapshots. This demonstrates the new collector on
this acquisition; it neither identifies the old gap's control phase nor proves
waveform quality. Raw Playback witness384000frames/1536000bytes is joined and
completed, sealed `identity_changed`.

## Closed Recording Analysis

Offline analysis runs remotely against closed, pinned inputs in the existing
isolated Python environment. Raw PCM and scalar chunks remain remote. All19
input/tool hashes remain unchanged. The independent OBS recording decodes to
903168stereo frames at48kHz,18.816s, without cropping. The actual PNG is visually
reviewed: an earlier unequal-channel transient and a later roughly six-second
changing interval remain visible. The transient's origin is not established.

Complete scalar history permits exact archived Playback FIR replay through
6036callbacks. Callback6037/timeline386304 is the first reset/epoch barrier;
the suffix through6041 is still validated. The barrier's phase is not established
merely by its position. Playback projection remains incomplete, and master
projection uses the nominal unchanged same-rate FIR without a native submission
witness or prepared-route authority.

One unique frozen recorder/replay association is found. Its -379881frame shift
aligns independent file origins and is not audio latency. Every903168recorder
frames is categorized, with unassessed0:

| Category | Frames |
| --- | ---: |
| Fixed-map compatible | 281152 |
| Outside replay | 516880 |
| Unknown replay | 3951 |
| Uninformative signal | 93760 |
| Anchor training | 7049 |
| Waveform residual | 64 |
| Unsupported block context | 312 |

The64frame residual occupies recorder[673152,673216). No nonfinite or clipped
frames are counted. Compatibility is not bit equality or complete delivery
proof. Training, silent/unsupported/outside intervals and the residual remain
visible; thresholds are unchanged and no residual is attributed to audio damage
without independent evidence. Waveform/continuity/physical qualification flags
remain false and physical latency remains null. No lower default, matched-path
winner, theoretical minimum or sub10ms result follows.

## Closure And Evidence

The final10:14:35 remote audit finds no owned process or running test task.
Protected original OBS/QQMusic identities, loaded driver/boot, bare healthy bus,
defaults and render endpoint controls remain unchanged. First-packet work stays
deferred. No background soak remains.

Compact ZIP152668bytes, SHA256
`EA354380B7FC09E5F1BCAC0D0D847454033D12F6409E19E9869D4082EF70B6D9`;
all49payload lengths/hashes and exact manifest membership are verified locally.
Raw PCM and scalar chunks are not transferred. Local evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-src-sampler-20261103-1000`.
Reviewed visualization: `archive\analysis-output\rms-coverage.png`.

An additional pure lifecycle fixture initially retains a stale pre-existing
window-before-socket expectation. Its failure is preserved, then corrected to
inactive -> socket -> window -> held-cleanup; unresolved recording still requires
held-cleanup before socket closure. All four pure runner suites pass, including
13lifecycle cases. Deployment uses independent ProcessStartInfo stdout/stderr
capture and actual exit codes to avoid the prior PS5 successful-unittest stderr
rejection. These are evidence-tool fixes, not audio behavior changes.
