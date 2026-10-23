import Foundation
import OrbitNativeProtocol
import OrbitNativeCalendar

package enum CalendarBridge {
    /// A calendar reply as the channel sends it. Bounded by the largest page the
    /// daemon admits (`CalendarLimits.maximumPageBytes`), which the controller
    /// already held every page to.
    package static func fields<T: Encodable>(_ value: T) throws -> [String: Any] {
        let data = try JSONEncoder().encode(value)
        guard data.count <= CalendarLimits.maximumPageBytes,
              let result = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw CalendarFailure.limitExceeded
        }
        return result
    }

    package static func fields(_ value: CalendarResult) throws -> [String: Any] {
        switch value {
        case .authorization(let permission): return ["calendars": permission.rawValue]
        case .inventory(let inventory): return try fields(inventory)
        case .page(let page): return try fields(page)
        case .saved(let saved): return try fields(saved)
        case .deleted(let deleted): return try fields(deleted)
        }
    }

    /// Two reads and nothing to select. `calendar_select` is gone, so
    /// a daemon that still sends it is refused by name rather than obeyed.
    /// And two writes, each with exactly its own field.
    package static func operation(_ command: NativeWireCommand) throws -> CalendarOperation {
        switch command {
        case .calendarSources(let wire): return .sources(afterIndex: wire.fields.calendar_page?.after_index ?? 0)
        case .calendarSnapshot(let wire): return .read(wire.fields.calendar_query)
        case .calendarSave(let wire): return .save(wire.fields.calendar_save)
        case .calendarDelete(let wire): return .delete(wire.fields.calendar_delete)
        case .hello, .heartbeat, .status, .calendarDisconnect,
             .privacyStop, .captureStop, .playbackStop, .rearm, .shutdown,
             .permissions, .audioStart, .audioPurpose, .audioMute, .audioWakePhrase, .audioStop, .playback,
             .playbackClip, .applicationSources,
             .screenSources, .screenshot, .selectedText,
             .assistantFolderPick, .assistantFolders, .assistantFilesList, .assistantFilesSearch,
             .assistantFilesRead, .windowText, .photoJPEG:
            throw CalendarFailure.invalidRequest
        }
    }

    package static func unixMilliseconds() -> Int64? {
        let value = Date().timeIntervalSince1970 * 1000
        guard value.isFinite, value >= Double(CalendarLimits.minimumUnixMilliseconds), value <= Double(CalendarLimits.maximumUnixMilliseconds) else { return nil }
        return Int64(value.rounded(.towardZero))
    }
}
