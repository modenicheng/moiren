# Direct remote category candidate deployment

Recorded November 7, 2026, Asia/Shanghai. The separate standalone target is
used directly under the user's amendment. Neither VMware startup nor desktop
unlock is a prerequisite for this SSH/CLI work. This is not host installation,
native naming acceptance, physical latency, or full S0-S5 completion.

## Provenance and preserved rollback

Source category candidate: `c321409e5b53cd38d4f2efb5ecdaba07a48ac6e5`.
Integration HEAD before this work: `c9a4cc8`; the tracked worktree was clean.
Only `driver/vad/STATUS.md` differs in the driver subtree from the candidate
source. The November 5 integration unsigned SYS/INF/CAT lengths and hashes
are rechecked before and after signing; their original bytes are unchanged.
All 29 original-worktree draft/cache hashes also match the preserved pins.

Evidence namespace:
`D:\coding\moiren-evidence\20261107-direct-remote-driver-1755`.
Remote namespace:
`D:\MoirenTest\20261107-direct-remote-driver-1755`.

The existing `3.1.1.611`/`oem205.inf` DriverStore package is copied, not moved
or removed, to a fresh owned `rollback-611` directory. All three files are
verified before/after copying, transferred to the host, and independently
verified again. The preserved package's original source is `48ecf08`, with
PlaybackCopyDiagnostic=1. Its manifest SHA-256 is:

```text
B51E217855A4DFAB4EB75E8C80F03342BD1E0A837E2B4A3381F3F089311C2D1B
```

Rollback readiness here means retained exact files, existing target trust and
successful signature/catalog-member verification. It does not mean an actual
rollback was exercised. No automatic rollback, removal or retry is performed.

## Signed candidate copy

The already analyzed unsigned integration package is copied to
`package-c321409-612`. Only the copy's INF DriverVer is stamped
`11/07/2026,3.1.1.612` to distinguish/rank it after the installed 611 package.
This is package metadata, not a new driver source revision or ABI version.
The candidate is the default PlaybackCopyDiagnostic=0 build; it is not the
opt-in 611 diagnostic build and does not borrow that build's diagnostic schema.

InfVerif passes. The SYS is signed through the existing host certificate store,
then Inf2Cat generates an x64 catalog for `10_NI_X64,10_GE_X64` and the CAT is
signed. No private key is exported, no trust store is changed and no security
policy is relaxed. Host signature verification retains its existing
UntrustedRoot failure rather than importing trust. Target verification uses
the already-present exact Root/TrustedPublisher certificate:
`7F17CCDCA257D6ABC84255DD0D1854E93B9B5C1D`.

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| moirenvad.cat | 2514 | `AD5090790C6B359A6D169A7B3206FF3EA6F0CE026507087E80F29A1BEF3A83BA` |
| MoirenVad.inf | 1799 | `BB526217B87462C8A986A62FC3348959C83E28CE97862C47D496D2F4CF84624D` |
| MoirenVad.sys | 147352 | `B25CA2E8DB8DD3E98D9B90AA6CF19822C879E6CA13D52B35D160CEF2056D1737` |

Frozen candidate manifest SHA-256:

```text
94883DAC472B67A3344D22D7DA10B355ECD6565C87BCD022DFC10D56D1C44EAF
```

## Deployment safety repairs and verification

The earlier finite deployer is adapted to explicit 611 -> 612 identities,
the new manifest, and the preserved rollback. It retains held process handles,
finite helper deadlines, explicit non-null zero exits, unhealthy/third-binding
refusal, no reboot on 3010, and no automatic retry/rollback/package deletion.
Only the exact `ROOT\MEDIA\0001` root is restarted; no global device restart
or Windows restart is requested.

New admission/pre-restart/final observations compare all six default roles,
all twelve existing render endpoint volume/mute controls, the original QQMusic
15096 and independently owned PowerShell27444 creation/session identities,
boot time, SecureBoot, DeviceGuard and raw BCD output SHA-256. No external
process is manipulated. Matching before/after identities do not prove
uninterrupted external playback between snapshots.

Two actual guard defects are caught before installation:

- PowerShell 5.1 `Select-Object` does not project dictionary keys in these
  fixtures, allowing null comparisons. Typed comparison rows fix the real
  helper; role/volume/mute mutation tests fail before and pass after the fix.
