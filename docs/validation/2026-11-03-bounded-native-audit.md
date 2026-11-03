# Bounded native audit without recursive report copies

Date: 2026-11-03, Asia/Shanghai. Software baseline: `10f3477`.
This repairs an evidence-runner failure, not audio, desktop acceptance or
physical qualification. No new native audio/UI task or long soak is started.

## Reproduced boundary

The preceding [target1280 acquisition](2026-11-03-lower-target1280.md)
retains a10,696,617-byte report, but its wrapper recursively copies both
`native.engine` and `collection.graph.engine`. Each includes the same large
startup history. Windows PowerShell5.1 pretty-printing produces a43,684,340-byte
audit and87,368,614-byte wrapper log, exceeding the unchanged32MiB downstream
reader. This is independent of the original native quality refusal.

The original closed report reproduces the amplification through the old
runner's actual assignment and Save blocks: a reduced integration audit still
reaches42,354,055bytes and fails the bounded assertion. A separate large-history
fixture also fails before repair. Missing-helper errors from an initial
integration harness attempt are retained, not counted as the regression RED.

## Narrow change

`scripts/native-report-sections.ps1` supplies a filesystem-only projection:

- Engine and collection graph become explicit references to the original
  `/native/engine` and `/collection/graph`, with report relative path, length
  and SHA256. The references say `file_backed_json_section_not_validated`.
- Other collection fields and published backend diagnostics remain literal;
  missing sections remain null. The original report is not mutated or trimmed.
- Audit serialization is capped at1MiB and uses CreateNew. The original32MiB
  report cap is unchanged; overflow refuses rather than increasing limits.
- Explicit depth checking prevents PS5.1 silently stringifying deep objects.
  That review finding has its own actual failing-then-passing regression.

The evidence-only `runner-v2` uses schema3, requires the pinned helper before
loading it, adds it to mandatory deployment pins, and emits only a tiny stdout
receipt. The old runner directory, intent, task outcomes and reports are never
overwritten. This revision is tested offline, not deployed for a native run;
its original create-once paths intentionally cannot rerun the prior case.
Actual argv, functional/quality expressions, source period, capture caps,
held-child deadlines, cleanup and post-catalog gates remain unchanged.

## Verification and remote closed replay

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-compact-native-audit-20261103-0855`.
Remote roots: `D:\MoirenTest\20261103-compact-audit-0855` and the separate
`20261103-compact-audit-0855-depth` follow-up. These are finite offline batches,
not native application/audio tasks.

Final helper21checks pass on PS5.1 and PS7 locally and remotely. The existing
target126/runner124/post-catalog7 checks pass on both host shells. The actual
runner assignment/Save/stdout integration preserves quality=false; its final
PS5 audit is111,505bytes and stdout308bytes, versus PS7's34,604/293bytes.
Authentic/missing/modified helper filesystem checks verify pre-load refusal.
Independent reviews identify and close the depth and pre-load-pin findings.

The first remote replay finishes08:53:46 with a109,152-byte full projection;
all92original artifact pins match before and after. Its original native task
remains Ready1, selfcheck Ready0, quality=false and native.error unchanged.
The8,107-byte output archive and all four entries are length/SHA256-verified:
`E30BA0D5C7A3188AB328415F60A2B7FAE716AB5B246F8AE4C02596D54A474CDF`.
The separately pinned depth-guard revision passes both21-check suites at
09:00:00 and reads the same original report, preserving all92pins. Its
108,394-byte sections-only projection is not the same payload as the first
full audit; neither size is represented as a new native result.

An exact clean HEAD archive plus only the two helper/test overlays excludes
all27original dirty/cache files. Concurrent feature-on Windows workspace
verification passes2359tests/9ignored/0failed across71harnesses, all-target
check, strict Clippy and formatting. No host native audio/UI is launched.

## Remaining boundaries

This removes an observed audit-size failure for the closed fixture and guards
future oversize/depth refusal; it does not retroactively repair the failed
wrapper or mark native acceptance successful. No quality threshold, default,
buffer, RT path or first-packet behavior changes. No new waveform plot or
latency measurement is produced by a JSON projection.

S1-S3 software is substantially implemented; successful native retained
Evaluate/Apply, desktop acceptance and calibrated physical/sustained gates
remain separate open evidence. Do not repeat the closed UIA hypotheses or
unchanged soaks merely to keep the goal active. Missing physical equipment and
the deferred first packet do not block independent development.
