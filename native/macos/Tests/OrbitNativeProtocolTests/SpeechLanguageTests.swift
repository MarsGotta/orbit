import Foundation
import XCTest
@testable import OrbitNativeHost
import OrbitNativeProtocol
import OrbitNativeTransport

/// A recognizer supplies invented results without speech assets or a microphone.
private final class FakeRecognizer: PhraseListening {
    private var policy: SpokenQuestionPolicy
    private let question: (SpokenQuestion, Double) -> Void
    private(set) var stopped = false

    init(names: [String], locale: NativeSpeechLocale,
         question: @escaping (SpokenQuestion, Double) -> Void) {
        policy = SpokenQuestionPolicy(assistantNames: names, locale: locale)
        self.question = question
    }

    func recognize(_ text: String) -> SpokenQuestion? {
        _ = policy.heard(.init(text: text, start: 0, end: 2, isFinal: true))
        let spoken = policy.voice(at: 3.5, speaking: false)
        if let spoken { question(spoken, 3.5) }
        return spoken
    }

    func begin() {}
    func stop() { stopped = true }
    func finish() { stop() }
    func receive(pcm16: Data, sampleRate: UInt32, rmsDBFS: Double) {}
}

final class SpeechLanguageTests: XCTestCase {
    @MainActor
    func testEachLanguageHearsItsPhraseAndQuestionAndIgnoresTheOther() async throws {
        for (locale, accepted, rejected, question) in [
            (NativeSpeechLocale.englishUS, "Hey Deimos, what changed?", "Oye Deimos, what changed?", "what changed?"),
            (.englishGB, "Hey Deimos, what changed?", "Oye Deimos, what changed?", "what changed?"),
            (.spanish, "Oye Deimos, ¿qué cambió?", "Hey Deimos, ¿qué cambió?", "¿qué cambió?")
        ] {
            let input = Pipe(), output = Pipe()
            let channel = NativeChannel(input: input.fileHandleForReading, output: output.fileHandleForWriting)
            let delivered = expectation(description: "recognized question reaches delivery guard")
            var recognizers = [FakeRecognizer]()
            var locales = [NativeSpeechLocale]()
            let trigger = AssistantVoiceTrigger(channel: channel, makeListener: { names, locale, _, callback in
                locales.append(locale)
                let fake = FakeRecognizer(names: names, locale: locale, question: callback)
                recognizers.append(fake)
                return fake
            })
            _ = trigger.start(streamID: "mic-1", locale: locale, allow: { _ in false })
            XCTAssertNil(recognizers[0].recognize(rejected))
            // Restart on the same stream also retains the recognizer's locale.
            _ = trigger.start(streamID: "mic-1", allow: { _ in delivered.fulfill(); return true })
            XCTAssertTrue(recognizers[0].stopped)
            XCTAssertEqual(recognizers[1].recognize(accepted)?.text, question)
            XCTAssertEqual(locales, [locale, locale])
            await fulfillment(of: [delivered], timeout: 5)
            trigger.stop()
            let flushed = await channel.flush()
            XCTAssertTrue(flushed)
            try output.fileHandleForWriting.close()
            let bytes = try XCTUnwrap(output.fileHandleForReading.readToEnd())
            let events = try String(decoding: bytes, as: UTF8.self).split(separator: "\n").map {
                try XCTUnwrap(JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any])
            }
            let questions = events.filter { $0["event"] as? String == "assistant_question" }
            XCTAssertEqual(questions.count, 1)
            XCTAssertEqual(questions.first?["text"] as? String, question)
            XCTAssertEqual(questions.first?["assistant_name"] as? String, "Deimos")
        }
    }

    @MainActor
    func testLanguageChangeReplacesTheRecognizerOnTheSameMicrophone() async throws {
        let input = Pipe(), output = Pipe()
        let channel = NativeChannel(input: input.fileHandleForReading, output: output.fileHandleForWriting)
        let delivered = expectation(description: "each re-armed question reaches the delivery guard")
        delivered.expectedFulfillmentCount = 3
        var recognizers = [FakeRecognizer]()
        var locales = [NativeSpeechLocale]()
        let trigger = AssistantVoiceTrigger(channel: channel, makeListener: { names, locale, _, callback in
            locales.append(locale)
            let fake = FakeRecognizer(names: names, locale: locale, question: callback)
            recognizers.append(fake)
            return fake
        })
        for (locale, phrase, foreign) in [
            (NativeSpeechLocale.spanish, "Oye Deimos, ¿qué cambió?", "Hey Deimos, what changed?"),
            (.englishUS, "Hey Deimos, what changed?", "Oye Deimos, ¿qué cambió?"),
            (.spanish, "Oye Deimos, ¿qué cambió?", "Hey Deimos, what changed?")
        ] {
            _ = trigger.start(streamID: "mic-1", locale: locale, allow: { _ in delivered.fulfill(); return false })
            if recognizers.count > 1 {
                XCTAssertTrue(recognizers[recognizers.count - 2].stopped)
            }
            let current = recognizers[recognizers.count - 1]
            XCTAssertNil(current.recognize(foreign))
            XCTAssertNotNil(current.recognize(phrase))
        }
        XCTAssertEqual(locales, [.spanish, .englishUS, .spanish])
        await fulfillment(of: [delivered], timeout: 5)
        trigger.stop()
        XCTAssertTrue(recognizers[2].stopped)
        let flushed = await channel.flush()
        XCTAssertTrue(flushed)
        try output.fileHandleForWriting.close()
        _ = try output.fileHandleForReading.readToEnd()
    }

    func testEnglishDoesNotUseSpanishRecognizerAliases() {
        XCTAssertNil(WakePhrase.question(after: "Hola Deimos, hello", locale: .englishUS))
        XCTAssertNil(WakePhrase.question(after: "Hey Demos, hello", locale: .englishUS))
        XCTAssertEqual(WakePhrase.question(after: "Hey Phobos, hello", assistantName: "Phobos", locale: .englishGB), "hello")
    }

    func testTypedListeningCommandCarriesTheLocaleAndRejectsUnknownLocales() throws {
        for locale in ["en_US", "en_GB", "es_ES"] {
            let data = Data("{\"v\":1,\"id\":\"wake\",\"command\":\"audio_wake_phrase\",\"stream_id\":\"mic-1\",\"enabled\":true,\"locale\":\"\(locale)\"}".utf8)
            guard case .audioWakePhrase(let wire) = try NativeWireCommand.decode(data) else {
                return XCTFail("missing listening command")
            }
            XCTAssertEqual(wire.fields.locale?.rawValue, locale)
        }
        XCTAssertThrowsError(try NativeWireCommand.decode(Data(
            "{\"v\":1,\"id\":\"wake\",\"command\":\"audio_wake_phrase\",\"stream_id\":\"mic-1\",\"enabled\":true,\"locale\":\"fr_FR\"}".utf8)))
    }
}
