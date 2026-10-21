import Foundation
import EventKit
import OrbitNativeProtocol

/// The writes. EventKit objects never leave the actor; the controller
/// sees values and has already decided the write may go (the calendar admits
/// changes, nobody else organized the event, the dates make sense).
///
/// An occurrence is found by its event (`calendarItemIdentifier`) and the
/// original date of the occurrence (`occurrenceDate`), never by an EventKit
/// event identifier alone, which may be absent. Every commit is a single
/// `save`/`remove` with `commit: true`; EventKit then posts
/// `EKEventStoreChanged`, and the daemon reads the agenda again.
extension EventKitCalendarReader {
    /// How far from its original date a moved occurrence of a series is looked
    /// for: one read window (`READ_WINDOW_MS`, 31 days) either side. An
    /// occurrence moved further is not found, and the refusal names the event
    /// and the date; it is never matched by guess.
    static let occurrenceSearchMilliseconds: Int64 = 31 * 86_400_000

    public func writableCalendar(_ id: String?, gate: CalendarReadGate) throws -> CalendarWriteCalendar {
        try authorize(gate)
        let store = requireStore()
        let calendar: EKCalendar
        if let id {
            guard let found = store.calendar(withIdentifier: id) else {
                throw CalendarWriteValidation.refusal("calendar_id", .unknownCalendar)
            }
            calendar = found
        } else {
            guard let found = store.defaultCalendarForNewEvents else {
                throw CalendarWriteValidation.refusal("calendar_id", .noDefaultCalendar)
            }
            calendar = found
        }
        try authorize(gate)
        guard let source = calendar.source else { throw CalendarFailure.readFailed }
        return CalendarWriteCalendar(
            reference: CalendarReference(calendarID: calendar.calendarIdentifier, sourceID: source.sourceIdentifier),
            canEdit: calendar.allowsContentModifications)
    }

    public func storedOccurrence(_ target: CalendarOccurrenceTarget,
                                 gate: CalendarReadGate) throws -> CalendarStoredOccurrence? {
        try authorize(gate)
        guard let event = try locate(target, store: requireStore()) else { return nil }
        try authorize(gate)
        guard let calendar = event.calendar, let source = calendar.source,
              let startDate = event.startDate, let endDate = event.endDate else {
            throw CalendarFailure.readFailed
        }
        return CalendarStoredOccurrence(
            calendar: CalendarReference(calendarID: calendar.calendarIdentifier, sourceID: source.sourceIdentifier),
            start: try Self.milliseconds(startDate), end: try Self.milliseconds(endDate),
            calendarCanEdit: calendar.allowsContentModifications,
            organizedBySomeoneElse: event.organizer.map { !$0.isCurrentUser } ?? false)
    }

    public func create(_ draft: CalendarEventDraft, in calendar: CalendarWriteCalendar,
                       gate: CalendarReadGate) throws -> CalendarWriteOutcome {
        try authorize(gate)
        let store = requireStore()
        guard let destination = store.calendar(withIdentifier: calendar.reference.calendar_id) else {
            throw CalendarWriteValidation.refusal("calendar_id", .unknownCalendar)
        }
        let event = EKEvent(eventStore: store)
        try CalendarStoreTransaction.run(reset: { store.reset() }) {
            event.calendar = destination
            try apply(CalendarEventChanges(title: draft.title, start: draft.start_ms, end: draft.end_ms,
                allDay: draft.all_day, timeZone: draft.time_zone, location: draft.location,
                notes: draft.notes, url: draft.url, availability: draft.availability,
                recurrence: draft.recurrence, alarmsMinutes: draft.alarms_minutes), to: event,
                preservingOtherAlarms: false)
            try authorize(gate)
            try Self.commit { try store.save(event, span: .thisEvent, commit: true) }
        }
        return try describe(event)
    }

