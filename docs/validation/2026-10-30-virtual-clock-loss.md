# Virtual-clock omitted-period accounting

Date: 2026-10-30, Asia/Shanghai. Audio source: `b56ae8b`.
Integration source: `95e5677`.
This is an offline S1/S2 observability repair. Native audio retries remain
paused after the [target bugcheck](2026-10-30-vad-software-readiness-crash.md).
It does not establish historical clock-loss totals, native PCM loss, crash
causation, physical latency or S1-S3 completion.

## Reproduced gap and repair

The pure-virtual owner can omit overdue Engine periods beyond its configured
recovery cap. Previously this appeared only in the optional bounded64-event
debug tail. Native observations and final probe quality could miss the loss.
The existing integrated fake clock waits10periods with cap1: it renders1 and
omits9. The new final-observation assertion first fails with `null` instead
of9; a separate final-only-loss probe regression also fails before its repair.

The owner now publishes cumulative rendered periods/frames, late-wake bursts,
clamped periods/frames, timeline resets, maximum backlog and a sticky arithmetic
saturation flag. Immutable rate/block/cap and schema1 accompany each snapshot.
Clamped frames mean omitted Engine frames, not independently measured native
PCM loss. Scheduling, recovery limits, transport budgets and defaults are
unchanged.

One writer publishes eight fixed atomic scalars with a non-wrapping sequence.
Readers make one bounded attempt: unstarted, contended or exhausted publication
is explicitly unavailable. There is no retry loop, allocation, logging or
per-frame clock read. The original owner/session retains its probe through
registry replacement and final join; it cannot acquire a successor's counters.
Even the directly constructed transport's fallback probe returns in `Exit`
for control-side reclamation.

Live probe collection skips an unavailable clock observation without inventing
zero loss or a cached fresh sample. Present invalid/reset/saturated observations
retain their raw evidence and validation error before failing the operational
guard. A pure-virtual route requires a valid joined-final clock sample and
rendered progress. Final-only clamped loss fails strict quality; correctness
collection may finish without making that quality pass. Live and final
cumulative samples are never added together. A physical-master route has no
virtual clock.

## Verification

After review repairs, host verification passes:

- Audio library:615passed /2ignored.
- Probe example:528passed.
- Counting-allocator integration:4passed, including publication/read accounting
  with zero allocations and deallocations.
- Strict audio all-target Clippy and workspace formatting.

Regressions cover9omitted periods /2304frames after join, retained trailing
old-generation loss, more events than the debug tail, bounded contention,
version exhaustion, arithmetic saturation, fully repaid jitter without loss,
and persistent overload with19renders /6resets. Physical-master observations
remain `null`. App changes only add the new optional field to test fixtures.
Final source review finds no remaining P1/P2 issue.

The first remote full app run retains567passed /1failed /3ignored. The graph
test `graph_curved_wire_click_after_fit_selects_real_edge_without_changing_topology`
asserts visibility of a clipped inspector control before waiting for the
panel's opening animation. Three fixed, isolated repetitions of the same
binary all fail at that assertion. The graph test/harness are identical to
`origin/main` before adjustment. Commit `95e5677` moves the existing wait before
both visibility assertions, retaining the initial render, real route click,
selection/topology and gain assertions. It changes no production UI behavior;
the original failure remains evidence, not a successful run.

## Physical-target offline integration and deployment

The owned clean source at `D:\MoirenBuild\s1-s3` advances from `9c86a1f`
through bundles, without intermediate remote pushes. The target is idle before
each build/deployment phase. Cargo uses the existing D: home/target/temp,
jobs2 and no cache clean. Source `95e5677` passes all six finite stages:

- Release app all-target strict Clippy with `gpui-test`.
- Exact graph visibility regression:1passed.
- Full app library/integration/headless UI suite:630passed /4ignored across
  12test binaries; the library contributes568passed /3ignored. The live default
  device startup test remains ignored, not counted as native validation.
- Release probe example:528passed.
- Release application and probe builds.

At2026-10-30T18:11:14+08:00, verified release artifacts replace the formal
`D:\MoirenTest\20261029-0118a54\bin` entry points:

| Binary | Bytes | SHA-256 |
| --- | ---: | --- |
| `moiren-app.exe` | 33490432 | `9D2478D610EADA4DA933C8DF85B97BBFC1DB3AFC6E936850EFC91CE29E9ADB7D` |
| `e2e_latency_probe.exe` | 7026688 | `690A09A4F2C060FDF2A4BF2562A3A3BD0E68E0F4F1E2899421A6E6D8F3BEBB50` |

The original files remain hash-verified in
`rollback-before-virtual-clock-95e5677`; the earlier readiness-specific probe
also remains unchanged. Each replacement uses same-volume `File.Replace`.
The script checks exact source, six successful stage receipts, current binary
hashes and the unchanged Running driver before mutation. Its failure path
quarantines an invalid success receipt, restores only known changed binaries
from verified original backups, retains both backups/new-file quarantine, and
records unresolved recovery errors. Four harmless file-fixture tests under
Windows PowerShell5.1 cover success, second replacement failure, manifest-write
failure and post-publication readback failure. Final review finds no P1/P2 issue.

The final18:11:54audit independently rechecks deployed/rollback hashes, clean
source, no build/app/probe processes, driver Running and root PnP OK. Installed
SYS remains `CCC0EC4FE4DDB51FB22B90A483E58FF3874D87FA8634D62C93BB8AA52FFFE471`;
it is not reinstalled. D free254283345920bytes, C free10355019776bytes;
3939774593bytes of reusable Cargo output are retained. There is no native audio
run, volume/mute/default-role write, system-setting change or requested reboot.

## Durable evidence and remaining gates

`virtual-clock-95e5677.zip` is29968bytes, SHA-256
`B521AE5AFBB41C77679ACBAAAC85E8627F522461CFE63DA06A66130EB9001968`.
The archive contains45manifest files plus the manifest itself, including the
initial failure/three fixed reproductions, all final stage results, deployment
receipt/scripts and independent final audit. The host verifies the exact
entry set, every length/hash, final source/stage exits, installed/build identity
and test counts; it explicitly retains the earlier exit101 results.

The archive, verifier/output, RED/GREEN host logs, build bundles/scripts and
deployment/recovery fixtures are copied outside Cargo output into
`D:\coding\moiren-evidence\20261030-virtual-clock-loss`. Its backup manifest
records every retained file's length/hash. No remote measurement, system dump,
driver rollback or cache is deleted to create this checkpoint.

Freshly fetched `origin/main` remains `c0bd006`, already contained by this
branch. Original S3 native desktop Apply/replacement evidence remains valid;
the new app deployment is not another native Apply test. S1 waveform/reference
quality and S2 native lower-budget feasibility remain open. Native retries
remain paused after the bugcheck pending a safe target-testing decision;
the small dump does not establish sole crash cause. No historical acquisition
is relabeled, VAD/Process winner selected, lower default promoted or stable
sub-10ms result claimed. S1-S3 and the overall goal remain incomplete.
