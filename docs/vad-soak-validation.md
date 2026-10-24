# VAD audit follow-up and continuous target tests

These are opt-in installed-driver tests. Ordinary Cargo tests do not open audio
devices, install drivers, change defaults, or begin a soak.

## Removal audit

The current `driver/vad/src/Lifecycle.h::Revoke` accepts `Creating`, `Present`
and `Removing`. Repeating REMOVE on a retained `Removing` child reaches
`MarkMissing` again. A failed `AcxDeviceRemoveCircuitDevice` leaves
`CircuitDeviceAdded` set and its status is returned to the caller. Successful
submission clears that flag; it does not establish final PDO destruction.
The slot remains unavailable for reuse until the final lifetime callback.

The reported "Removing always returns success without retry" is not present
in this revision. Test the real removal functions with an injected ACX failure,
as well as the lifecycle helpers, to protect this distinction.

## Control access boundary

The control bus deliberately permits authenticated standard users to open one
exclusive owning file object. This satisfies the requirement for normal use
without elevation or a resident privileged service. It is not an application
identity check: another permitted local process can acquire the idle bus first.
No process name, executable path, or embedded user-space secret establishes a
reliable security boundary between applications running with the same rights.

The relevant guarantees are narrower: a refused second open does not revoke
the current owner; management and PCM requests require the owning file object;
named suffix opens are rejected; the application handle is not inheritable;
closing its last handle revokes admission, cancels pending transfers, and
requests removal. Session and endpoint generations reject late requests.
This design does not isolate mutually untrusted local users who share the
single control bus. Changing that boundary needs an explicit product decision,
such as a separately provisioned permitted-user group.

## Continuous App soak

Build the opt-in probe from a reviewed, committed source state:

```powershell
cargo build --locked --release -p moiren-app --features vad-integration-probe --example vad_integration
Get-FileHash target/release/examples/vad_integration.exe -Algorithm SHA256
git rev-parse HEAD
```

Copy that executable and `scripts/vad-soak.ps1` into the dedicated test guest.
Record the exact binary hash and source commit. Run the script inside the guest:

```powershell
.\vad-soak.ps1 -Executable C:\MoirenVadTest\vad_integration.exe `
    -ExpectedSha256 <SHA256> -SourceCommit <COMMIT> `
    -OutputDirectory C:\MoirenVadTest\soak-unique-tag `
    -IdleSeconds 28800 -PcmSeconds 28800
```

First rehearse with short phase durations and a new evidence directory.
The runner checks the expected guest name, preserves existing evidence, refuses
another known probe/application process, verifies the executable digest, and
disables AC/DC sleep and hibernation before launch. It neither installs a driver
nor changes audio defaults. Each phase lasts continuously within one owning App
session; repeating short processes is not equivalent.

Keep `running.json`, `resources.jsonl`, `stdout.log`, `stderr.log`, the probe's
periodic progress, its final `report.json`, and the runner's `result.json`.
The resource log is streamed and flushed; the runner retains no sample array
that grows with the requested duration. It samples the probe's CPU, memory,
handles and threads, with a guest pool snapshot about once a minute using
`GetPerformanceInfo`. PnP enumeration runs separately from the supervisor so
that it cannot block cancellation or the outer deadline. System pool counters
include other drivers and do not identify a Moiren leak.
Resource-growth qualification requires reviewing the samples after completion;
`resource_growth_qualified` remains false in the runner result.

Use `Start-Process -WindowStyle Hidden` when starting an unattended runner.
Capture its PID and creation time, and leave the guest powered on. The probe has
an aggregate deadline including cleanup; the runner adds an outer margin.
An SSH disconnect alone does not prove that the process stopped or finished.
Inspect the durable progress and process identity on reconnect.

Creating the `cancel` file in the run's output directory makes the runner
terminate its retained probe process. Cancellation and deadline termination are
recorded as failures to complete; they do not prove orderly native cleanup.
Never stop a process by a reused PID or by a broad name pattern.

A started run is not a PASS. A successful short rehearsal does not qualify an
eight-hour run. Only completed phase durations and actual PCM/timeline evidence
count. Review discontinuities, silent frames, driver counters, queue behavior
and resource trends independently. Stereo Float32 exclusive event-mode App
evidence does not cover PCM16, mono, timer/shared mode, physical I/O, default
eligibility, software volume/mute, power transitions, Driver Verifier, or
production signing.
