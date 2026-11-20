# Native encoded-render evidence hook

Recorded 2026-11-03, Asia/Shanghai. Baseline `edc5f41`.
This connects the [bounded storage foundation](2026-11-03-render-evidence-foundation.md)
to the actual WASAPI write boundary, without enabling a production collector.
It is not a native waveform, sustained stability or physical latency pass.

## Scope and ownership

- `NativeStream` exposes the actual negotiated encoded layout and an opt-in
  pre-Start attach/take contract. Unsupported adapters return the writer intact.
  WASAPI rejects mismatched layouts, undersized packet scratch, duplicate,
  closed, used or already-staged writers before installation.
- The actual write path stages the encoded native bytes, ends that borrow,
  releases the complete lease once, then publishes the raw signed HRESULT from
  owned scratch before propagating a release error. The vtable call preserves
  nonzero success HRESULTs that the generated `Result<()>` wrapper discards.
- Staging refusal never prevents or retries audio submission. Null-buffer and
  encode-error silent fallbacks increment explicit unobserved packet/frame
  counts; neither synthetic PCM nor a successful fallback result is invented.
- A native-owner guard takes and closes the writer before native destruction
  on preparation failure, cancellation, ordinary stop and unwind. The thread
  result holds its scratch until the control-side joiner receives it. Native
  destructor failure cannot consume the retained writer. Existing native
  resource destruction and recovery authorization ordering stay unchanged.

The staging helper returns before ReleaseBuffer, rather than carrying a buffer
reference parameter across COM invalidation. A regression invalidates and frees
the native backing allocation during release, while stored evidence retains the
original bytes. Audio acceptance remains distinct from diagnostic storage loss
and physical playback. Both evidence endpoints are still control-reclaimed.

No backend option, application setting, automatic collector, CLI output or
archive format is added here. In particular, the previous nominal master FIR
projection is not retroactively upgraded to actual submission evidence. A
collector and a distinct full-application native acquisition remain follow-ups;
the direct native smoke below exercises this hook without that collector.
No driver, audio default, queue target, first-packet policy or gate changes.

## Verification and provenance

Tests precede the native API implementation and fail with the missing API and
helper diagnostics. Initial focused integration passes 27 tests, then 29 with
pending-stage admission, unobserved saturation and actual worker lifecycle
coverage. The all-target check finds a real compile integration mistake: an
example shadow-includes the codec without the new evidence module. Returning
only encoded layout primitives from the shared codec removes that coupling;
the typed evidence conversion stays in WASAPI. The original failure is retained.

Independent review identifies a native-buffer reference parameter spanning
ReleaseBuffer in the first helper. The final split ends the stage-only helper
before release and retains the regression that frees the backing allocation.
Ordinary tests check ordering and copied bytes, not an aliasing-model proof;
Miri is not claimed because the review environment lacks its encoded-MIR
sysroot. This review finding and repair are distinct from a native audio fault.

At 12:17:45-12:17:47, the remote host executes the exact pinned software-test
binary and passes all 29 focused cases. Protected OBS and QQMusic process
creation identities match before/after. This is a device-free software test,
not a native audio run. A separate finite full-library remote run accompanies
host integration; no unchanged audio soak or device/OBS contention is required.
That original 29-case binary's full remote library run completes at 12:25:35:
831 pass, four ignored, zero failures. It precedes the final borrow-scope split
and extra regressions, so it is not substituted for final-source verification.

The initial isolated feature-workspace passes 2,389 tests, nine ignored and no
failures across 71 harnesses; standalone RT passes 248. Workspace all-target
check, strict workspace all-target Clippy and formatting exit zero. After the
borrow split, layout matrix and ignored native-smoke addition, final source
passes 31 focused cases with the native case ignored, the full audio library
833/five ignored/zero failed, RT 248, feature-workspace all-target check, strict
audio all-target Clippy and formatting. The original full-workspace run is not
mislabelled a second full-workspace run. All 1,411 archive entries match baseline
Git blobs and all 11 final source overlays match their worktree hashes.

## Finite remote native smoke

An explicitly ignored test requires the exact remote computer and a caller-
pinned endpoint. It is not run by ordinary host tests. One limited interactive
remote task opens DAWN Shared Float32 stereo, requested period336/capacity738,
with preallocated diagnostic capacity4096 and max packet738. It submits 96,000
deterministically changing frames (two seconds) and compares every retained
encoded frame against the codec output. This is a submission-boundary test,
not a loopback waveform, delivery-gap, audible-quality or physical-delay test.

The first task ends Ready/result1 with only the harness header and no result
receipt. A device-free PS5 reproduction demonstrates that redirected native
stderr under ErrorActionPreference=Stop throws NativeCommandError and interrupts
the wrapper. The initial attempt stays incomplete; no native PASS is inferred.
A separate Start-Process capture initially exposes a missing fast-child exit
status; acquiring the held process handle first fixes that status check. The
final device-free control retains stdout/stderr and exact exit0. Only after
those controls does a separate corrected native task run, with the original
binary, settings and gates unchanged; this is not an unexplained retry-for-green.

Corrected task runs 12:37:25-12:37:28, test1 pass, child exit0 and task Ready0:

- 285 release attempts, 96,000 accepted/stored/observed frames, byte-for-byte
  agreement with the expected actual encoded stream, no rejected frames.
- Evidence overflow, abandoned reader, unobserved/pending frames, invalid
  attempts and counter saturation are zero; wait timeouts are zero.
- Start/run/Stop/release complete normally; writer closes and reader drains.
  No physical latency, sustained-quality or S1-S3 acceptance follows.

Binary SHA256 `AC00D62199D1FF85E022971040B09E27D7B2C6F995DA58957A9F4BDC4D83CAB9`.
The two compact logs are transferred and verified by byte length and SHA256.
Final 12:37:56 audit has no owned process or running Moiren task, original task1
and corrected task0 retained, protected OBS/QQMusic creation identities unchanged.
No driver, default, OBS configuration or host process is operated. Long device
soaks are not necessary to validate this two-second new boundary question.

Host evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-render-native-20261103-1210`.
Remote software-test root:
`D:\MoirenTest\20261103-render-evidence-1210`.
Builds use an exact HEAD archive plus explicit intended overlays, excluding the
27 preserved dirty/cache files. No host audio/UI is launched and no host process
is stopped. Compact logs/receipts transfer instead of raw audio.
