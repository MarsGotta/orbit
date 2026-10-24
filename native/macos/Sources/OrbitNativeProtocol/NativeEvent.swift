import Foundation

// The events the helper sends, and how each is made.
public enum NativeEvent: Encodable {
    case ready(NativeEventEnvelope<NativeReadyEventFields>)
    case state(NativeEventEnvelope<NativeStateEventFields>)
    case response(NativeEventEnvelope<NativeResponseEventFields>)
    case error(NativeEventEnvelope<NativeErrorEventFields>)
    case hotkey(NativeEventEnvelope<NativeHotkeyEventFields>)
    case privacyStop(NativeEventEnvelope<NativeStopEventFields>)
    case captureStopped(NativeEventEnvelope<NativeStopEventFields>)
    case localRearm(NativeEventEnvelope<NativeRearmEventFields>)
    case calendarInvalidated(NativeEventEnvelope<NativeCalendarInvalidatedFields>)
    case gap(NativeEventEnvelope<NativeGapEventFields>)
    case audio(NativeEventEnvelope<NativeAudioEventFields>)
    case playbackProgress(NativeEventEnvelope<NativePlaybackProgressFields>)
    case microphoneActivity(NativeEventEnvelope<NativeMicrophoneActivityFields>)
    case assistantQuestion(NativeEventEnvelope<NativeAssistantQuestionFields>)
    case voiceTrigger(NativeEventEnvelope<NativeVoiceTriggerFields>)
    case notice(NativeEventEnvelope<NativeNoticeEventFields>)

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .ready(let value): try value.encode(to: encoder)
        case .state(let value): try value.encode(to: encoder)
        case .response(let value): try value.encode(to: encoder)
        case .error(let value): try value.encode(to: encoder)
        case .hotkey(let value): try value.encode(to: encoder)
        case .privacyStop(let value), .captureStopped(let value): try value.encode(to: encoder)
        case .localRearm(let value): try value.encode(to: encoder)
        case .calendarInvalidated(let value): try value.encode(to: encoder)
        case .gap(let value): try value.encode(to: encoder)
        case .audio(let value): try value.encode(to: encoder)
        case .playbackProgress(let value): try value.encode(to: encoder)
        case .microphoneActivity(let value): try value.encode(to: encoder)
        case .assistantQuestion(let value): try value.encode(to: encoder)
        case .voiceTrigger(let value): try value.encode(to: encoder)
        case .notice(let value): try value.encode(to: encoder)
        }
    }
    public var name: String {
        switch self {
        case .ready: "ready"
        case .state: "state"
        case .response: "response"
        case .error: "error"
        case .hotkey: "hotkey"
        case .privacyStop: "privacy_stop"
        case .captureStopped: "capture_stopped"
        case .localRearm: "local_rearm"
        case .calendarInvalidated: "calendar_invalidated"
        case .gap: "gap"
        case .audio: "audio"
        case .playbackProgress: "playback_progress"
        case .microphoneActivity: "microphone_activity"
        case .assistantQuestion: "assistant_question"
        case .voiceTrigger: "voice_trigger"
        case .notice: "notice"
        }
    }
    public static let names: Set<String> = [
        "ready", "state", "response", "error", "hotkey", "privacy_stop",
        "capture_stopped", "local_rearm", "calendar_invalidated", "gap",
        "audio", "playback_progress", "microphone_activity", "assistant_question",
        "voice_trigger", "notice"
    ]

    public static func makeResponse(id: String, result: [String: Any]) throws -> NativeEvent {
        var fields = [String: NativeJSON]()
        for (key, value) in result {
            fields[key] = try NativeJSON.fromAny(value, event: "response", field: "result.\(key)")
        }
        return .response(.init(event: "response", fields: .init(id: id, ok: true,
            result: fields, error: nil)))
    }

    public static func makeRejection(id: String, code: String, message: String) -> NativeEvent {
        .response(.init(event: "response", fields: .init(id: id, ok: false,
            result: nil, error: .init(code: code, message: message))))
    }

    /// A refusal with its numbers. A detail that cannot be expressed
    /// as JSON is a programming error; the refusal still goes, by its code.
    public static func makeRejection(id: String, code: String, message: String,
                                     detail: [String: Any]) -> NativeEvent {
        var fields = [String: NativeJSON]()
        for (key, value) in detail {
            guard let json = try? NativeJSON.fromAny(value, event: "response", field: "error.detail.\(key)") else {
                continue
            }
            fields[key] = json
        }
        return .response(.init(event: "response", fields: .init(id: id, ok: false,
            result: nil, error: .init(code: code, message: message, detail: fields))))
    }

    public static func makeAudio(source: String, streamID: String, sequence: UInt64,
        sampleRate: UInt32, frames: UInt32, monotonicNS: UInt64, pcmBase64: String,
        processing: AudioProcessingObservation?,
        playback: (active: Bool, id: String?)? = nil) -> NativeEvent {
        return .audio(.init(event: "audio", fields: .init(source: source, stream_id: streamID,
            sequence: sequence, sample_rate: sampleRate, channels: 1, frames: frames,
            monotonic_ns: monotonicNS, clock_domain: "mach_absolute_time",
            pcm_s16le_base64: pcmBase64,
            processing: processing?.mode.rawValue ?? "unprocessed",
            processing_enabled: processing?.enabled, processing_bypassed: processing?.bypassed,
            agc_enabled: processing?.agcEnabled,
            playback_reference: processing.map { $0.sharedPlaybackReference ? "shared_engine" : "none" },
            pipeline_id: processing?.pipelineID,
            playback_active: playback?.active, playback_id: playback?.id)))
    }

    public static func makePlaybackProgress(id: String, state: String, active: Bool,
        sequence: UInt64? = nil, frames: UInt32? = nil, renderedFrames: UInt32? = nil,
        remainingFrames: UInt32? = nil, playedFrames: UInt32? = nil,
        mimeType: String? = nil, provider: String? = nil) -> NativeEvent {
        .playbackProgress(.init(event: "playback_progress", fields: .init(
            playback_id: id, state: state, playback_active: active, sequence: sequence,
            frames: frames, rendered_frames: renderedFrames, remaining_frames: remainingFrames,
            played_frames: playedFrames, mime_type: mimeType, provider: provider)))
    }

    public static func makeStop(latches: Bool, reason: String, stopped: Bool,
        drainComplete: Bool) -> NativeEvent {
        let fields = NativeStopEventFields(reason: reason, privacy_stopped: stopped,
            drain_complete: drainComplete)
        return latches
            ? .privacyStop(.init(event: "privacy_stop", fields: fields))
            : .captureStopped(.init(event: "capture_stopped", fields: fields))
    }

    public static func makeReady(selectedTextAvailable: Bool, launchMode: String,
        isolatedFixture: Bool) -> NativeEvent {
        .ready(.init(event: "ready", fields: .init(protocol: NativeProtocol.version,
            platform: "macos", selected_text_available: isolatedFixture ? false : selectedTextAvailable,
            supported_audio_processing: ["unprocessed", "voice_processing"],
            voice_processing_available: false, playback_reference_available: false,
            launch_mode: launchMode, fixture: isolatedFixture ? true : nil)))
    }

    public static func makeMicrophoneActivity(
        _ applications: [(pid: Int32, bundleID: String, name: String)]
    ) -> NativeEvent {
        .microphoneActivity(.init(event: "microphone_activity", fields: .init(
            applications: applications.map {
                .init(application_pid: $0.pid, bundle_id: $0.bundleID, name: $0.name)
            })))
    }

    public static func makeAssistantQuestion(streamID: String, text: String,
        startedMS: Int64, endedMS: Int64, assistantName: String? = nil) -> NativeEvent {
        .assistantQuestion(.init(event: "assistant_question", fields: .init(
            stream_id: streamID, text: text, started_ms: startedMS, ended_ms: endedMS,
            assistant_name: assistantName)))
    }

    public static func makeVoiceTrigger(state: String, cause: String? = nil,
        detail: String? = nil, percent: UInt8? = nil, osVersion: String? = nil) -> NativeEvent {
        .voiceTrigger(.init(event: "voice_trigger", fields: .init(
            state: state, cause: cause, detail: detail, percent: percent, os_version: osVersion)))
    }

    public static func makeHotkey(action: String, phase: String,
        applications: [(pid: Int32, bundleID: String, name: String)]? = nil) -> NativeEvent {
        .hotkey(.init(event: "hotkey", fields: .init(action: action, phase: phase,
            applications: applications?.map { .init(application_pid: $0.pid,
                bundle_id: $0.bundleID, name: $0.name) })))
    }

    public static func makeState(stopped: Bool, connected: Bool, instanceID: String,
        generation: UInt64, selectedTextAvailable: Bool, microphone: Bool,
        systemAudio: Bool, playing: Bool,
        voiceProcessingAvailable: Bool, playbackReferenceAvailable: Bool,
        audioProcessing: NativeAudioProcessingResultFields,
        calendar: NativeCalendarStatusResultFields) -> NativeEvent {
        .state(.init(event: "state", fields: .init(privacy_stopped: stopped,
            connected: connected, native_instance_id: instanceID, privacy_generation: generation,
            selected_text_available: selectedTextAvailable, microphone: microphone,
            system_audio: systemAudio, playing: playing,
            supported_audio_processing: ["unprocessed", "voice_processing"],
            voice_processing_available: voiceProcessingAvailable,
            playback_reference_available: playbackReferenceAvailable,
            audio_processing: audioProcessing, calendar: calendar)))
    }

    public static func makeError(code: String, osDomain: String? = nil, osStatus: Int? = nil) -> NativeEvent {
        .error(.init(event: "error", fields: .init(code: code, os_domain: osDomain, os_status: osStatus)))
    }

    public static func makeNotice(code: String, abandonment: WakePhraseAbandonment? = nil) -> NativeEvent {
        .notice(.init(event: "notice", fields: .init(code: code, abandonment: abandonment)))
    }

    public static func makeLocalRearm() -> NativeEvent {
        .localRearm(.init(event: "local_rearm", fields: .init(privacy_stopped: false)))
    }

    public static func makeCalendarInvalidated(code: String) -> NativeEvent {
        .calendarInvalidated(.init(event: "calendar_invalidated", fields: .init(code: code)))
    }

    public static func makeGap(source: String, streamID: String, reason: String,
                               frames: UInt64, preStopPossible: Bool? = nil,
                               durationUnknown: Bool? = nil, durationNS: UInt64? = nil) -> NativeEvent {
        .gap(.init(event: "gap", fields: .init(source: source, stream_id: streamID,
            reason: reason, frames: frames, pre_stop_possible: preStopPossible,
            duration_unknown: durationUnknown, duration_ns: durationNS)))
    }
}
