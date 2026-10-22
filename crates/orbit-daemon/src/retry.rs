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

#[derive(Debug, thiserror::Error)]
#[error("Transferencia rechazada ({0}); conservada localmente para revisión")]
pub struct Refused(pub u16);

/// The hub's own code for a refusal (`recording_finalized`,
/// `chunk_conflict`, `unauthorized`…), kept beside [`Refused`] in the error
/// chain so the person is told which rule fired, not only the HTTP status.
#[derive(Debug, thiserror::Error)]
#[error("código del hub: {0}")]
pub struct HubCode(pub String);

/// The hub has no room for this upload (HTTP 507): its code and the
/// numbers it gave. The audio stays on the Mac; nothing is lost, it waits.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("El hub no tiene espacio ({code}): el audio se queda en el Mac hasta que lo haya")]
pub struct OutOfSpace {
    /// `quota_exceeded`, `storage_full`…, or `insufficient_storage` (the
    /// status's own name) when the hub's body named none.
    pub code: String,
    pub used_bytes: Option<u64>,
    pub max_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
}

impl OutOfSpace {
    fn unexplained() -> Self {
        Self {
            code: "insufficient_storage".into(),
            used_bytes: None,
            max_bytes: None,
            free_bytes: None,
        }
    }
}

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
    if status == reqwest::StatusCode::INSUFFICIENT_STORAGE {
        return Err(OutOfSpace::unexplained().into());
    }
    anyhow::bail!("Transferencia pendiente ({status})")
}

/// [`check_status`] for an upload, with what the hub said: a refusal keeps
/// the hub's code ([`HubCode`]) and a hub out of space its numbers. A body
/// that cannot be read leaves the status alone, never a guess.
pub async fn check_response(response: reqwest::Response) -> anyhow::Result<reqwest::Response> {
    let status = response.status();
    let Err(error) = check_status(status) else {
        return Ok(response);
    };
    let Ok(said) =
        crate::hub::read_refusal(response, orbit_core::limits::MAX_HUB_HTTP_BODY_BYTES).await
    else {
        return Err(error);
    };
    let code = said
        .code
        .as_deref()
        .or_else(|| said.details["code"].as_str())
        .and_then(crate::hub::hub_code)
        .map(str::to_owned);
    if error.is::<OutOfSpace>() {
        return Err(OutOfSpace {
            code: code.unwrap_or_else(|| OutOfSpace::unexplained().code),
            used_bytes: said.numbers.get("used_bytes").copied(),
            max_bytes: said.numbers.get("max_bytes").copied(),
            // What is left: of the quota (`remaining_bytes`), or of the
            // disk that refused the write (`storage_full`'s `free_bytes`).
            free_bytes: said
                .numbers
                .get("remaining_bytes")
                .or_else(|| said.numbers.get("free_bytes"))
                .copied(),
        }
        .into());
    }
    match code {
        Some(code) => Err(error.context(HubCode(code))),
        None => Err(error),
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
