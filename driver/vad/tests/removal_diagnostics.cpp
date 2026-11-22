// SPDX-License-Identifier: MIT
#include <cstdio>
#include <cstdlib>
#include <cstdint>
#if __has_include("../src/RemovalDiagnostic.h")
#include "../src/RemovalDiagnostic.h"

static void Check(bool ok, const char* message)
{
    if (!ok) { std::fprintf(stderr, "FAIL: %s\n", message); std::exit(1); }
}
static void MissingStagesStayUnknown()
{
    vad::RemovalDiagnostic d = {};
    d.Reset(21, 99);
    Check(d.Runtime == 21 && d.Session == 99 && d.Pending == 0 && d.Sequence == 0,
        "fresh runtime starts with no fabricated removal observations");
    d.Record(21, 99, vad::RemovalStage::RemoveEnter, 100);
    d.Record(21, 99, vad::RemovalStage::RemoveReturn, 110, true, 0, true);
    const auto result = d.TakePending();
    Check(result.Sequence == 2 && result.Stages[0].Count == 1 &&
        result.Stages[1].Last.HasStatus && result.Stages[1].Last.Status == 0 &&
        result.Stages[1].Last.NativeCall && result.Stages[2].Count == 0 &&
        result.Stages[4].Count == 0 && result.Stages[5].Count == 0,
        "accepted remove does not invent ReleaseHardware, cleanup or destruction");
    Check(d.Pending == 0 && d.Sequence == 2 && d.TakePending().Pending == 0,
        "passive export drains pending bits, not retained evidence");
}
static void FailureSurvivesRetryAndReentrantCleanup()
{
    vad::RemovalDiagnostic d = {};
    d.Reset(21, 99);
    d.Record(21, 99, vad::RemovalStage::ReleaseEnter, 100);
    d.Record(21, 99, vad::RemovalStage::ReleaseReturn, 110, true, uint32_t(-37), true);
    d.Record(21, 99, vad::RemovalStage::ReleaseEnter, 120);
    d.Record(21, 99, vad::RemovalStage::Cleanup, 130);
    d.Record(21, 99, vad::RemovalStage::ReleaseReturn, 140, true, 0, true);
    d.Record(21, 99, vad::RemovalStage::Destroy, 150);
    const auto result = d.TakePending();
    const auto& release = result.Stages[static_cast<unsigned>(vad::RemovalStage::ReleaseReturn)];
    Check(release.Count == 2 && release.First.Status == uint32_t(-37) && release.Last.Status == 0 &&
        release.FirstFailure.Sequence == 2 && release.FirstFailure.Status == uint32_t(-37),
        "success never erases the first native release failure");
    Check(result.Stages[4].Last.Sequence == 4 && release.Last.Sequence == 5 &&
        result.Stages[5].Last.Sequence == 6,
        "actual callback order is retained without assuming ACX cannot reenter");
    d.Record(21, 99, vad::RemovalStage::ReleaseReturn, 160, true, uint32_t(-91), true);
    Check(d.Stages[3].FirstFailure.Status == uint32_t(-37), "first failure remains sticky after later failures");
}
static void RuntimeIsolationAndAbsentCircuit()
{
    vad::RemovalDiagnostic d = {};
    d.Reset(21, 99);
    d.Record(20, 99, vad::RemovalStage::ReleaseReturn, 100, true, uint32_t(-1), true);
    d.Record(21, 98, vad::RemovalStage::ReleaseReturn, 100, true, uint32_t(-1), true);
    Check(d.Sequence == 0 && d.Pending == 0, "foreign identities cannot contaminate this runtime");
    d.Record(21, 99, vad::RemovalStage::ReleaseReturn, 0, true, 0, false);
    Check(!d.Stages[3].Last.NativeCall && d.Stages[3].Last.Qpc == 0,
        "no published circuit and unavailable clock are not fabricated ACX evidence");
    d.Reset(22, 100);
    Check(d.Runtime == 22 && d.Session == 100 && d.Stages[3].Count == 0 && !d.Saturated &&
        d.TakePending().Pending != 0,
        "reuse must report lost pending evidence instead of silently erasing the old runtime");
    Check(d.DiscardedRuntimes == 1 && d.DiscardedRuntime == 21 && d.DiscardedSession == 99 &&
        d.DiscardedSequence == 1 && d.DiscardedStages == (uint32_t{1} << 3),
        "overwrite notice preserves exact prior runtime/session and pending stages");
    d.Reset(23, 101);
    Check(d.DiscardedRuntimes == 1 && d.Pending == vad::RemovalLossPending,
        "notice-only reuse preserves provenance without inventing another lost lifetime");
    d.Record(23, 101, vad::RemovalStage::Cleanup, 300);
    d.Reset(24, 102);
    Check(d.DiscardedRuntimes == 2 && d.DiscardedRuntime == 23 && d.DiscardedSession == 101,
        "repeated overwrite counts discarded lifetimes and retains the latest exact identity");
    d.DiscardedRuntimes = ~uint64_t{0};
    d.Record(24, 102, vad::RemovalStage::Destroy, 400);
    d.Reset(25, 103);
    Check(d.DiscardedRuntimes == ~uint64_t{0} && d.DiscardSaturated &&
        d.DiscardedRuntime == 24 && d.DiscardedStages == (uint32_t{1} << 5),
        "lost-lifetime counter saturation is explicit and never wraps into no loss");
}
static void SaturationCannotWrapIntoFreshEvidence()
{
    vad::RemovalDiagnostic d = {};
    d.Reset(21, 99);
    d.Sequence = ~std::uint64_t{0};
    d.Record(21, 99, vad::RemovalStage::Cleanup, 100);
    Check(d.Saturated && d.Sequence == ~std::uint64_t{0} && d.Stages[4].Count == 0 &&
        d.TakePending().Saturated, "sequence exhaustion is explicit and cannot wrap");
    d.Reset(21, 99);
    d.Stages[4].Count = ~std::uint32_t{0};
    d.Record(21, 99, vad::RemovalStage::Cleanup, 100);
    Check(d.Saturated && d.Stages[4].Count == ~std::uint32_t{0} && d.Sequence == 0,
        "stage-count exhaustion does not create a false latest observation");
}
int main()
{
    MissingStagesStayUnknown();
    FailureSurvivesRetryAndReentrantCleanup();
    RuntimeIsolationAndAbsentCircuit();
    SaturationCannotWrapIntoFreshEvidence();
    std::printf("PASS: bounded per-runtime removal diagnostics, unknown stages, retries, loss and saturation "
        "(4 cases); per-slot=%zu bytes\n", sizeof(vad::RemovalDiagnostic));
}
#else
int main()
{
    std::fputs("FAIL: per-runtime removal-stage evidence is not implemented\n", stderr);
    return 1;
}
#endif
