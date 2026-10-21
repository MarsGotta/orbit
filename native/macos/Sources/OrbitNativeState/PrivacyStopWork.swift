import Foundation

/// Whether a stop still has anything to stop.
///
/// Stopping what is already stopped looks harmless and is not. Each stop bumps
/// the privacy epoch, and the epoch is what a pending permission request checks
/// before handing macOS's answer back — so a redundant stop cancels a TCC dialog
/// the person is reading, and retires the calendar read in flight, for nothing. The
/// daemon asks for a stop on every failure it sees, and a stop that never
/// confirms its drain produces another failure, so the redundant case is not
/// rare: it is a loop.
///
/// A stop either writes the user's durable latch (`latches`) or only
/// cuts capture and playback; the second kind has nothing left to do once
/// nothing is live, whatever the latch says.
public struct PrivacyStopWork: Equatable, Sendable {
    public let latchAlreadyStopped: Bool
    public let microphoneSlot: CaptureSlot
    public let systemAudioSlot: CaptureSlot
    public let playbackLive: Bool
    public let cleanupInFlight: Int
    /// Whether this stop writes the durable latch (see `StopPolicy`).
    public let latches: Bool

    public init(latchAlreadyStopped: Bool, microphoneSlot: CaptureSlot, systemAudioSlot: CaptureSlot,
                playbackLive: Bool, cleanupInFlight: Int, latches: Bool = true) {
        self.latchAlreadyStopped = latchAlreadyStopped
        self.microphoneSlot = microphoneSlot
        self.systemAudioSlot = systemAudioSlot
        self.playbackLive = playbackLive
        self.cleanupInFlight = cleanupInFlight
        self.latches = latches
    }

    public var microphoneLive: Bool { microphoneSlot.isActive }
    public var systemAudioLive: Bool { systemAudioSlot.isActive }

    /// True only when nothing is running and, for a stop that latches, the
    /// latch is already durable.
    ///
    /// Deliberately conservative for live capture. An `unconfirmed` slot is an
    /// explicit post-stop outcome, so it does not make the hardware live again.
    public var isRedundant: Bool {
        (latchAlreadyStopped || !latches) && !microphoneLive && !systemAudioLive && !playbackLive
            && cleanupInFlight == 0
    }

    /// What a redundant stop reports as its drain result.
    ///
    /// A stop with nothing to drain is drained. Reporting `false` here is what
    /// made the daemon treat the answer as a failure and ask again.
    public var reportsDrained: Bool { isRedundant }

    /// Describes why a successful stop has no new drain work to perform.
    public var drain: String {
        if microphoneSlot.drainUnconfirmed || systemAudioSlot.drainUnconfirmed {
            return "unconfirmed_previously"
        }
        return "complete"
    }
}
