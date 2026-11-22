# Current Desktop Exact-Demand Acceptance

Recorded2026-11-03, Asia/Shanghai. Scope: one current-build ordinary production
desktop refusal, measured replacement, joined Stop and Quit. This is not a
waveform, sustained stability or calibrated physical latency qualification.

The earlier [ordinary desktop receipt](2026-11-02-current-measured-desktop.md)
used7508720, demand704 and target1536. This distinct attempt verifies that the
[exact-demand repair](2026-11-03-exact-master-admission.md) reaches the actual
desktop: demand528, target1024 refused and target1280 explicitly activated.
No unchanged soak, lower-target scan or fallback is performed.

## Build, Isolation And Preparation

Production source: `002aa21f0256f223e328fb40d56ce3675499a60c`.
The clean-source release is reused without rebuilding the dirty worktree:
34350080bytes, SHA256
`B56B0AECD297A8BC708CAC4878D97FD6D7040184D09CCD751CB151A4BDAF3E75`.
Its final software verification remains in the
[timeline implementation receipt](2026-11-03-render-timeline.md):2461passed/
10ignored feature-workspace tests, strict Clippy and fmt. These tests are not
rerun or recounted as native desktop results here.

Local evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-desktop-exact-demand-20261103-1730`.
Remote root: `D:\MoirenTest\20261103-desktop-exact-demand-1730`.
The manual-test app is `bin\moiren-app.exe` under that remote root. No driver
installation is required for this Process route.

Only `LAPTOP-NP49RG6H` over `ssh moiren-lan` runs audio/UI. Host activity is
limited to preparation, pure checks, source review and evidence handling.
The app receives no CLI arguments; UIA starts from its exact held HWND, not the
desktop root. Project, APPDATA and TEMP are isolated. Existing user OBS12648/
creation1793505454949 and QQMusic15096/creation1793504036422 are protected.
Matching identities do not alone prove uninterrupted user audio.

The prior timeline task and its recovered final audit are terminal before this
launch. The existing successful worker/supervisor is adapted rather than replaced
with a new framework. Preparation passes44pure checks on each PS5/PS7 and a
compile-only supervisor build. Collection helpers pass16pure checks per shell;
none of these local checks launch audio or the supervisor. Independent bounded
review finds no confirmed launch or collection blocker.

Final worker33915bytes, SHA256
`C347A07460528E3991BC858F39AA89140F26DC68D41D4229F8CEBCBD40C296AE`.
Supervisor17920bytes, SHA256
`0AADAEEE3CAC0D744371FC96225A6B8D289B9847103DB0753135606D9DB154B9`.
Reviewed/readiness-finalized intent1448bytes, SHA256
`5A728FB975B370C98A4291D22E6A61FF29555312B29D56EC179196D245C5EA45`.
The unfinalized frozen intent and its hash are retained separately.

Existing source probe7485952bytes, SHA256
`AB03BE0D3FA9C6230B31E70E33C19EF535382A214E2CF48CEB5370038F1B1685`,
is reused from `D:\MoirenTest\20261102-matched-graph-1020\bin\e2e_latency_probe.exe`.
The explicit source renders to Voicemeeter In2. Moiren captures only that owned
Process tree, then unity Bus -> DAWN PRO2. Engine48k/f32/block64, output
Shared336/capacity738 and render512/wake64/callback64 stay unchanged. Initial
Process bridge remains Compatible2048/8192; measured capacity16384/jitter96
is independent of target1280. No default, endpoint volume or timer is changed.

## Actual Desktop Lifecycle

Task `Moiren-DesktopExactDemand-20261103-1730` is launched once. The held worker
deadline150s and outer task cap240s are unchanged. Worker runs17:37:44.5967403
to17:39:10.1363575; supervisor ends17:39:10.2090541, all +08:00.

1. Stopped Apply is disabled, without InvokePattern, with the matching-running
   preflight refusal. Start confirms the actual native/render tuple before the
   owned changing source is activated.
2. Native ValuePattern edits request1024/16384/96. Apply is disabled and its
   visible reason reconciles exactly: minimum1161 = demand528 + delivery gap535
   + jitter96 + lookahead2, in48k input frames. Requested/active stay Compatible;
   after another second, the original owner is still unchanged. No1024 start or
   smaller target scan occurs. The gap is an observed modeled reserve input,
   not a physical delay or a whole-run delivery guarantee.
3. Change only target to1280 and invoke ordinary Apply once. Requested and active
   match exactly: NodeId(0), source PID25160/creation134381722669009178,
   target1280/capacity16384/jitter96 at48k, origin session1/owner1/revision2,
   selection1 and ingress elapsed limit Disabled. Confirmation occurs at source
   elapsed21.0103347s, before the40s schedule ends. Diagnostics independently
   expose actual Prime1280/capacity16384 and unchanged Shared/render settings.
4. App owner1 cleanup finishes17:38:38.941077 and reap17:38:38.943765 precedes
   owner2 start17:38:39.093500. Backend-native generation1 is not a global owner
   identity. No live ring resize or fallback tuple is reported.
5. Stop occurs after the complete source schedule and more than8s confirmed
   dwell, at source active40.1578516s. Owner2 cleanup17:39:00.910025 and reap
   17:39:00.911447 complete before the source stop command. The stopped UI
   retains the exact predecessor/source identity, then the source stops/joins.
6. Quit is revealed above the fixed footer and invoked. App25936/creation
   134381722688622089, source25160 and worker25444/creation134381722635815918
   exit0. No forced cleanup, cleanup error or supervisor error occurs.

At17:39:31the task is Ready/result0, supervisor/worker completed=true and
qualified=false, with no remaining app/probe/supervisor/analysis owner. Protected
identities match; before/after default roles, render settings and driver state/
path/hash agree. The running driver hash remains
`4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.

