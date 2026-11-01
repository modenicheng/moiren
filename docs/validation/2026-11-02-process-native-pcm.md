# Bounded native Process PCM and actual SRC replay

## Scope

This S1 increment closes the native offered-PCM/accepted-ring correspondence
gap in the professional application monitor. It does not change queue targets,
servo, bounded recovery, admission gates, audio conversion or defaults. It does
not close S1-S3 or establish OBS/render correspondence or physical latency.

`monitor --cursor-trace-directory NEW --cursor-trace-pcm-frames 65536` explicitly
enables the tap. Ordinary desktop and monitor owners allocate no PCM tap. The
configuration survives the measured baseline/replacement factories for both
processing precisions, without becoming a persisted desktop latency preset.

The capture ingress fixes the actual accepted prefix and buffer generation
before copying the offered native packet into a preallocated SPSC diagnostic
ring. Original f32 bits, including nonfinite payloads and signed zero, remain
unchanged. SILENT packets have no readable payload and retain zero samples;
mono leaves slot two zero. Packet sequence, offered/accepted frontiers, native
offsets, flags, generation, device position and raw packet QPC remain distinct.
Unpublished tails are retained but do not advance accepted-ring SRC indices.
Process positions/QPC do not establish physical sample age.

Tap capacity must fit a nonzero 8 MiB ceiling and is additionally charged to
the bridge's combined storage and the project's audio preparation budget.
Insufficient diagnostic capacity loses the whole evidence packet, not audio;
loss, reader abandonment and saturation are explicit sticky refusals. There
are no per-sample clocks, locks, disk IO, allocations or deallocations on the
streaming recording path. Final scalars are coherent only after writer closure.

The native stream retirement guard closes the writer on normal return, gate /
Start / packet failure and unwind, moving its storage into the capture report.
Even an abandoned diagnostic reader cannot free that storage on native
teardown. The normal control join or failed-report panic payload owns its final
reclamation. This is a streaming teardown guarantee; preparation failure before
the guard is installed remains outside that guarantee. Direct non-native
`capture_bridge` callers retain their existing off-RT destruction contract.

## Persistence and independent analysis

Per immutable owner, `native-pcm/` contains `samples.f32le` (eight original-bit
bytes per offered frame), `packets.jsonl`, `stream.json` and `final.json`.
The control diagnostic drains at most4096frames per batch, holds bounded pending
storage and caps sample plus packet metadata writes at512MiB per owner. Existing
eight-owner limits and expected baseline/replacement coverage remain enforced.
Files are create-new, flushed and synced; successful counters/mapping advance
only after both files succeed. A partial write is retained as failed evidence,
never retried as a successful prefix. A PCM final-receipt error does not suppress
the independent cursor receipt or normal monitor post-join JSON.

`scripts/process-native-pcm.py` independently audits the closed packet and
cursor archives, excluding native dropped tails from accepted input. It replays
the actual recorded f64 bit patterns by repeated phase addition/subtraction,
not `phase + k * step`; weighted interpolation and recovery fade precede the
f32 cast. The fade gain is `(33 - fade_remaining) / 32`. Each sample's two
accepted neighbors must match the span's generation. Nonfinite input becomes
zero and mono is duplicated exactly as in native ingress.

Requested output holes have zero-filled storage and an explicit validity mask:
they remain UNKNOWN/priming, not certified silent audio. The analyzer writes
disk-backed arrays, compact JSON and PNG; source evidence is immutable. Input
and derived storage are separately bounded to512MiB. Broken sequences, lengths,
frontiers, generations, metadata, IO, cap or closure receipts cause exit2 and
retained refusal outputs. A structurally complete replay is not a waveform,
OBS, no-quality-fault or physical qualification. Every result is qualified:false.

Example, run next to the remote artifacts, not by downloading PCM:

```powershell
python -B scripts/process-native-pcm.py --archive-directory D:/run/cursor-trace --output-directory D:/run/pcm-audit
```

## Regression evidence

Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Missing tap/pin APIs, pre-native storage accounting and the final-receipt helper
have recorded RED failures before implementation. The native teardown regression
first observes three unwanted deallocations; the guard then performs zero
allocations/deallocations and the joined report reclaims all three allocations.
Unwind preserves Failed status and the retained report rather than reporting
success. Focused replay compares every sample across mono/nonfinite input,
44.1kHz SRC, dropped tails, backlog fade, silence and generation changes.

App tests cover bounded opt-in, raw-bit preservation, accepted-zero packets,
mapping corruption, open/overflow refusal, final draining, existing-file refusal,
sample/metadata write and sync faults, and a successful prefix followed by failure.
The independent Python suite passes32tests; its final root receipt is
`s1-native-pcm-python-final-1102.log`. Software regression is not native PASS.

Fresh final workspace tests pass2168/8ignored across70targets
(`s1-native-pcm-workspace-final-1102.log`). Strict workspace/all-target Clippy
and formatting checks pass (`s1-native-pcm-{clippy,fmt}-final-1102.log`). The
explicit zero-refill regression passes5tests; marker/rate/qualification suites
also pass (`s1-native-pcm-{zero-refill,analyzers}-1102.log`). Eighteen preexisting
dirty files retain their baseline hashes and remain outside the commit.
The final headless measured UI suite passes20tests
(`s1-native-pcm-ui-final-1102.log`), not a native desktop/device qualification.

