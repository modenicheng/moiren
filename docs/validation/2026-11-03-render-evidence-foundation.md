# Encoded render evidence foundation

Recorded 2026-11-03, Asia/Shanghai. Baseline `3bf4c1b`.
This implements the first part of S1's missing actual render-boundary evidence.
It does not install a native tap or turn the earlier nominal master projection
into a submission witness. S1-S3 native and physical acceptance remain open.

## Original gap and implementation boundary

The [measured Process recording](2026-11-03-measured-process-waveform.md) and
[separate owner ledgers](2026-11-03-process-owner-overlay.md) reconstruct actual
input/SRC output but use a nominal master FIR projection. Neither records the
actual encoded WASAPI output and its ReleaseBuffer result. Repeating an
unchanged acquisition cannot supply that missing boundary retrospectively.

[`render_evidence.rs`](../../crates/moiren-windows-audio/src/render_evidence.rs)
adds an opt-in portable storage primitive, not an enabled backend feature:

- Preparation validates mono/stereo sample layout, actual container/valid bits,
  positive rate, bounded packet size and checked ring-plus-staging arithmetic.
  The 8 MiB payload cap includes the complete frame records and encoded scratch.
- `stage_packet` copies encoded bytes while the caller still holds the native
  buffer lease. It retains no native pointer and publishes nothing yet.
- After releasing the lease exactly once, the future native caller passes the
  release HRESULT to `finish_packet`. This reads only owned staging and records
  accepted/rejected attempts separately from evidence loss.
- Full/abandoned evidence storage drops the whole diagnostic packet without
  retrying, gating or changing audio. Packet sequences, missing evidence,
  invalid usage, sequence exhaustion and saturating counters stay explicit.
- Closing a pending stage records an unknown outcome, not an accepted or
  rejected submission. Closure plus an empty reader is not a native join.

The format preserves packed PCM24 versus 24-in-32, padding, Float32/Float64 bits
and mono/stereo width without decoding, rounding or normalizing samples.
Release outcomes are caller-supplied, not authenticated native provenance.
Even a future successful ReleaseBuffer witness will not prove playback,
waveform continuity, a common clock or physical end-to-end latency.

Both endpoints must survive native shutdown/join and be reclaimed on control.
In particular, the writer owns scratch: retaining only the reader does not make
writer destruction allocation/deallocation-free. Native attach/retirement and
the backend/CLI collector remain follow-up work; this slice changes no existing
owner path, audio policy, buffer default, driver or qualification gate.

## Verification

The tests are written before implementation. An isolated exact HEAD archive
initially fails to compile with the requested APIs absent. Two sequence-limit
assertions then fail against the first implementation and pass after the sticky
saturation flag is corrected. Final focused coverage passes 20 tests.

Independent mutations replace staging copies with zeros and invert the HRESULT
success test. Each produces the expected assertion failure; both are removed.
The suite covers exact layouts/bytes, ring wrap, whole-packet loss, abandoned
readers, malformed/nested/double operations, pending closure, sequence/counter
exhaustion and drain-after-close. The counting allocator observes no allocation
or deallocation across stage/finish/pop/snapshot/close and error/loss paths,
including 10,000 transfers. Endpoint destruction remains outside that scope.

An initial full-library invocation incorrectly reuses the deliberately mutated
test binary after restoring a source file with its old timestamp. Its six
failures remain archived as test-tool provenance, not production failures or a
green result. Source hashes match the worktree; explicitly refreshing the
restored file timestamp forces recompilation before final verification.

Final isolated verification completes at 12:04:25 +08:00: the audio library
passes 822 tests with four ignored and zero failures; workspace all-target
check, strict audio all-target Clippy and workspace formatting each exit zero.
These are software checks, not native acquisition or physical acceptance.

Host evidence:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-s3-remaining-20261103-1143`.
All 1,408 original archive entries match HEAD Git blobs. Intended source
overlays and verification logs stay separate from the 27 preserved dirty/cache
files. No native audio or desktop window is opened on the development host.

## Remote resource recovery

At 11:41, remote preflight finds no running Moiren task but only 225,284 KiB
free memory. Two old owned PowerShell report readers, PID 5508 and 26016, hold
approximately 27 GB combined. Their decoded commands only read the completed
1030/1100 acquisition reports and serialize decorated Get-Content output.
They are not audio owners or protected external applications.

Cleanup verifies each executable, full command SHA256, separate CIM creation
time, held .NET process start FileTime and terminal task result before killing
that exact held process. Both terminate; there is no process-name-wide kill.
Independent 11:51:55 readback confirms both absent, no running Moiren task and
26,537,228 KiB free. Protected OBS and QQMusic creation identities are unchanged.
No host process, external window, audio endpoint, default or driver is operated.

The resource cleanup restores the ability to run later isolated remote tests;
it is not an audio correctness fix or a native/physical qualification result.
