# W00 Windows audio probes

This crate contains experimental, opt-in probes. It has no dependency on the
Moiren graph or engine and does not implement the W01–W17 backend contracts.

The examples enumerate active endpoints and audio sessions, capture an explicitly
selected process tree, or probe explicit physical endpoints. Physical capture
reads microphone statistics; physical render submits SILENT frames. Neither
example saves PCM, changes volume/mute/defaults, or opens Exclusive mode.

## Process Loopback on Windows

From the workspace root:

```powershell
cargo run -p moiren-windows-audio --example w00_probe --locked -- --list

# Read the current PID. Do not reuse the PID from a previous experiment.
$targets = @(Get-CimInstance Win32_Process -Filter "Name = 'QQMusic.exe'")
if ($targets.Count -ne 1) { throw 'Expected exactly one QQMusic process.' }
$targetPid = $targets[0].ProcessId
New-Item -ItemType Directory -Path target/w00 -Force | Out-Null
cargo run -p moiren-windows-audio --example w00_probe --locked -- --pid $targetPid --seconds 60 |
    Set-Content -LiteralPath target/w00/qqmusic.json -Encoding utf8
if ($LASTEXITCODE -ne 0) { throw "Probe failed: $LASTEXITCODE" }
```

The capture duration is 1–600 seconds, with a default of 60. No argument or an
ambiguous choice of `--list` and `--pid` is rejected. An explicit PID is required
for capture; the probe never guesses an application or captures all applications.

JSON is emitted **after stopping and releasing the stream**. Capture packets are
analyzed during the WASAPI buffer lease. Only packet metadata is copied into a
bounded, preallocated vector; reaching its bound increments `metadata_dropped`.
No allocation, formatting, audio file writes, or logger queue consumption occurs
in the successful packet processing path. Event waits have at most a 100 ms
timeout so no-audio conditions still permit normal shutdown. Activation has a
10-second timeout. The original process handle and creation time identify the
capture target; target exit ends capture rather than rebinding to another PID.

`capture.status` distinguishes `completed_with_signal`, `completed_no_signal`,
`target_exited`, and `api_failed`. API errors contain a stage and original HRESULT;
stream failures also report whatever statistics had already been collected.
The CLI exits unsuccessfully if capture/stop or the post-capture snapshot fails.
An optional MMCSS registration failure is recorded without suppressing capture.
Endpoint/session field failures are local to their snapshot and must be checked
before treating missing values as proof of unchanged state.

The process capture stream explicitly requests **48 kHz, stereo, f32** with Windows
automatic format conversion. Its format is separate from endpoint mix formats
and unknown hardware formats. `GetDevicePeriod` in the endpoint snapshot reports
the endpoint's default/minimum period; it is not the process capture event period.

Statistics include RMS/peak, silent/zero-signal packets, initial and later
discontinuities, and timestamp errors/regressions. SILENT packets require no
readable payload. RMS includes silence and excludes counted NaN/Inf values.
Timestamps are recorded in the API's 100 ns QPC units, not raw performance-counter
ticks. "Valid" timestamp packets mean the error flag is absent; that does not
guarantee every timestamp field is useful. The first real QQMusic test returned
zero device positions for all packets. Do not use such positions as a hardware
clock or infer hardware drift from process loopback event timing.

Before/after snapshots compare default roles, endpoint volume/mute, and the
original PID's session volume/mute. `observed_state_changes` reports differences;
the probe never restores settings or attributes outside changes to itself.
Equal snapshots do not prove there was no audible glitch between snapshots.
Session instance identifiers are hashed for within-run comparison; these hashes
are not persistent application selectors. Full executable paths and PCM are not
written to the report. Reports still contain local endpoint names, IDs, and
process names; they are local experiment artifacts, not automatically uploaded.

## Physical input, silent output, and clocks

Use `w00_probe --list` to review endpoint names, directions, and opaque IDs. Supply
each chosen ID explicitly; the example does not guess devices by name or switch
defaults. Up to eight streams run concurrently, each owning its COM objects and
services on its own MTA thread. All workers stop and release before JSON output.

