// SPDX-License-Identifier: MIT
#include "../src/Lifecycle.h"
#include "../src/audio/CircuitIdentity.h"
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>

static_assert(MOIREN_VAD_CAP_CREATE_DEFERRED == 0x20u);
static_assert((MOIREN_VAD_CAP_CREATE_DEFERRED &
    (MOIREN_VAD_CAP_RAW_PDO | MOIREN_VAD_CAP_POLICY_STAGED |
        MOIREN_VAD_CAP_AUDIO | MOIREN_VAD_CAP_STREAMS | MOIREN_VAD_CAP_PCM)) == 0);
static_assert(MOIREN_VAD_CREATE_DEFERRED_KIND == 5u);
static_assert(MOIREN_VAD_CREATE_DEFERRED_KIND != MOIREN_VAD_STATE_PRESENT &&
    MOIREN_VAD_CREATE_DEFERRED_KIND != MOIREN_VAD_STATE_REMOVING &&
    MOIREN_VAD_CREATE_DEFERRED_KIND != MOIREN_VAD_STATE_REMOVED &&
    MOIREN_VAD_CREATE_DEFERRED_KIND != MOIREN_VAD_STATE_CREATING);

static void Check(bool condition, const char* message)
{
    if (!condition) {
        fprintf(stderr, "FAIL: %s\n", message);
        exit(1);
    }
}

static void CapacityAndRetirement()
{
    vad::Slot slots[MOIREN_VAD_MAX_ENDPOINTS] = {};
    uint64_t next = 0;
    for (uint32_t i = 0; i < MOIREN_VAD_MAX_ENDPOINTS; ++i) {
        int index = vad::Reserve(slots, 7, next, 1, (i % 2) + 1);
        assert(index == static_cast<int>(i));
        assert(slots[i].Id == i + 1);
        assert(!vad::Reusable(slots[i])); // Creation consumes quota, too.
        slots[i].PolicyPrepared = 1;
        slots[i].State = MOIREN_VAD_STATE_PRESENT;
    }
    assert(vad::Reserve(slots, 7, next, 1, 1) == -1);
    assert(next == 64);
    assert(vad::Revoke(slots[0]));
    assert(vad::Reserve(slots, 8, next, 2, 2) == -1);
    assert(vad::Revoke(slots[0])); // Repeated REMOVE can retry missing.
    assert(!vad::FinishLifetime(slots[0], 99)); // Stale callback cannot retire it.
    assert(!vad::Reusable(slots[0]));
    assert(vad::FinishLifetime(slots[0], 1));
    assert(vad::Reserve(slots, 8, next, 2, 2) == 0);
    assert(slots[0].Id == 65 && slots[0].Session == 8);
    assert(!vad::FinishLifetime(slots[0], 1));
    assert(slots[0].State == vad::Creating);
}

static void AdmissionAndWire()
{
    for (uint32_t intent = 0; intent < 4; ++intent) {
        for (uint32_t channels = 0; channels < 4; ++channels) {
            assert(vad::ValidEndpoint(intent, channels) ==
                ((intent == 1 || intent == 2) && (channels == 1 || channels == 2)));
        }
    }
    vad::Slot slot = {};
    slot.Id = 0x0102030405060708ull;
    slot.Session = 5;
    slot.State = MOIREN_VAD_STATE_PRESENT;
    slot.Intent = 2;
    slot.Channels = slot.PolicyPrepared = 1;
    slot.AdmissionGeneration = 1;
    auto snapshot = vad::Snapshot(slot);
    assert(snapshot.GateOpen == 0 && snapshot.AudioReady == 0);
    assert(snapshot.Size == 128 && snapshot.Version == 2);
    assert(snapshot.PolicyPrepared == 1);
    const unsigned char idBytes[] = {8, 7, 6, 5, 4, 3, 2, 1};
    assert(memcmp(reinterpret_cast<const unsigned char*>(&snapshot) + 8,
        idBytes, sizeof(idBytes)) == 0);
    assert(snapshot.Reserved == 0);
    assert(vad::Revoke(slot));
    snapshot = vad::Snapshot(slot);
    assert(snapshot.State == MOIREN_VAD_STATE_REMOVING);
    assert(snapshot.GateOpen == 0 && snapshot.AudioReady == 0);
    assert(vad::FinishLifetime(slot, slot.Id));
    assert(!vad::Revoke(slot));
    vad::Slot slots[MOIREN_VAD_MAX_ENDPOINTS] = {};
    uint64_t next = ~uint64_t{0};
    assert(vad::Reserve(slots, 1, next, 1, 1) == -2);
    assert(slots[0].Id == 0 && next == ~uint64_t{0});
}

