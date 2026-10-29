# Moiren Windows audio

Professional settings use `professional::ProfessionalBackend` for WASAPI
Shared/Exclusive and a single stereo ASIO driver, with f32/f64 processing.
ASIO capability queries, native callback diagnostics, bounded notification
recovery and finite opt-in probes are documented in the
[ASIO v1 acceptance record](../../docs/validation/asio-v1.md).

This crate contains Engine-backed Shared capture/render paths and independent,
opt-in W00 probes. It does not yet implement the complete W01–W17 backend contracts.

## Managed EngineHost backend

`engine_backend::ProfessionalBackend` implements
`moiren_engine::host::EngineBackend<S>` without an application dependency. Open
an `EngineHost` with an `EngineProject` and the adapter to prepare and activate
automatically; dropping the host stops and joins the native owners before
engine/control reclamation completes.

Project IO is configured through engine node configs:
`InputSource::Physical(EndpointSelector)`, `InputSource::Application { executable, intent }`,
`InputSource::Software` / `VirtualPlayback`, and `OutputTarget::Physical` /
`VirtualMicrophone` / `Software`, each with `DeviceOptions` (backend, Shared or
Exclusive mode, hardware rate, format, period frames, clock role). The
application selector is persistent by executable name; activation resolves it to
a live process identity verified by PID and creation time, and an
exited/recycled process is never silently replaced. Explicit endpoints wait for
their exact ID; `FollowDefault` retains its role and follows changes to the
default identity.

Graph IO is stereo; the Engine processing rate and sample format are
project-level (44.1–96 kHz, f32 or f64), and the professional path resamples
against the 48 kHz native carrier with a precomputed windowed-sinc SRC.
Supported native Shared formats
retain the Client3 period path: render uses 48 kHz stereo f32, while capture
also accepts 44.1 kHz and mono f32. Other valid PCM or float mix formats with
the same supported channel layout use the Windows Shared
AUTOCONVERTPCM/SRC_DEFAULT_QUALITY path for rate/bit-depth conversion and report
that conversion explicitly. Physical capture accepts mono or stereo; render
requires a stereo endpoint. Multichannel capture and stereo-to-mono render are
rejected before initialization, without an implicit downmix or channel mapper.
Explicit channel masks must identify front-center mono (4) or front-left/right
stereo (3); absent/zero masks retain conventional mono/stereo order. Other masks
are rejected, including valid masks for other two-channel speaker pairs. The
catalog marks these forbidden layouts unsupported rather than conversion candidates.
Converted packets are 48 kHz f32 and retain the endpoint's one or two channels,
so conversion adds no real-time decoding, allocation, or resampling. Catalog
conversion candidates remain unverified until stream initialization succeeds.
Physical capture keeps its existing
native-layout handling and clock adaptation after this boundary; process capture
keeps its existing fixed 48 kHz virtual path.
Every output has its own sink; exactly one master drives Engine and bounded
follower bridges adapt independent device clocks. Duplicate resolved endpoints
and missing/multiple masters are rejected. Explicit hardware-rate intent is
checked against the endpoint mix rate, not a converted client's rate. Shared
mode never reconfigures that endpoint. Custom input budgets still require
physical capture negotiation.
`DeviceOptions::period_frames` requests an exact Shared engine period in frames
(64..=4096, power of two; validated before initialization). Without an explicit
`hardware_sample_rate` the processing rate is used. This is period intent, not an
actual-period observation; Windows may negotiate a different period. Omitting the
field retains the latency profile or endpoint-default period policy.
Both sides share one cancellation signal and activation gate. `Running` requires
every native stream to acknowledge Start; a stream failure stops the linked owners.

Compatible updates retain every active output consumer and transfer each sole
writer at the block boundary. Managed attachment installs all required engine
sink guards; arbitrary candidates that omit any protected sink reuse are retired and
the current output continues. Successful swaps publish the current revision and
continuous frame.

The saved latency profile (Compatible / Balanced / Low) is an application-level
setting: `moiren-app` maps it to `DeviceOptions.period_frames` device buffer
requests when no explicit buffer is configured, and profile changes take effect
on the next start (see the
[managed routing design](../../docs/designs/14-managed-routing-low-latency.md)).
Smaller periods are requests, not measurements.
`ProfessionalBackend::observations()` publishes per-owner stream timing,
initialization fallback and bounded bridge counters. Process capture retains
its distinct virtual timing boundary.

