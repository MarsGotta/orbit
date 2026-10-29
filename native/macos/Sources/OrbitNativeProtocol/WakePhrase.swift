import Foundation

/// «Oye {name}» in Spanish and «Hey {name}» in English, followed by a
/// question in the same locale. Spanish retains its measured aliases.
/// Each assistant has its own name; the greeting alone opens
/// nothing («oye, una cosa…» is said in any meeting). The product decision
/// (2026-10-15): «Oye Deimos», also «Hola Deimos» and «Deimos,» at the start
/// of a phrase. macOS's on-device
/// Spanish transcription writes «Deimos» as «Demos» every time
/// so «Demos» is accepted too, but only at
/// the start of a phrase and followed by a pause or a comma: «Oye, demos
/// por cerrado…» never opens a question. The small risk of «oye, demos, …»
/// was accepted.
///
/// Pure: it reads transcribed text, never audio. Which microphone it is fed
/// from and when it listens is the host's decision.
public enum WakePhrase {
    /// The name as the transcription writes it.
    static let heardName = "demos"
    /// Marks after which a new phrase starts.
    static let terminators: Set<Character> = [".", "?", "!", "…"]
    /// Marks that may open a phrase without ending the start of it.
    static let openers: Set<Character> = ["¿", "¡", "«", "\"", "“", "-", "—"]
    /// Marks that stand for the pause after the name.
    static let pauses: Set<Character> = [",", ".", "…", ":", ";", "!", "?"]

    private enum Token {
        case word(String, Range<String.Index>)
        case mark(Character, Range<String.Index>)

        var value: String {
            switch self {
            case .word(let word, _): return word
            case .mark(let mark, _): return String(mark)
            }
        }

        var range: Range<String.Index> {
            switch self {
            case .word(_, let range), .mark(_, let range): return range
            }
        }
    }

    /// What follows the phrase in `text`, when a phrase of `text` starts with
    /// it: the question, possibly still empty (the name was said and the
    /// question has not come yet). `nil` when no phrase starts with it.
    public static func question(after text: String, assistantName: String = "Deimos",
                                locale: NativeSpeechLocale = .spanish) -> String? {
        let tokens = tokenize(text)
        let name = tokenize(assistantName)
        guard !name.isEmpty else { return nil }
        let acceptsHeardName = locale == .spanish && fold(assistantName.trimmingCharacters(in: .whitespacesAndNewlines)) == "deimos"
        var phraseStart = true
        for position in tokens.indices {
            if phraseStart, let end = phraseEnd(tokens, from: position, name: name, acceptsHeardName: acceptsHeardName, locale: locale) {
                let from = end < tokens.count ? tokens[end].range.lowerBound : text.endIndex
                return String(text[from...]).trimmingCharacters(in: .whitespacesAndNewlines)
            }
            switch tokens[position] {
            case .mark(let mark, _):
                if terminators.contains(mark) {
                    phraseStart = true
                } else if !openers.contains(mark) {
                    phraseStart = false
                }
            case .word:
                phraseStart = false
            }
        }
        return nil
    }

    /// The configured name that opens a phrase of `text`, and the question
    /// after it. There is one name per agent (hub-v1 «Agents»), each heard
    /// by the rules above. A name of more words is tried before a shorter
    /// one, so «Oye Atlas Azul, …» is Atlas Azul's and not Atlas's.
    public static func match(after text: String, assistantNames: [String],
                             locale: NativeSpeechLocale = .spanish) -> WakePhraseMatch? {
        let ordered = assistantNames.enumerated().sorted { lhs, rhs in
            let left = tokenize(lhs.element).count
            let right = tokenize(rhs.element).count
            return left != right ? left > right : lhs.offset < rhs.offset
        }
        for entry in ordered {
            if let heard = question(after: text, assistantName: entry.element, locale: locale) {
                return WakePhraseMatch(name: entry.element, question: heard)
            }
        }
        return nil
    }

