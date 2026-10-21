import Foundation
import EventKit
import OrbitNativeProtocol

enum CalendarOccurrenceOmission: String, Error {
    case missingTitle = "missing_title"
    case outsideQueryWindow = "outside_query_window"
}

extension EventKitCalendarReader {
    /// Invitees and the event URL are always read — the macOS permission
    /// is the only consent. So are the location, the notes (up to
    /// `maximumNotesBytes`; longer ones are named by size), the organizer, the
    /// availability, the first repetition rule and whether Orbit may change
    /// the event. Still no Contacts queries, conference extraction or URL
    /// activation.
    func occurrence(_ event: EKEvent, timezone: String,
                    gate: CalendarReadGate) throws -> CalendarOccurrence {
        try authorize(gate)
        guard let calendar = event.calendar else { throw CalendarFailure.readFailed }
        try authorize(gate)
        let calendarID = calendar.calendarIdentifier
        try authorize(gate)
        guard let source = calendar.source else { throw CalendarFailure.readFailed }
        try authorize(gate)
        let reference = CalendarReference(calendarID: calendarID, sourceID: source.sourceIdentifier)
        try authorize(gate)
        let itemID = event.calendarItemIdentifier
        try authorize(gate)
        let eventID = event.eventIdentifier
        try authorize(gate)
        let occurrence = try event.occurrenceDate.map(Self.milliseconds)
        try authorize(gate)
        guard let startDate = event.startDate else { throw CalendarFailure.readFailed }
        let start = try Self.milliseconds(startDate)
        try authorize(gate)
        guard let endDate = event.endDate else { throw CalendarFailure.readFailed }
        let end = try Self.milliseconds(endDate)
        try authorize(gate)
        let eventTimezone = event.timeZone?.identifier
        try authorize(gate)
        let allDay = event.isAllDay
        try authorize(gate)
        let detached = event.isDetached
        try authorize(gate)
        guard let title = event.title else { throw CalendarOccurrenceOmission.missingTitle }
        try authorize(gate)
        let status: String
        switch event.status {
        case .none: status = "none"
        case .confirmed: status = "confirmed"
        case .tentative: status = "tentative"
        case .canceled: status = "canceled"
        @unknown default: status = "unknown"
        }
        try authorize(gate)
        let modified = try event.lastModifiedDate.map(Self.milliseconds)
        try authorize(gate)
        let location = event.location
        try authorize(gate)
        let rawNotes = event.notes
        let notesBytes = rawNotes?.utf8.count ?? 0
        let notesFit = notesBytes <= CalendarLimits.maximumNotesBytes
        let notesLeftOut = notesFit ? nil
            : CalendarFieldLeftOut(bytes: notesBytes, limit: CalendarLimits.maximumNotesBytes)
        try authorize(gate)
        let availability = Self.availability(event.availability)
        try authorize(gate)
        let recurrence = Self.recurrence(event.recurrenceRules)
        try authorize(gate)
        let alarms = CalendarAlarmReading((event.alarms ?? []).map(Self.alarmFacts))
        try authorize(gate)
        let calendarCanEdit = calendar.allowsContentModifications

        // A field past its bound leaves this event out by
        // name — its dates, identifier, field and sizes — never the read.
        do {
            var invitees: [CalendarInvitee]?
            var inviteesStatus = "not_provided"
            try authorize(gate)
            if let participants = event.attendees {
                // Every guest travels: a meeting with more guests than a page holds
                // is split by the daemon into fragments, never trimmed here.
                invitees = try participants.map { try participant($0, gate: gate) }
                inviteesStatus = "provided"
            }
            try authorize(gate)
            let eventURL = event.url?.absoluteString
            let eventURLStatus = eventURL == nil ? "not_provided" : "provided"
            try authorize(gate)
            let organizer = try event.organizer.map { try participant($0, gate: gate) }
            // An invitation is moved by whoever sent it, as in Calendar.
            let organizedBySomeoneElse = organizer.map { !$0.is_current_user } ?? false
            try authorize(gate)
            return try CalendarOccurrence(id: UUID().uuidString.lowercased(), calendar: reference,
                calendarItemID: itemID, eventID: eventID, originalOccurrence: occurrence, start: start, end: end,
                eventTimezone: eventTimezone, interpretedTimezone: eventTimezone ?? timezone,
                allDay: allDay, detached: detached, title: title, status: status, lastModified: modified,
                invitees: invitees, inviteesStatus: inviteesStatus, eventURL: eventURL, eventURLStatus: eventURLStatus,
                location: location, notes: notesFit ? rawNotes : nil, notesLeftOut: notesLeftOut,
                organizer: organizer, availability: availability, recurrence: recurrence,
                canEdit: calendarCanEdit && !organizedBySomeoneElse, alarms: alarms)
        } catch let oversized as CalendarOversizedField {
            throw CalendarOversizedEvent(calendar: reference,
                calendarItemID: oversized.field == "calendar_item_id" ? nil : itemID,
                start: start, end: end, field: oversized.field, bytes: oversized.bytes, limit: oversized.limit)
        }
    }

