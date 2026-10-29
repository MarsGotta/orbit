import Carbon
import XCTest
@testable import OrbitNativeHost
import OrbitNativeProtocol

final class GlobalHotkeysTests: XCTestCase {
    @MainActor
    func testAConflictDoesNotPreventTheRemainingShortcutsFromRegistering() {
        var registered = Set<UInt32>()
        let hotkeys = GlobalHotkeys(registerKey: { id, _ in
            if id == 2 { return OSStatus(eventHotKeyExistsErr) }
            registered.insert(id)
            return noErr
        })
        let failures = hotkeys.register()
        XCTAssertEqual(failures, [NativeShortcutFailure(combination: "⌃⌥D",
            osStatus: OSStatus(eventHotKeyExistsErr))])
        XCTAssertEqual(registered, [1, 3, 4], "privacy stop survives a call-shortcut conflict")
        hotkeys.unregister()
    }

    func testFailedMicrophoneObservationCarriesItsOSStatusWithoutAnEmptyList() throws {
        let data = try JSONEncoder().encode(NativeEvent.makeMicrophoneActivityFailure(osStatus: -2000))
        let event = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(event["event"] as? String, "microphone_activity")
        XCTAssertEqual(event["os_status"] as? Int, -2000)
        XCTAssertNil(event["applications"])
    }
}