    /// What follows «Oye {nombre}» or «Hola {nombre}» anywhere in `text`: how
    /// a phrase already heard reads once the transcription joined the words
    /// said before it into its result and dropped the mark between them
    /// («Bueno. Oye Deimos, …» rewritten as «bueno oye Deimos, …»). Only the
    /// greeted phrase: a bare name in the middle of a sentence calls nobody.
    /// Never for a phrase not heard yet: by itself, «Le pregunté: oye Ares, …»
    /// opens nothing.
    public static func question(joinedTo text: String, assistantName: String = "Deimos",
                                locale: NativeSpeechLocale = .spanish) -> String? {
        let tokens = tokenize(text)
        let name = tokenize(assistantName)
        guard !name.isEmpty else { return nil }
        let acceptsHeardName = locale == .spanish && fold(assistantName.trimmingCharacters(in: .whitespacesAndNewlines)) == "deimos"
        for position in tokens.indices {
            guard case .word(let word, _) = tokens[position], locale.greetings.contains(word) else { continue }
            var index = position + 1
            if index < tokens.count, case .mark(",", _) = tokens[index] { index += 1 }
            if let end = nameEnd(tokens, from: index, name: name,
                                 acceptsHeardName: acceptsHeardName, greeted: true) {
                let from = end < tokens.count ? tokens[end].range.lowerBound : text.endIndex
                return String(text[from...]).trimmingCharacters(in: .whitespacesAndNewlines)
            }
        }
        return nil
    }

    /// Where the question starts when the phrase starts at `start`.
    private static func phraseEnd(_ tokens: [Token], from start: Int,
                                  name: [Token], acceptsHeardName: Bool, locale: NativeSpeechLocale) -> Int? {
        if case .word(let word, _) = tokens[start], locale.greetings.contains(word) {
            var index = start + 1
            if index < tokens.count, case .mark(",", _) = tokens[index] { index += 1 }
            if let end = nameEnd(tokens, from: index, name: name,
                                 acceptsHeardName: acceptsHeardName, greeted: true) { return end }
        }
        return nameEnd(tokens, from: start, name: name,
                       acceptsHeardName: acceptsHeardName, greeted: false)
    }

    private static func nameEnd(_ tokens: [Token], from start: Int, name: [Token],
                                acceptsHeardName: Bool, greeted: Bool) -> Int? {
        var index = start
        let spelled = index + name.count <= tokens.count && name.indices.allSatisfy {
            tokens[index + $0].value == name[$0].value
        }
        if spelled {
            index += name.count
        } else if acceptsHeardName, index < tokens.count,
                  case .word(let word, _) = tokens[index], word == heardName {
            index += 1
        } else {
            return nil
        }
        // The transcription ended the phrase here: that is the pause.
        guard index < tokens.count else { return index }
        if case .mark(let mark, _) = tokens[index], pauses.contains(mark) {
            index += 1
            while index < tokens.count, case .mark(let next, _) = tokens[index], pauses.contains(next) {
                index += 1
            }
            return index
        }
        // Only the spelled name after a greeting may run straight into the
        // question («Oye Deimos qué…»); «Demos» needs its pause.
        return greeted && spelled ? index : nil
    }

    private static func tokenize(_ text: String) -> [Token] {
        var tokens = [Token]()
        var index = text.startIndex
        while index < text.endIndex {
            let character = text[index]
            if character.isLetter || character.isNumber {
                var end = index
                while end < text.endIndex,
                      text[end].isLetter || text[end].isNumber || text[end] == "'" {
                    end = text.index(after: end)
                }
                tokens.append(.word(fold(String(text[index..<end])), index..<end))
                index = end
            } else if character.isWhitespace {
                index = text.index(after: index)
            } else {
                let next = text.index(after: index)
                tokens.append(.mark(character, index..<next))
                index = next
            }
        }
        return tokens
    }

    /// Case and accents aside, and «ph» as «f»: Spanish writes Phobos
    /// «Fobos», so the transcription may write either for the same name.
    /// Not measured on a Mac; it only makes the two spellings one name.
    static func fold(_ word: String) -> String {
        word.folding(options: [.caseInsensitive, .diacriticInsensitive],
                     locale: Locale(identifier: "en_US_POSIX"))
            .replacingOccurrences(of: "ph", with: "f")
    }
}

/// The name that opened a phrase, as it was configured, and what follows it.
public struct WakePhraseMatch: Equatable, Sendable {
    public let name: String
    public let question: String

    public init(name: String, question: String) {
        self.name = name
        self.question = question
    }
}

