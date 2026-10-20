// SPDX-License-Identifier: MIT
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <initializer_list>

// Only DDI boundaries are stubbed. The runner extracts the production bodies
// on every build; this does not model WDF/ACX concurrency or prove target IRQLs.
using ULONG = std::uint32_t;
using LONG = std::int32_t;
using NTSTATUS = std::int32_t;
constexpr unsigned PASSIVE_LEVEL = 0;
constexpr unsigned APC_LEVEL = 1;
constexpr unsigned DISPATCH_LEVEL = 2;
constexpr unsigned IRP_MJ_CLEANUP = 18;
constexpr unsigned IRP_MJ_CLOSE = 2;
constexpr int TRACE_LEVEL_INFORMATION = 4;
constexpr int TRACE_LEVEL_ERROR = 2;
constexpr int VAD_SESSION = 1;

struct Root;
struct File;
struct WdmFile {
    File* Framework = nullptr;
};
struct File {
    explicit File(Root* device) : Device(device), Wdm{this} {}
    Root* Device;
    WdmFile Wdm;
};
using WDFDEVICE = Root*;
using WDFFILEOBJECT = File*;
using PFILE_OBJECT = WdmFile*;
struct Stack {
    unsigned MajorFunction;
    PFILE_OBJECT FileObject;
};
struct Irp {
    Stack Current;
    bool Forwarded = false;
    NTSTATUS Status = -91;
};
using PIRP = Irp*;
struct Root {
    Root* Admin = this;
    File* Owner = nullptr;
    std::uint64_t Session = 7;
    volatile LONG Releasing = 0;
    volatile LONG Online = 1;
    Root* ControlQueue = this;
    Root* ControlWork = this;
    bool Locked = false;
    bool CallbackOnForward = false;
    bool OfflineOnFlush = false;
    unsigned Revokes = 0;
    unsigned Purges = 0;
    unsigned Flushes = 0;
    unsigned Starts = 0;
    unsigned Forwards = 0;
    unsigned ClientCallbacks = 0;
};
static unsigned CurrentIrql = PASSIVE_LEVEL;
static unsigned MapCalls = 0;

static void Check(bool condition, const char* message)
{
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        std::exit(1);
    }
}

#define NT_ASSERT(condition) Check((condition), "kernel assertion")
#define PAGED_CODE() Check(CurrentIrql == PASSIVE_LEVEL, "pageable cleanup requires passive")

unsigned KeGetCurrentIrql() { return CurrentIrql; }
template<typename... Args>
void VadTrace(int, int, const char*, Args...) {}
Root* VadRoot(Root* root) { return root; }
Root* WdfFileObjectGetDevice(File* file) { return file->Device; }
WdmFile* WdfFileObjectWdmGetFileObject(File* file) { return &file->Wdm; }
File* WdfDeviceGetFileObject(Root* root, WdmFile* file)
{
    ++MapCalls;
    Check(CurrentIrql == PASSIVE_LEVEL, "raw cleanup maps only at passive");
    return file != nullptr && file->Framework != nullptr && file->Framework->Device == root
        ? file->Framework : nullptr;
}
LONG InterlockedExchange(volatile LONG* value, LONG next)
{
    const auto previous = *value;
    *value = next;
    return previous;
}
LONG InterlockedCompareExchange(volatile LONG* value, LONG next, LONG expected)
{
    const auto previous = *value;
    if (previous == expected) {
        *value = next;
    }
    return previous;
}
void WdfWaitLockAcquire(Root* root, void*)
{
    Check(CurrentIrql == PASSIVE_LEVEL, "Admin wait requires passive");
    Check(!root->Locked, "Admin cannot recurse");
    root->Locked = true;
}
void WdfWaitLockRelease(Root* root)
{
    Check(root->Locked, "Admin must be held");
    root->Locked = false;
}
NTSTATUS VadRevokeSession(Root* root)
{
    Check(root->Locked && root->Owner == nullptr && root->Releasing == 1,
        "close admission before revoking under Admin");
    ++root->Revokes;
    // Removal submission failure must not retain ownership or skip draining.
    return -37;
}
void WdfIoQueuePurgeSynchronously(Root* root)
{
    Check(!root->Locked && root->Owner == nullptr && root->Releasing == 1,
        "purge outside Admin while owner admission is blocked");
    Check(root->Revokes == root->Purges + 1, "each revoke purges exactly once");
    ++root->Purges;
}
void WdfWorkItemFlush(Root* root)
{
    Check(!root->Locked && root->Releasing == 1 && root->Purges == root->Flushes + 1,
        "flush follows purge outside Admin while admission stays blocked");
    ++root->Flushes;
    if (root->OfflineOnFlush) {
        root->Online = 0;
    }
}
void WdfIoQueueStart(Root* root)
{
    Check(!root->Locked && root->Releasing == 1 && root->Online == 1 &&
        root->Flushes == root->Purges && root->Flushes != 0,
        "restart only after drain, while online and before lifting admission latch");
    ++root->Starts;
}
Stack* IoGetCurrentIrpStackLocation(PIRP irp) { return &irp->Current; }
void IoSkipCurrentIrpStackLocation(PIRP irp)
{
    Check(!irp->Forwarded, "IRP skip exactly once");
    irp->Forwarded = true;
}
void VadFileCleanup(WDFFILEOBJECT file);
NTSTATUS WdfDeviceWdmDispatchPreprocessedIrp(Root* root, PIRP irp)
{
    Check(irp->Forwarded && !root->Locked, "forward once outside Admin");
    ++root->Forwards;
    if (root->CallbackOnForward && irp->Current.MajorFunction == IRP_MJ_CLEANUP) {
        ++root->ClientCallbacks;
        VadFileCleanup(irp->Current.FileObject->Framework);
    }
    return irp->Status;
}

