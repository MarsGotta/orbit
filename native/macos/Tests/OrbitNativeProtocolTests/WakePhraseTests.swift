import XCTest
@testable import OrbitNativeProtocol

/// «Oye Deimos» on transcribed text. The accepted lines are the ones
/// macOS's on-device Spanish transcription wrote when measured on a Mac,
/// plus the forms a person would spell.
final class WakePhraseTests: XCTestCase {
    func testThePhraseAsMacOSTranscribesItOpensTheQuestion() {
        let cases: [(String, String)] = [
            ("Oye, Demos, ¿qué dijo Lucía del presupuesto de diciembre?",
             "¿qué dijo Lucía del presupuesto de diciembre?"),
            ("Oye, Demos, resume lo último.", "resume lo último."),
            ("Hola Demos. resume lo último.", "resume lo último."),
            ("Demos, resume lo último.", "resume lo último."),
            ("Oye Deimos, ¿qué dijo Lucía?", "¿qué dijo Lucía?"),
            ("Hola, Deimos: resume lo último", "resume lo último"),
            ("Deimos, resume.", "resume."),
            ("Oye Deimos qué se decidió", "qué se decidió"),
            ("OYE, DEMOS, resume", "resume"),
            ("Vale, gracias. Oye, Demos, ¿y el presupuesto?", "¿y el presupuesto?"),
            ("¡Oye, Deimos! ¿Lo apuntas?", "¿Lo apuntas?"),
        ]
        for (heard, question) in cases {
            XCTAssertEqual(WakePhrase.question(after: heard), question, heard)
        }
    }

    func testThePhraseAloneWaitsForTheQuestion() {
        for heard in ["Oye, Demos", "Oye, Demos.", "Deimos,", "Hola Deimos"] {
            XCTAssertEqual(WakePhrase.question(after: heard), "", heard)
        }
    }

    func testDemosInOrdinarySpeechOpensNothing() {
        for heard in [
            "Oye, demos por cerrado esto.",
            "Demos por cerrado el tema.",
            "Vamos a ver demos, el informe.",
            "Le dije: oye, demos, vale.",
            "Oye Orbit, resume lo último.",
            "Oye, Orbitz, resume lo último.",
            "Ellie Diamonds resume lo último.",
            "Oye Demos qué tal",
            "",
        ] {
            XCTAssertNil(WakePhrase.question(after: heard), heard)
        }
    }

    func testConfiguredNamesUseTheSamePhraseBoundariesWithoutDeimosAliases() {
        for text in ["Oye Atlas, resume", "HOLA ÁTLAS resume", "Átlas, resume",
                     "Seguimos. Oye Atlas: resume"] {
            XCTAssertEqual(WakePhrase.question(after: text, assistantName: "Atlas"), "resume", text)
        }
        for text in ["Oye Deimos, resume", "Demos, resume", "Oye Atlasiano, resume",
                     "Le dije oye Atlas, resume", "Atlas resume"] {
            XCTAssertNil(WakePhrase.question(after: text, assistantName: "Atlas"), text)
        }
        XCTAssertEqual(WakePhrase.question(after: "Hola ATLAS AZUL, resume",
                                           assistantName: "Átlas Azul"), "resume")
        XCTAssertEqual(WakePhrase.question(after: "Oye, Demos, resume",
                                           assistantName: "Déimos"), "resume")
        XCTAssertEqual(WakePhrase.question(after: "Hola, resume",
                                           assistantName: "Hola"), "resume")
        XCTAssertEqual(WakePhrase.question(after: "Hola Atlas, resume",
                                           assistantName: "Hola Atlas"), "resume")
    }

    func testConfiguredNameReachesTheSpokenQuestionPolicy() {
        var policy = SpokenQuestionPolicy(assistantName: "Atlas")
        XCTAssertNil(policy.heard(final("Oye Atlas, ¿qué decidió Ana Pérez?", 0, 2)))
        XCTAssertTrue(policy.listening)
        XCTAssertEqual(policy.voice(at: 3.5, speaking: false)?.text, "¿qué decidió Ana Pérez?")
        XCTAssertFalse(policy.listening)
    }

