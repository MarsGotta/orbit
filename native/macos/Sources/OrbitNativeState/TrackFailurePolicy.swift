import Foundation

/// One of the two tracks a meeting records.
public enum CaptureTrack: String, Equatable, Sendable {
    case microphone = "mic_local"
    case systemAudio = "remote_app"
}

public enum TrackFailureAction: Equatable, Sendable {
    /// Nothing else is being captured, or a stop is already under way: the
    /// whole capture stops, as every track failure used to.
    case stopCapture
    /// The system's audio stream is started again under the same stream, once.
    case restartSystemAudio
    /// Only this track ends, declared with a gap; the other keeps recording.
    case endTrack
}

/// What the failure of one track does to the capture. It used to stop both:
/// a call app's stream that stopped took the person's own voice with it, and
/// a microphone that failed took the other side of the call (2026-10-22).
public enum TrackFailurePolicy {
    /// How long one restart of the system's audio may take before the track
    /// is declared stopped. A start lists the shareable content and starts a
    /// stream, well under a second when it works; ten seconds bounds an SDK
    /// call that does not return, so the gap is said instead of waited on.
    /// It limits only that one attempt, never the recording.
    public static let systemAudioRestartBound: TimeInterval = 10

    public static func decide(track: CaptureTrack, otherTrack: CaptureSlot, closing: Bool,
                              restartAttempted: Bool) -> TrackFailureAction {
        guard !closing, otherTrack.isActive else { return .stopCapture }
        if track == .systemAudio && !restartAttempted { return .restartSystemAudio }
        return .endTrack
    }
}

/// Keeps macOS from napping this process while a capture is in progress
/// (`.userInitiatedAllowingIdleSystemSleep`: the Mac may still sleep, and
/// sleep stops the capture as before). App Nap delays timers and lowers the
/// priority of a process nobody is looking at, and a meeting is recorded with
/// the window closed. `begin` and `end` are the system's calls, given by the
/// host so the transitions can be tested.
public final class CaptureActivity {
    private let begin: () -> NSObjectProtocol
    private let end: (NSObjectProtocol) -> Void
    private var token: NSObjectProtocol?

    public init(begin: @escaping () -> NSObjectProtocol, end: @escaping (NSObjectProtocol) -> Void) {
        self.begin = begin
        self.end = end
    }

    public var held: Bool { token != nil }

    /// Holds the activity while `capturing`, releases it when not. True when
    /// this call changed it, so the caller logs each transition once.
    @discardableResult
    public func update(capturing: Bool) -> Bool {
        if capturing, token == nil {
            token = begin()
            return true
        }
        if !capturing, let held = token {
            token = nil
            end(held)
            return true
        }
        return false
    }
}
