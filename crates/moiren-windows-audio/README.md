# Moiren Windows audio

This crate contains Engine-backed Shared capture/render paths and independent,
opt-in W00 probes. It does not yet implement the complete W01–W17 backend contracts.

The examples enumerate active endpoints and audio sessions, capture an explicitly
selected process tree, or probe explicit physical endpoints. Physical capture
reads microphone statistics; physical render submits SILENT frames. Neither
example saves PCM, changes volume/mute/defaults, or opens Exclusive mode.

## First Engine-backed Shared output

```powershell
cargo run --locked -p moiren-app -- render --list
cargo run --locked -p moiren-app -- render --endpoint '<reviewed render endpoint ID>' --seconds 10 --frequency 440 --gain 0.05
```

This opt-in command submits audible PCM from a compiler-prepared tone graph.
Only native 48 kHz, stereo, f32 is supported; other formats are rejected before
Start. The selector queries active render endpoints, without requiring capture
or default devices. Per-endpoint errors remain visible; empty-ID diagnostics
cannot be selected. No volumes, mute settings or default endpoints are changed.

`render::DemandRenderer` owns exactly one Engine and its matching, initially
drained output reader. The WASAPI owner prepares COM/services, audio and separate
stop events, bounded staging and optional MMCSS. It submits initialized silence
before Start without consuming live capture reserves, then renders
`capacity - padding` frames on each audio wake, splitting processing into
Engine maximum blocks. Zero demand skips DSP and buffer acquisition. DSP and
bridge draining finish before acquiring the driver lease; only a bounded byte
copy occurs during it. Every lease is paired on its acquiring thread; an
abandoned shared-mode lease is cancelled with zero frames.

The same-owner bridge converts planar Graph output to interleaved staging; it
does not adapt independent clocks. Short transfers initialize missing samples
and fail the stream instead of submitting a stale or partial packet. Persistent
DSP counters include completed blocks even when their subsequent transfer fails;
session processed frames exclude any earlier Engine processing.

`RenderSession::request_stop` wakes independently of audio events. `join` returns
after stop/release, and dropping an unjoined session requests stop and joins.
Waits are bounded to 100 ms and duration is 1–600 seconds. The successful loop
uses prepared storage and scalar counters, with no Rust allocation, log
formatting or file writes. JSON is serialized after cleanup; optional period,
ducking and MMCSS failures retain HRESULTs. Own-session ducking opt-out affects
only this render session. `empty_padding_wakes` is diagnostic, not proof of an
underrun; `GetStreamLatency == 0` is not evidence of zero end-to-end latency.

The [10-second FreeDSP test](../../docs/experiments/windows/2026-10-08-shared-render.md)
completed with nonzero PCM and user-confirmed sound. Device loss/recovery, long
stress runs, SRC quality upgrades and plan replacement remain separate work.

## Engine-backed Process Loopback

```powershell
cargo run --locked -p moiren-app -- monitor --process <PID> --output '<reviewed render ID>' --seconds 10 --gain 0.05
```

`process_loopback::inspect_process(pid)` returns a PID, creation time in 100 ns
units, and executable basename. `ProcessLoopbackOptions` requires that identity;
the owner reopens and verifies it before activation. A stale/recycled PID fails
instead of binding another application. Targets containing this host (itself or
an ancestor) are rejected to prevent output feedback. Ancestry checks are
conservative when Windows parent PIDs are stale. Other application sources are
not enumerated or captured automatically.

The supported mode includes the target process tree. The virtual stream requests
48 kHz stereo f32 with Windows AUTOCONVERTPCM/SRC_DEFAULT_QUALITY, independently
of endpoint mix formats. Process Loopback requires a supported Windows build;
API failures retain the exact stage/HRESULT. See the [Microsoft sample](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/).

`start_process_capture` returns the same `PreparedCapture` as physical capture.
For cancellable startup, create `StopSignal::new()` first, clone it for the
control side, and pass it to `start_process_capture_with_stop`. Activation waits
at most 10 seconds after the async API returns and checks cancellation/target
liveness every 10 ms. The agile callback only sets an atomic flag; GetActivateResult
and audio-interface use/destruction stay on the capture owner. Arc-owned
activation parameters survive a late callback after cancellation/timeout; the
borrowed PROPVARIANT never frees Rust memory through PropVariantClear.

Streaming shares the physical packet lease, Clock Bridge and stop lifecycle.
The wait includes the pinned process handle, so a silent target can exit without
an audio event. `TargetExited` is distinct from duration completion and stops the
linked Render before native cleanup. No automatic rebinding occurs. Virtual
clients observed on this machine return zero device position for every packet;
process sources disable inferred position gaps while retaining native
DATA_DISCONTINUITY and timestamp diagnostics. Physical sources retain frame-gap
detection. Capture JSON schema 2 identifies source kind and process identity;
`endpoint_id` is null for a virtual process source, and Windows conversion is explicit.

Implementation and acceptance: [Process Loopback plan](../../docs/superpowers/plans/2026-10-09-process-loopback.md).

## Physical capture and clock bridge

```powershell
cargo run --locked -p moiren-app -- monitor --list
cargo run --locked -p moiren-app -- render --list
cargo run --locked -p moiren-app -- monitor --input '<reviewed capture ID>' --output '<reviewed render ID>' --seconds 10 --gain 0.05 --pan 0
```