```powershell
# Replace each placeholder with an ID reviewed in the current catalog.
$endpointIds = @('<render endpoint ID>', '<capture endpoint ID>')
$probeArgs = @('--seconds', '60')
foreach ($endpointId in $endpointIds) { $probeArgs += @('--endpoint', $endpointId) }
# Optional: compare an existing application's session settings before/after.
# $probeArgs += @('--observe-pid', $targetPid)
cargo run -p moiren-windows-audio --example w00_physical --locked -- @probeArgs |
    Set-Content -LiteralPath target/w00/physical.json -Encoding utf8
if ($LASTEXITCODE -ne 0) { throw "Physical probe failed: $LASTEXITCODE" }
```

Duration is 1–600 seconds. This experimental slice accepts native mix formats with
32-bit interleaved float samples and consistent block alignment. Other formats
are a **probe limitation**, not proof that the endpoint lacks WASAPI support.
The report distinguishes native stream format, default/minimum device periods in
the catalog, actual shared engine period, capacity, and stream latency. Unknown
properties and optional API errors remain visible.

Render primes the buffer with silence, then requests `capacity - current_padding`
frames on each audio wake. Zero demand skips `GetBuffer`. Capture drains complete
packets and releases each buffer on the acquiring thread. Packet/clock/demand
vectors are bounded and preallocated before Start; only metadata is retained.
An empty render padding observation is a diagnostic, not definitive underrun
proof. Both directions use category Other; the probe opts only its own **render**
session out of ducking. On the tested capture endpoints SetDuckingPreference
returns `AUDCLNT_E_WRONG_ENDPOINT_TYPE`; the probe does not call it there. This
preference cannot protect another application's session from attenuation.

`clock_points` preserves raw IAudioClock HRESULT, position units, and correlated
100 ns QPC values. Normalize clock positions by **GetFrequency**, never by stream
sample rate. S_FALSE, zero QPC, backwards positions/times, and repeated QPC with
different positions prevent or exclude a clock fit as appropriate. Diagnostics
cover the full trace; fit samples cover the shared QPC interval after the first
second of each stream. Residuals use a second pass to preserve microsecond
precision on long traces. Relative rates are ratios of fitted rates, and require
an overlapping QPC window.

Capture packet positions have a different contract: they count **frames**, with
QPC referring to the first frame. `capture_packet_clock` normalizes those by the
stream sample rate and excludes TIMESTAMP_ERROR packets. Later discontinuities
invalidate its fit. `comparison_clock_source` explicitly selects packet clocks
for capture and IAudioClock for render; raw capture IAudioClock anomalies remain
in `clock`. This avoids fitting an apparently advancing position against stale
QPC values observed on both tested capture endpoints, before and after packet
processing. These are Windows/driver exposed rates, not proof of physical
crystal identity or a completed adaptive clock bridge.

`device_clock_points` separately records optional IAudioClock2::GetDevicePosition
readings, whose positions are **device frames**. The device sample rate can differ
from the client's mix format. `device_clock.fit` therefore reports observed
frames/second and residual frames, without assuming a nominal hardware rate or
reporting hardware drift ppm. Raw HRESULTs and validity diagnostics remain
available even if that optional interface or its data is unusable.

Condense a trace after the run, retaining snapshots, anomalies, distributions,
and 30-second comparison fits. The standard-library script independently checks
the whole-window regression against the Rust result:

```powershell
python crates/moiren-windows-audio/scripts/summarize_physical.py `
    target/w00/physical.json docs/experiments/windows/physical-summary.json
```

Silent output verifies Shared render demand, buffer operation, and clock access.
It does not verify an audible signal reaches the transducer. Nonzero microphone
statistics can be background noise; they do not establish intelligibility,
channel routing, or physical input-to-output latency. Before/after equal settings
do not rule out temporary audible effects during the experiment.

## Software-device lifecycle and privileges

`w00_swdevice` tests generic PnP software nodes. It does not implement an audio
driver or create a usable WASAPI endpoint. It uses the fixed `MoirenW00`
enumerator, a fresh GUID identity per scenario, and hardware IDs unrelated to
installed audio packages. No existing device ID can be supplied for removal.

```powershell
$music = @(Get-Process QQMusic -ErrorAction Stop)
if ($music.Count -ne 1) { throw 'Expected exactly one QQMusic process.' }
New-Item -ItemType Directory -Path target/w00 -Force | Out-Null
cargo build -p moiren-windows-audio --example w00_swdevice --locked
if ($LASTEXITCODE -ne 0) { throw 'Probe build failed.' }
$probeExe = (Resolve-Path -LiteralPath target/debug/examples/w00_swdevice.exe).Path
$probeOutput = Join-Path (Resolve-Path -LiteralPath target/w00).Path swdevice.json
$probeArgs = @('--iterations', '3', '--observe-pid', $music[0].Id, '--output', $probeOutput)

