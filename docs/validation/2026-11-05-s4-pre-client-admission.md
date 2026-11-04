# S4 Separates Pre-Client Admission From PCM Confirmation

Date: 2026-11-05, Asia/Shanghai. Source base: `af98bc5`.
This repairs the existing streaming-rename probe, not the driver ABI or
production transport policy.

## Reproduced Contract Mismatch

The production provider already admits a prepared idle endpoint before any
native client exists. Its stamp has a valid PDO, session, endpoint generation
and gate epoch, while stream generation can still be zero. The existing
`prepared_idle_endpoint_can_be_admitted_without_a_native_client` regression in
`provider/lifecycle/tests.rs` covers that state. The protocol rejects zero
stream generation for non-Closed native streams, not every prepared endpoint.

The streaming coordinator checks `replacement(old, admission)` before the new
`Pair::prepare` and Start. Its shared `owner_running` predicate instead requires
all five stamp components to be nonzero. Therefore it rejects a legal newly
recreated endpoint before the probe can create its first client.

This is a source-contract defect reproduced through portable helpers, not a
native audio failure observed on the target.

## Correction And Preserved Completion Gates

- Admission requires the same clean Engine-confirmed owner, revision, binding,
  observation, nonzero identity components and gate epoch. It permits stream
  generation zero before the first native client; `running` still requires an
  open gate.
- Replacement permits that new pre-client state. Its old completed PCM owner
  must have a nonzero stream generation, a different PDO and an older endpoint
  generation in the same session.
- After actual PCM accumulation, `end_ready` returns pending for current
  generation zero. Success still needs a nonzero current stream generation,
  two successful Starts, the original strict 48,000-frame signal window and
  both open gates.
- `require_complete` permits historical pre-client admission, but requires
  nonzero confirmed stream generation and preserves identity, monotonic stamp,
  binding, revision and clean-release checks.

No freshness, phase deadline, re-admission bound, PCM quality gate, direct
Removed requirement or final watchdog is relaxed.

## Failure-First Evidence And Verification

Evidence directory:
`D:\coding\moiren-evidence\20261105-s4-idle-admission`.

Three added regressions first produce 135 passing tests and three assertion
failures, with no compilation failure. They exercise legal pre-client running
and replacement, final/old generation-zero refusal, invalid identity/epoch/
gate refusal, and real `observe_pcm` accumulation followed by pending zero
generation and successful nonzero confirmation.

| Check | Result |
| --- | --- |
| Windows opt-in integration example | 138 passed, zero failed |
| Linux stable Rust 1.99.1 opt-in integration example | 117 passed, zero failed |
| Existing production idle-admission lifecycle regression | 1 passed |
| Windows app all-target strict Clippy | Exit 0 |
| Linux stable example strict Clippy | Exit 0 |
| Windows workspace all-target check with probe feature | Exit 0 |
| Windows non-test integration example build | Exit 0 |
| Full workspace/headless, `gpui-test,vad-integration-probe` | 2625 passed, 10 ignored, zero failed, 73 harnesses; completed 03:48:36 |
| Workspace fmt and Git whitespace checks | Exit 0 |

`fix-manifest.json` and `fix-overlay.tar` pin the four code/test files.
`verification-final-pins.json` confirms all four still match the tested source
and all 29 original feature-worktree draft/cache files remain unchanged.
The explicit example totals are separate from the workspace total.

Independent read-only review verifies the actual pre-`Pair::prepare` call
order, shared completion helper and final confirmation gates. It finds no
remaining actionable blocker and makes no edits.

## CI And Qualification Limits

The existing published `919d10f` CI `41896388334` is still live at this
checkpoint: 18 jobs have succeeded, with the engine host Tree Borrows Miri
suite still running. It does not cover this later local repair. The existing
watcher is reused; no restart, manual dispatch or superseding push occurs.

No native client, desktop UI, OBS, endpoint or driver operation runs here.
The VAD release probe is subsequently rebuilt successfully from clean repair
commit `ee250640958635b13c69f3b16170cf56a8637057`, tree
`34ef39d478b7d730573e499ed454eeba2f35a3a4`, at 03:52:11. It is 7,806,976
bytes, SHA-256
`0D715ECBC4A086D8A022F2819EEC8B7ED712BD7FF5FCE6463F2B4AE349CAC52C`.
The immutable evidence copy is
`D:\coding\moiren-evidence\20261105-s4-idle-admission\vad_integration-ee25064.exe`.
Its length/hash match the build output. `release-vad-build-result.json` and
`release-vad-pins.json` retain the exact command, source and copy verification.
The executable is not run or deployed.

S1 matched-route/full-waveform correspondence, S2 sustained native budgets,
S3 ordinary desktop retained Evaluate/Apply, S4 actual Windows naming and
lifecycle matrix, and S5 sustained calibrated physical qualification remain
open. This correction proves neither sub-10-ms success nor device-limited
impossibility, and does not approve a lower default or a VAD/Process winner.
