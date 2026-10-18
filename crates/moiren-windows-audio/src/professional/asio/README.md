# Direct Windows ASIO backend

`AsioStream` implements `professional::native::NativeStream`. Create, configure,
start, stop and drop it on the same dedicated native owner thread. It is not
`Send`. `open` prepares the driver and its buffers; only `start` starts audio.
ASIO/WASAPI selection and requested-format validation belong to the parent
professional backend.

## Build and discovery

No downloaded SDK, C++ compiler, build script, environment variable, or new Cargo
dependency is required. Existing `rtrb` and Windows features
`Win32_Foundation`, `Win32_Security`, `Win32_System_Com` and
`Win32_System_Threading` suffice. Registry access uses four direct `advapi32`
declarations, so neither `windows-sys` nor a registry feature is needed.

The supported runtime target is 64-bit Windows with a matching 64-bit ASIO
driver. Other platforms and 32-bit Windows return `BackendError::Unsupported`.
The source includes portable ABI, packing and buffer tests. To run those with
the ordinary Linux test suite, the parent `professional` module must declare
`pub mod asio` without a Windows-only cfg.

`list_drivers()` reads `SOFTWARE\ASIO` in the native 64-bit registry view under
HKLM and HKCU. It does not activate COM drivers or start audio. Per-user
registrations override duplicate machine descriptions. IDs are canonical CLSIDs
including braces; ordering, friendly-name changes and new installations do not
change a driver's identity. No installed driver gives an empty list. A missing
registration, COM activation failure, driver init failure or absent direction
produces a descriptive error when opening.

## Negotiation

- Rates: 44100, 48000, 88200 and 96000 Hz. The backend calls `canSampleRate`,
  `setSampleRate` and `getSampleRate`, and verifies the result again after buffer
  creation and before starting. It never substitutes another rate.
- Explicit buffer requests: 64, 128, 256, 512, 1024, 2048 or 4096 frames. A
  request must satisfy the driver's min/max and granularity exactly. There is no
  fallback to the preferred size.
- `buffer_frames = 0`: choose the driver's preferred size, bounded to 64..=4096
  frames. A native non-power-of-two preferred size is accepted if it satisfies
  the reported driver rules. `info().buffer_frames` always reports the actual
  resolved size. The parent backend should map `StreamSettings.buffer_frames =
  None` (or `DeviceOptions.period_frames = None`) to this zero sentinel.
- PCM16, packed PCM24, PCM32, float32 and float64 support both byte orders.
  ASIO's left-aligned PCM24-in-32 and right-aligned PCM16/24-in-32 containers
  are also supported, with effective bit depth reported as PCM24 or PCM16/24.
  Unsupported/unknown sample types, including DSD and 18/20-bit PCM, are
  rejected before start.
- ASIO chooses the device format. Channel types are queried both before and
  after buffer creation. `info().format` reports the actual hardware sample
  class for parent requested-format validation; it never reports `Native`.
  Differing native byte order/container width are allowed when the effective
  sample class agrees. Mixed effective input/output sample classes are rejected.
- Channels 0 and 1 are used in each enabled direction, or channel 0 for mono.
  Mono capture duplicates into the stereo carrier; mono output averages it.
  Other hardware channels are not activated.
- `latencies()` returns optional driver-reported input/output latency in frames.
  It excludes application/engine queues. Latency-change notifications trigger a
  fresh query on the owner thread. Output-ready optimization is not enabled;
  callbacks never call driver methods.

## Callback ownership and teardown

A process-wide exclusive lease spans activation through final COM Release.
Competing opens return `ResourceUnavailable`. Every data/metadata callback
increments a global reader count **before** loading the context pointer.
Publication and reader retirement use sequentially consistent atomics.

Callbacks copy native sample bytes through fixed-capacity `rtrb` rings and
signal a kernel event. They do not convert samples, allocate/deallocate, acquire
locks, call IASIO, invoke application code, or wait. Even event-signaling errors
use the raw BOOL result rather than constructing a Windows error on the audio
thread. Audio callbacks use a nonblocking reentrancy guard. Malformed double
buffer indices fault the session without accessing native buffers.

Capture overflow drops a complete callback block. Output underrun writes native
zeroes. The first capture block after start is discarded as required by the
ASIO double-buffer startup model. Queues hold at most four hardware periods.
Sample conversion and finite-value sanitization run on the owner thread;
integer output saturates while floating-point output retains finite headroom.

Stop/teardown first detaches the global pointer and waits on the owner thread for
all readers to finish. Only then can stop/dispose run or memory be reclaimed.
Successful stop clears queued capture/output samples before a subsequent start.
Final teardown calls stop when needed, disposeBuffers even after partial
createBuffers failure, then final COM Release while descriptors, callback table,
rings and event still exist. Context/rings/event are reclaimed on the owner
thread. `close()` additionally reports disposal errors; Drop performs the same
ordered cleanup. Rate/clock changes, reset/resync requests and buffer changes
wake the owner and return an error requesting stream reopen.

This relies on the ASIO driver contract that stop prevents future buffer
callbacks and final Release/exit prevents all subsequent callbacks. The callback
table has static storage, and detached calls do not access session memory.

## Validation and hardware gating

The default tests use an in-process fake IASIO vtable and allocated native
buffers. They verify configuration-before-start, native duplex conversion,
preferred-buffer resolution, startup callbacks and cleanup order for negotiation,
create, null-buffer, start, stop and dispose failures. Callback tests cover
whole-block capture overflow, output silence, first-input discard, callback
reader retirement and exclusive lease reacquisition. No default test activates
an installed driver.

The local inventory test is registry-only and checks the supplied FL Studio and
Realtek CLSIDs. Run it individually:

```powershell
cargo test --locked -p moiren-windows-audio local_registry_contains_fl_studio_and_realtek_exact_clsids -- --ignored
```

`explicitly_opted_in_hardware_duplex_silence` is ignored and additionally requires
`MOIREN_ASIO_HARDWARE_TEST_DRIVER` to contain the exact driver CLSID. Run that
specific test only after explicit permission to activate the hardware. It opens
48000 Hz / 256 frames, captures real input, and submits output silence.

## Primary ABI references

Verified against Steinberg's official ASIO 2.3.4 package:

- <https://www.steinberg.net/asiosdk>
- <https://download.steinberg.net/sdk_downloads/ASIO-SDK_2.3.4_2025-10-15.zip>

The declarations follow `common/iasiodrv.h`, `common/asio.h`,
`common/asiosys.h` and `host/pc/asiolist.cpp`. The specification confirms
right-aligned PCM containers (pages 33-34), exact buffer granularity (page 15),
stop callback synchronization (page 13), and exit/Release callback retirement
(page 12). These files were consulted as reference; they are not build inputs.
