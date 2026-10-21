import Foundation
import OrbitNativeProtocol

/// What the controller decides about a write before EventKit is asked for a
/// commit. Pure: values in, a refusal with its cause out.
///
/// An event Orbit may not change is refused by name: a calendar that does not
/// admit changes, or an event someone else organized — an invitation is moved
/// by whoever sent it, as in Calendar.
public enum CalendarWritePolicy {
    public static func refusal(for calendar: CalendarWriteCalendar) -> CalendarWriteRefusal? {
        calendar.canEdit ? nil : .notEditable(.calendarReadOnly, calendarID: calendar.reference.calendar_id)
    }

    public static func refusal(for stored: CalendarStoredOccurrence) -> CalendarWriteRefusal? {
        if !stored.calendarCanEdit {
            return .notEditable(.calendarReadOnly, calendarID: stored.calendar.calendar_id)
        }
        if stored.organizedBySomeoneElse {
            return .notEditable(.organizedBySomeoneElse, calendarID: stored.calendar.calendar_id)
        }
        return nil
    }

    /// A change that moves only one end checks it against the other as it is
    /// now: the end before the start is refused with both dates.
    public static func checkDates(_ stored: CalendarStoredOccurrence, _ changes: CalendarEventChanges) throws {
        try CalendarWriteValidation.ordered(start: changes.start_ms ?? stored.start,
                                            end: changes.end_ms ?? stored.end)
    }
}

/// One EventKit write: the changes to an event and their commit. EventKit's
/// store is shared by every read and write of the helper; an event changed in
/// memory and then refused (an unknown zone, a failed `save`/`remove`, a gate
/// that closed) would otherwise stay changed in it, and a later read — or the
/// next commit — would show or apply a change the user was told did not happen.
/// On any failure the store is reset; the failure goes on unchanged.
enum CalendarStoreTransaction {
    static func run<T>(reset: () -> Void, _ body: () throws -> T) throws -> T {
        do { return try body() } catch {
            reset()
            throw error
        }
    }
}
