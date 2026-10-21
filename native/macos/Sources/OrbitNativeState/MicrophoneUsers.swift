import Foundation

/// One audio process as CoreAudio reports it.
public struct MicrophoneProcess: Equatable, Sendable {
    public let pid: Int32
    public let runningInput: Bool

    public init(pid: Int32, runningInput: Bool) {
        self.pid = pid
        self.runningInput = runningInput
    }
}

/// The application a process belongs to, as the person sees it in the Dock.
public struct ResponsibleApplication: Equatable, Sendable {
    public let pid: Int32
    public let bundleID: String
    public let name: String

    public init(pid: Int32, bundleID: String, name: String) {
        self.pid = pid
        self.bundleID = bundleID
        self.name = name
    }
}

/// Which applications are using the microphone right now.
///
/// A call is any application holding the microphone: Zoom, Teams, Slack,
/// FaceTime, or a browser tab with Meet. Nothing here knows which apps are
/// meetings; that decision, and when to offer or stop a recording, belongs to
/// orbit-core. This side answers one question and answers it the same way
/// every time, so the daemon can compare two answers.
public enum MicrophoneUsers {
    /// How many applications one event carries. A person is in one call, maybe
    /// two; sixteen leaves room for dictation tools and voice assistants
    /// without letting a misbehaving system put its process table on the wire.
    public static let maximum = MeetingApplications.maximum

    /// - `resolve` maps a process to its application: a browser's audio runs
    ///   in a helper process, and the person knows the browser, not the helper.
    ///   Processes without a visible application are left out.
    /// - Orbit itself is left out: while it records, it holds the microphone
    ///   too, and it must never detect its own recording as a call.
    /// - The result is sorted by process id, so the same set always compares
    ///   equal and only a real change produces an event.
    public static func reportable(
        _ processes: [MicrophoneProcess],
        ownPID: Int32,
        ownBundlePrefix: String,
        resolve: (Int32) -> ResponsibleApplication?
    ) -> [MeetingApplication] {
        var seen = Set<Int32>()
        var result = [MeetingApplication]()
        for process in processes where process.runningInput && process.pid > 0 {
            guard let application = resolve(process.pid),
                  application.pid != ownPID,
                  !application.bundleID.hasPrefix(ownBundlePrefix),
                  !application.bundleID.isEmpty, !application.name.isEmpty,
                  seen.insert(application.pid).inserted else { continue }
            result.append(MeetingApplication(
                applicationPID: application.pid,
                bundleID: application.bundleID,
                name: String(application.name.prefix(128))))
        }
        return Array(result.sorted { $0.applicationPID < $1.applicationPID }.prefix(maximum))
    }
}
