# Bounded process-output recording

The S1 diagnostic probe adds a capture-only command for retaining an actual
process tree's rendered PCM. It reuses the production Process Loopback owner;
it creates no render client, graph, source process or device-setting write.

```text
e2e_latency_probe record-process --source-pid PID --source-created-100ns N
    --artifact-dir NEW_DIR --seconds 1..600 --max-frames 1..33554432
```

The PID and creation timestamp are required. Selection resolves the process
and checks the supplied timestamp before activation; the native owner checks
identity and liveness again. Self/ancestor feedback targets are refused.
Windows converts process capture to the existing fixed48 kHz stereo Float32
format. The explicit frame capacity has a checked256 MiB ceiling and is
allocated before streaming. Duration and storage capacity are independent:
an undersized artifact retains overflow counters rather than growing.

The command reserves a new directory and uses create-new file opens. After
owner join it writes raw `capture.f32le`, `capture.metadata.json` and
`report.json`. Native preparation failures and join errors retain partial
reports; PCM write failure does not suppress the metadata attempt. Native
Stop/owner status, original identity, artifact overflow/loss and unused
compatibility-bridge counters remain separate. The unused bridge has no
consumer; its drop counters are not PCM artifact loss.

`recording_complete` means the finite capture completed, Stop succeeded and
nonempty PCM was persisted without artifact overflow or control/IO failure.
It does not certify waveform continuity, absence of discontinuities or
physical presentation. Inspect the retained native/artifact quality fields
and actual waveform independently. The report always has `qualified:false`
and returns1; malformed input or report IO failure returns2. `--dry-run`
reserves only the diagnostic report without inspecting/activating a process.

Capture includes **every render stream in the selected process tree**. The
`live-vad` probe itself owns both the external VAD source client and physical
graph output, so recording that probe cannot isolate graph output. For that
purpose select the production app's process and run its audio source as a
separate process outside the app tree. Even an isolated app recording is a
software observation, not an independent pre-route common-clock reference
or physical measurement.

## Software verification

The actual CLI regression first fails with the old
`unknown offline command: record-process` refusal. After implementation the
full example suite passes517 tests. Regressions cover explicit identity and
storage reporting, dry-run behavior, invalid duration/memory/identity,
missing/duplicate/unknown options, existing-evidence preservation, exact raw
PCM/metadata, capacity mismatch, metadata retention after PCM refusal, and
self-target refusal before native capture. Strict example Clippy passes.
These are software receipts; native acquisition is recorded separately.
