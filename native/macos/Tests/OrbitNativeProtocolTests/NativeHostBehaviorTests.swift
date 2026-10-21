import ApplicationServices
import CoreMedia
import Foundation
import XCTest
@testable import OrbitNativeHost
import OrbitNativeProtocol
import OrbitNativeState
import OrbitNativeTransport

final class NativeHostBehaviorTests: XCTestCase {
    private func command(_ name: String, fields: [String: Any] = [:]) throws -> NativeWireCommand {
        var value: [String: Any] = ["v": 1, "id": "host-test", "command": name]
        for (key, field) in fields { value[key] = field }
        return try NativeWireCommand.decode(JSONSerialization.data(withJSONObject: value))
    }

    private func channel() -> (NativeChannel, Pipe, Pipe) {
        let input = Pipe()
        let output = Pipe()
        return (NativeChannel(input: input.fileHandleForReading,
                              output: output.fileHandleForWriting), input, output)
    }

    func testApplicationCodeIdentityRefusesAnInvalidProcessBeforeSecurityLookup() async {
        let metadata = ApplicationProcessMetadata(
            pid: 0, launchDateBits: 0, bundleID: "test.fixture", bundlePath: "/")

        let identity = await ApplicationCodeIdentity.read(metadata)

        XCTAssertNil(identity)
    }

    func testAudioPlaybackRejectsMalformedAudioBeforeCreatingAnAudioGraph() throws {
        let (channel, input, output) = channel()
        _ = (input, output)
        let playback = AudioPlayback(session: SharedAudioSession(), channel: channel)
        guard case .playback(let malformed) = try command("playback", fields: [
            "playback_id": "fixture", "sequence": 0, "sample_rate": 32_000,
            "channels": 1, "pcm_s16le_base64": "AQ=="
        ]) else { return XCTFail("playback variant") }

        XCTAssertThrowsError(try playback.enqueue(malformed.fields)) { error in
            XCTAssertEqual(error as? ProtocolError, .invalidAudio)
        }
    }

    func testSelectedTextCandidateEqualityPinsEveryIdentityField() {
        let candidate = TextWindowCandidate(windowID: 7, title: "Edición", descriptorHash: "abc")

        XCTAssertEqual(candidate, candidate)
        XCTAssertNotEqual(candidate,
                          TextWindowCandidate(windowID: 8, title: "Edición", descriptorHash: "abc"))
        XCTAssertNotEqual(candidate,
                          TextWindowCandidate(windowID: 7, title: "Edicion", descriptorHash: "abc"))
        XCTAssertNotEqual(candidate,
                          TextWindowCandidate(windowID: 7, title: "Edición", descriptorHash: "def"))
    }