    /// One name per agent: each opens the phrase by the same rules, and the
    /// match says which one; «Demos» is still only Deimos's.
    func testEveryAgentNameOpensThePhraseAndSaysWhichOne() {
        let names = ["Deimos", "Atlas", "Atlas Azul"]
        let cases: [(String, String, String)] = [
            ("Oye Atlas, resume", "Atlas", "resume"),
            ("HOLA ÁTLAS resume", "Atlas", "resume"),
            ("Atlas Azul, resume", "Atlas Azul", "resume"),
            ("Oye Atlas Azul qué se decidió", "Atlas Azul", "qué se decidió"),
            ("Oye, Demos, resume", "Deimos", "resume"),
            ("Deimos, resume", "Deimos", "resume"),
            ("Vale. Hola Atlas: ¿y el presupuesto?", "Atlas", "¿y el presupuesto?"),
        ]
        for (text, name, question) in cases {
            XCTAssertEqual(WakePhrase.match(after: text, assistantNames: names),
                           WakePhraseMatch(name: name, question: question), text)
        }
        for text in ["Oye, demos por cerrado esto.", "Le dije oye Atlas, resume", "Oye Vega, resume"] {
            XCTAssertNil(WakePhrase.match(after: text, assistantNames: names), text)
        }
        XCTAssertNil(WakePhrase.match(after: "Oye Atlas, resume", assistantNames: []))
        XCTAssertNil(WakePhrase.match(after: "Demos, resume", assistantNames: ["Atlas"]))
    }

    func testTheSpokenQuestionSaysWhichAgentWasNamed() {
        var policy = SpokenQuestionPolicy(assistantNames: ["Deimos", "Atlas"])
        XCTAssertNil(policy.heard(final("Oye Atlas, ¿qué decidió Ana Pérez?", 0, 2)))
        XCTAssertTrue(policy.listening)
        XCTAssertEqual(policy.voice(at: 3.5, speaking: false),
                       SpokenQuestion(text: "¿qué decidió Ana Pérez?", start: 0, end: 2, name: "Atlas"))
        XCTAssertNil(policy.heard(final("Oye, Demos, resume lo último.", 10, 12)))
        XCTAssertEqual(policy.voice(at: 13.5, speaking: false)?.name, "Deimos")
    }

    // MARK: - The question, from results and the microphone's level

    private func final(_ text: String, _ start: Double, _ end: Double) -> HeardSegment {
        HeardSegment(text: text, start: start, end: end, isFinal: true)
    }

    private func volatile(_ text: String, _ start: Double, _ end: Double) -> HeardSegment {
        HeardSegment(text: text, start: start, end: end, isFinal: false)
    }

    /// Feeds the microphone's level every 10 ms from `from` to `to`.
    private func levels(_ policy: inout SpokenQuestionPolicy, from: Double, to: Double,
                        speaking: Bool) -> [SpokenQuestion] {
        var sent = [SpokenQuestion]()
        var time = from
        while time <= to + 1e-9 {
            if let question = policy.voice(at: time, speaking: speaking) { sent.append(question) }
            time += 0.01
        }
        return sent
    }

    func testTheQuestionEndsOnAPauseOfOneAndAHalfSeconds() {
        var policy = SpokenQuestionPolicy()
        XCTAssertTrue(levels(&policy, from: 0, to: 4, speaking: true).isEmpty)
        XCTAssertNil(policy.heard(volatile("Oye, Demos, ¿qué dijo", 0.5, 2)))
        XCTAssertTrue(policy.listening)
        XCTAssertNil(policy.heard(final("Oye, Demos, ¿qué dijo Lucía del presupuesto?", 0.5, 4)))
        XCTAssertTrue(levels(&policy, from: 4.01, to: 5.48, speaking: false).isEmpty,
                      "less than 1.5 s of silence")
        let sent = levels(&policy, from: 5.49, to: 9, speaking: false)
        XCTAssertEqual(sent, [SpokenQuestion(text: "¿qué dijo Lucía del presupuesto?", start: 0.5, end: 4)])
        XCTAssertFalse(policy.listening)
    }

    func testAQuestionInTwoResultsIsOneQuestion() {
        var policy = SpokenQuestionPolicy()
        XCTAssertNil(policy.heard(final("Oye, Demos,", 10, 11)))
        XCTAssertTrue(levels(&policy, from: 11, to: 11.8, speaking: false).isEmpty,
                      "a short breath after the name")
        XCTAssertNil(policy.heard(final("¿qué dijo Lucía del presupuesto?", 11.8, 13.5)))
        XCTAssertTrue(levels(&policy, from: 11.8, to: 13.5, speaking: true).isEmpty)
        let sent = levels(&policy, from: 13.51, to: 16, speaking: false)
        XCTAssertEqual(sent.map(\.text), ["¿qué dijo Lucía del presupuesto?"])
    }

