import Foundation
import XCTest
@testable import OrbitNativeHost
import OrbitNativeProtocol

final class NativeCommandRouterTests: XCTestCase {
    private func command(_ id: String, _ name: String, fields: [String: Any] = [:]) throws -> NativeWireCommand {
        var value: [String: Any] = ["v": 1, "id": id, "command": name]
        for (key, field) in fields { value[key] = field }
        return try NativeWireCommand.decode(JSONSerialization.data(withJSONObject: value))
    }

    func testStalledCommandRejectsItsQueueWhilePlaybackUsesItsOwnLane() throws {
        let slow = try command("slow", "permissions", fields: ["request": ["calendars"]])
        let waiting = try command("waiting", "screen_sources")
        let playback = try command("playback", "playback", fields: [
            "playback_id": "answer", "sequence": 0, "sample_rate": 16_000,
            "channels": 1, "pcm_s16le_base64": "AAA=",
        ])
        var router = NativeCommandRouter()

        XCTAssertEqual(router.route(slow, at: 0), .start)
        XCTAssertEqual(router.route(waiting, at: 1), .queue)

        let rejections = router.watchdog(at: 121)
        XCTAssertEqual(rejections.map(\.commandID), ["waiting"])
        XCTAssertEqual(rejections.map(\.code), ["native_command_stalled"])
        XCTAssertEqual(router.route(playback, at: 121), .bypass)
    }
    func testMicrophonePurposeCannotWaitBehindAPermissionPrompt() throws {
        var router = NativeCommandRouter()
        let permission = try command("permission", "permissions", fields: ["request": ["calendars"]])
        let purpose = try command("purpose", "audio_purpose", fields: [
            "stream_id": "shared-stream", "purpose": "voice_note",
        ])
        XCTAssertEqual(router.route(permission, at: 0), .start)
        XCTAssertEqual(router.route(purpose, at: 1), .bypass)
    }

    func testMicrophoneMuteCannotWaitBehindAPermissionPrompt() throws {
        var router = NativeCommandRouter()
        let permission = try command("permission", "permissions", fields: ["request": ["calendars"]])
        let mute = try command("mute", "audio_mute", fields: [
            "stream_id": "meeting-stream", "muted": true,
        ])
        XCTAssertEqual(router.route(permission, at: 0), .start)
        XCTAssertEqual(router.route(mute, at: 1), .bypass)
    }

}
