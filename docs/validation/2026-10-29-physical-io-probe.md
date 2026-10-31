# Exact physical Input -> Bus -> Output collection

Date: 2026-10-29. Source: `f83f3a5`.
This receipt covers physical-route orchestration software.
**S1-S3 remain incomplete; stable real output below 10 ms is not established.**

## Command and ownership

The actual `e2e_latency_probe live-physical` command constructs the production
three-node graph: physical stereo Input(Follower) -> Bus -> physical stereo
Output(Master), with 48 kHz processing and an explicit Engine block size.
It uses ProfessionalBackend and EngineHost, with no Gain, tone generator,
custom PCM transport or hidden physical output.

```text
live-physical --input CAPTURE_ID --output RENDER_ID
  --recorder-config recorder.json --artifact-dir NEW_DIRECTORY --seconds 10
  --validation-mode correctness --jitter-tolerance-ms 3000
```

Input, output and independent recorder require three explicit, distinct,
nonempty endpoint IDs. No name matching or default-device selector is used.
Requested node-to-endpoint mappings are retained, but observation node IDs are
not claimed as independently authenticated hardware identity.

The recorder config is schema 1, denies unknown fields and is limited to
64 KiB. For example:

```json
{
  "schema_version": 1,
  "input_endpoint_id": "INDEPENDENT_RECORDER_ID",
  "ring_frames": 8192,
  "max_pcm_bytes": 67108864,
  "max_metadata_bytes": 8388608,
  "max_packets": 100000
}
```

These are storage bounds, not a guaranteed sufficient capture budget. Ring
storage is nonzero and at most 8 MiB; PCM is capped at 3 GiB, metadata at
256 MiB and packet count at 1,000,000. The original config bytes are copied.
The artifact directory must be new; files use create-new semantics.
`--dry-run` validates and writes unqualified receipts without activating IO.

## Requests and observation

- `--capture-mode shared|exclusive` and `--capture-period-frames N` apply only
  to physical input. `--render-mode` and `--render-period-frames` independently
  apply to output. Explicit periods are powers of two from 64 through 4096.
  Omission preserves native/default settings.
- The existing common request validator refuses unknown or mismatched actual
  periods, Shared compatibility/conversion fallback and Exclusive capacity
  mismatch. Requested period, actual period, capacity, Engine block size and
  queue target remain separate.
- `--input-target-frames`, `--input-capacity-frames` and
  `--input-jitter-frames` reach the actual backend latency budget. Omission
  preserves compatibility. Static impossibilities fail before artifacts;
  native-dependent admission still occurs in the backend.
- `--render-target-frames N|capacity`, `--wake-margin-frames` and
  `--callback-budget-frames` reach the render queue policy. Exclusive output
  requires full-capacity rendering. Correctness grace never expands a queue.

The independent MeasurementSession is prepared, activated and acknowledged
Running before the graph opens. EngineHost automatically starts its physical
workers. Exact native admission is consequently **post-activation validation**,
followed by retirement on refusal; this is not a pre-audio gate.

The immutable admission decision retains actual metadata and its refusal
reason separately from later observations. Current host, owner, resource,
plan and timeline identity, exact native nodes, direction, mode and format
are checked. A replacement owner cannot be adopted silently.

## Duration, quality and retirement

`--seconds` is the requested full steady collection duration, from 1 to
600 seconds. One separate startup window of 10 seconds plus selected progress
grace is anchored before recorder preparation. The overall collection ceiling
is fixed at startup ceiling plus steady duration. Blocking native preparation
and synchronous joins cannot be interrupted by these control-side checks;
deadlines are checked when the existing APIs return.

Readiness requires input native activity, output writes and Engine timeline
progress. Follower priming silence stays in a separate startup snapshot;
steady counters use checked deltas from that baseline. Progress is supervised
in all three dimensions, including a terminal observation. A late counter
advance cannot erase an already expired observation/progress interval.

`latency-sensitive` fails fast on available quality faults. `correctness`
retains those faults and continues with configurable 0..3000 ms additional
progress grace. Identity, format, generation, native API, malformed evidence,
storage and retirement failures remain hard errors. Grace is neither audio
latency allowance nor permission for unbounded execution.

