# Integrated 5f9 deployment - 2026-10-31

## Scope and provenance

Revision `5f9eb02526b691ecc988a0c939a32d90c35492a7` integrates main's PR52
Exclusive capture polling fix. The experimental WASAPI position/QPC/evidence
path remains, with main's Shared zero-frame and out-of-order regressions.
This deployment does not change defaults, declared budgets or quality gates.
Playback / virtual speaker remains Moiren Input / Source; MIC remains
Moiren Output / Sink.

Host evidence: `D:\coding\moiren-evidence\20261031-integrated5f9-deployment`.
`root-integration-validation.json` retains672 feature audio tests passed,
2 ignored,45 scoped WASAPI tests, the complete622-test probe suite passed,
strict feature-library Clippy and fmt PASS. The retained
`integrated-audio-tests.stdout.log` and `integrated-probe-tests.stdout.log`
confirm672/0/2ignored and622/0/0ignored respectively. These are root's executed
checks, not tests rerun by this documentation author or native qualification.

`independent-5f9-wasapi-merge-review.json` retains28/28 structural checks and
no blocking findings, SHA256
`83F1BC527BFE29807E1AF80729E8AAF98AB28E56A1146E9E284B437F61B018FE`.
It verifies the exact two-parent merge, one polling helper, preserved experimental
position/QPC/raw-evidence and lease ordering, and main's Shared zero-frame and
Exclusive out-of-order regressions. Relative to the experimental first parent,
production text is unchanged after comment removal; test coverage is restored.
The review did not run Cargo/native tests; its then-empty-log note is historical,
not certification of the later populated test logs or native behavior. Source
preservation proves neither timestamp accuracy nor lossless capture/latency.

## Frozen side-by-side binaries

| Artifact | Bytes | SHA256 |
| --- | ---: | --- |
| `moiren-app.exe` | 33780736 | `4B3DD61FA488F5BB5C36AA9D0124AB0912D55FE79F49CFC33EE18882E5AFBCDE` |
| `e2e_latency_probe.exe` | 7435776 | `17BEDCF5964850B79572FF5D2FD42CD63B8C0325437926E73265AD9DB5F946B3` |

`app-build-pin.json` and `probe-build-pin.json` identify the exact revision and
target paths under `D:\MoirenTest\20261031-integrated5f9-deployment\bin`.
Build completion and target staging are not themselves a native acquisition.

## Verified application smoke

`app-deployment.json`, UTC10:39:49.0110045, records the exact application
length/hash, successful help2, monitor-help2 and offline2 exits0. The offline
gain demonstration returns the expected eight samples. The isolated smoke
profile does not establish saved desktop preferences or native UI behavior.

`GuiStarted=false`, `NativeAudioOpened=false`, `DriverInstalled=false` and
volume/mute/default-role write counters0 are retained, with no remaining owners.
Installed driver610 remains Running, on-disk SYS SHA256
`94EAE9BC2F1611C38F050977F7E740A09FE8C5B8EB19BC8780A82036FD0D32C7`.
The receipt retains boot `2026-10-30T09:50:49.6598450+08:00`. This is not an
in-memory driver attestation, new desktop acceptance or PCM measurement.

## Exact software IO/Bus acquisition pending

The proposed next control reuses the existing `live-vad` owned
`Playback -> Bus -> MIC` graph and native-first MIC retirement, with an
Interactive/Limited launch. Its actual command, context, negotiated periods,
PCM/packet evidence, joined counters, cleanup and archive must be recorded from
the acquired outcome, not inferred from these build/smoke receipts.

At this documentation checkpoint root reports the reviewed new launch refused
its busy guard before task registration: no new run, launch receipt or outer log.
An unrelated `D:\moiren-two-tier` workload continues into workspace tests and is
preserved, not killed. This refusal is not an audio startup or native result.

No such new acquisition result is asserted here. The separate Shared interactive
holder used Gain(0.5), not this Bus route, and did not capture post-Gain MIC PCM.
Physical calibration/reference equipment remains unestablished. Neither this
deployment nor an operational software control completes S1-S3, proves stable
sub10ms or impossibility, qualifies physical latency or promotes a default.
Later acquired IO/Bus results belong in a separate receipt; canonical STATUS is
not changed by this deployment-only document.
