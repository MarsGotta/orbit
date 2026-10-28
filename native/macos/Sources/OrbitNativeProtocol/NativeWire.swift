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
    /// A microphone that starts muted («Silenciar» kept over a pause).
    public let muted: Bool?
}
/// «Silenciar»: the microphone stream's samples leave as silence, or stop doing so.
public struct NativeAudioMuteFields: Decodable, Sendable {
    public let stream_id: String
    public let muted: Bool
}
/// «Oye Deimos» on (or off) on the live microphone stream. `assistant_names`
/// is every agent's name (hub-v1 «Agents»); `assistant_name`, the default
/// agent's, is what a daemon that knows one name sends.
public struct NativeAudioWakePhraseFields: Decodable, Sendable {
    public let locale: NativeSpeechLocale?
    public let assistant_name: String?
    public let assistant_names: [String]?
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
    case playbackPause(NativeWireEnvelope<NativeEmptyFields>)
    case playbackResume(NativeWireEnvelope<NativeEmptyFields>)
    case playbackStop(NativeWireEnvelope<NativeEmptyFields>)
    case rearm(NativeWireEnvelope<NativeEmptyFields>)
    case shutdown(NativeWireEnvelope<NativeEmptyFields>)
    case permissions(NativeWireEnvelope<NativePermissionsFields>)
    case audioStart(NativeWireEnvelope<NativeAudioStartFields>)
    case audioPurpose(NativeWireEnvelope<NativeAudioPurposeFields>)
    case audioMute(NativeWireEnvelope<NativeAudioMuteFields>)
    case interfaceLanguage(NativeWireEnvelope<NativeInterfaceLanguageFields>)
    case audioWakePhrase(NativeWireEnvelope<NativeAudioWakePhraseFields>)
    case audioStop(NativeWireEnvelope<NativeStreamFields>)
    case playback(NativeWireEnvelope<NativePlaybackFields>)
    case playbackClip(NativeWireEnvelope<NativePlaybackClipFields>)
    case applicationSources(NativeWireEnvelope<NativeApplicationSourcesFields>)
    case screenSources(NativeWireEnvelope<NativeEmptyFields>)
    case screenshot(NativeWireEnvelope<NativeWindowFields>)
    case selectedText(NativeWireEnvelope<NativeWindowFields>)
    /// The assistant's folders (`AssistantFolders.swift`).
    case assistantFolderPick(NativeWireEnvelope<NativeEmptyFields>)
    case assistantFolders(NativeWireEnvelope<AssistantFoldersFields>)
    case assistantFilesList(NativeWireEnvelope<AssistantFilesListFields>)
    case assistantFilesSearch(NativeWireEnvelope<AssistantFilesSearchFields>)
    case assistantFilesRead(NativeWireEnvelope<AssistantFilesReadFields>)
    /// «Adjuntar ventana»'s text and a HEIC photo as JPEG (`WindowText.swift`).
    case windowText(NativeWireEnvelope<NativeWindowTextFields>)
    case photoJPEG(NativeWireEnvelope<NativePhotoJPEGFields>)

