import Foundation

/// A field past its bound, by name and size. Raised while an event is built;
/// the reader turns it into a `CalendarOversizedEvent` for that event and goes
/// on with the rest of the window: a field is never a reason to lose the read.
public struct CalendarOversizedField: Error, Equatable, Sendable {
    public let field: String
    public let bytes: Int
    public let limit: Int
    public init(field: String, bytes: Int, limit: Int) {
        self.field = field; self.bytes = bytes; self.limit = limit
    }
    /// Throws when `value` is longer, in UTF-8 bytes, than `limit`.
    public static func check(_ field: String, _ value: String?, _ limit: Int) throws {
        let bytes = value?.utf8.count ?? 0
        if bytes > limit { throw CalendarOversizedField(field: field, bytes: bytes, limit: limit) }
    }
}

/// One event that could not travel, named instead of trimmed. The
/// window's other events arrive; this says which event it
/// was and why. `calendar_item_id` is absent only when it is the field to blame.
public struct CalendarOversizedEvent: Codable, Equatable, Sendable, Error {
    public let calendar: CalendarReference
    public let calendar_item_id: String?
    public let start_unix_ms: Int64
    public let end_unix_ms: Int64
    public let field: String
    public let bytes: Int
    public let limit: Int
    public init(calendar: CalendarReference, calendarItemID: String?, start: Int64, end: Int64,
                field: String, bytes: Int, limit: Int) {
        self.calendar = calendar; calendar_item_id = calendarItemID
        start_unix_ms = start; end_unix_ms = end
        self.field = field; self.bytes = bytes; self.limit = limit
    }
}

public struct CalendarInvitee: Codable, Equatable, Sendable {
    public let name: String?
    public let participant_url: String
    public let email: String?
    public let role: String
    public let response_status: String
    public let participant_type: String
    public let is_current_user: Bool
    public init(name: String?, url: String, role: String, responseStatus: String,
                participantType: String, isCurrentUser: Bool) throws {
        try CalendarOversizedField.check("invitee_name", name, CalendarLimits.maximumTitleBytes)
        try CalendarOversizedField.check("invitee_url", url, CalendarLimits.maximumURLBytes)
        guard !url.isEmpty,
              !url.unicodeScalars.contains(where: { $0.value < 32 || $0.value == 127 }),
              [role, responseStatus, participantType].allSatisfy({ !$0.isEmpty && $0.utf8.count <= 64 }) else {
            throw CalendarFailure.limitExceeded
        }
        self.name = name; participant_url = url; email = Self.mailbox(url)
        self.role = role; response_status = responseStatus
        participant_type = participantType; is_current_user = isCurrentUser
    }

    /// A conservative address candidate, never a confirmed person or delivery address.
    /// Non-mailto URIs remain URIs; headers, multiple recipients and malformed escapes
    /// cannot masquerade as a single email. No Contacts access or URL activation.
    public static func mailbox(_ value: String) -> String? {
        guard value.utf8.count <= CalendarLimits.maximumURLBytes, value.prefix(7).lowercased() == "mailto:",
              !value.unicodeScalars.contains(where: { $0.value < 32 || $0.value == 127 }),
              validPercentEscapes(value), let parts = URLComponents(string: value),
              parts.scheme?.lowercased() == "mailto", parts.host == nil,
              parts.query == nil, parts.fragment == nil,
              let address = parts.percentEncodedPath.removingPercentEncoding,
              !address.isEmpty, address.utf8.count <= 320,
              !address.unicodeScalars.contains(where: {
                  CharacterSet.whitespacesAndNewlines.contains($0) || $0.value < 32 || $0.value == 127
                    || "<>,;:\"\\[]".unicodeScalars.contains($0)
              }) else { return nil }
        let pieces = address.split(separator: "@", omittingEmptySubsequences: false)
        guard pieces.count == 2, !pieces[0].isEmpty, !pieces[1].isEmpty else { return nil }
        return address
    }

    private static func validPercentEscapes(_ value: String) -> Bool {
        let bytes = Array(value.utf8)
        var index = 0
        func hexadecimal(_ byte: UInt8) -> Bool {
            (48...57).contains(byte) || (65...70).contains(byte) || (97...102).contains(byte)
        }
        while index < bytes.count {
            if bytes[index] == 37 {
                guard index + 2 < bytes.count, hexadecimal(bytes[index + 1]), hexadecimal(bytes[index + 2]) else { return false }
                index += 3
            } else { index += 1 }
        }
        return true
    }
}

