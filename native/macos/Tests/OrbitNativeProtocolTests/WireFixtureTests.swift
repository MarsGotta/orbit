import Foundation
import XCTest
@testable import OrbitNativeProtocol

private struct NativeFixtureDocument: Decodable {
    let schema: String
    let frames: [NativeWireCommand]
}

final class WireFixtureTests: XCTestCase {
    private func fixtureURL(_ name: String) -> URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 {
            root.deleteLastPathComponent()
        }
        return root.appendingPathComponent("contracts/fixtures/\(name)")
    }

    func testSharedNativeWireFixtureDecodes() throws {
        let decoder = JSONDecoder()
        let native = try decoder.decode(
            NativeFixtureDocument.self,
            from: Data(contentsOf: fixtureURL("native-frame-v1.json"))
        )
        XCTAssertEqual(native.schema, "orbit.native-frame.v1")
        XCTAssertEqual(Set(native.frames.map(\.command)),
                       ["privacy_stop", "audio_start"])
        for frame in native.frames {
            XCTAssertEqual(frame.v, NativeProtocol.version)
            XCTAssertFalse(frame.id.isEmpty)
            switch frame {
            case .privacyStop: break
            case .audioStart(let wire):
                XCTAssertNil(wire.fields.application_pid)
                XCTAssertEqual(wire.fields.capture_scope, .system)
            default: XCTFail("Unrecognized fixture command: \(frame.command)")
            }
        }
    }

    private func nativeFixtureFiles(_ kind: String) throws -> [URL] {
        let directory = fixtureURL("native/\(kind)")
        let files = try FileManager.default.contentsOfDirectory(
            at: directory, includingPropertiesForKeys: nil
        )
        XCTAssertTrue(files.allSatisfy { $0.pathExtension == "json" })
        return files.filter { $0.pathExtension == "json" }
    }

    func testEveryNativeCommandDecodesAndEveryProducedEventMatchesItsFixture() throws {
        let commands = try nativeFixtureFiles("commands")
        XCTAssertEqual(Set(commands.map { $0.deletingPathExtension().lastPathComponent }), NativeWireCommand.names)
        for file in commands {
            let frame = try NativeWireCommand.decode(Data(contentsOf: file))
            XCTAssertEqual(frame.command, file.deletingPathExtension().lastPathComponent)
            XCTAssertEqual(frame.v, NativeProtocol.version)
            XCTAssertFalse(frame.id.isEmpty)
        }
        let id = "00000000-0000-4000-8000-000000000911"
        let processing = AudioProcessingObservation(mode: .unprocessed, enabled: false,
            bypassed: true, agcEnabled: false, sharedPlaybackReference: false, pipelineID: "fixture")
        let produced: [String: NativeEvent] = [
            "ready": .makeReady(selectedTextAvailable: true, launchMode: "managed_stdio", isolatedFixture: false),
            "state": .makeState(stopped: false, connected: true, instanceID: id, generation: 1,
                selectedTextAvailable: true, microphone: false, systemAudio: false, playing: false,
                voiceProcessingAvailable: false, playbackReferenceAvailable: false,
                audioProcessing: .init(mode: "unprocessed", enabled: false, bypassed: true,
                    agcEnabled: false, playbackReference: "none", pipelineID: "fixture",
                    engineRunning: false, renderSampleRate: 48_000),
                calendar: .init(authorization: "full_access", physicalReadPending: false, failure: nil)),
            "response": try .makeResponse(id: id, result: [:]),
            "error": .makeError(code: "audio_route_start_failed", osDomain: "NSOSStatusErrorDomain", osStatus: -50),
            "hotkey": .makeHotkey(action: "meeting_toggle", phase: "pressed",
                applications: [(pid: 4242, bundleID: "com.example.meeting", name: "Fixture")]),
            "privacy_stop": .makeStop(latches: true, reason: "local_hotkey", stopped: true, drainComplete: true),
            "capture_stopped": .makeStop(latches: false, reason: "session_inactive", stopped: false, drainComplete: true),
            "local_rearm": .makeLocalRearm(),
            "calendar_invalidated": .makeCalendarInvalidated(code: "calendar_invalidated"),
            "gap": .makeGap(source: "mic_local", streamID: "stream-1", reason: "audio_device_changed",
                frames: 0, durationUnknown: false, durationNS: 2_000_000_000),
            "audio": .makeAudio(source: "mic_local", streamID: "stream-1", sequence: 0,
                sampleRate: 32_000, frames: 1, monotonicNS: 100, pcmBase64: "AQI=",
                processing: processing, playback: (active: true, id: "play-1")),
            "playback_progress": .makePlaybackProgress(id: "play-1", state: "started", active: true, sequence: 0),
            "microphone_activity": .makeMicrophoneActivity([(pid: 4242, bundleID: "us.zoom.xos", name: "zoom.us")]),
            "assistant_question": .makeAssistantQuestion(streamID: "stream-1",
                text: "¿Qué dijo Lucía del presupuesto?", startedMS: 1_800_000_000_000, endedMS: 1_800_000_004_200,
                assistantName: "Vega"),
            "voice_trigger": .makeVoiceTrigger(state: "downloading", percent: 42),
            "notice": .makeNotice(code: "wake_phrase_abandoned", abandonment:
                .init(cause: .phraseRevised, elapsed_ms: 1_800, silent_ms: 0)),
        ]
        // helper_disconnected is synthesized by the daemon, never by Swift.
        let events = try nativeFixtureFiles("events").filter { $0.lastPathComponent != "helper_disconnected.json" }
        XCTAssertEqual(Set(events.map { $0.deletingPathExtension().lastPathComponent }), NativeEvent.names)
        XCTAssertEqual(Set(produced.keys), NativeEvent.names)
        for file in events {
            let name = file.deletingPathExtension().lastPathComponent
            let event = try XCTUnwrap(produced[name])
            XCTAssertEqual(event.name, name)
            XCTAssertEqual(try JSONSerialization.jsonObject(with: JSONEncoder().encode(event)) as? NSDictionary,
                           try JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? NSDictionary, name)
        }
    }

    func testResultFixturesTravelThroughTheProductionResponseEncoder() throws {
        for file in try nativeFixtureFiles("results") {
            let body = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
            let event = try NativeEvent.makeResponse(id: "fixture", result: body)
            let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(event)) as? [String: Any])
            XCTAssertEqual(encoded["ok"] as? Bool, true)
            XCTAssertEqual(encoded["result"] as? NSDictionary, body as NSDictionary, file.lastPathComponent)
        }
    }
}
