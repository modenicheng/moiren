# Current clean-build measured Process CLI acceptance

Date: 2026-11-02, Asia/Shanghai. Application source: clean `78967f4`.
One finite native run verifies fresh measured admission, exact1536-frame target
activation through owner replacement, and normal joined exit. This is not
desktop acceptance, waveform continuity, sustained quality or physical latency
qualification. `completed=true` is operational; `qualified=false` stays explicit.

## Current-build result

Runner interval:12:51:16.637-12:51:39.601 on `LAPTOP-NP49RG6H`. The finite
production CLI runs through the same `ManagedDriver`, using SSH session0 with
an elevated token, **not** the Limited Interactive desktop token. No window,
recorder, OBS operation, driver install or host-native audio is started.

| Acceptance item | Observed result |
| --- | --- |
| Fresh Compatible baseline |10000ms, app generation1/native generation1 |
| Measured request | target1536/capacity16384/jitter96; exact source PID23088/creation134380686794291279 |
| Fresh delivery envelope | packet480/gap548frames at48kHz, Empirical |
| Replacement | app generation1 cleanup/reap before generation2 starts |
| Exact active request |1536/16384/96; source identity and selection1 confirmed |
| Post-replacement finite run |8002ms; no Compatible substitution |
| Final join | run/cleanup complete; joined fault and cleanup error null |
| Process completion | app/source exit0, no forced termination/cleanup errors |

Native generation1 appears in both app generations: it is local to each backend
instance, not a globally unique replacement identifier. The generation1 cleanup
finishes at12:51:30.270, reaps at12:51:30.275; app generation2 starts at
12:51:30.440 and reaps at12:51:38.453. Matching the app generation, active request,
source identity and lifecycle logs establishes replacement; native1 alone cannot.

## Exact route and admitted configuration

The source renders to existing Voicemeeter In2
`{0.0.0.00000000}.{06eb2c6c-bab1-48b8-a0c9-af825233813d}` with native
Shared/Float32/stereo/48kHz storage1056frames. The graph is exactly Process input
-> unity Bus -> DAWN PRO2
`{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}`.

Engine48kHz/F32/block64 and independent Shared output request336 are confirmed.
Client3 actual period336/capacity738, no fallback or Windows auto-conversion;
render target512/wake64/callback64. Actual Process prime/servo1536,
capacity16384, low706/high15904. The modeled minimum1350 includes whole-wake
demand704, observed gap548, jitter96 and two input frames. This is preparation
arithmetic, not physical residence/latency or proof of a minimum configuration.
1024/512/256 do not fit this envelope and are not launched. No bound, gate,
capacity, recovery policy or default is relaxed to obtain the result.

## Telemetry and limits

Final **replacement-owner** counters retain800packets,384000captured/published
frames, maximum gap108781*100ns =10.8781ms, zero packet/gap-bound violations,
dropped/silent/nonfinite/underrun/discontinuity/timestamp/reset counters and no
backlog/age/ingress recovery or low-water guard. These counters do not cover the
whole preceding baseline. Oldest sample age remains Unknown and all reported
Process device positions are0, not hardware clocks.

Render writes385232frames, with dropped/silent counters0 but **one native wait
timeout retained**. The final counter does not locate that event within steady
running versus teardown; it is not erased or reclassified as harmless. No
quality-qualified result follows. Initial priming2880 and prime-discard384frames
remain separately reported; first-packet investigation stays deferred.

The deterministic ten-second source submits all480000frames/nine indexed-v2
codes and stays alive until after the monitor joins. It continues intentional
silence after its schedule, so most of the replacement-owner interval is not
exposed to changing content. Submission is not playback/capture proof. There is
no PCM recording, acquired identity/continuity result or delay distribution.
This limited configuration/lifecycle test does not repair or supersede the
earlier Process18/19 and VAD17/19 waveform refusals or historical native faults.

The four-panel `run-plot-operational-overview.png` shows only the retained queue
tail0.3360621s and producer tail2.5496046s, cumulative replacement counters and
the explicit qualification boundary. Queue history retains256/6020events with
5764overwritten; producer history256/800 with544overwritten. These clean tails
are not a whole-run trace or an observed-window replay acceptance. The measured
request's `observed_window` remains null.