#include "cleanup-production.inc"

static void RawCleanupWithoutClientCallback()
{
    Root root;
    File owner(&root);
    root.Owner = &owner;
    Irp cleanup{{IRP_MJ_CLEANUP, &owner.Wdm}};
    Check(TraceFileIrp(&root, &cleanup) == -91, "preserve downstream cleanup status");
    Check(root.Owner == nullptr, "raw CLEANUP revokes when client callback is skipped");
    Check(root.Releasing == 0 && root.Revokes == 1 && root.Purges == 1 &&
        root.Flushes == 1 && root.Starts == 1 && root.Forwards == 1 && root.ClientCallbacks == 0,
        "one complete revoke/drain despite failed removal and absent client callback");
}

static void ClientCleanupStillWorks()
{
    Root root;
    File owner(&root);
    root.Owner = &owner;
    VadFileCleanup(&owner);
    Check(root.Owner == nullptr && root.Releasing == 0 && root.Revokes == 1 &&
        root.Purges == 1 && root.Flushes == 1 && root.Starts == 1 && root.Forwards == 0,
        "client cleanup independently retains the same revoke/drain contract");
}

static void DuplicateCleanupAndCloseAreIdempotent()
{
    Root root;
    File owner(&root);
    root.Owner = &owner;
    Irp cleanup{{IRP_MJ_CLEANUP, &owner.Wdm}};
    TraceFileIrp(&root, &cleanup);
    VadFileCleanup(&owner);
    VadFileCleanup(&owner);
    Irp duplicate{{IRP_MJ_CLEANUP, &owner.Wdm}};
    TraceFileIrp(&root, &duplicate);
    Irp close{{IRP_MJ_CLOSE, &owner.Wdm}};
    TraceFileIrp(&root, &close);
    Check(root.Revokes == 1 && root.Purges == 1 && root.Flushes == 1 &&
        root.Starts == 1 && root.Releasing == 0 && root.Forwards == 3,
        "duplicate raw/client cleanup and CLOSE cannot repeat the transaction");
}

static void DelayedOldFileCannotRevokeNextOwner()
{
    Root root;
    File owner(&root);
    File next(&root);
    root.Owner = &owner;
    Irp cleanup{{IRP_MJ_CLEANUP, &owner.Wdm}};
    TraceFileIrp(&root, &cleanup);
    root.Owner = &next;
    ++root.Session;
    VadFileCleanup(&owner);
    Irp stale{{IRP_MJ_CLEANUP, &owner.Wdm}};
    TraceFileIrp(&root, &stale);
    Irp close{{IRP_MJ_CLOSE, &owner.Wdm}};
    TraceFileIrp(&root, &close);
    Check(root.Owner == &next && root.Revokes == 1 && root.Purges == 1,
        "old exact-file cleanup and late CLOSE leave the new owner untouched");
    VadFileCleanup(&next);
    Check(root.Owner == nullptr && root.Releasing == 0 && root.Revokes == 2 &&
        root.Purges == 2 && root.Flushes == 2 && root.Starts == 2,
        "the next owner's own cleanup remains independently effective");
}

