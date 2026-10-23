import Foundation

/// Liveness of the local channel to the daemon, decided by observed traffic.
///
/// This was a `connected` flag that the watchdog and the transport's disconnect
/// callback each set to false, and that nothing ever set back to true. The flag
/// gates rearming and every permission prompt, so one late heartbeat during
/// sleep or a busy second disabled both for the rest of the run, without a
/// message. Evidence decides liveness in both directions: silence takes the
/// channel down, and a later beat brings it back.
///
/// Silence is only counted while this process is running. Beats used to be
/// read on the main thread, the same thread the watchdog ticked on; when that
/// thread was held, the beats waited in its queue and the first tick after it
/// was released read them as silence (2026-10-22: a recording cut 25 minutes
/// in). `DaemonLivenessWatch` now observes them on the channel's reader queue
/// and ticks on a queue of its own, so a late tick only means this whole
/// process was suspended.
public struct DaemonChannelHealth: Equatable, Sendable {
    /// The daemon beats every two seconds; three missed beats is silence.
    public static let beatTimeout: TimeInterval = 6

    /// The watchdog ticks every second. A tick more than one beat period
    /// (2 s) after the previous one means this process was not running for
    /// at least a beat: a beat may be waiting unread behind the tick, so the
    /// silence starts again from that tick. It is the beat period and not
    /// `beatTimeout`: a held thread released after 5 s, with the last beat
    /// read 2 s before it, would otherwise expire before reading the beats
    /// queued meanwhile. It does not shorten or lengthen the silence the
    /// helper waits for while it is awake.
    public static let lateTick: TimeInterval = 2

    public private(set) var connected: Bool
    private var lastBeat: TimeInterval
    /// Unknown until the first tick: lateness needs a previous tick.
    private var lastTick: TimeInterval?
    /// The time between the last tick and the one before it when that tick
    /// was late, else nil: what the helper's log says about its own stall.
    public private(set) var lateTickGap: TimeInterval?

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

    /// True exactly on the transition into silence, so the caller acts once
    /// rather than on every tick of the watchdog. A late tick never expires:
    /// it restarts the silence from now, and the beats queued behind it are
    /// read before the next tick decides.
    public mutating func tick(now: TimeInterval) -> Bool {
        let previous = lastTick
        lastTick = now
        lateTickGap = nil
        if let previous, now - previous > Self.lateTick {
            lateTickGap = now - previous
            lastBeat = now
            return false
        }
        guard connected, now - lastBeat > Self.beatTimeout else { return false }
        connected = false
        return true
    }

    /// Whether the daemon falling silent stops the capture. A recording or a
    /// voice note in progress keeps capturing: the transport keeps on this Mac
    /// what the daemon does not read and delivers it in order when it reads
    /// again. Only the person (finish, ⌃⌥Esc), Orbit closing (end of input),
    /// the privacy stop and macOS sleep stop a capture. With nothing being
    /// captured, silence is handled as before: the stop has nothing to cut.
    public static func silenceStopsCapture(microphone: CaptureSlot, systemAudio: CaptureSlot) -> Bool {
        !microphone.isActive && !systemAudio.isActive
    }
}
