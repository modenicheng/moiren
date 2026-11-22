# Probe example test-build portability

Date: 2026-10-30, Asia/Shanghai. Code: `9de94e4`.
This is an offline compilation/test and probe-only deployment receipt, not
native audio or physical latency qualification. S1-S3 remain incomplete.

## Exact CI failure and repair

PR26 run `41043645184`, source `94c8efd`, passes the Linux standalone RT
harness with196 tests, then fails the default-feature example-test command:

```text
cargo +stable test --locked -p moiren-windows-audio --example e2e_latency_probe --example shared_render_preflight
```

Four helpers are compiled without Linux callers: `poll_finite`,
`observe_measurement_quality`, `validate_measurement_outcome_with_policy`
and Process `validate_measurement_outcome`. Local Linux-target
`cargo test --example e2e_latency_probe --no-run` reproduces all four denied
unused errors. The preceding `cargo check --tests` and strict all-target
Clippy had not exposed this test-codegen failure; those checks are not
substitutes for the exact example-test build.

The fix gates only these native Windows entry points and the corresponding
`Instant` import with `cfg(windows)`, matching their actual callers and native
measurement fixtures. `poll_finite_with_clock`, `Duration`, portable finite
deadline/cancellation regressions and all existing tests remain available.
No function body, native scheduling, budget, runtime policy, test assertion,
warning level or qualification gate changes. Read-only review finds no P1/P2
issue in the three-file code diff.

## Verification and disclosed limits

- Windows default-feature examples:528 probe and5 preflight tests pass.
- Actual Debian Linux execution with the existing nightly1.101 toolchain and
  reused host Cargo cache passes all413 portable probe and5 preflight tests.
  This is not the stable-toolchain CI result; the nightly and registry setup
  are disclosed rather than relabeling the earlier failed stable attempts.
- The same Debian setup then passes527 audio crate tests across32 test/doc
  binaries, VAD probe example suites6+11+37 and the independent196-test RT
  harness. No native device operation runs in these portable suites.
- Strict Windows and Linux-target audio all-target Clippy with `vad-probe`
  pass; workspace format check passes.
- Linux-target Miri discovers413 probe and5 preflight tests. Actual Tree
  Borrows execution passes both portable finite-poll tests and all5 preflight
  tests. Discovery is not execution of the other411 probe tests.
- The exact post-fix Linux cross-test command passes the original four
  compiler errors but cannot link with the host's Windows `cc`; its linker
  rejects `--eh-frame-hdr`. Exit101 is retained and is not recorded as a pass.
- The broad11-test trace Miri attempt is stopped while interpreting a costly
  16MiB serialization-cap regression. Its incomplete log and owned-process
  cancellation receipt are retained; it is not a passing suite or a skipped
  CI test. The bounded finite-poll/preflight Miri runs above are separate.
- Debian's original offline registry lacks `gpui-kit`; its online retry fails
  querying the configured registry. Reusing the existing host Cargo cache
  avoids downloading dependencies, but Debian stable1.94 is too old for the
  workspace's existing `atomic_try_update` usage. These environment failures
  are retained, not repaired by weakening source or modifying global config.

The original CI watch exits on a TLS handshake timeout, not a test verdict.
Subsequent independent API snapshots first show pending jobs; a cache-busted
19:26 query confirms the run has completed with19 successful jobs and only
the exact Ubuntu failure above. Windows and all Miri jobs pass. This is a
failed old-source run, not green or a verdict on the repair. The corrected
revision requires its own CI result.

## Physical-target offline delivery

The owned source on `LAPTOP-NP49RG6H` / `192.168.12.97` advances from
`c222df2` to exact `9de94e445086aa5d47b0c02a6c162e3bc186d6c2`
through a verified incremental Git bundle. Source is clean and no app,
probe or build process is active before work. Reusable D: caches are retained,
with jobs2, no incremental output and no audio/build overlap.

Only three release stages run, each exiting0: strict audio all-target Clippy,
both default-feature example suites528+5, and release probe build. The app
does not need another full rebuild for example-only cfg edits.

At19:18:46+08:00 only the formal probe entry point is replaced:

| Artifact | Source | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `e2e_latency_probe.exe` | `9de94e4` | 7026688 | `851954D6FCC4224C35AA6CD29D569183C22D901D8BC637956571EB89BA967D8A` |
| unchanged `moiren-app.exe` | `c222df2` | 33490432 | `60228B9F74D99B3DCFE5C379CB4F1E079A826939C743781CBE9B464B3F041EC5` |

The old probe is retained under
`D:\MoirenTest\20261029-0118a54\rollback-before-probe-cfg-9de94e4`,
hash `295E2C4CC99F234C1C5ABC907D7B98B2AFE832BF0F575779FF308D1417187F0C`.
The19:18:47 independent audit verifies installed/rollback identities, the
unchanged app, clean source, no build/audio process, driver Running and root
PnP OK. Installed SYS hash remains
`CCC0EC4FE4DDB51FB22B90A483E58FF3874D87FA8634D62C93BB8AA52FFFE471`.
There is no driver reinstall, native playback, cache cleanup, requested
reboot or volume/default-role/system-setting write.

`probe-cfg-9de94e4.zip` is12920 bytes with17 manifest files plus the manifest,
SHA-256 `3912A58855063985C2665C995C28BD089719E8A9FF0E285D27BEEB857DC37991`.
The independent host verifier checks every entry/size/hash, exact source and
three-stage results, both test counts, unchanged app provenance and deployed
probe identity. D free254124208128 bytes; reusable Cargo outputs4006225772
bytes remain. The remote disk is not routinely cleaned after each build.

Evidence lives outside Cargo at
`D:\coding\moiren-evidence\20261030-ci-final-94c8efd`, including original
failure logs, all incomplete attempts and the independently verified target
archive. The earlier native-helper backup is unchanged. Unrelated `gate.log`
is neither staged nor changed. Source and receipt use local atomic commits;
the verified integration checkpoint is pushed together, not per small edit.

Native audio retries remain paused after the earlier bugcheck. This repair
neither establishes waveform continuity/common calibration nor S2 native
lower-budget feasibility. S3's earlier production desktop Apply/replacement
evidence remains distinct. No stable sub-10ms output, source-path winner,
lower default, PR merge or overall S1-S3 completion is claimed.
