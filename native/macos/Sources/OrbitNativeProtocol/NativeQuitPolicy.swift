import Foundation

/// Instructions for the trusted caller, never process or I/O effects themselves.
public enum NativeQuitEffect: Equatable, Sendable {
    case none
    /// Stop capture, then terminate — and
    /// terminate anyway once `NativeQuitPolicy.orphanExitGrace` has passed.
    case stopThenTerminate
    case terminate
}

/// A quit request is not proof that its flush completed. The caller supplies
/// identities and observations; this policy owns no clock, task, pipe,
/// process, retry or mutable global state.
///
/// The helper never hosts the daemon: in production it is the
/// daemon's managed child (`--stdio --managed`), and a managed helper whose
/// daemon has closed the pipe — an orderly shutdown or a crash alike — has no
/// one left who can ever command it. It leaves, and with it go the global
/// shortcuts a new helper needs to register. The isolated fixture also exits
/// when its pipe closes.
public struct NativeQuitPolicy: Equatable, Sendable {
    /// How long a managed helper whose daemon is gone gives its capture
    /// stop — the audio drain — before it terminates
    /// anyway.
    ///
    /// What it protects: the shortcuts and the next helper. A stop that never
    /// completes must not keep a process nobody can reach alive with the
    /// privacy shortcut registered; that is how the next helper got
    /// `hotkey_registration_failed`.
    ///
    /// Why this size: an idle helper's stop returns in milliseconds; two seconds
    /// also covers stopping a live
    /// capture. It must stay under the daemon's `HELPER_EXIT_GRACE` (3 s,
    /// `crates/orbit-daemon/src/native.rs`), so that in an orderly shutdown the
    /// helper leaves by itself before the daemon kills it. The two are kept
    /// in step by hand; `NativeQuitPolicyTests` checks this one is under 3 s.
    ///
    /// What it is not: a wait for the daemon. The daemon is gone when this runs.
    public static let orphanExitGrace: TimeInterval = 2

    private enum Phase: Equatable, Sendable {
        case idle
        case flushing(UUID)
        case failed
        case leaving
        case finished
    }

    private var phase: Phase = .idle

    public init() {}

    /// A terminal channel failure stays latched, including before an explicit
    /// quit. Neither a late receipt nor another click can rearm this instance.
    public var isRequested: Bool { phase != .idle }

    public mutating func beginFlush(id: UUID) -> Bool {
        guard phase == .idle else { return false }
        phase = .flushing(id)
        return true
    }

    /// Only the flush this policy admitted can end it. A successful one
    /// terminates: the helper has no child and nothing else to wait for.
    public mutating func completeFlush(id: UUID, succeeded: Bool) -> NativeQuitEffect {
        guard case .flushing(let current) = phase, current == id else { return .none }
        guard succeeded else {
            phase = .failed
            return .none
        }
        phase = .finished
        return .terminate
    }

    /// The channel to the daemon is gone. The helper leaves, once, in
    /// every phase but the end.
    public mutating func disconnected(reason: String) -> NativeQuitEffect {
        switch phase {
        case .leaving, .finished:
            return .none
        case .idle, .flushing, .failed:
            phase = .leaving
            return .stopThenTerminate
        }
    }
}