    func participant(_ value: EKParticipant, gate: CalendarReadGate) throws -> CalendarInvitee {
        try authorize(gate)
        let name = value.name
        try authorize(gate)
        let url = value.url.absoluteString
        try authorize(gate)
        let role: String
        switch value.participantRole {
        case .unknown: role = "unknown"
        case .required: role = "required"
        case .optional: role = "optional"
        case .chair: role = "chair"
        case .nonParticipant: role = "non_participant"
        @unknown default: role = "unknown"
        }
        try authorize(gate)
        let response: String
        switch value.participantStatus {
        case .unknown: response = "unknown"
        case .pending: response = "pending"
        case .accepted: response = "accepted"
        case .declined: response = "declined"
        case .tentative: response = "tentative"
        case .delegated: response = "delegated"
        case .completed: response = "completed"
        case .inProcess: response = "in_process"
        @unknown default: response = "unknown"
        }
        try authorize(gate)
        let type: String
        switch value.participantType {
        case .unknown: type = "unknown"
        case .person: type = "person"
        case .room: type = "room"
        case .resource: type = "resource"
        case .group: type = "group"
        @unknown default: type = "unknown"
        }
        try authorize(gate)
        let currentUser = value.isCurrentUser
        return try CalendarInvitee(name: name, url: url, role: role, responseStatus: response,
            participantType: type, isCurrentUser: currentUser)
    }

    /// What decides whether Orbit edits a reminder.
    nonisolated static func alarmFacts(_ alarm: EKAlarm) -> CalendarAlarmFacts {
        CalendarAlarmFacts(relativeOffset: alarm.relativeOffset,
                           hasAbsoluteDate: alarm.absoluteDate != nil,
                           hasLocation: alarm.structuredLocation != nil || alarm.proximity != .none)
    }

    nonisolated static func availability(_ value: EKEventAvailability) -> CalendarAvailability {
        switch value {
        case .notSupported: return .notSupported
        case .busy: return .busy
        case .free: return .free
        case .tentative: return .tentative
        case .unavailable: return .unavailable
        @unknown default: return .notSupported
        }
    }

    /// The first rule, as values; nil when the event does not repeat, or when
    /// EventKit's rule has no shape Orbit can carry (then it is not read,
    /// rather than stopping the read). `rules_count` only past one.
    nonisolated static func recurrence(_ rules: [EKRecurrenceRule]?) -> CalendarRecurrence? {
        guard let rules, let rule = rules.first, rule.interval >= 1 else { return nil }
        let frequency: CalendarFrequency
        switch rule.frequency {
        case .daily: frequency = .daily
        case .weekly: frequency = .weekly
        case .monthly: frequency = .monthly
        case .yearly: frequency = .yearly
        @unknown default: return nil
        }
        var days = [CalendarRecurrenceDay]()
        for value in rule.daysOfTheWeek ?? [] {
            guard let weekday = CalendarWeekday(eventKitNumber: value.dayOfTheWeek.rawValue) else { return nil }
            let week = value.weekNumber
            guard (-53...53).contains(week) else { return nil }
            let day = CalendarRecurrenceDay(day: weekday, week: week == 0 ? nil : week)
            if !days.contains(day) { days.append(day) }
        }
        var end: CalendarRecurrenceEnd?
        if let recurrenceEnd = rule.recurrenceEnd {
            if let date = recurrenceEnd.endDate {
                guard let until = try? milliseconds(date) else { return nil }
                end = CalendarRecurrenceEnd(untilMs: until)
            } else if recurrenceEnd.occurrenceCount > 0 {
                end = CalendarRecurrenceEnd(count: recurrenceEnd.occurrenceCount)
            }
        }
        return CalendarRecurrence(frequency: frequency, interval: rule.interval,
            daysOfWeek: days.isEmpty ? nil : days, end: end,
            rulesCount: rules.count > 1 ? rules.count : nil)
    }

    nonisolated static func milliseconds(_ date: Date) throws -> Int64 {
        let value = date.timeIntervalSince1970 * 1000
        guard value.isFinite, value >= -62_135_596_800_000, value <= 253_402_300_799_999 else {
            throw CalendarFailure.readFailed
        }
        return Int64(value.rounded(.towardZero))
    }
}
