# Current-code regressions and one native VAD IO/Bus/IO collection

Date: 2026-11-01. Tested source: `497957e8bb3a0a6389e39de05c1a7fa5a895ae42`,
including playback zero-refill repair `353f0db`.
The user explicitly requested the three software regression commands and one
short native VAD playback -> Bus -> microphone test. This authorizes this
bounded software-route acquisition, not driver replacement, physical output,
VM timing qualification, a buffer scan or a physical-latency claim.

**Software regressions pass71 tests. The native test fails retirement with
`slot=0 HRESULT=0x8007051A`; collection/cleanup and quality remain incomplete.**
The acquired waveform does not establish whole-route continuity or a latency
distribution. S1-S3 remain incomplete.

Evidence root:
`D:\coding\moiren-evidence\20261101-user-regression-native-102415`.
Target root:
`D:\MoirenTest\20261101-user-regression-native-102415` on the existing separate
physical Windows test machine. No test driver is installed on the development
host or replaced on the target.

## Fresh software checks and build

All three requested commands execute against the current worktree and exit0:

```text
cargo test --locked -p moiren-windows-audio --lib zero_refill
cargo test --locked -p moiren-windows-audio --test latency_marker --test latency_marker_rate --test latency_qualification
cargo test --locked -p moiren-app --features gpui-test measured_ui_tests
```

- Zero-refill library regressions:5 passed,0 failed/ignored.
- Marker, rate and qualification integration suites:19+8+24 passed,0 failed/ignored.
- Actual headless desktop `measured_ui_tests`:15 passed,0 failed/ignored.
  Filtered zero-test binaries are not additional coverage.
- The optimized probe build uses
  `cargo build --locked --release -p moiren-windows-audio --example e2e_latency_probe --features vad-probe`.
  It exits0 and produces7446528 bytes, SHA256
  `9462CF10B053B89B11160C7E7EFB403AF8AA9F9547D3433242D00282BD9EADDE`.
  Transfer hashes and target `--help` smoke pass before native activation.

These are fresh focused checks, not another workspace-wide test, Clippy, Miri
or remote-CI run. The prior full-suite receipt remains tied to its own source.

## Fixed configuration and single request

The prior both-minimum control's input JSON bytes are retained exactly:
30 seconds/1440000 source frames,29 indexed-v2 markers of1024 frames at
amplitude0.025,48kHz stereo Float32. Both source and recorder request their
Exclusive minimum; actual native buffers are144/144 with successful negotiation
and no recorder alignment retry. The graph is the production exact
Playback Input -> unity Bus -> Microphone Output, Engine32,
target1024/capacity4800/jitter96, maximum packet/gap480 and recovery cap8.
The request ceiling is60 seconds; correctness grace remains3000ms.
No physical renderer is selected.

The copied Interactive/Limited launcher and runner retain exact-host, console
SID/session1, no-existing-owner, healthy-root/no-child, file-hash, disk,
requested-period and no-relaunch guards. Additional checks pin `oem205.inf`
before native work and recheck the driver hash/binding afterward. Adapted
Windows PowerShell5.1 inert guard tests pass55/55. The launcher requests only
`Moiren-ZeroRefill-20261101-102415`; scheduling delay does not trigger a second
request. The probe uses a bounded120-second exit wait and exits without forced
termination. WPP is not started.

This is a current-code reproduction, not an isolated old/new SRC-fix A/B:
the prior frozen9968505 source lacks several later fixes and scalar capture
changes, and its WPP policy differs. The current scalar instrumentation and
streaming writer can perturb servicing. The graph uses32-frame segments;
the specific short-segment zero-refill defect is covered by the fresh library
regressions, not independently reproduced by this native run.

## Actual native result

- Source acknowledges1440000/1440000 scheduled frames and29696 nonzero frames.
  Native submission totals1450944 frames including144 preroll and tail frames;
  successful ReleaseBuffer is not evidence of delivery of every source sample.
  Source native Stop and join succeed.
- Recorder/sink retain10114 complete144-frame packets,1456416 frames and
  11651328 PCM bytes:30.342 seconds at nominal48kHz. Stored packet sequences
  and positions are contiguous; all flags0, with zero reported discontinuity,
  timestamp-error, overflow, recorder loss or incomplete packet. Direct PCM
  readback finds no nonfinite samples,31483 nonzero channel0 frames and identical
  stereo channels. These are stored-data facts, not independent-reference or
  whole-waveform acceptance.
