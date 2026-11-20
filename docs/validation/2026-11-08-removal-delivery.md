# Closed-run delivery with independently verified target acknowledgment

Date: 2026-11-08, Asia/Shanghai. Base: `dcda8cc`. This follows the
[closed-run exporter](2026-11-08-removal-closed-export.md). Full S0-S5 acceptance
remains open; the canonical verdict is [`driver/vad/STATUS.md`](../../driver/vad/STATUS.md).

## Implemented path

The actual host entry `scripts/vad-delivery.py` now connects the closed-run
artifacts to the real retained SSH transport and durable target finalizer:

1. Freeze the original canonical target run and independently supplied local
   acknowledgment SHA256. Missing explicit user reboot confirmation refuses
   before target access or output creation.
2. Create a new host evidence directory. Download original
   `*.archive-local.json`, then `*.zip.inventory.json`, then `*.zip`, exactly
   once each. Verify the old ack before the second request and the pinned
   inventory before the archive request. Never follow paths from a downloaded
   document, extract an archive, overwrite/resume a file or retry a failed phase.
3. Independently stream and verify archive length/SHA256 and every member,
   including empty files/directories. Require exact membership, depth/member/
   size bounds and exact embedded/sidecar inventory bytes. The inventory lists
   all members except itself; add that one self entry from its raw bytes.
4. Match nonce, original result bytes, result/input/manifest pins, named source
   snapshot and in-run terminal/staged pins. Source pins outside the run remain
   provenance, never external files to reopen. Keep failed diagnostics unsafe
   and unqualified. Produce durable `host-delivery.json` with
   `Meaning=HOST_ARCHIVE_DELIVERY_VERIFIED`.
5. Upload exactly that receipt into `*.delivery-host.json` with create-new and
   length/hash readback. Before any upload, hash-check the independently pinned
   `delivery-finalize.ps1` and both same-directory dependencies,
   `closed-export.ps1` and `finite-tool.ps1`. Keep their read-sharing-only
   handles through dot-sourcing and finalization.
6. The target rechecks the separately pinned host receipt and original local ack,
   whole run/result/inventory/archive/provenance chain, then creates
   `*.archive-delivered.json`. It reconstructs no private controller/export CWT.
7. Download only the fixed original-run target-ack path returned by finalization.
   Verify its exact pin, original nonce/pins and literal host receipt JSON.
   Lock/recheck all five original host evidence files through create-new,
   fsync and readback of `delivery-complete.json`.

The historical local ack retains `Transferred=false`. The host and target
delivery receipts use `Transferred=true`, but `NativePassed=false` and
`Qualified=false` throughout. Completion explicitly keeps
`CleanupAuthorized=false`. A delivery acknowledgment proves neither native
removal nor authorization to delete evidence. These archives may contain
bounded PCM/WAV/ETL, not only metadata.

## Retained transport and file lifetime

`scripts/vad-delivery-transport.py` pins SSH to `moiren-lan` with batch mode,
strict host-key checking, ten-second connection timeout and bounded keepalive.
The remote script checks exact target hostname/SID and literal non-reparse paths.
It copies raw binary stdout into the original exclusive create-new host file;
stdout and stderr are independently capped while their excess is drained.

The original Popen and stdout/stderr/input owners survive deadline, observation,
pump construction/start, read/write and flush failures. Known undispatched
thread setup can finish on the same process and same constructed threads; an
ambiguous dispatched thread is never duplicated. Deadline/failure is sticky
even if the original child later exits successfully. No kill, subprocess retry,
replacement PID or lost-reply-as-absence path is introduced. The host entry
drains an exposed pending owner before rethrowing the original failure.

Windows host verification uses GENERIC_READ / FILE_SHARE_READ / OPEN_EXISTING,
one HANDLE-to-CRT-fd ownership transfer and path `(device,inode)` identity checks.
Readers remain held through receipt persistence/readback, excluding compatible
new writers/deletion during that verified lifetime. This does not claim hostile
arbitrary-code protection or durable restart/crash recovery.

## Verification scope and retained evidence

Evidence root: `D:\coding\moiren-evidence\20261108-delivery`.

- Host verifier/flow: 17 real-file tests pass, covering every-member ZIP content,
  independently pinned ack, canonical run, duplicate/typed JSON, bad inventory,
  corrupt/extra/case/traversal ZIP entries, unchanged qualification flags,
  named source pins, missing confirmation, download order, sticky pending
  owners, foreign final response and actual Windows writer exclusion.
