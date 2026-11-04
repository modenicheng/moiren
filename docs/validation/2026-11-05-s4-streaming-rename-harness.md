# S4 streaming rename harness - bounded design and validation

## Approved scope and source pin

- Worktree: `D:\coding\moiren\.worktrees\s4-streaming-rename`.
- Branch: `test/vad-streaming-rename-20261105`.
- Base: `fa47ffcd19a22d4b81d70cec69168fa3fac269a8`.
- This design is recorded before implementation. The user authorized the
  original S4 goal/complete pipeline; the parent selected this first increment.
  This is not direct user approval of a new exact design. No staging, commit,
  push, native audio/UI, installation or VM work.
- Only the VAD integration example/modules and this receipt are owned. The
  legacy `--rename`, its `Case` acceptance gates and category worktree stay intact.

## Contract

Add a distinct opt-in `--exercise --rename-streaming` mode, exclusive with legacy
rename, PCM, soak and input-capture modes. Run one rename per direction on the
existing persisted two-endpoint route. Preserve project endpoint IDs, graph
bindings and exact raw `PKEY_Device_FriendlyName` acceptance, not a normalized or
composite-name substitute. This is not immutable Windows MMDevice identity.

For each direction: Apply and confirm the old route; initialize and actually
Start the exact-owned exclusive native Pair; verify the existing strict known
signal gate; edit only the target name while Pair remains RUN and pumping;
retain interruption/native invalidation independently of PCM qualification;
release both old clients on every path; require direct old Removed evidence;
read the replacement Windows name through a short-lived read-only MMDevice
owner (no exclusive client initialization), drop it before Apply; Apply/confirm
the replacement revision and generations; initialize/Start a fresh exact-owned
Pair with a distinct signal and verify matching PCM from this replacement;
release every owner.

Fresh post-rename PCM must be tied to the replacement native identity, newer
service observation and confirmed revision. Old signal totals cannot satisfy a
post gate. The existing 48,000 consecutive-frame gate, tolerance, settling and
phase limits are reused without relaxation. A recreation interruption is not a
continuous-soak pass or a zero-interruption promise. Partial Start success and
Stop failures survive in the report; cleanup attempts and release are explicit.
Failures are sticky even if bounded evidence collection or later cleanup works.
Diagnostic completion is separate from acceptance; missing/failed direct
Removed, exact name or PCM never permits a streaming rename pass. Saved failed
or unknown observations are not replaced by later successes.

## Small implementation shape

- A focused portable streaming evidence/type module with regression tests.
- A bounded Windows coordinator reusing native Pair, actual MMDevice readback,
  request/snapshot evidence and the existing rename lifecycle predicates.
- Small Pair property-observation/start/cleanup adaptations; no second exclusive
  client for names, extra control owner, IOCTL or diagnostic framework.
- New optional report field, explicit parser/help and dispatch. Legacy result
  schemas remain unchanged when this mode is absent.

## Validation plan

Test-first RED/GREEN covers missing Start/PCM, stale post-replacement evidence,
partial owner release and sticky failures. Run only portable example unit tests,
host compile/lint/format checks, serial Cargo (`-j 1`) with
`CARGO_TARGET_DIR=D:\coding\moiren-evidence\cargo-target-s4-streaming`.
Record actual command results and source hashes below after execution. Host
tests/builds do not prove Windows visible-name behavior or native audio success.

## Explicitly unexercised

The full race/repetition/sibling-isolation/default-role matrix, user applications,
driver migration, physical latency and sustained qualification are not included.
The first-packet issue remains deferred. No calibrated equipment is required to
implement or host-test this harness. Native execution remains remote-only after
separate authorization and available target/VM; none is performed here.

## Primary references

- https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-start
- https://learn.microsoft.com/en-us/windows/win32/coreaudio/recovering-from-an-invalid-device-error
- https://learn.microsoft.com/en-us/windows/win32/coreaudio/pkey-device-friendlyname

## Results

Initial RED: explicit parser test failed because `--rename-streaming` was absent.
The four separate evidence regressions then failed on missing Start/PCM, stale
post identity/revision, skipped partial-Start Stop, and diagnostic completion
erasing a failure. A callback RED separately demonstrated the missing both-Start
guard on the actual sample-observation helper. A retirement-observation RED
demonstrated loss of the first unknown observation. Logs are retained under
`D:\coding\moiren-evidence\20261105-s4-streaming`.

