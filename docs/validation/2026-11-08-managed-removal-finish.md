# Closed-ETL diagnosis and managed removal finish

Date: 2026-11-08, Asia/Shanghai. This source integration advances the finite
diagnostic for S1's unresolved VAD removal. It does not repair or qualify
native removal, Windows naming, waveform continuity or physical latency.

## Actual composition and ordering

`scripts/vad-removal/trace-decode.ps1` now connects the retained stopped native
trace/readback/STOP receipt with the original managed controller, registered
task, worker, independently bound probe, full arguments and exact artifact
report. Pins include the closed ETL, diagnostic613 package, matching flat TMFs
and PDB derivation, tool images, existing offline reader and original probe
release manifest. The actual managed `SourceHead`/`Files[]` manifest is supported;
malformed current manifests cannot fall back to an unrelated historical schema.

`scripts/vad-removal/finish-session.ps1` uses the existing production boundaries
in this order:

1. Independently reobserve this task, worker and probe as terminal. Cached
   `Terminal` flags do not authorize STOP or cleanup.
2. Stop this retained trace once. Pending/unknown STOP retains the original
   owner without retry. Status234 may establish stop, not final counters.
3. Expose `AwaitingDecodePins`; only now can the caller pin the closed ETL and
   attach the actual decode object. Retain it before any tool invocation.
4. Attempt decode once. Observe every original finite child, both redirected
   stream copies and flushes through exit, even after dispatch exceptions or
   timeout. Never restart, kill or reacquire by PID to hide uncertainty.
5. Preserve the original create-new decode receipt's byte/hash pin. Recheck
   inputs, outputs, original nested owner references and full managed provenance
   before saving independently flushed/hash-readback `finish-evidence.json`.
6. Verify that evidence again before guarded exact task retirement. Original
   task XML is archived and hash-verified by the existing retirement boundary.
7. Persist create-new `finish-result.json`, pinning preceding evidence and the
   original retired task XML. A post-delete persistence failure keeps the
   retired owner and refuses terminal completion; deletion is never retried.

Decoder refusal or nonzero/late exit remains failed diagnosis. It can permit
retirement of a safely resolved task, not `NativePassed` or `Qualified`. Missing
or replaced receipts, changed inputs, foreign provenance, unresolved tools and
existing result paths prevent deletion. MORE_DATA raw loss remains unknown.

## Verification

The 16 CI preparation suites pass **213 checks per interpreter** under Windows
PowerShell5.1 and PowerShell7. Decode contributes17 and finish19. The existing
offline stage reader separately passes29 tests. Workflow YAML, formatting and
diff checks pass. New decode/finish suites are wired into the existing Windows
CI matrix; these local results are not a hosted result for the new commit.

Finish tests exercise the production C# retained owner, actual Marshal-based
query/STOP parsing, task/probe terminal correlation, decoder and Python reader.
Only native ETW delegates, scheduler access and managed process metadata are
fixtures; finite decoder children are harmless real PowerShell/Python processes.
They are not real tracefmt, audio or driver runs.

Observed failing-then-passing regressions cover missing composition, same-run
receipt replacement, original report/input mutation, aliased full-argv changes,
and terminal-result persistence. Other cases cover live original worker exit,
foreign owners, pending STOP, MORE_DATA, decoder refusal/nonzero exit, exact
pending child observation after dispatch failure/timeout, evidence collisions,
and ambiguous deletion without retry. Independent static review clears the
final production hashes; the parent runs the process composition tests.

Evidence root:
`D:\coding\moiren-evidence\20261108-112100-collector-worker`.
`finish-freeze-suite-receipts.json` pins source hashes and the32 final suite
logs. `managed-removal-finish-source-and-logs.zip` is independently read back
member-by-member; its verified archive receipt records size/hash/membership.
Closed host fixtures remain because the earlier deletion attempt was rejected;
no alternate-shell/language deletion bypass or complete-cleanup claim follows.

The predecessor published `26cd706` completes exact-head CI42454664245 with all
20 jobs successful, independently queried and with original watcher36444
exiting0. This result does not qualify the later source delta. PR26 remains Draft.

## Remaining native admission and acceptance

Real task registration/dispatch, trace dispatch, default decoder tools and
task deletion still unconditionally refuse. The combined actual runner still
needs fresh post-reboot target/package/token/session/endpoint provenance,
exclusive ownership and safety readback, finite acquisition, verified archive
transfer and authorized cleanup. File caps after tool exit do not hard-bound
tracefmt output-file growth; synchronous hashing/COM calls are not cancellable
deadlines. No durable controller-crash recovery is claimed.

The frozen installed613 and the locally pinned managed release are unchanged;
neither is relabelled or remotely replaced. No remote command, native audio,
driver/device action, OBS launch/copy or reboot occurs in this source slice.
The user owns any reboot. Use the standalone target directly, without a VM or
blanket desktop-unlock condition, and retain the one fixed dedicated OBS path
for future recording tests. Every original S0-S5 acceptance requirement remains
open where its actual native/physical evidence is missing.
