import Foundation
import CoreGraphics
import EventKit
import OrbitNativeProtocol

/// EventKit ownership never leaves this actor. No EKEventStore at initialization
/// and no fetch with write-only access.
///
/// Both reads cover every event calendar the macOS permission allows.
/// Nothing is selected, and no count of calendars, events or invitees is
/// refused here: the controller cuts the results into pages for the daemon.
///
/// It also saves and removes events the user changes from Orbit
/// (`EventKitCalendarWriter.swift`), with full access only — the permission is
/// the whole decision — and only after the controller decided the write may go.
public actor EventKitCalendarReader: CalendarBackend {
    private var eventStore: EKEventStore?
    public init() {}

    public nonisolated static func authorization() -> CalendarAuthorization {
        switch EKEventStore.authorizationStatus(for: .event) {
        case .notDetermined: return .notDetermined
        case .restricted: return .restricted
        case .denied: return .denied
        case .fullAccess: return .fullAccess
        case .writeOnly: return .writeOnly
        @unknown default: return .unknown
        }
    }

    public func requestAccess(gate: CalendarReadGate) async throws -> CalendarAuthorization {
        try gate.check()
        let current = Self.authorization()
        guard current == .notDetermined || current == .writeOnly else { return current }
        let store = requireStore()
        try gate.check()
        // The physical controller slot remains held until this callback returns,
        // even if the user leaves the TCC sheet open beyond the reply deadline.
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            store.requestFullAccessToEvents { _, error in
                if error != nil { continuation.resume(throwing: CalendarFailure.readFailed) }
                else { continuation.resume(returning: ()) }
            }
        }
        try gate.check()
        return Self.authorization() // The callback Bool is not persistent authority.
    }

    public func sources(gate: CalendarReadGate) throws -> [CalendarEntry] {
        try authorize(gate)
        let store = requireStore()
        try authorize(gate)
        let defaultID = store.defaultCalendarForNewEvents?.calendarIdentifier
        try authorize(gate)
        var entries = [CalendarEntry]()
        for calendar in store.calendars(for: .event) {
            try authorize(gate)
            let entry = try descriptor(calendar, defaultID: defaultID, gate: gate)
            guard !entries.contains(where: { $0.reference.calendar_id == entry.reference.calendar_id }) else {
                throw CalendarFailure.readFailed
            }
            entries.append(entry)
        }
        try authorize(gate)
        return entries
    }

    public func read(query: CalendarReadQuery, gate: CalendarReadGate) throws -> CalendarEventBatch {
        try authorize(gate)
        guard query.isValid else { throw CalendarFailure.invalidRequest }
        let store = requireStore()
        let timezone = TimeZone.current.identifier
        // EventKit specifies the process default zone. Never silently treat a
        // distinct overridden default as the observed system interpretation.
        guard NSTimeZone.default.identifier == timezone else { throw CalendarFailure.invalidated }
        try authorize(gate)
        var events = [CalendarOccurrence]()
        var omitted = [String: Int]()
        var oversized = [CalendarOversizedEvent]()
        var failure: CalendarFailure?
        // `calendars: nil` is every event calendar in the store: the permission
        // is the only thing that decides which calendars Orbit reads.
        let predicate = store.predicateForEvents(
            withStart: Date(timeIntervalSince1970: Double(query.start_unix_ms) / 1000),
            end: Date(timeIntervalSince1970: Double(query.end_unix_ms) / 1000),
            calendars: nil)
        try authorize(gate)
        // Synchronous SDK enumeration on this actor, not Main. Stop cooperatively
        // before the next event/getter; SDK-internal allocations are not bounded by us.
        store.enumerateEvents(matching: predicate) { event, stop in
            do {
                try self.authorize(gate)
                let value = try self.occurrence(event, timezone: timezone, gate: gate)
                guard value.start_unix_ms < query.end_unix_ms,
                      value.end_unix_ms > query.start_unix_ms
                        || value.start_unix_ms == value.end_unix_ms && value.start_unix_ms >= query.start_unix_ms else {
                    throw CalendarOccurrenceOmission.outsideQueryWindow
                }
                events.append(value)
            } catch let omission as CalendarOccurrenceOmission {
                omitted[omission.rawValue, default: 0] += 1
            } catch let left as CalendarOversizedEvent {
                // Decision 4: named, and the rest of the window goes on.
                oversized.append(left)
            } catch {
                failure = (error as? CalendarFailure) ?? .readFailed
                stop.pointee = true
            }
        }
        if let failure { throw failure }
        try authorize(gate)
        guard TimeZone.current.identifier == timezone, NSTimeZone.default.identifier == timezone else {
            throw CalendarFailure.invalidated
        }
        // This is one local-store observation of one window, not an atomic
        // database snapshot or proof that iCloud/Google/Exchange finished syncing.
        let omissions = omitted.keys.sorted().compactMap { reason in
            omitted[reason].map { CalendarEventOmission(reason: reason, count: $0) }
        }
        return CalendarEventBatch(systemTimezone: timezone, events: events, omitted: omissions,
            oversized: oversized)
    }

    func requireStore() -> EKEventStore {
        if let eventStore { return eventStore }
        let store = EKEventStore(); eventStore = store
        return store
    }

    func authorize(_ gate: CalendarReadGate) throws {
        try gate.check()
        guard Self.authorization() == .fullAccess else { throw CalendarFailure.permissionRequired }
    }

    private func descriptor(_ calendar: EKCalendar, defaultID: String?,
                            gate: CalendarReadGate) throws -> CalendarEntry {
        try authorize(gate)
        let id = calendar.calendarIdentifier
        try authorize(gate)
        guard let source = calendar.source else { throw CalendarFailure.readFailed }
        try authorize(gate)
        let sourceID = source.sourceIdentifier
        try authorize(gate)
        let title = calendar.title
        try authorize(gate)
        let sourceTitle = source.title
        try authorize(gate)
        let type: String
        switch calendar.type {
        case .local: type = "local"
        case .calDAV: type = "caldav"
        case .exchange: type = "exchange"
        case .subscription: type = "subscription"
        case .birthday: type = "birthday"
        @unknown default: type = "unknown"
        }
        try authorize(gate)
        let color = Self.hexColor(calendar.cgColor)
        try authorize(gate)
        let canEdit = calendar.allowsContentModifications
        let row = CalendarEntry(reference: CalendarReference(calendarID: id, sourceID: sourceID),
            title: title, sourceTitle: sourceTitle, calendarType: type,
            color: color, canEdit: canEdit, isDefault: defaultID == id)
        guard row.isValid else { throw CalendarFailure.limitExceeded }
        return row
    }

    /// The calendar's color as `#RRGGBB` in sRGB; nil when macOS gives none
    /// or one with no sRGB form (a pattern, say). Never a guessed default.
    nonisolated static func hexColor(_ color: CGColor?) -> String? {
        guard let color, let space = CGColorSpace(name: CGColorSpace.sRGB),
              let converted = color.converted(to: space, intent: .defaultIntent, options: nil),
              let components = converted.components, components.count >= 3 else { return nil }
        let channels = components.prefix(3).map { value -> Int in
            Int((min(max(value, 0), 1) * 255).rounded())
        }
        return String(format: "#%02X%02X%02X", channels[0], channels[1], channels[2])
    }
}
