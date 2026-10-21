#include "OrbitAudioRing.h"
#include <stdatomic.h>
#include <stdlib.h>
#include <math.h>
#include <string.h>

#define ORBIT_RING_CAPACITY 32
struct OrbitAudioRing {
    _Atomic uint64_t head;
    _Atomic uint64_t tail;
    _Atomic uint64_t admitted_frames;
    _Atomic uint64_t dropped_frames;
    _Atomic uint64_t capture_cutoff_ns;
    _Atomic bool timing_failed;
    _Atomic unsigned callbacks_in_flight;
    /* Bit 0: producer owns a write; bit 1: admission disabled. A single CAS
       linearizes admission against stop, including a producer already in flight. */
    _Atomic unsigned producer_state;
    uint64_t next_sequence;
    OrbitAudioBlock blocks[ORBIT_RING_CAPACITY];
};

OrbitAudioRing *orbit_ring_create(void) {
    OrbitAudioRing *ring = calloc(1, sizeof(OrbitAudioRing));
    if (ring) {
        atomic_init(&ring->head, 0);
        atomic_init(&ring->tail, 0);
        atomic_init(&ring->admitted_frames, 0);
        atomic_init(&ring->dropped_frames, 0);
        atomic_init(&ring->capture_cutoff_ns, UINT64_MAX);
        atomic_init(&ring->timing_failed, false);
        atomic_init(&ring->callbacks_in_flight, 0);
        atomic_init(&ring->producer_state, 0);
    }
    return ring;
}

void orbit_ring_set_capture_cutoff_ns(OrbitAudioRing *ring, uint64_t ns) {
    uint64_t previous = atomic_load_explicit(&ring->capture_cutoff_ns, memory_order_acquire);
    while (ns < previous && !atomic_compare_exchange_weak_explicit(
        &ring->capture_cutoff_ns, &previous, ns, memory_order_acq_rel, memory_order_acquire)) {}
}

void orbit_ring_mark_timing_failure(OrbitAudioRing *ring) {
    atomic_store_explicit(&ring->timing_failed, true, memory_order_release);
}

bool orbit_ring_has_timing_failure(OrbitAudioRing *ring) {
    return atomic_load_explicit(&ring->timing_failed, memory_order_acquire);
}

void orbit_ring_callback_begin(OrbitAudioRing *ring) {
    atomic_fetch_add_explicit(&ring->callbacks_in_flight, 1, memory_order_acq_rel);
}

void orbit_ring_callback_end(OrbitAudioRing *ring) {
    atomic_fetch_sub_explicit(&ring->callbacks_in_flight, 1, memory_order_release);
}

/* Integer sample-start comparison, without overflow or a floating-point clock
   assumption. At most the prefix starting before cutoff is admitted. */
static uint32_t frames_before_cutoff(OrbitAudioRing *ring, uint32_t frames,
                                     uint32_t rate, uint64_t capture_ns) {
    uint64_t cutoff = atomic_load_explicit(&ring->capture_cutoff_ns, memory_order_acquire);
    if (cutoff == UINT64_MAX) return frames;
    if (capture_ns >= cutoff) return 0;
    uint64_t delta = cutoff - capture_ns;
    uint64_t full_duration = ((uint64_t)frames * 1000000000ULL + rate - 1) / rate;
    if (delta >= full_duration) return frames;
    /* delta < duration of a uint32 frame count: this product fits uint64. */
    uint64_t prefix = (delta * rate + 999999999ULL) / 1000000000ULL;
    return prefix < frames ? (uint32_t)prefix : frames;
}
void orbit_ring_destroy(OrbitAudioRing *ring) { free(ring); }
void orbit_ring_set_enabled(OrbitAudioRing *ring, bool enabled) {
    if (!enabled) atomic_fetch_or_explicit(&ring->producer_state, 2, memory_order_acq_rel);
    else {
        unsigned expected = 2;
        atomic_compare_exchange_strong_explicit(&ring->producer_state, &expected, 0,
                                                memory_order_acq_rel, memory_order_acquire);
    }
}

bool orbit_ring_is_quiescent(OrbitAudioRing *ring) {
    return atomic_load_explicit(&ring->callbacks_in_flight, memory_order_acquire) == 0 &&
           (atomic_load_explicit(&ring->producer_state, memory_order_acquire) & 1) == 0;
}

