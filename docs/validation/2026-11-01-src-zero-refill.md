# Playback SRC zero-refill segments must not rearm prime

## Reproduced defect and narrow repair

Base `72d7afe` rearms the playback prime gate whenever a carrier read transfers
zero frames. A short upsampled segment can instead require zero new input:
the production FIR already retains enough lookahead to produce it. The empty
read succeeds, but the old condition unprimes playback. The following segment
then waits for the target despite having sufficient valid carrier/FIR content.
Actual Engine parameter events can split a normal block into these short calls.

The frozen hypothesis, source pins, command logs and failed controls are in
`D:\coding\moiren-evidence\20261101-src-zero-refill`. The first behavioral RED
runs one actual boundary regression and fails at Engine frame2: zero transferred
instead of one. A separate mutation restores only the old condition and fails
the compiled unity-Bus regression at `block=1 sample=0 value=0`, instead of the
expected left-channel0.25. The fixed production file is restored byte-for-byte.
This is a reproduced software audio defect, not just a telemetry discrepancy.

The repair requires `required_input > 0` as well as zero transfer before
rearming prime. It does not bypass reset/epoch fencing or change FIR history,
servo math, genuine starvation/reprime, initial32-frame mute, operating budgets,
defaults or quality reporting. The sole other non-test-module edit enables the
existing library-test allocation counter under `cfg(test)` on every platform;
production builds install no allocator through that module.

## Five bounded regressions

- A96-kHz, max32/target384 source receives one prefill, then64 one-frame calls.
  Zero-demand calls must not make the next call wait for another target fill.
- The rate/target/segment/precision matrix covers44100/48000/88200/96000 Hz,
  Compatible and explicit384 targets,1/2/3/7/16/32-frame segments and f32/f64:
  96 configurations,128 calls each. It replenishes exactly consumed PCM and
  checks constant stereo content after initial mute, without recovery/XRUNs
  or heap allocation/deallocation during actual source reads.
- An empty carrier with sufficient FIR history still produces the next sample.
  A following positive-demand starvation remains an XRUN and rearms prime;
  refill383 waits and384 resumes with counted recovery mute.
- A reset at a zero-refill point still requires producer/consumer quiescence,
  discards old data only after acknowledgement and requires fresh target data.
- A real compiled Source -> unity Bus -> Sink graph uses31 accepted, applied
  send-gain controls per32-frame block to force32 one-frame segments. Across16
  blocks in both precisions, it checks exact applied frames,512 source
  checkpoints, expected PCM and allocation/deallocation-free Engine render.
  The sink is a bounded Engine-rate test carrier, not a native MIC or recorder.

The final regression isolates an existing production segmentation path. It is
not the previously proposed broad PCM-worker/Bus/MIC fixture, which remains
unapproved and unimplemented, and creates no additional S1-S3 acceptance gate.
Constant synthetic PCM here does not establish changing-waveform continuity
or source/native/recorder sample correspondence.

## Fresh verification and retained failures

- Final-source workspace all-target/all-feature tests pass3044 tests in82
  summaries, with10 ignored, including the five regressions and headless gpui
  coverage. A preceding3044/10 run began before the test-only allocator cfg
  repair; it is retained separately, not relabeled as final-source verification.
- Final focused `zero_refill` suite passes5 tests. Standalone RT shadow passes229;
  the shadow harness does not itself include these five boundary regressions.
  Their source-read/Engine-render allocation checks run in the actual audio
  library test binary.
- Final-source workspace all-target/all-feature strict Clippy and rustfmt pass.
  Linux audio all-target/all-feature check passes **compile-only**; no Linux
  test execution, remote CI or Miri is claimed.
- The first Linux check fails with two E0433 errors because the pre-existing
  library-test allocator module was Windows-only. The portable test module now
  reuses that same counter under `cfg(test)`; the failure and successful rerun
  both remain. Behavioral RED and compiled mutation RED also remain failures.

Local review is single-agent, not independent review. Exact-path staging
excludes the original221-byte `gate.log`; its SHA-256 remains unchanged.
No push, subagent, native acquisition/replay, remote audio, speaker rendering,
driver/system-control write, reboot or memory update occurs.

This closes only the reproduced zero-demand false-starvation defect. S1-S3
remain incomplete: clean acquired changing waveform, independent common
reference, valid sample-origin correspondence, matched source-path conditions
and calibrated sustained physical quality/latency are still missing. This does
not establish sub-10-ms output, impossibility, a theoretical floor, a lower
default promotion or a VAD/Process path winner. Historical refusals stay intact.
