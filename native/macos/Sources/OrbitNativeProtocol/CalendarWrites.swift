import Foundation

// The events the user creates, moves, edits and deletes from Orbit. The
// macOS permission (full access) is the whole decision: nothing here asks for
// a confirmation. Mirrors `orbit_core::calendar::writes`; the fixtures in
// `contracts/fixtures/native/calendar/` are decoded on both sides.

public enum CalendarAvailability: String, Codable, Equatable, Sendable {
    case busy, free, tentative, unavailable
    /// The calendar keeps no availability. Read only: a write never asks for it.
    case notSupported = "not_supported"
}

public enum CalendarFrequency: String, Codable, Equatable, Sendable {
    case daily, weekly, monthly, yearly
}

public enum CalendarWeekday: String, Codable, Equatable, Hashable, Sendable {
    case sunday, monday, tuesday, wednesday, thursday, friday, saturday

    /// EventKit numbers the week from Sunday, 1, to Saturday, 7.
    public var eventKitNumber: Int {
        switch self {
        case .sunday: 1
        case .monday: 2
        case .tuesday: 3
        case .wednesday: 4
        case .thursday: 5
        case .friday: 6
        case .saturday: 7
        }
    }

    public init?(eventKitNumber: Int) {
        switch eventKitNumber {
        case 1: self = .sunday
        case 2: self = .monday
        case 3: self = .tuesday
        case 4: self = .wednesday
        case 5: self = .thursday
        case 6: self = .friday
        case 7: self = .saturday
        default: return nil
        }
    }
}

/// One day a rule repeats on; `week` (−53…53, never 0) picks one week of the
/// month or year, and is absent for every week.
public struct CalendarRecurrenceDay: Codable, Equatable, Hashable, Sendable {
    public let day: CalendarWeekday
    public let week: Int?
    public init(day: CalendarWeekday, week: Int? = nil) { self.day = day; self.week = week }
}

/// Where a rule stops: a date or a number of occurrences, exactly one.
public struct CalendarRecurrenceEnd: Codable, Equatable, Sendable {
    public let until_ms: Int64?
    public let count: Int?
    public init(untilMs: Int64? = nil, count: Int? = nil) { until_ms = untilMs; self.count = count }
}

/// One repetition rule, as a write asks for it.
public struct CalendarRecurrenceRule: Codable, Equatable, Sendable {
    public let frequency: CalendarFrequency
    public let interval: Int
    public let days_of_week: [CalendarRecurrenceDay]?
    public let end: CalendarRecurrenceEnd?
    public init(frequency: CalendarFrequency, interval: Int, daysOfWeek: [CalendarRecurrenceDay]? = nil,
                end: CalendarRecurrenceEnd? = nil) {
        self.frequency = frequency; self.interval = interval; days_of_week = daysOfWeek; self.end = end
    }
}

/// How an event read from the Mac repeats: its first rule, and `rules_count`
/// when EventKit holds more than one.
public struct CalendarRecurrence: Codable, Equatable, Sendable {
    public let frequency: CalendarFrequency
    public let interval: Int
    public let days_of_week: [CalendarRecurrenceDay]?
    public let end: CalendarRecurrenceEnd?
    public let rules_count: Int?
    public init(frequency: CalendarFrequency, interval: Int, daysOfWeek: [CalendarRecurrenceDay]? = nil,
                end: CalendarRecurrenceEnd? = nil, rulesCount: Int? = nil) {
        self.frequency = frequency; self.interval = interval; days_of_week = daysOfWeek
        self.end = end; rules_count = rulesCount
    }
}

/// EventKit's two spans. The whole series is `future` from its first
/// occurrence; there is no third value.
public enum CalendarSpan: String, Codable, Equatable, Sendable {
    case thisEvent = "this"
    case futureEvents = "future"
}

/// A new event. Without `calendar_id`, the calendar macOS uses for new events.
public struct CalendarEventDraft: Codable, Equatable, Sendable {
    public let calendar_id: String?
    public let title: String
    public let start_ms: Int64
    public let end_ms: Int64
    public let all_day: Bool
    public let time_zone: String?
    public let location: String?
    public let notes: String?
    public let url: String?
    public let availability: CalendarAvailability?
    public let recurrence: CalendarRecurrenceRule?
    /// Minutes before the start each alarm sounds (0: at the start).
    public let alarms_minutes: [Int]?

    init(calendarID: String? = nil, title: String, start: Int64, end: Int64, allDay: Bool = false,
                timeZone: String? = nil, location: String? = nil, notes: String? = nil, url: String? = nil,
                availability: CalendarAvailability? = nil, recurrence: CalendarRecurrenceRule? = nil,
                alarmsMinutes: [Int]? = nil) {
        calendar_id = calendarID; self.title = title; start_ms = start; end_ms = end; all_day = allDay
        time_zone = timeZone; self.location = location; self.notes = notes; self.url = url
        self.availability = availability; self.recurrence = recurrence; alarms_minutes = alarmsMinutes
    }
}