Initial GREEN: 122 tests, then 125 tests after callback/report/cleanup regressions.
First Clippy run found only a collapsible `if` in the new bounded wait helper;
fixed without suppressing the lint. Final verification and pins follow below.
No native/name/physical success claim.

## Integration verification

The parent imports all 12 frozen files without modification on integration
base `3c2c094`. Manifest SHA-256:
`BBC8940C2B0101BBAFEADFA7400B3C5EB632928B8F1E3B1940FAFF94A9E1D37B`.
An independent read-only review finds no actionable correctness issue; legacy
strict predicates and PCM validation remain unchanged. The coordinator uses
the same tested transition helper, retaining the first actual observation.

Fresh integration checks on 2026-11-05:

- Explicit `vad-integration-probe` example tests: 127 passed, 0 failed.
- Non-test example build, strict example Clippy and workspace fmt: exit 0.
- Workspace tests with `moiren-app/gpui-test`: 2609 passed, 10 ignored, 0 failed
  across 73 harnesses, completing at 01:44:35.
- Workspace all-target check and strict all-target Clippy with
  `moiren-app/vad-integration-probe`: exit 0, completing at 01:45:04.

The first parent example invocation omits the required opt-in feature and is
refused before compilation; the corrected commands and both outcomes are
retained. The optional local Linux example check first lacks cached crates,
then reaches missing `glib-2.0` development libraries. That WSL environment
does not have the Linux desktop dependencies installed by CI; no source
workaround, suppression or Linux pass is claimed. Exact-head remote CI remains
required after publication. No host desktop or native audio is launched.

The final transition RED showed that candidate `rename_pending` alone must not
release a still-running old Pair. The coordinator now calls the same tested
`Case::observe_transition(runtime, edit)` helper, not a duplicate predicate.
It retains and publishes the first actual observation, and releases only after
the edit is consumed and gate closure/resource withdrawal/native identity change
is observed. The first unknown retirement observation and last actual retirement
result are also retained independently of acceptance and sticky failures.

### Final host verification (2026-11-05)

All Cargo commands used the dedicated target directory above and `-j 1`;
none ran the example binary, native clients, an audio/UI application or a driver.
Toolchain: rustc 1.99.0 `b940084d7`, Cargo 1.99.0 `5f94df478`,
`x86_64-pc-windows-msvc`. Final execution session: `50181`, completed exit 0.

| Command | Result | Evidence log |
| --- | --- | --- |
| `cargo test --locked -j 1 -p moiren-app --features vad-integration-probe --example vad_integration` | 127 passed, 0 failed; 7.47s test execution | `final-tests.log` |
| `cargo build --locked -j 1 -p moiren-app --features vad-integration-probe --example vad_integration` | exit 0; 8.00s | `final-build.log` |
| `cargo clippy --locked -j 1 -p moiren-app --features vad-integration-probe --example vad_integration -- -D warnings` | exit 0; 1.51s | `final-clippy.log` |
| `rustfmt --edition 2024 --check crates/moiren-app/examples/vad_integration.rs` | exit 0, including child modules | `final-rustfmt.log` |
| `git diff --check` | exit 0 | `final-diff-check.log` |

All logs are in `D:\coding\moiren-evidence\20261105-s4-streaming`.
RED logs: `red-options.log`, `red-stream.log` (4 failing behavioral tests),
`red-callback.log`, `red-retirement.log`, `red-transition.log`.
These are genuine assertion failures before the corresponding fixes, not
compiler failures. Nine new focused streaming tests plus the explicit-mode
parser regression run alongside all existing example tests. Legacy acceptance
tests remain unchanged. No Linux/native/remote run was performed by this worker;
the parent's newer integration results are not this candidate's evidence.

### Ownership and limits

- `native::Pair` exclusively owns render/capture clients on the existing worker
  COM apartment. The App service remains the sole driver control owner.
- Name observations use local read-only MMDevice/property-store references and
  never Activate/Initialize an audio client. They return owned text and release
  COM references before Apply; fresh Pair initialization follows Engine confirm.
