import Foundation

/// REF-10: nothing is listed for choosing and nothing is selected. The helper
/// asks macOS for the permission, lists every calendar, and reads a window of
/// every calendar; the permission is the only thing any of them waits on.
/// P9-21: it also saves and deletes events Mars changes from Orbit, under the
/// same permission and in the same single EventKit slot.
public enum CalendarOperation: Equatable, Sendable {
    case requestAccess
    case sources(afterIndex: UInt32)
    case read(CalendarReadQuery)
    case save(CalendarSaveRequest)
    case delete(CalendarDeleteRequest)

    /// A write: its late answer may describe a change already applied.
    public var isWrite: Bool {
        switch self {
        case .save, .delete: true
        case .requestAccess, .sources, .read: false
        }
    }
}

public struct CalendarReadPermit: Equatable, Sendable {
    public let id: String
    public let operation: CalendarOperation
    public let generation: UInt64
    public let started: UInt64
    public let deadline: UInt64
}

/// Pure RAM-only authority over the single EventKit slot. Revocation and
/// timeout retire delivery, not the SDK slot, which is released only when the
/// framework actually returns. IDs, clocks and authorization are supplied by
/// adapters, never fabricated here.
public struct CalendarPolicy {
    public let nativeInstanceID: String
    public private(set) var pending: CalendarReadPermit?
    private var delivery = false

    public init(nativeInstanceID: String) { self.nativeInstanceID = nativeInstanceID }

    public mutating func begin(_ operation: CalendarOperation, id: String, now: UInt64,
                               generation: UInt64, stopped: Bool, authorization: CalendarAuthorization) throws -> CalendarReadPermit {
        guard pending == nil else { throw CalendarFailure.busy }
        // A permission request waits for a person, not for EventKit, so it is
        // given its own deadline instead of the read one.
        let lifetime = CalendarLimits.lifetime(for: operation)
        guard CalendarLimits.uuid(id), CalendarLimits.uuid(nativeInstanceID),
              now <= UInt64.max - lifetime else { throw CalendarFailure.invalidRequest }
        if operation != .requestAccess {
            guard !stopped else { throw CalendarFailure.privacyStopped }
            guard authorization == .fullAccess else { throw CalendarFailure.permissionRequired }
        }
        if case .read(let query) = operation, !query.isValid { throw CalendarFailure.invalidRequest }
        let permit = CalendarReadPermit(id: id, operation: operation, generation: generation,
            started: now, deadline: now + lifetime)
        pending = permit; delivery = true
        return permit
    }

    public func validate(_ permit: CalendarReadPermit, now: UInt64, generation: UInt64,
                         stopped: Bool, authorization: CalendarAuthorization) throws {
        guard pending == permit, delivery, permit.generation == generation else { throw CalendarFailure.invalidated }
        guard now >= permit.started, now < permit.deadline else { throw CalendarFailure.expired }
        if permit.operation != .requestAccess {
            guard !stopped else { throw CalendarFailure.privacyStopped }
            guard authorization == .fullAccess else { throw CalendarFailure.permissionRequired }
        }
    }

    public mutating func expire(_ permit: CalendarReadPermit) { if pending == permit { delivery = false } }
    public mutating func complete(_ permit: CalendarReadPermit) {
        guard pending == permit else { return }
        pending = nil; delivery = false
    }
    public mutating func invalidate() { delivery = false }
}