- Independent read-only review identifies omitted catalog snapshot errors.
  The real `Read-SafetySnapshot`, with only system/tool boundaries substituted,
  reproduces six false acceptances. The corrected helper requires explicit
  empty error arrays at report, snapshot and every endpoint; missing, null,
  scalar and nonempty errors refuse. All nine real-reader checks then pass.

Final deployer `deploy-category-v2.ps1` SHA-256:

```text
708FC702B5E273319A4719355CD830F12B7D93AE21E5602F568A914E10AA2FFA
```

Each of PowerShell 5.1 and 7.6.6 passes 124 actual AST helper/state-machine
checks, 16 snapshot-comparison checks and nine real-reader checks, 149 total.
The final source/pins/results receive independent read-only review with no
remaining load-bearing finding. Earlier failures and the superseded first
preflight remain separate, not overwritten or retroactively upgraded.

The corrected runner's actual target `ValidateOnly` run at18:14:38..46 passes
all eight candidate/rollback signature/member commands, verifies the old611
binding unchanged, and records no install/restart attempt or audit error.

## Actual installation

At18:17:04..16, the reviewed runner executes exactly one
`pnputil /add-driver <candidate-inf> /install` and one
`pnputil /restart-device ROOT\MEDIA\0001`; both helpers exit0 with pinned
handles. The post-install binding already identifies612 before the explicit
device restart. Final observations verify:

- `oem206.inf`, DriverVersion `3.1.1.612`, Running, PnP OK/problem0.
- Candidate installed INF hash and service-file SYS hash exactly match the
  signed package. Service path is
  `C:\WINDOWS\system32\DriverStore\FileRepository\moirenvad.inf_amd64_61749bf36d3be9d1\MoirenVad.sys`.
- Boot remains `2026-10-30T09:50:55.3826800+08:00`; no reboot is requested.
- All guarded defaults/render controls/protected identities/security observations
  remain unchanged. No present Moiren child or test/audio/build owner remains.
- The rollback triple remains verified. No package is removed, no external
  client is stopped, and no native stream or desktop interaction is launched
  by this deployment.

`deploy.json` records Verified=true, no primary error and no audit errors.
This establishes installed package/service-file and healthy PnP binding after
an explicit device restart, not direct attestation of the kernel's in-memory
image. Native functional and exact-name acceptance must still run separately.

## First native functional observations

The frozen source-`8761d07` optimized `vad_integration` example is uploaded
with its original7819264-byte/SHA-256 pin. An evidence-only worker loads only
the reviewed deployer's hash-verified AST helper definitions, never its
assignment/installation entrypoint. Both PowerShell versions pass the actual
invocation function's16 tests. A review catches a test-only self-caught
negative assertion; it is corrected before native execution and the real
nonzero/null/unknown-exit refusal checks pass. No product gate is weakened.

- Stage1 uses30s overall watchdog and passes: restored project, Apply with no
  probe native clients, Stop/exit and service join/control release. All external
  state/driver audits pass. This fixture has Gain(.5), so it is functional
  integration evidence, not the minimum IO/Bus physical-latency route.
- Streaming runs once at18:21:03..32. Its final report is failed after21168ms,
  before any native Start or rename edit. Playback name enumeration expires;
  the production ALL-state name observation still reports the old composed
  Speakers name, while the harness admits only ACTIVE devices. Microphone
  obtains a name, but Pair requires both endpoints and also cannot initialize.
  This does not establish two independent endpoint defects, cache causality,
  or an error in the loaded memory image. Join/control release succeed and
  the report has no cleanup_error; all safety audits and no-owner/no-child
  checks pass. No PCM, removal or streaming-rename success is claimed.

The terminal deployment/Stage1/streaming artifacts are archived without raw
audio. All135 archive entries have separately verified remote/local lengths
and SHA-256, with no remaining test owner or present child. The129084-byte ZIP
SHA-256 is:

```text
3E26B02FBA097AF8CBEB19D6859164C396C6AD68A902800CDCDF58B3C2F1D1A7
```

The first archiver is refused before ZIP creation because PowerShell5 lacks
the separately loaded Compression assembly; the corrected second invocation
loads it explicitly. It does not rerun any native experiment.

## Fresh explicit fixture diagnostic

