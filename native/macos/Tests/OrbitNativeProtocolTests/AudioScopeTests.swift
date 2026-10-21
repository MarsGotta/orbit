import Foundation
import XCTest
@testable import OrbitNativeHost
import OrbitNativeProtocol

/// The pure scope decision behind `audio_start` for `remote_app`.
///
/// The executable target that owns the router (`OrbitNative`) cannot be
/// imported by this test bundle, so the rule the router applies — which
/// ScreenCaptureKit filter shape each `capture_scope` needs, and when a
/// positive PID is still required — must live in a pure OrbitNativeHost
/// function, where this bundle can test it.
final class AudioScopeTests: XCTestCase {
    private func audioStart(_ fields: [String: Any]) throws -> NativeWireCommand {
        var value: [String: Any] = ["v": 1, "id": "scope-test", "command": "audio_start",
                                    "source": "remote_app", "stream_id": "remote-1"]
        for (key, field) in fields { value[key] = field }
        return try NativeWireCommand.decode(JSONSerialization.data(withJSONObject: value))
    }

    func testSystemScopeExcludesOrbitAndApplicationScopeIncludesOnlyThatApp() throws {
        let system = try SystemAudioCapture.scopeFilter(for: audioStart(["capture_scope": "system"]))
        XCTAssertEqual(system, .excludeApplications(["app.orbit.companion"]))

        let application = try SystemAudioCapture.scopeFilter(
            for: audioStart(["capture_scope": "application", "application_pid": 42]))
        XCTAssertEqual(application, .includeApplication(processID: 42))
    }

    func testSystemScopeNeedsNoPidWhileApplicationStillRequiresAPositiveOne() throws {
        let accepted = try SystemAudioCapture.scopeFilter(for: audioStart(["capture_scope": "system"]))
        XCTAssertEqual(accepted, .excludeApplications(["app.orbit.companion"]))

        for invalid in [["capture_scope": "application"],
                        ["capture_scope": "application", "application_pid": 0],
                        ["capture_scope": "application", "application_pid": -1]] {
            XCTAssertThrowsError(
                try SystemAudioCapture.scopeFilter(for: audioStart(invalid)),
                "application without a positive PID must still be refused: \(invalid)"
            ) { error in
                XCTAssertEqual(error as? AudioScopeRejection, .applicationPIDRequired)
            }
        }
    }
}
