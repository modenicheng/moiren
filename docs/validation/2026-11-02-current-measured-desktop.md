# Current measured Process desktop replacement

Date: 2026-11-02, Asia/Shanghai. Production baseline: clean `7508720`
(same application source as the repaired build in
[requested/active diagnostics](2026-11-02-process-diagnostics-accessibility.md)).
This is one finite ordinary per-wake measured-budget desktop acceptance,
not live-window or evaluated closed-prefix acceptance, a waveform result,
a long stability qualification or a physical latency measurement.

## Isolation and pins

All native audio, GUI and device activity runs on remote `LAPTOP-NP49RG6H`,
Limited Interactive session 1. The development host only builds, runs headless
software tests and reads compact evidence. The eighteen preexisting dirty
PCM/SRC/driver paths and both Python cache directories are excluded from the
clean source export; no audio policy, default, driver or analysis gate changes.

Remote application available for manual testing:
`D:\MoirenTest\20261102-measured-desktop-1430\bin\moiren-app.exe`.
SHA256: `7282994A001122C520CF4386A334F61076D321EED165A85AA664A872AEB6144F`.
No driver install is needed for this Process route. The existing loaded
`MoirenVad.sys` stays Running at SHA256
`4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.

Source probe:
`D:\MoirenTest\20261102-matched-graph-1020\bin\e2e_latency_probe.exe`,
SHA256 `AB03BE0D3FA9C6230B31E70E33C19EF535382A214E2CF48CEB5370038F1B1685`.
Catalog probe:
`D:\MoirenTest\20261029-0118a54\bin\w00_probe.exe`, SHA256
`58552AEC72650250FA0C01A309B6B885561A38F925842175C1C5B0EEEB0F081A`.
The source renders to the explicit Voicemeeter In 2 endpoint; Moiren captures
the exact owned Process tree, then unity Bus -> DAWN PRO2. Engine remains
48 kHz/F32/block64, output Shared336/actual native capacity738,
render target512/wake64/callback64. The initial bridge is Compatible2048/8192.

The task `Moiren-MeasuredDesktop-20261102-1430` uses an isolated project and
preferences directory, direct GUI supervisor and exact held owner identities.
The existing thirty-second launch admission,150-second worker watchdog and
240-second task cap are unchanged. The source's finite90-second lifetime is
a guard, not the changing schedule duration; it is stopped explicitly below.
No OBS recording or change to the user's OBS/QQMusic is made.

## Exact refusal and successful replacement

1. Stopped ordinary Apply is natively disabled, has no InvokePattern and shows
   the expected stopped preflight refusal. Start then confirms the exact
   Compatible/native/render tuple and both requested/active native summaries.
   Only after these checks is the owned changing source activated.
2. Native ValuePattern edits request target1024/capacity16384/jitter96. The
   ordinary action is disabled with no InvokePattern. Its visible reason is
   `minimum 1335 = demand 704 + delivery gap 533 + jitter 96 + lookahead 2`
   in48kHz input frames. Capacity16384 is storage, not operating target; the
   UI explicitly describes this as modeled reserve, not measured latency.
   Requested and active both stay Compatible, still so after a one-second
   recheck; app owner1 is not replaced. No1024 launch or512/256 scan occurs.
3. Change only target to1536 and invoke `Apply measured budget and restart`
   once. Requested equals active exactly: source NodeId(0), PID25464,
   creation134380753718183678,1536/16384/96 at48kHz, origin session1/owner1/
   revision2, selection1, ingress elapsed limitDisabled. Confirmation occurs
   at source elapsed20.3676714s, before the scheduled changing signal ends.
4. Logs confirm app owner1 cleanup finished14:43:38.258115 and reap14:43:38.259313
   before owner2 starts14:43:38.349164. Native generation1 is backend-local;
   it need not become2 when app generation2 replaces the owner. Native UIA
   diagnostics independently report actual bridge Prime1536/capacity16384,
   output336/capacity738 and render512/capacity738, with no fallback tuple.
5. Stop follows completion of the40-second source schedule and more than eight
   seconds after exact measured confirmation. Source active time before Stop
   is40.0443643s. App owner2 cleanup finishes14:44:00.244137 and is reaped
   at14:44:00.245382 with cleaning0. Stopped diagnostics retain the exact
   session2/source identity/native1 predecessor. The source stops and joins.
6. Quit is fully revealed above the fixed footer: UIA y1071..1120, safe
   bottom1125; the owned-window screenshot agrees. Invoke returns and app exits0.
   A requested viewport resize is not asserted as an observed final size.

The worker starts14:42:50.7290297 and finishes14:44:08.8508504; supervisor
finishes14:44:08.8949848. App17112/creation134380753721623795,
source25464/creation134380753718183678 and worker24436/
creation134380753702741937 all exit0. No forced termination, cleanup error
or supervisor error occurs. Final read-only poll14:46:28.9978016 finds task
Ready/result0 and no native owners. Result is `completed=true`, `qualified=false`.

## Changing-source exposure and limits

The explicit source schedule is40s/1,920,000 frames at48kHz stereo,
39 distinct indexed codes at0.5..38.5s, each1024frames at amplitude0.025.
The same sample appears on both source channels; these are not independent
measurement channels. Submission receipts cover the entire schedule and
39,936 nonzero frames; silent native preroll1056frames is counted separately.
Final source elapsed42.3230981s includes the tail/cleanup. Native stop succeeds,
worker joins and final source failure is null.

These are successful ReleaseBuffer submission receipts, not proof of played
or captured samples. The measured owner overlaps the changing schedule, but
no PCM is recorded or analyzed in this run. Do not claim correspondence,
waveform continuity, zero whole-run quality faults or physical latency.
The stopped Process trace remains explicitly refused at `EventLimit`; it is
not bypassed or converted into a trace PASS. Ordinary per-wake admission does
not require that archived prefix. Live-window and evaluated closed-prefix UI
need separate native acceptance and are not covered by this attempt.

Earlier Process18/19 and VAD17/19 waveform refusals, the later graph batch
**NOT ANALYZED** admission and old failed desktop attempts remain unchanged.
First-packet investigation stays deferred; no unchanged soak, relaxed gate,
lower default, path-winner or calibrated sub10ms claim follows. Independent
product work need not wait for unavailable physical measurement equipment.

## Evidence integrity and reader failure

Host evidence:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-current-measured-desktop-20261102-1430`.
Original remote evidence remains under
`D:\MoirenTest\20261102-measured-desktop-1430`.
Worker SHA256: `E08AAB9424FA6325B3F3CAF372A9A804E7CF940F709C0C653D38E9D6240047B3`.
Supervisor SHA256: `82ECA861C70B8203B660EAABD2383026572B759CDE377B8CF7207ECBA9D6F7F5`.