static void ForwardedClientCallbackCannotDoubleRevoke()
{
    Root root;
    File owner(&root);
    root.Owner = &owner;
    root.CallbackOnForward = true;
    Irp cleanup{{IRP_MJ_CLEANUP, &owner.Wdm}};
    TraceFileIrp(&root, &cleanup);
    Check(root.Owner == nullptr && root.Revokes == 1 && root.Purges == 1 &&
        root.Flushes == 1 && root.Releasing == 0 && root.ClientCallbacks == 1,
        "forwarding to a working client callback does not double-revoke");
}

static void NonownerNullUnmappedAndCloseDoNotRevoke()
{
    Root root;
    Root otherRoot;
    File owner(&root);
    File other(&root);
    File foreign(&otherRoot);
    WdmFile unmapped;
    root.Owner = &owner;
    Irp nonowner{{IRP_MJ_CLEANUP, &other.Wdm}};
    Irp absent{{IRP_MJ_CLEANUP, nullptr}};
    Irp unknown{{IRP_MJ_CLEANUP, &unmapped}};
    Irp wrongDevice{{IRP_MJ_CLEANUP, &foreign.Wdm}};
    Irp close{{IRP_MJ_CLOSE, &owner.Wdm}};
    for (auto irp : {&nonowner, &absent, &unknown, &wrongDevice, &close}) {
        Check(TraceFileIrp(&root, irp) == -91, "nonowner IRPs retain dispatch status");
    }
    Check(root.Owner == &owner && root.Revokes == 0 && root.Purges == 0 && root.Forwards == 5,
        "only the exact mapped owner CLEANUP may revoke");
    root.Owner = nullptr;
    RevokeFileOwner(&root, nullptr);
    Check(root.Revokes == 0 && root.Purges == 0, "null owner/file equality is not ownership");
}

static void OfflineCleanupNeverRestartsQueue()
{
    for (bool offlineDuringFlush : {false, true}) {
        Root root;
        File owner(&root);
        root.Owner = &owner;
        root.Online = offlineDuringFlush ? 1 : 0;
        root.OfflineOnFlush = offlineDuringFlush;
        Irp cleanup{{IRP_MJ_CLEANUP, &owner.Wdm}};
        TraceFileIrp(&root, &cleanup);
        Check(root.Owner == nullptr && root.Revokes == 1 && root.Purges == 1 &&
            root.Flushes == 1 && root.Starts == 0 && root.Releasing == 0,
            "offline before or during drain never restarts the control queue");
    }
}

static void MissingControlQueueStillReleasesAdmission()
{
    Root root;
    File owner(&root);
    root.Owner = &owner;
    root.ControlQueue = nullptr;
    root.ControlWork = nullptr;
    Irp cleanup{{IRP_MJ_CLEANUP, &owner.Wdm}};
    TraceFileIrp(&root, &cleanup);
    Check(root.Owner == nullptr && root.Revokes == 1 && root.Releasing == 0 &&
        root.Purges == 0 && root.Flushes == 0 && root.Starts == 0 && root.Forwards == 1,
        "a missing control queue does not leave owner admission latched");
}

static void ElevatedIrqlOnlyForwards()
{
    for (unsigned irql : {APC_LEVEL, DISPATCH_LEVEL}) {
        Root root;
        File owner(&root);
        root.Owner = &owner;
        CurrentIrql = irql;
        const auto maps = MapCalls;
        Irp cleanup{{IRP_MJ_CLEANUP, &owner.Wdm}};
        Irp close{{IRP_MJ_CLOSE, &owner.Wdm}};
        Check(TraceFileIrp(&root, &cleanup) == -91 && TraceFileIrp(&root, &close) == -91,
            "unexpected elevated IRQL still preserves forwarding status");
        Check(root.Owner == &owner && root.Revokes == 0 && root.Forwards == 2 && MapCalls == maps,
            "unexpected elevated IRQL cannot map or enter passive owner cleanup");
        CurrentIrql = PASSIVE_LEVEL;
    }
}

int main()
{
    RawCleanupWithoutClientCallback();
    ClientCleanupStillWorks();
    DuplicateCleanupAndCloseAreIdempotent();
    DelayedOldFileCannotRevokeNextOwner();
    ForwardedClientCallbackCannotDoubleRevoke();
    NonownerNullUnmappedAndCloseDoNotRevoke();
    OfflineCleanupNeverRestartsQueue();
    MissingControlQueueStillReleasesAdmission();
    ElevatedIrqlOnlyForwards();
    std::puts("PASS: production-extracted cleanup, exact-file ownership, idempotence, drain ordering, forwarding and IRQL guards (9 cases)");
}
