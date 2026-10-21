//! A bounded retry gate: successful durable work drains immediately; failures
//! back off without retrying prompts or desktop effects.
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct RetryGate {
    failures: u32,
    next: Option<Instant>,
}

impl RetryGate {
    pub fn ready(&self, clock: Instant) -> bool {
        self.next.is_none_or(|deadline| clock >= deadline)
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

#[derive(Debug, thiserror::Error)]
#[error("Transferencia rechazada ({0}); conservada localmente para revisión")]
pub struct Refused(pub u16);

pub fn check_status(status: reqwest::StatusCode) -> anyhow::Result<()> {
    if status.is_success() {
        return Ok(());
    }
    if matches!(
        status.as_u16(),
        400 | 401 | 403 | 404 | 409 | 413 | 415 | 422
    ) {
        return Err(Refused(status.as_u16()).into());
    }
    anyhow::bail!("Transferencia pendiente ({status})")
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
