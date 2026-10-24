//! A voice turn's audio, transcribed by the hub (`POST
//! /v1/audio/transcribe`), with every failure named by its cause.
use super::*;
use orbit_core::VoiceTranscriptionFailure as Failure;

#[derive(Deserialize)]
struct SpeechResponse {
    text: String,
}

pub async fn transcribe(
    settings: &Settings,
    token: &str,
    development: bool,
    wav: Vec<u8>,
    op: &Op,
) -> std::result::Result<String, Failure> {
    let base = validate_hub_url(
        settings
            .hub_url
            .as_deref()
            .ok_or_else(|| failure("transcription_hub_not_configured", None))?,
        development,
    )
    .map_err(|_| failure("transcription_hub_invalid_url", None))?;
    let url = base
        .join("v1/audio/transcribe")
        .map_err(|_| failure("transcription_hub_invalid_url", None))?;
    let response = request(reqwest::Method::POST, url, Some(token), op)
        .map_err(|_| failure("transcription_hub_client_error", None))?
        .timeout(VOICE_HUB_HTTP_TIMEOUT)
        .header("Content-Type", "audio/wav")
        .body(wav)
        .send()
        .await
        .map_err(|error| unanswered(&error))?;
    if !response.status().is_success() {
        return Err(refused(response).await);
    }
    let status = response.status().as_u16();
    let value: SpeechResponse = serde_json::from_value(
        bounded_json(response)
            .await
            .map_err(|error| invalid_response(error, status))?,
    )
    .map_err(|_| {
        let mut result = failure("transcription_hub_invalid_response", Some(status));
        result.attempts = Some(1);
        result
    })?;
    Ok(value.text)
}

/// A failure named by `code`, with the hub's status when it answered.
fn failure(code: &str, hub_status: Option<u16>) -> Failure {
    Failure {
        code: code.to_owned(),
        provider_status: None,
        hub_status,
        timeout_ms: None,
        bytes: None,
        max_bytes: None,
        attempts: None,
    }
}

/// An answer that does not read as one, or too large to read.
fn invalid_response(error: anyhow::Error, status: u16) -> Failure {
    let mut result = failure("transcription_hub_invalid_response", Some(status));
    if let Some(size) = error.downcast_ref::<HubBodyTooLarge>() {
        result.code = "transcription_hub_response_too_large".into();
        result.bytes = Some(size.bytes);
        result.max_bytes = Some(size.max_bytes);
    }
    result.attempts = Some(1);
    result
}

/// The request got no answer: why, and which deadline fired.
fn unanswered(error: &reqwest::Error) -> Failure {
    let mut result = failure(
        match TransportCause::of(error) {
            TransportCause::ConnectionTimeout => "transcription_hub_connection_timeout",
            TransportCause::Timeout => "transcription_hub_timeout",
            TransportCause::Connection => "transcription_hub_connection_error",
            TransportCause::Cut => "transcription_hub_transport_error",
        },
        None,
    );
    if error.is_timeout() {
        // This reports the daemon→hub bound that fired, not elapsed
        // time and not either of the hub→Whisper budgets.
        result.timeout_ms = Some(
            if error.is_connect() {
                HUB_CONNECT_TIMEOUT
            } else {
                VOICE_HUB_HTTP_TIMEOUT
            }
            .as_millis() as u64,
        );
    }
    result.attempts = Some(1);
    result
}

/// The hub refused the audio: its code and numbers, as it said them.
async fn refused(response: reqwest::Response) -> Failure {
    let status = response.status().as_u16();
    let value = match read_refusal(response, MAX_HUB_HTTP_BODY_BYTES).await {
        Ok(value) => value,
        Err(error) => return invalid_response(error, status),
    };
    let code = value.details["code"]
        .as_str()
        .or(value.code.as_deref())
        .and_then(hub_code)
        .unwrap_or("transcription_hub_invalid_response");
    let number = |key: &str| value.numbers.get(key).copied();
    let mut result = failure(code, Some(status));
    result.provider_status = number("provider_status")
        .and_then(|number| u16::try_from(number).ok())
        .filter(|status| (100..=599).contains(status));
    result.timeout_ms = number("timeout_ms");
    result.bytes = number("bytes");
    result.max_bytes = number("max_bytes");
    result.attempts = number("attempts")
        .and_then(|number| u32::try_from(number).ok())
        .or(Some(1));
    result
}

#[cfg(test)]
mod voice_transport_tests {
    use super::*;

    #[tokio::test]
    async fn missing_and_invalid_hub_urls_have_distinct_causes() -> Result<()> {
        let missing = Settings::default();
        let invalid = Settings {
            hub_url: Some("not a hub URL".into()),
            ..Settings::default()
        };
        let Err(missing_failure) =
            transcribe(&missing, "synthetic", true, vec![0], &orbit_ipc::new_op()).await
        else {
            anyhow::bail!("missing hub URL unexpectedly transcribed voice")
        };
        let Err(invalid_failure) =
            transcribe(&invalid, "synthetic", true, vec![0], &orbit_ipc::new_op()).await
        else {
            anyhow::bail!("invalid hub URL unexpectedly transcribed voice")
        };
        assert_eq!(missing_failure.code, "transcription_hub_not_configured");
        assert_eq!(invalid_failure.code, "transcription_hub_invalid_url");
        Ok(())
    }

    #[tokio::test]
    async fn an_unreachable_hub_is_not_reported_as_unreachable_whisper() -> Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        drop(listener);
        let settings = Settings {
            hub_url: Some(url),
            ..Settings::default()
        };
        let Err(failure) =
            transcribe(&settings, "synthetic", true, vec![0], &orbit_ipc::new_op()).await
        else {
            bail!("the closed loopback port unexpectedly accepted the request");
        };
        assert_eq!(failure.code, "transcription_hub_connection_error");
        assert_eq!(failure.attempts, Some(1));
        assert_eq!(failure.provider_status, None);
        Ok(())
    }
}
