import Foundation
import XCTest
@testable import OrbitNativeHost
import OrbitNativeProtocol
import OrbitNativeTransport

/// What the helper says about «Oye Deimos» and the question it
/// sends. The detector itself needs macOS 26 and a microphone: the reviewer
/// measures it on the Mac.
/// A listener that records what the host does to it.
private final class FakeListener: PhraseListening, @unchecked Sendable {
    private let lock = NSLock()
    private var events = [String]()
    var recorded: [String] { lock.withLock { events } }

    func begin() { lock.withLock { events.append("begin") } }
    func stop() { lock.withLock { events.append("stop") } }
    func finish() { lock.withLock { events.append("finish") } }
    func receive(pcm16: Data, sampleRate: UInt32, rmsDBFS: Double) {
        lock.withLock { events.append("frame") }
    }
}

final class AssistantVoiceTriggerTests: XCTestCase {
    /// The channel's pipes live as long as the test: a local the optimizer
    /// releases after its last use closes the reading end, and the trigger's
    /// next report then dies of SIGPIPE.
    private var pipes = [Pipe]()

    private func channel() -> NativeChannel {
        let input = Pipe()
        let output = Pipe()
        pipes += [input, output]
        return NativeChannel(input: input.fileHandleForReading,
                             output: output.fileHandleForWriting)
    }

    func testConfiguredNameReachesTheListenerAndSurvivesAStreamRestart() throws {
        let command = try NativeWireCommand.decode(JSONSerialization.data(withJSONObject: [
            "v": 1, "id": "wake-name", "command": "audio_wake_phrase",
            "stream_id": "mic-1", "enabled": true, "assistant_name": "Atlas"]))
        guard case .audioWakePhrase(let wire) = command else {
            return XCTFail("missing typed wake command")
        }
        var names = [String]()
        let trigger = AssistantVoiceTrigger(channel: channel(), makeListener: { name, _, _ in
            names.append(name)
            return FakeListener()
        })
        _ = trigger.start(streamID: "mic-1", assistantName: wire.fields.assistant_name,
                          allow: { _ in true })
        trigger.stop()
        _ = trigger.start(streamID: "mic-1", allow: { _ in true })
        XCTAssertEqual(names, ["Atlas", "Atlas"])
        trigger.stop()
    }

    /// The trigger's `stop` — what every helper stop, the user's privacy
    /// brake included, calls first — stops its listener at once, and a
    /// second stop does nothing more.
    func testStoppingTheTriggerStopsItsListenerAtOnce() {
        let channel = channel()
        let fake = FakeListener()
        let trigger = AssistantVoiceTrigger(channel: channel, makeListener: { _, _, _ in fake })
        let sink = trigger.start(streamID: "mic-1", allow: { _ in true })
        XCTAssertTrue(sink === fake)
        XCTAssertTrue(trigger.isListening)
        XCTAssertEqual(fake.recorded, ["begin"])
        trigger.stop()
        XCTAssertFalse(trigger.isListening)
        XCTAssertEqual(fake.recorded, ["begin", "stop"])
        trigger.stop()
        XCTAssertEqual(fake.recorded, ["begin", "stop"])
    }

    /// When the microphone stream ends, its listener is finished
    /// before the stop returns, and it receives nothing after.
    func testAStreamThatEndsFinishesItsListenerBeforeTheStopReturns() async throws {
        let channel = channel()
        let relay = try AudioFrameRelay(
            streamID: "mic-1", source: "mic_local", channel: channel,
            processing: AudioProcessingObservation(mode: .unprocessed, enabled: false,
                bypassed: true, agcEnabled: false, sharedPlaybackReference: false,
                pipelineID: "fixture"))
        let fake = FakeListener()
        relay.setFrameSink(fake)
        let drained = await relay.stop()
        XCTAssertTrue(drained)
        XCTAssertEqual(fake.recorded, ["finish"])
    }

