# Opt-in actual raw VAD playback-input reference

Date: 2026-11-02. Baseline: `41abcd9f89dc8abeb3108952a2739b8cdbedaef2`.
This advances S1 instrumentation, not native or physical qualification.

## Missing connection and bounded change

The existing library collector retains actual Playback input bytes before
decode, with owner identity and per-packet publication dispositions. The app
can use it, but the direct E2E probe did not connect it. Scalar SRC histories
and nominal source schedules cannot substitute for these actual input bytes.

The probe now accepts explicit `--vad-input-capture-frames 1..480000` for
`live-vad`, except diagnostic soak. Omission preserves existing behavior. A
selected-interface constructor validates/reserves the existing capture budget
before native connection and reuses the caller's chosen control root. No new
collector hook, MIC output witness, driver change, buffer default, scheduling
algorithm or quality-gate change is introduced.

The raw prefix is bounded by the existing collector: 480000 frames, 4 MiB and
4096 records, with whole-packet capture. After host shutdown, only the actual
worker-join guard permits export. Fresh exclusive files retain original format
bytes in `vad-input.raw.bin` and identity/format/publication/worker outcome in
`vad-input.metadata.json`. A failed joined worker can retain partial evidence;
empty evidence remains unknown. Export failure remains unavailable, even if a
raw file exists; metadata remains in the native report if file persistence
fails. Existing files are never overwritten. Input/report/manifest path aliases
are refused before artifact-directory creation.

`published` does not prove graph or MIC delivery. The cap includes pre-source
input and need not contain the whole source schedule. Driver transfer QPC is
not a sample timestamp or a recorder-clock map. Raw retention never supplies a
physical latency result, and every new evidence object remains unqualified.

## Software evidence

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-vad-input-reference-20261102-2010`.
The clean baseline archive SHA256 is
`13FEFD78EFD95A7E3125083F2C66A91BC1E5EE75D7AB038738658E726F85302C`.
`exact-export-verification.json` compares all 1358 archived files byte-for-byte:
only the intended three existing source files differ, with one new example
module. All 27 original dirty/cache files are excluded and preserved.

- Genuine preimplementation assertion RED rejects the new flag as unknown;
  seven final example regressions pass. They cover invalid budgets/modes,
  duplicates, aliases, omission, dry-run, unjoined refusal, failed partial raw
  PCM16, overwrite refusal, metadata-write failure and empty joined evidence.
- Two constructor tests cover invalid budgets before connection and both valid
  bounds, selected endpoint reservation and unchanged connection errors. The
  existing 22 input-capture tests also pass.
- Exact-final example feature-off/on suites each pass 701 tests, 1 ignored;
  the Windows audio library passes 767 tests, 4 ignored. Focused counts overlap
  these suites. Strict example/test Clippy, formatting and release build pass.
- Full clean-export Windows workspace checks include `moiren-app/gpui-test`
  and `moiren-windows-audio/vad-probe`: 2313 tests pass, 9 ignored, 0 failed
  across 71 test/doc surfaces, including 13 UI acceptance tests. All-target
  workspace check, strict Clippy and formatting exit 0. `workspace-counts.json`
  and command logs retain these results separately from the example suites.
  This is headless/local software verification, not Linux/Miri/remote CI or
  host audio execution.

Independent read-only review found no concrete blocker in selected-root
preservation, actual-join export, bounds, partial/empty classification or
export-failure reporting.

## One bounded remote attempt, before audio Start

Remote root: `D:\MoirenTest\20261102-vad-input-reference-2010`.
Exact probe: `e2e_latency_probe.exe`, SHA256
`B969CC0B9C4B23C33CB559FF1468A5224BC1285EBDD583B62F1CEBA97EF942AB`.
This is a probe, not a new application or driver package. `native-intent.json`
pins the baseline plus all four overlay hashes honestly; it is not an
unmodified-commit binary. Input scripts/configuration hashes are checked too.

The new question is whether the explicit collector persists an actual joined
input prefix through this entry point, not another stability soak. A separate
remote dry-run passes with no native bytes and normal process termination.
The one native attempt runs at 20:19:00.600-20:19:04.979 Asia/Shanghai. It requests
Compatible Playback -> unity Bus -> MIC, Engine64, a six-second indexed source,
480000-frame capture cap and a 20-second upper collection window. No smaller
target, load injection, source-mode fallback or parameter scan is added.

Both Float32 and PCM16 activate successfully, but each raw Exclusive format
query returns `-2004287480` (`0x88890008`). No exact candidate is selected. The
retained preparation identity identifies the selected Playback device; source
Start is false, submitted frames are zero, and the input worker never starts.
`vad_input_reference` correctly reports `unavailable/not_started_unknown` with
no fabricated PCM or metadata artifact. Collection/source/cleanup completion
remain false. The bounded owner exits 1 normally, without timeout or forced
termination. This is a pre-Start format refusal, not evidence of steady-state
corruption or a successful native collector test. Its cause is not established;
the earlier run's successful format query cannot explain this refusal.

Both exports reach Removed/Closed, only the bare control bus remains, and no
test audio/Python owner remains. External OBS/QQMusic PID/creation identities,
loaded driver hash and boot identity are unchanged. Twelve compact receipt/log
files, 103413 bytes total, are copied and individually length/SHA256 verified;
raw audio is not transferred because none was acquired. A waveform plot or
delay estimate would be fictitious here, so neither is produced.
The separately hash/length-verified final read-only audit at 20:24:47 confirms
the absent raw/metadata files, zero submitted frames and unchanged safety state.

No unchanged native retry follows. The first-packet issue stays deferred.
Actual collector native acceptance, actual MIC-output/recorder association,
the smaller target's earlier failure and calibrated physical measurements
remain open. Independent development is not blocked by absent physical gear;
no lower default, path winner, sub-10-ms claim or S1-S3 completion follows.
