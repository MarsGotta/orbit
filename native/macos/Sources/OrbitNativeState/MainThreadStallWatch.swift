import Foundation

/// A hold of the helper's main thread, as `helper_main_thread_stalled` logs it.
///
/// Codes and numbers only: how long, whether a capture was live and the kind
/// of the last daemon command the main thread started, never its content.
public struct MainThreadStall: Equatable, Sendable {
    /// How long the main thread had not run a block posted to it.
    public let stalledMs: Int
    /// True while the hold still lasts: said once, so a helper that never
    /// comes back still leaves a line. False when the hold has ended.
    public let ongoing: Bool
    public let recording: Bool
    public let lastCommand: String

    public var fields: [String: String] {
        ["stalled_ms": String(stalledMs), "ongoing": ongoing ? "true" : "false",
         "recording": recording ? "true" : "false", "last_command": lastCommand]
    }
}

/// The pure part of the main-thread hang detector: it decides from times it
/// is given, never reads a clock, and never touches the main thread.
///
/// A background timer posts a ping to the main queue whenever none is
/// outstanding; the main thread answers by running it. A ping that waited
/// longer than `threshold` is a hold of the main thread, measured from the
/// moment it was posted. This follows Zed's `hang_telemetry` (a background
/// watcher of the UI thread) and complements `helper_watchdog_late`, which
/// only sees holds longer than the watchdog's 2 s late-tick rule and says
/// nothing about what the thread was doing.
public struct MainThreadStallWatch: Equatable, Sendable {
    /// How long the main thread may leave a ping unanswered before the helper
    /// logs `helper_main_thread_stalled`. What it protects: a hold of the
    /// thread that reads the daemon's beats, starts and stops the audio and
    /// runs every command is seen in `native.log` with its length and the
    /// command it had started. Why 250 ms: it is the length from which Apple
    /// counts a main-thread hang (Xcode Organizer's hang rate, Instruments'
    /// hang detection); shorter holds are an ordinary busy run loop, and each
    /// line costs a disk flush. It stops, delays and refuses nothing.
    public static let threshold: TimeInterval = 0.25

    /// How often the background timer looks. A hold that begins just after
    /// a ping was answered is measured from the next ping, so it can read up
    /// to this much shorter than it was; 100 ms keeps that error under half
    /// of `threshold`. It wakes a utility queue ten times a second and posts
    /// one empty block to the main queue each time; nothing else.
    public static let pingInterval: TimeInterval = 0.1

    /// How long a hold may last before it is said while it still lasts. It
    /// is `DaemonChannelHealth.lateTick`, the daemon's beat period: past it,
    /// beats are waiting unread behind the held thread. A helper killed
    /// during a longer hold leaves this line; one that comes back also logs
    /// the hold's whole length when it ends.
    public static let ongoingMark: TimeInterval = DaemonChannelHealth.lateTick

    /// Longest command kind kept; every wire command name fits.
    static let maximumCommandBytes = 40

    /// When the outstanding ping was posted; nil when none is outstanding.
    private var pingPostedAt: TimeInterval?
    private var ongoingSaid = false
    public private(set) var recording = false
    public private(set) var lastCommand = "none"

    public init() {}

    /// The background timer fired at `now`. Returns whether to post a ping
    /// (none is outstanding) and the ongoing hold to say, at most once each.
    public mutating func timerFired(at now: TimeInterval) -> (postPing: Bool, ongoing: MainThreadStall?) {
        guard let posted = pingPostedAt else {
            pingPostedAt = now
            return (true, nil)
        }
        guard !ongoingSaid, now - posted > Self.ongoingMark else { return (false, nil) }
        ongoingSaid = true
        return (false, stall(since: posted, now: now, ongoing: true))
    }