/// One transcription result: its text and where it sits on the audio
/// timeline (seconds from the first sample the listener was fed). A volatile
/// result is replaced by later ones for the same audio; a final one stays.
public struct HeardSegment: Equatable, Sendable {
    public let text: String
    public let start: Double
    public let end: Double
    public let isFinal: Bool

    public init(text: String, start: Double, end: Double, isFinal: Bool) {
        self.text = text
        self.start = start
        self.end = end
        self.isFinal = isFinal
    }
}

/// A step of «Oye Deimos» the helper reports to the daemon's log
/// (`notice`), so a question lost on the way can be followed: the phrase
/// was heard, it was heard and no question followed, or a question was heard
/// and not sent. A question sent says itself (`assistant_question`). Its
/// code goes on the wire; nothing heard rides with it.
public enum WakePhraseStep: Equatable, Sendable {
    case heard
    case abandoned(WakePhraseAbandonment)
    case questionDropped

    public var code: String {
        switch self {
        case .heard: return "wake_phrase_heard"
        case .abandoned: return "wake_phrase_abandoned"
        case .questionDropped: return "wake_phrase_question_dropped"
        }
    }
}

public enum WakePhraseAbandonmentCause: String, Encodable, Sendable {
    case phraseRevised = "phrase_revised"
    case questionWaitExpired = "question_wait_expired"
    case questionTooLong = "question_too_long"
}

/// Only the reason and audio durations travel; never recognized words.
public struct WakePhraseAbandonment: Encodable, Equatable, Sendable {
    public let cause: WakePhraseAbandonmentCause
    public let elapsed_ms: UInt64
    public let silent_ms: UInt64

    public init(cause: WakePhraseAbandonmentCause, elapsed_ms: UInt64, silent_ms: UInt64) {
        self.cause = cause
        self.elapsed_ms = elapsed_ms
        self.silent_ms = silent_ms
    }
}

/// A question heard after the phrase: its text, its span on the audio
/// timeline, from the phrase's start to the end of the question, and the
/// configured name that opened it.
public struct SpokenQuestion: Equatable, Sendable {
    public let text: String
    public let start: Double
    public let end: Double
    public let name: String

    public init(text: String, start: Double, end: Double, name: String = "Deimos") {
        self.text = text
        self.start = start
        self.end = end
        self.name = name
    }
}

/// From transcription results and the microphone's level to one
/// spoken question. Idle, it looks for the phrase at the start of a phrase
/// of any result; once heard, the question is what follows, until
/// `pauseSeconds` without voice or `maximumSeconds` from the phrase. The pause
/// is measured on the microphone's level, which runs ahead of the
/// transcription, so the question also waits for a final result that reaches
/// the end of the voice (at most `catchUpSeconds` more): a volatile result's
/// range already covers audio whose words have not come yet. A result that starts before
/// the end of a question it already sent is the same words again and is
/// ignored, so they never ask twice. Pure and deterministic: times are passed
/// in.
public struct SpokenQuestionPolicy {
    /// Silence that ends the question (`NativeLimits.assistantQuestionPauseMilliseconds`).
    public let pauseSeconds: Double
    /// The longest question (`NativeLimits.maximumAssistantQuestionMilliseconds`).
    public let maximumSeconds: Double
    /// After the phrase with nothing following it, how long the policy waits
    /// for the question before forgetting the phrase: the user may pause after
    /// «Oye Deimos». Four pauses; it is not a recognition timeout.
    public var waitSeconds: Double { pauseSeconds * 4 }
    /// Results kept while idle: the newest ones, for the phrase. Bounds the
    /// policy's memory over an hour-long meeting; a phrase is in the last few.
    public static let maximumSegments = 64
    /// After the pause, how much longer the question waits for the final
    /// result of its words. Measured on a Mac (runs 21 and 22 of the
    /// wake-phrase trials): when the pause ended only
    /// volatile results had come, whose range covered the whole voice but
    /// whose words stopped at «de qué estamos» and «de qué»; in earlier runs
    /// the late final result then asked the same question again. Twice the
    /// pause: it bounds a wait for a result that may never come, and never
    /// cuts a question whose final result has arrived.
    public static let catchUpSeconds = 3.0
    /// How far the end of the final result may fall short of the end of the
    /// voice and still count as reached: the level stays above the threshold
    /// for the fade after the last word. Not a recognition window.
    public static let catchUpToleranceSeconds = 0.3

