import Foundation

/// Synchronization adapter between the main-thread admission policy and a
/// blocking read worker. Revocation cannot cancel a framework call in progress.
public final class WindowReadRevocation: @unchecked Sendable {
    private let lock = NSLock()
    private var cancelled = false

    public init() {}
    public func revoke() { lock.lock(); cancelled = true; lock.unlock() }
    public var revoked: Bool { lock.lock(); defer { lock.unlock() }; return cancelled }

    /// Refresh and privacy stop must revoke getters AND delivery together. The
    /// policy retains its physical slot until the original operation returns.
    public static func invalidate(_ policy: inout WindowSelectionPolicy, worker: WindowReadRevocation?) {
        worker?.revoke()
        policy.invalidate()
    }
}
