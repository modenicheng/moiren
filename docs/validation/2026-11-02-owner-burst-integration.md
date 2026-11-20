# Virtual owner whole-burst integration

Scope: deterministic, headless software coverage of the existing S2 policy.
No production behavior, buffer default, driver, native audio or recovery policy
changes are introduced by this slice.

## Coverage

`virtual_owner::owner::tests::burst` executes the actual
`run_with_recovery_cap` loop with real compiled stereo
`VirtualPlaybackSource -> unity Bus -> VirtualMicrophoneSink` bindings.

- Engine 48 kHz / 64 frames; independently admitted recovery caps 1, 2 and 8.
- Hand-derived native targets 257, 321 and 706 frames respectively, retaining
  the declared packet128 / gap128 / jitter32 / FIR32 admission terms.
- The same target is used with carrier capacities4096 and8192. Complete output
  PCM, remaining input content and clock counters must match across capacities.
- A test-only 128-frame producer/consumer packet is ordered immediately before
  the repayment burst or after it, with the latter at a half-Engine-period wake
  phase. This is an isolated prefilled-carrier burst, **not** an ongoing native
  packet-cadence or delivery-gap simulation.
- Initial32-frame source muting is completed/classified separately. Every
  callback reports a complete transfer with zero xruns/discontinuities and
  unchanged SRC reset generations; short reads and recovery muting remain zero.
- An isolated wake three periods beyond each cap must omit exactly192 Engine
  frames, publish one late burst, repay exactly the configured cap and keep both
  boundaries unreset. This asserts the existing bounded-loss policy, not lossless
  recovery beyond the cap or the withdrawn fail-fast behavior.
- Separate FIR instances replay the known submitted input and recorded SRC
  steps, comparing every captured sample, including the burst-only segment.
  This catches graph ordering/gain/silence errors; it does not replace the
  independent numerical SRC oracle in `boundary/phase_tests`.

The fixture performs allocation only as test infrastructure. It is neither a
native PCM pump nor evidence of RT allocation safety; existing guards cover that
separate contract.

## Verification

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.

- The first fixture run incorrectly treated classified startup mute as zero
  soft events. Both tests failed with32 versus0; the assertion was corrected to
  require exactly32 initial-muted frames, no recovery mute and no short read.
  No production fix or genuine defect is inferred from this fixture error.
- Two mutation checks in an exported HEAD tree plus these tests both fail:
  a zero-gain send fails the burst-only non-silence assertion; gain0.5 fails
  sample equality. The development worktree was not mutated for these checks.
  Logs: `s2-burst-zero-gain-red-0500.log` and
  `s2-burst-wrong-gain-red-0503.log`.
- Final `cargo test --locked --workspace`:2101 passed,8 ignored,67 test targets.
  Log: `s2-burst-workspace-0504.log`.
- Final `cargo test --locked -p moiren-app --features gpui-test measured_ui_tests`:
  20 passed. Log: `s2-burst-ui-0504.log`.
- Strict workspace all-target Clippy and fmt pass. An intermediate Clippy run
  rejected two constant-size `chunks_exact` uses; both now use `as_chunks`.
  Logs: `s2-burst-clippy-0504.log`, `s2-burst-fmt-0519.log`.
- `s2-burst-verification-0519.json` pins logs and verifies all18 preexisting dirty
  files byte-identical and excluded. No push or memory update is performed.

This adds combination coverage; it does not establish a new production bug,
qualify native timing, promote smaller defaults or complete S1-S3.