Resource enumeration and availability polling are owned by the application
service (`moiren-app`), not by this adapter. Identity or availability changes
and a recoverable preparation failure generate a new resource generation;
static unsupported configurations do not arm retry probes. Endpoint probing
supplies retry eligibility; actual stream preparation still establishes format
and availability.

Native attachment errors and caught audio/bookkeeping panics return the Engine
to its non-RT joiner. `last_native_report` retains native render/capture failures
along with owned-resource return. Deterministic tests use fake native owners.
Separate muted hardware checks verify automatic Shared startup, same-owner hot
updates, automatic output-selection replacement with a new epoch, and physical
capture/render shutdown. Unplug/replug and occupation recovery remain covered by
controlled resource-event/probe tests rather than a physical unplug experiment.

```text
cargo test --locked -p moiren-windows-audio --test engine_backend -- --ignored --nocapture --test-threads=1
```

The examples enumerate active endpoints and audio sessions, capture an explicitly
selected process tree, or probe explicit physical endpoints. Physical capture
reads microphone statistics; physical render submits SILENT frames. Neither
example saves PCM or changes volume/mute/defaults. The separate `exclusive_probe`
below explicitly opens Exclusive mode.

## Exclusive occupation / process capture experiment

```powershell
cargo run --locked -p moiren-windows-audio --example w00_probe -- --list
cargo run --locked -p moiren-windows-audio --example exclusive_probe -- --pid <PID> --endpoint '<render endpoint ID>' --seconds 3
```

Keep the selected application playing through that endpoint for the entire run.
This temporarily interrupts **all shared playback on that endpoint**. The probe
does not modify endpoint settings, volume, mute, or defaults. Exclusive access
and shared-stream preemption must already be allowed by the device settings.
It negotiates a supported native Exclusive format and submits silence; it never
falls back to Shared. Driver rejection is reported with stage and HRESULT.

One process-tree capture client remains active across `baseline`, `exclusive`,
and `released`. Each phase discards 500 ms of transition data, then measures
`--seconds` (1..60, default 3). JSON includes RMS, peak, signal frames,
discontinuities, Exclusive callbacks, and both Stop results. Only signal above
1e-5 over at least 80% of wall-clock time counts as sustained evidence. Exit 0
means signal was observed during Exclusive with a valid baseline; exit 2 means
no sustained signal or an inconclusive baseline; exit 1 means an API/lifecycle
failure. JSON is emitted only after owners stop and join. No PCM is saved.

This is a diagnostic experiment, not a managed-backend output mode. Successful
capture alone does not establish selective application takeover, suppression of
physical sound, recovery, or compatibility with other capture software. Verify
that the PID really renders to the chosen endpoint and assess the released
window separately. Windows can disconnect shared sessions when Exclusive mode
overrides them, so applications may stop, retry, or move to another endpoint.
See the [experiment and results](../../docs/superpowers/plans/2026-10-15-exclusive-loopback.md).

## Engine-backed Process Loopback

The retired `render`/`monitor` app CLI documented this path before the
professional-only consolidation; the library behavior below is current.

`process_loopback::inspect_process(pid)` returns a PID, creation time in 100 ns
units, and executable basename. The graph-level selector is
`InputSource::Application { executable, intent }`; `ProcessLoopbackOptions`
carries the resolved identity and the owner reopens and verifies it before
activation. A stale/recycled PID fails instead of binding another application.
Targets containing this host (itself or an ancestor) are rejected to prevent
output feedback. Ancestry checks are conservative when Windows parent PIDs are
stale. Other application sources are not enumerated or captured automatically.

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
linked render before native cleanup. No automatic rebinding occurs. Virtual
clients observed on this machine return zero device position for every packet;
process sources disable inferred position gaps while retaining native
DATA_DISCONTINUITY and timestamp diagnostics. Physical sources retain frame-gap
detection. The capture report identifies source kind and process identity;
`endpoint_id` is null for a virtual process source, and Windows conversion is
explicit.