    func testWindowReadChecksWriteOnlyTheirNamedResultAndNumericIdentity() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let log = DiagnosticLog(directory: directory)
        XCTAssertFalse(recordWindowReadCheck(false, "text_screen_permission", diagnostics: log,
                                             fields: ["window_id": "42", "pid": "99"]))
        XCTAssertTrue(recordWindowReadCheck(true, "screenshot_descriptor", diagnostics: log))
        log.recordAndWaitOffMain("test_flush")
        let lines = try String(contentsOf: log.url, encoding: .utf8).split(separator: "\n")
        let row = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(try XCTUnwrap(lines.first).utf8)) as? [String: Any])
        let fields = try XCTUnwrap(row["fields"] as? [String: String])
        XCTAssertEqual(row["event"] as? String, "window_read_check")
        XCTAssertEqual(fields, ["check": "text_screen_permission", "passed": "false", "window_id": "42", "pid": "99"])
        let success = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(try XCTUnwrap(lines.dropFirst().first).utf8)) as? [String: Any])
        XCTAssertEqual(success["fields"] as? [String: String], ["check": "screenshot_descriptor", "passed": "true"])
        XCTAssertEqual(lines.count, 3)
    }

    func testAXTextAccessPreservesUTF16AndEnforcesItsDeclaredLimit() throws {
        let value = "Órbita" as CFString

        XCTAssertEqual(try AXTextAccess.text(value, maximumUnits: 6, maximumBytes: 16), "Órbita")
        XCTAssertThrowsError(try AXTextAccess.text(value, maximumUnits: 5, maximumBytes: 16)) {
            XCTAssertEqual($0 as? SelectedTextFailure, .limit)
        }
    }

    func testCalendarBridgeMapsOnlyTheCommandSpecificOperation() throws {
        XCTAssertEqual(try CalendarBridge.operation(command("calendar_sources")), .sources(afterIndex: 0))
        XCTAssertEqual(try CalendarBridge.operation(command("calendar_sources",
            fields: ["calendar_page": ["after_index": 300]])), .sources(afterIndex: 300))
        XCTAssertEqual(try CalendarBridge.operation(command("calendar_snapshot",
            fields: ["calendar_query": ["start_unix_ms": 0, "end_unix_ms": 1_000, "after_index": 2]])),
            .read(CalendarReadQuery(start: 0, end: 1_000, afterIndex: 2)))
        // REF-10: there is nothing to select any more; a daemon that asks is told so.
        XCTAssertThrowsError(try command("calendar_select"))
        for name in ["status"] {
            XCTAssertThrowsError(try CalendarBridge.operation(command(name))) {
                XCTAssertEqual($0 as? CalendarFailure, .invalidRequest)
            }
        }
    }

    /// P9-21: each write maps to its operation with exactly its own field.
    func testCalendarBridgeMapsTheWritesWithTheirOwnFieldOnly() throws {
        let delete: [String: Any] = ["calendar_item_id": "item-1", "occurrence_start_ms": 1_000, "span": "this"]
        XCTAssertEqual(try CalendarBridge.operation(command("calendar_delete", fields: ["calendar_delete": delete])),
            .delete(CalendarDeleteRequest(calendarItemID: "item-1", occurrenceStart: 1_000, span: .thisEvent)))
        let draftFields: [String: Any] = ["title": "Café", "start_ms": 0, "end_ms": 1_000, "all_day": false]
        let draft = try JSONDecoder().decode(CalendarEventDraft.self,
            from: JSONSerialization.data(withJSONObject: draftFields))
        let create: [String: Any] = ["create": draftFields]
        XCTAssertEqual(try CalendarBridge.operation(command("calendar_save", fields: ["calendar_save": create])),
            .save(CalendarSaveRequest(create: draft)))
        XCTAssertThrowsError(try CalendarBridge.operation(command("calendar_save",
            fields: ["calendar_save": create, "calendar_delete": delete]))) {
            XCTAssertEqual($0 as? CalendarFailure, .invalidRequest)
        }
    }

    func testAudioStopUsesTheUrgentLaneWhilePermissionsAreInFlight() throws {
        var router = NativeCommandRouter()
        let permissions = try command("permissions", fields: ["request": ["calendars"]])
        let stop = try command("audio_stop", fields: ["stream_id": "microphone-1"])

        XCTAssertEqual(router.route(permissions, at: 0), .start)
        XCTAssertEqual(router.route(stop, at: 1), .bypass)
    }

    func testCaptureClockConvertsOnlyValidNonnegativeTimes() {
        XCTAssertEqual(CaptureClock.nanoseconds(CMTime(value: 3, timescale: 2)), 1_500_000_000)
        XCTAssertNil(CaptureClock.nanoseconds(CMTime(value: -1, timescale: 1)))
        XCTAssertNil(CaptureClock.nanoseconds(.invalid))
    }

    func testObservedWindowDescriptorHashIsStableAndIncludesIdentity() throws {
        let original = ObservedWindowDescriptor(
            windowID: 7, applicationPID: 42, applicationInstanceID: "instance-a",
            launchDateBits: 10, bundleID: "test.fixture", title: "Documento",
            x: 1, y: 2, width: 800, height: 600, level: 0)
        let changed = ObservedWindowDescriptor(
            windowID: 7, applicationPID: 42, applicationInstanceID: "instance-b",
            launchDateBits: 10, bundleID: "test.fixture", title: "Documento",
            x: 1, y: 2, width: 800, height: 600, level: 0)

        XCTAssertEqual(original.hash, original.hash)
        XCTAssertNotNil(original.hash)
        XCTAssertNotEqual(original.hash, changed.hash)
    }

    func testPlaybackActivityClearsTheIdentifierWhenPlaybackStops() {
        let activity = PlaybackActivity()
        activity.update(active: true, playbackID: "answer-1")
        XCTAssertEqual(activity.snapshot().id, "answer-1")
        XCTAssertEqual(activity.snapshot().active, true)

        activity.update(active: false, playbackID: "must-not-survive")
        XCTAssertNil(activity.snapshot().id)
        XCTAssertEqual(activity.snapshot().active, false)
    }

    /// REF-06 A2: a leaving helper releases its shortcuts explicitly, and the
    /// same release runs again when the object goes. Neither may touch Carbon
    /// for shortcuts that were never registered — this test registers none, so
    /// it takes nothing from the machine that runs it.
    func testA2ReleasingShortcutsThatWereNeverRegisteredIsANoOp() {
        let hotkeys = GlobalHotkeys()
        hotkeys.unregister()
        hotkeys.unregister()
    }
}