# Normal token: expect exit 1 and permission_denied / 0x80070005 in both cases.
& $probeExe @probeArgs

# Successful lifecycle test: Windows presents its UAC consent prompt.
# The elevated executable writes JSON after closing and uninstalling its nodes.
$elevatedProbeArgs = @('--iterations', '3', '--observe-pid', $music[0].Id,
    '--output', ('"{0}"' -f $probeOutput))
$probeProcess = Start-Process -FilePath $probeExe -ArgumentList $elevatedProbeArgs `
    -Verb RunAs -WindowStyle Hidden -PassThru -Wait
if ($probeProcess.ExitCode -ne 0) { throw 'Inspect swdevice.json for incomplete cleanup.' }
```

Each scenario tests the asynchronous callback, default Handle lifetime, rejection
of a second open handle with the same identity, and three create/close/uninstall
cycles by default (1–10 allowed). The callback context exists before creation and
survives until `SwDeviceClose` returns; the FFI callback catches Rust panics.
Closing a handle initiates removal. The probe waits for not-present status,
records the remaining phantom instance, then calls `DiUninstallDevice` on only
the exact callback identity created by that cycle and waits for its absence.
`NeedReboot` is recorded; the probe never reboots.

The raw-node case bound the Windows inbox `c_swdevice.inf` on the tested machine.
The DriverRequired case had problem code 28 and no bound INF. Neither created an
audio endpoint. All-state endpoint IDs are compared during each cycle and after
the run, alongside default roles, endpoint volume/mute and the observed PID's
session volume/mute. JSON preserves HRESULTs and CONFIGRET values. Exit 1 means
a cycle, cleanup or state comparison did not pass; a rejected normal-token
creation is an expected negative test, not successful lifecycle coverage.

This example does not install, update or delete driver packages, use persistent
ParentPresent lifetime, test forced process termination, or test deletion of an
audio endpoint held open by another application. See the
[2026-10-08 experiment](../../docs/experiments/windows/2026-10-08-w00-swdevice.md).
API contracts: [SwDeviceCreate](https://learn.microsoft.com/en-us/windows/win32/api/swdevice/nf-swdevice-swdevicecreate),
[SwDeviceClose](https://learn.microsoft.com/en-us/windows/win32/api/swdevice/nf-swdevice-swdeviceclose),
[DiUninstallDevice](https://learn.microsoft.com/en-us/windows/win32/api/newdev/nf-newdev-diuninstalldevice).

## Checks

```powershell
cargo fmt -p moiren-windows-audio --check
cargo check -p moiren-windows-audio --all-targets --locked
cargo test -p moiren-windows-audio --locked
cargo clippy -p moiren-windows-audio --all-targets --locked -- -D warnings
```

Pure statistics tests do not require hardware. Windows callback/BLOB lifetime
tests use locally implemented COM objects, without activating an audio device.
Real device tests are only run through the examples, not by `cargo test`.

The W00 result is limited to the tested machine, application, format, and duration.
Process isolation controls, include/exclude comparisons, browsers/children,
restart/PID reuse, OBS, Takeover, audible render, cross-clock bridges, device loss,
and long-term behavior remain separate experiments.

References: [Microsoft Application Loopback Sample](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/),
[ActivateAudioInterfaceAsync](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-activateaudiointerfaceasync),
[IAudioCaptureClient::GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer),
[IAudioRenderClient::ReleaseBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiorenderclient-releasebuffer),
[IAudioClock::GetPosition](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclock-getposition),
[IAudioClock::GetFrequency](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclock-getfrequency),
[IAudioClock2::GetDevicePosition](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclock2-getdeviceposition).
