import Foundation

/// Whether Orbit can be rearmed on this Mac right now, and why not when it cannot.
///
/// The menu item was six conditions joined by a `guard ... else { return }`. A
/// person clicking the only rearm control Orbit offers got no dialog, no error
/// and no change, which reads as a broken app rather than as a refusal. Naming
/// the blocking condition is the whole point: every one of them is temporary,
/// and knowing which one it is tells the person what to do next.
public struct RearmReadiness: Equatable, Sendable {
    public let connected: Bool
    public let quitRequested: Bool
    public let commandInFlight: Bool
    public let cleanupInFlight: Int
    public let microphoneSlot: CaptureSlot
    public let systemAudioSlot: CaptureSlot

    public init(connected: Bool, quitRequested: Bool, commandInFlight: Bool,
                cleanupInFlight: Int, microphoneSlot: CaptureSlot, systemAudioSlot: CaptureSlot) {
        self.connected = connected
        self.quitRequested = quitRequested
        self.commandInFlight = commandInFlight
        self.cleanupInFlight = cleanupInFlight
        self.microphoneSlot = microphoneSlot
        self.systemAudioSlot = systemAudioSlot
    }

    public var allowed: Bool { refusal == nil }

    /// Internal refusal text; presentation renders the refusal code.
    ///
    /// Ordered by what a reader can act on: a dead channel needs a restart, a
    /// closing app explains itself, and the two transient ones are worth
    /// waiting out. Reporting only the first blocker keeps the dialog short.
    public var refusal: String? {
        if !connected {
            return "Orbit lost its service connection. Quit Orbit "
                + "and open it again."
        }
        if quitRequested {
            return "Orbit is closing. Open it again and then rearm it."
        }
        if cleanupInFlight > 0 {
            return "Orbit is finishing saving previous audio. "
                + "Try again in a few seconds."
        }
        if commandInFlight {
            return "Orbit is handling another operation. "
                + "Try again in a few seconds."
        }
        if microphoneSlot.isActive || systemAudioSlot.isActive {
            return "Orbit is still starting or capturing audio. "
                + "Stop capture before rearming it."
        }
        if microphoneSlot == .draining || systemAudioSlot == .draining {
            return "Orbit is finishing saving previous audio. "
                + "Try again in a few seconds."
        }
        return nil
    }
}
