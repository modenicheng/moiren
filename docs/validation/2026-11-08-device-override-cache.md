# Device override inspector: local cache dependencies

Date: 2026-11-08, Asia/Shanghai. This is an independent S3 software repair
after the [main integration](2026-11-08-main-settings-integration.md), not
interactive target acceptance or an audio/latency result.

## Reproduction and repair

The Devices inspector uses retained panel paint. Its key previously omitted
the device audio draft and its nested Shared-period text inputs. A local
Override could change the draft without painting its editor until another
snapshot, scroll or whole-window refresh happened. Tests using convenience
click helpers could mask that dependency because those helpers refresh the
entire window.

The new frozen-snapshot regression changes the local override, checks that
the period editor paints, emits invalid input and requires visible refusal,
then discards the draft and requires the editor to disappear. The original
service snapshot must remain pointer-identical. Before the repair this test
fails on the first missing editor; after it, both it and a caret/focus-only
nested-notification test pass.

The inspector key now projects the device draft, transaction busy state and
current input entity/value only when it renders an input/output device
inspector. Global drafts do not invalidate unrelated graph inspectors. A
shared device-period input iterator supplies the key and the inspector's
nested subscriptions, including notifications whose text is unchanged.

## Verification

Evidence root:
`D:\coding\moiren-evidence\20261108-094031-pipeline-next`.

- `inspector-cache-red.log`: real frozen-override test fails before the key
  repair; the separate notification test already passes in that snapshot.
- `inspector-cache-green.log`: both focused tests pass.
- `cargo test --locked -p moiren-app --features gpui-test --lib ui::window::`:
  250passed,0failed,1ignored (`inspector-window-suite.log`).
- `cargo clippy --locked -p moiren-app --all-targets --features gpui-test,vad-integration-probe -- -D warnings`:
  exit0 (`inspector-clippy.log`).

These checks run headless against the non-audio test service. No visible host
UI, native driver/audio, remote OBS or target reboot is started. Successful
ordinary target desktop interaction and continuous native waveform acceptance
remain required; the original S0-S5 goal remains open.
