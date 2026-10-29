import Foundation

// The fields of each event the helper sends, under their envelope.
public struct NativeEventEnvelope<Fields: Encodable>: Encodable {
    public let v: Int
    public let fields: Fields
    private let event: String
    private enum CodingKeys: String, CodingKey { case v, event }

    public init(event: String, fields: Fields) {
        self.v = NativeProtocol.version
        self.event = event
        self.fields = fields
    }

    public func encode(to encoder: Encoder) throws {
        var header = encoder.container(keyedBy: CodingKeys.self)
        try header.encode(v, forKey: .v)
        try header.encode(event, forKey: .event)
        try fields.encode(to: encoder)
    }
}

public struct NativeReadyEventFields: Encodable {
    public let preferred_languages: [String]?
    public let `protocol`: Int
    public let platform: String
    public let selected_text_available: Bool
    public let supported_audio_processing: [String]
    public let voice_processing_available: Bool
    public let playback_reference_available: Bool
    public let launch_mode: String
    public let fixture: Bool?
}
/// `detail` carries a refusal's names and numbers by code (the calendar
/// write refusals); absent from every other error.
public struct NativeWireError: Encodable {
    public let code: String
    public let message: String
    public let detail: [String: NativeJSON]?
    public init(code: String, message: String, detail: [String: NativeJSON]? = nil) {
        self.code = code; self.message = message; self.detail = detail
    }
}
public struct NativeResponseEventFields: Encodable {
    public let id: String
    public let ok: Bool
    public let result: [String: NativeJSON]?
    public let error: NativeWireError?
}
public struct NativeErrorEventFields: Encodable {
    public let code: String
    public let os_domain: String?
    public let os_status: Int?
}
public struct NativeWireMeetingApplication: Encodable {
    public let application_pid: Int32
    public let bundle_id: String
    public let name: String
}
public struct NativeHotkeyEventFields: Encodable {
    public let action: String
    public let phase: String
    public let applications: [NativeWireMeetingApplication]?
}
/// The applications holding the microphone now, sorted by process id; empty
/// when none does. Sent only when the set changes.
public struct NativeMicrophoneActivityFields: Encodable {
    public let applications: [NativeWireMeetingApplication]?
    public let os_status: Int32?
}
/// The question the user asked after «Oye Deimos» on the meeting's
/// microphone, as text. `started_ms`/`ended_ms` are Unix milliseconds of the
/// phrase's start and the question's end. Nothing else heard is sent.
public struct NativeAssistantQuestionFields: Encodable {
    public let stream_id: String
    public let text: String
    public let started_ms: Int64
    public let ended_ms: Int64
    /// The name heard, exactly as `audio_wake_phrase` gave it, so the
    /// question goes to that agent. Omitted when absent.
    public let assistant_name: String?
}
/// Where the «Oye Deimos» detector is: `unavailable` (with `cause`
/// and `os_version`), `idle`, `downloading` (with `percent`), `listening` or
/// `failed` (with `cause` and Apple's `detail`, never what was heard).
public struct NativeVoiceTriggerFields: Encodable {
    public let state: String
    public let cause: String?
    public let detail: String?
    public let percent: UInt8?
    public let os_version: String?
}
/// A step the helper reports that answers no order, as a code the
/// daemon records (`wake_phrase_heard`, `wake_phrase_abandoned`,
/// `wake_phrase_question_dropped`). Never words that were said.
public struct NativeNoticeEventFields: Encodable {
    public let code: String
    public let abandonment: WakePhraseAbandonment?
}
public struct NativeStopEventFields: Encodable {
    public let reason: String
    public let privacy_stopped: Bool
    public let drain_complete: Bool
}
public struct NativeRearmEventFields: Encodable { public let privacy_stopped: Bool }
public struct NativeCalendarInvalidatedFields: Encodable { public let code: String }
public struct NativeGapEventFields: Encodable {
    public let source: String
    public let stream_id: String
    public let reason: String
    public let frames: UInt64
    public let pre_stop_possible: Bool?
    public let duration_unknown: Bool?
    public let duration_ns: UInt64?
}
public struct NativeAudioEventFields: Encodable {
    public let source: String
    public let stream_id: String
    public let sequence: UInt64
    public let sample_rate: UInt32
    public let channels: UInt8
    public let frames: UInt32
    public let monotonic_ns: UInt64
    public let clock_domain: String
    public let pcm_s16le_base64: String
    public let processing: String
    public let processing_enabled: Bool?
    public let processing_bypassed: Bool?
    public let agc_enabled: Bool?
    public let playback_reference: String?
    public let pipeline_id: String?
    public let playback_active: Bool?
    public let playback_id: String?

    private enum CodingKeys: String, CodingKey {
        case source, stream_id, sequence, sample_rate, channels, frames, monotonic_ns
        case clock_domain, pcm_s16le_base64, processing, processing_enabled
        case processing_bypassed, agc_enabled, playback_reference, pipeline_id
        case playback_active, playback_id
    }

