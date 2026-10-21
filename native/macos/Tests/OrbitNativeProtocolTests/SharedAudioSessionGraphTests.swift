import AVFoundation
import XCTest
@testable import OrbitNativeHost

/// 2026-10-17: every call with Deimos failed to open the microphone with
/// «com.apple.coreaudio.avfaudio -10875». The mixer was created only when the
/// player was connected, after voice processing was turned on, and an engine
/// in that order never starts (measured on Mars's MacBook Pro). The graph has
/// its mixer from the start. Nothing here starts the engine or opens a device.
final class SharedAudioSessionGraphTests: XCTestCase {
    func testTheGraphHasItsMixerBeforeAnyModeIsChosen() {
        let session = SharedAudioSession()
        // `attachedNodes` reads the graph without creating a node; asking for
        // `mainMixerNode` here would create the mixer and prove nothing.
        let nodes = session.engine.attachedNodes
        XCTAssertTrue(nodes.contains { $0 is AVAudioMixerNode },
                      "the mixer exists before voice processing can be turned on")
        XCTAssertFalse(session.configured)
    }

    func testStopReleasesTheGraphAndTheNextUseHasANewPipeline() {
        let session = SharedAudioSession()
        let previousEngine = session.engine
        let previousPipeline = session.pipelineID

        session.stop()

        XCTAssertEqual(session.mode, .unprocessed)
        XCTAssertFalse(session.configured)
        XCTAssertEqual(session.status["engine_running"] as? Bool, false)
        XCTAssertNotEqual(session.pipelineID, previousPipeline)
        XCTAssertFalse(session.engine === previousEngine,
                       "stopping must release the graph that owns VoiceProcessingIO")
        XCTAssertTrue(session.engine.attachedNodes.contains { $0 is AVAudioMixerNode })
    }

    func testCaptureEndingDuringPlaybackKeepsTheGraphUntilPlaybackEnds() {
        let session = SharedAudioSession()
        let previousEngine = session.engine
        session.captureDidStart()
        session.playbackActivity.update(active: true, playbackID: "answer")

        session.captureDidStop()
        session.stopIfIdle()
        XCTAssertTrue(session.engine === previousEngine)

        session.playbackActivity.update(active: false, playbackID: nil)
        session.stopIfIdle()
        XCTAssertFalse(session.engine === previousEngine)
    }

    func testPlaybackEndingDuringCaptureKeepsTheGraphUntilCaptureEnds() {
        let session = SharedAudioSession()
        let previousEngine = session.engine
        session.captureDidStart()
        session.playbackActivity.update(active: true, playbackID: "answer")

        session.playbackActivity.update(active: false, playbackID: nil)
        session.stopIfIdle()
        XCTAssertTrue(session.engine === previousEngine)

        session.captureDidStop()
        session.stopIfIdle()
        XCTAssertFalse(session.engine === previousEngine)
    }

    func testIdleStopAndStatusDoNotCreateAGraphOrInventDuckingReadback() throws {
        let session = SharedAudioSession()
        let pipeline = session.pipelineID
        session.stopIfIdle()
        session.stop()
        XCTAssertEqual(session.pipelineID, pipeline)
        XCTAssertNil(session.status["other_audio_ducking_level"])
        XCTAssertNil(session.status["other_audio_ducking_advanced"])
        let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(session.statusFields)) as? [String: Any]
        XCTAssertNil(encoded?["other_audio_ducking_level"])
        XCTAssertNil(encoded?["other_audio_ducking_advanced"])
        XCTAssertEqual(encoded?["engine_running"] as? Bool, false)
        session.stop()
        XCTAssertEqual(session.pipelineID, pipeline, "status must not have constructed a graph")
    }
    func testRouteComparisonUsesEitherDefaultDeviceAndPreservesUnknown() {
        let route = AudioRouteDevices(input: 1, output: 2)
        XCTAssertEqual(route.changed(from: route), false)
        XCTAssertEqual(AudioRouteDevices(input: 3, output: 2).changed(from: route), true)
        XCTAssertEqual(AudioRouteDevices(input: 1, output: 3).changed(from: route), true)
        XCTAssertNil(route.changed(from: nil))
        XCTAssertNil(AudioRouteDevices(input: nil, output: 2).changed(from: route))
        XCTAssertEqual(AudioRouteDevices(input: nil, output: 3).changed(from: route), true)
        XCTAssertEqual(AudioRouteDevices(input: 3, output: nil).changed(from: route), true)
    }

}
