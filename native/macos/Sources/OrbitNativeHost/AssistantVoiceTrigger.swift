import Foundation
import OrbitNativeProtocol
import OrbitNativeTransport

/// Receives the meeting microphone's PCM (16-bit mono) on the relay's drain
/// queue, after the ring and outside the audio callback: the same samples the
/// recording sends, never a second microphone stream.
package protocol MicrophoneFrameSink: AnyObject {
    func receive(pcm16: Data, sampleRate: UInt32, rmsDBFS: Double)
    /// The stream ended; nothing more will arrive.
    func finish()
}

/// A phrase listener the host can stop at once (privacy stop, audio_stop).
protocol PhraseListening: MicrophoneFrameSink {
    func begin()
    func stop()
}

/// What the detector says: about itself (`voice_trigger`), or a step of the
/// phrase for the daemon's log (`notice`), so a lost question can be followed.
enum VoiceTriggerReport: Equatable {
    case unavailable(cause: String, osVersion: String?)
    case idle
    case downloading(percent: UInt8?)
    case listening
    case failed(cause: String, detail: String?)
    case step(WakePhraseStep)

    var event: NativeEvent {
        switch self {
        case .unavailable(let cause, let osVersion):
            return .makeVoiceTrigger(state: "unavailable", cause: cause, osVersion: osVersion)
        case .idle:
            return .makeVoiceTrigger(state: "idle")
        case .downloading(let percent):
            return .makeVoiceTrigger(state: "downloading", percent: percent)
        case .listening:
            return .makeVoiceTrigger(state: "listening")
        case .failed(let cause, let detail):
            return .makeVoiceTrigger(state: "failed", cause: cause, detail: detail)
        case .step(let step):
            if case .abandoned(let abandonment) = step {
                return .makeNotice(code: step.code, abandonment: abandonment)
            }
            return .makeNotice(code: step.code)
        }
    }
}

/// «Oye Deimos» on the meeting's microphone. It runs only while a
/// meeting recording captures the user's microphone (the daemon's
/// `audio_wake_phrase`), with the user's stop off; it is fed from that same stream; and
/// nothing it hears leaves the Mac except the question that follows the
/// phrase, as text. On macOS before 26 it says it is unavailable, with the
/// system's version, and «Preguntar a Deimos» still works.
///
/// Used from the main thread only (the application's command and stop
/// paths); the listener it owns is thread-safe on its own.
package final class AssistantVoiceTrigger {
    /// Builds a listener from its report and question callbacks. Production
    /// uses macOS 26's speech analysis; tests pass a fake.
    typealias ListenerFactory = (
        _ assistantNames: [String],
        _ locale: NativeSpeechLocale,
        _ report: @escaping (VoiceTriggerReport) -> Void,
        _ question: @escaping (SpokenQuestion, Double) -> Void
    ) -> PhraseListening

    private let channel: NativeChannel
    private let makeListener: ListenerFactory?
    private var listener: PhraseListening?
    private var generation: UInt64 = 0
    /// Every agent's name, as the daemon last gave them.
    private var assistantNames = ["Deimos"]
    private var locale: NativeSpeechLocale = .spanish

    package init(channel: NativeChannel) {
        self.channel = channel
        self.makeListener = nil
    }

    init(channel: NativeChannel, makeListener: @escaping ListenerFactory) {
        self.channel = channel
        self.makeListener = makeListener
    }

    /// Whether a listener is on (tests and diagnostics).
    package var isListening: Bool { listener != nil }

    /// The macOS version, said with the cause when the detector cannot run.
    static var osVersion: String {
        let version = ProcessInfo.processInfo.operatingSystemVersion
        return "\(version.majorVersion).\(version.minorVersion).\(version.patchVersion)"
    }

    /// After `ready`: whether this Mac can listen for the phrase.
    package func announce() {
        if #available(macOS 26.0, *) {
            channel.send(VoiceTriggerReport.idle.event)
        } else {
            channel.send(VoiceTriggerReport.unavailable(cause: "macos_version",
                osVersion: Self.osVersion).event)
        }
    }

    /// Starts listening on the meeting microphone `streamID` for every name
    /// in `assistantNames` (or the one `assistantName`); without either, for
    /// the names it last had. `allow` is asked on the main actor before a
    /// question leaves: the user's stop off and the stream still the active
    /// microphone.
    package func start(streamID: String, assistantName: String? = nil,
                       assistantNames: [String]? = nil, locale: NativeSpeechLocale? = nil,
                       allow: @escaping @MainActor (String) -> Bool) -> MicrophoneFrameSink? {
        stop()
        if let locale { self.locale = locale }
        if let assistantNames, !assistantNames.isEmpty {
            self.assistantNames = assistantNames
        } else if let assistantName {
            self.assistantNames = [assistantName]
        }
        let channel = self.channel
        let generation = self.generation
        let report: (VoiceTriggerReport) -> Void = { [weak self] report in
            DispatchQueue.main.async {
                guard let self, self.generation == generation else { return }
                channel.send(report.event)
            }
        }
        let question: (SpokenQuestion, Double) -> Void = { spoken, fedSeconds in
            let now = Int64((Date().timeIntervalSince1970 * 1_000).rounded())
            let ended = now - Int64(((fedSeconds - spoken.end) * 1_000).rounded())
            let started = now - Int64(((fedSeconds - spoken.start) * 1_000).rounded())
            Task { @MainActor in
                AssistantVoiceTrigger.deliver(.makeAssistantQuestion(streamID: streamID, text: spoken.text,
                    startedMS: started, endedMS: ended, assistantName: spoken.name),
                    streamID: streamID, allow: allow, channel: channel)
            }
        }
        let listener: PhraseListening
        if let makeListener {
            listener = makeListener(self.assistantNames, self.locale, report, question)
        } else if #available(macOS 26.0, *) {
            listener = SpeechPhraseListener(assistantNames: self.assistantNames, locale: self.locale, report: report, question: question)
        } else {
            announce()
            return nil
        }
        self.listener = listener
        listener.begin()
        return listener
    }

    /// Sends a heard question if `allow` still lets it leave; otherwise says
    /// only that it was dropped, never its text.
    @MainActor static func deliver(_ question: NativeEvent, streamID: String,
                                   allow: @MainActor (String) -> Bool,
                                   channel: NativeChannel) {
        if allow(streamID) {
            channel.send(question)
        } else {
            channel.send(VoiceTriggerReport.step(.questionDropped).event)
        }
    }

    /// Stops listening now: the recording stopped, paused, or the user's stop.
    package func stop() {
        generation &+= 1
        guard let listener else { return }
        listener.stop()
        self.listener = nil
        channel.send(VoiceTriggerReport.idle.event)
    }
}