    public func change(_ change: CalendarEventChange, gate: CalendarReadGate) throws -> CalendarWriteOutcome {
        try authorize(gate)
        let store = requireStore()
        guard let event = try locate(change.target, store: store) else {
            throw CalendarWriteRefusal.eventNotFound(change.target)
        }
        let changes = change.changes
        try CalendarStoreTransaction.run(reset: { store.reset() }) {
            if let id = changes.calendar_id {
                guard let destination = store.calendar(withIdentifier: id) else {
                    throw CalendarWriteValidation.refusal("calendar_id", .unknownCalendar)
                }
                event.calendar = destination
            }
            try apply(changes, to: event, preservingOtherAlarms: true)
            try authorize(gate)
            try Self.commit { try store.save(event, span: Self.eventKit(change.span), commit: true) }
        }
        return try describe(event)
    }

    private func apply(_ changes: CalendarEventChanges, to event: EKEvent,
                       preservingOtherAlarms: Bool) throws {
        if let title = changes.title { event.title = title }
        if let start = changes.start_ms { event.startDate = Self.date(start) }
        if let end = changes.end_ms { event.endDate = Self.date(end) }
        if let allDay = changes.all_day { event.isAllDay = allDay }
        if let zone = changes.time_zone { event.timeZone = try Self.timeZone(zone) }
        if let location = changes.location { event.location = location.isEmpty ? nil : location }
        if let notes = changes.notes { event.notes = notes.isEmpty ? nil : notes }
        if let url = changes.url { event.url = url.isEmpty ? nil : try Self.url(url) }
        if let availability = changes.availability { event.availability = Self.eventKit(availability) }
        if changes.remove_recurrence == true || changes.recurrence != nil {
            for rule in event.recurrenceRules ?? [] { event.removeRecurrenceRule(rule) }
        }
        if let rule = changes.recurrence { event.addRecurrenceRule(Self.eventKit(rule)) }
        if let minutes = changes.alarms_minutes {
            // The list replaces the reminders Orbit edits; the
            // others (at a date, by place, after the start) stay.
            let kept = preservingOtherAlarms ? (event.alarms ?? []).filter {
                EventKitCalendarReader.alarmFacts($0).editableMinutes == nil
            } : []
            event.alarms = kept + minutes.map(Self.alarm)
        }
    }

    public func delete(_ request: CalendarDeleteRequest, gate: CalendarReadGate) throws {
        try authorize(gate)
        let store = requireStore()
        guard let event = try locate(request.target, store: store) else {
            throw CalendarWriteRefusal.eventNotFound(request.target)
        }
        try CalendarStoreTransaction.run(reset: { store.reset() }) {
            try authorize(gate)
            try Self.commit { try store.remove(event, span: Self.eventKit(request.span), commit: true) }
        }
    }

    /// The occurrence of `calendarItemID` whose original date is
    /// `occurrenceStart`. A single event has one occurrence, its own; a series
    /// is searched around that date, where a moved occurrence still keeps it.
    private func locate(_ target: CalendarOccurrenceTarget, store: EKEventStore) throws -> EKEvent? {
        guard let item = store.calendarItem(withIdentifier: target.calendarItemID) as? EKEvent else { return nil }
        if !item.hasRecurrenceRules && !item.isDetached {
            // Both are implicitly unwrapped in EventKit's headers; read them as
            // the optionals they are.
            let occurrenceDate: Date? = item.occurrenceDate
            let startDate: Date? = item.startDate
            guard let original = occurrenceDate ?? startDate else { return nil }
            return try Self.milliseconds(original) == target.occurrenceStart ? item : nil
        }
        let reach = Self.occurrenceSearchMilliseconds
        let lower = max(target.occurrenceStart - reach, -62_135_596_800_000)
        let upper = min(target.occurrenceStart + reach, 253_402_300_799_999)
        let predicate = store.predicateForEvents(withStart: Self.date(lower), end: Self.date(upper), calendars: nil)
        for event in store.events(matching: predicate) where event.calendarItemIdentifier == target.calendarItemID {
            guard let original = event.occurrenceDate else { continue }
            if try Self.milliseconds(original) == target.occurrenceStart { return event }
        }
        return nil
    }

