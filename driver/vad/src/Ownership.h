// SPDX-License-Identifier: MIT
#pragma once

namespace vad {
// These helpers do not synchronize. Kernel callers supply their existing
// locks; keeping the ownership transitions separate makes them host-testable.
// Preserve the first NTSTATUS failure without skipping later retained references.
template<typename Status, typename Remove>
Status RemoveAllRetained(unsigned count, Status success, Remove remove)
{
    auto result = success;
    for (unsigned i = 0; i < count; ++i) {
        const auto status = remove(i);
        if (result >= 0 && status < 0) {
            result = status;
        }
    }
    return result;
}

template<typename Status, typename Owner, typename Epoch, typename Revoke, typename Counter>
Status ClaimOwner(Status preflight, Owner& owner, Epoch& session, Owner file,
    Revoke revoke, Counter counter)
{
    if (preflight < 0) {
        return preflight;
    }
    // A successful sweep acknowledges submission, not final PDO destruction.
    const auto status = revoke();
    if (status < 0) {
        return status;
    }
    const auto now = counter();
    session = now > session ? now : session + 1;
    owner = file;
    return status;
}

template<typename Entry, unsigned Capacity>
struct CompletionBatch {
    Entry Entries[Capacity];
    unsigned Count;

    bool Append(const Entry& entry)
    {
        if (Count == Capacity) {
            return false;
        }
        Entries[Count++] = entry;
        return true;
    }

    template<typename Complete>
    void Drain(Complete complete)
    {
        const auto count = Count;
        Count = 0;
        for (unsigned i = 0; i < count; ++i) {
            complete(Entries[i]);
        }
    }
};

template<typename Request>
struct ControlHandoff {
    Request Pending;

    bool Publish(Request request)
    {
        if (Pending != Request{}) {
            return false;
        }
        Pending = request;
        return true;
    }

    Request Take()
    {
        const auto request = Pending;
        Pending = Request{};
        return request;
    }

    bool Cancel(Request request)
    {
        if (Pending != request) {
            return false;
        }
        (void)Take();
        return true;
    }
};
}
