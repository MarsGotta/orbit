//! The bodies of the helper's answers that Orbit reads field by field.
use super::*;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeStateResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_languages: Option<Vec<String>>,
    pub privacy_stopped: bool,
    pub connected: bool,
    pub native_instance_id: String,
    pub privacy_generation: u64,
    pub selected_text_available: bool,
    pub microphone: bool,
    pub system_audio: bool,
    pub playing: bool,
    pub supported_audio_processing: Vec<String>,
    pub voice_processing_available: bool,
    pub playback_reference_available: bool,
    pub audio_processing: NativeAudioProcessingState,
    #[serde(deserialize_with = "read_native_calendar_status")]
    pub calendar: CalendarStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<PermissionsResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drain_complete: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drain: Option<String>,
}

#[derive(Deserialize)]
struct ReceivedCalendarStatus {
    authorization: CalendarAuthorization,
    physical_read_pending: bool,
    #[serde(default, deserialize_with = "native_present")]
    failure: Option<CalendarFailure>,
}

fn native_present<'de, D, T>(decoder: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(decoder).map(Some)
}

fn read_native_calendar_status<'de, D>(decoder: D) -> Result<CalendarStatus, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let received = ReceivedCalendarStatus::deserialize(decoder)?;
    Ok(CalendarStatus {
        authorization: received.authorization,
        physical_read_pending: received.physical_read_pending,
        failure: received.failure,
    })
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(transparent)]
pub struct NativeCalendarStatusBody(
    #[serde(deserialize_with = "read_native_calendar_status")] pub CalendarStatus,
);

result_struct!(NativeAudioProcessingState {
    mode: String,
    enabled: bool,
    bypassed: bool,
    agc_enabled: bool,
    playback_reference: String,
    pipeline_id: String,
    engine_running: bool,
    render_sample_rate: u32,
});
result_struct!(PermissionsResult {
    microphone: String,
    screen: bool,
    accessibility: bool,
    calendars: String
});
result_struct!(SelectedTextResult {
    binding: WindowBinding,
    clock_domain: String,
    capture_started_monotonic_ns: u64,
    capture_completed_monotonic_ns: u64,
    mime_type: String,
    source: String,
    text: String,
    range: SelectedTextRange,
    ax_binding_quality: String,
    selection_scope: String,
    sensitivity: String
});