static void ClosedGenerationCannotReadmitOldStream()
{
    vad::Slot slot = {};
    slot.Id = 5;
    slot.Session = 9;
    slot.State = MOIREN_VAD_STATE_PRESENT;
    slot.Intent = 1;
    slot.Channels = 2;
    slot.PolicyPrepared = slot.GateOpen = slot.AudioReady = 1;
    slot.AdmissionGeneration = 42;
    const uint64_t oldGeneration = slot.AdmissionGeneration;
    vad::CloseGate(slot);
    assert(slot.GateOpen == 0 && slot.AdmissionGeneration == oldGeneration + 1);
    vad::CloseGate(slot); // Idempotent close does not consume generations.
    assert(slot.AdmissionGeneration == oldGeneration + 1);
    slot.GateOpen = 1; // Simulate a future reviewed audio admission path.
    assert(slot.AdmissionGeneration != oldGeneration);
    assert(vad::Revoke(slot));
    assert(slot.GateOpen == 0 && slot.State == MOIREN_VAD_STATE_REMOVING);
    slot.State = MOIREN_VAD_STATE_PRESENT;
    slot.GateOpen = 1;
    slot.AdmissionGeneration = ~uint64_t{0};
    vad::CloseGate(slot);
    assert(slot.GateOpen == 0 && slot.AdmissionGeneration == ~uint64_t{0});
}

static void NativeStreamAndGateAreIndependent()
{
    vad::Slot slot = {};
    slot.Id = 5;
    slot.Session = 8;
    slot.State = MOIREN_VAD_STATE_PRESENT;
    slot.AudioReady = 1;
    slot.GateOpen = 1;
    slot.AdmissionGeneration = 4;
    slot.StreamGeneration = 6;
    slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
    MOIREN_VAD_PCM_HEADER header = {80, 2, 5, 8, 5, 6, 4, 0, 0, 2, 480, 2, 0};
    assert(vad::SameStream(slot, header));
    for (uint32_t field = 0; field < 5; ++field) {
        auto stale = header;
        switch (field) {
        case 0: --stale.SessionEpoch; break;
        case 1: --stale.EndpointGeneration; break;
        case 2: --stale.StreamGeneration; break;
        case 3: --stale.GateEpoch; break;
        case 4: --stale.EndpointId; break;
        }
        assert(!vad::SameStream(slot, stale));
    }
    vad::CloseGate(slot);
    assert(!vad::SameStream(slot, header));
    assert(slot.StreamGeneration == 6 && slot.StreamState == MOIREN_VAD_STREAM_RUNNING);
    slot.GateOpen = 1;
    assert(!vad::SameStream(slot, header));
    header.GateEpoch = slot.AdmissionGeneration;
    assert(vad::SameStream(slot, header));
    ++slot.StreamGeneration;
    assert(!vad::SameStream(slot, header));
    slot.State = vad::Creating;
    assert(vad::Revoke(slot)); // Closing an app during async creation also retires it.
    assert(!slot.AudioReady && !slot.GateOpen);
}

