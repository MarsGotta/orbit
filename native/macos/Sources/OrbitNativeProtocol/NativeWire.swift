import Foundation
import CoreFoundation

/// Native-v1: commands are decoded once, then dispatched by their typed variant.
public struct NativeWireEnvelope<Fields: Decodable & Sendable>: Decodable, Sendable {
    public let v: Int
    public let id: String
    public let fields: Fields

    private enum CodingKeys: String, CodingKey { case v, id }

    public init(from decoder: Decoder) throws {
        let header = try decoder.container(keyedBy: CodingKeys.self)
        v = try header.decode(Int.self, forKey: .v)
        id = try header.decode(String.self, forKey: .id)
        fields = try Fields(from: decoder)
    }
}

public struct NativeEmptyFields: Decodable, Sendable {}
public enum NativeWirePermission: String, Decodable, Sendable {
    case microphone, screen, accessibility, calendars
}
public enum NativeWireAudioSource: String, Decodable, Sendable {
    case micLocal = "mic_local", remoteApp = "remote_app"
}
public enum NativeWireCaptureScope: String, Decodable, Sendable {
    case system, application
}
public struct NativeCalendarSourcesFields: Decodable, Sendable { public let calendar_page: CalendarPageRequest? }
public struct NativeCalendarSnapshotFields: Decodable, Sendable { public let calendar_query: CalendarReadQuery }
/// Create an event or change an occurrence.
public struct NativeCalendarSaveFields: Decodable, Sendable { public let calendar_save: CalendarSaveRequest }
/// Delete an occurrence, or it and the ones after it.
public struct NativeCalendarDeleteFields: Decodable, Sendable { public let calendar_delete: CalendarDeleteRequest }
public struct NativePermissionsFields: Decodable, Sendable { public let request: [NativeWirePermission]? }
public enum NativeMicrophonePurpose: String, Decodable, Sendable {
    case meeting
    case voiceNote = "voice_note"
}
public struct NativeAudioPurposeFields: Decodable, Sendable {
    public let stream_id: String
    public let purpose: NativeMicrophonePurpose
}
public struct NativeAudioStartFields: Decodable, Sendable {
    public let stream_id: String
    public let source: NativeWireAudioSource
    public let processing: AudioProcessingMode?
    public let purpose: NativeMicrophonePurpose?
    public let application_pid: Int32?
    public let capture_scope: NativeWireCaptureScope?
}
/// «Oye Deimos» on (or off) on the live microphone stream.
public struct NativeAudioWakePhraseFields: Decodable, Sendable {
    public let assistant_name: String?
    public let stream_id: String
    public let enabled: Bool
}
public struct NativeStreamFields: Decodable, Sendable { public let stream_id: String }
public struct NativePlaybackFields: Decodable, Sendable {
    public let playback_id: String
    public let sequence: UInt64
    public let sample_rate: Int
    public let channels: Int
    public let pcm_s16le_base64: String
}
public struct NativePlaybackClipFields: Decodable, Sendable {
    public let playback_id: String
    public let mime_type: String
    public let provider: String
    public let audio_base64: String
}
public struct NativeApplicationSourcesFields: Decodable, Sendable { public let bundle_ids: [String] }
public struct NativeWindowFields: Decodable, Sendable { public let window_binding: WindowBinding }

public enum NativeWireCommand: Decodable, Sendable {
    case hello(NativeWireEnvelope<NativeEmptyFields>)
    case heartbeat(NativeWireEnvelope<NativeEmptyFields>)
    case status(NativeWireEnvelope<NativeEmptyFields>)
    case calendarDisconnect(NativeWireEnvelope<NativeEmptyFields>)
    case calendarSources(NativeWireEnvelope<NativeCalendarSourcesFields>)
    case calendarSnapshot(NativeWireEnvelope<NativeCalendarSnapshotFields>)
    case calendarSave(NativeWireEnvelope<NativeCalendarSaveFields>)
    case calendarDelete(NativeWireEnvelope<NativeCalendarDeleteFields>)
    case privacyStop(NativeWireEnvelope<NativeEmptyFields>)
    case captureStop(NativeWireEnvelope<NativeEmptyFields>)
    case playbackStop(NativeWireEnvelope<NativeEmptyFields>)
    case rearm(NativeWireEnvelope<NativeEmptyFields>)
    case shutdown(NativeWireEnvelope<NativeEmptyFields>)
    case permissions(NativeWireEnvelope<NativePermissionsFields>)
    case audioStart(NativeWireEnvelope<NativeAudioStartFields>)
    case audioPurpose(NativeWireEnvelope<NativeAudioPurposeFields>)
    case audioWakePhrase(NativeWireEnvelope<NativeAudioWakePhraseFields>)
    case audioStop(NativeWireEnvelope<NativeStreamFields>)
    case playback(NativeWireEnvelope<NativePlaybackFields>)
    case playbackClip(NativeWireEnvelope<NativePlaybackClipFields>)
    case applicationSources(NativeWireEnvelope<NativeApplicationSourcesFields>)
    case screenSources(NativeWireEnvelope<NativeEmptyFields>)
    case screenshot(NativeWireEnvelope<NativeWindowFields>)
    case selectedText(NativeWireEnvelope<NativeWindowFields>)

