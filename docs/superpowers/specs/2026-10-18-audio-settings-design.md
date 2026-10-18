# Professional audio settings

## Accepted requirements

Settings is a modal overlay over the current workspace. Backdrop, Escape and
Close dismiss it without changing the underlying page, camera or selection.
Audio defaults live in Settings; Devices edits persistent overrides keyed by
stable device identity and direction. Removing an override restores inheritance.

Processing rates are 44,100, 48,000, 88,200 and 96,000 Hz. Processing precision
is 32-bit or 64-bit float, independent of device PCM packing. Processing and
device buffers offer 64, 128, 256, 512, 1024, 2048 and 4096 frames.

The selectable APIs are WASAPI and ASIO. WASAPI has Shared and Exclusive modes.
Input/output defaults each specify a rate, sample format and buffer. A device
override replaces that direction's defaults for that device only. Changes to
defaults therefore affect devices that inherit, not devices with overrides.

## Configuration ownership

Portable audio intent and validation belong to `moiren-engine::audio_config`.
DeviceOptions carries resolved backend, mode, format, rate, period and clock
role. EngineProject keeps the real graph and processing configuration.

AppPreferences persists audio defaults and overrides with backward compatible
serde defaults. Projects also persist processing intent, allowing old projects
without an audio field to continue loading. Runtime state is never serialized
as requested intent.

The service owns requested and active settings separately. Saving settings does
not start audio. Applying while running explicitly restarts audio and active
settings are published only after the new owner reports Running. Device and
format validation must happen before stream activation; a rejected format must
not appear active. No silent fallback between WASAPI and ASIO or between Shared
and Exclusive is allowed.

## Native behavior

Native format is an explicit choice. Shared mode can use Windows client-format
conversion without changing the endpoint's system mix format. Exclusive mode
must pass native support checks and report alignment or ownership failures.
Requested periods are not assumed equal to negotiated capacity or device
latency. ASIO enumerates installed drivers and uses one driver session for its
duplex input/output; conflicting driver-wide rate/buffer requests are rejected.
ASIO drivers determine supported sample encodings.

Audio resources and conversion storage are prepared on the control/native
owner. RT processing/callbacks do not allocate, free memory or wait on locks.
Stop cancels and joins all owners and returns Engine ownership for control-side
retirement.

## Interface

The overlay groups Audio System, Inputs, Outputs, Appearance and Diagnostics.
Persistent navigation and a pinned footer keep Save/Restart and errors visible.
Settings presents requested settings separately from the currently running
configuration and observed native timing.

Devices distinguishes inspecting/configuring a resource from binding it to
the graph. Each physical endpoint offers Use defaults, Override and Reset.
Unavailable devices retain their stored override without being silently rebound.

## Verification

Portable tests cover validation, inheritance, JSON compatibility, exact buffer
choices, graph translation, native packing/SRC and requested/active lifecycle.
Headless UI tests cover overlay dismissal, interaction isolation, draft saving
and device overrides. Native hardware exercises remain separate from portable
checks. Workspace tests, formatting, Clippy and all-target checks finish the
implementation; screenshots are reviewed using the UI capture fixture.