    func testAQuestionThatNeverPausesIsSentAtThirtySeconds() {
        var policy = SpokenQuestionPolicy()
        _ = levels(&policy, from: 0, to: 1, speaking: true)
        XCTAssertNil(policy.heard(volatile("Oye Deimos, resume lo que", 0, 1)))
        XCTAssertNil(policy.heard(volatile("Oye Deimos, resume lo que dicen todos", 0, 20)))
        let sent = levels(&policy, from: 1, to: 40, speaking: true)
        XCTAssertEqual(sent.count, 1)
        XCTAssertEqual(sent.first?.text, "resume lo que dicen todos")
        XCTAssertEqual(sent.first?.end ?? 0, 31, accuracy: 0.02, "thirty seconds after the phrase was heard")
    }

    func testThePhraseWithNothingAfterItIsForgotten() {
        var policy = SpokenQuestionPolicy()
        XCTAssertNil(policy.heard(final("Oye, Demos.", 0, 1)))
        XCTAssertTrue(levels(&policy, from: 1, to: 8, speaking: false).isEmpty)
        XCTAssertFalse(policy.listening, "four pauses without a question")
        XCTAssertNil(policy.heard(final("Seguimos con el orden del día.", 8, 10)))
        XCTAssertTrue(levels(&policy, from: 10, to: 14, speaking: false).isEmpty)
    }

    func testWordsThatStopOpeningWithThePhraseCancelIt() {
        var policy = SpokenQuestionPolicy()
        XCTAssertNil(policy.heard(volatile("Oye, Demos,", 0, 1)))
        XCTAssertTrue(policy.listening)
        XCTAssertNil(policy.heard(final("Oye, demos por cerrado esto.", 0, 2.5)))
        XCTAssertFalse(policy.listening)
        XCTAssertTrue(levels(&policy, from: 2.5, to: 6, speaking: false).isEmpty)
    }

    func testTheSameWordsNeverAskTwice() {
        var policy = SpokenQuestionPolicy()
        XCTAssertNil(policy.heard(volatile("Deimos, resume lo último.", 0, 2)))
        _ = levels(&policy, from: 0, to: 2, speaking: true)
        // No final result: it leaves after the pause and the catch-up wait.
        XCTAssertEqual(levels(&policy, from: 2.01, to: 7, speaking: false).count, 1)
        // The final result for the same audio arrives late.
        XCTAssertNil(policy.heard(final("Deimos, resume lo último.", 0, 2.1)))
        XCTAssertFalse(policy.listening)
        XCTAssertTrue(levels(&policy, from: 7, to: 8, speaking: false).isEmpty)
        // A new phrase later asks again.
        XCTAssertNil(policy.heard(final("Oye Deimos, ¿y ahora?", 8, 9)))
        XCTAssertEqual(levels(&policy, from: 9, to: 11, speaking: false).map(\.text), ["¿y ahora?"])
    }

    /// One phrase asked twice (seen on a Mac). The question leaves with its final
    /// result; another final result for the same audio, later and with its
    /// range running on through the pause, never asks again.
    func testALateFinalResultThatRunsPastTheQuestionNeverAsksAgain() {
        var policy = SpokenQuestionPolicy()
        _ = levels(&policy, from: 0, to: 2.5, speaking: true)
        XCTAssertNil(policy.heard(volatile("Oye, Demos, ¿de qué estamos hablando?", 0.4, 2.5)))
        XCTAssertTrue(levels(&policy, from: 2.51, to: 4.5, speaking: false).isEmpty)
        XCTAssertEqual(policy.heard(final("Oye Deimos, ¿de qué estamos hablando?", 0.4, 2.5))?.text,
                       "¿de qué estamos hablando?")
        // The analyzer's next final result for the same audio, ending after the send.
        XCTAssertNil(policy.heard(final("Oye Deimos, ¿de qué estamos hablando?", 0.4, 4.8)))
        XCTAssertFalse(policy.listening)
        XCTAssertTrue(levels(&policy, from: 4.51, to: 12, speaking: false).isEmpty)
        // A phrase said after the question asks again.
        _ = levels(&policy, from: 12, to: 13, speaking: true)
        XCTAssertNil(policy.heard(final("Oye Deimos, ¿y ahora?", 12, 13)))
        XCTAssertEqual(levels(&policy, from: 13.01, to: 15, speaking: false).map(\.text), ["¿y ahora?"])
    }

