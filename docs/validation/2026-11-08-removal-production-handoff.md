# Production Input, Retained Entry And Delivery Handoff

Date: 2026-11-08, Asia/Shanghai. Base: `1d7aab3`. This continues the
[closed-run delivery](2026-11-08-removal-delivery.md), not a new native verdict.
The full S0-S5 acceptance remains open.

## Actual Production Handoffs

`scripts/vad-removal/run-manifest.ps1` generates the actual consumer schema from
an independently SHA-pinned typed layout. It holds regular bounded originals,
checks every reviewed source/tool/symbol/package file and the unchanged
26cd706 probe/23807f3 signed613 content. A complete independently pinned
`ReviewedBundle` records the source base commit and exact source bytes without
claiming additions already exist in that Git tree. The existing consumer
validates before create-new publication and after durable readback.

`scripts/vad-diagnostic.py` reads an independently pinned local copy of the
target manifest. Before importing target PowerShell, its fixed direct-SSH
bootstrap checks host/SID/elevation and opens read-sharing-only handles for the
original manifest, every production source and the pinned shell executable.
It does not copy a binary, elevate, reboot or bypass the closed release guard.

`entry-launch.ps1` launches exactly one original child. It retains Process,
creation identity, original dual-pipe copy tasks and jointly capped 1-MiB logs
through natural exit. Missing provably predispatch channel setup resumes on
that original source; an ambiguous copy call is never repeated. The independent
600-second outer envelope does not alter the controller's240seconds or the
direct-Removed20seconds. Deadline failure drains rather than kills/relaunches;
ordinary delivery keeps its120-second default.

The original child's strict whole JSON stdout supplies the acknowledgment pin.
That pin must agree with the unchanged actual ack/result/manifest/inventory/
archive before the target publishes a create-new terminal receipt. The host
verifies the returned receipt pin, downloads that exact terminal and both logs,
then independently checks the original stdout acknowledgment before using its
SHA in `deliver_run`. Closed exit1 is eligible failure evidence; exit0 without
an acknowledgment or any incomplete/unknown orchestration is not delivery
authority. Final handoff keeps `NativePassed=false`, `Qualified=false` and
`CleanupAuthorized=false`.

## Verification Boundaries

Evidence root: `D:\coding\moiren-evidence\20261108-production-handoff`.

The missing production host handoff first fails RED. A second actual RED shows
that replacing the original acknowledgment with a current receipt hash would
reach delivery; the host now independently verifies retained stdout/log bytes.
Fourteen focused host tests pass, including the generated real PowerShell
bootstrap refusing a mutated dependency before import and the real CLI refusing
an unconfirmed reboot before output or remote access. Thirty-six retained
transport tests pass; defaults remain120seconds, a fixed per-owner600-second
entry envelope is bounded/typed and still sticky after natural completion.

The entry tests exercise real harmless child processes, strict stdout/ack
binding, normal/failed exits, cap draining and second-channel setup refusal.
The manifest tests use real filesystem hashing and the production consumer,
with only frozen content constants substituted for tiny fixtures. They do not
run tools/native collection or certify actual target layout availability.

The separate file-only actual-input preparation stops before calling the
generator: the fixed Catalog tool original
`D:\MoirenTest\20261029-0118a54\bin\w00_probe.exe` is not available locally
under its required SHA256
`58552AEC72650250FA0C01A309B6B885561A38F925842175C1C5B0EEEB0F081A`.
The bounded local evidence/tool search finds no matching original. The receipt
`actual-inputs/actual-inputs-refusal.json` records one prerequisite refusal,
zero builder calls and zero successful actual manifests; its SHA256 is
`E93209140F1D414093584A98D2A803A888E524BA7FC188F5B9D35DE22EC0DE26`.
No frozen pin is substituted and no test-machine access is made to fill this
gap. This is not an actual-layout generator pass.

The additional composed fixture runs actual Finish/seal/export, actual entry
wrapper with a harmless original-envelope emitter, host handoff, real delivery
verification and a separate target-finalizer process. Readiness/native owners,
exact local config admission and byte transport are explicit fixtures, not
SSH/target/native operations. Both normal and offline-association-refusal cases
pass under PowerShell7. Earlier fixture-only missing `Refused` and reserved
`$Host` errors are retained rather than described as native failures or passes.
The original frozen43-suite-per-interpreter process finishes with86suites,
1032checks, zero failures and zero source drift. Every recorded log hash is
independently verified against `final-matrix-terminal.json`. This receipt is
the pre-encoding-repair source, not an unqualified final-source total.

## Published CI And Remaining Work

