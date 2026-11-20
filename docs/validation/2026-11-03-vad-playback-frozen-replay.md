# Archived VAD Playback replay and frozen recorder association

Date: 2026-11-03, Asia/Shanghai. Source baseline: `eec7e51` on
`feat/minimal-io-latency`. This is an offline diagnostic of the already closed
[shared-source waveform recording](2026-11-03-shared-waveform-recordings.md),
not a new native acquisition, stability soak or S1-S3 acceptance PASS.

Local evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-vad-playback-replay-20261103-0545`.
Remote computation root: `D:\MoirenTest\20261103-vad-playback-replay-0545`.
Original inputs stay under `D:\MoirenTest\20261103-vad-waveform-joined-0515`.

## Question and immutable evidence

Can the actual published VAD input FIFO and archived Playback cursor/step
history reconstruct that boundary, and does its nominal master-render
projection explain the OBS nominal marker refusals? A marker refusal is not
a lost-frame count. Neither an unexplained refusal nor a successful local
association establishes a production defect, continuous clean output or
physical latency.

The input is the actual stereo Pcm16 publication prefix, decoded with the
production VAD codec, not the source MMDevice's negotiated F32 format. Replay
fixes the published FIFO origin at zero, pushes exactly each archived carrier
frontier, applies the archived `processing_step` bits directly and checks
every before/after cursor including floating-point bits. There is no ppm
inversion, rate/phase fit, packet/QPC origin search or fabricated input silence.
Initial unprocessed callbacks and the 32-frame priming mute remain invalid
support. Projection ends at the first epoch/FIR retirement barrier while the
entire closed archive, including its suffix, is structurally validated.

The source archive has `route={prepared:null,invalidated:false}`. This is
unknown route evidence, not unity/graph authority. The same-rate master FIR
projection is consequently **nominal**, not a witnessed native master replay.
Validity expands to its FIR dependencies; unknown support stays unknown.

Recorder analysis uses the unchanged committed `process-replay-frozen.py`:
coarse peak ratio 1.8, training/held-out NCC 0.99, training/held-out NRMSE 0.05
and local NRMSE 0.1. The first training-compatible origin/gain/DC is frozen;
held-out failure cannot select a replacement. Both channels' original support
components are retained, including the earlier unknown transient. Long
components are partitioned into contiguous 256-8192-frame windows without
dropping or padding their tail. These windows are not marker identities.

## Independent callback oracle

The helper is evidence-only, outside the repository. Its FIR before the
exact-step adapter and its VAD codec match the committed production snapshots.
An independent clean `git archive eec7e51` export runs the actual
`VirtualPlaybackSource<f32>` with a real audio bridge and synthetic Pcm16
packets, variable 32/64-frame requests, startup waits and a retirement.
It does not use the helper's replay loop as the oracle and opens no device.

The final release helper reproduces all 8416 stereo F32 callback frames,
67328 bytes, byte-for-byte. Both output SHA256 values are
`BBFCECC55BF8AEFA69106FB463D1AD08BFC5093E1B434AE70BC01A92A7388637`.
It checks 158 cursor pairs and stops before sequence159, while validating
all160 archived callbacks. The fixture retains160 startup-zero frames,
32 muted frames and3246 unconsumed input frames.

The first oracle attempt correctly records a diagnostic-parser refusal:
the helper treated a non-null route wrapper as a non-null prepared route.
A failing regression fixes that distinction, retaining false invalidation
as a requirement and `route_claim:null`. Independent review also reproduces
an impossible same-reset cursor advance in a retired suffix. New checks bound
input advance by carrier frames and output advance by engine frames without
projecting that suffix. Final helper debug/release tests:46 passed; formatting
and Clippy pass with the documented inherited snapshot exemptions.

The unchanged nominal master projection example passes14 tests and builds
in the independent clean export. Wrapper tests cover finite held-child
deadlines, cleanup after non-timeout exceptions and unconditional post-run
original-input hash audits. Association is a held child under the aggregate
210-second compute budget; final audits have reserved time within the
300-second scheduled task. All processes have task lifecycle. The pinned
Python3.10 runtime uses a tested local `enterContext` compatibility shim.
Fresh host wrapper/window checks pass17/4 tests; no host native audio/UI is
launched or manipulated.

## Deployment boundary

The single deployment archive contains15 entries,380936 bytes, SHA256
`2C6461D39E75F50B60C9BC665AB5EAF626C64883710D62ADB367AF015F548252`.
The final Playback helper SHA256 is
`E79062F1D89A52CFAA1C862C3D1AF2F1F3A0990CED654D4434BDA0C0AB6685BE`;
the nominal master projection executable SHA256 is
`E7D47F505228929F81139CDC4F0AC4E2A6A6088D39D17C97CB2AC814EBCFC943`.
The remote Python executable and numpy1.25.2/scipy1.9.3/matplotlib3.7.5
module ancestry are rechecked in their existing isolated environment.
No audio device, driver configuration, UI, OBS or player is opened or changed.

## Actual remote result

The single task finishes Ready/result0 in35.964 seconds. All12 held children
exit0 without forced termination. The remote pinned runtime passes17 wrapper,
4 window-partition,12 frozen-association and25 scalar-archive tests. All six
original input byte lengths/SHA256 hashes remain unchanged after computation.
Protected OBS12648 and QQMusic15096 retain their creation identities, no test
audio owner remains and the loaded driver hash/boot remain unchanged.

Actual Playback replay validates all6062 callbacks, checks6051 exact cursor
pairs and ends before the sequence6052 retirement at timeline frame387264.
It consumes381798 of384480 published input frames, retaining2682 unconsumed
frames; output contains5568 startup-zero frames,32 priming-muted frames and
381664 valid frames. The nominal master projection has387248 frames,381633
with known FIR support. Ending at the retirement is an intentional prefix,
not full-recording or full-stream qualification.

The earlier unknown stereo transient remains four refused windows, with
no unique training anchor. The following six pulse-support components all
pass the unchanged frozen held-out gates in both channels. Anchor window4
fixes the integer recorder-to-replay shift at -349521 frames, with gains
1.0000000254/0.9999998716 and DC approximately -5e-8. It is an independent
stored-origin association, **not a delay to divide by sample rate**.
Later windows neither reanchor nor refit. Across these twelve channel/window
evaluations, held-out NCC is at least0.9999999999988, NRMSE is at most1.57e-6
and worst local NRMSE is at most4.38e-6.

This supports a concrete diagnostic conclusion: all six OBS pulse regions,
including those with unresolved nominal code identity, are consistent with
the actual Playback SRC history followed by the nominal master FIR. The
earlier nominal refusals therefore do not justify declaring missing frames
or making a speculative production fix. This does not assign marker IDs by
order, explain the unknown transient, witness the master native submissions,
authenticate an absent prepared route, certify continuous waveform quality
or establish physical latency. Original nominal code refusals are unchanged.

The remotely generated figure is length/SHA256 verified after transfer and
visually inspected:
`archive/association-output/frozen-replay-association.png` under the local
evidence root. Its top panel retains the unknown transient and all six pulse
regions; held-out panels show the six compatible windows only, not a PASS
for the earlier four refused windows. Raw PCM/scalar archives stay remote.
The compact archive has33 payload files, excluding its manifest,104085 bytes,
SHA256
`EB3422E884D9EA1CC5516CA260C9DD97AF5C86C3FDE28311AF8646089E60718D`;
all included entries have exact manifest lengths/hashes and no extra entries.
The fresh post-transfer audit independently repeats terminal-task, input-hash,
protected-process and driver checks, with no running Moiren task or test
audio/replay owner. All27 original dirty/cache file hashes remain unchanged;
they are excluded from clean source exports, helper builds and this commit.

## Development decision

Missing physical multichannel equipment does not block independent development.
First-packet work stays deferred; Compatible defaults, quality thresholds and
buffer policies stay unchanged. The original recordings' nominal refusals
remain preserved. Do not repeat an unchanged soak or another origin fit for
this question. S2/S3 software integration is not reopened by nominal marker
refusals; the remaining retained desktop acceptance requires an unobscured
interactive session, and physical qualification requires suitable hardware.
Independent application/lifecycle work can proceed while newly justified,
finite remote tests run in parallel with isolated owners and explicit cleanup.
