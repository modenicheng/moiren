# Closed VAD cascade waveform replay

This S1 increment separates a deterministic software amplitude control from
reconstruction of the already-closed native capture. It does not change audio
processing, transport policy, defaults, collection gates or the driver. The
first-packet investigation remains deferred. No native audio starts on the
development host, and no new recording is required for this increment.

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-vad-cascade-replay-20261102-2145`.

## Coupled software control

The existing coupled compiled `Source -> Bus -> Sink` fixture gains a separate
amplitude module, rather than another native test framework. At 48 kHz and
Engine block64 it exercises sparse changing markers at amplitude0.025 and a
DC control, for both f32 and f64 processing. The markers use `MarkerGenerator`,
not the indexed identity codebook used in the retained native acquisition.

Two separately fed production FIR instances receive the actual applied servo
corrections. Every compiled unity-Bus sample, microphone carrier sample and
final PCM16 encoding is checked. The codec expectation is an independent
scalar quantizer, not another call to the encoder under test. Steady DC stays
at amplitude0.025 and encodes to819; both-polarity changing-signal overshoot is
allowed only when it equals the FIR control. Normalized coefficient sum is a
DC property, not a promise that every changing-signal peak stays below its
input peak.

Mutation controls change unity Send gain and PCM16 scale separately in the
clean exported test tree. Both fail the new assertions; both mutations are
restored. They are not production fixes. This establishes test sensitivity,
not the cause of any native observation or independent validation of the FIR
kernel design.

## Closed-data question

The earlier [output witness](2026-11-02-vad-output-reference.md) retains actual
Playback Float32 input, actual post-encode MIC PCM16 output and complete scalar
SRC callbacks. All651840 decoded output/recorder scalar values are equal, but
the observed input/output peak difference was not yet explained.

The next bounded offline operation uses the original input bytes and actual
per-callback steps, not a nominal marker template or fitted rate, phase, gain
or shift. Startup clear callbacks still feed MIC zero blocks without advancing
Playback's FIR; carrier consumption and FIR pushes remain separate. Explicit
32-frame mute advances the FIR while replacing only the emitted values.
Both before/after cursor states must agree exactly, including floating-point
bits. The first reset/epoch barrier terminates the projected prefix; retirement
is not joined onto the earlier audio or silently erased. The evidence-only
Rust helper snapshots the unchanged production FIR and VAD codec, with only
an explicit exact-step adapter. It does not invert the archived step to ppm.
JSON float roundtrip is enabled rather than accepting phase tolerances.

Raw PCM and the full archive stay on `moiren-lan`. Only compact receipts and
plots return to the host. A fixed stored-FIFO-prefix comparison is not a native
clock association or physical end-to-end latency. The original first retained
packet discontinuity and collection refusal remain intact regardless of a
waveform comparison result.

## Results

The single remote offline execution runs on2026-11-02 at21:53:21-21:53:30
Asia/Shanghai. Archive validation, Rust replay and Python plotting all exit0
normally, without timeout, forced termination or cleanup error. Their owned
child elapsed times are0.762s,0.562s and4.296s respectively. No native source,
Engine owner, UI, recorder or OBS reconfiguration starts.

The archive retains5121callbacks per boundary. Replay checks the full scalar
archive and projects5113callbacks, stopping before callback5114's first
Playback retirement/reset barrier. It consumes309444of312000published input
frames. The unconsumed2556input frames and1524replayed MIC FIFO tail frames
are reported separately, not dropped-sample claims. Startup includes17856
Engine zero frames and the explicit32-frame prime mute; those are retained
facts, not a new first-packet investigation or silence exemption.

All325920actual encoded stereo MIC frames, comprising651840scalar samples,
equal the reconstructed FIFO prefix exactly. There are zero mismatches and
zero PCM16 LSB maximum difference, with no search or gain/rate/phase fit. The
replayed and actual peaks both equal0.049835205078125, and per-channel RMS both
equal0.0032092010284031154. Thus this capture's larger changing-signal peak is
fully reproduced by the existing dual-FIR/unity-route/codec transformation;
it does not require an extra Bus gain or unexplained byte corruption. This
does not independently certify the filter's perceptual quality, nor prove a
general absence of audio faults outside this retained prefix.

The previously established output/recorder equality remains separate. Together
these observations reconcile the retained input-to-recorded content without
converting stored FIFO coordinates into clocks or physical latency. The
unchanged collection still refuses its first retained discontinuity flag.

`remote/plots/fixed-fifo-replay.png` shows RMS overlays, both channels' maximum
absolute residual across every480-frame bin, and the fixed-index peak window.
The plot is visually reviewed. All14returned compact artifacts pass individual
length/SHA-256 verification. The13original native artifact hashes agree before
and after; raw PCM and the full archive never leave the target. Final audit
finds no test owner, the bare healthy control bus, and unchanged protected
OBS/QQMusic identities, loaded SYS hash and boot identity.

Review finds an evidence-wrapper failure path: a runner metadata exception
could be caught while its child receipt still says exit0. The original used
wrapper and run are preserved. A separate hardened wrapper propagates that
exception after retaining the receipt and checks runner status/error/manifest.
Three fake-runner scenarios pass after a genuine original-wrapper assertion
failure; no child process starts in those tests. For this completed execution,
all three actual runner receipts have `child_exited` status and no error, and
their three manifests plus all nine referenced receipt/log hashes are checked
independently. This verifies the existing evidence without rerunning replay or
native audio. Seventeen compact artifacts are retained in total.

## Verification

- The coupled amplitude test covers four scenarios and has two independent
  mutation assertion failures, followed by a restored passing run.
- Clean-export library tests pass791/4ignored with default features and again
  with `vad-probe`; scoped strict all-target Clippy and owned-file fmt pass.
  Only the new amplitude module and its registration differ from the export.
- The evidence-only replay has20passing tests, a retained assertion RED, exact
  source snapshot checks, and passing release/all-target/fmt checks. These are
  bounded tooling tests, not independent validation of the production kernel.
- The fixed-prefix plot helper has five passing tests, including changed-byte,
  shift/gain disagreement, short-prefix and bounded/aligned-input refusal.
- Read-only review finds no concrete replay arithmetic/frontier blocker. All27
  original dirty/cache file hashes remain unchanged. No production source,
  default or driver changes, push, Linux/Miri/CI or new full-workspace run are
  claimed by this test-only increment.

## Acceptance boundary

The original S1-S3 goal remains active. No smaller default, source-path winner,
physical/sub-10-ms result, steady-state qualification or first-packet exemption
follows from these software controls. Unavailable physical gear is not a
blocker for independent software development. Remote work uses bounded owned
processes with isolated environments and leaves external OBS/QQMusic untouched.
