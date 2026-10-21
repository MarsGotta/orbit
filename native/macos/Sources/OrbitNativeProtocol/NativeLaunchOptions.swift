/// Launch syntax and command admission only. Filesystem ownership, symlinks and
/// an empty private state directory must be checked by the host before startup.
///
/// The helper always speaks over its standard input and output. Since REF-06
/// it never hosts the daemon: production launches it as the daemon's managed
/// child, and a launch without arguments — the retired mode in which the helper
/// started the daemon itself — is refused like any other unknown syntax.
public struct NativeLaunchOptions: Equatable, Sendable {
    public enum Mode: Equatable, Sendable {
        case managedStdio
        case isolatedStdio(stateDirectory: String)
    }

    public enum ParseError: Error, Equatable, Sendable {
        case invalidArguments
        case invalidStateDirectory
    }

    public let mode: Mode

    private init(mode: Mode) {
        self.mode = mode
    }

    /// Arguments exclude argv[0]. Reject unsupported syntax before any effects;
    /// do not silently turn an invalid fixture request into a production launch.
    public static func parse(_ arguments: [String]) throws -> Self {
        if arguments == ["--stdio", "--managed"] {
            return Self(mode: .managedStdio)
        }
        guard arguments.count == 3,
              arguments[0] == "--stdio",
              arguments[1] == "--test-state-directory" else {
            throw ParseError.invalidArguments
        }
        let directory = arguments[2]
        guard isCanonicalAbsolutePath(directory) else {
            throw ParseError.invalidStateDirectory
        }
        return Self(mode: .isolatedStdio(stateDirectory: directory))
    }

    public var isManaged: Bool {
        mode == .managedStdio
    }

    public var isIsolatedFixture: Bool {
        if case .isolatedStdio = mode { return true }
        return false
    }

    public var stateDirectory: String? {
        if case .isolatedStdio(let directory) = mode { return directory }
        return nil
    }

    /// The `launch_mode` a host reports in `ready` and in its diagnostics.
    public var launchModeName: String {
        switch mode {
        case .managedStdio: return "managed_stdio"
        case .isolatedStdio: return "isolated_test_stdio"
        }
    }

    /// Non-fixture launches retain the protocol's existing authority checks.
    /// The isolated fixture can inspect state and stop, but cannot acquire data,
    /// request permissions, play audio, rearm privacy or invoke unknown commands.
    public func permits(command: String) -> Bool {
        guard isIsolatedFixture else { return true }
        switch command {
        case "hello", "heartbeat", "status", "privacy_stop", "capture_stop", "shutdown": return true
        default: return false
        }
    }

    public static func isCanonicalAbsolutePath(_ value: String) -> Bool {
        guard value.hasPrefix("/"), value.utf8.count <= 4096,
              !value.unicodeScalars.contains(where: {
                  $0.value < 0x20 || (0x7F...0x9F).contains($0.value)
              }) else { return false }
        let components = value.split(separator: "/", omittingEmptySubsequences: false)
        return components.count > 1 && components.dropFirst().allSatisfy {
            !$0.isEmpty && $0 != "." && $0 != ".."
        }
    }
}