Worker-side `prepare_capture_with_gate` and `prepare_process_capture_with_gate`
negotiate native streams before one shared `ActivationGate` releases their
owners. Ready does not mean Running: no capture Start, render DSP, or render
Start occurs before activation. Duration starts after native Start;
`has_started` acknowledges only successful native Start. Cancellation wakes both
prepared and running owners, including continuous runs.

Implementation and acceptance: [Process Loopback plan](../../docs/superpowers/plans/2026-10-09-process-loopback.md).

## Physical capture and clock bridge

Physical capture opens a Shared `WasapiStream` on a dedicated COM owner and
returns a `PreparedCapture { session, source, observer, sample_rate, channels }`.
Supported native 44.1/48 kHz mono/stereo f32 retains its Client3 path; native mono
is explicitly copied to both graph channels. Other valid mono/stereo PCM/float
formats use Windows Shared conversion to 48 kHz f32, preserving channel count.
Multichannel microphone arrays require an explicit channel-mapping path that
is not implemented here; they are rejected rather than downmixed. The returned
sample rate/channel count describes client
packets, not the endpoint mix. The selector does not require a render/default
endpoint. Conversion candidates have `format_supported: null` and
`windows_conversion_required: true` until actual initialization verifies them.
The command never changes device format or system/session settings.

Both capture and render reports carry `shared_period.mix_format`, `client_format`,
`windows_auto_conversion`, initialization API and fallback reason. Conversion
uses legacy Shared initialization, not a Client3 low-latency promise. Requested
milliseconds remain intent; `requested_period_frames` and Client3 capabilities
stay unknown on that path. Raw `engine_period_frames` / `engine_sample_rate` are
separate from `actual_period_frames`, which is ceiling-normalized to client
frames before queue budgets are calculated. Optional period-query failures
remain unknown rather than substituting a request or a guessed period.

PCM 8/16/24/32-bit containers (including valid bits such as 24-in-32), float
32/64-bit containers are validated before initialization. Descriptor validation
does not authorize channel conversion: only the mono/stereo layouts and masks
described above are admitted. Windows/driver acceptance remains authoritative; this is not
Exclusive-mode format support or a bit-perfect output guarantee. The capture
report schema is version 7.

When Windows SRC changes the capture rate, native device positions and client
packet lengths have different frame units. Such streams do not infer position
gaps, queued-frame age or capture/QPC rate from those incompatible clocks; native
discontinuity flags still reset the bridge, and the inventory servo still
adapts independent device clocks. Same-rate native/converted streams retain
their position-based timing.

Latency recorders keep fixed storage during callbacks. Final trace snapshots
own only their valid entries and are allocated after Stop; this avoids copying
full trace arrays through nested Windows cleanup frames. No PCM is saved.

Every nonempty packet is released on its acquiring thread, including invalid
packets and bridge overflow. SILENT needs no readable pointer, TIMESTAMP_ERROR
invalidates timing diagnostics without discarding valid audio, and nonfinite
samples become zero. DATA_DISCONTINUITY, position gaps and overflow change the
frame generation; interpolation never crosses generations. Stop is checked
between packets and wins over audio events. `CaptureSession` supports
`request_stop`, `is_finished`, `join`; Drop stops and joins the owner.

`CaptureSession::stop_signal()` shares one kernel stop event across owners;
both broadcast it before retiring their bridge or releasing native objects. A finished
producer must not leave the render side running while control waits for COM cleanup;
this shutdown race was reproduced and fixed during hardware validation.

`clock_bridge::capture_bridge` prepares a fixed-capacity SPSC frame ring, a
`CaptureIngress`, stereo `ClockSource: RtAudioSource<f32>`, and cloneable scalar
`BridgeObserver`. Default storage is 8192 stereo frames including generation
metadata (128 KiB), with a 2048-input-frame target. Initial/reprime backlog is
trimmed to the target once; `prime_discarded_frames` records intentional old
frame removal. Offline callers can disable `trim_on_prime` to start at frame 0.
Overflow drops new tail frames; underrun clears the tail and reprimes.

