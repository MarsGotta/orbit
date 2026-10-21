import Foundation

/// The applications a meeting hotkey should tell the daemon about.
///
/// Pressing the meeting hotkey used to say only that it had been pressed, so a
/// meeting started that way recorded the microphone and nothing else. Which of
/// these to record is decided in orbit-core, deliberately: keeping the list of
/// conferencing apps in one language avoids the two copies drifting apart.
///
/// This side only answers what is running and visible.
public struct MeetingApplication: Equatable, Sendable {
    public let applicationPID: Int32
    public let bundleID: String
    public let name: String

    public init(applicationPID: Int32, bundleID: String, name: String) {
        self.applicationPID = applicationPID
        self.bundleID = bundleID
        self.name = name
    }

    public var fields: [String: Any] {
        ["application_pid": applicationPID, "bundle_id": bundleID, "name": name]
    }
}

public enum MeetingApplications {
    /// How many applications travel with a hotkey press.
    ///
    /// Bounded because this rides on a keystroke: an unbounded list would put
    /// the whole running-process table on the wire every time the user starts a
    /// meeting.
    public static let maximum = 16

    /// Filters and bounds what the hotkey reports.
    ///
    /// - `regular` keeps background daemons and menu-bar helpers out: an app the
    ///   person cannot see is not the one holding their call.
    /// - Terminated and unnamed entries are dropped rather than sent as blanks,
    ///   which would read downstream as a real candidate with no name.
    public static func reportable(
        _ running: [(pid: Int32, bundleID: String?, name: String?, regular: Bool, terminated: Bool)]
    ) -> [MeetingApplication] {
        var seen = Set<Int32>()
        var result = [MeetingApplication]()
        for application in running {
            guard application.regular, !application.terminated, application.pid > 0,
                  let bundleID = application.bundleID, !bundleID.isEmpty,
                  let name = application.name, !name.isEmpty,
                  seen.insert(application.pid).inserted else { continue }
            result.append(
                MeetingApplication(
                    applicationPID: application.pid,
                    bundleID: bundleID,
                    name: String(name.prefix(128))))
            if result.count == maximum { break }
        }
        return result
    }
}