/// A field that did not travel because it is longer than its limit (the
/// notes): its bytes and the limit, both UTF-8 bytes.
public struct CalendarFieldLeftOut: Codable, Equatable, Sendable {
    public let bytes: Int
    public let limit: Int
    public init(bytes: Int, limit: Int) { self.bytes = bytes; self.limit = limit }
}

public struct CalendarOccurrence: Codable, Equatable, Sendable {
    public let observation_id: String
    public let calendar: CalendarReference
    public let calendar_item_id: String
    public let event_id: String?
    public let original_occurrence_unix_ms: Int64?
    public let start_unix_ms: Int64
    public let end_unix_ms: Int64
    public let event_timezone: String?
    public let interpreted_timezone: String
    public let is_floating: Bool
    public let is_all_day: Bool
    public let is_detached: Bool
    public let title: String
    public let status: String
    public let last_modified_unix_ms: Int64?
    public let invitees: [CalendarInvitee]?
    public let invitees_status: String
    public let event_url: String?
    public let event_url_status: String
    // What the edit needs. Absent only from a helper that predates them.
    public let location: String?
    public let notes: String?
    /// Notes longer than `maximumNotesBytes` stay behind, by their size.
    public let notes_left_out: CalendarFieldLeftOut?
    public let organizer: CalendarInvitee?
    public let availability: CalendarAvailability?
    public let recurrence: CalendarRecurrence?
    /// The calendar admits changes and the event was not organized by
    /// someone else.
    public let can_edit: Bool?
    /// The reminders Orbit edits (relative, at or before the start),
    /// in minutes before it, within the write's own limits; `nil` when not
    /// read or when there are more than `maximumAlarms`.
    public let alarms_minutes: [Int]?
    /// How many reminders Orbit does not edit (at a date, by place,
    /// after the start, or past the limits); `nil` for none, never 0.
    public let alarms_not_editable: Int?

    public init(id: String, calendar: CalendarReference, calendarItemID: String, eventID: String?,
                originalOccurrence: Int64?, start: Int64, end: Int64, eventTimezone: String?,
                interpretedTimezone: String, allDay: Bool, detached: Bool, title: String, status: String,
                lastModified: Int64?, invitees: [CalendarInvitee]?, inviteesStatus: String,
                eventURL: String?, eventURLStatus: String, location: String? = nil, notes: String? = nil,
                notesLeftOut: CalendarFieldLeftOut? = nil, organizer: CalendarInvitee? = nil,
                availability: CalendarAvailability? = nil, recurrence: CalendarRecurrence? = nil,
                canEdit: Bool? = nil, alarms: CalendarAlarmReading = .unread) throws {
        try CalendarOversizedField.check("title", title, CalendarLimits.maximumTitleBytes)
        try CalendarOversizedField.check("location", location, CalendarLimits.maximumTitleBytes)
        try CalendarOversizedField.check("event_url", eventURL, CalendarLimits.maximumURLBytes)
        try CalendarOversizedField.check("event_timezone", eventTimezone, CalendarLimits.maximumTimezoneBytes)
        try CalendarOversizedField.check("interpreted_timezone", interpretedTimezone,
            CalendarLimits.maximumTimezoneBytes)
        try CalendarOversizedField.check("calendar_item_id", calendarItemID, CalendarLimits.maximumLabelBytes)
        try CalendarOversizedField.check("event_id", eventID, CalendarLimits.maximumLabelBytes)
        guard CalendarLimits.uuid(id), calendar.isValid, CalendarLimits.identifier(calendarItemID),
              eventID.map(CalendarLimits.identifier) ?? true,
              CalendarLimits.date(start), CalendarLimits.date(end), end >= start,
              originalOccurrence.map(CalendarLimits.date) ?? true, lastModified.map(CalendarLimits.date) ?? true,
              title.utf8.count <= CalendarLimits.maximumTitleBytes,
              interpretedTimezone.utf8.count <= CalendarLimits.maximumTimezoneBytes, !interpretedTimezone.isEmpty,
              (eventTimezone?.utf8.count ?? 0) <= CalendarLimits.maximumTimezoneBytes,
              ["none", "confirmed", "tentative", "canceled", "unknown"].contains(status),
              ["not_requested", "not_provided", "provided"].contains(inviteesStatus),
              (inviteesStatus == "provided") == (invitees != nil),
              ["not_requested", "not_provided", "provided"].contains(eventURLStatus),
              (eventURLStatus == "provided") == (eventURL != nil),
              (eventURL?.utf8.count ?? 0) <= CalendarLimits.maximumURLBytes,
              (notes?.utf8.count ?? 0) <= CalendarLimits.maximumNotesBytes,
              notesLeftOut.map({ notes == nil && $0.limit == CalendarLimits.maximumNotesBytes && $0.bytes > $0.limit }) ?? true,
              recurrence.map({ $0.interval >= 1 && ($0.rules_count ?? 2) >= 2 }) ?? true,
              alarms.isValid else {
            throw CalendarFailure.limitExceeded
        }
        observation_id = id; self.calendar = calendar; calendar_item_id = calendarItemID; event_id = eventID
        original_occurrence_unix_ms = originalOccurrence; start_unix_ms = start; end_unix_ms = end
        event_timezone = eventTimezone; interpreted_timezone = interpretedTimezone; is_floating = eventTimezone == nil
        is_all_day = allDay; is_detached = detached; self.title = title; self.status = status
        last_modified_unix_ms = lastModified; self.invitees = invitees; invitees_status = inviteesStatus
        event_url = eventURL; event_url_status = eventURLStatus
        self.location = location; self.notes = notes; notes_left_out = notesLeftOut
        self.organizer = organizer; self.availability = availability; self.recurrence = recurrence
        can_edit = canEdit
        alarms_minutes = alarms.minutes; alarms_not_editable = alarms.notEditable
    }
}

