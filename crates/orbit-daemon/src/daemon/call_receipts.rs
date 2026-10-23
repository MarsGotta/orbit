//! What the helper says about the call's audio: the start's readback, which
//! has to prove voice processing and its shared reference, and the voice
//! capabilities it reports.
use super::*;

#[derive(Deserialize)]
pub(super) struct CallStartReceipt {
    pub(super) stream_id: String,
    pub(super) processing: String,
    pub(super) processing_enabled: bool,
    pub(super) processing_bypassed: bool,
    pub(super) agc_enabled: bool,
    pub(super) playback_reference: String,
    pub(super) pipeline_id: String,
}

#[derive(Deserialize)]
pub(super) struct VoiceCapabilities {
    #[serde(default)]
    pub(super) supported_audio_processing: Option<Vec<String>>,
    #[serde(default)]
    pub(super) voice_processing_available: Option<bool>,
    #[serde(default)]
    pub(super) playback_reference_available: Option<bool>,
}

#[derive(Deserialize)]
pub(super) struct ProcessingEvidence<'a> {
    pub(super) processing: &'a str,
    pub(super) processing_enabled: bool,
    pub(super) processing_bypassed: bool,
    pub(super) agc_enabled: bool,
    pub(super) playback_reference: &'a str,
    pub(super) pipeline_id: &'a str,
}

pub(super) fn processing_pipeline(value: &Value) -> Option<&str> {
    let evidence = ProcessingEvidence::deserialize(value).ok()?;
    if evidence.processing != "voice_processing"
        || !evidence.processing_enabled
        || evidence.processing_bypassed
        || evidence.agc_enabled
        || evidence.playback_reference != "shared_engine"
    {
        return None;
    }
    Some(evidence.pipeline_id).filter(|id| !id.is_empty() && id.len() <= 128)
}

impl Runtime {
    pub(in crate::daemon) async fn call_start_response(
        &mut self,
        command_id: &str,
        result: &Value,
    ) -> Result<bool> {
        let Some(call) = self.audio.call.as_ref() else {
            return Ok(false);
        };
        if call.start_request != command_id {
            return Ok(false);
        }
        if call.closing_at_ms.is_some() || self.state.durable.voice.call.is_none() {
            // The start can open after a stop overtook it. Let the retired
            // start path close the stream identified by this late receipt.
            return Ok(false);
        }
        let readback: CallStartReceipt = decode_result_body("audio_start", result)?;
        if Some(readback.stream_id.as_str()) != self.audio.mic_stream.as_deref() {
            bail!("La confirmación de llamada corresponde a otro stream");
        }
        if readback.processing != "voice_processing"
            || !readback.processing_enabled
            || readback.processing_bypassed
            || readback.agc_enabled
            || readback.playback_reference != "shared_engine"
            || readback.pipeline_id.is_empty()
            || readback.pipeline_id.len() > 128
        {
            bail!("El Mac no acreditó la ruta VoiceProcessing y su referencia compartida");
        }
        let pipeline = readback.pipeline_id;
        // What the microphone gives under voice processing: its channels and
        // rate, before the helper mixes them to the call's mono stream.
        self.log(
            "call_audio_opened",
            &[
                ("input_channels", &result["input_channels"].to_string()),
                ("sample_rate", &result["sample_rate"].to_string()),
            ],
        );
        let recording = call.authorization.id.clone();
        let job = Job {
            id: format!("call-processing-{recording}"),
            kind: orbit_store::QueueKind::AudioProcessingReceipt.into(),
            state: JobState::Succeeded,
            created_at: now(),
            payload: json!({"recording_id":recording,
                "observed":result,"aec_hardware_validated":false,"origin":"native_start_readback"}),
        };
        self.store_actor
            .run(move |store| Ok(store.enqueue_job(&job)?))
            .await?;
        let call = self.audio.call.as_mut().context("Llamada perdida")?;
        call.pipeline_id = Some(pipeline);
        call.last_audio = Instant::now();
        self.audio
            .pending_starts
            .retain(|pending| pending != command_id);
        self.state.set_voice_processing_available(true);
        self.state.set_playback_reference_available(true);
        self.apply(Event::CallReady).await?;
        Ok(true)
    }

    /// Supported modes permit only a user-requested attempt. Availability flags
    /// are kept separate from per-stream processing proof and hardware efficacy.
    pub(in crate::daemon) fn observe_voice_capabilities(&mut self, value: &Value) -> Result<()> {
        let previous = self.state.capabilities().clone();
        let observed: VoiceCapabilities = decode_result_body("voice_capabilities", value)?;
        if let Some(modes) = observed.supported_audio_processing {
            self.state.set_voice_processing_supported(
                modes.iter().any(|mode| mode == "voice_processing"),
            );
        }
        if let Some(available) = observed.voice_processing_available {
            self.state.set_voice_processing_available(available);
        }
        if let Some(available) = observed.playback_reference_available {
            self.state.set_playback_reference_available(available);
        }
        if previous != *self.state.capabilities() {
            self.store_actor.defer_snapshot(&self.state)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod result_fixture_tests {
    use super::*;

    #[test]
    fn call_start_and_ready_fixtures_use_the_call_readers() -> Result<()> {
        let start: Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/native/results/audio_start-mic.json"
        ))?;
        let receipt: CallStartReceipt = decode_result_body("audio_start", &start)?;
        assert_eq!(receipt.stream_id, "stream-1");
        assert_eq!(receipt.processing, "voice_processing");
        assert!(receipt.processing_enabled);
        assert!(!receipt.processing_bypassed);
        assert!(!receipt.agc_enabled);
        assert_eq!(receipt.playback_reference, "shared_engine");
        assert_eq!(receipt.pipeline_id, "fixture");
        let ready: Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/native/events/ready.json"
        ))?;
        let facts: VoiceCapabilities = decode_result_body("voice_capabilities", &ready)?;
        assert_eq!(facts.voice_processing_available, Some(false));
        assert_eq!(facts.playback_reference_available, Some(false));
        assert!(
            facts
                .supported_audio_processing
                .context("processing modes")?
                .iter()
                .any(|mode| mode == "voice_processing")
        );
        Ok(())
    }
}
