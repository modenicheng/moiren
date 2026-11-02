# Exact Shared periods through the S1 probe entry points

Date: 2026-11-02, Asia/Shanghai. Baseline: `6be7470`.
This fixes a demonstrated probe configuration mismatch, not production audio,
native admission, a latency default, or a waveform/physical qualification gate.

## Why this is an actual S1 gap

The application and production WASAPI backend already accept exact Shared
period requests within `moiren_engine::audio_config::SHARED_PERIOD_RANGE`
(64..4096), separately from power-of-two Engine presets. Shared336 has current
native application receipts. Nevertheless `e2e_latency_probe live-vad` and
`live-process-loopback` validate physical output through the older preset-only
`PhysicalRenderRequest`; `live-physical` also has a preset-only pre-parser.
The [exact336 VAD recording](2026-11-02-vad-dawn-exact336-functional.md)
therefore had to use desktop automation rather than this direct probe entry.

The original S1 requests common explicit output mode/period across paths.
The mismatch prevents expressing an already-supported setting and adds an
unnecessary UIA dependency. It is not evidence that336 is unsupported or that
another native acquisition is needed just to reproduce parser refusal.

## Repair and retained limits

- `render_request.rs`: explicit Shared and omitted mode (default Shared) accept
  the existing production64..4096range, including65/144/336/441/480/4095.
- `live.rs`: Process/VAD physical-output requests retain the exact period in
  manifest/report and preserve omitted-mode null versus explicit Shared intent.
- `physical.rs`: independent capture480/render336 Shared requests pass through
  parse, exact IO/Bus topology and dry-run artifacts without conflating directions.
- Exclusive remains power-of-two presets. Only the preexisting explicit
  `vad-probe` Exclusive-render144 exception in the Process/VAD paths survives;
  `live-physical` capture and render retain their previous stricter rejection.
- No native-observation validator changes. Explicit Shared requires exact
  requested/actual period, Client3 and no fallback/conversion/initialization
  error. Native device capability and observed period can still refuse a request.
  Exclusive still requires FullCapacity rendering. The virtual MIC route still
  rejects physical-render controls.

Production backend, driver, application settings, input target/capacity,
recovery, scheduling, DSP, and Compatible defaults are unchanged. This restores
probe request expressiveness; it does not certify any arbitrary integer period.

## Test-first evidence

Clean source comes from `git archive 6be7470`; only the three intended example
modules are overlaid. The original18dirty paths and9cache files are excluded.
Two actual RED invocations are retained:

1. Direct Shared exact-period range regression fails at non-preset65 with the
   old preset-only validation, not a compile or dependency error.
2. Actual Process/VAD dry-run336 and physical capture480/render336 roundtrip
   tests both fail before artifact creation with the old validation messages.

After repair, an intermediate full run catches a Windows-invalid quote in the
new test's debug-formatted temp-directory label. Only the test label changes;
that680pass/1failure receipt stays retained. Final default and `vad-probe`
example suites each pass683tests,1ignored. Tests cover range edges, None versus
Shared, exact topology/serialization, unchanged Exclusive336 refusal,
feature-on/off144 behavior, explicit physical-direction144 rejection,
Shared336 exact native readback, missing observations and fallback refusal.
Strict example/test-profile Clippy, workspace formatting and release build pass.
The separate `cargo test --locked -p moiren-windows-audio` library/integration
regression exits0:1074passed,5ignored,0failures. Its complete log is retained
alongside the example suites; all checks use the same clean source export.

Local evidence:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-probe-exact-shared-20261102-1805`.
Initial/final logs are immutable separate files, not replaced by green results.

## Remote deployment and finite dry-run

Deployment:
`D:\MoirenTest\20261102-probe-exact-shared-1805\e2e_latency_probe.exe`.
SHA256:
`59C12E05880386F2DE952F984A408B192B4D17EA5AFAE8E1C9B18CA0A2DFE336`.
This executable is built from the clean baseline plus the pinned three-file
patch, not the worktree's unrelated dirty probe/analyzer experiments.

One remote wrapper runs18:11:50-18:11:52, with a20s deadline per held process:

| Case | Exit | Meaning |
| --- | ---: | --- |
| `live-vad --help` | 0 | Updated exact Shared subcommand documentation |
| VAD Shared336 `--dry-run` | 1 | Exact requested336 retained; intentionally unqualified |
| VAD omitted-mode336 `--dry-run` | 1 | Default Shared semantics with mode intent still null |
| VAD Exclusive336 `--dry-run` | 2 | Invalid request; no artifact directory |
| Physical capture480/render336 `--dry-run` | 1 | Separate exact requests, no native observation |

No native audio, Moiren application, device, source, driver operation, OBS or
QQMusic control is launched. The process-only help/dry-run checks are not
native device acceptance or a soak. Protected process creation identities,
running SYS and binary hash are checked unchanged; the18:13:05 final audit
finds no remaining analysis/native owner. All22 compact artifacts are downloaded
and verified by length/SHA256. ZIP15597bytes, SHA256
`97D63A17F00F209D36D2184C75CF53CC876DC8DAEC1D1CF9E983550C7514869D`.

The top-level `cli.rs` help still prints its historical preset list. That file
is part of the original preserved dirty edits and is deliberately not rewritten
or staged. Use `live-vad --help` or `live-process-loopback --help` for the
updated source-route help; `live-physical --help` documents both directions.
No claim is made that the top-level help text was updated.

## Next-action boundary

Read-only requirement audits find no missing original S2 or S3 baseline
functionality in the committed production source. Their native evidence remains
conditional: old945/cap1 has a running MIC completion fault after startup and
dirty-build provenance, while current Process records retain delivery-gap
violations/render timeout. Neither first-packet deferral nor the latest FIR
waveform agreement erases those facts.

This change removes a real S1 execution obstacle without another audio run,
parameter scan, new render journal or weaker gate. Subsequent native work may
request the same accepted Shared336 directly through the probe when it answers
a new question. It need not repeat existing captures just to get a green task.
Physical delay, matched-path quality and lower-setting qualification remain open;
the full S1-S3 goal stays active. Original27dirty/cache hashes/status remain
preserved; local commit only, no push or memory update.
