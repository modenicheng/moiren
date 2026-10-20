# ACX audio lifecycle

The `src/audio/` subtree is now linked by `MoirenVad.vcxproj`. This file retains
its original development filename; it no longer describes an excluded source
drop. Target-machine audio/PnP qualification is still pending; see STATUS.md.

Child initialization follows the pinned SdcaVad raw-audio PDO sequence:
ConfigureInit before WdfDeviceCreate, Initialize afterward, then
AcxDeviceAddCircuitDevice on the root. There is no second static child list.
PrepareHardware runs policy staging on a system worker and adds the child's
circuit only after successful registry preparation. RemoveCircuitDevice starts
asynchronous PnP removal; ReleaseHardware stops/drains native streams and removes
the circuit. Final PDO destruction alone releases its quota slot.

One native stream can open while the Engine PCM gate is closed. Native stream
generation and gate epoch are independent. Render discards and capture returns
silence while closed; no file, tone, fake hardware volume/mute or DSP paths are
present. PCM uses the bounded root manual queues described by ABI.md.

Native packet/timer resources are serialized by an embedded passive lifecycle
mutex. DPC state uses nonpaged spin-locked helpers. Timer stop/drain precedes
packet detachment/free, and cleanup handles partially initialized streams.
QPC presentation positions are monotonic over pause/resume and rebase after a
stall; each callback completes at most one packet. Tests run outside WDF.
Every full packet-clock reset advances the PCM gate epoch before purging
requests, including stream ReleaseHardware, FreeRtPackets and cleanup. Reusing
the same ACX stream for Release/Prepare restarts packet numbering under a new
epoch even when polling misses all intermediate states. The new epoch and paused
state are published under the same lock, so fresh requests cannot enter before
the pending purge; admission resumes with RUN. Exhausted epochs close admission
instead of wrapping.

Native allocation currently admits exactly two packets for event-driven
streaming. Single-packet timer-driven allocation is rejected before allocating
storage: its partially consumed circular buffer needs segmented position and
consumption accounting. Timer-driven client compatibility remains an open
implementation and target-qualification item; the current two-packet path does
not claim that support.

See PROVENANCE.md for pinned sources and retained MS-PL notices. Linked WDK
build/static analysis is independent of installation, signing, verifier and
real Windows audio validation.