/// What a change sets; absent leaves a field as it is. An empty `location`,
/// `notes` or `url` clears it; `remove_recurrence` stops the repetition;
/// `alarms_minutes` replaces every alarm.
public struct CalendarEventChanges: Codable, Equatable, Sendable {
    public let calendar_id: String?
    public let title: String?
    public let start_ms: Int64?
    public let end_ms: Int64?
    public let all_day: Bool?
    public let time_zone: String?
    public let location: String?
    public let notes: String?
    public let url: String?
    public let availability: CalendarAvailability?
    public let recurrence: CalendarRecurrenceRule?
    public let remove_recurrence: Bool?
    public let alarms_minutes: [Int]?

    public init(calendarID: String? = nil, title: String? = nil, start: Int64? = nil, end: Int64? = nil,
                allDay: Bool? = nil, timeZone: String? = nil, location: String? = nil, notes: String? = nil,
                url: String? = nil, availability: CalendarAvailability? = nil,
                recurrence: CalendarRecurrenceRule? = nil, removeRecurrence: Bool? = nil,
                alarmsMinutes: [Int]? = nil) {
        calendar_id = calendarID; self.title = title; start_ms = start; end_ms = end; all_day = allDay
        time_zone = timeZone; self.location = location; self.notes = notes; self.url = url
        self.availability = availability; self.recurrence = recurrence
        remove_recurrence = removeRecurrence; alarms_minutes = alarmsMinutes
    }
}

/// Names one occurrence: its event and the original date of the occurrence.
public struct CalendarOccurrenceTarget: Equatable, Hashable, Sendable {
    public let calendarItemID: String
    public let occurrenceStart: Int64
    public init(calendarItemID: String, occurrenceStart: Int64) {
        self.calendarItemID = calendarItemID; self.occurrenceStart = occurrenceStart
    }
}

public struct CalendarEventChange: Codable, Equatable, Sendable {
    public let calendar_item_id: String
    public let occurrence_start_ms: Int64
    public let span: CalendarSpan
    public let changes: CalendarEventChanges
    public init(calendarItemID: String, occurrenceStart: Int64, span: CalendarSpan, changes: CalendarEventChanges) {
        calendar_item_id = calendarItemID; occurrence_start_ms = occurrenceStart
        self.span = span; self.changes = changes
    }
    public var target: CalendarOccurrenceTarget {
        CalendarOccurrenceTarget(calendarItemID: calendar_item_id, occurrenceStart: occurrence_start_ms)
    }
}

/// `calendar_save`: exactly one of `create` and `change`.
public struct CalendarSaveRequest: Codable, Equatable, Sendable {
    public let create: CalendarEventDraft?
    public let change: CalendarEventChange?
    private enum CodingKeys: String, CodingKey { case create, change }
    public init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        create = try fields.decodeIfPresent(CalendarEventDraft.self, forKey: .create)
        change = try fields.decodeIfPresent(CalendarEventChange.self, forKey: .change)
        guard (create != nil) != (change != nil) else { throw CalendarFailure.invalidRequest }
    }
    public init(create: CalendarEventDraft) { self.create = create; change = nil }
    public init(change: CalendarEventChange) { create = nil; self.change = change }
}

/// `calendar_delete`: one occurrence, or it and the ones after it.
public struct CalendarDeleteRequest: Codable, Equatable, Sendable {
    public let calendar_item_id: String
    public let occurrence_start_ms: Int64
    public let span: CalendarSpan
    public init(calendarItemID: String, occurrenceStart: Int64, span: CalendarSpan) {
        calendar_item_id = calendarItemID; occurrence_start_ms = occurrenceStart; self.span = span
    }
    public var target: CalendarOccurrenceTarget {
        CalendarOccurrenceTarget(calendarItemID: calendar_item_id, occurrenceStart: occurrence_start_ms)
    }
}

/// The answer to `calendar_save`: the occurrence as it is now, the shape of a read.
public struct CalendarSaveResult: Encodable, Equatable, Sendable {
    public let write_id: String
    public let native_instance_id: String
    public let privacy_generation: UInt64
    public let system_timezone: String
    public let occurrence: CalendarOccurrence
    public init(id: String, nativeInstanceID: String, generation: UInt64, systemTimezone: String,
                occurrence: CalendarOccurrence) {
        write_id = id; native_instance_id = nativeInstanceID; privacy_generation = generation
        system_timezone = systemTimezone; self.occurrence = occurrence
    }
}

/// The answer to `calendar_delete`: what was removed.
public struct CalendarDeleteResult: Encodable, Equatable, Sendable {
    public let write_id: String
    public let native_instance_id: String
    public let privacy_generation: UInt64
    public let calendar_item_id: String
    public let occurrence_start_ms: Int64
    public let span: CalendarSpan
    public init(id: String, nativeInstanceID: String, generation: UInt64, request: CalendarDeleteRequest) {
        write_id = id; native_instance_id = nativeInstanceID; privacy_generation = generation
        calendar_item_id = request.calendar_item_id; occurrence_start_ms = request.occurrence_start_ms
        span = request.span
    }
}

