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

    /// Spanish, because this text is read by the person holding the Mac.
    ///
    /// Ordered by what a reader can act on: a dead channel needs a restart, a
    /// closing app explains itself, and the two transient ones are worth
    /// waiting out. Reporting only the first blocker keeps the dialog short.
    public var refusal: String? {
        if !connected {
            return "Orbit perdió la conexión con su servicio. Sal de Orbit "
                + "y vuelve a abrirlo."
        }
        if quitRequested {
            return "Orbit se está cerrando. Ábrelo de nuevo y reactívalo entonces."
        }
        if cleanupInFlight > 0 {
            return "Orbit está terminando de guardar el audio anterior. "
                + "Inténtalo de nuevo en unos segundos."
        }
        if commandInFlight {
            return "Orbit está atendiendo otra operación. "
                + "Inténtalo de nuevo en unos segundos."
        }
        if microphoneSlot.isActive || systemAudioSlot.isActive {
            return "Orbit todavía está iniciando o capturando audio. "
                + "Detén la captura antes de reactivarlo."
        }
        if microphoneSlot == .draining || systemAudioSlot == .draining {
            return "Orbit está terminando de guardar el audio anterior. "
                + "Inténtalo de nuevo en unos segundos."
        }
        return nil
    }
}