Prepared input watermarks also bound running backlog. On a nonempty read at
or above high water, the consumer discards an old prefix from one queue
snapshot, including held interpolation frames, down to the independent servo
reference. It resets phase and controller state and fades the retained stream
in over 32 output frames, continuing across Engine segments. Generation
checks still precede interpolation. `backlog_discarded_frames` and
`backlog_recoveries` count intentional recovery separately from startup trim,
producer drops and native discontinuities. Recovery marks the boundary as
discontinuous without an xrun unless interpolation also encounters starvation
or a generation boundary. `trim_on_prime=false` disables both trimming
policies; configurations without prepared watermarks keep their existing
running behavior. Audio-age limits remain future work.
Probe envelope schema 4 and application monitor schema 5 include
the bridge scalar counters; Capture/Render schemas stay 6/5.

Continuous linear interpolation handles nominal 44.1→48 kHz and small clock
differences. A smoothed fill PI servo, with anti-windup, bounds correction to
±2000 ppm. This is a functional SRC baseline, with frequency-response limits;
it is not a professional bandlimited converter. `correction_ppm` is a queue
control signal, not a hardware clock measurement. The target adds buffering
latency and neither `GetStreamLatency` nor nominal sample rate measures the
complete input-to-output latency.

The controller samples inventory every 32 successful output frames, carrying
its cadence and ratio across Engine segments. Splitting one demand into
parameter segments therefore follows the same control trajectory. At a
prepared low-water boundary it temporarily uses the configured negative
correction limit and freezes integral learning, so stale filtered backlog
cannot accelerate a depleted queue. `low_water_guard_frames` counts only
successful guarded output. Refilling releases the guard at the next fixed
quantum. The guard never invents samples or clears underrun counts and cannot
replace the period, gap and jitter admission budget.

`ClockSource::oldest_frame_stamp()` returns an owner-local `QueuedFrameStamp`
for the oldest pending native frame. Packet position, first-frame QPC,
accepted-prefix length and frame offset travel in the same SPSC slot as the
samples and generation. Trimming, overflow and ring wrap therefore cannot
relabel old samples with a newer packet's clock. Deferred interpolation phase
is accounted for when selecting the pending frame; a stamp does not establish
that enough lookahead is available to render it.

Timing needs two consecutive plausible packets in one generation. Zero/error,
nonmonotonic or implausible QPC, overflowing position and disabled position-clock
tracking produce `None`; audio continues through the existing FIFO controller.
The broad 1% packet-slope gate tolerates 100 ns quantization and does not estimate
drift or widen the SRC correction limit. First-frame QPC and native offset stay
separate: this API does not report audio age or physical presentation time.
Preparation includes all sample/stamp slot storage in `byte_budget`. Telemetry
schemas and the approximate latest-received packet scalars remain unchanged.

Qualified packets also carry `rate_correction_ppm`, a capture/QPC estimate
relative to the nominal input rate. It requires two successive windows of at
least one second and eight packets, each within 2000 ppm of nominal and agreeing
within 100 ppm. Integer differences are taken before division; callback arrival
time and large absolute QPC values do not bias this estimate. Invalid clocks,
generation changes and fully dropped packets restart qualification. An already
queued packet keeps its original estimate.

Prepared live inputs (`prepared_input` and `trim_on_prime`) use the oldest
current frame's estimate as servo feed-forward. Capture/QPC drift does not
measure the ratio between independent capture and render clocks, so the FIFO
controller continues to learn the residual. Feed-forward and FIFO correction
share one total configured limit; low-water protection overrides their sum.
Legacy inputs, finite offline policy and unqualified timestamps retain FIFO
control. This estimate neither enables age-based discard nor qualifies a
hardware latency profile.

Snapshots are approximate independent atomic scalars; final snapshots after
joining both owners are stable. Capture/render failures, packet discontinuity,
dropped/silent/nonfinite samples, startup trim, priming silence, underrun,
resets, fill and correction remain separately observable. No PCM is saved.
The last underrun's output position and producer-finished flag distinguish a
running starvation from a teardown race; diagnostics never clear xrun counters.
Stop/rebind must prepare a fresh bridge; Process Loopback, multiple devices,
quality SRC and automatic recovery remain separate slices.

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
without opening an audio device. Real device tests are opt-in examples, never `cargo test`.

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

## Bounded latency observations