/// One reminder as EventKit holds it, reduced to what decides whether Orbit
/// edits it. Pure: no EventKit here.
public struct CalendarAlarmFacts: Equatable, Sendable {
    /// Seconds from the start; negative is before it.
    public let relativeOffset: Double
    public let hasAbsoluteDate: Bool
    public let hasLocation: Bool
    public init(relativeOffset: Double, hasAbsoluteDate: Bool, hasLocation: Bool) {
        self.relativeOffset = relativeOffset; self.hasAbsoluteDate = hasAbsoluteDate
        self.hasLocation = hasLocation
    }

    /// Minutes before the start when Orbit edits this reminder — relative,
    /// at or before the start, whole minutes, within `maximumAlarmMinutes` —
    /// or `nil` when it does not (and a write keeps it as it is).
    public var editableMinutes: Int? {
        guard !hasAbsoluteDate, !hasLocation, relativeOffset <= 0 else { return nil }
        let seconds = -relativeOffset
        guard seconds.truncatingRemainder(dividingBy: 60) == 0 else { return nil }
        let minutes = Int(seconds / 60)
        return minutes <= CalendarLimits.maximumAlarmMinutes ? minutes : nil
    }
}

/// An event's reminders as a read carries them.
public struct CalendarAlarmReading: Equatable, Sendable {
    public let minutes: [Int]?
    public let notEditable: Int?
    public static let unread = CalendarAlarmReading(minutes: nil, notEditable: nil)

    public init(minutes: [Int]?, notEditable: Int?) {
        self.minutes = minutes; self.notEditable = notEditable
    }

    /// The editable ones sorted. Past `maximumAlarms` none travel and all
    /// count as not editable: the app then offers no change of reminders,
    /// so a write cannot replace a list it never saw whole.
    public init(_ alarms: [CalendarAlarmFacts]) {
        let editable = alarms.compactMap(\.editableMinutes).sorted()
        let fits = editable.count <= CalendarLimits.maximumAlarms
        let other = alarms.count - (fits ? editable.count : 0)
        minutes = fits ? editable : nil
        notEditable = other > 0 ? other : nil
    }

    var isValid: Bool {
        (minutes.map { $0.count <= CalendarLimits.maximumAlarms
            && $0.allSatisfy { (0...CalendarLimits.maximumAlarmMinutes).contains($0) } } ?? true)
            && (notEditable.map { $0 > 0 } ?? true)
    }
}

public struct CalendarEventOmission: Codable, Equatable, Sendable {
    public let reason: String
    public let count: Int

    public init(reason: String, count: Int) {
        self.reason = reason
        self.count = count
    }
}

