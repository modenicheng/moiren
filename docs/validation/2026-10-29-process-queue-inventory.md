# Consumer-owned Process queue inventory and desktop confirmation

Recorded 2026-10-29 (Asia/Shanghai). Implementation commits: `7c2f3a0`
(audio evidence and observation fixtures) and `343c8d5` (desktop confirmation).
Both were pushed and independently matched to the remote branch SHA.
Continues S1-S3; it does not close their physical or full software acceptance.
No native audio, VM test, deployment, driver installation or driver package
change occurred. Compatibility defaults and measured admission are unchanged.

## Why another history is necessary

Existing producer events describe completed native drains, including dropped
frames; packet publication happens within those drains. Existing consumer
events describe complete Engine demand at a pre-render boundary, not the
ClockSource's actual native-rate FIFO removals. They do not establish entry
inventory or the intra-drain publication timeline. Replaying them with an
invented starting fill would not justify an alternate operating target.

The new history records actual consumer-owned inventory instead. For each
ClockSource read boundary:

- `D`: cumulative successful ring pops and committed ring discards.
- `q`: ring occupancy acquired by that same sole consumer.
- `h`: frames held outside the ring for interpolation.
- Observed publication frontier `P = D + q`.
- Retired frontier `R = D - h`.
- Pending native input inventory `I = q + h = P - R`.

The acquired producer frontier may advance between observations. No
independently sampled producer-written counter is combined with the consumer
cursor to manufacture a queue boundary. Dropped new tails are not publications.
Output-rate demand and output frames are kept distinct from native input
retirement, including both sample-rate conversion directions.

## Runtime and diagnostic contract

Every nonzero read records before/after boundaries, requested/output frames,
separate prime/backlog/age/reset-held discards, reset count and underrun frames.
Zero-output priming remains observable; zero requests and invalid output shapes
do not emit reads. All actual ring removal paths use the counted helpers.
Retained interpolation frames are not double-counted as recovery losses.
Existing audio/servo/recovery behavior remains unchanged.

The reused optional QPC labels only the pre-read boundary. It does not
timestamp the later post-read boundary or any source sample. Missing/zero
clocks and regressions remain explicit; missing stamps do not hide regression
against the last known stamp. Clock availability does not invalidate otherwise
conserved inventory. No new per-sample clock reads are introduced.

The fixed history retains 256 events with 16 atomic scalar fields per event.
One unique writer publishes with atomic version markers and scalar payloads;
control reads make one attempt and return unknown on contention. They do not
retry, retain mutable slots or silently reuse stale success. Checked arithmetic
and boundary continuity failures preserve the last valid prefix and set a
sticky saturated flag. Overwrite counts disclose truncated history.

Preparation allocates the history off RT. Its 32,808-byte published storage
(`size_of::<PublishedHistory>()`, excluding Arc allocator bookkeeping) is added
to `ClockBridgeConfig::storage_bytes()` and therefore actual backend preparation
budgeting. Audio publication does not allocate, deallocate, log, lock or retry;
owned observer copies allocate only on the control side. Endpoint destruction
remains a control-side responsibility, like the existing audio ring.

`BridgeObserver::queue_history()` projects into both ordinary native bridge
observations and Process delivery observations. The retained native read handle
preserves exact source identity and owner generation after the live registry
clears, including final subsequent source reads. A new generation cannot
replace or mix with the old history. Reading that handle is not teardown proof.
The existing measured per-wake API does not require this diagnostic history.

## Desktop settings truthfulness

Requested and active Process rows independently describe measured selection,
source node, PID/creation time, target/capacity/jitter and originating
session/owner/revision. An active measured override is not labeled Compatible
because its ordinary transport setting is omitted.

A save captures the latest service selection, not the stale selection from
when a desktop draft opened. An unchanged transport save preserves it; any
actual transport save captures its expected clearing, matching the existing
`SetTransportLatency` contract, and displays that consequence. Desired clearing
does not manufacture an active owner change.

Save/restart confirmation matches the exact captured measured identity in
addition to audio/transport/profile settings. Running state, settled graph
revision and absence of failure remain required for restart confirmation.
A different later selection cannot falsely satisfy a pending save. This adds
no new measured-selector wizard, service API, saved proof or production preset.

## Verification

