# Dedicated OBS reuse helper, without per-run executable copies

Date: 2026-11-08, Asia/Shanghai. This follows the completed
[30-copy cleanup](2026-11-08-reused-obs-cleanup.md). It implements the reuse
boundary for future acquisition harnesses, not another waveform acceptance run.

## Fixed-path API

`scripts/obs-reuse.py` defaults to the existing dedicated installation:

```text
D:\MoirenTest\20261101-clean-comb-probe-1320\obs-portable
```

The only command-line action is `--check`: read-only target/SID, executable
hash, existing OBS owner and websocket-port4471 checks. It cannot launch OBS,
record, install or choose an arbitrary new executable path. Default API calls
also enforce target/owner admission before touching the configuration. Root
and idle-check injection exist for file-only tests, not as a native admission
bypass for deployed runners.

A future target acquisition uses `ReusedObs` with its existing bounded
`case.launch(exe, args, name, cwd)` process-owner implementation:

1. `prepare(case.root)` retains the live exclusive byte-range lease, verifies
   the executable, archives the small existing configuration, and writes the
   dedicated Isolated profile/collection. Recording storage must be a new
   per-run directory outside OBS. No EXE/DLL/plugin tree is copied or moved.
2. `launch(case.launch)` rechecks the executable and absence of existing OBS
   owners/listeners, then permits one launch at the fixed path. The returned
   process owner is retained. An exception/invalid owner consumes that attempt
   and preserves uncertainty; it cannot authorize a retry or successful finish.
3. The acquisition harness performs authenticated websocket readiness, exact
   input configuration and its recording protocol. Reuse the existing graceful
   close order: confirm inactive, close owned websocket, request normal window
   close, join the exact owned process. This helper does not substitute for
   those steps or terminate any process.
4. `finish()` refuses a live/unknown owner, archives resulting config/logs and
   retains the actual OBS exit code. It releases the lease only after this
   step. A nonzero exit is retained as failure, never qualification.

The lease follows the live kernel handle, not a PID/receipt age or timeout.
An orphan OBS after a harness crash is still caught by the next owner query;
there is no stale-lock breaking or automatic termination. This boundary
serializes cooperating test harnesses; it does not claim to stop an external
application from racing an independent launch.

Historical completed off-repo runners remain frozen evidence and still contain
their old `copytree` code. Do not rerun them unchanged. This helper has not yet
been wired into a new native waveform acquisition: that next runnable harness
must import/use it explicitly rather than importing the obsolete setup code.
No native test is authorized merely because these helper tests pass.

## Bounds and regression evidence

Configuration archives allow at most1024regular files/16MiB,2048total entries,
32directory levels and512relative-path characters. Directory links are checked
before descent. Reparse/symlink/hardlinked paths, application/recording files,
noncanonical `..` paths, reused evidence and existing recording directories
are refused. Output JSON is capped independently at1MiB. Original ordinary OBS
configuration outside the dedicated tree is never a write target.

Evidence root:
`D:\coding\moiren-evidence\20261108-004446-reusable-obs`.

- Initial16tests fail for the missing helper, then pass after implementation.
- Independent review's traversal, uncertain-launch retry and lock-initialization
  defects reproduce before fixes. A two-byte lock file no longer poisons the
  lease: byte0 is locked directly, even past EOF, without initialization writes.
- A real second Python process holds the lease until its actual exit. This
  test launches no OBS, native audio, UI or driver operation.
- A hardlinked configuration reproduces an external-settings write risk before
  the path check is extended. The fixed case preserves external bytes.
- All26focused file/mocked-boundary tests pass on Windows. The nine related
  offline suites pass179tests in total; original29draft/cache pins are unchanged.
  The suite is added to the existing
  offline CI job; no Linux or native OBS acceptance is inferred.
- The production-extracted `interface_name_native` host suite still passes
  after the separate comment-only naming-contract correction. It does not
  establish identity preservation or accepted native naming.

At09:35 the final helper SHA256
`46853DBAAC524C78B72162B6CDB400B4E841A91235C5BBCA44AD9DA04FD9F5DF`
passes `--check` on the actual separate target using its existing pinned Python.
No OBS is launched; no configuration/security/driver changes or executable
copies occur. The earlier read-only receipt uses an older helper and is not
evidence for this final version. The target copy is named `obs-reuse-final.py`
in the existing small cleanup workspace; it is not another OBS application.

## Reboot/freshness remains separate

During work the observed local/remote time advances from00:56 to09:29. The
reported boot timestamp drifts from09:50:55 to09:50:57 on the same October30
date. An initial exact-string comparison incorrectly labels that change a
reboot; a separate correction receipt explicitly withdraws that flag. Bounded
uptime readback shows approximately776million milliseconds, not a new boot.
No native operation follows the incorrect comparison.

A performance-CIM uptime query times out; a separate bounded OS/tick-count
readback supplies the correction. A timestamp string difference is not an
authorization to proceed. Re-establish freshness/protected owners after a real
user reboot; do not reuse stale native admission after clock changes.613still
awaits the user's reboot and subsequent driver-provenance check.

PR26 is currently Open/Draft at935f196, with merge state DIRTY and no reported
status checks for that head at09:32. Do not claim green CI or merge the whole
branch on the strength of local helper tests.