    private var header: (v: Int, id: String) {
        switch self {
        case .hello(let value), .heartbeat(let value), .status(let value),
             .calendarDisconnect(let value),
             .privacyStop(let value), .captureStop(let value), .playbackStop(let value),
             .rearm(let value), .shutdown(let value), .screenSources(let value): return (value.v, value.id)
        case .calendarSources(let value): return (value.v, value.id)
        case .calendarSnapshot(let value): return (value.v, value.id)
        case .calendarSave(let value): return (value.v, value.id)
        case .calendarDelete(let value): return (value.v, value.id)
        case .permissions(let value): return (value.v, value.id)
        case .audioStart(let value): return (value.v, value.id)
        case .audioPurpose(let value): return (value.v, value.id)
        case .audioWakePhrase(let value): return (value.v, value.id)
        case .audioStop(let value): return (value.v, value.id)
        case .playback(let value): return (value.v, value.id)
        case .playbackClip(let value): return (value.v, value.id)
        case .applicationSources(let value): return (value.v, value.id)
        case .screenshot(let value), .selectedText(let value): return (value.v, value.id)
        }
    }

    public var id: String { header.id }
    public var v: Int { header.v }

    public static func decode(_ data: Data) throws -> Self {
        guard data.count <= NativeProtocol.maximumLineBytes else { throw ProtocolError.messageTooLarge }
        let value = try JSONDecoder().decode(Self.self, from: data)
        guard value.v == NativeProtocol.version else { throw ProtocolError.versionMismatch }
        guard !value.id.isEmpty, value.id.utf8.count <= 128 else { throw ProtocolError.invalidIdentifier }
        return value
    }

    private enum CodingKeys: String, CodingKey {
        case command, calendar_query, calendar_page, calendar_save, calendar_delete
    }

    public init(from decoder: Decoder) throws {
        let header = try decoder.container(keyedBy: CodingKeys.self)
        let command = try header.decode(String.self, forKey: .command)
        let calendarField: CodingKeys?
        switch command {
        case "calendar_sources": calendarField = .calendar_page
        case "calendar_snapshot": calendarField = .calendar_query
        case "calendar_save": calendarField = .calendar_save
        case "calendar_delete": calendarField = .calendar_delete
        default: calendarField = nil
        }
        if let calendarField {
            for key in [CodingKeys.calendar_query, .calendar_page, .calendar_save, .calendar_delete]
                where key != calendarField && header.contains(key) {
                guard try header.decodeNil(forKey: key) else { throw CalendarFailure.invalidRequest }
            }
        }
        switch command {
        case "hello": self = .hello(try .init(from: decoder))
        case "heartbeat": self = .heartbeat(try .init(from: decoder))
        case "status": self = .status(try .init(from: decoder))
        case "calendar_disconnect": self = .calendarDisconnect(try .init(from: decoder))
        case "calendar_sources": self = .calendarSources(try .init(from: decoder))
        case "calendar_snapshot": self = .calendarSnapshot(try .init(from: decoder))
        case "calendar_save": self = .calendarSave(try .init(from: decoder))
        case "calendar_delete": self = .calendarDelete(try .init(from: decoder))
        case "privacy_stop": self = .privacyStop(try .init(from: decoder))
        case "capture_stop": self = .captureStop(try .init(from: decoder))
        case "playback_stop": self = .playbackStop(try .init(from: decoder))
        case "rearm": self = .rearm(try .init(from: decoder))
        case "shutdown": self = .shutdown(try .init(from: decoder))
        case "permissions": self = .permissions(try .init(from: decoder))
        case "audio_start": self = .audioStart(try .init(from: decoder))
        case "audio_purpose": self = .audioPurpose(try .init(from: decoder))
        case "audio_wake_phrase": self = .audioWakePhrase(try .init(from: decoder))
        case "audio_stop": self = .audioStop(try .init(from: decoder))
        case "playback": self = .playback(try .init(from: decoder))
        case "playback_clip": self = .playbackClip(try .init(from: decoder))
        case "application_sources": self = .applicationSources(try .init(from: decoder))
        case "screen_sources": self = .screenSources(try .init(from: decoder))
        case "screenshot": self = .screenshot(try .init(from: decoder))
        case "selected_text": self = .selectedText(try .init(from: decoder))
        default: throw DecodingError.dataCorruptedError(forKey: .command,
            in: try decoder.container(keyedBy: CodingKeys.self), debugDescription: "Comando nativo desconocido: \(command)")
        }
    }

