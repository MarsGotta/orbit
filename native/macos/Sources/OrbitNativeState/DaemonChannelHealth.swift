import Foundation

/// Liveness of the local channel to the daemon, decided by observed traffic.
///
/// This was a `connected` flag that the watchdog and the transport's disconnect
/// callback each set to false, and that nothing ever set back to true. The flag
/// gates rearming and every permission prompt, so one late heartbeat during
/// sleep or a busy second disabled both for the rest of the run, without a
/// message. Evidence decides liveness in both directions: silence takes the
/// channel down, and a later beat brings it back.
public struct DaemonChannelHealth: Equatable, Sendable {
    /// The daemon beats every two seconds; three missed beats is silence.
    public static let beatTimeout: TimeInterval = 6

    public private(set) var connected: Bool
    private var lastBeat: TimeInterval

    public init(now: TimeInterval) {
        connected = true
        lastBeat = now
    }

    /// A beat is proof the daemon is answering, and the only evidence that can
    /// restore the channel. This never rearms privacy: that stays a human act.
    public mutating func observedBeat(at now: TimeInterval) {
        lastBeat = now
        connected = true
    }

    /// The transport reported the channel gone. Only new traffic revives it.
    public mutating func disconnected() {
        connected = false
    }

    /// True exactly on the transition into silence, so the caller stops capture
    /// once rather than on every tick of the watchdog.
    public mutating func tick(now: TimeInterval) -> Bool {
        guard connected, now - lastBeat > Self.beatTimeout else { return false }
        connected = false
        return true
    }
}
