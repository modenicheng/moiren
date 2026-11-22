# VAD format publication query, 2026-11-02

## Question and boundary

The preceding [input-reference acquisition](2026-11-02-vad-input-reference.md)
stopped before Start: both exact exclusive Float32 and PCM16 candidates on the
selected Playback endpoint returned `0x88890008`. MIC format refusal is not
evidenced by that attempt. No input reference was acquired.
This diagnostic asks a different, narrower question: which exact support
results are visible while a freshly created endpoint publishes?

One throwaway, evidence-only Rust example was added to a clean `5e2cada`
archive. It used the selected control root and exact adapter/stable identities,
created its own stereo Playback and MIC exports, and queried fresh
`IAudioClient` instances at scheduled offsets 0, 100, 500, 2000 and 5000 ms.
It never called Initialize or Start, prepared an Engine, or submitted PCM.
Mix-format and period queries followed the exact support queries and are
ancillary observations, not initialized-stream measurements.

The example is not a production change or a delay/retry policy. Its first
compile errors were corrected before execution; they are not behavioral RED
evidence. Build and strict Clippy passed. An independent safety review found
no blocker before the single remote invocation.

## Pinned execution

- Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-vad-format-investigation-20261102-2035`.
- Remote root: `D:\MoirenTest\20261102-format-publication-2040` on `LAPTOP-NP49RG6H`.
- Binary SHA-256: `BD85A23E6C07BA3968345C3F88A692E77E59A7803B0606C5D662C42F2207CE3B`.
- Diagnostic source SHA-256: `BEBB90A1CC0202D8595E146D643238C03C75D752F5FD9D1D2CAE00D5C8FB860D`.
- Loaded SYS SHA-256: `4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.
- Execution: 2026-11-02 20:42:37.386 to 20:42:43.911 +08:00; normal exit 0, 6.520 seconds; no timeout or forced termination.

`exact-export.json` verifies the 1360 baseline files, with only the added
example and its manifest registration changed. `remote-manifest.json` pins
all five transferred report/audit/owner-log files; their lengths and hashes
were individually verified. PCM was not acquired, so there is no waveform
analysis or latency plot from this diagnostic.

## Observations

| Scheduled Offset | Playback | MIC |
| --- | --- | --- |
| 0, 100, 500 ms | No exact matching ACTIVE endpoint | No exact matching ACTIVE endpoint |
| 2000 ms | Both exact exclusive formats S_OK | Both exact exclusive formats S_OK |
| 5000 ms | Both exact exclusive formats S_OK | Both exact exclusive formats S_OK |

At the first published observation, actual Playback query work spans roughly
2018-2189 ms and MIC work 2189-2252 ms. Identity rechecks match. Both ancillary
mix descriptors are 48 kHz stereo, 32-bit extensible; their subtype was not
recorded. Default period is 100000 hns and minimum is 30000 hns. Neither period
is this diagnostic's running stream period.

Before publication the control snapshots show stream generation 0/gate 1.
By the first published observation they show Playback generation 5/gate 16
and MIC generation 4/gate 13, still Closed. These are observed external Windows
preparation transitions, not Initialize/Start calls made by the diagnostic.
They do not establish why the earlier active-device query was refused.

Both owned exports reach Removed/Closed. Final audit has no test owners and
only the bare control bus. Protected OBS/QQMusic PID-plus-creation identities,
loaded driver and boot remain unchanged. The read-only registry/event snapshot
contains post-removal cached endpoint properties and unrelated events; it is
not treated as causal evidence.

## Verdict

The earlier active-device format refusal is **not reproduced**. No causal
fix, automatic retry, new publication wait, format fallback, or production
policy follows from this one query-only experiment. It does not prove audio
correctness, native collection, stability or physical/sub-10-ms qualification.
Independent S1-S3 development continues; first-packet investigation stays
deferred.