- Source submission and postroll complete, but probe/task exit1. The retained
  runtime failure is `ShutdownHost: VAD EngineHost cleanup: resource unavailable:
  VAD PCM shutdown terminal failures: slot=0 HRESULT=0x8007051A`, followed by
  failed/unknown final graph-native summary. `collection_complete=false`,
  `cleanup_complete=false`, `quality_passed=false` and `quality_complete=false`
  remain unchanged. No environmental cause or new root cause is established.
- Last Running provider snapshots report zero drops, silence, resets and
  recovery in both directions. Final Removed snapshots retain reset/recovery1/1;
  Playback submitted/completed10077/10073 and MIC10114/10114 remain visible.
  Endpoint removal and absent processes do not turn failed runtime cleanup
  into successful cleanup.
- Coarse boundary reporting retains initial prime32, zero sampled steady
  deltas and15 unknown observations, with `final_after_join=false`. These
  observations do not erase the more complete callback history below.

## Waveform and SRC-history diagnostics

The nominal indexed diagnostic retains the original support-window rule:
channel0 nonzero clusters separated by more than32 zero frames, padded64 on
each side. All29 windows compete all29 codes; no identity comes from window
order. Thresholds remain correlation0.7, margin0.05 and RMS0.0001, with
164159488 charged samples below the256000000 cap.

The corrected valid invocation exits1 with21 unique identities and8 missing
classifications (indices2,4,5,6,7,10,20,24), no duplicates and9785 native-QPC
interval-policy violations. Source/recorder origin remains unknown. These
are not proved missing markers, lost frames or a physical-delay result; the
21/29 versus historical14/29 counts do not establish an improvement under
unmatched code/instrumentation conditions.

The new native scalar stream retains91130 checkpoints in45 chunks,
67712745 JSONL bytes. Independent `scripts/vad-src-archive.py` readback exits0,
verifies both boundaries'45565 callbacks without missing/unretained callbacks,
and confirms stream SHA256
`95b1331286a3daa2f1a909166f9572413470851786e7087a338f6905a047f875`.
Scalar completeness does not mean clean audio:

- Playback has272 callbacks with XRUN/discontinuity fields:263 before its
  first successful FIR output at sequence264, then9 at sequences45557..45565.
  Sequence264 explicitly mutes32 initial-prime frames.
- Microphone has4 callbacks with XRUN/discontinuity fields at45562..45565.
  No such fields occur in the middle callback interval264..45556 for either
  boundary. These are Engine callback coordinates, not calibrated source or
  recorder time. Tail faults and boundary epoch changes remain failures/barriers.
- All callbacks retain initial plan1/control-generation0. Five bounded
  first/early/middle/late/final microphone queries use the existing cascade
  mapper. Three middle queries map dependency support; startup and final
  queries refuse tainted/startup support and a boundary-epoch barrier. Overall
  exit1 is retained. This maps FIR dependencies, not actual source-to-recorder
  correspondence, complete sample values, marker identity or physical delay.

## Archive, errors retained and target cleanup

The closed archive contains33 payloads plus its manifest,12443891 bytes,
SHA256 `0CF8640E9BAF44B8704DA4A6A34D628E785E83AFD510385510C24D112A2EC1C7`.
Manifest SHA256:
`EDCDBCC14307B0585528869B03E173A01A4991D8F192D2DD202412B2C7374ECB`.
Host extraction independently checks every ZIP member's length/hash and
rechecks extracted bytes. Original PCM SHA256:
`8F2D3AAEDDEAD06A9458C3C37527F0A29627221FD67C94F44BA7632ED4334CF0`.

Initial remote JSON-summary/poll commands time out while parsing the large
report; they do not imply native timeout or justify another acquisition.
Small terminal inspection confirms the owner already exited. Initial archive
helper invocations hit the SSH command-line limit and a missing PS5.1 assembly;
the corrected uploaded helper explicitly loads both compression assemblies and
verifies the already-created manifest without replacing it. The first nominal
analyzer invocation has an unsupported32-MiB metadata cap and exits2; the
corrected16-MiB cap leaves PCM, windows and thresholds unchanged. Initial errors,
command receipts, stderr and corrected outputs are all retained.

After archive verification, guarded cleanup unregisters only the exact unchanged
terminal task. Fresh target checks confirm no audio owners or raw children,
healthy Running611 driver and unchanged `oem205.inf`/SYS hash, boot, six default
roles and12 render mute/volume states. The pre-existing unrelated task remains.
No evidence, driver rollback or system file is deleted. No reboot, physical
rendering, volume/default/privacy write or push occurs.

The immediate follow-up is source inspection of the retained shutdown epoch/
pending-completion failure and source/native/recorder correspondence, using
these artifacts before proposing another native run. The original waveforms,
quality refusals and physical/common-reference requirements remain intact.