The pre-integration GitHub observations show PR26 OPEN/Draft, head1d7aab3, mergeable=false,
mergeable_state=dirty, merge_commit_sha=null and no exact-head run. The PR's
cached base SHA is c8ab5e4 while actual branches/main is7e4024e. The workflow
runs on pull_request and main push, not this branch push or workflow_dispatch;
this is not a silently omitted path surface. Predecessor dcda8cc's successful
run42472082524 does not cover this head. Main integration is prepared in a
separate worktree; no unreviewed merge/rebase of the original drafts occurs.
That isolated merge is now committed as `822e5374`, tree
`8a6170677ae49ad7c904fdd8edd81b8b11f00a40`, with parents `1d7aab3` and
`7e4024e`. An independent static review covers all seven content conflicts,
thirteen feature relocations and five additional migrated test modules.
Headless app tests pass939, with four ignored, including13UI acceptance tests;
all-target gpui-test check, formatting and strict scoped Clippy finish0.
The staged merge/closed verification logs are archived in
`main-integration-8a617067.zip`,649166bytes/104members, SHA256
`6EC20AD888F15DEAEEC366A45BC2BB6AA713F7B4878CFE4E7177AFFBC3558C47`,
with every member independently read back. The handoff implementation is
committed as `54c1ecd3`; `c68990a9` combines it with `822e5374` without changing
the tested application tree or the frozen probe/driver bytes.

Main then advances to `a50d52cd`. A second isolated integration retains the
preference-store grouping and all feature assertions. Its reviewed index tree
`8cc7c5152ebe17b390ddf5d7548f8d916320eedf` is committed as `dec2e5d1`, then
combined in `bd149cd9`. All26application source pins match the tested tree;
scripts, driver, backend, core, engine, signals and Cargo.lock remain identical
to `54c1ecd3`. The final all-target check, headless tests939passed/0failed/
4ignored, workspace fmt and strict scoped Clippy all finish0. Initial fmt and
E0308 fixture failures remain archived; the source snapshot was taken during
the test, not falsely described as before its launch. Independent review
covers this exact staged tree. `combined-a50-review.json` SHA256 is
`7D9694FBC2803FDD1F2B63BB2BD3638568CDA5A76725B4234BCAFA2587DFEBC6`.
`main-integration-a50-8cc7c515.zip` is190067bytes/41members, including26source
files and original terminal/log pins; every member is independently read back.
Its SHA256 is
`2F755B11C743F1241E86AD88423365F2E8D3FD10E445ED32CF4C73F1D5D3158D`.
This is local feature integration, not a merge to main or native acceptance.
Publication must remain a normal fast-forward of the feature ref with PR26
Draft; only the resulting exact-head run can establish its CI verdict.

The final independent review also identifies an actual-entry encoding gap:
`invoke-diagnostic.ps1` does not set child console UTF-8, although host delivery
strictly consumes the original stdout as UTF-8. Harmless test children set it
explicitly and therefore do not cover this real-entry difference. The actual
entry, with only its three target/elevation/root guards replaced by explicit
harmless fixtures, first fails all four cases under each interpreter: inherited
437loses Chinese paths/text and936does not produce valid strict UTF-8. Source
stays frozen until the original full matrix is terminal. The actual entry then
sets no-BOM UTF-8 before any output, without changing host decoding.
Final focused entry tests pass6checks under each interpreter, including the
original two early refusals. Entry wrapper and actual handoff/delivery
composition also pass, for42checks across six post-fix suites. The67neighboring
Python tests pass. Only the entry and its regression differ from the original
81file matrix freeze; post-fix receipts pin that exact scope. No native call
is made by the encoding fixtures.

## Closed Test Artifact Cleanup

The completed matrix's new owned regular fixture roots are archived to
`closed-new-matrix-fixtures.zip`,14670253bytes, SHA256
`51B75DAEA3F245A77BA38991BE6578C6C2C0720C9A53D8BB61E228D824E02AB3`.
All15572files in59roots are read back against exact source hashes, totaling
44127762source bytes. Two export-test roots contain intentional junction
rejection fixtures and are excluded rather than traversed or deleted. The
subsequent native PowerShell literal-path cleanup request is rejected by tool
policy before execution; all59roots remain and zero bytes are reclaimed.
`new-fixtures-cleanup-policy-refusal.json` records that refusal. No alternate
shell/language deletion is attempted, and no old draft/cache or remote OBS
installation is touched. Additional post-fix fixtures are outside this frozen
cleanup inventory.

No test-machine operation, reboot, driver/audio/OBS action, firewall change,
new OBS EXE tree or native-release opening occurs. The user still performs any
reboot and has not confirmed it. Target placement/provenance and executable
release review remain necessary before the next finite diagnostic; direct
Removed, Windows names/identity/settings, interactive Apply/Stop, matched audio
and sustained calibrated p95/p99 below10ms (or the specified evidenced device
fallback) remain unqualified. Verified evidence transfer is not cleanup
authorization, and cleanup refusal is not a claim that storage was reclaimed.
