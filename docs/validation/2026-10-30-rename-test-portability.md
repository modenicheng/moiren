# Portable rename diagnostic coverage

Date: 2026-10-30, Asia/Shanghai. Tests-only source: `ccd72c6`.
No production code, schema, native operation or qualification gate changes.

## Failure found after the previous repair

PR26 run `41048375599`, source `501d186`, passes the formerly failing
Linux probe413/preflight5 suites and VAD example suites6+11+37. Its next
command fails compiling the application integration example:

```text
cargo +stable test --locked -p moiren-app --features vad-integration-probe --example vad_integration
```

Denied-unused diagnostics identify eight `TopologyReadError` variants,
`SetupInterfaceEvidence::Error`, and four `Phase` variants with no portable
test constructors. Their native Windows consumers remain valid. Existing
portable tests covered only part of the diagnostic/report contract.

Rather than suppress warnings, shrink the portable schema, cfg out fixtures
or remove tests, add two useful portable regressions:

- Every11 topology error variants preserve exact JSON discriminator and all
  raw payload fields, independently and inside SetupAPI error evidence.
- Every10 rename phases retain their original failed progress and per-phase
  error after a successful retry. The first error stays primary, entries stay
  bounded and serialized phase tags remain distinct. A fully complete
  lifecycle fixture proves failed acceptance is not just missing evidence.

All existing workspace tests and production bodies stay unchanged. The new
tests are not an attempt to make a native rename qualify on Linux. Final
read-only review finds no P1/P2 issue; its suggested stronger completed-case
fixture is included.

## Fresh verification and exact limits

- Windows full integration example passes110 tests with
  `vad-integration-probe`, then110 with CI's combined
  `gpui-test,vad-integration-probe`. Strict example Clippy and workspace
  rustfmt pass on the final completed-case version.
- A direct host-to-Linux `--no-run` cannot reach the example because GTK
  dependencies lack a cross sysroot/pkg-config wrapper. Its retained exit101
  is an environment limitation, not reproduction of the enum error or GREEN.
- A deliberately isolated diagnostic harness copies actual `rename.rs`
  unchanged and extracts the verbatim production `ExportRetirementEvidence`
  DTO. With the old tests it reproduces the same three enum diagnostics;
  with the final tests it executes24 successfully under Debian nightly and
  Linux-target Miri Stacked Borrows. Both RED and GREEN are retained.
- This small harness excludes exactly one existing report-coupled test,
  `cleanup_failures_are_retained_separately_from_the_primary_failure`, because
  it requires the full application report/dependency graph. No stub report is
  fabricated. The exclusion exists only in the external harness and its
  `scope.json`; the real workspace retains this test and executes it on
  Windows. It remains included in the full Linux CI target, whose corrected
  source result is pending. Harness24 is not full application, GTK,
  production service or native Windows verification.
- The initial harness attempt failed on its missing full-report dependency;
  that failure is retained separately and is not the enum RED. The later
  scoped harness gives the actual matching diagnostic.

Evidence, logs and harness scope are retained outside Cargo output at
`D:\coding\moiren-evidence\20261030-ci-501d186`. Previous failed CI and
checkpoint receipts are not overwritten. `gate.log` remains unrelated and
unstaged. Source and documentation are committed separately and delivered
together at a verified checkpoint; the new source requires its own CI result.

## Deployment and outstanding work

No app, probe or driver redeployment is needed for this tests-only change.
The physical target retains app `c222df2`, probe `9de94e4` and the unchanged
Running driver from the [verified probe-only delivery](2026-10-30-probe-test-cfg.md).
No remote build, native playback, speaker volume/default change, reboot or
cache cleanup runs for this repair.

The post-bugcheck native retry decision remains open. Real changing PCM,
continuity/common calibrated reference and S2 smaller-budget feasibility are
not established by serialization/phase tests. Earlier S3 desktop evidence
remains separate. S1-S3 and stable sub-10ms output are still incomplete; no
source-path winner, lower default, PR merge or native rename qualification
follows from this portable test coverage.