    private var header: (v: Int, id: String) {
        switch self {
        case .hello(let value), .heartbeat(let value), .status(let value),
             .calendarDisconnect(let value),
             .privacyStop(let value), .captureStop(let value), .playbackStop(let value),
             .playbackPause(let value), .playbackResume(let value),
             .rearm(let value), .shutdown(let value), .screenSources(let value): return (value.v, value.id)
        case .calendarSources(let value): return (value.v, value.id)
        case .calendarSnapshot(let value): return (value.v, value.id)
        case .calendarSave(let value): return (value.v, value.id)
        case .calendarDelete(let value): return (value.v, value.id)
        case .permissions(let value): return (value.v, value.id)
        case .audioStart(let value): return (value.v, value.id)
        case .audioPurpose(let value): return (value.v, value.id)
        case .audioMute(let value): return (value.v, value.id)
        case .interfaceLanguage(let value): return (value.v, value.id)
        case .audioWakePhrase(let value): return (value.v, value.id)
        case .audioStop(let value): return (value.v, value.id)
        case .playback(let value): return (value.v, value.id)
        case .playbackClip(let value): return (value.v, value.id)
        case .applicationSources(let value): return (value.v, value.id)
        case .screenshot(let value), .selectedText(let value): return (value.v, value.id)
        case .assistantFolderPick(let value): return (value.v, value.id)
        case .assistantFolders(let value): return (value.v, value.id)
        case .assistantFilesList(let value): return (value.v, value.id)
        case .assistantFilesSearch(let value): return (value.v, value.id)
        case .assistantFilesRead(let value): return (value.v, value.id)
        case .windowText(let value): return (value.v, value.id)
        case .photoJPEG(let value): return (value.v, value.id)
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
        case "playback_pause": self = .playbackPause(try .init(from: decoder))
        case "playback_resume": self = .playbackResume(try .init(from: decoder))
        case "playback_stop": self = .playbackStop(try .init(from: decoder))
        case "rearm": self = .rearm(try .init(from: decoder))
        case "shutdown": self = .shutdown(try .init(from: decoder))
        case "permissions": self = .permissions(try .init(from: decoder))
        case "audio_start": self = .audioStart(try .init(from: decoder))
        case "audio_purpose": self = .audioPurpose(try .init(from: decoder))
        case "audio_mute": self = .audioMute(try .init(from: decoder))
        case "interface_language": self = .interfaceLanguage(try .init(from: decoder))
        case "audio_wake_phrase": self = .audioWakePhrase(try .init(from: decoder))
        case "audio_stop": self = .audioStop(try .init(from: decoder))
        case "playback": self = .playback(try .init(from: decoder))
        case "playback_clip": self = .playbackClip(try .init(from: decoder))
        case "application_sources": self = .applicationSources(try .init(from: decoder))
        case "screen_sources": self = .screenSources(try .init(from: decoder))
        case "screenshot": self = .screenshot(try .init(from: decoder))
        case "selected_text": self = .selectedText(try .init(from: decoder))
        case "assistant_folder_pick": self = .assistantFolderPick(try .init(from: decoder))
        case "assistant_folders": self = .assistantFolders(try .init(from: decoder))
        case "assistant_files_list": self = .assistantFilesList(try .init(from: decoder))
        case "assistant_files_search": self = .assistantFilesSearch(try .init(from: decoder))
        case "assistant_files_read": self = .assistantFilesRead(try .init(from: decoder))
        case "window_text": self = .windowText(try .init(from: decoder))
        case "photo_jpeg": self = .photoJPEG(try .init(from: decoder))
        default: throw DecodingError.dataCorruptedError(forKey: .command,
            in: try decoder.container(keyedBy: CodingKeys.self), debugDescription: "Unknown native command: \(command)")
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
        case .playbackPause: "playback_pause"
        case .playbackResume: "playback_resume"
        case .rearm: "rearm"
        case .shutdown: "shutdown"
        case .permissions: "permissions"
        case .audioStart: "audio_start"
        case .audioPurpose: "audio_purpose"
        case .audioMute: "audio_mute"
        case .interfaceLanguage: "interface_language"
        case .audioWakePhrase: "audio_wake_phrase"
        case .audioStop: "audio_stop"
        case .playback: "playback"
        case .playbackClip: "playback_clip"
        case .applicationSources: "application_sources"
        case .screenSources: "screen_sources"
        case .screenshot: "screenshot"
        case .selectedText: "selected_text"
        case .assistantFolderPick: "assistant_folder_pick"
        case .assistantFolders: "assistant_folders"
        case .assistantFilesList: "assistant_files_list"
        case .assistantFilesSearch: "assistant_files_search"
        case .assistantFilesRead: "assistant_files_read"
        case .windowText: "window_text"
        case .photoJPEG: "photo_jpeg"
        }
    }
    public static let names: Set<String> = [
        "hello", "heartbeat", "status", "calendar_disconnect",
        "calendar_sources", "calendar_snapshot", "calendar_save", "calendar_delete",
        "privacy_stop", "capture_stop",
        "playback_stop", "playback_pause", "playback_resume", "rearm", "shutdown", "permissions", "audio_start",
        "audio_purpose", "audio_mute", "interface_language", "audio_wake_phrase", "audio_stop", "playback", "playback_clip",
        "application_sources", "screen_sources",
        "screenshot", "selected_text",
        "assistant_folder_pick", "assistant_folders", "assistant_files_list",
        "assistant_files_search", "assistant_files_read",
        "window_text", "photo_jpeg"
    ]
}