## Interactive startup controls remain separate

The preceding [desktop attempts](2026-11-02-current-desktop-lifecycle-attempt.md)
remain failed. A bounded read-only event query finds no matching PS engine-start
event for the1222supervisor in its sampled interval; that absence is not a
root-cause proof. A single25-second no-audio PS5 EncodedCommand task also ends
Ready/result267014 without an entry receipt. A separate25-second WScript GUI
control writes its entry file but likewise ends at the cap rather than exit0.
Both leave no child/native owners and unchanged protected identities. They do
not prove that `-WindowStyle Hidden` caused the problem: older successful tasks
used it too. No further interactive/audio retry follows.

The direct SSH path can start current-app `monitor --help`, exit0. That first
help check is not APPDATA-isolated and writes the app's ordinary log-initializing
line; it opens no audio. The actual native run uses isolated APPDATA/TEMP/project
state and retains its session/token difference rather than calling it desktop
acceptance. One oversized encoded-command submission fails before the first
no-audio control executes; transferring the script permits its single execution.

## Reproducibility, checks and closure

```text
App: D:\MoirenTest\20261102-desktop-lifecycle-1222\bin\moiren-app.exe
SHA256 4C96B3BB7284AC3721616863FE0D83BC13798925E00E85F996BBE5A30E9D51C4
Probe: D:\MoirenTest\20261102-matched-graph-1020\bin\e2e_latency_probe.exe
SHA256 AB03BE0D3FA9C6230B31E70E33C19EF535382A214E2CF48CEB5370038F1B1685
Running SYS SHA256
4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73
Runner SHA256
E0F346C9F8CC9D63554995E6B5DFB848CD2FD1F1165B79F30F120E39BB384861
```

The native runner executes once from `D:\MoirenTest` through SSH and writes
`D:\MoirenTest\20261102-current-cli-1252\run`. It verifies binary hashes and
source identity, preserves exact child handles before fallible receipt/pipe
setup, and does not let a failed cleanup-stage write prevent teardown. Review
finds those two cleanup risks before execution; both are corrected and a
no-audio held-child fault injection proves reclamation after both injected
failures. PowerShell5 parsing and deployed hash checks pass.

Fresh clean-export `cargo test --locked -p moiren-app --lib
cli::monitor::measured::tests -j1 -- --test-threads=1` passes13tests. It does not
rerun the workspace or establish native performance. The app's preceding clean
release build and measured-UI/cleanup tests are linked in the desktop receipt.

Only after native exit, remote isolated Python `-I -B` renders the closed-report
plot and exits0 normally, using prefix-checked matplotlib, one numerical thread
and an output-local plotting cache. The original report hash is unchanged.
The PNG is downloaded, length/hash verified and visually reviewed. The1.81MB
full monitor report remains remote; an initial overbroad readback unnecessarily
transfers5.14MB of serialized histories, then compact extraction is used. Its
first extractor labels a missing `producer_history`; this derived-only error is
corrected to the actual `packet_history` before final artifact verification,
without a native rerun or any modification of original result/report/log files.

Host evidence:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-current-cli-20261102-1252`.
`result.json`, `compact.json`, manifests, source final receipt, commands, fresh
test log, plot and closure audit retain the result. All selected downloads are
verified by exact bytes/SHA256; original raw files are pinned separately.

The12:55:56 final read-only audit confirms no owned native/analysis/runner
leftovers; all four earlier interactive tasks are terminal. OBS12648 and
QQMusic15096 retain their exact creation identities. Defaults, render volumes/
mutes, current executable and Running SYS hash are unchanged. All eighteen
preexisting dirty files remain hash-equal; both Python cache directories are
preserved. No push or memory update occurs.

This supplies a current clean-build measured CLI activation/join result, not
overall S1-S3 completion. Current desktop acceptance, intact changing waveform,
native quality/lower-setting qualification and physical calibration stay open.
Do not repeat unchanged soaks or promote defaults from this finite receipt;
independent product development may continue while useful isolated remote tests
run alongside it.
