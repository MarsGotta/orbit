#ifndef ORBIT_AUDIO_RING_H
#define ORBIT_AUDIO_RING_H
#include <stdint.h>
#include <stdbool.h>

#define ORBIT_AUDIO_MAX_FRAMES 4096
typedef struct OrbitAudioRing OrbitAudioRing;
typedef struct {
    uint64_t sequence;
    uint64_t monotonic_ns;
    uint32_t frames;
    uint32_t sample_rate;
    int16_t samples[ORBIT_AUDIO_MAX_FRAMES];
} OrbitAudioBlock;

OrbitAudioRing *orbit_ring_create(void);
void orbit_ring_destroy(OrbitAudioRing *ring);
void orbit_ring_set_enabled(OrbitAudioRing *ring, bool enabled);
/* Retain samples strictly before this capture-clock instant, including callbacks
   delivered later. Repeated stops can only move the cutoff earlier. For planar
   input this is host nanoseconds; timed interleaved input supplies its own clock. */
void orbit_ring_set_capture_cutoff_ns(OrbitAudioRing *ring, uint64_t cutoff_ns);
void orbit_ring_mark_timing_failure(OrbitAudioRing *ring);
bool orbit_ring_has_timing_failure(OrbitAudioRing *ring);
/* Bracket the whole platform callback, including clock/format inspection before
   a push. No allocation or lock; final drain waits for these callbacks too. */
void orbit_ring_callback_begin(OrbitAudioRing *ring);
void orbit_ring_callback_end(OrbitAudioRing *ring);
/* After disabling admission, wait for true before the final consumer drain.
   Does not block. Owners must retain the ring while any callback can access it. */
bool orbit_ring_is_quiescent(OrbitAudioRing *ring);
bool orbit_ring_push_planar(OrbitAudioRing *ring, const float *const *planes,
                           uint32_t channels, uint32_t frames, uint32_t sample_rate,
                           uint64_t monotonic_ns);
bool orbit_ring_push_interleaved_timed(OrbitAudioRing *ring, const float *samples,
                                      uint32_t channels, uint32_t frames, uint32_t sample_rate,
                                      uint64_t capture_ns, uint64_t monotonic_ns);
bool orbit_ring_pop(OrbitAudioRing *ring, OrbitAudioBlock *block);
uint64_t orbit_ring_admitted_frames(OrbitAudioRing *ring);
uint64_t orbit_ring_take_dropped_frames(OrbitAudioRing *ring);
#endif