    private func fields(_ event: NativeEvent) throws -> NSDictionary {
        let data = try JSONEncoder().encode(event)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? NSDictionary)
    }

    func testEveryStateSaysItsCauseAndNumbersAndNothingElse() throws {
        XCTAssertEqual(try fields(VoiceTriggerReport.unavailable(cause: "macos_version",
            osVersion: "15.4.1").event),
            ["v": 1, "event": "voice_trigger", "state": "unavailable",
             "cause": "macos_version", "os_version": "15.4.1"])
        XCTAssertEqual(try fields(VoiceTriggerReport.idle.event),
            ["v": 1, "event": "voice_trigger", "state": "idle"])
        XCTAssertEqual(try fields(VoiceTriggerReport.downloading(percent: 42).event),
            ["v": 1, "event": "voice_trigger", "state": "downloading", "percent": 42])
        XCTAssertEqual(try fields(VoiceTriggerReport.listening.event),
            ["v": 1, "event": "voice_trigger", "state": "listening"])
        XCTAssertEqual(try fields(VoiceTriggerReport.failed(cause: "assets_download_failed",
            detail: "red no disponible").event),
            ["v": 1, "event": "voice_trigger", "state": "failed",
             "cause": "assets_download_failed", "detail": "red no disponible"])
    }

    /// Steps carry no words; an abandonment carries its cause and audio times.
    func testEveryStepOfThePhraseIsANoticeWithoutWords() throws {
        XCTAssertEqual(try fields(VoiceTriggerReport.step(.heard).event),
            ["v": 1, "event": "notice", "code": "wake_phrase_heard"])
        XCTAssertEqual(try fields(VoiceTriggerReport.step(.questionDropped).event),
            ["v": 1, "event": "notice", "code": "wake_phrase_question_dropped"])
        XCTAssertEqual(VoiceTriggerReport.step(.heard).event.name, "notice")
    }

    private func assertAbandonment(_ policy: SpokenQuestionPolicy, cause: String,
                                   elapsedMS: UInt64, silentMS: UInt64,
                                   file: StaticString = #filePath, line: UInt = #line) throws {
        XCTAssertFalse(policy.listening, file: file, line: line)
        let step = try XCTUnwrap(policy.step, file: file, line: line)
        XCTAssertEqual(try fields(VoiceTriggerReport.step(step).event),
            ["v": 1, "event": "notice", "code": "wake_phrase_abandoned",
             "abandonment": ["cause": cause, "elapsed_ms": elapsedMS, "silent_ms": silentMS]],
            file: file, line: line)
    }

    func testRevisedPhraseReportsItsCauseAndTimesWithoutWords() throws {
        var policy = SpokenQuestionPolicy()
        XCTAssertNil(policy.heard(.init(text: "Oye, Demos", start: 0, end: 1, isFinal: false)))
        XCTAssertNil(policy.voice(at: 2.5, speaking: false))
        XCTAssertNil(policy.heard(.init(text: "Oye, demos por cerrado", start: 0, end: 2.8, isFinal: true)))
        try assertAbandonment(policy, cause: "phrase_revised", elapsedMS: 1_800, silentMS: 0)
        XCTAssertNil(policy.voice(at: 3, speaking: false))
        XCTAssertNil(policy.step)
    }

    func testNoQuestionDuringTheWaitReportsItsCauseAndTimes() throws {
        var policy = SpokenQuestionPolicy()
        XCTAssertNil(policy.heard(.init(text: "Oye, Demos", start: 0, end: 1, isFinal: true)))
        XCTAssertNil(policy.voice(at: 6.999, speaking: false))
        XCTAssertTrue(policy.listening)
        XCTAssertNil(policy.step)
        XCTAssertNil(policy.voice(at: 7, speaking: false))
        try assertAbandonment(policy, cause: "question_wait_expired", elapsedMS: 6_000, silentMS: 6_000)
    }

    func testTheQuestionCeilingReportsItsCauseAndTimesWhileVoiceContinues() throws {
        var policy = SpokenQuestionPolicy()
        XCTAssertNil(policy.heard(.init(text: "Oye, Demos", start: 0, end: 1, isFinal: true)))
        let end = 1 + policy.maximumSeconds
        XCTAssertNil(policy.voice(at: end - 0.001, speaking: true))
        XCTAssertTrue(policy.listening)
        XCTAssertNil(policy.step)
        XCTAssertNil(policy.voice(at: end, speaking: true))
        try assertAbandonment(policy, cause: "question_too_long",
                             elapsedMS: UInt64(policy.maximumSeconds * 1_000), silentMS: 0)
    }

    /// A question the user's stop or a new stream does not let leave is
    /// said as dropped, without its text; one that may leave is sent.
    @MainActor
    func testAQuestionThatMayNotLeaveIsSaidAsDroppedWithoutItsText() async throws {
        let output = Pipe()
        let channel = NativeChannel(input: Pipe().fileHandleForReading,
                                    output: output.fileHandleForWriting)
        let question = NativeEvent.makeAssistantQuestion(streamID: "mic-1",
            text: "¿Qué dijo Lucía?", startedMS: 1, endedMS: 2)
        AssistantVoiceTrigger.deliver(question, streamID: "mic-1", allow: { _ in false },
                                      channel: channel)
        AssistantVoiceTrigger.deliver(question, streamID: "mic-1", allow: { _ in true },
                                      channel: channel)
        let flushed = await channel.flush()
        XCTAssertTrue(flushed)
        try output.fileHandleForWriting.close()
        let bytes = try XCTUnwrap(output.fileHandleForReading.readToEnd())
        let lines = String(decoding: bytes, as: UTF8.self).split(separator: "\n").map(String.init)
        XCTAssertEqual(lines.count, 2)
        let dropped = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(lines[0].utf8))
            as? NSDictionary)
        XCTAssertEqual(dropped, ["v": 1, "event": "notice", "code": "wake_phrase_question_dropped"])
        XCTAssertFalse(lines[0].contains("Lucía"))
        let sent = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(lines[1].utf8))
            as? NSDictionary)
        XCTAssertEqual(sent["event"] as? String, "assistant_question")
    }

    /// The phrase is turned on and off on a live stream by its own
    /// order, so it follows the meeting whoever opened the microphone.
    func testThePhraseHasItsOwnOrderOnALiveStream() throws {
        let on = try NativeWireCommand.decode(JSONSerialization.data(withJSONObject: [
            "v": 1, "id": "wake-1", "command": "audio_wake_phrase", "stream_id": "mic-1",
            "enabled": true]))
        guard case .audioWakePhrase(let wire) = on else {
            return XCTFail("audio_wake_phrase lost its typed variant")
        }
        XCTAssertEqual(wire.fields.stream_id, "mic-1")
        XCTAssertTrue(wire.fields.enabled)
        XCTAssertThrowsError(try NativeWireCommand.decode(JSONSerialization.data(withJSONObject: [
            "v": 1, "id": "wake-2", "command": "audio_wake_phrase", "stream_id": "mic-1"])), "enabled is required")
    }

    func testReadyKeepsVoiceFactsWithoutNegotiatingThePackagedProtocol() throws {
        let ready = try fields(NativeEvent.makeReady(selectedTextAvailable: false,
            launchMode: "managed_stdio", isolatedFixture: false))
        XCTAssertNil(ready["capabilities"])
        XCTAssertEqual(ready["supported_audio_processing"] as? [String], ["unprocessed", "voice_processing"])
        XCTAssertEqual(ready["voice_processing_available"] as? Bool, false)
        let fixture = try fields(NativeEvent.makeReady(selectedTextAvailable: true,
            launchMode: "isolated_test_stdio", isolatedFixture: true))
        XCTAssertEqual(fixture["selected_text_available"] as? Bool, false)
        XCTAssertEqual(fixture["fixture"] as? Bool, true)
    }
}
