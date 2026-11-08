# Removal diagnostic preparation

These Windows PowerShell 5.1-compatible definitions prepare the next finite VAD
removal diagnostic. They are **not a native-approved runnable pipeline**.
Loading them launches no tools. Real task registration/dispatch and default
trace tools unconditionally refuse until reviewed orchestration exists;
changing an approval boolean is not authorization.

- `finite-tool.ps1` retains exact process/handle/creation and bounded redirected
  output. Timeout retains the same owner, not kill/relaunch or success.
- `task-boundary.ps1` retains scheduler registration intent before mutation,
  one RunEx instance and the first worker Process. It independently checks
  image, command line, creation, session and limited token through that handle.
  Exact task retirement archives and hash-verifies original XML first, requires
  fresh stopped trace and task/worker/probe ownership, and never retries an
  ambiguous delete. Its real scheduler cleanup guard remains closed.
- `trace-boundary.ps1` retains exact finite tool owners, validates inventory,
  preserves ambiguous create/stop failures and raw unknown loss. A successful
  exact create is not independent post-create session readback. This historical
  command boundary remains guarded; it is not the new session's ownership proof.
- `trace-native.ps1` retains the successful StartTrace handle/GUID, one provider
  enable and one exact pre-STOP query/STOP. Returned final counters are copied
  into a read-only receipt; MORE_DATA means stopped with unknown final counters,
  and ACTIVE_CONNECTIONS means pending with the original owner retained.
- `trace-query.ps1` queries that retained session, enumerates the classic WPP
  provider, queries its full-width ULONG LoggerId and rechecks the original
  session. Missing, replaced, truncated or errored results never prove zero loss.
- `trace-readback.ps1` binds admission and monotonic counters to those actual
  snapshots. Final loss needs the original successful STOP receipt, not a
  pre-STOP query or an equal-looking replacement object.
- `trace-session.ps1` composes the owner/query/readback. The collector refreshes
  the exact live session immediately before probe admission. Invalid readback
  blocks admission but does not prevent cleanup of an independently owned trace.
- `trace-decode.ps1` requires the original stopped trace and terminal managed
  task/probe, pins the closed ETL, 613 package/TMF/PDB and actual managed release
  manifest, and retains original finite children before observation. It runs
  the existing offline reader, preserves refusal/raw unknown loss, and writes
  a create-new, flushed and hash-readback receipt. Default tool launch refuses.
- `finish-session.ps1` orders fresh task/worker/probe terminal, one exact STOP,
  post-STOP decode attachment, original child settlement, verified evidence and
  guarded task retirement. The caller obtains closed-file pins only after the
  `AwaitingDecodePins` state, then attaches a decode object before tool use.
  Failed/time-limited tools are observed through their original owners without
  relaunch. Fully drained failure/refusal may permit task retirement, never
  qualification. Missing/mutated evidence or unresolved tools prevent deletion.
  A separately flushed/hash-readback final result pins confirmed retirement
  and original task XML. Post-delete persistence failure retains the outcome
  without repeating deletion or declaring finish complete.

`WorkerTerminal` proves only observed scheduler/worker exit. The managed path
now also binds the live probe independently before admission, retains that
original Process, and observes its natural exit. `Complete-RemovalProbeTerminal`
checks the actual task owner/current observation internally and compares the
child's independently observed exit with the worker result; detached JSON or
a caller-supplied boolean cannot establish full `Terminal`. Terminal ownership
still does not mean native audio success or latency qualification.

- `probe-boundary.ps1` builds exact `managed-removal` arguments, verifies the
  real child handle/creation/parent/session/token/image/hash/command line and
  publishes a bounded create-new admission only after binding.
- `worker-boundary.ps1` owns the child and jointly capped stdout/stderr through
  natural exit and observed drains. Its separate 120-second observation budget
  does not change the eight-second acquisition or 20-second Removed gate. A
  timeout writes one pending receipt and retains the same worker/child; it is
  not kill, restart or cleanup permission.