static bool enter_producer(OrbitAudioRing *ring, uint32_t frames) {
    unsigned expected = 0;
    bool admitted = atomic_compare_exchange_strong_explicit(&ring->producer_state, &expected, 1,
                                                           memory_order_acq_rel, memory_order_acquire);
    if (!admitted && !(expected & 2)) {
        atomic_fetch_add_explicit(&ring->dropped_frames, frames, memory_order_relaxed);
    }
    return admitted;
}

static void leave_producer(OrbitAudioRing *ring) {
    atomic_fetch_and_explicit(&ring->producer_state, ~1u, memory_order_release);
}

#ifdef ORBIT_RING_TEST_HOOKS
extern void orbit_ring_test_after_reserve(OrbitAudioRing *ring);
#endif

static OrbitAudioBlock *reserve(OrbitAudioRing *ring, uint32_t frames) {
    uint64_t head = atomic_load_explicit(&ring->head, memory_order_relaxed);
    uint64_t tail = atomic_load_explicit(&ring->tail, memory_order_acquire);
    uint64_t sequence = ring->next_sequence++;
    if (frames > ORBIT_AUDIO_MAX_FRAMES || head - tail >= ORBIT_RING_CAPACITY) {
        atomic_fetch_add_explicit(&ring->dropped_frames, frames, memory_order_relaxed);
        return NULL;
    }
    OrbitAudioBlock *block = &ring->blocks[head % ORBIT_RING_CAPACITY];
    block->sequence = sequence;
    block->frames = frames;
    return block;
}

static bool has_capacity_for_chunks(OrbitAudioRing *ring, uint32_t frames) {
    uint64_t head = atomic_load_explicit(&ring->head, memory_order_relaxed);
    uint64_t tail = atomic_load_explicit(&ring->tail, memory_order_acquire);
    uint64_t occupied = head - tail;
    uint64_t chunks = ((uint64_t)frames + ORBIT_AUDIO_MAX_FRAMES - 1) / ORBIT_AUDIO_MAX_FRAMES;
    /* Capacity admission is all-or-nothing: producer ownership keeps another
       push from advancing head, and a consumer can only free slots. Once this
       preflight succeeds every chunk fits before the first one is published.
       A concurrent capture cutoff is distinct and may retain a pre-stop prefix. */
    if (occupied <= ORBIT_RING_CAPACITY && chunks <= ORBIT_RING_CAPACITY - occupied) return true;
    atomic_fetch_add_explicit(&ring->dropped_frames, frames, memory_order_relaxed);
    ring->next_sequence += chunks;
    return false;
}

static int16_t to_pcm(float sample) {
    if (!isfinite(sample)) return 0;
    if (sample >= 1.0f) return INT16_MAX;
    if (sample <= -1.0f) return INT16_MIN;
    return (int16_t)(sample * 32767.0f);
}

static uint64_t timestamp_after_frames(uint64_t timestamp, uint32_t frames,
                                       uint32_t rate) {
    uint64_t seconds = frames / rate;
    uint64_t remainder = frames % rate;
    uint64_t offset = seconds * 1000000000ULL + remainder * 1000000000ULL / rate;
    return timestamp > UINT64_MAX - offset ? UINT64_MAX : timestamp + offset;
}