static void PcmCanQueueBeforeNativeRunButCannotTransferWhilePaused()
{
    vad::Slot slot = {};
    slot.Id = 5;
    slot.Session = 8;
    slot.State = MOIREN_VAD_STATE_PRESENT;
    slot.Intent = MOIREN_VAD_INTENT_PLAYBACK;
    slot.Channels = 2;
    slot.AudioReady = slot.GateOpen = 1;
    slot.AdmissionGeneration = 4;
    slot.StreamGeneration = 6;
    slot.StreamState = MOIREN_VAD_STREAM_PAUSED;
    MOIREN_VAD_PCM_HEADER header = {80, 2, 5, 8, 5, 6, 4, 0, 0, 2, 480, 2, 0};

    assert(vad::CanQueuePcm(slot, header));
    assert(!vad::SameStream(slot, header));
    slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
    assert(vad::CanQueuePcm(slot, header));
    assert(vad::SameStream(slot, header));
    vad::CloseGate(slot);
    assert(!vad::CanQueuePcm(slot, header));
}

static void DirectIoShapesAreBounded()
{
    const uint32_t formats[] = {MOIREN_VAD_FORMAT_PCM16, MOIREN_VAD_FORMAT_FLOAT32};
    for (const uint32_t format : formats) {
        for (uint32_t channels = 1; channels <= 2; ++channels) {
            for (uint32_t frames = 1; frames <= MOIREN_VAD_MAX_PACKET_FRAMES; ++frames) {
                MOIREN_VAD_PCM_HEADER header = {};
                header.Size = sizeof(header);
                header.Version = MOIREN_VAD_ABI_VERSION;
                header.SampleFormat = format;
                header.Channels = channels;
                header.Frames = frames;
                assert(vad::ValidPcmShape(header));
                assert(vad::PcmBytes(header) <= MOIREN_VAD_MAX_PCM_BYTES);
                for (uint32_t field = 0; field < 7; ++field) {
                    auto invalid = header;
                    switch (field) {
                    case 0: invalid.Size = ~uint32_t{0}; break;
                    case 1: invalid.Version = 1; break;
                    case 2: invalid.SampleFormat = 3; break;
                    case 3: invalid.Channels = ~uint32_t{0}; break;
                    case 4: invalid.Frames = ~uint32_t{0}; break;
                    case 5: invalid.Frames = 0; break;
                    case 6: invalid.Reserved = 1; break;
                    }
                    assert(!vad::ValidPcmShape(invalid));
                }
            }
        }
    }
}

static void DistinctEndpointsHaveDistinctCircuitInstances()
{
    const MOIREN_VAD_ENDPOINT_ID first = {1, 0x4d6f6972656e5052ull};
    const MOIREN_VAD_ENDPOINT_ID second = {2, first.High};
    const uint32_t intents[] = {MOIREN_VAD_INTENT_PLAYBACK,
        MOIREN_VAD_INTENT_MICROPHONE};
    for (const uint32_t intent : intents) {
        const auto a = vad::audio::CircuitInstanceIdentity(first, intent);
        const auto b = vad::audio::CircuitInstanceIdentity(second, intent);
        assert(wcscmp(a.Uri, b.Uri) != 0);
    }
}

static void CircuitIdentityRetainsEveryStableBitAndDirection()
{
    const MOIREN_VAD_ENDPOINT_ID zero = {};
    const auto baseline = vad::audio::CircuitInstanceIdentity(zero,
        MOIREN_VAD_INTENT_PLAYBACK);
    assert(wcscmp(baseline.Uri,
        L"moiren:vad:p:00000000000000000000000000000000") == 0);
    assert(wcslen(baseline.Uri) < sizeof(baseline.Uri) / sizeof(baseline.Uri[0]));
    vad::audio::CircuitIdentity bitIdentities[128] = {};
    for (uint32_t bit = 0; bit < 128; ++bit) {
        auto endpoint = zero;
        if (bit < 64) {
            endpoint.Low = uint64_t{1} << bit;
        } else {
            endpoint.High = uint64_t{1} << (bit - 64);
        }
        const auto playback = vad::audio::CircuitInstanceIdentity(endpoint,
            MOIREN_VAD_INTENT_PLAYBACK);
        const auto capture = vad::audio::CircuitInstanceIdentity(endpoint,
            MOIREN_VAD_INTENT_MICROPHONE);
        const auto recreated = vad::audio::CircuitInstanceIdentity(endpoint,
            MOIREN_VAD_INTENT_PLAYBACK);
        assert(wcscmp(playback.Uri, baseline.Uri) != 0);
        assert(wcscmp(playback.Uri, capture.Uri) != 0);
        assert(wcscmp(playback.Uri, recreated.Uri) == 0);
        for (uint32_t previous = 0; previous < bit; ++previous) {
            assert(wcscmp(playback.Uri, bitIdentities[previous].Uri) != 0);
        }
        bitIdentities[bit] = playback;
    }
    const MOIREN_VAD_ENDPOINT_ID maximum = {~uint64_t{0}, ~uint64_t{0}};
    const auto full = vad::audio::CircuitInstanceIdentity(maximum,
        MOIREN_VAD_INTENT_MICROPHONE);
    assert(wcscmp(full.Uri,
        L"moiren:vad:m:ffffffffffffffffffffffffffffffff") == 0);
    const MOIREN_VAD_ENDPOINT_ID ordered = {0x0123456789abcdefull, 0xfedcba9876543210ull};
    const auto value = vad::audio::CircuitInstanceIdentity(ordered,
        MOIREN_VAD_INTENT_PLAYBACK);
    assert(wcscmp(value.Uri,
        L"moiren:vad:p:fedcba98765432100123456789abcdef") == 0);
}