`capture::start_capture` prepares a native Shared stream on a dedicated COM owner,
and returns `PreparedCapture { session, source, observer, sample_rate, channels }`.
The startup handshake rejects unsupported native formats before returning a
prepared source. Only native 44.1/48 kHz mono/stereo f32 is accepted; mono is
explicitly copied to both graph channels. The read-only capture selector does
not require a render/default endpoint. Other formats appear as unsupported;
the command never changes device format or system/session settings.

Every nonempty packet is released on its acquiring thread, including invalid
packets and bridge overflow. SILENT needs no readable pointer, TIMESTAMP_ERROR
invalidates timing diagnostics without discarding valid audio, and nonfinite
samples become zero. DATA_DISCONTINUITY, position gaps and overflow change the
frame generation; interpolation never crosses generations. Stop is checked
between packets and wins over audio events. `CaptureSession` supports
`request_stop`, `is_finished`, `join`; Drop stops and joins the owner.

To link both streams, pass `CaptureSession::stop_signal()` to
`render::start_render_with_stop`. Both owners broadcast the shared kernel stop
event before retiring their bridge or releasing native objects. A finished
producer must not leave Render running while control waits for COM cleanup;
this shutdown race was reproduced and fixed during hardware validation.
Independent multi-source hosts instead use separate stop events. Capture calls
`CaptureIngress::finish` before native Stop/COM cleanup; host polling closes only
that source's gate as soon as its observer reports producer completion, without
waiting for the worker join or stopping peer sources/render. Explicit finish is
idempotent, rejects later packet publication and leaves queued samples owned for
finite consumers to drain. Drop remains the completion fallback.

`clock_bridge::capture_bridge` prepares a fixed-capacity SPSC frame ring, a
`CaptureIngress`, stereo `ClockSource: RtAudioSource<f32>`, and cloneable scalar
`BridgeObserver`. Default storage is 8192 stereo frames including generation
metadata (128 KiB), with a 2048-input-frame target. Initial/reprime backlog is
trimmed to the target once; `prime_discarded_frames` records intentional old
frame removal. Offline callers can disable `trim_on_prime` to start at frame 0.
Overflow drops new tail frames; underrun clears the tail and reprimes.

Continuous linear interpolation handles nominal 44.1→48 kHz and small clock
differences. A smoothed fill PI servo, with anti-windup, bounds correction to
±2000 ppm. This is a functional SRC baseline, with frequency-response limits;
it is not a professional bandlimited converter. `correction_ppm` is a queue
control signal, not a hardware clock measurement. The target adds buffering
latency and neither `GetStreamLatency` nor nominal sample rate measures the
complete input-to-output latency.

Snapshots are approximate independent atomic scalars; final snapshots after
joining both owners are stable. Capture/render failures, packet discontinuity,
dropped/silent/nonfinite samples, startup trim, priming silence, underrun,
resets, fill and correction remain separately observable. No PCM is saved.
`live_underrun_frames` retains cumulative running starvation even when the last
underrun later belongs to an ended producer. The last-underrun position and
producer-finished flag describe that last event; neither resets XRUN counters.
Bounded packet-size/arrival diagnostics and first-underrun packet count/age help
investigate publication gaps without storing audio. At 48 kHz the default target
covers about 42.7 ms; longer input gaps can still starve it and trigger re-prime.
Stop/rebind prepares a fresh bridge. Process Loopback and independent multi-source
ownership are integrated by the [app Host](../moiren-app/README.md); quality SRC
and automatic device recovery remain separate work.

The [implementation plan](../../docs/superpowers/plans/2026-10-09-audio-backend.md)
and [validation record](../../docs/experiments/windows/2026-10-09-capture-clock-bridge.md)
distinguish simulation from hardware testing. Repeat the two-hour synthetic
drift test for both native rates and both ±1000 ppm signs:

```powershell
$env:MOIREN_CLOCK_SIM_SECONDS = '7200'
cargo test --release --locked -p moiren-windows-audio --test clock_bridge adaptive_bridge_handles_both_drift_signs_and_packet_jitter -- --nocapture
Remove-Item Env:MOIREN_CLOCK_SIM_SECONDS
```

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
Demand splitting, shortfall, allocation counting and CLI tests are pure. Stop
wake, format and render-lease tests use kernel events or local fake COM clients,
without opening an audio device. Real device tests are opt-in examples or the
app's explicit `render` command, never `cargo test`.

The W00 result is limited to the tested machine, application, format, and duration.
Process isolation controls, include/exclude comparisons, browsers/children,
restart/PID reuse, OBS, Takeover, cross-clock bridges, device loss,
and long-term behavior remain separate experiments.

References: [Microsoft Application Loopback Sample](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/),
[ActivateAudioInterfaceAsync](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-activateaudiointerfaceasync),
[IAudioCaptureClient::GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer),
[IAudioRenderClient::ReleaseBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiorenderclient-releasebuffer),
[IAudioClock::GetPosition](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclock-getposition),
[IAudioClock::GetFrequency](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclock-getfrequency),
[IAudioClock2::GetDevicePosition](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclock2-getdeviceposition).
