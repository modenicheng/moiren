# Parallel remote Process2048 operational baseline - 2026-11-02

This bounded task runs while the exact Expert editor is developed. It does not
block application work or replace the unresolved S1 waveform/physical gates.
First-packet silence remains deferred.

## Contract and provenance

Remote directory: `D:\MoirenTest\20261102-s3-baseline2048-long-0238`.
Limited Interactive task: `Moiren-Baseline2048-20261102-0238`.
Process -> unity Bus -> DAWN PRO2 at48k/F32, Engine64, Shared requested/actual336,
native capacity738, explicit render target512/wake64/callback64; no conversion
or fallback. Input capacity16384/target2048/jitter96, actual max packet480,
measured gap536 and complete consumer demand704frames. The independently
admitted modeled minimum is1338=704+536+96+2, low706/high15904. Capacity and
these reserves are not physical latency.

App binary is the preceding admitted-reserve build:
`D:\MoirenTest\20261102-s3-input-reserve-0222\bin\moiren-app.exe`, SHA256
`611D172B25BBF51C49B8720277AA218C334FA5797D78FD615FB1B31DF315C138`.
Its source manifest pins HEAD8075482 plus disclosed dirty sources, source-equivalent
to ced9858; this is not a clean-commit binary. The new exact Expert editor is
verified separately and was not the binary under this long test.

Probe SHA256 `3FB5330D790767F68B0114162F137E685AE472D17717CF5C4967D8EFA81A2E28`;
loaded Running SYS SHA256
`4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.
Runner SHA256 `A447F47A67F52566FE6168DBCE4B7DCF29DB2FA584B32753007C557FCBAF3F24`.
No driver installation, host native audio or default promotion occurs.

The first separate runner attempt0232 asks for source660seconds, exceeding the
existing1..600limit. It fails before readiness, source exit2, task Ready/result1,
no owners. Inspection precedes a new task/directory; its receipt is not erased.
The corrected attempt uses source600seconds and monitor540seconds.

## Terminal results

- observed Running540001ms; monitor/source exit0; exact contract matches and
  operational completion true. Cleanup joins normally with no forced exit,
  cleanup errors, native fault or remaining owned processes. Task Ready.
- Captured/published25920480frames in54001packets. Dropped/silent/nonfinite,
  underrun/discontinuity/timestamp-error/reset, backlog/age/ingress recovery,
  low-water-guard counters are all0.
- **Seven delivery-gap-bound violations remain**, maximum112914*100ns=
  11.2914ms. Packet-bound/unavailable/regressed-clock counts0. Telemetry-clean
  qualification remains false; neither faults nor thresholds are suppressed.
- Startup prime3264/discard352 is separate. Fill min1602/max2498 is occupancy,
  not residence or physical latency. Startup investigation remains deferred.
- Original OBS12648/QQMusic15096 unchanged. No calibration, isolated waveform
  or whole-run continuity evidence is claimed; source also directly renders
  DAWN, so the resulting output mix is unsuitable for isolated latency claims.

This is not a matched A/B against the earlier1344run: delivery-bound measurement
and wall-clock conditions differ. Seven versus93violations does not establish
a buffer effect, a stable default or a latency winner.

## Evidence and visual

Host evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Length/SHA256 verified downloads:

| Artifact | Bytes | SHA256 |
| --- | ---: | --- |
| `s3-baseline2048-result-0248.json` | 15385 | `324EC0C5178D475D227EA0F8661BEF5CC9C8887EE3FFAE7807DF74042DA38FF8` |
| Remote `monitor.json` | 1591485 | `54770C75105C2B7C8E281AF86B29B3BEFB683CE2962AE988823963787B8C892B` |
| `s3-baseline2048-plot-0250.png` | 411825 | `0622FD543ADDD0088979F599422A537DF99EDB76A343B0B08658E7E3FF498BDD` |

PNG is visually reviewed. Remote `operational-telemetry-final.png`/SVG and
`plot-summary-final.json` are generated in the isolated analysis environment.
The preliminary plot incorrectly hardcodes54000packets; it remains preserved
but is superseded by the final plot using the actual counter54001. No numeric
counter result changes. No raw PCM or full report transfer is needed.

Retained histories are bounded tails:256queue events over0.3360079seconds and
256producer events over2.549415seconds, with404774/53745events overwritten.
A zero-violation tail does not erase the seven aggregate violations or become
whole-run waveform evidence. S1-S3 remain incomplete, with useful application
development proceeding independently.
