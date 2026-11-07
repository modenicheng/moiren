# Matched waveform attempt: recorder readiness failure

Date: 2026-11-03, Asia/Shanghai. Baseline: `9d7a836` on
`feat/minimal-io-latency`. This slice separates one finite remote acquisition
from parallel offline-tool development. It is not another stability soak,
an audio-engine regression result, or an S1-S3 qualification PASS.

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-matched-shared-waveform-20261103-0430`.

Remote root:
`D:\MoirenTest\20261103-matched-shared-waveform-0430`.

## Intended question and frozen conditions

The new question is whether short changing waveforms correspond for sequential
Process and VAD playback routes when both owned sources actually use Shared480,
48k stereo F32 and native capacity1056. Both graphs request the same DAWN
physical render endpoint, Shared336, queue target512, wake/callback64 and
Engine F32/block64, with unchanged Compatible input budgets. This is not a
buffer sweep, lower-default proposal or calibrated physical measurement.

The six-code indexed source schedule is unchanged:1024-frame codes at
frames12000/60000/108000/156000/204000/252000 in288000 source frames. The planned
return boundary is isolated OBS WASAPI output capture, not a DAC/physical
measurement. Process reference capture and raw VAD playback input are different
source boundaries and must not be relabeled as identical clocks.

The frozen release probe is the clean `50c97e5` binary, SHA256
`6290C3878A6136007DC4551E9DBFCBDF4E8AF92C91D2EFE0467EF58E7F703EDB`.
The clean application package is the one documented in
`2026-11-03-process-source-period-and-app-handoff.md`; no new application or
driver version is deployed in this slice.

## Preflight and isolation

The launcher uses one Limited Interactive session1 scheduled task, a420s outer
bound and125s per-case bound. First-route acquisition or cleanup failure stops
the pair. There is no automatic retry or second-route launch after failure.
Only exact held child handles may be terminated; external OBS12648 and
QQMusic15096 retain their recorded creation identities.

A new isolated Python environment contains the copied websocket dependency;
no global packages are installed. Its33-file non-pyc fingerprint is
`CD24FA8C556890903C8D023CFB6F681E04DC0281631FAC3437D08F4893B5CC95`.
Windows venv launches retain a waiting Python parent. A non-audio identity
inspection confirms this, and preflight excludes only the exact parent PID,
executable and complete argv, not unrelated Python owners. Six local pure
runner checks and PS5/PS7 entrypoint parsing pass. The six-check local receipt
is `runner-tests-local-six.log`; the earlier archived remote test file/log
contains five checks and is not relabeled as six. Independent bounded review
finds no remaining prelaunch blocker.

The original selfcheck remains retained. The final selfcheck finishes Ready/0,
complete=true, native_launched=false with the exact deployment intent SHA256
`23642C75AD7EB87ED83B22A92285A93AAECF9C116480AD5066A4EAB1D7A10A93`.
The actual acquisition runner SHA256 is
`C6664B257F51734B5A9AB025F613D2EC9D45E7E6F7121B9ED233811BA7D4901B`.

## Actual result: no audio source was started

The one native task starts04:46:33. Its Process case runs04:47:11.721 through
04:47:43.481. It creates only the isolated portable OBS child15164; it never
creates the source worker, application monitor, Process recorder or VAD route.

The runner sends `StartRecord`, then immediately asserts
`GetRecordStatus.outputActive`. The first status is false, so acquisition
stops. The OBS log subsequently records DAWN WASAPI initialization at
04:47:31.622 and Recording Start at04:47:31.819. The observed fault is the
runner treating an asynchronous recording transition as immediate readiness;
it does not establish a Moiren audio-engine or driver regression.

The predecessor also sets its cleanup recording flag only after that assertion.
Consequently the in-flight recording is not stopped by its error branch.
Normal WM_CLOSE does not finish within12s, and the exact held OBS child is
terminated, exit1. Its5659135-byte partial MKV remains remote and is not valid
waveform, continuity, marker-coverage or latency evidence. It is not decoded
or plotted as if it contained the intended source.

The task finishes with result1 and complete=false. No second route or retry
occurs. Final audit has no test audio owners, no audit_error, unchanged driver
hash/boot, unchanged protected process identities, unchanged default roles,
and unchanged render volume/mute/mix-format settings. Only the preexisting
external OBS remains. No host native audio/UI is launched or manipulated.

## Parallel development and verification

While the finite remote acquisition runs, offline analysis support is completed
without changing production audio code. It consumes actual retained recording
paths, validates source-reference schemas, retains absent/invalid evidence,
and supports a single retained route without inventing the missing route.
The unchanged nominal Rust full-six-code detector and thresholds
`.7/.05/.0001` remain authoritative. Nominal refusals are not lost-frame counts;
no SRC model, packet timestamps, identity-by-order or latency is manufactured.

The actual isolated remote analysis Python passes22synthetic tests, exit0,
including plot construction using synthetic inputs. Its executable hash and
numpy/scipy/matplotlib package ancestry are verified. This is not real waveform
analysis; no new waveform figure is published for the failed acquisition.
Frozen analysis SHA256 values are:

- `analysis.py`: `A9281241136337B0F256DA496216629F0B8A17741227727EA0E6DAD8B1199EED`.
- `analysis.tests.py`: `3ECCE3B5EAB12986944714A02469151533E8F58264DFB53D26B4E67ABC5691C5`.

A separate evidence-only `run-next.py` corrects recording lifecycle ownership:
claim responsibility before dispatch, wait boundedly for active, observe an
in-flight start during error cleanup, dispatch Stop only once, observe inactive,
then close the owned OBS window/socket. Unresolved transitions invalidate the
attempt and fall back only to held-child cleanup. The deployed/pinned runner
and failed evidence are not overwritten. Synthetic regressions demonstrate
RED before the correction and13passing checks afterward. This correction has
not been natively validated or launched as a second acquisition.
The corrected runner/tests are pinned in `local-final-receipt.json`; their
SHA256 values are respectively
`A754A1947C9B8B8E1A4607D4274C720A5E9565A793CAF2EE28ED9416582F6F23`
and `1DBEBF26F1A0DCE320B0059B5869E7914217C052AADA91FDF8CD84A6346B3075`.

## Retention and remaining work

All33compact metadata/script/log files are individually length/SHA256 checked
after transfer. The50429-byte archive SHA256 is
`53DC3B11B0B4C63A3BD2A666CD116840A4212D7C92A5CB933F1FCD18D89CFF70`.
Raw PCM, scalar streams and the partial MKV are not transferred. All27original
dirty/cache file hashes remain unchanged and outside build/commit scope.

S2/S3 software status is unchanged from the preceding audit; this harness
failure does not justify speculative production fixes or enlarged buffers.
First-packet work remains deferred. S1 matched waveform, retained desktop
acceptance and calibrated physical qualification remain open; none is claimed
from this attempt. Lack of multichannel physical capture hardware does not
block independent application development. Future finite remote long tests
may run in parallel only for an identified new question, with isolated owners
and bounded termination, rather than serializing development behind repeated
unchanged stability scans.