    /// The main thread ran the outstanding ping at `now`. Returns the hold it
    /// ended, if the ping waited longer than `threshold`.
    public mutating func pingAnswered(at now: TimeInterval) -> MainThreadStall? {
        guard let posted = pingPostedAt else { return nil }
        pingPostedAt = nil
        ongoingSaid = false
        guard now - posted > Self.threshold else { return nil }
        return stall(since: posted, now: now, ongoing: false)
    }

    /// The main thread started a daemon command of this kind.
    public mutating func commandStarted(_ kind: String) {
        lastCommand = Self.code(kind)
    }

    /// Whether a microphone or system-audio capture is live.
    public mutating func captureChanged(_ capturing: Bool) {
        recording = capturing
    }

    /// A wire name kept only when it looks like one, so no text reaches the log.
    static func code(_ kind: String) -> String {
        let letters: ClosedRange<UInt8> = UInt8(ascii: "a")...UInt8(ascii: "z")
        let digits: ClosedRange<UInt8> = UInt8(ascii: "0")...UInt8(ascii: "9")
        let marks: Set<UInt8> = [UInt8(ascii: "_"), UInt8(ascii: ".")]
        let bytes = Array(kind.utf8)
        let allowed = bytes.allSatisfy { byte in
            letters.contains(byte) || digits.contains(byte) || marks.contains(byte)
        }
        let valid = !bytes.isEmpty && bytes.count <= maximumCommandBytes && allowed
        return valid ? kind : "unrecognized"
    }

    private func stall(since posted: TimeInterval, now: TimeInterval, ongoing: Bool) -> MainThreadStall {
        MainThreadStall(stalledMs: Int(((now - posted) * 1000).rounded()), ongoing: ongoing,
                        recording: recording, lastCommand: lastCommand)
    }
}

/// Watches the main queue from a background queue of its own.
///
/// The main thread only runs the posted ping, which reads the clock and
/// hands the time back; every decision and every log line happen on the
/// watcher's queue (and the log's own worker). It measures; it changes
/// nothing the main thread does.
public final class MainThreadHangDetector: @unchecked Sendable {
    private let queue = DispatchQueue(label: "orbit.native.main-thread-watch", qos: .utility)
    private let target: DispatchQueue
    private let clock: @Sendable () -> TimeInterval
    private let report: (MainThreadStall) -> Void
    /// Only touched on `queue`.
    private var watch = MainThreadStallWatch()
    private var timer: DispatchSourceTimer?

    /// `target` is the queue watched: the main queue, or a stand-in in tests.
    /// `report` runs on the watcher's queue.
    public init(target: DispatchQueue = .main,
                clock: @escaping @Sendable () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
                report: @escaping (MainThreadStall) -> Void) {
        self.target = target
        self.clock = clock
        self.report = report
    }

    deinit { timer?.cancel() }

    public func start() {
        queue.async { [self] in
            guard timer == nil else { return }
            let source = DispatchSource.makeTimerSource(queue: queue)
            let interval = DispatchTimeInterval.milliseconds(Int(MainThreadStallWatch.pingInterval * 1000))
            source.schedule(deadline: .now() + interval, repeating: interval, leeway: .milliseconds(25))
            source.setEventHandler { [weak self] in self?.fire() }
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

    /// Called on the main thread as a command starts: one async hop, nothing more.
    public func commandStarted(_ kind: String) {
        queue.async { [self] in watch.commandStarted(kind) }
    }

    public func captureChanged(_ capturing: Bool) {
        queue.async { [self] in watch.captureChanged(capturing) }
    }

    private func fire() {
        let decision = watch.timerFired(at: clock())
        if let ongoing = decision.ongoing { report(ongoing) }
        guard decision.postPing else { return }
        let clock = self.clock
        target.async { [weak self] in
            let answered = clock()
            self?.queue.async { [weak self] in self?.answered(at: answered) }
        }
    }

    private func answered(at now: TimeInterval) {
        if let stall = watch.pingAnswered(at: now) { report(stall) }
    }
}
