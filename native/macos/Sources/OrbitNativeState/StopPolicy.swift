import Foundation

/// Which stops are Mars's own brake (REF-12).
///
/// Only these write the durable latch, so only they survive a restart of the
/// helper and of the daemon. Every other reason — the session going inactive,
/// an audio route change, the service not answering, a lost pipe, quitting or
/// the daemon's own closing — stops capture and playback and leaves the latch
/// exactly as it was.
///
/// Mirrored in Rust by `orbit_core::privacy_stop::LATCHING_HELPER_CODES`,
/// compared by tests against `contracts/fixtures/native-latching-stops.json`.
public enum StopPolicy {
    /// ⌃⌥Esc and the daemon passing on her order.
    public static let latchingReasons: Set<String> = ["local_hotkey", "daemon_requested"]

    /// The reason the helper gives when the daemon asks it to cut the capture
    /// without the latch (`capture_stop`).
    public static let daemonCutReason = "daemon_cut"

    public static func latches(reason: String) -> Bool {
        latchingReasons.contains(reason)
    }

    /// The event a stop is announced with. A stop that latches is a privacy
    /// stop; any other is a capture stop, and the two never share a word.
    public static func eventName(reason: String) -> String {
        latches(reason: reason) ? "privacy_stop" : "capture_stopped"
    }
}
