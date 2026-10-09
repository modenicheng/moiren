# M0.6 Audio Host verification record

This record distinguishes implemented contracts, deterministic regression evidence and hardware evidence. The milestone is complete only after all six acceptance requirements have evidence. Original frontend development remains in the main checkout.

## CI regression diagnosis

The failed run [37898563420](https://github.com/modenicheng/moiren/actions/runs/37898563420), commit `c581bbf`, failed two Compressor tests under Tree Borrows. The log reports assertion failures, rather than an undefined-behavior diagnostic:

- `compressor_hold_delays_release_but_never_attack_and_persists_between_windows`: sample examples differ by about `1e-16` between whole/split processing.
- `compressor_silence_remains_finite_and_matches_variable_blocks`: the sample `0.5194523896757088` is compared with `0.5194523896757085` using exact equality.

The fix checks every sample and exact vector shape, using a tolerance of `32 * f64::EPSILON * max(1, |actual|, |expected|)`. Nonfinite differences still fail. Existing independent envelope/hold/silence expectations remain in the tests. No DSP behavior or aliasing checks have been removed.

The same log shows 1,127 seconds for the full `u16` Bus port fixture and 3,271 seconds for graph integration tests under Tree Borrows. The native 65,536-port integration fixture remains intact. Miri now checks the actual production count guard at its exact lower/upper boundaries without constructing 65,536 ports, and keeps all small Bus edit tests. Deep-chain Miri tests use 32 nodes; native stress still uses 512 nodes. Both exercise iterative ordering, reconnection and cycle rejection.

Native verification after this change: locked `moiren-core` and `moiren-engine` full tests and all-target strict Clippy passed on Windows. Local Miri verification is still in progress; no Miri pass is claimed here.

## Continuous backend ownership

`RenderOptions::continuous`, `CaptureOptions::continuous` and `ProcessLoopbackOptions::continuous` explicitly support running until stopped. Other durations still require the existing 1..600 second interval.

The render worker now returns its engine and output reader after its COM scope has ended. `join_with_renderer` returns these resources to control; the compatibility `join` and session Drop destroy them on the caller. Hardware-independent tests cover stopped and panicking native owner scopes, actual invalid-endpoint COM startup failure, and caller-thread processor destruction.

The render observer publishes atomic timeline/counter snapshots. The render allocation test includes an attached observer and verifies zero allocations and deallocations. Attachment has exclusive publication ownership, preventing two renderers from interleaving writes into one observer.

## Acceptance ledger

| Requirement | Evidence required | Current state |
| --- | --- | --- |
| Continuous single render / Gain/Pan | Real device session with online parameter receipts | Backend lifetime and ownership tests passed; Host integration pending |
| Real plan swap / control reclamation | Device session retains its output bridge across compressor insertion/removal | Existing engine swap regression passed; Host integration pending |
| Physical and process inputs together | Two concurrent independent capture owners feeding the Bus | Pending |
| Source failure isolation and replacement | Source exit/stop/restart while another input and output continue | Pending |
| Cold start / drift / stop / reconfiguration | Deterministic regressions, fixed Miri checks and recorded device stress | CI failure diagnosed and native fix validated; additional evidence pending |
| UI control contract | Graph/runtime/parameter/revision/device/diagnostic API and tests | Domain Host implementation pending |

Hardware measurements will record aggregate counts, durations and reproduction conditions. PCM and stable device IDs are excluded from committed evidence. Device disappearance, sleep and external application matrices must be reported separately from scenarios actually performed.
