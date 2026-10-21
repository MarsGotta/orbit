import Foundation
import Synchronization
import OrbitNativeProtocol

/// Only value DTOs leave the backend actor; EKEventStore and EKObject never do.
/// REF-10: both reads cover every calendar the permission allows; nothing is
/// selected, and the controller cuts the results into pages.
///
/// P9-21: the backend also writes. It offers values to decide on (a
/// calendar, an occurrence) and three commits; the controller decides —
/// whether the calendar admits changes, whether someone else organized the
/// event, whether the dates still make sense — before any commit is asked for.
public protocol CalendarBackend: Actor {
    func requestAccess(gate: CalendarReadGate) async throws -> CalendarAuthorization
    /// Every event calendar on the Mac.
    func sources(gate: CalendarReadGate) async throws -> [CalendarEntry]
    /// Every event of every calendar in the query's window, in any order.
    func read(query: CalendarReadQuery, gate: CalendarReadGate) async throws -> CalendarEventBatch
    /// The calendar a new event goes to: `id`, or the one macOS uses for new
    /// events when nil. Throws `CalendarWriteRefusal.invalidChange`
    /// (`unknown_calendar`, `no_default_calendar`) when there is none.
    func writableCalendar(_ id: String?, gate: CalendarReadGate) async throws -> CalendarWriteCalendar
    /// The occurrence `target` names, as values; nil when the store has none.
    func storedOccurrence(_ target: CalendarOccurrenceTarget, gate: CalendarReadGate) async throws
        -> CalendarStoredOccurrence?
    /// Commits a new event in `calendar` and describes it as a read would.
    /// A commit EventKit refuses throws `CalendarWriteRefusal.saveFailed`.
    func create(_ draft: CalendarEventDraft, in calendar: CalendarWriteCalendar,
                gate: CalendarReadGate) async throws -> CalendarWriteOutcome
    /// Commits a change to the occurrence (and, with `future`, the ones after
    /// it) and describes the occurrence as it is now.
    func change(_ change: CalendarEventChange, gate: CalendarReadGate) async throws -> CalendarWriteOutcome
    /// Removes the occurrence (and, with `future`, the ones after it).
    func delete(_ request: CalendarDeleteRequest, gate: CalendarReadGate) async throws
}

/// A calendar a write may go to, as values.
public struct CalendarWriteCalendar: Equatable, Sendable {
    public let reference: CalendarReference
    public let canEdit: Bool
    public init(reference: CalendarReference, canEdit: Bool) { self.reference = reference; self.canEdit = canEdit }
}

/// What the controller needs to know about an occurrence before changing it.
public struct CalendarStoredOccurrence: Equatable, Sendable {
    public let calendar: CalendarReference
    public let start: Int64
    public let end: Int64
    /// The calendar's `allowsContentModifications`.
    public let calendarCanEdit: Bool
    /// The event has an organizer who is not Mars.
    public let organizedBySomeoneElse: Bool
    public init(calendar: CalendarReference, start: Int64, end: Int64, calendarCanEdit: Bool,
                organizedBySomeoneElse: Bool) {
        self.calendar = calendar; self.start = start; self.end = end
        self.calendarCanEdit = calendarCanEdit; self.organizedBySomeoneElse = organizedBySomeoneElse
    }
}

/// A committed write, described as a read would describe it.
public struct CalendarWriteOutcome: Equatable, Sendable {
    public let occurrence: CalendarOccurrence
    public let systemTimezone: String
    public init(occurrence: CalendarOccurrence, systemTimezone: String) {
        self.occurrence = occurrence; self.systemTimezone = systemTimezone
    }
}

/// Compiler-checked Sendable cancellation, independent of a blocked SDK actor.
/// Cancellation prevents the NEXT getter/delivery; it cannot interrupt EventKit.
public final class CalendarReadGate: Sendable {
    private let failure = Mutex<CalendarFailure?>(nil)
    private let deadline: UInt64
    private let started: UInt64
    private let clock: @Sendable () -> UInt64?
    public init(started: UInt64, deadline: UInt64, clock: @escaping @Sendable () -> UInt64?) {
        self.started = started; self.deadline = deadline; self.clock = clock
    }
    public func revoke(_ reason: CalendarFailure = .invalidated) { failure.withLock { if $0 == nil { $0 = reason } } }
    public func check() throws {
        if let reason = failure.withLock({ $0 }) { throw reason }
        guard let now = clock() else { throw CalendarFailure.clockUnavailable }
        guard now >= started, now < deadline else { throw CalendarFailure.expired }
    }
}

public enum CalendarResult: Sendable {
    case authorization(CalendarAuthorization)
    case inventory(CalendarInventoryPage)
    case page(CalendarReadPage)
    case saved(CalendarSaveResult)
    case deleted(CalendarDeleteResult)
}

/// What the helper says about the calendar without reading it. REF-10: no
/// selection, so no `connected` or `scope`. P9-21: no `read_only` either —
/// the helper writes, and its `calendar` capability says so.
public struct CalendarStatus: Encodable, Sendable {
    public let authorization: CalendarAuthorization
    public let physical_read_pending: Bool
    public let failure: CalendarFailure?
    public init(authorization: CalendarAuthorization, physicalReadPending: Bool = false,
                failure: CalendarFailure? = nil) {
        self.authorization = authorization
        physical_read_pending = physicalReadPending; self.failure = failure
    }
}
