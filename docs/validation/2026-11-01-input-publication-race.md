# Raw input capture publication identity race

## Scope and reproduced defect

Base `c039303` already checks publication identity on both sides of playback
decode for aggregate telemetry. The optional raw input capture records the
packet before decode, but previously labelled every `Ok(true)` as `published`
without a corresponding post-write owner check. Revocation during that write
therefore produced a `published` packet beside correctly ambiguous aggregate
telemetry. This is a diagnostic attribution defect, not evidence of a new
audio drop or a source/driver/sample-clock origin.

The frozen hypothesis, source hashes, alternatives and exact command receipts
are under `D:\coding\moiren-evidence\20261101-input-publication-race`.
`race-red-corrected` executes the actual production pump with the existing
post-write revocation hook and fails specifically on `published` versus
`published_identity_changed`. The earlier `race-red` command mistakenly used
`--exact` with an unqualified filter and ran zero tests; it is retained, not
counted as a reproduction. `export` retains the missing explicit feature error;
the corrected command enables `vad-integration-probe`.

## Narrow repair

The single-shot capture's actual owner identity is checked immediately before
and after the existing decode. It uses the same reset, admission, ticket and
revision predicate as its existing pre-decode admission, independently of the
aggregate telemetry history, which can already have been invalidated by an
earlier native open. No new native query or clock is read. Checks are skipped
when the current packet has no retained capture record, including disabled,
sealed and capacity-limited capture.

An actual successful write with an observed identity change retains its raw
bytes, original owner and real decode result, but records
`published_identity_changed`. An open capture seals with
`identity_changed_during_publication`; an earlier capacity/shape seal remains
unchanged. Packet-level attribution must therefore be inspected even when the
terminal reason is `frame_capacity`. A sealed capture never reopens when later
mutable admission fields happen to match again. `carrier_full` and
`decode_error` retain their own precedence and original audio behavior.

This is two-sided observation, not a transactional owner lock or proof of all
unobserved intermediate state changes. `published` does not prove Engine
consumption, native delivery, source sample age, recorder correspondence,
waveform continuity or physical delay. Historical artifacts from before this
fix are not retrospectively upgraded. Public capture DTO fields and exporter
schema1 stay unchanged; the diagnostic string preserves the new ambiguous
outcome. Export remains create-new and `qualified=false`.

The PCM worker remains non-RT. No RT boundary, DSP/servo, carrier policy,
operating target, native period, capacity, default or quality gate changes.
No push, subagent, native acquisition/replay, remote audio, speaker rendering,
driver/system-control write, reboot or memory update occurs. Unrelated
`gate.log` remains excluded with its original221 bytes and SHA-256.

## Regression surfaces

- Production pump checks seven changes (revoke, reset, cancel, drain,
  deactivate, same-revision ticket replacement and new-revision replacement),
  before/after write, with open and exactly-full captures:28 combinations.
- Each case preserves actual changing stereo samples and successful decode
  counters while recording the ambiguity; the original owner is never relabelled.
- Enabled/disabled capture pairs check the same14 race combinations for exact
  carrier samples, transfer result, IO collection/submission, remaining tickets,
  reset epoch and deterministic telemetry equality.
- Subsequent PCM continues through the real carrier but cannot extend a sealed
  artifact, even after admission restoration. Full/malformed failures are not
  converted into successful publication; invalid aggregate history does not
  invalidate a current independent capture owner.
- Allocation accounting checks the actual enabled pump and the raced seal
  transition. The test-only boxed race hooks are outside that allocation
  assertion because consuming them frees test instrumentation. The existing
  4096-record/capacity assertion remains in place.
- The actual app exporter preserves raw bytes, original identity, the raced
  packet and an earlier capacity seal, refuses overwrite and remains unqualified.

## Fresh verification

- `cargo test --locked -p moiren-windows-audio --all-features`:1012 passed,
  5 ignored. Focused input-capture suite:17 passed, including7 new regressions.
- Audio probe examples with all features:762 passed,1 ignored.
- App all-feature tests (including headless UI acceptance):673 passed,4 ignored.
  Explicit `vad-integration-probe` example:117 passed, including the new export
  regression. No ignored hardware exercise is enabled.
- Standalone production RT shadow harness:229 passed.
- Independent Python archive/phase/cascade suites:25/16/16 passed.
- Strict audio/app all-target/all-feature Clippy, app all-target/all-feature
  check, rustfmt and whitespace checks pass.
- Linux audio and RT shadow all-target checks pass **compile-only**. No Linux
  execution, Miri, remote CI or full-workspace result is claimed.

Only the first focused `--exact` filter mistake and the explicit-feature error
precede their corrected receipts; all failed logs remain. A combined generated
pyc cleanup command is policy-rejected without execution, then exact literal
file/directory cleanup succeeds. Source review is local and single-agent;
no independent-review claim follows. The final manifest independently checks
command/log byte counts and hashes, exact changed paths, committed source pins,
unchanged DSP/servo/RT policy surfaces and the unrelated `gate.log` hash.

This closes only the reproduced host-verifiable diagnostic race. Original
S1-S3 acceptance still requires the missing native changing-waveform/common
reference and calibrated physical evidence; no latency floor or path winner
is established here.