- PCM callbacks copy already leased samples into the existing strict Observation
  counters. They run only after both successful Starts. Before/fresh counters,
  known patterns, revisions and native admissions are distinct. Callback errors
  are published before native cleanup. Fixed 1s finite-only settling and 12s PCM
  limits are unchanged; native Start re-admission uses existing GateProgress 5s
  behavior, rebuilt per phase, never across a replacement PDO.
- Old pump invalidation records HRESULT/message and ends pumping that invalid
  client; it is not new PCM success. Other pump failures remain failures. Both
  Stops are attempted; Stop errors, including invalidation, remain cleanup
  failures even after all native references are dropped.
- Direct old Removed uses the original helper and its existing bounded wait.
  Missing direct evidence or exact Windows name does not prevent bounded later
  name/PCM diagnostic collection, but always prevents `completed`/overall pass.
- Strict raw name mismatch is not normalized. Stable project binding is checked;
  replacement MMDevice textual ID may change. No category/name-hash/driver code,
  policy, ABI or legacy held-client semantics changed.
- This remains a first increment, not full S4 qualification: no races/repeated
  rename/independent sibling/default-role acceptance, native success, calibrated
  physical latency, first-packet qualification or sustained-run claim.

### Frozen source manifest

Base/HEAD remains `fa47ffcd19a22d4b81d70cec69168fa3fac269a8`; no commits or index
changes. Parent-reported `e59d7d9` was not merged/rebased or otherwise modified.
Paths below are relative to this worktree; SHA-256 is over exact file bytes.
The receipt's own hash is in the external `frozen-manifest.json` to avoid a
self-referential hash. All 12 changed/new paths, binary pin and final logs are
listed there. App source is frozen after final verification; this receipt is
frozen once that external manifest is written.

| Path | SHA-256 |
| --- | --- |
| `crates/moiren-app/examples/vad_integration.rs` | `705198eb18d73294fd3fa6279b62ac06897fd5eee692a4595d71cfe56f5d153a` |
| `crates/moiren-app/examples/vad_integration/native.rs` | `9b4a290e71d2900e5769a9fc1ef5dc61916dbb44600d25350a0fe7df2e3bda13` |
| `crates/moiren-app/examples/vad_integration/native/rename_stream.rs` | `8845fb08ccc3aaf1d22a2b6801bdc6c19ef89c401b5512681506de99842cc6cc` |
| `crates/moiren-app/examples/vad_integration/options.rs` | `d911acb97a941895fdd0f63974745df17eac220aaffbb91154ef00b119854067` |
| `crates/moiren-app/examples/vad_integration/rename.rs` | `f6fe962e0c670e6bfbe878d647c3c1f411351397f7bf44851d02e6e2d134cb28` |
| `crates/moiren-app/examples/vad_integration/rename/target.rs` | `4f9feb3feaae71348970f0ecdcd73cbb9a2c39db0f89d99d22a7c48a80cf4888` |
| `crates/moiren-app/examples/vad_integration/rename/stream.rs` | `d53df40475d0e896cf5108304932bf96d7acec3d119ed7302c43851ce0f1097e` |
| `crates/moiren-app/examples/vad_integration/rename/stream/target.rs` | `f409c4eb2cc937d30427c3ea9b9bbf8157e0210f8d7cde427893c4e536c1eefb` |
| `crates/moiren-app/examples/vad_integration/rename/stream/tests.rs` | `1e91a07886850c721be206f572bb79df61513741ec7c5ab4e776b1f14def6762` |
| `crates/moiren-app/examples/vad_integration/report.rs` | `45f07178df165ca59222158402f4abac7a7467c947fe0502579f4095b1574d9f` |
| `crates/moiren-app/examples/vad_integration/target.rs` | `30b0bf21675db494d489c78bc320d139ec6e7e345aceb121e0109a6761884c00` |

Host-build-only artifact (never executed):
`D:\coding\moiren-evidence\cargo-target-s4-streaming\debug\examples\vad_integration.exe`,
17,672,192 bytes, SHA-256
`dc9e95b9ebf7c0f4f687ad1ea32482fc9699cb2884dfc3db520e306273037853`.