    public var command: String {
        switch self {
        case .hello: "hello"
        case .heartbeat: "heartbeat"
        case .status: "status"
        case .calendarDisconnect: "calendar_disconnect"
        case .calendarSources: "calendar_sources"
        case .calendarSnapshot: "calendar_snapshot"
        case .calendarSave: "calendar_save"
        case .calendarDelete: "calendar_delete"
        case .privacyStop: "privacy_stop"
        case .captureStop: "capture_stop"
        case .playbackStop: "playback_stop"
        case .rearm: "rearm"
        case .shutdown: "shutdown"
        case .permissions: "permissions"
        case .audioStart: "audio_start"
        case .audioPurpose: "audio_purpose"
        case .audioWakePhrase: "audio_wake_phrase"
        case .audioStop: "audio_stop"
        case .playback: "playback"
        case .playbackClip: "playback_clip"
        case .applicationSources: "application_sources"
        case .screenSources: "screen_sources"
        case .screenshot: "screenshot"
        case .selectedText: "selected_text"
        }
    }
    public static let names: Set<String> = [
        "hello", "heartbeat", "status", "calendar_disconnect",
        "calendar_sources", "calendar_snapshot", "calendar_save", "calendar_delete",
        "privacy_stop", "capture_stop",
        "playback_stop", "rearm", "shutdown", "permissions", "audio_start",
        "audio_purpose", "audio_wake_phrase", "audio_stop", "playback", "playback_clip",
        "application_sources", "screen_sources",
        "screenshot", "selected_text"
    ]
}

public enum NativeJSON: Encodable {
    case string(String), integer(Int64), unsigned(UInt64), decimal(Double), bool(Bool), array([NativeJSON])
    case object([String: NativeJSON]), null

    public static func fromAny(_ value: Any, event: String, field: String) throws -> NativeJSON {
        if value is NSNull { return .null }
        if let number = value as? NSNumber {
            if CFGetTypeID(number) == CFBooleanGetTypeID() { return .bool(number.boolValue) }
            if let integer = Int64(number.stringValue) { return .integer(integer) }
            if let unsigned = UInt64(number.stringValue) { return .unsigned(unsigned) }
            guard number.doubleValue.isFinite else {
                throw NativeEventFieldError(event: event, field: field, expected: "finite number")
            }
            return .decimal(number.doubleValue)
        }
        if let string = value as? String { return .string(string) }
        if let values = value as? [Any] {
            return .array(try values.enumerated().map {
                try fromAny($0.element, event: event, field: "\(field)[\($0.offset)]")
            })
        }
        if let values = value as? [String: Any] {
            var result = [String: NativeJSON]()
            for (key, item) in values {
                result[key] = try fromAny(item, event: event, field: field.isEmpty ? key : "\(field).\(key)")
            }
            return .object(result)
        }
        throw NativeEventFieldError(event: event, field: field, expected: "JSON value")
    }

    public func encode(to encoder: Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .string(let item): try value.encode(item)
        case .integer(let item): try value.encode(item)
        case .unsigned(let item): try value.encode(item)
        case .decimal(let item):
            guard item.isFinite else {
                throw NativeEventFieldError(event: "response",
                    field: encoder.codingPath.map(\.stringValue).joined(separator: "."),
                    expected: "finite Double")
            }
            try value.encode(item)
        case .bool(let item): try value.encode(item)
        case .array(let item): try value.encode(item)
        case .object(let item): try value.encode(item)
        case .null: try value.encodeNil()
        }
    }
}

public struct NativeEventFieldError: Error, CustomStringConvertible {
    public let event: String
    public let field: String
    public let expected: String
    public var description: String {
        "native_event_\(event)_\(field)_expected_\(expected.replacingOccurrences(of: " ", with: "_"))"
    }
}

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
    public let applications: [NativeWireMeetingApplication]
}
/// The question the user asked after «Oye Deimos» on the meeting's
/// microphone, as text. `started_ms`/`ended_ms` are Unix milliseconds of the
/// phrase's start and the question's end. Nothing else heard is sent.
public struct NativeAssistantQuestionFields: Encodable {
    public let stream_id: String
    public let text: String
    public let started_ms: Int64
    public let ended_ms: Int64
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
        startedMS: Int64, endedMS: Int64) -> NativeEvent {
        .assistantQuestion(.init(event: "assistant_question", fields: .init(
            stream_id: streamID, text: text, started_ms: startedMS, ended_ms: endedMS)))
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

public struct NativeStateEventFields: Encodable {
    public let privacy_stopped: Bool
    public let connected: Bool
    public let native_instance_id: String
    public let privacy_generation: UInt64
    public let selected_text_available: Bool
    public let microphone: Bool
    public let system_audio: Bool
    public let playing: Bool
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
