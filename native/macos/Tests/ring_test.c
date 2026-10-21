#include "OrbitAudioRing.h"
#include <assert.h>
#include <math.h>
#include <stdint.h>
#include <pthread.h>
#include <stdatomic.h>
#include <sched.h>

static OrbitAudioRing *thread_ring;
static _Atomic bool producer_done;
static void *produce(void *unused) {
    (void)unused;
    float value = 0.5f;
    for (uint64_t index = 0; index < 100000; index++) {
        while (!orbit_ring_push_interleaved_timed(thread_ring, &value, 1, 1, 48000, index, index)) sched_yield();
    }
    atomic_store_explicit(&producer_done, true, memory_order_release);
    return NULL;
}

static void test_admitted_frames_counts_only_accepted_frames(void) {
    float samples[] = {0.25f, 0.5f, -0.25f, -0.5f};
    OrbitAudioBlock block;

    OrbitAudioRing *ring = orbit_ring_create();
    assert(ring);
    assert(orbit_ring_admitted_frames(ring) == 0);
    assert(!orbit_ring_push_interleaved_timed(ring, samples, 0, 4, 48000, 100, 100));
    assert(orbit_ring_admitted_frames(ring) == 0);
    assert(orbit_ring_push_interleaved_timed(ring, samples, 1, 4, 48000, 101, 101));
    assert(orbit_ring_admitted_frames(ring) == 4);
    assert(orbit_ring_admitted_frames(ring) == 4);
    orbit_ring_set_enabled(ring, false);
    assert(!orbit_ring_push_interleaved_timed(ring, samples, 1, 4, 48000, 102, 102));
    assert(orbit_ring_admitted_frames(ring) == 4);
    orbit_ring_destroy(ring);

    ring = orbit_ring_create();
    assert(ring);
    for (int index = 0; index < 32; index++) {
        assert(orbit_ring_push_interleaved_timed(ring, samples, 1, 4, 48000,
                                           (uint64_t)index, (uint64_t)index));
    }
    assert(!orbit_ring_push_interleaved_timed(ring, samples, 1, 4, 48000, 33, 33));
    assert(orbit_ring_admitted_frames(ring) == 128);
    orbit_ring_destroy(ring);

    ring = orbit_ring_create();
    assert(ring);
    orbit_ring_set_capture_cutoff_ns(ring, 1040000);
    assert(orbit_ring_push_interleaved_timed(ring, samples, 1, 4, 48000,
                                             1000000, 7000000));
    assert(orbit_ring_admitted_frames(ring) == 2);
    assert(orbit_ring_pop(ring, &block));
    assert(block.frames == 2);
    orbit_ring_destroy(ring);
}

static int16_t expected_sample(uint32_t frame) {
    switch (frame % 4) {
    case 0: return 0;
    case 1: return 16383;
    case 2: return -16383;
    default: return INT16_MAX;
    }
}

static void test_a2_large_push_is_chunked_without_loss(void) {
    static float samples[10000];
    for (uint32_t frame = 0; frame < 10000; frame++) {
        switch (frame % 4) {
        case 0: samples[frame] = 0.0f; break;
        case 1: samples[frame] = 0.5f; break;
        case 2: samples[frame] = -0.5f; break;
        default: samples[frame] = 1.0f; break;
        }
    }
    const float *planes[] = {samples};
    for (unsigned route = 0; route < 2; route++) {
        OrbitAudioRing *ring = orbit_ring_create();
        assert(ring);
        bool accepted = route == 0
            ? orbit_ring_push_interleaved_timed(ring, samples, 1, 10000, 48000, 1000000, 1000000)
            : orbit_ring_push_planar(ring, planes, 1, 10000, 48000, 1000000);
        assert(accepted);

        OrbitAudioBlock block;
        uint32_t received = 0;
        uint64_t blocks = 0;
        while (orbit_ring_pop(ring, &block)) {
            assert(block.sequence == blocks);
            assert(block.frames > 0 && block.frames <= ORBIT_AUDIO_MAX_FRAMES);
            for (uint32_t frame = 0; frame < block.frames; frame++) {
                assert(block.samples[frame] == expected_sample(received + frame));
            }
            received += block.frames;
            blocks++;
        }
        assert(blocks > 1 && received == 10000);
        assert(orbit_ring_admitted_frames(ring) == 10000);
        assert(orbit_ring_take_dropped_frames(ring) == 0);
        orbit_ring_destroy(ring);
    }
}

int main(void) {
    test_admitted_frames_counts_only_accepted_frames();
    test_a2_large_push_is_chunked_without_loss();
    OrbitAudioRing *ring = orbit_ring_create();
    assert(ring);
    float samples[] = {2.0f, -2.0f, NAN, 0.0f};
    assert(orbit_ring_push_interleaved_timed(ring, samples, 1, 4, 48000, 123, 123));
    OrbitAudioBlock block;
    assert(orbit_ring_pop(ring, &block));
    assert(block.sequence == 0 && block.frames == 4 && block.monotonic_ns == 123);
    assert(block.samples[0] == INT16_MAX && block.samples[1] == INT16_MIN);
    assert(block.samples[2] == 0 && block.samples[3] == 0);
    for (int i = 0; i < 32; i++) assert(orbit_ring_push_interleaved_timed(ring, samples, 1, 4, 48000, 124, 124));
    assert(!orbit_ring_push_interleaved_timed(ring, samples, 1, 4, 48000, 125, 125));
    assert(orbit_ring_take_dropped_frames(ring) == 4);
    assert(orbit_ring_take_dropped_frames(ring) == 0);
    for (int i = 0; i < 32; i++) assert(orbit_ring_pop(ring, &block));
    assert(!orbit_ring_pop(ring, &block));
    assert(orbit_ring_push_interleaved_timed(ring, samples, 1, 4, 48000, 126, 126));
    assert(orbit_ring_pop(ring, &block) && block.sequence == 34);
    orbit_ring_set_enabled(ring, false);
    assert(!orbit_ring_push_interleaved_timed(ring, samples, 1, 4, 48000, 127, 127));
    orbit_ring_destroy(ring);
    thread_ring = orbit_ring_create();
    assert(thread_ring);
    atomic_init(&producer_done, false);
    pthread_t producer;
    assert(pthread_create(&producer, NULL, produce, NULL) == 0);
    uint64_t received = 0;
    while (received < 100000) {
        if (orbit_ring_pop(thread_ring, &block)) {
            assert(block.monotonic_ns == received && block.frames == 1 && block.samples[0] == 16383);
            received++;
        } else {
            if (atomic_load_explicit(&producer_done, memory_order_acquire)) {
                if (!orbit_ring_pop(thread_ring, &block)) break;
                assert(block.monotonic_ns == received && block.samples[0] == 16383);
                received++;
            } else sched_yield();
        }
    }
    assert(pthread_join(producer, NULL) == 0 && received == 100000);
    orbit_ring_destroy(thread_ring);
    return 0;
}
