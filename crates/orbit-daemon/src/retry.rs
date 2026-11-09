//! A bounded retry gate: successful durable work drains immediately; failures
//! back off without retrying prompts or desktop effects.
use std::time::{Duration, Instant};

pub use orbit_hub_client::hub::{HubCode, OutOfSpace, Refused, check_response, check_status};

#[derive(Default)]
pub struct RetryGate {
    failures: u32,
    next: Option<Instant>,
}

impl RetryGate {
    pub fn ready(&self, clock: Instant) -> bool {
        self.next.is_none_or(|deadline| clock >= deadline)
    }

    /// Hold every transfer until at least `clock + wait`: a hub out of space
    /// does not free itself in seconds. The next success lifts it.
    pub fn hold(&mut self, wait: Duration, clock: Instant) {
        let until = clock + wait;
        self.next = Some(self.next.map_or(until, |next| next.max(until)));
    }

    pub fn finish(&mut self, success: bool, clock: Instant) {
        if success {
            self.failures = 0;
            self.next = None;
        } else {
            self.failures = self.failures.saturating_add(1);
            let seconds = (1u64 << self.failures.min(6)).min(60);
            self.next = Some(clock + Duration::from_secs(seconds));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_are_bounded_and_success_has_no_artificial_throttle() {
        let clock = Instant::now();
        let mut gate = RetryGate::default();
        assert!(gate.ready(clock));
        for _ in 0..100 {
            gate.finish(false, clock);
        }
        assert!(!gate.ready(clock + Duration::from_secs(59)));
        assert!(gate.ready(clock + Duration::from_secs(60)));
        gate.finish(true, clock);
        assert!(gate.ready(clock));
        gate.finish(false, clock);
        assert!(gate.ready(clock + Duration::from_secs(2)));
    }
}
