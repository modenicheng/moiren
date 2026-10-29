# DAWN native Process desktop follow-up

Host date: 2026-10-30, Asia/Shanghai. Physical target:
`LAPTOP-NP49RG6H` / `192.168.12.97`. The target's unchanged clock reports
2026-10-25; no cross-machine wall-clock subtraction is used.

This receipt continues the native desktop controls audit. It is not a
waveform-continuity, physical latency or S1-S3 completion certificate.

## Authorized output and isolation

The user identifies the newly connected DAWN as a headphone amplifier with
headphones attached and authorizes its volume adjustment. No other speaker
volume or mute state may be changed. The pinned active render endpoint is:

`{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}`

Catalog identity is `DAWN PRO2`, 48 kHz, stereo Float32, volume scalar 1,
mute false. Its observed default period is 100,000 units of 100 ns and its
minimum reported period is 30,000 units. Those properties are not an
end-to-end delay measurement. Realtek remains volume 0 / muted.

The test makes zero endpoint-volume, endpoint-mute or default-role writes.
Before/after catalogs compare all active render endpoint volume/mute states
and every default role. DAWN was already the default render endpoint in the
initial catalog; the harness does not assign it as default.

Each hidden production desktop instance has isolated APPDATA, a preloaded
explicit `Process Loopback -> Bus -> DAWN` project, and a pinned owned source
PID plus creation timestamp. The source also renders explicitly to DAWN;
it does not use a virtual endpoint whose external mixer routing is unknown.
Its changing markers have amplitude 0.025. Both direct and processed paths
can therefore reach DAWN. This is not an independent dual-channel reference
capture or a physical latency A/B comparison.

The project runs 48 kHz / F32 / block 256 with no extra DSP nodes and unity
Bus/send gains. Native observations confirm Shared Float32 stereo output at
48 kHz and an actual 1056-frame device buffer. That buffer is distinct from
the Process input target and does not establish physical delay.

## Original production desktop evidence

App source `e195bfe`, SHA-256
`09A0053326179928FB20E6F90DD1472501EF4A7F2D10907BEE0E9C550566F93A`,
and probe `f72e03c` remain unchanged during the first three attempts.

- `desktop-dawn-process1` restores the isolated project successfully but
  waits for Stop without starting it. A restored project is initially idle,
  unlike the earlier ordinary default-project startup. This is a harness
  failure, not failed audio preparation.
- `desktop-dawn-process2` explicitly invokes Start. DAWN opens and owner
  generation 1 starts. Its global source-label lookup finds both the graph
  sidebar and settings selector, so the harness refuses the ambiguous match.
- `desktop-dawn-process3` scopes settings lookups to the modal. It explicitly
  selects capacity 16384, target 4096, jitter 96, and invokes the real measured
  Apply. Logs record generation 1 cleanup/reaping followed by generation 2
  starting on the same DAWN endpoint. This is native owner-replacement
  evidence, not merely an Invoke return.

The last candidate is diagnostic, not a proposed low-latency preset. Its
target must not be called a measured physical delay. Native requested/active
confirmation is still unavailable in the original app: both the before and
after UIA trees omit the status/summary text. Save has no InvokePattern in
the retained post-Apply tree. The helper then fails and falls back to bounded
shutdown, ultimately forcing this owned app process to exit. GPUI logs retain
`0x80040102` / `0x80070578` window errors. That run does not establish normal
Quit or native confirmation. The source stops/joins with exit 0. All output
volume/mute states and default roles remain unchanged.

## Accessibility repair

Commit `60d4864` adds `Role::Label` to measured feedback and the shared
requested/active summary builder. Their existing IDs and exact labels remain
unchanged. It does not change settings, admission, buffers, DSP, native owner
replacement, or default selection.

Root cause is the missing role: the two original native trees contain none
of these labels, and GPUI's accessibility implementation explicitly omits
nodes without a role even when an ID/aria label is supplied. The existing
service/UI tests cover requested versus active confirmation separately; a
fresh production native tree must verify this repair instead of treating
software fixture success as native evidence.

The attempted repair validation uses the shared `D:\MoirenBuild\target` and
existing Cargo home. Its measured/UI release-test build hits the explicit
30-minute watchdog while still compiling dependencies; the retained result
is TimedOut=true, not a test or compilation PASS. The release application
stage never starts. No new application is deployed and native run 4 is not
launched. The dependent, bounded follow-up task records a build failure and
exits without deployment. Caches are retained.

Before resuming the same source/profile with that cache, the exclusive-build
guard detects a different native Cargo build, PID 20952, with parent 15640:
`cargo build --release -p moiren-app --locked`. Its compiler commands target
`D:\moiren\target\release` and the normal user Cargo home, not this task's
`D:\MoirenBuild\target`. Its ownership is not established. The resume launch
refuses before creating its scheduled task; the external build is not stopped,
and no new acquisition runs against it. The accessibility repair remains
committed but unverified by a compiled/native retest. Local formatting and
diff-whitespace checks pass. No driver reinstall or cache clean occurs.

## Remaining boundaries

DAWN resolves the lack of an authorized physical render endpoint; previous
muted-Realtek limitations describe historical runs, not current routing.
Independent common-clock reference capture and calibration remain absent.
No physical delay distribution, VAD/Process latency winner, smaller default,
continuous-waveform pass or stable sub-10-ms result follows from this work.

## Checkpoint retention

All three completed attempt directories and the watchdog build logs are
downloaded under `target/physical-1921681297-20261029`. The 67-file host
inventory is `dawn-checkpoint-host-manifest.json`, SHA-256
`FC0EE820B04E7408308C082602DC2E891CD315AE90630EDFC1D3AC1DD2C4A192`.
It is a local evidence inventory, not an independent authenticated recording.
Remote originals remain retained; no evidence is pruned without a verified
archive. The planned four-run archive is not produced because run 4 never
starts.

The post-audit receipt confirms all five owned terminal scheduled tasks are
unregistered, no app/probe audio process remains, and the deployed app still
matches the original `e195bfe` hash. The unrelated Cargo build remains running
and untouched. Shared caches are retained; D has 268,809,793,536 bytes free
at that snapshot. C free space is 6,239,395,840 bytes; this task's configured
Cargo home, target and temporary paths remain on D.
