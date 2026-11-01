# VAD gate close: revoke endpoint-local admission before native close

Date: 2026-11-01, Asia/Shanghai. Parent source:9f1a262 on
`feat/minimal-io-latency`. This is a narrow S2 epoch/ownership contract repair,
not attribution or resolution of the live desktop0x8007051A fault.

## Demonstrated defect

`Lifecycle::gate(..., false, ...)` validates the cached and fresh endpoint
stamps, then closes the native gate. Before this repair it never revoked the
endpoint's local session admission on success. The shared revision/ticket
could remain current, so `SessionState::admitted()` still returned true after
the native gate closed. On failure revocation occurred only after native IO
returned. `Lifecycle::poll()` had the same ordering gap when a negative
preparation status forced gate closure while a session was still admitted.

Deterministic regression receipts in
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`:

- `gate-close-red-155346.log`:three tests execute, two fail. The injected native
  close observes an admitted local session; another test observes admission
  surviving successful close. Stale-close refusal already passes.
- `gate-close-green-155447.log`:all three pass after explicit-close revocation.
- `gate-close-lifecycle-155453.log`:38 lifecycle tests pass at that intermediate
  source; it is not the final-source integration result.
- `gate-close-poll-red-155549.log`:the preparation-fault polling test fails
  because the native close still sees an admitted session.
- `gate-close-final-focused-155719.log`:all four final focused tests pass,
  including preparation-fault polling and explicit-close success/failure.

The tests exercise actual lifecycle code using the existing injectable
`EndpointControl`. A fake close observes the shared `SessionState` before
native completion; no sleeps or probabilistic thread scheduling are needed.

## Final-source software verification

- `cargo test --locked --workspace`:2065 passed,8 ignored, zero failures across
  67 result summaries, including doc tests. Audio-library subset:733 passed,
  4 ignored. Receipt: `gate-close-workspace-test-155727.log`,221921 bytes,
  SHA256 `8D27D01E95FCB086CE3ECCBF06AB022F1C5AE941DB567C1230C667B6B9AB2E73`.
- `cargo test --locked -p moiren-app --features gpui-test measured_ui_tests`:
  15 actual headless UI tests pass; filtered zero-test binaries are not counted.
  Receipt: `gate-close-headless-ui-160109.log`.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`:passed;
  `gate-close-workspace-clippy-160218.log`.
- `cargo fmt --all -- --check`:passed; `gate-close-fmt-160230.log` is empty
  because the successful formatter emits no text, with command exit0 retained
  by the terminal execution receipt.
- `cargo build --locked --release -p moiren-app`:passed in51.72s;
  `gate-close-release-build-160341.log`. Built executable:33873920 bytes,
  SHA256 `E6FD312E74BD00B44C72B5499AF07A4CE6FDF5CD3E3D2C3AB0AE8E7D208FC102`.

These are software checks on the final Rust changes, not native audio tests.
No smaller VAD/Process target or default is promoted by them.

## Repair and preserved boundaries

- Revoke every session belonging to the affected endpoint before dispatching
  a validated explicit native close, or a close forced by lifecycle polling.
- Keep failed native close errors and endpoint tracking unchanged. Revocation
  is not cancellation, a drain acknowledgment, a recovery or an epoch reset.
- Do not change the shared revision/ticket authority: a sibling session stays
  admitted. Unrelated endpoints must not be revoked by this endpoint's close.
- Reject stale close dispatches before local revocation or native gate IO.
  Ordinary observation cannot restore permission; reopening requires fresh
  explicit admission with the current endpoint stamp.
- Do not change PCM pump logic, queues, defaults, cancellation classification,
  same-file fault observation, peer-drain obligations or RT processing.

This removes demonstrated stale local authority. It does not serialize native
PCM and SET_GATE, fence a worker already inside a pump, or prove that a request
cannot race a later native epoch change. In particular, the old desktop INFO
log does not show whether an explicit close, native pause/rebase, release, or
some other transition caused its fault. No claim is made that this repair
eliminates0x8007051A or makes a failed stream safe to resume silently.

## Native status

At15:54:16.345+08:00 the original remote desktop log remains10875 bytes with
SHA256 `3319787822687DBC037DB84D98AEE5B4CAC43CC2EF3CBF6A3B5CB1D73EA2D142`.
Original Moiren PID13120 and OBS PID12648 retain their creation identities and
executable paths. There is no later recovery entry. Receipt:
`remote-gate-audit-state-20261101-155403.stdout.log`.

The existing diagnostic launcher/binary remains staged separately. It has not
been launched over the original GUI and does not contain this new repair.
Driver, boot, user profiles, defaults, volume and OBS configuration are not
changed. New VAD waveform/continuity validation and fault-time diagnostic
reproduction still require controlled, non-competing ownership handover.
Physical calibration remains deferred; Process-only OBS path separation is not
a VAD or calibrated physical result. S1-S3 remain incomplete.
