# Physical target deployment and first native receipts

Development-host date: 2026-10-29, Asia/Shanghai. This is a deployment and
diagnostic receipt, not S1-S3 completion or sub-10-ms qualification.

## Identity, source and package

- Development source `0118a54` merges `origin/main` at `497ac7c` into
  `feat/minimal-io-latency`. Workspace all-target check and the 8 negotiation /
  40 WASAPI-focused regressions passed before the merge commit.
- The target is physical `LAPTOP-NP49RG6H`, TIMI A30, Windows 11 build 22631,
  reached through the user's existing verified SSH host at `192.168.12.97`.
  Administrative access, testsigning enabled and Secure Boot disabled were
  observed; boot configuration was not changed.
- Release application and explicit probes were built on the development host,
  deployed to `D:\MoirenTest\20261029-0118a54`, and checked against host SHA-256
  and length manifests. No compiler/cache/source tree was copied to the target.
- The analyzed WDK 10.0.28000.0 driver build and INF validation succeeded. The
  target package was signed with the existing test certificate, public
  thumbprint `7F17CCDCA257D6ABC84255DD0D1854E93B9B5C1D`. Only its public
  certificate was transferred and trusted in the target Root/TrustedPublisher
  stores. SYS, catalog and catalog membership verification passed there.
- Devcon installed a new `ROOT\MEDIA\0001` control bus, published as
  `oem147.inf`, driver version `18.58.53.555`. PnP status is OK/problem 0;
  the service is Running. The service image in DriverStore and the transferred
  SYS have the same full SHA-256:
  `CCC0EC4FE4DDB51FB22B90A483E58FF3874D87FA8634D62C93BB8AA52FFFE471`.
  There was no reboot or development-host driver installation.

The first service-image comparison script selected no manifest entry because
it used the wrong slash spelling; it failed closed. Direct file comparison
then established the match above. Initial development-host Authenticode
verification failed on untrusted test-certificate root; host trust was not
modified. Target trust and subsequent verification are separate evidence.

## Measurement environment limits

The target wall clock reads 2026-10-25 while the development host reads
2026-10-29. Original timestamps are retained, not silently corrected; no
cross-machine wall-clock latency calculation is valid. Target clock settings
were not changed.

The available Realtek physical render endpoint is
`{0.0.0.00000000}.{5657d4dc-1974-4bb6-9649-f3dd08c4e7ba}`, at 48 kHz stereo.
Its volume was 0 and muted. This state and all default roles were preserved.
The built-in Intel microphone array is available, but no independent
pre-route reference, physical cable/dual-channel recorder or calibration was
established. Its diagnostic recording is not a calibrated comparison.

All initial artifacts are under worktree
`target/physical-1921681297-20261029`. The first target evidence archive is
`evidence-round1.zip`, 2,027,756 bytes, SHA-256
`F7D69F09B3EB6CB172A1DC79E044D542B1E2DAA3335DE6D42857B5637AE9959C`.
All 131 individual evidence files were verified by length and SHA-256 after
transfer. Raw recordings and large JSON remain ignored artifacts, not Git
content. At the first disk check staging used about 74 MB on D:, not C:.

## Actual native results at 0118a54

- Capability discovery: one ABIv2 control interface, PCM admission available,
  64 raw PDO limit, four request slots, 48 kHz, mono/stereo PCM16/Float32.
- Lifecycle: the first simultaneous exercise failed its audio-ready check.
  Later single and simultaneous exercises each completed four rounds with
  exit 0. The initial failure remains evidence, not a passed retry.
- Native exclusive event baseline: eight direction/channel/format cases
  completed. A second run with reliable process exit capture also returned 0.
  Actual buffer/driver packet size was 480 frames; all eight reported zero
  native discontinuities and unchanged default endpoint roles.
- Exclusive event minimum: both playback and microphone subsets returned 0,
  four cases each, with actual 144-frame buffers/packets at 48 kHz, a 30,000
  100-ns-unit request (3 ms), no alignment retry and zero reported native
  discontinuities. These short isolated clients are not Engine route stability
  or physical latency evidence.
- Exclusive timer minimum: playback completed four cases. Microphone first
  failed with `0x88890004` and later completed four cases. Actual sizes for
  mono PCM16, mono Float32, stereo PCM16, stereo Float32 were respectively
  2048, 1024, 1024, 512 frames despite the 3-ms request. Do not label those
  timer cases 3-ms operation or discard the device-invalidated failure.
- Real app-service integration passed its persisted project, Apply, two
  distinct native PCM/gain phases, Stop silence and service teardown. Each
  signal phase observed 48,000 matching frames with zero unexpected/silent
  frames or discontinuities; Stop observed 48,000 silent frames. This probe
  uses Gain(0.5), so it is not the exact minimal IO/Bus latency route.
- Exact VAD playback -> Bus -> microphone collected changing marker PCM.
  The initial 256,320-frame recording completed source submission but failed
  EngineHost cleanup with `GetOverlappedResult(VAD PCM)` / `0x800703E3`.
  A minimum-event-period run completed collection/cleanup operationally;
  quality did not pass. Reducing recovery cap to one reproduced cleanup
  failure and microphone/provider quality faults. No default was changed.
- VAD playback -> Bus -> Realtek completed a correctness-mode diagnostic
  collection and cleanup. Quality failed (including boundary soft events and
  final native wait timeouts). The muted output and unrelated microphone
  recording establish neither acoustic output nor physical delay.
- The real `moiren-app monitor` entry point was exercised with owned, changing
  source processes. Compatible/512 preparation intermittently failed the
  native capture 8-MiB bound; one measured-1024 baseline collected for a second
  then correctly refused its target against measured demand/gap. Another
  attempt stopped capture early and had incomplete cleanup. The source
  workers themselves stopped/joined with exit 0. None of these candidates
  is admitted/stable; the native-size failure remains under investigation.

The first two process wrappers exposed null exit codes. Their stdout/errors
were retained, and later rounds use `Diagnostics.Process` with actual waited
exit codes, concurrent pipe drains and explicit watchdog outcomes. Null is not
reinterpreted as success.

## Follow-up integration and acceptance

Commit `87fbaaa` retains native capture discontinuity/timestamp-error/silence
observations through the Professional publication and physical collector.
Unsupported/contended values remain unknown; startup, checked steady deltas
and final-after-join observations are separate. Focused publication tests
passed 4, physical regressions passed 8, the full probe suite passed 479 and
scoped strict Clippy passed. The concurrent full audio run had 578 pass /
5 fail / 2 ignored while new cancellation regressions were still RED; that
run is not relabeled green. Capture flags alone do not prove waveform
continuity, so `quality_complete` and `qualified` remain false.

S1-S3 remain open: complete native cleanup and Process preparation diagnostics,
retest the actual final binaries, then collect all exact routes with matched
settings and an independent calibrated reference. No VAD/Process latency
winner, stable smaller default, or stable sub-10-ms result is established.
Correctness progress grace remains diagnostic only; hard IO/resource/identity
and cleanup errors are not converted to quality-only warnings.
