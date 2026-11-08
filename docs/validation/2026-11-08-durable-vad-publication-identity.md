# Durable VAD publication identity - 2026-11-08

This is an implementation checkpoint toward S4, not Windows name acceptance,
native rename/default-role qualification, or an aggregate S0-S5 pass. The
separate test target still awaits the user's reboot of diagnostic613. Nothing
in this change installs, restarts, or operates a driver/audio endpoint.

## Identity contract

The old interface/circuit reference includes the displayed name. A rename can
therefore expose a different Windows audio endpoint even when the saved graph
keeps its StableId. All interface categories and ACX already share the same
policy-owned reference; this change supplies its durable machine-local origin.

- The base v2 capability reply remains0x3f and the v2 CREATE wire layout is
  unchanged. A separate identity capability query negotiates the new request.
- `CREATE_IDENTITY` uses an independent version1,176-byte request. Validation
  checks both reserved fields, direction, channels, name, nonzero StableId and
  explicit mode before storage, UUID allocation or child publication. The
  validated request is normalized to the existing168-byte v2 internal shape;
  replies retain the existing snapshot/deferred ABI.
- `EstablishNew` is explicit authority from a newly allocated application ID.
  The root's persisted installation UUID plus StableId and direction generate
  a name-independent publication reference. A retry retains that identity.
- `ReopenOrAdopt` is for saved IDs and later recreations. It reads a committed
  child record or uniquely adopts supported historical interface references.
  Missing, partial, conflicting, malformed or foreign-installation provenance
  refuses rather than inventing a replacement endpoint identity.
- The installation UUID and child record use their hardware registry keys.
  Exact typed reads, flush and readback precede interface/circuit publication.
  The record validates schema, installation, StableId, direction, bridge pin,
  reference length and canonical zero padding. Passive control-side registry
  operations do not enter the real-time processing path.
- Historical adoption uses `IoGetDeviceInterfaces`, interface property queries
  and alias checks, not symbolic-link parsing. All three category references
  and ACX consume the same committed child reference.
- A legacy v2 CREATE reuses a valid committed identity if one exists. It cannot
  rotate a new-protocol endpoint by changing its name. With no identity record,
  legacy name-derived behavior remains compatible; corrupt/I/O-failed records
  refuse instead of falling back.

The protocol is negotiated compatibility, not a hostile-client authorization
boundary. The typed application allocation token prevents accidental promotion
of a saved ID; a caller with driver access can still construct protocol bytes.

## Actual application path

The ordinary Devices/Create button now submits `AppRequest::CreateExport`.
The service validates the candidate and calls `NewEndpointIdentity::allocate`;
it retains that exact allocation in an in-memory, document-scoped Arc. Only
plain StableIds remain in graph snapshots, undo content and project files.

Lifecycle dispatch carries that Arc to `WindowsVadProvider::establish_export`.
Existing/plain IDs call `reopen_export`, never the old generic creation path.
Validated deferral and uncertain CREATE retain the original token without
allocating another ID. A fresh, accepted, error-free Present observation with
policy prepared consumes the token; transport/PCM readiness is not substituted
for policy publication. Present polling does not issue an extra CREATE just to
switch modes. A subsequent actual post-Removed rename recreation reopens the
same StableId. The provider also supports a prepared same-spec one-way
Establish-to-Reopen management transition without native I/O.

Successful project load or graph replacement revokes outstanding tokens before
clearing their map, including Arcs retained by queued work. Such work refuses
instead of silently becoming a reopen or fresh fallback. Failed validation or
project load preserves the current candidate/authority. Already-entered native
I/O is not retroactively cancelled by an atomic revocation flag.

Local UI validation preserves explanatory UTF-16/name feedback; mailbox and
service validation remain additional boundaries. Neither new intent nor fresh
allocation grants default-role eligibility or binds an Engine node implicitly.

## Verification and limits

Evidence root:
`D:\coding\moiren-evidence\20261108-112100-collector-worker`.

- `s4-identity-source-checkpoint-20261108-final.json` pins23 driver/backend files
  and five command receipts. The parent independently verifies every source
  and receipt hash before integration.
- Driver checks: all22 MSVC suites and all17 Ubuntu portable suites pass;
  WDK analyzed build and INF verification pass. These include production
  Control/Policy/ACX extraction,14 malformed identity dispatch cases with no
  storage/UUID/child side effects, exact wire normalization, persistence,
  adoption/new behavior and mixed-version reference retention.
- Backend checks:280 VAD-filtered Rust library tests pass,611 filtered;
  focused library/tests strict Clippy passes. This is not a workspace total.
- Application regression first reproduces a missing fresh-token dispatch;
  its fixed final seven managed tests cover repeated deferral, uncertainty,
  preparation/error distinction, rename reopen, loaded IDs, queued revocation,
  document replacement and rejected validation. Existing headless UI tests
  reproduce and verify preservation of the actionable invalid-name feedback.
- Final parent workspace run with both `moiren-app/gpui-test` and
  `moiren-app/vad-integration-probe` passes2,665tests,10ignored, across73test
  result groups. All-target workspace strict Clippy with both features and
  workspace formatting pass. No native audio/desktop UI is launched.
- Predecessor4573d48 CI run42449989373 finishes20/20successful jobs and the
  original watcher23584 exits0. This proves the published managed-admission
  predecessor, not a hosted CI result for this newer uncommitted identity slice.

Independent static review finds and closes mixed-v2 identity rotation and the
normal-button feedback regression. Software checks do not establish that the
currently installed613 image contains this code. New package/signature/SYS
provenance and a separately admitted target deployment remain necessary.

Mutable final Windows/MMDevice FriendlyName publication is still separate
work. A durable reference alone does not prove that Windows displays the new
name, retains default roles/settings, or behaves correctly with streaming,
held clients, sibling endpoints, races and A-to-B-to-A repetitions. No buffer,
timing, waveform, removal or physical-latency gate is relaxed.
