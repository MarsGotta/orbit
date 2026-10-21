import AppKit
import OrbitNativeTransport
import OrbitNativeProtocol
import OrbitNativeState
import Foundation
import Darwin

// Validate launch intent and fixture isolation before constructing AppKit,
// registering shortcuts or selecting the user's state root. The helper always
// speaks over its standard input and output: in production it is the daemon's
// managed child (`--stdio --managed`), and it never starts a daemon itself
// (that mode, its login item and its instance lock were retired).
let launchOptions: NativeLaunchOptions
let latch: PrivacyLatch
do {
    launchOptions = try NativeLaunchOptions.parse(Array(CommandLine.arguments.dropFirst()))
} catch {
    fputs("Orbit: invalid launch arguments.\n", stderr)
    exit(2)
}
let diagnostics: DiagnosticLog
if let path = launchOptions.stateDirectory {
    let root: URL
    do { root = try NativeFixtureDirectory.validate(path) }
    catch {
        fputs("Orbit: invalid test state directory.\n", stderr)
        exit(3)
    }
    // The isolated protocol reports through stdio and must leave only its latch.
    // Discard lifecycle diagnostics instead of creating fixture or profile logs.
    diagnostics = DiagnosticLog(directory: URL(fileURLWithPath: "/dev", isDirectory: true),
                                fileName: "null")
    latch = PrivacyLatch(root: root)
    do { try latch.stop() } // Durably stopped before any command arrives.
    catch {
        diagnostics.record("fixture_latch_unconfirmed")
        fputs("Orbit: the test latch was not confirmed.\n", stderr)
        exit(4)
    }
} else {
    diagnostics = DiagnosticLog()
    latch = PrivacyLatch()
}
diagnostics.record("host_starting", [
    "mode": launchOptions.launchModeName,
    "privacy_stopped": latch.stopped ? "true" : "false"
])

signal(SIGPIPE, SIG_IGN)
let application = NSApplication.shared
application.setActivationPolicy(.accessory)

let channel = NativeChannel(input: .standardInput, output: .standardOutput)
let delegate = OrbitApplication(channel: channel, latch: latch,
                                launchOptions: launchOptions, diagnostics: diagnostics)
application.delegate = delegate
application.run()