static void DeferredCreateHasCanonicalWireAndNoMutation()
{
    MOIREN_VAD_CREATE_REQUEST request = {};
    request.Size = sizeof(request);
    request.Version = MOIREN_VAD_ABI_VERSION;
    request.StableId = {0x0123456789abcdefull, 0xfedcba9876543210ull};
    request.AllowDefault = 1;
    request.Name[0] = 'x';
    vad::Slot slot = {};
    slot.Id = 91;
    slot.Session = 7;
    slot.StableId = request.StableId;
    slot.State = MOIREN_VAD_STATE_REMOVING;
    slot.PolicyPrepared = 1;
    slot.StreamGeneration = 23;
    slot.AdmissionGeneration = 29;
    slot.LastStatus = 0xc0000001u;
    slot.DroppedPackets = 31;
    slot.SilencePackets = 37;
    unsigned char before[sizeof(slot)];
    memcpy(before, &slot, sizeof(slot));
    const uint64_t session = 0x1122334455667788ull;
    unsigned char expected[128] = {};
    expected[0] = 128;
    expected[4] = 2;
    expected[16] = 5;
    const unsigned char sessionBytes[] = {0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11};
    const unsigned char stableBytes[] = {
        0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01,
        0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe};
    memcpy(expected + 40, sessionBytes, sizeof(sessionBytes));
    memcpy(expected + 72, stableBytes, sizeof(stableBytes));
    for (uint32_t intent = 1; intent <= 2; ++intent) {
        for (uint32_t channels = 1; channels <= 2; ++channels) {
            request.Intent = intent;
            request.Channels = channels;
            unsigned char requestBefore[sizeof(request)];
            memcpy(requestBefore, &request, sizeof(request));
            MOIREN_VAD_CREATE_DEFERRED reply;
            memset(&reply, 0xa5, sizeof(reply));
            Check(vad::TryDeferCreate(slot, session, request, reply),
                "a revoked old-session stable-ID collision acknowledges deferred CREATE");
            expected[20] = static_cast<unsigned char>(intent);
            expected[24] = static_cast<unsigned char>(channels);
            Check(memcmp(&reply, expected, sizeof(expected)) == 0,
                "deferred CREATE matches all 128 canonical wire bytes");
            Check(memcmp(&slot, before, sizeof(slot)) == 0 &&
                memcmp(&request, requestBefore, sizeof(request)) == 0,
                "deferred CREATE never mutates the old slot or request");
        }
    }
}

