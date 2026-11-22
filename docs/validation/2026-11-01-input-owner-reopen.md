# Raw input capture must stop at a PCM owner reopen

## Reproduced defect and scope

Base `eeda3fa` binds the raw capture whenever `run_playback` creates a new
`PcmOwner`. Its comment already states that reopening an identical native
stamp breaks the diagnostic segment. Aggregate publication telemetry enforces
this, but `CaptureWorker::bind` previously only replaced its owner identity.
After retaining PCM, a second same-identity bind could therefore append bytes
from the next owner to one apparently uninterrupted raw capture.

The frozen hypothesis, alternatives, source hashes and command receipts are in
`D:\coding\moiren-evidence\20261101-input-owner-reopen`. The `red` receipt runs
one regression against the actual production playback pump before and after
that bind boundary. It fails specifically on `worker_exit` versus
`owner_reopened`; it is not a native acquisition or a constructed driver handle.

## Narrow repair

Every bind asks the existing capture seal to record `owner_reopened`. The seal
only changes an open artifact that already has retained identity/data. Rebinding
before the first retained packet remains allowed, including the existing
pre-first-packet reset/startup case. An earlier capacity, shape, identity-race
or failure seal keeps its original precedence. Identical scalar identities do
not prove the same owner instance and are deliberately not compared here.

The original identity, raw bytes and packet records remain intact. The next
owner still pumps its PCM into the real carrier normally; the single-shot
diagnostic simply does not append it. No reset, drop, retry, clock, query,
allocation or lock is introduced by this transition. Public fields and the
schema1 app exporter are unchanged; export remains unqualified. Historical
artifacts are not retroactively repaired.

Three regressions in a separate responsibility-sized module cover:

- Same-identity reopen after actual production-pump publication; both owners'
  changing stereo samples still reach the carrier, but raw evidence stops at
  the first owner.
- Same/changed native identity with open/exactly-full captures; original identity,
  records and prior capacity seal survive, without allocation/deallocation on
  bind or fabricated excluded-packet counts.
- Pre-first-data rebinding retains the first actually observed owner's identity
  and accepts its PCM. Existing reset, join, capacity and publication-race tests
  remain part of the focused suite.

## Fresh verification

- Before the repair, workspace all-target/all-feature tests pass3036 tests in82
  target summaries, with10 ignored. This predates the new regressions and is
  not the repaired-source result.
- Repaired workspace all-target/all-feature tests pass3039 tests in82 target
  summaries, with10 ignored, including all three new regressions and the
  headless gpui tests. No ignored hardware/window exercise is enabled.
- Focused raw-capture tests pass20; standalone production RT shadow passes229.
- Workspace all-target/all-feature strict Clippy and final rustfmt pass.
  Audio all-target/all-feature Linux check passes **compile-only**; its
  Windows-only provider is not exercised by that cross-check.
- Initial rustfmt check reports one trailing blank line after the new module
  extraction; the failure is retained, followed by format and passing rerun.
  The behavioral RED is retained as well. No remote CI or Miri is claimed.

The pre-edit baseline workspace run is retained separately in
`D:\coding\moiren-evidence\20261101-s1-s3-current-acceptance-audit`, with pinned
requirements and a requirement-by-requirement audit. That audit distinguishes
implemented bounded software from missing native/physical acceptance rather
than adding the proposed PCM-worker/Bus fixture as another completion gate.
The latter remains unimplemented and unapproved.

No push, subagent, native acquisition/replay, remote audio, speaker rendering,
driver/system-control write, reboot or memory update occurs. DSP, servo,
carrier policy, operating budgets, defaults and quality gates are unchanged.
Unrelated `gate.log` remains excluded with its original221 bytes and SHA-256.

This closes a host-reproduced evidence-attribution defect only. Original S1-S3
acceptance still lacks clean acquired waveform/common-reference and calibrated
physical results. No sub-10-ms result, impossibility, theoretical floor, lower
default promotion or VAD/Process path winner is established.