The first polling reader fails to parse UTF8 summaries containing a Unicode
ellipsis under Windows PowerShell5.1 default decoding; polls02/03 retain those
errors. The worker and supervisor's UTF8 result handling are unaffected and
finish normally. The corrected read-only poll explicitly uses UTF8 and
`native-poll04-utf8.json` verifies the original terminal result. This is not
a new audio run, retry, altered acceptance gate or overwritten failure verdict.

All33 compact archived files are verified by length and SHA256 after download.
Archive803246bytes, SHA256
`FF30CE6130C9670C196C84428F1E8AC1FF232C4C0CE2D72CE74D7EF619DA761B`.
Original JSON, UIA snapshots, stdout/stderr, source receipts and screenshots
are retained. Visual review covers1024 refusal,1536 requested/active summary,
native diagnostics and fully revealed Quit; screenshot viewport coverage is
not substituted for exact offscreen diagnostic values in the UIA snapshot.

Protected OBS12648/creation1793505454949 and QQMusic15096/
creation1793504036422 remain unchanged, as do default endpoint roles,
catalogued render volumes/mutes and the running driver pin. No app/source/
worker owner remains. This closes the current ordinary measured desktop
refusal/replacement/Stop/Quit surface, not the original complete S1-S3
waveform/native/physical qualification objective.
