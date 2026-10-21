import Foundation

public enum CalendarAuthorization: String, Codable, Equatable, Sendable {
    case notDetermined = "not_determined", restricted, denied
    case fullAccess = "full_access", writeOnly = "write_only", unavailable, unknown
}

public enum CalendarFailure: String, Error, Codable, Equatable, Sendable {
    case permissionRequired = "calendar_permission_required"
    case invalidRequest = "invalid_calendar_request"
    case busy = "calendar_busy"
    case expired = "calendar_read_expired"
    case invalidated = "calendar_invalidated"
    case selectionUnavailable = "calendar_selection_unavailable"
    case limitExceeded = "calendar_limit_exceeded"
    case readFailed = "calendar_read_failed"
    case clockUnavailable = "calendar_clock_unavailable"
    case privacyStopped = "privacy_stopped"
    /// One event whose own fields do not fit the largest page the
    /// daemon admits (thousands of invitees). Named, never trimmed or dropped.
    case eventTooLarge = "calendar_event_too_large"
}

/// What the daemon asks for — every calendar, one window, from one
/// position in the window's ordered events. Nothing selects calendars.
public struct CalendarReadQuery: Codable, Equatable, Sendable {
    public let start_unix_ms: Int64
    public let end_unix_ms: Int64
    public let after_index: UInt32
    public init(start: Int64, end: Int64, afterIndex: UInt32 = 0) {
        start_unix_ms = start; end_unix_ms = end; after_index = afterIndex
    }
    public var isValid: Bool {
        CalendarLimits.date(start_unix_ms) && CalendarLimits.date(end_unix_ms) && end_unix_ms > start_unix_ms
            && end_unix_ms - start_unix_ms <= CalendarLimits.maximumQueryMilliseconds
    }
}

/// Which page of the calendar list the daemon asks for.
public struct CalendarPageRequest: Codable, Equatable, Sendable {
    public let after_index: UInt32
    public init(afterIndex: UInt32) { after_index = afterIndex }
}

public struct CalendarWindow: Codable, Equatable, Sendable {
    public let start_unix_ms: Int64
    public let end_unix_ms: Int64
    public init(start: Int64, end: Int64) { start_unix_ms = start; end_unix_ms = end }
}

public struct CalendarReference: Codable, Equatable, Sendable {
    public let calendar_id: String
    public let source_id: String
    public init(calendarID: String, sourceID: String) { calendar_id = calendarID; source_id = sourceID }
    public var isValid: Bool { CalendarLimits.identifier(calendar_id) && CalendarLimits.identifier(source_id) }
}

public struct CalendarEntry: Codable, Equatable, Sendable {
    public let reference: CalendarReference
    public let title: String
    public let source_title: String
    public let calendar_type: String
    /// The calendar's color in sRGB, `#RRGGBB`; absent when macOS gave
    /// one that cannot be expressed so.
    public let color: String?
    /// `allowsContentModifications`.
    public let can_edit: Bool?
    /// The calendar macOS puts new events in.
    public let is_default: Bool?
    public init(reference: CalendarReference, title: String, sourceTitle: String, calendarType: String,
                color: String? = nil, canEdit: Bool? = nil, isDefault: Bool? = nil) {
        self.reference = reference; self.title = title; source_title = sourceTitle; calendar_type = calendarType
        self.color = color; can_edit = canEdit; is_default = isDefault
    }
    public var isValid: Bool {
        reference.isValid && title.utf8.count <= CalendarLimits.maximumTitleBytes
            && source_title.utf8.count <= CalendarLimits.maximumTitleBytes
            && ["local", "caldav", "exchange", "subscription", "birthday", "unknown"].contains(calendar_type)
            && (color.map(Self.hexColor) ?? true)
    }
    /// `#RRGGBB`, the only color form that travels.
    public static func hexColor(_ value: String) -> Bool {
        let digits = Array(value.utf8)
        return digits.count == 7 && digits[0] == UInt8(ascii: "#")
            && digits.dropFirst().allSatisfy { (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0) }
    }
}

/// One page of the list of every calendar on the Mac. Nothing is
/// offered for choosing; the list says how many calendars there are and names
/// them. It is paged so no number of calendars needs one oversized message.
public struct CalendarInventoryPage: Encodable, Equatable, Sendable {
    public let inventory_id: String
    public let native_instance_id: String
    public let privacy_generation: UInt64
    public let clock_domain = "mach_absolute_time"
    public let observed_monotonic_ns: UInt64
    public let completed_monotonic_ns: UInt64
    public let calendars: [CalendarEntry]
    public let after_index: UInt32
    public let total: UInt32
    public let next_after_index: UInt32?
    public init(id: String, nativeInstanceID: String, generation: UInt64, started: UInt64, completed: UInt64,
                calendars: [CalendarEntry], afterIndex: UInt32, total: UInt32, nextAfterIndex: UInt32?) {
        inventory_id = id; native_instance_id = nativeInstanceID; privacy_generation = generation
        observed_monotonic_ns = started; completed_monotonic_ns = completed; self.calendars = calendars
        after_index = afterIndex; self.total = total; next_after_index = nextAfterIndex
    }
}