bool orbit_ring_push_planar(OrbitAudioRing *ring, const float *const *planes,
                           uint32_t channels, uint32_t frames, uint32_t rate, uint64_t ns) {
    if (!ring || !planes || !channels || channels > 32 || !rate) return false;
    for (uint32_t ch = 0; ch < channels; ch++) if (!planes[ch]) return false;
    frames = frames_before_cutoff(ring, frames, rate, ns);
    if (!frames) return false;
    if (!enter_producer(ring, frames)) return false;
    if (!has_capacity_for_chunks(ring, frames)) { leave_producer(ring); return false; }
    uint32_t offset = 0;
    bool accepted = false;
    while (offset < frames) {
        uint32_t remaining = frames - offset;
        uint32_t chunk = remaining > ORBIT_AUDIO_MAX_FRAMES ? ORBIT_AUDIO_MAX_FRAMES : remaining;
        OrbitAudioBlock *block = reserve(ring, chunk);
        if (!block) {
            atomic_fetch_add_explicit(&ring->dropped_frames, remaining - chunk, memory_order_relaxed);
            leave_producer(ring);
            return false;
        }
#ifdef ORBIT_RING_TEST_HOOKS
        orbit_ring_test_after_reserve(ring);
#endif
        uint64_t chunk_ns = timestamp_after_frames(ns, offset, rate);
        block->sample_rate = rate;
        block->monotonic_ns = chunk_ns;
        for (uint32_t i = 0; i < chunk; i++) {
            float sample = 0;
            for (uint32_t ch = 0; ch < channels; ch++) sample += planes[ch][offset + i];
            block->samples[i] = to_pcm(sample / channels);
        }
        /* Stop can arrive while this callback copies. Recheck before publication. */
        block->frames = frames_before_cutoff(ring, chunk, rate, chunk_ns);
        if (!block->frames) {
            leave_producer(ring);
            return accepted;
        }
        uint32_t admitted_frames = block->frames;
        atomic_fetch_add_explicit(&ring->head, 1, memory_order_release);
        atomic_fetch_add_explicit(&ring->admitted_frames, admitted_frames, memory_order_relaxed);
        accepted = true;
        offset += block->frames;
        if (block->frames < chunk) break;
    }
    leave_producer(ring);
    return accepted;
}

bool orbit_ring_push_interleaved_timed(OrbitAudioRing *ring, const float *samples,
                                      uint32_t channels, uint32_t frames, uint32_t rate,
                                      uint64_t capture_ns, uint64_t ns) {
    if (!ring || !samples || !channels || channels > 32 || !rate) return false;
    frames = frames_before_cutoff(ring, frames, rate, capture_ns);
    if (!frames) return false;
    if (!enter_producer(ring, frames)) return false;
    if (!has_capacity_for_chunks(ring, frames)) { leave_producer(ring); return false; }
    uint32_t offset = 0;
    bool accepted = false;
    while (offset < frames) {
        uint32_t remaining = frames - offset;
        uint32_t chunk = remaining > ORBIT_AUDIO_MAX_FRAMES ? ORBIT_AUDIO_MAX_FRAMES : remaining;
        OrbitAudioBlock *block = reserve(ring, chunk);
        if (!block) {
            atomic_fetch_add_explicit(&ring->dropped_frames, remaining - chunk, memory_order_relaxed);
            leave_producer(ring);
            return false;
        }
#ifdef ORBIT_RING_TEST_HOOKS
        orbit_ring_test_after_reserve(ring);
#endif
        uint64_t chunk_capture_ns = timestamp_after_frames(capture_ns, offset, rate);
        block->sample_rate = rate;
        block->monotonic_ns = timestamp_after_frames(ns, offset, rate);
        for (uint32_t i = 0; i < chunk; i++) {
            float sample = 0;
            size_t frame = (size_t)offset + i;
            for (uint32_t ch = 0; ch < channels; ch++) sample += samples[frame * channels + ch];
            block->samples[i] = to_pcm(sample / channels);
        }
        block->frames = frames_before_cutoff(ring, chunk, rate, chunk_capture_ns);
        if (!block->frames) {
            leave_producer(ring);
            return accepted;
        }
        uint32_t admitted_frames = block->frames;
        atomic_fetch_add_explicit(&ring->head, 1, memory_order_release);
        atomic_fetch_add_explicit(&ring->admitted_frames, admitted_frames, memory_order_relaxed);
        accepted = true;
        offset += block->frames;
        if (block->frames < chunk) break;
    }
    leave_producer(ring);
    return accepted;
}

bool orbit_ring_pop(OrbitAudioRing *ring, OrbitAudioBlock *block) {
    uint64_t tail = atomic_load_explicit(&ring->tail, memory_order_relaxed);
    if (tail == atomic_load_explicit(&ring->head, memory_order_acquire)) return false;
    memcpy(block, &ring->blocks[tail % ORBIT_RING_CAPACITY], sizeof(*block));
    atomic_fetch_add_explicit(&ring->tail, 1, memory_order_release);
    return true;
}
uint64_t orbit_ring_admitted_frames(OrbitAudioRing *ring) {
    return atomic_load_explicit(&ring->admitted_frames, memory_order_relaxed);
}
uint64_t orbit_ring_take_dropped_frames(OrbitAudioRing *ring) {
    return atomic_exchange_explicit(&ring->dropped_frames, 0, memory_order_relaxed);
}