First quality/failure evidence, startup counters, latest observations and final
checked deltas remain separate. Available professional observations do not
expose graph capture discontinuities, timestamp errors or silent-packet
frames. These remain null, with `quality_complete: false` and an explicit
limited quality scope. Unknown counters are never replaced by zero.

Cleanup requests recorder stop, drops/joins the host, then joins recorder
native ownership before finishing its sink. EngineHost has no separate public
asynchronous stop API. Native and professional handles are pinned before
host drop and inspected afterwards for Closed, exact generation and faults.
All independent cleanup stages are attempted after an error or unwind.
Recorder reconciliation reuses the existing structured validator, including
exact accounting of loss versus persisted PCM/packets.

Both `manifest.json` and `report.json` are attempted on operational failure.
They retain partial native evidence; an escaped collector unwind produces
failure receipts with cleanup explicitly unverified. Raw independent capture
uses `measurement.f32le` and `measurement-packets.jsonl`.

Correctness exit 0 requires collection and cleanup completion without a hard
failure. `quality_passed` may still be false. Strict runs and dry runs keep the
unqualified/nonzero convention. All reports remain `qualified: false`.

## Evidence and delivery

Ignored logs, final review diff and frozen source hashes are under
`target/s1-s3-20261028/`.

| Check | Fresh result / log |
| --- | --- |
| Parser/topology behavioral RED | Missing command/implementation and topology failures: `physical-io-parser-red.log`, `physical-io-topology-red.log` |
| Invalid-token/jitter regressions | Runtime RED -> GREEN; final 34-frame checked-add boundary in `physical-io-jitter-boundary-{red,green}.log` |
| Actual collector invocation and unwind receipts | Runtime RED -> GREEN; final parser/runner suite 13 passed in `physical-io-runner-panic-green.log` |
| Lifecycle/admission/progress/first-quality/final deltas | Multiple genuine behavioral RED -> GREEN logs named `physical-io-{session,admission,final-evidence,first-quality}-red.log` |
| Terminal stalled/recovered interval | Real controller -> tracker regression RED -> 14 session tests passed, `physical-io-terminal-{red,green}.log` |
| Portable native observation projection | Linux test-profile import failure in `physical-io-linux-tests.log`; repaired without cfg-hiding controller tests; Windows 15 session and 1 projection tests passed in `physical-io-portability-windows-{session,projection}.log` |
| Complete final Windows probe suite | 471 passed, `physical-io-full-final.log` |
| Audio library/integration/doc regression | 831 passed / 3 ignored, 31 result summaries, `physical-io-audio.log` |
| Strict audio all-target Clippy | Passed, `physical-io-clippy-final-all.log` |
| Strict actual probe test-profile Clippy | Passed, `physical-io-clippy-final-tests.log` |
| Workspace fmt / scoped diff | Passed, `physical-io-fmt-accepted.log` and pre-commit diff checks |
| Linux normal / actual probe test-profile compile | Passed, `physical-io-linux-final-{normal,tests}.log`; respectively 1 / 4 existing cfg-specific dead-code warnings |

Linux checks compile, not execute, the normal example and actual example test
configuration. The temporary Linux failure and earlier pre-fix results remain
separate logs; they are not relabeled as final passes. The repaired projection
retains the full original native snapshot and complete immutable per-stream
metadata while keeping the supervision types portable.

Independent Task1, lifecycle/progress, backend/request and receipt reviews found
no remaining concrete scoped issue after fixes. Review found the terminal
interval defect; its frozen and recovered-counter cases failed genuinely
before the fix. Hardware-free tests exercise the same controller/tracker used
by the native adapter; they do not execute COM, native endpoints or the driver.
No Miri, app suite or remote CI execution is claimed for this example-only
slice. Audio test totals are regression coverage, not separate native or
allocation-only acceptance tests.

Source is committed locally as `f83f3a5`; receipt/ledger/plan follow in a
separate documentation commit. No push, VM access, driver install/deployment
or native audio execution occurred. Unrelated `gate.log` remains untouched.
Remote checkpoints remain batched rather than one push per atomic commit.

External changing signal, wiring, common pre-route reference and physical
boundary are caller responsibilities and remain unverified. No independently
calibrated acquired waveform, physical delay distribution, lower native preset
or matched VAD/Process A/B result follows from this software slice.
The [S1-S3 ledger](2026-10-29-s1-s3-software-ledger.md) retains those gates.
