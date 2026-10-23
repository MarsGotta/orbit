import Foundation
import Synchronization

/// What the watch knew of the capture when the daemon fell silent.
public struct DaemonSilence: Equatable, Sendable {
    public let microphone: Bool
    public let systemAudio: Bool

    /// A recording or a voice note was capturing: it goes on, and the
    /// transport keeps what the daemon does not read.
    public var capturing: Bool { microphone || systemAudio }
}

/// The daemon's liveness, observed and judged off the main thread.
///
/// The daemon's beats used to be read on the main thread, the same thread
/// the watchdog ticked on, the calendar ran on and every command was served
/// on: a held main thread was the daemon's silence, and on 2026-10-22 a
/// recording was cut 25 minutes in. Here a beat is observed where it is read,
/// on the channel's reader queue, before the main thread runs anything of
/// it, and the silence is decided on a queue of its own once a second. The
/// main thread is left for AppKit and the commands: a capture in progress
/// never waits on it to go on.
public final class DaemonLivenessWatch: @unchecked Sendable {
    private struct Watched {
        var health: DaemonChannelHealth
        var microphone = false
        var systemAudio = false
    }

    /// How often the silence is judged: `DaemonChannelHealth`'s watchdog
    /// period, which its late-tick rule is measured against.
    public static let tickInterval: TimeInterval = 1

    private let queue = DispatchQueue(label: "orbit.native.daemon-liveness", qos: .userInitiated)
    private let state: Mutex<Watched>
    private let clock: @Sendable () -> TimeInterval
    private let fellSilent: (DaemonSilence) -> Void
    private let lateTick: (TimeInterval) -> Void
    /// Only touched on `queue`.
    private var timer: DispatchSourceTimer?

    /// `fellSilent` and `lateTick` run on the watch's own queue, never on
    /// the main thread.
    public init(clock: @escaping @Sendable () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
                fellSilent: @escaping (DaemonSilence) -> Void,
                lateTick: @escaping (TimeInterval) -> Void) {
        self.clock = clock
        self.state = Mutex(Watched(health: DaemonChannelHealth(now: clock())))
        self.fellSilent = fellSilent
        self.lateTick = lateTick
    }

    deinit { timer?.cancel() }

    /// Whether the daemon is answering, readable from any thread.
    public var connected: Bool { state.withLock { $0.health.connected } }

    public func start() {
        queue.async { [self] in
            guard timer == nil else { return }
            let source = DispatchSource.makeTimerSource(queue: queue)
            let interval = DispatchTimeInterval.milliseconds(Int(Self.tickInterval * 1000))
            source.schedule(deadline: .now() + interval, repeating: interval, leeway: .milliseconds(50))
            source.setEventHandler { [weak self] in self?.tick() }
            timer = source
            source.resume()
        }
    }

    public func stop() {
        queue.async { [self] in
            timer?.cancel()
            timer = nil
        }
    }

    /// The daemon answered: any command read from the pipe, beats included.
    /// Called on the reader's queue, before the command reaches main.
    public func observedBeat() {
        let now = clock()
        state.withLock { $0.health.observedBeat(at: now) }
    }

    /// The transport reported the channel gone. Only new traffic revives it.
    public func disconnected() {
        state.withLock { $0.health.disconnected() }
    }

    /// What is being captured, as the main thread last published it.
    public func captureChanged(microphone: Bool, systemAudio: Bool) {
        state.withLock {
            $0.microphone = microphone
            $0.systemAudio = systemAudio
        }
    }

    /// One judgement of the silence. The timer calls it on the watch's
    /// queue; a test calls it with its own clock.
    func tick() {
        let now = clock()
        let (silence, gap) = state.withLock { watched -> (DaemonSilence?, TimeInterval?) in
            let expired = watched.health.tick(now: now)
            let silence = expired
                ? DaemonSilence(microphone: watched.microphone, systemAudio: watched.systemAudio)
                : nil
            return (silence, watched.health.lateTickGap)
        }
        if let gap { lateTick(gap) }
        if let silence { fellSilent(silence) }
    }
}
