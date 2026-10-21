import Foundation

/// The lifecycle of one physical capture source.
///
/// Drain confirmation is deliberately separate from capture activity: an
/// unconfirmed drain is no longer recording and may be started again, while its
/// uncertain outcome remains visible to status consumers.
public enum CaptureSlot: Equatable, Sendable {
    case idle
    case starting
    case active
    case draining
    case unconfirmed(reason: String)

    public var isActive: Bool {
        switch self {
        case .starting, .active:
            return true
        case .idle, .draining, .unconfirmed:
            return false
        }
    }

    public var drainUnconfirmed: Bool {
        if case .unconfirmed = self { return true }
        return false
    }

    /// Applies only transitions that preserve the physical capture lifecycle.
    @discardableResult
    public mutating func transition(to destination: CaptureSlot) -> Bool {
        switch (self, destination) {
        case (.idle, .starting),
             (.starting, .active),
             (.starting, .idle),
             (.starting, .draining),
             (.active, .draining),
             (.draining, .idle),
             (.draining, .unconfirmed(reason: _)),
             (.unconfirmed(reason: _), .starting):
            self = destination
            return true
        default:
            return false
        }
    }
}