All commands ran on the development host with
`CARGO_TARGET_DIR=D:\coding\moiren\target`; none opened an audio device.
Final checks cover the committed implementation plus the final acceptance
fixture and storage-budget regression. Counts for overlapping test selections
are not additive.

| Command | Result |
| --- | --- |
| `cargo test --locked -p moiren-windows-audio --lib` | 456 passed, 2 ignored |
| `cargo test --locked -p moiren-windows-audio --test clock_bridge --test queue_inventory` | Existing bridge 31 passed; initial inventory 8 passed |
| `cargo test --locked -p moiren-windows-audio --test queue_inventory` | Final expanded inventory 9 passed |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe --features vad-probe` | 259 passed |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` | 100 passed |
| `cargo test --locked -p moiren-app --lib --features gpui-test -- --test-threads=4` | 453 passed, 3 ignored |
| `cargo test --locked -p moiren-app --target x86_64-pc-windows-msvc --test ui_acceptance --features gpui-test native_live:: -- --test-threads=1` | 3 passed, 10 filtered out |
| `cargo clippy --locked -p moiren-windows-audio --all-targets --features vad-probe -- -D warnings` | Exit 0 |
| `cargo clippy --locked -p moiren-app --all-targets --features gpui-test -- -D warnings` | Final exit 0 |
| `cargo check --locked -p moiren-app --all-targets --features gpui-test` | Exit 0 |
| `cargo check --locked --manifest-path tools/rt-check/Cargo.toml --all-targets --target x86_64-unknown-linux-gnu` | Exit 0; cross-compilation only |
| `cargo fmt --all -- --check`; `git diff --check` | Exit 0 |

Inventory regressions cover empty priming, exact frontiers, startup trimming,
held resets, both conversion directions, dropped overflow tails, underrun,
one-frame backlog recovery, age recovery, preparation storage refusal,
concurrent producer commits and zero audio-side allocation/deallocation over
300 reads. Pure history tests cover checked arithmetic, conservation,
continuity, fixed suffix/overwrites, coherent concurrent snapshots, contention,
clock diagnostics, saturation and JSON unknowns. Native tests cover retained
generation-pinned projection after registry clear.

Test-first evidence is retained rather than hidden:

- Pure queue history: 15 failures before implementation, then 15 passing;
  `D:\coding\moiren\target\queue-history-{red,green}.log`.
- Initial bridge integration compile failed because the queue module was absent;
  `s3-queue-inventory-red.log`. Expanded green logs retain the bridge and
  inventory runs; the final log includes the 8192-byte storage-budget case and
  concurrent publication test.
- Desktop model terminal sessions `8009` and `1731`: 9 passed/4 failed before
  the fix, then all 13 passed. These session outputs were not written to files.
  Session `46173` subsequently passed all 30 settings-filtered headless tests.
  The independent full app run includes those regressions.
- The first all-target app Clippy failed with E0063 on the missing
  `queue_history` field in `tests/ui_acceptance/native_live.rs`. The fixture
  was corrected; the final strict Clippy, all-target check and three affected
  rendered acceptance tests pass. The failed log remains available.

Unless explicitly absolute above, logs are in this worktree's
`target/s1-s3-20261028/s3-queue-*.log`. Independent read-only audio and desktop
reviews report no remaining Important/Critical findings. No Linux test
execution, Ubuntu CI, Miri, physical timing or sustained output is claimed.

## Open work

- Actual inventory is necessary evidence, not alternate-target finite-window
  queue-feasibility replay or proof of sustainable queue balance. Packet
  availability within a drain remains unknown.
- Process source sample age is still unknown and unsupported age enforcement
  remains refused. Pre-read QPC must not become a sample timestamp.
- VAD forward packet-sequence holes are not yet separately counted. Add typed,
  epoch-fenced missing-slot/frame diagnostics without inventing non-silent loss,
  forcing resets or turning correctness-mode jitter into fail-fast behavior.
- VAD queue timestamp provenance/oldest sample age, actual lower native-period
  negotiation, matched physical routes and stable sub-10-ms qualification stay
  OPEN or hardware-deferred. Do not retry VM timing or install on the host.

The complete S1-S3 objective stays active. No software test or relaxed
correctness-mode threshold substitutes for suitable-hardware qualification.