static void DeferredCreateRetainsEveryStableBit()
{
    MOIREN_VAD_CREATE_REQUEST request = {};
    request.Intent = MOIREN_VAD_INTENT_PLAYBACK;
    request.Channels = 2;
    vad::Slot slot = {};
    slot.Session = 7;
    slot.State = MOIREN_VAD_STATE_REMOVING;
    unsigned char expected[128] = {};
    expected[0] = 128;
    expected[4] = 2;
    expected[16] = 5;
    expected[20] = 1;
    expected[24] = 2;
    expected[40] = 8;
    for (uint32_t bit = 0; bit < 128; ++bit) {
        request.StableId = {};
        if (bit < 64) {
            request.StableId.Low = uint64_t{1} << bit;
        } else {
            request.StableId.High = uint64_t{1} << (bit - 64);
        }
        slot.StableId = request.StableId;
        memset(expected + 72, 0, 16);
        expected[72 + bit / 8] = static_cast<unsigned char>(1u << (bit % 8));
        MOIREN_VAD_CREATE_DEFERRED reply = {};
        Check(vad::TryDeferCreate(slot, 8, request, reply) &&
            memcmp(&reply, expected, sizeof(expected)) == 0,
            "every stable-ID bit survives deferred encoding at its LE wire offset");
    }
}

static void CheckCreateNotDeferred(const vad::Slot& slot, uint64_t session,
    const MOIREN_VAD_CREATE_REQUEST& request)
{
    unsigned char before[sizeof(slot)];
    memcpy(before, &slot, sizeof(slot));
    MOIREN_VAD_CREATE_DEFERRED reply;
    memset(&reply, 0xa5, sizeof(reply));
    unsigned char replyBefore[sizeof(reply)];
    memcpy(replyBefore, &reply, sizeof(reply));
    Check(!vad::TryDeferCreate(slot, session, request, reply),
        "only a prior nonzero session with a revoked matching slot may defer CREATE");
    Check(memcmp(&slot, before, sizeof(slot)) == 0 &&
        memcmp(&reply, replyBefore, sizeof(reply)) == 0,
        "a refused deferral changes neither the slot nor reply");
}

static void DeferredCreateRequiresOldRevokedMatchingSlot()
{
    MOIREN_VAD_CREATE_REQUEST request = {};
    request.StableId = {0x0123456789abcdefull, 0xfedcba9876543210ull};
    vad::Slot slot = {};
    slot.Session = 7;
    slot.State = MOIREN_VAD_STATE_REMOVING;
    slot.StableId = request.StableId;
    CheckCreateNotDeferred(slot, 7, request);
    const uint64_t notPrior[] = {0, 8, 9, ~uint64_t{0}};
    for (const auto session : notPrior) {
        auto invalid = slot;
        invalid.Session = session;
        CheckCreateNotDeferred(invalid, 8, request);
    }
    CheckCreateNotDeferred(slot, 0, request);
    const uint32_t states[] = {0, MOIREN_VAD_STATE_PRESENT,
        MOIREN_VAD_STATE_REMOVED, MOIREN_VAD_STATE_CREATING, 5, ~uint32_t{0}};
    for (const auto state : states) {
        auto invalid = slot;
        invalid.State = state;
        CheckCreateNotDeferred(invalid, 8, request);
    }
    const uint32_t nonzero[] = {1, 2, ~uint32_t{0}};
    for (const auto value : nonzero) {
        auto invalid = slot;
        invalid.GateOpen = value;
        CheckCreateNotDeferred(invalid, 8, request);
        invalid = slot;
        invalid.AudioReady = value;
        CheckCreateNotDeferred(invalid, 8, request);
    }
    for (uint32_t bit = 0; bit < 128; ++bit) {
        auto invalid = slot;
        if (bit < 64) {
            invalid.StableId.Low ^= uint64_t{1} << bit;
        } else {
            invalid.StableId.High ^= uint64_t{1} << (bit - 64);
        }
        CheckCreateNotDeferred(invalid, 8, request);
    }
}