    public func encode(to encoder: Encoder) throws {
        var fields = encoder.container(keyedBy: CodingKeys.self)
        try fields.encode(source, forKey: .source)
        try fields.encode(stream_id, forKey: .stream_id)
        try fields.encode(sequence, forKey: .sequence)
        try fields.encode(sample_rate, forKey: .sample_rate)
        try fields.encode(channels, forKey: .channels)
        try fields.encode(frames, forKey: .frames)
        try fields.encode(monotonic_ns, forKey: .monotonic_ns)
        try fields.encode(clock_domain, forKey: .clock_domain)
        try fields.encode(pcm_s16le_base64, forKey: .pcm_s16le_base64)
        try fields.encode(processing, forKey: .processing)
        try fields.encodeIfPresent(processing_enabled, forKey: .processing_enabled)
        try fields.encodeIfPresent(processing_bypassed, forKey: .processing_bypassed)
        try fields.encodeIfPresent(agc_enabled, forKey: .agc_enabled)
        try fields.encodeIfPresent(playback_reference, forKey: .playback_reference)
        try fields.encodeIfPresent(pipeline_id, forKey: .pipeline_id)
        try fields.encodeIfPresent(playback_active, forKey: .playback_active)
        if playback_active != nil && playback_id == nil {
            try fields.encodeNil(forKey: .playback_id)
        } else {
            try fields.encodeIfPresent(playback_id, forKey: .playback_id)
        }
    }
}
public struct NativePlaybackProgressFields: Encodable {
    public let playback_id: String
    public let state: String
    public let playback_active: Bool
    public let sequence: UInt64?
    public let frames: UInt32?
    public let rendered_frames: UInt32?
    public let remaining_frames: UInt32?
    public let played_frames: UInt32?
    public let mime_type: String?
    public let provider: String?

    private enum CodingKeys: String, CodingKey {
        case playback_id, state, playback_active, sequence, frames, rendered_frames
        case remaining_frames, played_frames, mime_type, provider
    }

    public func encode(to encoder: Encoder) throws {
        var fields = encoder.container(keyedBy: CodingKeys.self)
        try fields.encode(playback_id, forKey: .playback_id)
        try fields.encode(state, forKey: .state)
        try fields.encode(playback_active, forKey: .playback_active)
        try fields.encodeIfPresent(sequence, forKey: .sequence)
        try fields.encodeIfPresent(frames, forKey: .frames)
        try fields.encodeIfPresent(rendered_frames, forKey: .rendered_frames)
        try fields.encodeIfPresent(remaining_frames, forKey: .remaining_frames)
        if state == "cancelled" && played_frames == nil {
            try fields.encodeNil(forKey: .played_frames)
        } else {
            try fields.encodeIfPresent(played_frames, forKey: .played_frames)
        }
        try fields.encodeIfPresent(mime_type, forKey: .mime_type)
        try fields.encodeIfPresent(provider, forKey: .provider)
    }
}

public struct NativeShortcutFailure: Encodable, Equatable {
    public let combination: String
    public let os_status: Int32
    public init(combination: String, osStatus: Int32) {
        self.combination = combination; self.os_status = osStatus
    }
}

public struct NativeStateEventFields: Encodable {
    public let preferred_languages: [String]?
    public let privacy_stopped: Bool
    public let connected: Bool
    public let native_instance_id: String
    public let privacy_generation: UInt64
    public let selected_text_available: Bool
    public let microphone: Bool
    public let system_audio: Bool
    public let playing: Bool
    public let shortcut_failures: [NativeShortcutFailure]
    public let supported_audio_processing: [String]
    public let voice_processing_available: Bool
    public let playback_reference_available: Bool
    public let audio_processing: NativeAudioProcessingResultFields
    public let calendar: NativeCalendarStatusResultFields
}
public struct NativeAudioProcessingResultFields: Encodable {
    public let mode: String
    public let enabled: Bool
    public let bypassed: Bool
    public let agc_enabled: Bool
    public let playback_reference: String
    public let pipeline_id: String
    public let engine_running: Bool
    public let render_sample_rate: UInt32
    public let other_audio_ducking_level: UInt?
    public let other_audio_ducking_advanced: Bool?

    public init(mode: String, enabled: Bool, bypassed: Bool, agcEnabled: Bool,
        playbackReference: String, pipelineID: String, engineRunning: Bool,
        renderSampleRate: UInt32, otherAudioDuckingLevel: UInt? = nil,
        otherAudioDuckingAdvanced: Bool? = nil) {
        self.mode = mode; self.enabled = enabled; self.bypassed = bypassed
        self.agc_enabled = agcEnabled; self.playback_reference = playbackReference
        self.pipeline_id = pipelineID; self.engine_running = engineRunning
        self.render_sample_rate = renderSampleRate
        self.other_audio_ducking_level = otherAudioDuckingLevel
        self.other_audio_ducking_advanced = otherAudioDuckingAdvanced
    }
}
public struct NativeCalendarStatusResultFields: Encodable {
    public let authorization: String
    public let physical_read_pending: Bool
    public let failure: String?

    public init(authorization: String, physicalReadPending: Bool, failure: String?) {
        self.authorization = authorization; self.physical_read_pending = physicalReadPending
        self.failure = failure
    }
}
