/* Deterministic injection into the REAL ring implementation. The hook is absent
   from production builds and pauses only this test's producer. */
#define ORBIT_RING_TEST_HOOKS 1
#include "../Sources/OrbitAudioRing/OrbitAudioRing.c"
#include <assert.h>
#include <pthread.h>
#include <sched.h>

static _Atomic bool reserved;
static _Atomic bool release_copy;
static OrbitAudioRing *tested_ring;
static bool planar;
static bool timed;

void orbit_ring_test_after_reserve(OrbitAudioRing *ring) {
    assert(ring == tested_ring);
    atomic_store_explicit(&reserved, true, memory_order_release);
    while (!atomic_load_explicit(&release_copy, memory_order_acquire)) sched_yield();
}

static void *copy_pending_tail(void *unused) {
    (void)unused;
    float samples[] = {0.25f, 0.5f, -0.25f, -0.5f};
    const float *planes[] = {samples};
    bool accepted = timed
        ? orbit_ring_push_interleaved_timed(tested_ring, samples, 1, 4, 48000, 1000000, 7000000)
        : planar
        ? orbit_ring_push_planar(tested_ring, planes, 1, 4, 48000, 123)
        : orbit_ring_push_interleaved_timed(tested_ring, samples, 1, 4, 48000, 123, 123);
    assert(accepted);
    return NULL;
}

int main(void) {
    for (unsigned route = 0; route < 2; route++) {
        tested_ring = orbit_ring_create();
        assert(tested_ring);
        planar = route != 0;
        atomic_init(&reserved, false);
        atomic_init(&release_copy, false);
        pthread_t producer;
        assert(pthread_create(&producer, NULL, copy_pending_tail, NULL) == 0);
        while (!atomic_load_explicit(&reserved, memory_order_acquire)) sched_yield();
        orbit_ring_set_enabled(tested_ring, false);
        /* The old boolean gate could not distinguish this producer from drained. */
        assert(!orbit_ring_is_quiescent(tested_ring));
        OrbitAudioBlock block;
        assert(!orbit_ring_pop(tested_ring, &block));
        atomic_store_explicit(&release_copy, true, memory_order_release);
        assert(pthread_join(producer, NULL) == 0);
        assert(orbit_ring_is_quiescent(tested_ring));
        assert(orbit_ring_pop(tested_ring, &block));
        assert(block.frames == 4 && block.monotonic_ns == 123 && block.sequence == 0);
        assert(block.samples[0] == 8191 && block.samples[3] == -16383);
        assert(!orbit_ring_pop(tested_ring, &block));
        assert(orbit_ring_admitted_frames(tested_ring) == 4);
        /* A callback delivered after admission closes cannot enter the ring. */
        float late[] = {0.1f, 0.2f};
        assert(!orbit_ring_push_interleaved_timed(tested_ring, late, 1, 2, 48000, 122, 122));
        assert(orbit_ring_take_dropped_frames(tested_ring) == 0);
        assert(orbit_ring_admitted_frames(tested_ring) == 4);
        assert(!orbit_ring_pop(tested_ring, &block));
        orbit_ring_destroy(tested_ring);
    }

    /* Stop arrives while the REAL timed producer is mid-copy: retain exactly
       the two samples starting before cutoff, in a different output clock. */
    tested_ring = orbit_ring_create();
    assert(tested_ring);
    timed = true;
    atomic_store(&reserved, false);
    atomic_store(&release_copy, false);
    pthread_t producer;
    assert(pthread_create(&producer, NULL, copy_pending_tail, NULL) == 0);
    while (!atomic_load_explicit(&reserved, memory_order_acquire)) sched_yield();
    orbit_ring_set_capture_cutoff_ns(tested_ring, 1040000);
    assert(!orbit_ring_is_quiescent(tested_ring));
    atomic_store_explicit(&release_copy, true, memory_order_release);
    assert(pthread_join(producer, NULL) == 0);
    OrbitAudioBlock block;
    assert(orbit_ring_pop(tested_ring, &block));
    assert(block.frames == 2 && block.monotonic_ns == 7000000);
    assert(block.samples[0] == 8191 && block.samples[1] == 16383);

    /* A sample buffer captured BEFORE stop but dispatched AFTER it must survive.
       A later redundant stop may never reopen the post-stop interval. */
    orbit_ring_set_capture_cutoff_ns(tested_ring, 2000000);
    float backlog[] = {0.125f, 0.25f, 0.5f, -0.5f};
    assert(orbit_ring_push_interleaved_timed(tested_ring, backlog, 1, 4, 48000, 900000, 6900000));
    assert(orbit_ring_pop(tested_ring, &block));
    assert(block.frames == 4 && block.monotonic_ns == 6900000);
    assert(block.samples[3] == -16383);
    assert(!orbit_ring_push_interleaved_timed(tested_ring, backlog, 1, 4, 48000, 1040000, 7040000));
    assert(!orbit_ring_pop(tested_ring, &block));
    assert(orbit_ring_take_dropped_frames(tested_ring) == 0);
    orbit_ring_callback_begin(tested_ring);
    /* A platform callback inspecting its clock has not reserved a block yet. */
    assert(!orbit_ring_is_quiescent(tested_ring));
    orbit_ring_callback_end(tested_ring);
    assert(orbit_ring_is_quiescent(tested_ring));
    assert(!orbit_ring_has_timing_failure(tested_ring));
    orbit_ring_mark_timing_failure(tested_ring);
    assert(orbit_ring_has_timing_failure(tested_ring));
    orbit_ring_set_enabled(tested_ring, false);
    assert(orbit_ring_is_quiescent(tested_ring));
    orbit_ring_destroy(tested_ring);

    /* Prefix arithmetic: exact sample boundary is excluded, all supported rates
       retain the proper prefix, and nanosecond values near UInt64.max are safe. */
    const uint32_t rates[] = {8000, 16000, 24000, 44100, 48000, 96000, 192000};
    for (unsigned i = 0; i < sizeof(rates) / sizeof(rates[0]); i++) {
        tested_ring = orbit_ring_create();
        assert(tested_ring);
        const uint64_t start = UINT64_MAX - 10000000;
        const uint32_t rate = rates[i];
        const uint64_t cutoff = start + 1000000000ULL / rate;
        orbit_ring_set_capture_cutoff_ns(tested_ring, cutoff);
        assert(orbit_ring_push_interleaved_timed(tested_ring, backlog, 1, 4, rate, start, 42));
        assert(orbit_ring_pop(tested_ring, &block));
        assert(block.frames == 1 && block.monotonic_ns == 42);
        assert(!orbit_ring_pop(tested_ring, &block));
        orbit_ring_destroy(tested_ring);
    }
    return 0;
}
