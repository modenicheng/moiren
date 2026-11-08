# Removal diagnostic preparation

These Windows PowerShell 5.1-compatible definitions prepare the next finite VAD
removal diagnostic. They are **not a native-approved runnable pipeline**.
Loading them launches no tools. Lower native/tool guards require the exact
retained operation context; the executable release unconditionally refuses
until actual target orchestration/archive review. Changing an approval boolean
is not authorization.

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
.\test-readonly-preflight.ps1
.\test-preflight-collector.ps1
.\test-read-wrapper.ps1
.\test-collector-worker.ps1
.\test-preflight-session.ps1
.\test-session-identity.ps1
.\test-target-readiness.ps1
.\test-preflight-entry.ps1
.\test-diagnostic-lease.ps1
.\test-run-inputs.ps1
.\test-safety-audit.ps1
.\test-diagnostic-controller.ps1
.\test-diagnostic-entry.ps1
.\test-diagnostic-lifetime.ps1
.\test-native-admission.ps1
.\test-native-guard-routing.ps1
.\test-native-query-admission.ps1
.\test-native-admission-lifetime.ps1
.\test-closed-controller.ps1
.\test-closed-export.ps1
.\test-closed-entry.ps1
.\test-closed-finish.ps1
```

The task scheduler, device APIs and target identity are mocked. Real children only output,
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
the native owner remains context-guarded behind a closed release. Provider-idle enumeration is
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

## Read-only target readiness

`invoke-preflight.ps1 -EvidenceDirectory <new-directory> -UserRebootConfirmed`
is the actual read-only target entry. Do not invoke it remotely until the user
has confirmed their reboot. Missing confirmation or wrong host/SID refuses
before imports, process discovery, device collection or evidence creation.
This entry neither launches audio/OBS/ETW nor registers/deletes tasks, changes
drivers/settings, contacts another machine or reboots.

`target-readiness.ps1` retains both an actual `preflight-session.ps1` collector
and `session-identity.ps1` user-session owner. The collector's controller
session can be zero; its value is never replaced with a guessed desktop
session. The separate user owner reads the exact Windows Explorer process
handle/image/token, requires the observed target SID, nonzero session and
Limited token, and rereads the same retained process rather than reopening a
PID. Multiple candidate sessions require `-WorkerSessionId <observed-id>`;
no Explorer/access-denied/unknown identity refuses instead of inventing
session1. This discovery does not need an unlocked desktop and is not proof
that a future worker can access audio. Actual worker/probe binding remains
the final authority; no WTSQueryUserToken/SeTcb assumption is introduced.

Readiness requires fresh post-install boot plus corroborating typed uptime,
frozen613 file/INF/signature/catalog/PnP provenance, complete empty native
owners/pending tasks/raw children, explicit external owners, all six default
roles, render volume/mute and security observations. Original finite children
and runspace remain owned after failure/timeout through natural exit/drains;
no retry/kill or acceptance of late completion follows. The create-new flushed
and readback-hashed target receipt links the original preflight receipt to
the separately observed worker identity. Every assertion revalidates both
retained owners and evidence. `NativeApproved`, `Qualified` and
`AudioAccessProven` stay false.

An alternate collector in host tests is explicitly `InjectedCollectorSource`.
Hashing repository source at construction pins this run, not a released
source package or native authorization. The future combined diagnostic must
require its reviewed repository release/source pins, exclusive ownership,
privileged trace controller, Limited worker, finish/decode, safety-after
audit and archive transfer. This readiness entry is not that full runner.

Full S0-S5 acceptance is unchanged. Use the separate standalone remote target
directly; no VM or blanket desktop-unlock prerequisite. The user owns reboot.
Reuse the fixed dedicated OBS installation; never copy executable trees or
change firewall/security policy to suppress prompts.

## Retained diagnostic controller preparation

`invoke-diagnostic.ps1` is the full-lifetime software entry, not approval to
run the native diagnostic. It refuses missing user reboot confirmation or the
wrong target host/SID before imports and requires an already elevated
controller. It never elevates or reboots automatically. No native target run
has been made with this entry; the lower scheduler/ETW/decode/retirement
dispatch guards require the owner-aware context described below. Its executable
release guard remains unconditionally closed pending target runner/archive review.

`native-admission.ps1` retains exact resource associations and once-only
operation keys, not a Reviewed/NativeApproved flag. Birth rechecks immutable
inputs, actual readiness and deadline after blocking validation; the final
RunEx and post-readback acknowledgement have their own admission checks.
Cleanup requires original controller/resource/writer identity independently of
expired BEFORE or Explorer liveness. Original COM references/XML, full-chain
terminal checks and saved finish/abort evidence guard task retirement. Decode
static pins match the original input snapshot, and canonical tool requests
cannot adopt late alternate tools. Start and tool dispatch intent distinguishes
known predispatch refusal from actual unknown output without retries.
The [dated receipt](../../docs/validation/2026-11-08-removal-native-admission.md)
states test adapters, verification scope and the remaining release/target gates.

The controller retains one `diagnostic-lease.ps1` writer at the fixed
`D:\MoirenTest\removal-613-controller.lock` path through original before
collection, trace, managed worker/probe, finish/decode, after collection and
flushed/readback-hashed result. A live FileStream excludes cooperating writers;
file contents alone never establish ownership. It is closed, not deleted or
rotated. `run-inputs.ps1` requires an independently supplied manifest SHA256
and revalidates the actual production script set, managed26cd706 probe release,
613 signed package/PDB/TMF set, original schedule and exact tool pins.

The original SignTool/catalog/BcdEdit pins cross the actual collector runspace
as detached JSON values and are reused for AFTER. They are not freshly adopted
after admission. `safety-audit.ps1` retains the historical accepted BEFORE
evidence independently of later readiness freshness or Explorer liveness. Its
Audit-mode collector preserves busy/failed snapshots without granting new
admission. Missing/replaced external process identities, changed defaults,
render controls, security, boot or driver provenance are evidenced as unsafe;
external processes are never stopped or adopted as test owners.

Timeout or changed outer configuration closes new admission but still observes
original pending collector/decoder children. An unknown Start/registration/
RunEx/delete result cannot become absence, a fresh retry or a by-name cleanup.
A captured nonzero Start status does not create trace ownership or permit
STOP. A confirmed never-dispatched task has its own archive-before-delete
retirement, at most once; pending AFTER collection cannot repeat deletion.
The root cannot complete or release its lease without original persisted AFTER
and controller result evidence. NativePassed/Qualified stay false even for
safe orchestration; driver removal and physical qualification are separate.

Controller unit tests replace lower boundaries. The lifetime composition test
uses real writer exclusion, actual retained synthetic collector runspaces and
production C# trace-owner buffers with injected native delegates. Its children
only wait/output/exit. Neither test is actual scheduler/ETW/audio acceptance,
an OBS acquisition, or a complete approved native diagnostic.

## Closed-run evidence export

`controller-seal.ps1` privately retains constructor and creation-seam resource
references. It seals only the original persisted terminal result while the
original writer is still held, then confirms that exact FileStream is closed.
Public `Completed` flags, copied controllers and substituted owners grant no
export authority. Identity failure is sticky: original process handles and
already-started collectors/tools continue to drain independently, but cannot
create new bindings, admission, Finish, retirement or a completion seal.
Known pre-BeginInvoke refusals are recorded separately from an unknown dispatch;
no synthetic EndInvoke or missing-output-as-absence shortcut is introduced.

`closed-export.ps1` consumes that original seal and archives the complete closed
run tree, including empty directories and failed evidence, plus the immutable
input/source/result provenance. It copies no probe or OBS executable. The five
staged worker scripts must still match their original captured pins. Limits are
eight run-relative components, 512 total ZIP members, 16 MiB per member and
64 MiB total uncompressed. Reparse paths, executables, unknown artifact classes,
missing terminal pins or any changed membership refuse rather than truncate.

The normal entry and its caught/drained path converge on one export tail.
Create-new sibling ZIP and inventory are independently reopened and every
member length/SHA256 is checked. A second readback binds the original export,
result, nonce, inventory and ZIP to `*.archive-local.json`. Its explicit meaning
is `LOCAL_ARCHIVE_VERIFICATION_ONLY`: it is not remote download confirmation,
cleanup permission, native success or physical qualification. Any archive/ack
failure preserves existing outputs and the refused result, with no retry,
overwrite, deletion or owner relaunch. The separate delivery flow below now
implements transfer/acknowledgment; actual target execution and release review
remain open. The native executable release stays closed.

The closed-controller tests compose actual root/FileStream/collector ownership
with real export and local acknowledgment after injected trace-enable failure.
Exporter-only tests use a synthetic seal getter, and entry-tail tests replace
export IO. These distinct test boundaries must not be presented as native runs.

## Closed-run delivery and acknowledgment

`../vad-delivery.py` consumes an independently supplied original local-ack SHA256,
one canonical `D:\MoirenTest\...` run and a create-new host evidence directory.
It downloads exactly the original local ack, inventory sidecar and ZIP, in that
order. Ack identity/schema/false flags are checked before another download;
inventory bytes are checked before downloading the archive. Target pin paths
are metadata only on the host, never paths to reopen or extract.

The host verifies archive length/SHA256 and every exact member, including empty
files/directories, with the original depth/member/size caps. The embedded
inventory must equal the independently pinned sidecar; inventory membership
excludes itself, so the verifier adds exactly that self entry from its raw bytes.
Nonce, exact result JSON and result/input/manifest provenance must agree. Named
source pins retain their original `Name` field. Unknown classes, executable
images, duplicate/case-colliding paths, links, malformed/duplicate-key JSON,
changed bytes or promoted flags refuse rather than truncate.

`vad-delivery-transport.py` uses only strict-host-key, batch-mode SSH to the
standalone `moiren-lan` target. It streams raw binary stdout into an exclusively
created host file; it does not use overwrite-capable scp, extraction, resume or
automatic retry. One retained process and its original stdout/stderr/input
owners settle even after deadline, cap or write failure. The executable drains
an exposed pending transport owner before rethrowing that sticky failure. A
failed phase never starts the next download/finalization phase.

The host emits `host-delivery.json` with meaning
`HOST_ARCHIVE_DELIVERY_VERIFIED`. Finalization creates the exact target sibling
`*.delivery-host.json`, verifies its readback and calls `delivery-finalize.ps1`
only after independently checking that script and its `closed-export.ps1` and
`finite-tool.ps1` dependencies while retaining their read handles. The target
rechecks the original run/result/local ack/inventory/ZIP chain and emits a
create-new `*.archive-delivered.json`. It restores no private controller/export
CWT and does not reopen old collectors, source tools or external clients.

The host then downloads that exact returned target ack, verifies all original
pins and the exact uploaded host receipt JSON, and persists
`delivery-complete.json`. The historical local ack remains `Transferred=false`;
host/delivered receipts use `Transferred=true`, with `NativePassed=false`,
`Qualified=false`, and no cleanup authorization. This flow can include bounded
PCM/WAV/ETL evidence, so it is not a metadata-only transfer.

The CLI requires `--run-directory`, `--expected-local-ack-sha256`, a new absolute
`--output-directory`, `--finalizer-path`, `--expected-finalizer-sha256` and explicit
`--user-reboot-confirmed`. The independently supplied finalizer SHA must also
match reviewed local source; dependency pins derive from its exact local files.
No confirmation means refusal before target access or output creation. No
remote delivery is claimed by host-only tests. See the
[delivery receipt](../../docs/validation/2026-11-08-removal-delivery.md) for the
actual test boundaries and outstanding native/cleanup gates.

## Production Input And Entry Handoff

`run-manifest.ps1::New-RemovalRunManifest -LayoutPath <reviewed-layout.json>
-ExpectedLayoutSha256 <independent-sha> -ManifestPath <new-file.json>` creates
the actual input manifest without launching any tool. The typed layout contains
`SchemaVersion`, `SourcesRevision`, `SourceProvenance`, `SourceDirectory`, the
complete `Sources`, original `Probe`, `Driver`, `Schedule`, `Tools` and `Symbols`.
All file rows carry independent `Path`/`Bytes`/`Sha256` pins; source and signed
package rows also carry `Name`. No current-file hash is adopted as a substitute
for the reviewed layout or the frozen original binary pins.

`SourceProvenance` explicitly uses `SourceKind=ReviewedBundle`, `BaseCommit`,
`BundleIdentity`, `SourcesRevisionMeaning=ReviewedBundleBaseCommit` and an
independent `BundleManifestPin`. That bundle identifies every source by exact
name/length/hash. The base commit is not a claim that locally reviewed additions
already exist in an exact Git tree. The builder holds original inputs, validates
with `Read-RemovalRunPins`, publishes create-new, flushes, reads back and invokes
the production `New-RemovalRunInputs` validator. It neither signs nor deploys.

`../vad-diagnostic.py` connects a locally retained independently SHA-pinned copy
of that target manifest to one remote entry attempt and the existing delivery.
Its bootstrap verifies the exact target host/SID and already-elevated token,
then locks the original manifest, all production source files and PowerShell
executable before the first import. The complete source membership includes
`run-manifest.ps1` and `entry-launch.ps1`; tests are not target dependencies.
Only canonical existing target paths are consumed; this command does not stage
or copy executables, elevate, reboot or make the closed release runnable.
The real diagnostic child fixes console output to UTF-8 without a BOM before
its first JSON/error output. Host delivery still requires the original strict
UTF-8 bytes; legacy codepages must not replace Unicode paths with question marks
or be repaired by permissive decoding.

`entry-launch.ps1` retains one exact Process and bounded original stdout/stderr
copies until natural exit. Known predispatch channel setup can resume on the
same pipe; ambiguous CopyToAsync dispatch is not repeated. The outer envelope
is 600 seconds, while the native controller remains 240 seconds and direct
Removed remains 20 seconds. Deadline/cap/observation failures stay sticky and
continue draining the original owner, so 600 seconds is not a kill deadline.
Ordinary delivery transport retains its separate 120-second default.

The entry persists sibling `*.entry-intent.json`, `*.entry-stdout.log`,
`*.entry-stderr.log` and `*.entry-terminal.json`. Its local-ack pin must match
the original child's entire strict JSON stdout and unchanged original run,
result, inventory and archive. A closed exit 1 can deliver failed evidence;
exit 0 without a valid original acknowledgment cannot. The host independently
downloads and verifies the terminal and both log pins before passing that
original acknowledgment hash to `deliver_run`. It persists
`handoff-complete.json` only after the original target delivery acknowledgment
has returned. `NativePassed`, `Qualified` and `CleanupAuthorized` stay false.

CLI required inputs are `--manifest-copy`, `--expected-manifest-sha256`,
`--remote-manifest`, `--run-directory`, `--powershell-pin` (a local pin JSON),
`--expected-powershell-pin-sha256`, `--output-directory` and explicit
`--user-reboot-confirmed`; optional `--worker-session-id` defaults to zero.
The release guard still refuses native dispatch. The user has not confirmed
the reboot; no target execution follows this host-only implementation.