static void DeferredCreateWaitsForOldLifetimeWithoutReserving()
{
    vad::Slot slots[MOIREN_VAD_MAX_ENDPOINTS] = {};
    uint64_t next = 0;
    for (uint32_t i = 0; i < MOIREN_VAD_MAX_ENDPOINTS; ++i) {
        Check(vad::Reserve(slots, 7, next, 1, 2) == static_cast<int>(i),
            "old-session children consume the full quota");
        slots[i].State = MOIREN_VAD_STATE_PRESENT;
        slots[i].StableId = {i + 1, 0x0123456789abcdefull};
        slots[i].GateOpen = 1;
        slots[i].AudioReady = 1;
    }
    MOIREN_VAD_CREATE_REQUEST request = {};
    request.StableId = slots[0].StableId;
    request.Intent = MOIREN_VAD_INTENT_MICROPHONE;
    request.Channels = 1;
    const uint64_t oldId = slots[0].Id;
    Check(vad::Revoke(slots[0]), "cleanup revokes the old child before PnP removal");
    unsigned char before[sizeof(slots)];
    memcpy(before, slots, sizeof(slots));
    for (unsigned retry = 0; retry < 3; ++retry) {
        MOIREN_VAD_CREATE_DEFERRED reply = {};
        Check(vad::TryDeferCreate(slots[0], 8, request, reply),
            "new owner can defer while accepted old removal is asynchronous");
        Check(next == 64 && memcmp(slots, before, sizeof(slots)) == 0,
            "deferral consumes no runtime ID and transfers no old ownership");
    }
    Check(vad::Reserve(slots, 8, next, request.Intent, request.Channels) == -1,
        "old PDO lifetime still holds its quota");
    Check(vad::FinishLifetime(slots[0], oldId), "old PDO final destruction retires its slot");
    CheckCreateNotDeferred(slots[0], 8, request);
    Check(vad::Reserve(slots, 8, next, request.Intent, request.Channels) == 0,
        "a fresh CREATE can reserve after old PDO destruction");
    Check(slots[0].Id == 65 && slots[0].Session == 8 &&
        slots[0].State == vad::Creating,
        "recreation receives a fresh identity in the new session");
}

int main()
{
    // Gate opening is a two-phase operation: clearing storage can overlap
    // native pause/reset or PnP retirement, so readiness must be revalidated.
    {
        vad::Slot slot = {};
        slot.Id = 5;
        slot.Session = 9;
        slot.State = MOIREN_VAD_STATE_PRESENT;
        slot.PolicyPrepared = slot.AudioReady = 1;
        slot.AdmissionGeneration = 11;
        MOIREN_VAD_GATE_REQUEST request = {};
        request.Endpoint.EndpointId = request.Endpoint.EndpointGeneration = 5;
        request.Endpoint.SessionEpoch = 9;
        request.GateEpoch = 11;
        Check(vad::CanCommitGateOpen(slot, request),
            "gate may open after storage clear only with unchanged identity and epoch");
        vad::InvalidatePcmEpoch(slot);
        Check(!vad::CanCommitGateOpen(slot, request),
            "native reset during storage clear prevents opening the stale epoch");
        request.GateEpoch = slot.AdmissionGeneration;
        slot.State = MOIREN_VAD_STATE_REMOVING;
        Check(!vad::CanCommitGateOpen(slot, request),
            "retirement during storage clear cannot readmit the endpoint");
        slot.State = MOIREN_VAD_STATE_PRESENT;
        slot.AudioReady = 0;
        Check(!vad::CanCommitGateOpen(slot, request), "unprepared hardware stays closed");
    }
    CapacityAndRetirement();
    AdmissionAndWire();
    ClosedGenerationCannotReadmitOldStream();
    NativeStreamAndGateAreIndependent();
    PcmCanQueueBeforeNativeRunButCannotTransferWhilePaused();
    DirectIoShapesAreBounded();
    DistinctEndpointsHaveDistinctCircuitInstances();
    CircuitIdentityRetainsEveryStableBitAndDirection();
    DeferredCreateHasCanonicalWireAndNoMutation();
    DeferredCreateRetainsEveryStableBit();
    DeferredCreateRequiresOldRevokedMatchingSlot();
    DeferredCreateWaitsForOldLifetimeWithoutReserving();
    puts("PASS: quota, retirement, wire layout, native/gate generations, Direct I/O shapes, circuit identity, deferred CREATE guards and canonical wire");
    return 0;
}