    private var segments = [HeardSegment]()
    /// Where the result with the phrase starts, when it was heard, and the
    /// name that opened it.
    private var phrase: (segmentStart: Double, heardAt: Double, name: String)?
    private var lastVoice = 0.0
    /// Where the final results have reached on the audio timeline: the end
    /// of the newest final result with words.
    private var finalThrough = -Double.infinity
    /// The transcription was asked to finalize this question's words.
    private var finalizeRequested = false
    private var now = 0.0
    /// Results that start before this were already used: a question sent,
    /// or a phrase forgotten with nothing after it.
    private var ignoreBefore = -Double.infinity

    private let assistantNames: [String]
    private let locale: NativeSpeechLocale

    public init(
        assistantName: String = "Deimos",
        locale: NativeSpeechLocale = .spanish,
        pauseSeconds: Double = Double(NativeLimits.assistantQuestionPauseMilliseconds) / 1_000,
        maximumSeconds: Double = Double(NativeLimits.maximumAssistantQuestionMilliseconds) / 1_000
    ) {
        self.init(assistantNames: [assistantName], locale: locale, pauseSeconds: pauseSeconds,
                  maximumSeconds: maximumSeconds)
    }

    /// Listens for every agent's name; the question says which one was heard.
    public init(
        assistantNames: [String],
        locale: NativeSpeechLocale = .spanish,
        pauseSeconds: Double = Double(NativeLimits.assistantQuestionPauseMilliseconds) / 1_000,
        maximumSeconds: Double = Double(NativeLimits.maximumAssistantQuestionMilliseconds) / 1_000
    ) {
        self.assistantNames = assistantNames
        self.locale = locale
        self.pauseSeconds = pauseSeconds
        self.maximumSeconds = maximumSeconds
    }

    /// Whether the phrase was heard and the question is being listened to.
    public var listening: Bool { phrase != nil }
    /// The latest `heard` or `voice` call's step, absent when nothing changed.
    public private(set) var step: WakePhraseStep?