- Transport: the parent rerun passes all 33 tests, including harmless original
  processes, binary/Unicode pipes, late exit, transient poll/read, sink failure,
  cap/drain and thread constructor/start/lost-start outcomes. Generated actual
  PowerShell dependency checks run locally under both 5.1 and 7. There is no SSH
  connection or target operation in these tests.
- Target durable finalizer: the parent tests pass 12 cases under each of
  PowerShell 5.1 and 7, replacing only the exact target guard for host fixtures.
  Malformed/duplicate JSON, manifest/source snapshot changes, altered original
  files/ZIP membership and overwrite/retry attempts refuse.
- End-to-end host composition: both interpreters pass three cases. Production
  Finish/decoder settlement, result persistence, private seal, real exporter,
  host `deliver_run`, separate actual finalizer process and final target-ack
  download/readback compose for normal and refused offline association. Native
  scheduler/trace/process metadata remain existing injected fixtures; transport
  is an ordered bounded local byte copy. Native release is still checked closed.
- Production source membership grows from 27 to 28 with the required durable
  finalizer; focused tests cover its omission. CI lists 40 preparation suites in
  both PowerShell engines plus both Python delivery suites. The final frozen
  matrix completes 80/80 suite processes, 484 checks per interpreter, 968 in
  total with zero failures. All 77 script/workflow pins remain unchanged and
  every per-suite output retains its own length/hash receipt. The 29-case
  offline reader, 58-case CI surface mapping, 72-script parse, YAML, fmt and
  staged whitespace checks pass separately. Original 29 dirty draft/cache pins
  in the old worktree also verify unchanged. Final receipts and pins are kept
  separately under the evidence root; these software counts are not native gates.

The first host run fails because implementation is absent. Later retained
RED/GREEN cases catch the actual named-source shape, discarded pending owner,
writer-sharing and path-identity holes. Initial composition adapter parse/path
errors are retained as failed test setup, never native failures or passes.
Transport and target JSON/manifest/source/thread faults retain their separate
agent RED/GREEN evidence. Independent source review reports CODE CLEAR for
host `C5F5FCEC950CCD56F218D62209450B4919A40E5C0642DE383263DB4007D893B4`,
transport `2B461AED533A46E78754A5D0D63FE3628FA34CE4D2EC72596350551120437AAF`,
and target `8A43212276457A7F3D73F9FC65E80632F126DFF8508853F32AF3DEE714D0F2CE`.

Exact predecessor `dcda8cc` CI `42472082524` completes all 20 jobs successfully;
its original watcher `73572` exits 0. That CI verifies the predecessor, not this
new slice. No future published-head CI result is inferred from it.

## Remaining user/native gates

No remote transfer, native audio/OBS/driver/device/scheduler action or reboot
was performed for this implementation. The user has not confirmed reboot after
the signed613 deployment returned3010. Fresh boot/package/signature/SYS/PnP
provenance and reviewed real target execution remain necessary; the executable
native release guard remains unconditionally closed.

The next production handoff is concrete, not another generic guard layer:
generate/pin the actual target run manifest, hash/hold the entry and complete
dependencies before its first import in an already-elevated single-launch
wrapper, then persist the original entry terminal/run/local-ack pin and pass
that independent SHA into `deliver_run`. Closed `Safe=false` diagnostics must
still deliver their failure evidence; exit0 alone cannot authorize retirement
or cleanup. The current controller's internal stages are connected, but these
outer production handoffs are not yet provided by the delivery-only CLI.

Use the standalone target directly, not a mandatory VM or blanket unlocked
desktop. Reuse the fixed dedicated OBS installation without per-run EXE trees,
preserve ordinary clients and security, and delete only owned unused artifacts
after verified archival under separate cleanup authority. The earlier seven
exporter fixture roots remain undeleted after tool-policy refusal; this slice
does not misreport that retained storage as reclaimed or bypass the refusal.

This turn also archives 59 owned host-only fixture roots into
`owned-delivery-fixtures.zip`: 18,870 members, 209,422,547 uncompressed bytes,
16,823,562 archive bytes, SHA256
`39CCCD54AB26A9394ABA95B7E932336385188FE05DFABE516F5210886F7E47FC`.
Every member is independently read back; one reparse entry is recorded without
following it. Exact-root native PowerShell deletion is rejected by tool policy
before process creation. None of these 59 roots is deleted; there is no
alternate-shell/language bypass or reclaimed-storage claim. Final source/docs
and receipts receive a separate create-new checkpoint after this outcome.

Direct Removed, Windows-name/default/settings retention, streaming/held-client/
race/A -> B -> A cases, matched waveform and calibrated sustained p95/p99 below
10ms (or the evidenced target-device fallback) remain unqualified. The full
goal is not completed by this software-delivery checkpoint.
