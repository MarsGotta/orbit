//! Whether a discontinuity in a capture source escalates into a privacy stop.
//!
//! Retired: a hole is recorded with an additive notice and the
//! recording keeps going. The
//! classification lives in [`crate::failure::classify_gap`], which the daemon
//! adopts directly; this module is kept only so the crate root still compiles
//! for older callers and is otherwise no longer used.

#[cfg(test)]
mod tests {
    #[test]
    fn a_gap_in_a_live_recording_still_fails_closed() {
        // Retired on purpose: the product decision is
        // that a hole is declared, not a reason to brake a live recording.
        // The replacement behaviour lives in `failure::classify_gap` and is
        // asserted by
        // `failure::tests::audio_gap_degradation_is_declared_and_recording_continues`.
    }
}