The standalone RT harness initially fails to compile because its shadow modules
need explicit native-PCM/interpolation paths and the same private interpolation
history, rather than only the public reexports. Explicit shadow wiring repairs
that test surface without widening production visibility. Its final run passes
239tests (`s1-native-pcm-rt-final2-1102.log`); the original compiler failure is
retained in `s1-native-pcm-rt-final-1102.log`.

After that wiring repair, fresh workspace2168/8ignored, measured UI20,
strict workspace/all-target Clippy and fmt checks all pass again; receipts are
`s1-native-pcm-{workspace,ui,clippy,fmt}-post-wiring-1102.log`. An exported
staged tree independently passes2167/8ignored across70targets, measured UI20,
strict Clippy and Python32 (`s1-native-pcm-scoped-{workspace,ui,clippy,python}-1102.log`).
The one-test difference is the preexisting, excluded
`vad::pcm::tests::pcm_shutdown_terminal_evidence_retains_request_stamp_after_slot_retirement`;
it is not a lost test in this increment. This verifies that the intended commit
does not depend on the preserved dirty source edits.
The same export's standalone RT harness also passes239tests
(`s1-native-pcm-scoped-rt-1102.log`). All26 selected source/test files remain
byte-identical to that tested export; subsequent edits only add these receipts.

## Parallel remote test

While implementation advances, a single600.002s Process application run uses
the earlier reviewed binary `D53D9509757DC0DF79D353C8017ECA562937A0EFCACD0F90569DF615E8BA8AED`.
Its586s changing source completes28128000frames without source underflow. The
source and monitor exit0; owned portable OBS finalizes recording then exits
0xC0000005. No owned process remains. User OBS PID12648 and QQMusic PID15096
start times are unchanged. Native drop/silent/underrun/reset counters are zero,
but13delivery gaps exceed the admitted bound (maximum11.6315ms), and all four
fixed affine waveform windows are refused. There is no latency/continuity PASS.

Both independently audited cursor owners total914904events without gaps/loss.
Raw media stays in `D:\MoirenTest\20261102-054404-process-sidecar-soak\records`.
Compact verified host receipts have prefix `20261102-054404-`; the final ledger
is `sidecar-result.json`. The264132byte overview PNG is visually reviewed,
SHA256 `92A6A5DBE9970819A55583E6B8F63A609A34D598DDAC437FD94BC71680B9A778`.
Its empty offset panel correctly preserves all four refusals. This older binary
does not contain this increment's PCM tap; its results are not substituted for
the separate feature smoke.

## Native PCM feature smoke

One fresh10s baseline plus60.000s replacement runs in
`D:\MoirenTest\20261102-060101-native-pcm-smoke`. Binary SHA256 is
`62286832D9ECB359C8E6B8F7772A594259F73EA722E5D98C1AA5E2AA67C4287A`.
The source-manifest build includes the preserved dirty sources and is not a
clean-commit binary. It contains the native retirement guard; it predates only
the later diagnostic PCM-final-write-failure cursor-receipt preservation fix.
That failure-only delta is software tested, not claimed native exercised.

The exact route is Python to separate Voicemeeter In2; Process capture to unity
Bus to DAWN PRO2, with independent OBS Process reference and DAWN return.
Format48kHz, Engine64, Shared buffer336, render target512/wake64/callback64;
measured replacement target2048/capacity16384/jitter96, packet480/gap531.
Source46s/2208000frames has zero source underflows. Source, monitor and owned
OBS exit0 normally. All three native/offline tasks are Ready/result0, no owned
process or diagnostic port remains, and protected user process start times and
controls are unchanged.

Both native PCM/cursor owners close and independently audit as structurally
complete, with no evidence-loss/IO/cap/sequence refusal:

| Owner | Offered = accepted PCM frames | Cursor events | Reconstructed output | UNKNOWN/priming requested frames |
| --- | ---: | ---: | ---: | ---: |
| Baseline | 480000 | 14930 | 477760 | 3584 |
| Replacement | 2880480 | 89952 | 2878464 | 3264 |

The complete native analysis takes14.969s on the target. Actual replay holes
remain masked, not certified silence; reconstruction has not been compared
against an independently captured Engine-output stream. Native quality retains
eight gap-bound violations (maximum11.3064ms); drop/silent/nonfinite/native
discontinuity/timestamp-error/underrun/reset counters are zero. Separate OBS
affine analysis accepts two of four fixed windows and refuses two. The two
accepted diagnostic offsets are66.5639and67.0033ms, neither physical latency nor
whole-run percentiles. This is not a zero-quality-fault/continuity PASS, and the
new actual replay has not yet been associated with OBS samples.

The82.112s/73047370byte recording remains at
`records\2026-11-02 06-06-12.mkv` on the target. Raw PCM and derived arrays also
remain there. Compact evidence/summary/PNG downloads have matching remote/local
lengths and hashes, with host prefix `20261102-060101-pcm-smoke-`. The final
ledger is `result.json`, detailed receipt `evidence.json`, native root
`pcm-summary.json`. Both per-owner waveforms and OBS overview are visually
reviewed. PNG SHA256:

- Baseline136007bytes: `D438D3FE28192AC643A146B6261CA288227D411E0391C4EE359197DCB36489E8`.
- Replacement260294bytes: `0EA6296E10BC699150CEBBCADF4C4A53D4B2692FA7011EEF0C5E033518D0ACF8`.

First-packet investigation remains deferred. Physical calibration is unavailable
and does not block independent software development. No smaller default,
relaxed gate, fail-fast replacement policy, host device operation or push occurs.
