# Virtual budget diagnostics - 2026-11-02

Application development continues independently of bounded remote native tests.
First-packet investigation stays deferred; physical calibration remains an open
qualification boundary, not a reason to block useful application development.

## Change

The existing live native Diagnostics section now projects the already-prepared
`PreparedVirtualBudget` for both Virtual playback and Virtual microphone:

- Engine rate, processing block, complete consumption burst and recovery cap;
- native 48 kHz transfer, FIR lookahead and optional wake allowance;
- delivery-bound provenance, packet/gap bounds and optional negotiated packet.

This is read-only presentation of existing preparation evidence. It does not
change graph processing, microphone servo direction, carrier target/capacity,
native negotiation, admission, defaults, recovery or the driver. Engine frames
and native 48 kHz frames stay on separate rows. Model reserves are explicitly
not latency. Compatible absent bounds and an absent negotiated packet remain
Unknown; declared/empirical/native-packet bound provenance cannot synthesize a
negotiated packet. In particular, an immutable initial DTO is not updated just
because a provider subsequently admits a client.

The existing phase/generation/running filter applies before these rows. Missing
budgets do not generate fictitious rows for unrelated streams. Both English and
Simplified Chinese labels preserve units, provenance and measurement limits.

At 48 kHz / Engine64 / cap1, the existing 4800/945/96/480/480 Declared request
prepares burst64, transfer65, FIR32, wake96, low97/high4320 and minimum673 input
frames. Default cap8 needs the complete 512-frame burst and does not admit945
under the same target policy. These are model results, not native quality or
physical latency. No new preset/default is introduced.

## Software verification

Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.

- `s2-vad-budget-red-0302.log`: two actual failing tests identify the missing
  Engine-demand row before production implementation; eleven adjacent tests pass.
- `s2-vad-budget-app-0307.log`: full app library 519 passed / 2 ignored. Four
  added tests cover both directions, stale/idle/stopped hiding, Compatible
  Unknowns, negotiated-packet distinction, all provenance variants and Chinese.
- `s2-budget-workspace-0309.log`: 2096 passed / 8 ignored across 67 targets.
- `s2-budget-gpui-0309.log`: 629 passed / 3 ignored, no native audio devices.
- `s2-budget-clippy-0309.log` and `s2-budget-gpui-clippy-0309.log`: strict
  all-target Clippy passes. `s2-budget-release-0309.log`: release build passes.
  `cargo fmt --all -- --check` and `git diff --check` pass.
- Read-only review reports no new concrete code findings, including the
  microphone target/servo audit. The eighteen preserved dirty files remain
  hash-equal and are excluded from this slice (`s2-preserved-0308.json`).

Release binary: 34006528 bytes, SHA256
`49987B9FE05F0E187C61F6FD6296A4AE32B575941F5D6D7E67DEED3B5F364D9C`.
`s2-budget-source-manifest-0318.json` records HEAD f3a3e68 plus exact dirty file
hashes; this is not a clean-commit binary. It is staged at
`D:\MoirenTest\20261102-s2-budget-diagnostics-0318\bin\moiren-app.exe`.

## Native diagnostic smoke

Limited Interactive task `Moiren-S2BudgetDiagnostics-20261102-0318` uses fresh
owned IDs ending91/92 and isolated preferences with the same Engine64/945/cap1
request. It opens no external source, recorder or physical audio endpoint.
This verifies native presentation, not PCM quality. At03:16:55.557 through
03:17:26.057+08:00, UIA confirms all six exact rows for Playback and MIC,
including the honest Unknown negotiated packet. Stop reaps the owner, then
native Quit exits0 without forced termination, diagnostic/cleanup errors or
remaining owners. Task is Ready/result0. Original OBS12648/QQMusic15096,
default roles and render volumes are unchanged.

The stored UIA tree contains the exact labels even when those lower rows are
outside the screenshot viewport; this is not a claim that the top-of-page PNG
visually displays all six. The native screenshot is not used as row evidence.
Verified result `s2-budget-native-result-0322.json`:118266bytes, SHA256
`D21ADD67C703D499001F984C23CE7534B2CAA476E492ABBF800D6DADFE039A39`.
Remote `03-budget-verified.json`:27038bytes, SHA256
`FD0D9C9E60DA5FE0EAB6144EFC154E7FE2780446FF9F9010579AAFA4572B00EF`.
Manifest and owner/task audit: `s2-native-final-manifest-0324.stdout.log`.

The first launch request fails the runner hash guard before creating/starting
a task: its expected hash was computed from pre-write text, not the actual
Windows newline bytes. The exact existing uploaded file is rehashed and the
new launch receipt verifies
`718F14F2AC80D5182790BD4FAD2A96EB89E6D3055C9A033E8397681D66D6418B`
before the single successful launch. No audio settings or executable changed.

The parallel [945/cap1 native receipt](2026-11-02-vad945-cap1.md) is a failed
quality candidate, not evidence that this presentation feature fixes the
existing running MIC completion fault. No push or memory update is made.