    /// The occurrence as a read describes it. The commit is done: a failure
    /// to describe it now does not undo it, so it is no refusal — the outcome
    /// is unknown to the daemon, which reads the agenda again.
    private func describe(_ event: EKEvent) throws -> CalendarWriteOutcome {
        let timezone = TimeZone.current.identifier
        // Already committed: nothing may retire this answer half-way, so the
        // gate here only asks EventKit for the permission it still has.
        let open = CalendarReadGate(started: 0, deadline: UInt64.max, clock: { 1 })
        do {
            let occurrence = try occurrence(event, timezone: timezone, gate: open)
            return CalendarWriteOutcome(occurrence: occurrence, systemTimezone: timezone)
        } catch {
            throw CalendarWriteRefusal.outcomeUnknown(cause: "saved_but_not_readable")
        }
    }

    nonisolated static func date(_ milliseconds: Int64) -> Date {
        Date(timeIntervalSince1970: Double(milliseconds) / 1000)
    }

    nonisolated static func timeZone(_ identifier: String) throws -> TimeZone {
        guard let zone = TimeZone(identifier: identifier) else {
            throw CalendarWriteValidation.refusal("time_zone", .unknownTimeZone)
        }
        return zone
    }

    nonisolated static func url(_ value: String) throws -> URL {
        guard let url = URL(string: value), url.scheme != nil else {
            throw CalendarWriteValidation.refusal("url", .invalidURL)
        }
        return url
    }

    nonisolated static func alarm(_ minutes: Int) -> EKAlarm {
        EKAlarm(relativeOffset: -TimeInterval(minutes) * 60)
    }

    nonisolated static func eventKit(_ span: CalendarSpan) -> EKSpan {
        switch span {
        case .thisEvent: return .thisEvent
        case .futureEvents: return .futureEvents
        }
    }

    nonisolated static func eventKit(_ availability: CalendarAvailability) -> EKEventAvailability {
        switch availability {
        case .busy: return .busy
        case .free: return .free
        case .tentative: return .tentative
        case .unavailable: return .unavailable
        case .notSupported: return .notSupported
        }
    }

    nonisolated static func eventKit(_ rule: CalendarRecurrenceRule) -> EKRecurrenceRule {
        let frequency: EKRecurrenceFrequency
        switch rule.frequency {
        case .daily: frequency = .daily
        case .weekly: frequency = .weekly
        case .monthly: frequency = .monthly
        case .yearly: frequency = .yearly
        }
        let days = rule.days_of_week?.compactMap { day -> EKRecurrenceDayOfWeek? in
            guard let weekday = EKWeekday(rawValue: day.day.eventKitNumber) else { return nil }
            return EKRecurrenceDayOfWeek(weekday, weekNumber: day.week ?? 0)
        }
        var end: EKRecurrenceEnd?
        if let until = rule.end?.until_ms {
            end = EKRecurrenceEnd(end: date(until))
        } else if let count = rule.end?.count {
            end = EKRecurrenceEnd(occurrenceCount: count)
        }
        return EKRecurrenceRule(recurrenceWith: frequency, interval: rule.interval, daysOfTheWeek: days,
            daysOfTheMonth: nil, monthsOfTheYear: nil, weeksOfTheYear: nil, daysOfTheYear: nil,
            setPositions: nil, end: end)
    }

    /// EventKit's refusal by its domain and code, never its text.
    nonisolated static func commit(_ body: () throws -> Void) throws {
        do { try body() } catch {
            let failure = error as NSError
            throw CalendarWriteRefusal.saveFailed(domain: failure.domain, code: failure.code)
        }
    }
}
