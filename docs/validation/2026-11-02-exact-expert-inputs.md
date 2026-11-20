# Exact Expert Process / VAD input editing - 2026-11-02

S1-S3 application work continues while a separately owned remote native test
runs. First-packet silence remains deferred. This slice does not change audio
algorithms, admission, defaults, driver code or physical qualification.

## Behavior

The existing Expert editor now offers exact integer entry for capacity, target,
jitter, maximum packet and maximum delivery gap, separately for Process and VAD.
Preset buttons remain available. These are 48 kHz input frames, not measured
latency. Declared / Empirical provenance remains explicit; manual text does not
become a measured Process selection.

Save / Restart captures all current text synchronously before the normal settings
transaction, including activation before another render/event-drain. Invalid
text cannot submit a preceding valid number. Portable shape/relationship checks
and native complete-demand admission remain authoritative. Save while stopped
does not start audio; running restart remains pending until requested settings
and the exact active owner match.

Discard, policy replacement and returning to presets clear corresponding text
state deliberately. Delayed callbacks check entity identity and busy state.
Input focus owns typing/graph shortcuts. Device-only settings do not capture
unrelated global expert text.

## Software evidence

Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.

- `s3-expert-red-0234.log`: three new headless tests fail because the exact
  Expert controls do not exist, before implementation.
- `s3-expert-green-0239.log`: four focused tests pass, covering nonpreset VAD
  4800/945, all five invalid fields, combined gap/jitter refusal, policy/discard,
  focus and delayed running-owner confirmation.
- `s3-expert-stale-red-0258.log`: removing the entity guard fails the explicit
  delayed-Change negative control (old945 overwrites new2048). Guard is restored.
- `s3-expert-app-green-final-0300.log`: 625 passed / 3 ignored, including the
  explicit stale-entity callback regression. These fixtures do not open native audio.
- `s3-expert-workspace-0241.log`: 2092 passed / 8 ignored across 67 test/doc-test
  targets. `s3-expert-clippy-final-0300.log`: strict workspace/all-target Clippy.
- `s3-expert-release-final-0246.log`: release build succeeds. fmt and diff checks
  pass. Read-only review reports no findings; existing dirty VAD/probe work is
  explicitly excluded from review and staging.

Latest successfully verified deployed binary:
`D:\MoirenTest\20261102-s3-expert-inputs-0257\bin\moiren-app.exe`, 33991168 bytes,
SHA256 `E8A0E998834ABFE92566B4B1CC765556C9F242EA983F4694626255FCA8B48372`.
`s3-expert-source-manifest-final-0247.json` pins HEAD ced9858 plus exact dirty
sources; it is not clean-commit binary provenance. The final rebuild hash replaces
the preliminary build hash; only the final pinned binary may be used for the
native receipt. `s3-expert-source-manifest-final-0303.json` additionally records
the later test-only explicit delayed-Change control; production guard is unchanged.

## Parallel remote test boundary

A task-lifecycle nine-minute Process baseline uses the preceding binary
611D172B...C138, not this editor build. Shared336 / render512 / Engine64 /
measured input2048, capacity16384 and jitter96 are explicit experimental intent,
not a new default or a relaxed gate. The source also renders directly to DAWN:
this is mixed-output operational telemetry, not isolated PCM or physical latency.

First attempt at `20261102-s3-baseline2048-long-0232` fails before native source
startup because the existing source worker permits only 1..600 seconds and the
runner requested 660. The task reaches Ready/result1, source exit2 and no owners;
OBS/QQMusic are unchanged. The failure receipt/logs are preserved. Only after
inspection, a new directory/task `20261102-s3-baseline2048-long-0238` uses source
600 seconds and monitor540 seconds. This is a runner correction, not an audio
fix. No hidden restart, fault threshold change or default promotion is made.

All audio/device/native UI execution stays on LAPTOP-NP49RG6H. Local compilation,
headless tests and offline evidence checks are separate. External OBS12648 and
QQMusic15096 are preserved. The [terminal baseline receipt](2026-11-02-process-baseline2048.md)
confirms540001ms operational completion but seven delivery-gap-bound violations;
telemetry-clean qualification remains false. This is not a matched A/B against1344.

## Terminal native editor receipt

Successful deployment:
`D:\MoirenTest\20261102-s3-expert-inputs-0257\bin\moiren-app.exe`, same final
E8A0E998...8372 binary above. Task `Moiren-S3ExpertInputs-20261102-0257` uses a
restored stopped project with unavailable prior Process identity, not automatic
default physical capture. It finishes02:53:15+08:00, Ready/result0.

Native UIA invokes VAD Expert and exact editing, enters4800/945/96/480/480,
submits invalid945.5 and verifies it does not persist the preceding valid target,
then saves945 and verifies all five exact values plus Declared provenance in
isolated preferences. Process policy remains Compatible. No audio owner starts;
native Quit exits0 without forced cleanup, remaining owners or cleanup errors.
Original OBS/QQMusic remain unchanged. This proves stopped editing/persistence,
not native admission of945 under any active output demand.

Two earlier native harness attempts are explicitly unsuccessful and preserved:
0243 used a default graph that automatically started physical capture and invoked
a clipped Expert policy; 0253 used the stopped project but invoked the exact
toggle in the footer-overlap/clipped area. Both reach Ready/result1 and require
owned-app forced cleanup; original external processes are unchanged. Their logs
also contain GPUI Windows0x80040102/0x80070578 during cleanup, not a new VAD
0x8007051A result. No claim of native success relies on these attempts. The final
runner adds a conservative footer margin, checks the toggle state after invoke,
and retains failed evidence instead of treating Invoke return as confirmation.

Verified downloads, with the final screenshot visually reviewed:

| Artifact | Bytes | SHA256 |
| --- | ---: | --- |
| `s3-expert-native-result-0301.json` | 112779 | `F71374FD138186FB13B970C1D678635ED80A26DD896C3CE4E6ED2E0B18A9C203` |
| `s3-expert-native-screen-0301.png` | 260860 | `C230DC1758C50A1E872883A2F030F0EE39A6AFCDFC1D7AE1DEC49885B5559FDB` |

Screenshot visibly shows4800/945/96/480 and saved feedback; maximum-gap row is
partly clipped. Its480value is established by ValuePattern plus the persisted
preferences receipt, not by pretending the screenshot displays all five rows.
Final manifest receipt: `s3-expert-native-manifest-0301.stdout.log`.
All18 unrelated files remain hash-equal/excluded (`s3-expert-preserved-0302.json`).
No push, memory update, admission weakening or default promotion; S1-S3 remain
incomplete and unavailable physical calibration does not block application work.