    /// A transcription result.
    public mutating func heard(_ segment: HeardSegment) -> SpokenQuestion? {
        step = nil
        // By its start, not its end: the final result for a question arrives
        // after the question was sent, and its range can run on through the
        // pause that ended it. Taken by its end, it opened the same question
        // again (one phrase asked twice in 2 of 4 runs on a Mac).
        guard segment.start >= ignoreBefore else { return nil }
        segments.removeAll { known in
            let replaced = !known.isFinal && known.start >= segment.start - 0.05
            let covered = segment.isFinal && known.start < segment.end && segment.start < known.end
            return replaced || covered
        }
        segments.append(segment)
        segments.sort { $0.start < $1.start }
        if segments.count > Self.maximumSegments {
            segments.removeFirst(segments.count - Self.maximumSegments)
        }
        now = max(now, segment.end)
        if !segment.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            lastVoice = max(lastVoice, segment.end)
            if segment.isFinal { finalThrough = max(finalThrough, segment.end) }
        }
        if phrase == nil {
            var found: (segment: HeardSegment, name: String)?
            for known in segments {
                if let match = WakePhrase.match(after: known.text, assistantNames: assistantNames, locale: locale) {
                    found = (known, match.name)
                    break
                }
            }
            if let found {
                // The ceiling counts from when the phrase was heard, so a
                // long result that ends with it never cuts the question.
                phrase = (found.segment.start, now, found.name)
                step = .heard
                lastVoice = max(lastVoice, found.segment.end)
            }
        } else if currentQuestion() == nil {
            if let phrase, segment.isFinal, segment.start < phrase.heardAt,
               WakePhrase.match(after: segment.text, assistantNames: assistantNames,
                                locale: locale)?.name == phrase.name {
                // The final result replaced the phrase's volatile ones but
                // starts later, still opening with the phrase: the English
                // recognizer trims the silence before «Hey» (real test,
                // 2026-10-29: volatile results from 56.58 s, the final one
                // «Hey, Deimos, …» from 57.30 s). The phrase follows it.
                self.phrase = (segment.start, phrase.heardAt, phrase.name)
            } else {
                // The words changed and no longer open with the phrase
                // («Oye, Demos» became «Oye, demos por cerrado…»).
                abandon(.phraseRevised)
                reset(ignoring: ignoreBefore)
            }
        }
        let spoken = decide()
        if spoken != nil { step = nil }
        return spoken
    }

    /// The microphone at `time` on the audio timeline: whether it carries
    /// voice. The pause is measured on the audio, which keeps advancing when
    /// the transcription has nothing new to say.
    public mutating func voice(at time: Double, speaking: Bool) -> SpokenQuestion? {
        step = nil
        now = max(now, time)
        if speaking { lastVoice = max(lastVoice, time) }
        return decide()
    }

    /// Where the transcription should finalize its results now, once per
    /// question: the pause has come and only volatile words hold the
    /// question. macOS's transcriber keeps the last word of a phrase volatile
    /// while only silence follows (wake-phrase trial runs 25 to 27: «hablando» never
    /// came in the 3 s of catch-up), so the listener asks it to finalize
    /// through `now`, all the audio fed so far.
    public mutating func finalizationDue() -> Double? {
        guard phrase != nil, !finalizeRequested,
              !(currentQuestion() ?? "").isEmpty,
              now - lastVoice >= pauseSeconds,
              finalThrough < lastVoice - Self.catchUpToleranceSeconds else { return nil }
        finalizeRequested = true
        return now
    }

    /// The question so far, `nil` when the phrase is no longer there.
    private func currentQuestion() -> String? {
        guard let phrase else { return nil }
        let covering = segments.last(where: {
            $0.start <= phrase.segmentStart + 0.05 && $0.end > phrase.segmentStart
        })
        let opening = covering ?? segments.first(where: {
            abs($0.start - phrase.segmentStart) < 0.05
        })
        guard let opening else { return nil }
        var first = WakePhrase.question(after: opening.text, assistantName: phrase.name, locale: locale)
        if first == nil, opening.start < phrase.segmentStart - 0.05 {
            // The result now starts earlier: the transcription joined words
            // said before the phrase and dropped the mark between them. The
            // phrase's own audio is the same, so it is still the phrase.
            first = WakePhrase.question(joinedTo: opening.text, assistantName: phrase.name, locale: locale)
        }
        guard let first else { return nil }
        let rest = segments.filter({ $0.start > opening.start + 0.05 })
            .map({ $0.text.trimmingCharacters(in: .whitespacesAndNewlines) })
            .filter({ !$0.isEmpty })
        return ([first] + rest).filter { !$0.isEmpty }.joined(separator: " ")
    }

    private mutating func decide() -> SpokenQuestion? {
        guard let phrase else {
            // Idle: forget what is older than one question could be.
            let cutoff = now - maximumSeconds
            segments.removeAll { $0.end < cutoff }
            return nil
        }
        let question = currentQuestion() ?? ""
        let silent = now - lastVoice
        let elapsed = now - phrase.heardAt
        if question.isEmpty {
            if silent >= waitSeconds || elapsed >= maximumSeconds {
                abandon(silent >= waitSeconds ? .questionWaitExpired : .questionTooLong)
                reset(ignoring: now)
            }
            return nil
        }
        let reached = finalThrough >= lastVoice - Self.catchUpToleranceSeconds
        guard (silent >= pauseSeconds && reached)
                || silent >= pauseSeconds + Self.catchUpSeconds
                || elapsed >= maximumSeconds else { return nil }
        let spoken = SpokenQuestion(text: question, start: phrase.segmentStart,
                                    end: min(lastVoice, now), name: phrase.name)
        reset(ignoring: spoken.end)
        return spoken
    }

    private mutating func abandon(_ cause: WakePhraseAbandonmentCause) {
        guard let phrase else { return }
        step = .abandoned(.init(cause: cause,
            elapsed_ms: UInt64((max(0, now - phrase.heardAt) * 1_000).rounded()),
            silent_ms: UInt64((max(0, now - lastVoice) * 1_000).rounded())))
    }

    private mutating func reset(ignoring time: Double) {
        phrase = nil
        finalizeRequested = false
        segments.removeAll()
        ignoreBefore = time
    }
}