public struct CalendarEventBatch: Equatable, Sendable {
    public let systemTimezone: String
    public let events: [CalendarOccurrence]
    public let omitted: [CalendarEventOmission]
    /// Events of the window that could not travel, named.
    public let oversized: [CalendarOversizedEvent]
    public init(systemTimezone: String, events: [CalendarOccurrence], omitted: [CalendarEventOmission] = [],
                oversized: [CalendarOversizedEvent] = []) {
        self.systemTimezone = systemTimezone
        self.events = events
        self.omitted = omitted
        self.oversized = oversized
    }
}

/// One page of one window of every calendar. No scope — every event
/// carries its own calendar, and invitees and URL are always read.
public struct CalendarReadPage: Encodable, Equatable, Sendable {
    public let read_id: String
    public let native_instance_id: String
    public let privacy_generation: UInt64
    public let window: CalendarWindow
    public let clock_domain = "mach_absolute_time"
    public let capture_started_monotonic_ns: UInt64
    public let capture_completed_monotonic_ns: UInt64
    public let observed_at_unix_ms: Int64
    public let system_timezone: String
    public let provider_sync_status = "unknown"
    public let enumeration = "bounded_local_read"
    public let events: [CalendarOccurrence]
    public let omitted: [CalendarEventOmission]
    /// Events of the window that could not travel, named; `nil` (and absent
    /// from the wire) when there is none, so the page is otherwise unchanged.
    public let oversized: [CalendarOversizedEvent]?
    public let after_index: UInt32
    public let total: UInt32
    public let next_after_index: UInt32?
    public init(id: String, nativeInstanceID: String, generation: UInt64, window: CalendarWindow,
                started: UInt64, completed: UInt64, observedAt: Int64, systemTimezone: String,
                events: [CalendarOccurrence], omitted: [CalendarEventOmission],
                oversized: [CalendarOversizedEvent] = [],
                afterIndex: UInt32, total: UInt32, nextAfterIndex: UInt32?) {
        read_id = id; native_instance_id = nativeInstanceID; privacy_generation = generation
        self.window = window; capture_started_monotonic_ns = started; capture_completed_monotonic_ns = completed
        observed_at_unix_ms = observedAt; system_timezone = systemTimezone
        self.events = events; self.omitted = omitted
        self.oversized = oversized.isEmpty ? nil : oversized
        after_index = afterIndex; self.total = total; next_after_index = nextAfterIndex
    }
}

/// How one enumeration of a window is cut into pages. Pure: no EventKit, no clock.
public enum CalendarPaging {
    /// The same order on every enumeration of the same store state, so a page
    /// index means the same event on the next page request.
    public static func ordered(_ events: [CalendarOccurrence]) -> [CalendarOccurrence] {
        events.sorted { left, right in
            if left.start_unix_ms != right.start_unix_ms { return left.start_unix_ms < right.start_unix_ms }
            if left.end_unix_ms != right.end_unix_ms { return left.end_unix_ms < right.end_unix_ms }
            if left.calendar.calendar_id != right.calendar.calendar_id {
                return left.calendar.calendar_id < right.calendar.calendar_id
            }
            if left.calendar.source_id != right.calendar.source_id {
                return left.calendar.source_id < right.calendar.source_id
            }
            if left.calendar_item_id != right.calendar_item_id { return left.calendar_item_id < right.calendar_item_id }
            let leftOriginal = left.original_occurrence_unix_ms ?? Int64.min
            let rightOriginal = right.original_occurrence_unix_ms ?? Int64.min
            if leftOriginal != rightOriginal { return leftOriginal < rightOriginal }
            if (left.event_id ?? "") != (right.event_id ?? "") { return (left.event_id ?? "") < (right.event_id ?? "") }
            return left.title < right.title
        }
    }

    /// The items of `values` from `after` that fill `budget` bytes, whole, and
    /// where the next page starts. At least one item while items remain, so a
    /// walk over pages always moves. `after` past the end is refused: the list
    /// this index was computed against is not the one being paged now.
    public static func page<T: Encodable>(_ values: [T], after: Int, budget: Int) throws -> (items: [T], next: Int?) {
        guard after >= 0, after <= values.count else { throw CalendarFailure.invalidated }
        var items = [T]()
        var used = 0
        var index = after
        while index < values.count {
            let cost = try CalendarLimits.encodedSize(values[index]) + 1
            if !items.isEmpty && used + cost > budget { break }
            items.append(values[index]); used += cost; index += 1
            if used > budget { break }
        }
        return (items, index < values.count ? index : nil)
    }
}