## Source And Evidence Limits

The unchanged source schedule is1920000frames/40s at48k stereo,39indexed-v2
1024-frame markers at0.5..38.5s, amplitude0.025, with scheduled silence between
markers. It is not a continuously nonzero signal or independent stereo capture.
All1920000source frames and39936nonzero frames are acknowledged by successful
ReleaseBuffer submissions; native silent preroll1056frames is counted separately.
Final source elapsed42.6992636s includes tail/cleanup; stop succeeds, worker joins
and failure is null. The probe's3000ms diagnostic jitter tolerance is unchanged
and does not relax production audio or latency gates.

These are submission and desktop lifecycle observations, not proof of played,
captured or continuously intact samples. No OBS, PCM recording, correlation or
physical measurement runs in this attempt. Stopped Process trace still refuses
at EventLimit, rather than being upgraded to a trace PASS. Retained closed-prefix
Evaluate and live-window native success remain separate acceptance gaps; their
automation timing limits do not establish a product defect. Earlier waveform
residuals and delivery-gap warnings remain unchanged. First-packet work stays
deferred; no lower default, source-path winner or sub10ms result is claimed.

## Verified Compact Archive

The create-new archive804110bytes, SHA256
`EE5F4849319F6E34D7B2C3C59353DA97E456C750378C457BAB0E52BED4DE9431`,
contains33compact payloads. External manifest SHA256 is
`4DA240F24CE2E99BBE41DC0A272AF9179C6D937141EDB080EBA22AFC5FABCCDD`.
Every archive entry is checked for exact name, size and hash, including unchanged
source pins; the host independently verifies the transfer and all33payloads.
No raw audio crosses the network. Actual refusal, active1280, native diagnostics
and revealed Quit PNGs are inspected. Offscreen diagnostics come from the exact
UIA snapshots, not assumed screenshot coverage.

The independent17:43:55..17:44:08read-only final audit exits0 with SSH exit0.
It rehashes all33original compact files, archive, external manifest, app and
finalized intent:38pin references across37unique files. Default roles, all12
render endpoint settings, driver/boot and protected creation identities agree
with the retained baseline. No native/analysis owner or running Moiren task
remains; no cleanup, audio/UI launch or raw transfer occurs. Its create-once
2582-byte receipt SHA256 is
`DCD8AF15DDA3E2DA13F33F0DF29B34DC77F6DD53FF82FAEE9305DAAF2D05AC0C`.

The27preexisting dirty/cache pins and unrelated main-worktree audit files remain
unchanged. This increment updates evidence only, with no production change,
unchanged rerun or push. It closes the current-build ordinary desktop ticket,
not overall S1-S3 waveform/native/sustained/physical qualification.