public enum CalendarNotEditable: String, Equatable, Sendable {
    case calendarReadOnly = "calendar_read_only"
    case organizedBySomeoneElse = "organized_by_someone_else"
}

public enum CalendarChangeProblem: String, Equatable, Sendable {
    case empty
    case tooLarge = "too_large"
    case tooMany = "too_many"
    case endBeforeStart = "end_before_start"
    case invalidDate = "invalid_date"
    case invalidIdentifier = "invalid_identifier"
    case invalidValue = "invalid_value"
    case unknownCalendar = "unknown_calendar"
    case noDefaultCalendar = "no_default_calendar"
    case unknownTimeZone = "unknown_time_zone"
    case invalidURL = "invalid_url"
    case recurrenceNeedsFutureSpan = "recurrence_needs_future_span"
    case nothingToChange = "nothing_to_change"
}

/// One field of a write that cannot be applied, with its numbers.
public struct CalendarInvalidChange: Equatable, Sendable {
    public let field: String
    public let problem: CalendarChangeProblem
    public var bytes: Int?
    public var count: Int?
    public var limit: Int?
    public var value: Int64?
    public var startMs: Int64?
    public var endMs: Int64?
    public init(field: String, problem: CalendarChangeProblem) { self.field = field; self.problem = problem }
}

/// A write not applied, or whose outcome nobody can know. `code` is the
/// native error code; `detail` travels next to it with every number.
public enum CalendarWriteRefusal: Error, Equatable, Sendable {
    case eventNotFound(CalendarOccurrenceTarget)
    case notEditable(CalendarNotEditable, calendarID: String)
    case invalidChange(CalendarInvalidChange)
    case permissionDenied(CalendarAuthorization)
    case saveFailed(domain: String, code: Int)
    case outcomeUnknown(cause: String)

    public var code: String {
        switch self {
        case .eventNotFound: "calendar_event_not_found"
        case .notEditable: "calendar_not_editable"
        case .invalidChange: "calendar_invalid_change"
        case .permissionDenied: "calendar_permission_denied"
        case .saveFailed: "calendar_save_failed"
        case .outcomeUnknown: "calendar_write_outcome_unknown"
        }
    }

    public var detail: [String: Any] {
        switch self {
        case .eventNotFound(let target):
            return ["calendar_item_id": target.calendarItemID, "occurrence_start_ms": target.occurrenceStart]
        case .notEditable(let reason, let calendarID):
            return ["reason": reason.rawValue, "calendar_id": calendarID]
        case .invalidChange(let change):
            var fields: [String: Any] = ["field": change.field, "problem": change.problem.rawValue]
            if let bytes = change.bytes { fields["bytes"] = bytes }
            if let count = change.count { fields["count"] = count }
            if let limit = change.limit { fields["limit"] = limit }
            if let value = change.value { fields["value"] = value }
            if let start = change.startMs { fields["start_ms"] = start }
            if let end = change.endMs { fields["end_ms"] = end }
            return fields
        case .permissionDenied(let authorization):
            return ["authorization": authorization.rawValue]
        case .saveFailed(let domain, let code):
            return ["error_domain": domain, "error_code": code]
        case .outcomeUnknown(let cause):
            return ["cause": cause]
        }
    }

    /// English diagnostic text alongside the refusal code. The
    /// daemon builds what the user reads from the code and the detail.
    public var message: String {
        switch self {
        case .eventNotFound: "There is no occurrence of that event on that date; nothing was changed."
        case .notEditable(.calendarReadOnly, _): "The calendar is read-only; nothing was changed."
        case .notEditable(.organizedBySomeoneElse, _): "Someone else organized the event; nothing was changed."
        case .invalidChange(let change): "Field \(change.field) is invalid (\(change.problem.rawValue)); nothing was changed."
        case .permissionDenied: "macOS has not granted Orbit full calendar access; nothing was changed."
        case .saveFailed: "EventKit rejected the change; nothing was changed."
        case .outcomeUnknown: "The calendar change outcome is unknown; Orbit will not repeat it."
        }
    }
}

/// Refusals that need facts read by EventKit: the actual calendar, time zone,
/// URL conversion, or the other end of an occurrence being moved.
public enum CalendarWriteValidation {
    public static func refusal(_ field: String, _ problem: CalendarChangeProblem) -> CalendarWriteRefusal {
        .invalidChange(CalendarInvalidChange(field: field, problem: problem))
    }

    public static func ordered(start: Int64, end: Int64) throws {
        guard end >= start else {
            var change = CalendarInvalidChange(field: "end_ms", problem: .endBeforeStart)
            change.startMs = start; change.endMs = end
            throw CalendarWriteRefusal.invalidChange(change)
        }
    }
}