Rather than repeat the failed streaming acquisition unchanged, the existing
legacy held-client mode is run once with explicit seed202611071825 and90s
overall watchdog, in a different fresh output directory. This changes baseline
names and stable IDs for a separately identified diagnostic; both renames
within it would still retain their own project identity. No new mode or
round-trip modifier is implemented and no random IDs are introduced.

At18:25:25..36, Playback's exclusive held client actually initializes. The
production name observation reads:
`Moiren integration playback seed 202611071825 (69- Moiren virtual audio endpoint)`.
This is not Speakers, but its adapter suffix still fails the original exact
requested-name requirement. The changed fixture alone cannot isolate whether
old cache/identity/state caused the previous acquisition failure.

Before the rename request, Stop reports a sticky worker failure:
`Unsupported("VAD endpoint has no ready, running native packet stream")`.
The baseline/held sequence therefore fails; Microphone cannot continue past
the same service failure. No rename, direct old Removed, replacement client,
Start or PCM is accepted. Service join/control release return, but the report
cleanup_error remains nonempty because the public exit evidence does not
confirm requested removal/closed gates. It is a failed lifecycle, not graceful
cleanup acceptance. The wrapper's later driver/idle/external-state audits pass.

Source inspection identifies a concrete early-owner defect: the driver legally
publishes initial Paused with PacketFrames=0 before packet allocation, while
the playback worker immediately calls pcm_shape and turns NotReady into a
sticky failure. The native receipt lacks the exact fault-time shape, so this
is a source-proven defect consistent with the observation, not an assertion
that it is the unique native cause. A focused regression/fix follows separately;
Running-zero-shape, old transport, protocol/IO and cleanup faults must stay
fatal, and neither name nor native acceptance is inferred from software tests.

The seeded run's17 terminal artifacts are independently archived and all
remote/local/archive-entry lengths and hashes match. The30954-byte ZIP SHA is:

```text
5C996909F07C869EF637CA458DCAEEEEF9C3F8DF4C5B978CE3C2651E4D6B015F
```

## Playback preparation repair

The worker's existing playback preparation condition is extracted unchanged
into `transport::playback_ready`, with the actual run_playback caller using it.
A new regression fails on the old decision after successful compilation:
legal first Paused/zero-packet observations wrongly authorize owner creation.
The earlier missing test import is retained as a scaffold compilation error,
not counted as the behavioral RED.

The fix defers only the first transport's Paused/zero-frame allocation state
while reset/producer/consumer epochs are all zero. The ordinary bounded poll
and subsequent real QUERY remain responsible for fresh native shape; no retry
deadline is extended, gate opened, silent PCM fabricated or error reclassified.
A valid Paused shape still takes the existing prerun preparation path.
Running-zero-shape, a previously started transport and nonzero recovery epochs
still reach the original strict shape validation. Cancellation, actual QUERY/
protocol/PCM errors and sticky gate/drain/cleanup failures are unchanged.

The final focused suite includes both format-present and real Acquire's
format-absent zero-packet state, valid Paused shape, Running-zero-shape,
transport-started and all three nonzero epoch cases. It proves the actual
preparation decision, not an independently observed native READ. Source
review finds no blocking issue; native cause/acceptance still require the
corrected release to be tested separately.

Software checks: VAD270 passed; full Windows-audio library876 passed,5ignored;
final startup suite9 passed; strict all-target Windows-audio Clippy exit0.
After the final test strengthening, the serial verification chain completes
at18:40:31 with all three exits0:

- `cargo test --locked -j 1 --workspace --features
  moiren-app/gpui-test,moiren-app/vad-integration-probe`; Windows-audio again
  reports876 passed,5ignored. The full workspace includes headless UI tests.
- The explicit opt-in `vad_integration` example suite reports145 passed.
- Strict workspace/all-target Clippy with both opt-in features passes.

The chain shares the external target directory
`D:\coding\moiren-evidence\cargo-target-minimal-latency`; full command logs and
terminal timestamps are in `workspace-tests.log`,
`integration-example-tests.log`, `workspace-clippy.log` and
`source-verification.json`. Fresh formatting and diff-whitespace checks also
pass. Independent final source review finds no blocking issue. These are host
software checks; a separately pinned corrected release and remote run are
still required for native lifecycle evidence.
