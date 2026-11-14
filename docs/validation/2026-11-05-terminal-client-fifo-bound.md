# Frozen Terminal Client-FIFO Bound

Date: 2026-11-05, Asia/Shanghai. This is a local analysis of immutable native
operation metadata, not a new audio acquisition, waveform fit or physical test.
The original delivery-gap failures and waveform residuals remain failures.

## Provenance And Scope

The planned foreground remote recorder/terminal audit refuses before directory
creation at 01:38:12: SSH has the expected SID but an administrator token.
A read-only inspection confirms session0/admin and the absence of the package.
The non-admin guard stays unchanged. No remote audit script or ffprobe executes;
there is no fresh remote safety attestation, recorder PTS result or complete
remote audit receipt.

An independent SFTP-only archive at 01:47:59-01:48:00 reads just eight existing
`stream.json`, `final.json`, `packets.jsonl` and `timeline.jsonl` files. The two
owner directory identities remain distinct. All eight files, totaling 914870
bytes, match the previously frozen manifest's exact lengths/SHA-256. There is
no remote shell execution, task, audio/device/UI operation, remote file write,
raw PCM transfer, refit or alignment change in this archive.

Host evidence root:
`D:\coding\moiren-evidence\20261104-pipeline-resume`.

- `terminal-recorder-audit/input-manifest.json`: original remote pins and ledger
  expectations, unchanged.
- `terminal-fifo-metadata-20261105-0150/transfer-receipt.json`: successful SFTP
  archive and all eight matching pins.
- `analyze-terminal-fifo-20261105.py`: local sub-analysis, not the remote runner
  or complete-audit CLI. It preserves the full-interval closure checks.
- `terminal-fifo-metadata-20261105-0150/client-fifo-summary.json`: result SHA-256
  `D68C9996821874F17570E4420FD3F595ECC117DE14AD9F02EB483DD9D6AAD857`.

The original association/locator/monitor/native-result authority files match
their frozen pins. Three production source files match commit `002aa21`.
Nineteen pure accounting/timestamp tests cover the conservative analyzer.
An independent reviewer traverses the actual full packet/event intervals and
recomputes the bounds separately; no actionable discrepancy is found.

## Bound And Actual Full-Interval Checks

For one successful same-client Write, let C be accepted frames before that
Write and p its actual pre-release GetCurrentPadding observation. At least
C-p frames have already left this client's FIFO. A subsequent successful
submission of w frames leaves at most p+w queued. Later native consumption
can only reduce that pending bound. Stop is not assumed to drain.

This conclusion requires a single closed Start..Stop interval, contiguous
packet/event sequences, exact accepted frontiers, successful releases,
source-derived target enforcement and no reset, missing submission, archive
loss, pending writer or unknown padding. Those checks pass for both frozen
owners. Elapsed QPC time is not converted into extra consumed frames.

| Frozen owner | Packets / events | Accepted frames | Last pre-query p / submission w | Guaranteed FIFO-departed prefix | Possibly still queued upper bound |
| --- | ---: | ---: | ---: | --- | ---: |
| Baseline | 290 / 582 | 97616 | 176 / 336 | [0, 97104) | 512 |
| Replacement | 1147 / 2296 | 385568 | 176 / 336 | [0, 385056) | 512 |

These are actual padding/submission bounds, not a nominal target-only argument.
Both intervals contain one Start and one successful Stop; final persisted
packet/event counts match with no missing, dropped or unobserved entries.

## Frozen Residual Consequence

No individual sample is read or reclassified. Original block counts alone
bound how many existing residual-labelled frames lie in those fixed prefixes.

| Frozen owner | Original residual total | Residual count in guaranteed-departed prefix | Residual count possibly in remaining client queue |
| --- | ---: | --- | ---: |
| Baseline | 2827 | Exactly 2315 in [0, 97104) | At most 512 |
| Replacement | 689 | Between 177 and 689 in [0, 385056) | At most 512 |

The baseline exact count refers only to that fixed guaranteed prefix; it is
not the total or upper bound of all residual frames that might actually have
left the FIFO later. For the replacement, at least 177 original residual frames
cannot be explained solely as remaining in this client's Stop-tail queue.
That rules out one blanket explanation, not a production root cause.

The original baseline early/tail split remains 2042/785. Both original
delivery-gap violations remain, with maximum 111165 * 100 ns. The 48000 unknown
frames in each owner remain unknown and are not promoted into matching audio.

## Not Proven And Next Evidence

- Client-FIFO departure is not Windows mixer/endpoint/DAC presentation.
- This does not identify missing, dropped, duplicated or inaudible samples.
- Original residual totals remain 2827/689; no waveform quality gate passes.
- Recorder packet PTS/duration remains unread; no recorder-continuity claim or
  corrected alignment is produced. A metadata timestamp mismatch would not
  by itself prove audio loss either.
- Native faults, full changing-waveform correspondence, matched route timing,
  sustained load and calibrated physical acceptance remain open.

The next useful discriminator is the separately reviewed, actually non-admin
foreground recorder-metadata audit or another source-supported explanation of
the existing frozen residuals. Do not rerun unchanged audio for luck, weaken a
gate, enlarge a buffer or reinterpret FIFO departure as a physical pass.
