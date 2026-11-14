# Removal Production Handoff Implementation Plan

**Goal:** Connect reviewed input generation, one retained diagnostic entry and
closed-run delivery without weakening the original S0-S5 acceptance requirements.

**Architecture:** Keep the existing controller/export/delivery ownership model.
Generate a complete manifest from independently pinned actual target files; hold
every source before importing the target entry wrapper. Persist the original
child's terminal evidence and acknowledgment pin, then use the existing verified
delivery pipeline even when the closed diagnostic reports exit 1.

**Constraints:** No target access before the user's reboot confirmation. No VM
requirement, blanket desktop unlock, auto elevation/reboot, copied OBS trees,
driver replacement, gate relaxation or native release approval. Preserve frozen
613/probe/helper pins and unrelated original worktree drafts. Mocked transport is
not remote acquisition. Evidence delivery is not cleanup or native qualification.

## Task 1: Actual Run Manifest (Dalton)

- [x] Add `scripts/vad-removal/test-run-manifest.ps1`; observe missing API RED.
- [x] Add `scripts/vad-removal/run-manifest.ps1`, deriving actual bounded pins
  from an independently hashed layout, including the complete source set.
- [x] Validate with the existing production input consumer; create-new, flush
  and read back the output. Reject changed originals rather than relabel them.
- [x] Run the new tests in Windows PowerShell 5.1 and PowerShell 7.

Both original focused processes complete17/17. File-only actual-layout
preparation separately refuses a missing independently pinned Catalog tool
before invoking the builder; fixture success is not an actual-layout pass.

## Task 2: Retained Target Entry (Turing)

- [x] Add `scripts/vad-removal/test-entry-launch.ps1`; observe missing API RED.
- [x] Add `scripts/vad-removal/entry-launch.ps1`; use one exact already-elevated
  process and retained bounded dual-pipe writers through natural completion.
- [x] Persist `<run>.entry-terminal.json` independently of the child stdout.
  An original exit 1 may deliver failed evidence; exit 0 without an original
  acknowledgment never becomes delivery authority.
- [x] Exercise harmless real children plus boundary failures on PS5.1/PS7.

## Task 3: Host Handoff (Parent)

- [x] Add `scripts/tests/vad-diagnostic.tests.py`; observe missing API RED.
- [x] Add `scripts/vad-diagnostic.py`, reading an independently pinned local
  manifest copy, verifying all target source handles before the first import,
  and using the existing retained SSH transport exactly once for entry launch.
- [x] Download and validate the original entry terminal receipt/pin; pass its
  independent acknowledgment hash to `deliver_run`, including closed exit 1.
- [x] Persist a bounded handoff receipt with false native/qualification flags.

## Task 4: Integration And Review

- [x] Add new production sources to the canonical set and existing fixtures;
  wire both interpreters and Python tests into existing CI surfaces.
- [x] Independent review the actual combined source and failure tests.
- [x] Run focused and neighboring regressions; inspect current exact-head CI.
- [x] Document actual scope and remaining target/native gates; archive and
  verify this checkpoint. Stage only the requested paths; keep PR26 Draft.

Original matrix:86suites/1032checks, zero failures/drift. Final review's actual
entry UTF-8 defect is reproduced RED on both interpreters, then repaired;
post-fix entry/wrapper/handoff42checks and neighboring Python67tests pass.
The handoff is committed54c1ecd3and combined with the first reviewed main
merge822e5374in c68990a9. The second main snapshot a50d52cdis integrated in
reviewed tree8cc7c515, committeddec2e5d1and combined in bd149cd9. Its final
all-target check,939headless tests/0failures/4ignored, workspace fmt and strict
scoped Clippy finish0; all41archive members read back. Exact published-head CI
still needs observation after the normal feature-ref push; PR26 stays Draft
and main is not merged. New fixture archival is verified, but the literal
PowerShell deletion request is rejected by tool policy; zero reclaimed bytes
are claimed and no alternate deletion is attempted.

Full completion still requires direct target removal, Windows naming/identity,
interactive Apply/Stop, matched waveform and sustained calibrated p95/p99 below
10 ms (or the specified evidenced device-limited fallback). This plan does not
substitute software checks for those requirements.