    /// Wake-phrase trial runs 21 and 22 on a Mac: the pause after the voice came before the
    /// question's words, and it left as «de qué estamos» or «de qué». A
    /// volatile result's range covers the whole voice while its words are
    /// still cut: the question waits for the final result.
    func testTheQuestionWaitsForTheFinalResultOfItsWords() {
        var policy = SpokenQuestionPolicy()
        _ = levels(&policy, from: 0, to: 2.6, speaking: true)
        XCTAssertNil(policy.heard(volatile("Oye, Demos, ¿de qué", 0.4, 2.6)))
        XCTAssertTrue(levels(&policy, from: 2.61, to: 4.5, speaking: false).isEmpty,
                      "paused, but only volatile words have come")
        let spoken = policy.heard(final("Oye, Demos, ¿de qué estamos hablando?", 0.4, 2.6))
        XCTAssertEqual(spoken?.text, "¿de qué estamos hablando?")
        XCTAssertFalse(policy.listening)
    }

    /// The pause asks the transcription to finalize the question's words,
    /// once; the final result then sends the question.
    func testAPausedQuestionAsksTheTranscriptionToFinalizeItsWordsOnce() {
        var policy = SpokenQuestionPolicy()
        _ = levels(&policy, from: 0, to: 2.6, speaking: true)
        XCTAssertNil(policy.heard(volatile("Oye, Demos, ¿de qué estamos", 0.4, 2.6)))
        XCTAssertNil(policy.finalizationDue(), "still speaking")
        var asked = [Double]()
        var time = 2.61
        while time <= 4.6 {
            XCTAssertNil(policy.voice(at: time, speaking: false))
            if let through = policy.finalizationDue() { asked.append(through) }
            time += 0.01
        }
        XCTAssertEqual(asked.count, 1, "once per question")
        XCTAssertEqual(asked.first ?? 0, 4.1, accuracy: 0.02, "at the pause, through all the audio fed")
        XCTAssertEqual(policy.heard(final("Oye, Demos, ¿de qué estamos hablando?", 0.4, 2.7))?.text,
                       "¿de qué estamos hablando?")
        XCTAssertNil(policy.finalizationDue(), "nothing is listened to any more")
    }

    /// A final result that never comes does not hold the question for ever:
    /// after the catch-up wait it leaves with what was heard.
    func testAQuestionWhoseFinalResultNeverComesLeavesAfterTheCatchUpWait() {
        var policy = SpokenQuestionPolicy()
        _ = levels(&policy, from: 0, to: 2.6, speaking: true)
        XCTAssertNil(policy.heard(volatile("Oye, Demos, ¿de qué estamos", 0.4, 2.6)))
        let wait = policy.pauseSeconds + SpokenQuestionPolicy.catchUpSeconds
        XCTAssertTrue(levels(&policy, from: 2.61, to: 2.6 + wait - 0.02, speaking: false).isEmpty)
        let sent = levels(&policy, from: 2.6 + wait, to: 2.6 + wait + 1, speaking: false)
        XCTAssertEqual(sent.map(\.text), ["¿de qué estamos"])
    }

    /// The steps the helper reports to the daemon's log: the phrase
    /// heard, the phrase with no question after it, and nothing when the
    /// question itself is asked (the question says itself).
    func testStepsSayThePhraseHeardAndThePhraseAbandonedButNotAnAskedQuestion() {
        var policy = SpokenQuestionPolicy()
        XCTAssertNil(policy.heard(final("Oye, Demos.", 0, 1)))
        XCTAssertEqual(policy.step, .heard)
        XCTAssertNil(policy.voice(at: 8, speaking: false))
        XCTAssertEqual(policy.step, .abandoned(.init(cause: .questionWaitExpired,
                                                    elapsed_ms: 7_000, silent_ms: 7_000)))
        XCTAssertNil(policy.voice(at: 9, speaking: false))
        XCTAssertNil(policy.step, "the abandonment is reported once")

        XCTAssertNil(policy.heard(final("Oye Deimos, ¿y ahora?", 10, 11)))
        XCTAssertTrue(policy.listening)
        XCTAssertEqual(policy.step, .heard)
        XCTAssertEqual(policy.voice(at: 13, speaking: false)?.text, "¿y ahora?")
        XCTAssertNil(policy.step, "a sent question says itself")
    }

    func testIdleResultsAreBounded() {
        var policy = SpokenQuestionPolicy()
        for index in 0..<500 {
            let start = Double(index)
            XCTAssertNil(policy.heard(final("Seguimos con el punto \(index).", start, start + 0.9)))
        }
        XCTAssertFalse(policy.listening)
        XCTAssertNil(policy.heard(final("Oye Deimos, ¿quién habló último?", 500, 501)))
        XCTAssertEqual(levels(&policy, from: 501, to: 503, speaking: false).map(\.text),
                       ["¿quién habló último?"])
    }
}