public enum CalendarLimits {
    /// How many bytes of events a page carries before the next one starts.
    /// Soft: a page always carries at least one whole event. Mirrors
    /// `orbit_core::calendar::NATIVE_PAGE_EVENT_BYTES`; contracts/constants.json
    /// registers both copies for the cross-language guard.
    public static let pageEventBytes = 262_144
    /// The largest page the daemon admits: its reader drops the whole helper
    /// link on a line over 1 MiB, audio included. Mirrors
    /// `orbit_core::calendar::MAX_NATIVE_PAGE_BYTES`. Not a limit on events or
    /// calendars: only one event larger than this on its own is refused, by name.
    public static let maximumPageBytes = 786_432
    /// EventKit's `predicateForEvents` silently shortens a range longer than four
    /// years to its first four, so a wider query would lose events without a
    /// word. 1,460 days is never longer than four calendar years. It is
    /// EventKit's limit, not Orbit's: the daemon reads 31-day windows. Mirrors
    /// `orbit_core::calendar::EVENTKIT_MAX_QUERY_MS` (1,460 × 86,400,000).
    public static let maximumQueryMilliseconds: Int64 = 126_144_000_000
    /// Field bounds. A field is not a reason to leave an
    /// event out while the event fits a page on its own, so titles, names and
    /// URLs may be as large as a whole device-bridge page (96 KiB) — larger
    /// could never reach Deimos. Identifiers and zones keep a shape check (an
    /// EventKit id or an IANA zone of that length is not one). An event with
    /// a field past its bound is left out *by name* (`CalendarOversizedEvent`:
    /// its dates, identifier, the field, its bytes and the bytes that fit) and
    /// the rest of the read arrives; it is never trimmed and never stops the
    /// read. Mirror `MAX_TITLE_BYTES`, `MAX_URL_BYTES`, `MAX_LABEL_BYTES` and
    /// `MAX_TIMEZONE_BYTES` in `orbit_core::calendar`; `contracts/constants.json`
    /// registers both copies and its guard compares them.
    public static let maximumTitleBytes = 98_304
    public static let maximumURLBytes = 98_304
    public static let maximumLabelBytes = 1_024
    public static let maximumTimezoneBytes = 128
    /// The longest notes that travel with an event, in bytes. They
    /// must not crowd out the events sharing a page — above all the 96 KiB
    /// bridge page Deimos reads. A read carries the event without longer notes
    /// and says so (`notes_left_out`: their bytes and this limit); a write
    /// with longer notes is refused with the same numbers. Mirrors
    /// `orbit_core::calendar::MAX_NOTES_BYTES` (32 KiB); `contracts/constants.json`
    /// registers both copies and its guard compares them.
    public static let maximumNotesBytes = 32 * 1024
    /// How many alarms one written event may carry: a shape check on
    /// one write command (Calendar offers two). Mirrors `MAX_EVENT_ALARMS`.
    public static let maximumAlarms = 16
    /// The furthest an alarm may sound before its event, in minutes
    /// (four weeks): a shape check. Mirrors `MAX_ALARM_MINUTES`.
    public static let maximumAlarmMinutes = 40_320
    /// The largest single event a helper page carries: `maximumPageBytes` less
    /// 64 KiB for the page's own fields and its lists of omissions and events
    /// left out. Only an event with thousands of guests reaches it; it is left
    /// out by name, never trimmed, and the rest of the window arrives.
    public static let maximumEventBytes = 720_896
    public static let readLifetimeNanoseconds: UInt64 = 5_000_000_000
    /// How long one EventKit save or removal may hold the slot before
    /// the answer is that its outcome is unknown. A local commit takes
    /// milliseconds; unlike a late read, a late write may already be applied,
    /// so it is never answered as expired and never retried. Mirrors
    /// `orbit_core::calendar::WRITE_LIFETIME_NS`.
    public static let writeLifetimeNanoseconds: UInt64 = 5_000_000_000
    /// How long a permission request may stay outstanding.
    ///
    /// A read waits for EventKit. A permission request waits for a person to
    /// notice a system dialog, read it and decide, and five seconds discarded
    /// their answer while the prompt was still on screen: granting it changed
    /// nothing and the status stayed undetermined with no error to show.
    public static let permissionLifetimeNanoseconds: UInt64 = 120_000_000_000
    public static func lifetime(for operation: CalendarOperation) -> UInt64 {
        if operation == .requestAccess { return permissionLifetimeNanoseconds }
        return operation.isWrite ? writeLifetimeNanoseconds : readLifetimeNanoseconds
    }
    public static func uuid(_ value: String) -> Bool {
        value.utf8.count == 36 && UUID(uuidString: value)?.uuidString.lowercased() == value.lowercased()
    }
    public static func identifier(_ value: String) -> Bool {
        !value.isEmpty && value.utf8.count <= maximumLabelBytes
            && !value.unicodeScalars.contains { $0.value < 32 || $0.value == 127 }
    }
    public static func date(_ value: Int64) -> Bool { (-62_135_596_800_000...253_402_300_799_999).contains(value) }
    public static func encodedSize<T: Encodable>(_ value: T) throws -> Int {
        try JSONEncoder().encode(value).count
    }
}
