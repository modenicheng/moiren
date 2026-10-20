// SPDX-License-Identifier: MIT
#include <cstdio>
#include <cstdlib>

#include "../src/Lifecycle.h"
#include "../src/Ownership.h"

static void Check(bool condition, const char* message)
{
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        std::exit(1);
    }
}

static void CompletionOwnership()
{
    vad::CompletionBatch<unsigned, 4> batch = {};
    for (unsigned i = 0; i < 4; ++i) {
        Check(batch.Append(i + 1), "all four retrieved requests fit");
    }
    Check(!batch.Append(99) && batch.Count == 4, "completion storage is bounded");
    bool locked = true;
    bool rundown = true;
    unsigned completed = 0;
    // Collection does not invoke completion. The caller unlocks before draining.
    Check(completed == 0 && batch.Count == 4, "collection retains ownership");
    locked = false;
    batch.Drain([&](unsigned request) {
        Check(!locked && rundown, "completion occurs unlocked with rundown held");
        Check(request == ++completed, "requests complete once in retrieval order");
    });
    rundown = false;
    Check(batch.Count == 0 && completed == 4, "drain transfers every request");
    batch.Drain([&](unsigned) { Check(false, "a second drain must be empty"); });
    Check(batch.Append(5), "a drained batch can be reused");
}

static void ControlOwnership()
{
    vad::ControlHandoff<unsigned> handoff = {};
    Check(handoff.Publish(1), "the sequential queue publishes one request");
    Check(!handoff.Publish(2) && handoff.Pending == 1,
        "a second handoff cannot overwrite a live request");
    Check(!handoff.Cancel(2) && handoff.Pending == 1,
        "a stale cancellation cannot detach another request");
    Check(handoff.Cancel(1) && handoff.Take() == 0,
        "cancellation before claim leaves no worker-owned request");
    Check(handoff.Publish(2) && handoff.Take() == 2,
        "successful unmark hands the request to the worker");
    Check(!handoff.Cancel(2), "a claimed request is no longer in the handoff");
    Check(handoff.Publish(3) && !handoff.Cancel(2) && handoff.Pending == 3,
        "late cancellation cannot touch the next sequential request");
    Check(handoff.Take() == 3 && handoff.Take() == 0, "claim is exactly once");
    Check(handoff.Publish(4) && handoff.Take() == 4,
        "a canceled unmark detaches before handing completion to cancellation");
    Check(handoff.Take() == 0, "a concurrent worker cannot unmark canceled work twice");
    Check(handoff.Publish(5) && !handoff.Cancel(4) && handoff.Take() == 5,
        "the winning cancel callback cannot detach a later request");
}

static void RetainedRemovalAttemptsAllChildren()
{
    const long statuses[] = {0, -37, 0, -61, 1};
    unsigned attempted = 0;
    const auto status = vad::RemoveAllRetained(5, 0L, [&](unsigned index) {
        Check(index == attempted++, "each retained child is attempted once in order");
        return statuses[index];
    });
    Check(attempted == 5, "a failed submission does not skip later retained children");
    Check(status == -37, "the first submission failure survives later results");
    Check(vad::RemoveAllRetained(0, 0L, [](unsigned) {
        Check(false, "an empty sweep cannot attempt a child");
        return -1L;
    }) == 0, "an empty sweep succeeds");
    Check(vad::RemoveAllRetained(2, 0L, [](unsigned index) {
        return static_cast<long>(index);
    }) == 0, "successful submissions retain the aggregate success status");
}

static void ReopenRetriesFailedCleanupBeforeClaimingOwner()
{
    vad::Slot slot = {};
    slot.Id = 5;
    slot.Session = 7;
    slot.State = MOIREN_VAD_STATE_PRESENT;
    slot.GateOpen = 1;
    slot.AudioReady = 1;
    Check(vad::Revoke(slot), "departing owner revokes its child");

    unsigned owner = 0;
    uint64_t session = slot.Session;
    unsigned attempts = 0;
    unsigned counterReads = 0;
    long submission = -37;
    auto revoke = [&] {
        Check(owner == 0 && session == 7 && counterReads == 0,
            "recovery precedes all owner/session mutation and counter reads");
        return vad::RemoveAllRetained(1, 0L, [&](unsigned) {
            ++attempts;
            return submission;
        });
    };
    auto counter = [&] {
        ++counterReads;
        return uint64_t{100};
    };
    Check(revoke() == -37 && attempts == 1, "cleanup reports its failed submission");
    Check(vad::ClaimOwner(0L, owner, session, 11u, revoke, counter) == -37,
        "the next open refuses ownership while removal submission still fails");
    Check(owner == 0 && session == 7 && attempts == 2 && counterReads == 0,
        "failed recovery leaves ownership and session unchanged for another open");

    submission = 0;
    Check(vad::ClaimOwner(0L, owner, session, 11u, revoke, counter) == 0,
        "a later open retries and accepts successful removal submission");
    Check(owner == 11 && session == 100 && attempts == 3 && counterReads == 1,
        "successful recovery claims exactly one new owner/session");
    Check(slot.State == MOIREN_VAD_STATE_REMOVING && !vad::Reusable(slot) &&
        slot.Session == 7 && slot.GateOpen == 0 && slot.AudioReady == 0,
        "accepted asynchronous removal permits open before old child destruction");
}

static void RefusedOpenDoesNotRevokeOrMutateOwnership()
{
    // Device.cpp supplies the refusal from its name/online/releasing/owner/epoch checks.
    for (long refusal = -1; refusal >= -5; --refusal) {
        unsigned owner = 19;
        uint64_t session = ~uint64_t{0};
        const auto status = vad::ClaimOwner(refusal, owner, session, 23u, [] {
            Check(false, "a refused open must never sweep a live owner's children");
            return 0L;
        }, [] {
            Check(false, "a refused open must never read the session counter");
            return uint64_t{0};
        });
        Check(status == refusal && owner == 19 && session == ~uint64_t{0},
            "refusal preserves the existing owner, session and error");
    }
}

static void OwnerSessionAdvancesWhenCounterDoesNot()
{
    unsigned owner = 0;
    uint64_t session = 7;
    Check(vad::ClaimOwner(0L, owner, session, 11u,
        [] { return 0L; }, [] { return uint64_t{6}; }) == 0,
        "accepted recovery permits a new owner with a lagging counter");
    Check(owner == 11 && session == 8, "new ownership always advances the session");
}

int main()
{
    CompletionOwnership();
    ControlOwnership();
    RetainedRemovalAttemptsAllChildren();
    ReopenRetriesFailedCleanupBeforeClaimingOwner();
    RefusedOpenDoesNotRevokeOrMutateOwnership();
    OwnerSessionAdvancesWhenCounterDoesNot();
    std::puts("PASS: deferred completions, control ownership, aggregate removal and recovery before owner admission");
}