- `worker.ps1` is the pinned actual task action. `collector-boundary.ps1` stages
  only five small scripts and a pinned configuration, never another EXE tree,
  then observes worker/probe and orders independent admission.
- The Rust probe's opt-in `managed-removal` prefix opens exact controller and
  worker handles and waits at most 15 seconds for the matching receipt before
  entering logging/audio CLI. Ordinary CLI commands are unchanged. A new managed
  release is built/pinned locally at26cd706, not deployed or run; the frozen
  historical probe is not relabelled. Combined target admission is still required.

Run host-only checks from this directory with WindowsPowerShell 5.1 and 7:

```powershell
.\test-finite-tool.ps1
.\test-task-boundary.ps1
.\test-task-process.ps1
.\test-task-retirement.ps1
.\test-trace-boundary.ps1
.\test-trace-finite.ps1
.\test-trace-query.ps1
.\test-trace-readback.ps1
.\test-trace-native.ps1
.\test-trace-session.ps1
.\test-trace-composition.ps1
.\test-trace-decode.ps1
.\test-finish-session.ps1
.\test-probe-boundary.ps1
.\test-worker-boundary.ps1
.\test-collector-boundary.ps1
.\test-worker-process.ps1
.\test-worker-entry.ps1
```

The task scheduler and target identity are mocked. Real children only output,
wait and exit. No logman/tracelog, ETW, audio, OBS, remote target or driver is
operated. `test-task-process.ps1` requires a non-elevated Windows host process
in an existing nonzero session; it verifies real child token/CIM/command line
and natural exit, not actual scheduler behavior. Other device/security/task
boundaries remain mocked. Generated test outputs are ignored, not sources.
`test-worker-entry.ps1` exercises the actual production entry's pinned bounded
loader and rejection paths on a non-target host. It also verifies retention on
unexpected entry failure with a synthetic owner; no real native launch occurs.
For actual cross-language receipt exchange, build the example **test** executable
with `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe
--no-run --message-format=json`, then run `test-managed-protocol.ps1` with its
exact `-TestExecutable` and `-ExpectedSha256`. It selects one exact Rust test
fixture whose admitted closure only writes `cli-stub-only`; it never invokes
the audio CLI. It must not receive a release/native executable instead.

`finite-tool.ps1` is pinned by trace SHA256 to the reviewed predecessor. Its
synchronous file/hash/dispatch calls are not hard-bounded. COM/CIM/property
reads are not hard cancellation or crash recovery. Retain unresolved owners.

The five trace suites run the production C# layouts, allocation/parsing and
retained-owner composition with only native API delegates replaced. They do
not create a real ETW session. Real native start/enable/query/STOP dispatch in
the native owner remains unconditionally guarded. Provider-idle enumeration is
not an atomic reservation against another controller; combined admission must
exclude competing owners rather than claim this observation prevents stealing.

Remaining integration includes fresh post-reboot provenance, reviewed actual
collector/privileged trace/Limited worker dispatch, bounded target collection,
actual closed-ETL decode/report collection and authorized task cleanup. The
software finish composition connects these retained boundaries, but does not
approve native dispatch, prove tracefmt output-file growth hard-bounded, or
provide restart/crash recovery. `finish-evidence.json` explicitly precedes task
deletion; `finish-result.json` pins its hash and the retired original task XML
only after confirmed retirement. Neither file asserts native qualification.
Managed process tests use harmless fixture entries, not native audio. Scheduled
registration/dispatch and trace launch remain disabled. A same-user writable
receipt directory is correlation, not a hostile-process security boundary.
Worker/controller crashes after admission still require observation of the
original surviving owner; this is not durable crash recovery or task deletion.
This preparation's default stop also refuses; it must not be adopted as a
running native trace's cleanup authorization policy.

Full S0-S5 acceptance is unchanged. Use the separate standalone remote target
directly; no VM or blanket desktop-unlock prerequisite. The user owns reboot.
Reuse the fixed dedicated OBS installation; never copy executable trees or
change firewall/security policy to suppress prompts.