Capture and Render options accept `latency_telemetry`: `Disabled`, `Summary`, or
`Trace`. The application enables Summary. The owner prepares fixed histograms
and a 256-entry trace before activation. A full trace drops diagnostics while
continuing audio; the report keeps cumulative histogram samples and drop counts.
Disabled profiling performs no QPC reads. Quantile scans and JSON serialization
happen after streaming, and snapshots own their data.

All owner timestamps are normalized QPC in 100 ns units. Histogram p50/p95/p99
values are upper bounds with four buckets per octave, capped by the exact
observed maximum. Render padding is recorded before refill. Capture submitted
frames count frames accepted by its bridge, and Capture has no render-padding
samples. Callback duration covers successful wake processing through the native
transfer and sample statistics, excluding the subsequent telemetry recording;
the benchmark reports that instrumentation cost separately. Startup prefill,
activation-gate waits, Stop, and failed transfers are outside these histograms.

The retired `latency_probe` example collected these observations with explicit
endpoint IDs (it was removed with the legacy stack in the 2026-10-27
consolidation). The recorder remains benchmarkable in-process. The probe
reported raw `GetStreamLatency` separately and kept end-to-end measurement
method `unknown`. Queue observations and callback timings cannot establish
physical end-to-end latency. Bridge `last_qpc_100ns` belongs to the latest
received valid packet, even if overflow drops it; it is not FIFO audio age.

## Shared engine periods

Physical Capture and Render accept `shared_period: SharedPeriodPolicy`. The
default desired period is 5 ms; `desired_period_ms: None` requests the device's
default. Preparation queries Client3 capabilities after setting client
properties and rounds to a fundamental multiple inside the min/max range at
each stream's native rate. A 5 ms desire can therefore produce a longer request.
Client3 initialization uses `EVENTCALLBACK` and the stream's session GUID.
Process Loopback retains its independent initialization path.

Capture report schema 6 and Render schema 5 include `shared_period`: desired
time, capabilities, effective request, actual engine period, initialization
API, and error details. The existing Render `engine_period_frames` mirrors the
same actual-period observation. Capacity remains a separate `buffer_frames`
value. An unavailable actual-period query stays null with its HRESULT.

Only a missing Client3 interface or an explicitly unsupported capability query
permits legacy initialization. The report preserves the failed Client3 stage
and HRESULT alongside the fallback reason. Invalid capabilities, device loss,
and failed Client3 initialization return an error. No attempted initialization
is retried on the same client and no automatic period/format-conflict retry is
provided in this slice.

Actual engine periods and callback demand are observable through
`ProfessionalBackend::observations()`; the desired value and
`GetStreamLatency` alone do not establish end-to-end latency.

## Explicit render queue targets and preparation budgets

Render options accept `queue: RenderQueuePolicy`. `FullCapacity` preserves the
existing prefill and refill behavior. `Target` requires a frame target, wake
margin, and positive callback budget, all in native output frames. Preparation
requires an actual engine period and rejects targets that cannot cover the sum
of period, wake margin and callback budget, or exceed capacity. It never clamps an
unachievable deadline budget to capacity.

Render report schema 7 includes the immutable prepared `queue`. Startup prefill
and callback demand both use its target, while staging retains native capacity.
Padding above target causes zero demand without advancing Engine time; native
`min_writable_frames`/`max_writable_frames` retain their capacity-minus-padding
meaning. The latency histogram measures the actual requested refill demand.

The pure `prepare_latency_budget` API resolves independent input capacity,
prime/servo/low/high targets and a render queue from both native timings. Custom
input admission models one capture period plus explicit jitter, maximum render
demand converted to native input frames, and two-frame interpolation lookahead.
Compatible resolution preserves the 8192/2048 input settings and full output
capacity, without claiming a modeled deadline. These calculations do not prove
that actual device jitter fits the supplied assumptions.

Two-phase native owner preparation negotiates both owners before the control
side creates the bridge and graph, then each owner receives its prepared
runtime object. Process Loopback retains its independent initialization path.
Compatible mode preserves the legacy 8192/2048 input budget; custom mode
installs the admitted input targets in the physical Clock Bridge. Low-inventory
servo/backlog enforcement remains separate follow-up work.

The declared wake margin/callback budget are assumptions to validate, not
measured values. An unsupported or too-large actual period rejects this custom
queue before activation and cleans up both owners.
