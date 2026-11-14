# Warm-control Process waveform diagnostic - 2026-11-02

## Scope and acquisition

Improve the previous isolated OBS acquisition while application development
and headless verification continue independently. This is not a lower-target
scan or a physical latency test. First-packet investigation remains deferred.

Remote base: `D:\MoirenTest\20261102-s3-warm-control-0337`.
Task `Moiren-S3WarmControl-20261102-0337`: Limited Interactive, isolated owned
portable OBS/configuration, six-minute scheduler limit, task lifecycle.
Application SHA256
`49987B9FE05F0E187C61F6FD6296A4AE32B575941F5D6D7E67DEED3B5F364D9C`
is the preceding budget-diagnostics binary, not the new clock-diagnostics build.

Route: Python stereo Float32/48k WASAPI to separate Voicemeeter In2
`{0.0.0.00000000}.{06eb2c6c-bab1-48b8-a0c9-af825233813d}`; Moiren captures
that process tree -> unity Bus -> DAWN PRO2
`{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}`. Owned OBS records
Python Process reference and DAWN output as separate lossless PCM24 stereo48k
tracks, monitoring off, offsets0 and gains1. User OBS is untouched. Native
Shared request/actual period336, buffer738, render target512/wake64/callback64,
Engine64; input target2048/capacity16384/jitter96. Ten-second measurement admits
packet480/gap533 and full modeled minimum1335; it does not promise later gaps.

Before Moiren starts, record a distinct six-second preroll pilot, two seconds
silence, and an entire fixed four-second bandlimited changing control. Pilot
submission is not recorder-ready evidence. The complete recorded control must
still meet the unchanged exact-reference NCC0.99 and competitor1.8 gates;
there is no selected interior subsection or relaxed threshold. Main source
retains the prior32-second noise/sweep/silence schedule.

### Operational failures are retained

Initial separate attempt0332 stops before opening the player/Moiren: websocket
Identify succeeds before OBS frontend readiness; GetVersion returns207. Owned
OBS requires forced cleanup. Original external apps and device controls stay
unchanged; no audio-route or waveform result exists for that attempt. Fresh
attempt0337 adds a finite30-second retry for readiness207 only; one such retry
occurs. Neither receipt is overwritten.

Attempt0337 saves a closed MKV and the entire1536000-frame main source with
zero reported source underflows. However, after StopRecord the harness's
80-second wait expires around the requested90-second monitor lifetime. Its
original capture receipt remains complete=false, task Ready/result1. Cleanup
observes monitor/player/owned OBS all exiting0 normally without force and no
remaining owned processes. The final monitor report independently records
90001ms running, run_complete=true, cleanup_complete=true, failure=null. These
do not rewrite the harness failure as a successful acquisition. Future runner
waits need a declared lifetime/deadline budget, not an unchanged80-second wait
after extending the run. No immediate third audio acquisition is attempted.

Final Process counters:10618packets/10617gap observations, maximum gap12.1036ms,
five gap-bound violations; packet-bound violations0. Dropped/silent/underrun,
discontinuity/timestamp-error, resets and backlog/age recoveries remain0;
startup prime discard352 is separate. These counters do not certify waveform
integrity. Source maximum blocking write217.6216ms over its lifetime is also
retained rather than hidden by zero underflow reports. OBS logs initial window
lookup retry and reference invalidation/reinitialization, and one lagged video
frame; none establishes a waveform root cause. Original OBS12648/QQMusic15096,
default roles and device master controls remain unchanged.

## Offline analysis and visualization

Analysis stays beside the remote closed recording, with existing fixed gates;
only compact reports/plots are downloaded. The guarded initial analysis refuses
the incomplete acquisition. A separate diagnostic-only entry explicitly retains
the failed capture context and extracts the final monitor JSON without editing
the original capture receipt. It may analyze PCM, not declare qualification.

Both tracks decode to3283968stereo frames. Exact main noise/reference mapping
is NCC1.0 at1790622frames. But the full control/reference candidate is only
NCC0.352135, competitor ratio1.12817; control correspondence and isolation are
refused. The reference level plot contains holes during pilot/control; apparent
quietness in the return is not proof of isolation and no cause is assigned.

Five of28 main windows pass the unchanged raw/highpass/stereo support gates;
their uncalibrated OBS relative offsets span95.7917..95.8750ms,
p50=95.8542ms,p95=p99=95.8750ms. Those selected offsets are not a whole-run
distribution, not qualified isolated latency, and not physical DAC/ADC latency.
They cannot be compared to the earlier47ms subset to infer a path regression:
control, recorder clock/offset and whole-waveform support are unresolved.

The four-panel `analysis-raw/overview.png` retains full levels, log control
levels, refused correlation candidates and selected offsets. It is downloaded
and visually reviewed. A held-out linear-clock comparison then exceeds its
180-second subprocess limit without producing a result; it is not called a
failed model fit or a passed test. Final audit03:40:16+08:00 confirms no remaining
analysis or native test processes. No model thresholds are changed or partial
windows cherry-picked. The12 isolated and13 linear-model regression tests pass
on host and target; that does not substitute for this timed-out actual fit.

## Artifacts and boundaries

Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Raw media remains remote under `records\2026-11-02 03-32-44.mkv`; decoded PCM
remains in `analysis-raw`. Pin receipt:
`s2-clock-s3-warm-final-audit-0408.stdout.log`. Verified host downloads:

| Artifact | Bytes | SHA256 |
| --- | ---: | --- |
| `s3-warm-result-0401.json` | 9836 | `0398B17B20F43EFC93EF4F6C624339E3B2B6A2C9F5AF20B510E6117E195A893C` |
| `s3-warm-monitor-0401.json` | 1631308 | `0ACFE42C933065560C9E03A71F1E856F28BA84E6CE87D860024400315ABCF3E3` |
| `s3-warm-raw-summary-0401.json` | 53673 | `CF4C3196E04838D4F491D780BA9B7ABF01039FA2C2C92664ADAB4AAE2BA58D6D` |
| `s3-warm-raw-plot-0401.png` | 354799 | `2DAEE0D21BFE8692380443A84B8A1E3CFCE41CBAB511ADE51248EF0C43A53B28` |

This is retained diagnostic evidence with failed acquisition/control gates,
not a low-latency PASS. No lower defaults, scheduling/kernel changes, enlarged
buffers, driver installation, physical qualification or path winner follows.
Missing physical calibration does not block continued S1-S3 implementation;
the user explicitly permits independent bounded remote tests during development
and does not request another stability-testing loop.
