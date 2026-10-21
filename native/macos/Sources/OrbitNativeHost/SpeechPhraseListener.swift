import AVFoundation
import CoreMedia
import Foundation
import OrbitNativeProtocol
import Speech

/// P9-12: on-device recognition of «Oye Deimos» with macOS 26's
/// `SpeechAnalyzer` + `SpeechTranscriber` (Spanish, `es_ES`), fed with the
/// meeting microphone's samples. Measured on Mars's Mac
///: the Spanish model downloads once through
/// `AssetInventory`, analysis runs on the Mac without network, and analysing
/// audio Orbit already has asked for no permission. The live path (this
/// file) is not measured yet: latency, CPU over an hour and any prompt are
/// the reviewer's measurements in that evidence file.
///
/// Threads: `receive` runs on the relay's drain queue, results on the
/// analyzer's task, `stop` on the main actor; one lock guards the state.
/// Nothing heard is logged or sent except the question (`SpokenQuestion`).
@available(macOS 26.0, *)
final class SpeechPhraseListener: PhraseListening, @unchecked Sendable {
    /// Audio blocks waiting for the analyzer. Protects the helper's memory if
    /// the analyzer stalls: the newest are kept and older ones dropped,
    /// which can only lose a phrase, never the recording (the recording's
    /// samples went to the daemon before this). About five seconds of 10 ms
    /// blocks; not a recognition window.
    static let maximumPendingBuffers = 512
    /// Voice above this level (RMS, dBFS) keeps a question open; below it
    /// the 1.5 s pause runs. Speech on a laptop microphone sits well above
    /// it and a quiet room well below; unmeasured with Mars's voice, see the
    /// evidence file. It is not a recording threshold.
    static let speechLevelDBFS = -45.0
    /// The detector reports the model download at most this often.
    static let progressInterval: Duration = .milliseconds(500)

    private let lock = NSLock()
    private var policy: SpokenQuestionPolicy
    private var fedSeconds = 0.0
    private var analyzerOrigin: Double?
    private var continuation: AsyncStream<AnalyzerInput>.Continuation?
    private var analyzerFormat: AVAudioFormat?
    private var converter: AVAudioConverter?
    private var converterInput: AVAudioFormat?
    private var analyzer: SpeechAnalyzer?
    private var setup: Task<Void, Never>?
    private var results: Task<Void, Never>?
    private var stopped = false
    private let report: (VoiceTriggerReport) -> Void
    private let question: (SpokenQuestion, Double) -> Void

    init(assistantName: String, report: @escaping (VoiceTriggerReport) -> Void,
         question: @escaping (SpokenQuestion, Double) -> Void) {
        self.policy = SpokenQuestionPolicy(assistantName: assistantName)
        self.report = report
        self.question = question
    }

    func begin() {
        let task = Task { [weak self] in
            guard let self else { return }
            await self.prepare()
        }
        lock.withLock { setup = task }
    }

    private var isStopped: Bool { lock.withLock { stopped } }

    /// Reports unless stopped: a late report never says «listening» after
    /// the recording ended.
    private func say(_ value: VoiceTriggerReport) {
        guard !isStopped else { return }
        report(value)
    }

    private func prepare() async {
        guard let locale = await SpeechTranscriber.supportedLocale(
            equivalentTo: Locale(identifier: "es_ES")) else {
            say(.unavailable(cause: "locale_unsupported", osVersion: nil))
            return
        }
        let transcriber = SpeechTranscriber(locale: locale, transcriptionOptions: [],
            reportingOptions: [.volatileResults], attributeOptions: [.audioTimeRange])
        do {
            if let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
                say(.downloading(percent: 0))
                let progress = request.progress
                let watcher = Task { [weak self] in
                    var last: UInt8 = 0
                    while !Task.isCancelled {
                        try? await Task.sleep(for: Self.progressInterval)
                        let fraction = min(1, max(0, progress.fractionCompleted))
                        let percent = UInt8((fraction * 100).rounded())
                        if percent != last {
                            last = percent
                            self?.say(.downloading(percent: percent))
                        }
                    }
                }
                defer { watcher.cancel() }
                try await request.downloadAndInstall()
            }
        } catch {
            say(.failed(cause: "assets_download_failed", detail: String(describing: error)))
            return
        }
        guard !isStopped else { return }
        guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(
            compatibleWith: [transcriber]) else {
            say(.failed(cause: "audio_format_unsupported", detail: nil))
            return
        }
        let (stream, continuation) = AsyncStream.makeStream(of: AnalyzerInput.self,
            bufferingPolicy: .bufferingNewest(Self.maximumPendingBuffers))
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        let reading = Task { [weak self] in
            do {
                for try await result in transcriber.results {
                    self?.heard(text: String(result.text.characters),
                                start: result.range.start.seconds,
                                end: CMTimeRangeGetEnd(result.range).seconds,
                                isFinal: result.isFinal)
                }
            } catch {
                self?.say(.failed(cause: "analyzer_failed", detail: String(describing: error)))
            }
        }
        do {
            try await analyzer.start(inputSequence: stream)
        } catch {
            reading.cancel()
            continuation.finish()
            say(.failed(cause: "analyzer_failed", detail: String(describing: error)))
            return
        }
        let accepted = lock.withLock { () -> Bool in
            guard !stopped else { return false }
            self.analyzer = analyzer
            self.analyzerFormat = format
            self.continuation = continuation
            self.results = reading
            return true
        }
        guard accepted else {
            reading.cancel()
            continuation.finish()
            await analyzer.cancelAndFinishNow()
            return
        }
        say(.listening)
    }

    func receive(pcm16: Data, sampleRate: UInt32, rmsDBFS: Double) {
        guard sampleRate > 0, pcm16.count >= 2 else { return }
        let frames = pcm16.count / 2
        let work = lock.withLock { () -> (SpokenQuestion?, Double, AnalyzerInput?, AsyncStream<AnalyzerInput>.Continuation?, WakePhraseStep?, (SpeechAnalyzer, CMTime)?) in
            guard !stopped else { return (nil, 0, nil, nil, nil, nil) }
            let blockStart = fedSeconds
            fedSeconds += Double(frames) / Double(sampleRate)
            let spoken = policy.voice(at: fedSeconds, speaking: rmsDBFS > Self.speechLevelDBFS)
            let step = policy.step
            var input: AnalyzerInput?
            if continuation != nil, let analyzerFormat {
                if analyzerOrigin == nil { analyzerOrigin = blockStart }
                if let buffer = convert(pcm16, frames: frames, sampleRate: sampleRate,
                                        to: analyzerFormat) {
                    input = AnalyzerInput(buffer: buffer)
                }
            }
            // The pause came with only volatile words: ask the transcriber to
            // finalize through all the audio fed so far (on its timeline).
            var finalize: (SpeechAnalyzer, CMTime)?
            if spoken == nil, let analyzer, let origin = analyzerOrigin,
               let through = policy.finalizationDue() {
                finalize = (analyzer, CMTime(seconds: max(0, through - origin),
                                             preferredTimescale: 1_000))
            }
            return (spoken, fedSeconds, input, continuation, step, finalize)
        }
        if let input = work.2 { work.3?.yield(input) }
        if let step = work.4 { say(.step(step)) }
        if let spoken = work.0 { question(spoken, work.1) }
        if let (analyzer, through) = work.5 {
            // A refusal leaves the question to the catch-up wait, which sends
            // it with the words heard so far; nothing else depends on it.
            Task { try? await analyzer.finalize(through: through) }
        }
    }

    private func heard(text: String, start: Double, end: Double, isFinal: Bool) {
        let emitted = lock.withLock { () -> (SpokenQuestion?, Double, WakePhraseStep?)? in
            guard !stopped, let origin = analyzerOrigin,
                  start.isFinite, end.isFinite else { return nil }
            let segment = HeardSegment(text: text, start: origin + start, end: origin + end,
                                       isFinal: isFinal)
            let spoken = policy.heard(segment)
            let step = policy.step
            return (spoken, fedSeconds, step)
        }
        if let step = emitted?.2 { say(.step(step)) }
        if let emitted, let spoken = emitted.0 { question(spoken, emitted.1) }
    }

    /// 16-bit mono at the capture's rate to the analyzer's format. Called
    /// with the lock held, on the drain queue.
    private func convert(_ pcm16: Data, frames: Int, sampleRate: UInt32,
                         to format: AVAudioFormat) -> AVAudioPCMBuffer? {
        guard let source = AVAudioFormat(commonFormat: .pcmFormatInt16,
                                         sampleRate: Double(sampleRate), channels: 1,
                                         interleaved: true),
              let input = AVAudioPCMBuffer(pcmFormat: source,
                                           frameCapacity: AVAudioFrameCount(frames)),
              let samples = input.int16ChannelData else { return nil }
        input.frameLength = AVAudioFrameCount(frames)
        pcm16.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else { return }
            UnsafeMutableRawPointer(samples[0]).copyMemory(from: base, byteCount: frames * 2)
        }
        if source == format { return input }
        if converter == nil || converterInput != source {
            converter = AVAudioConverter(from: source, to: format)
            converter?.primeMethod = .none
            converterInput = source
        }
        guard let converter else { return nil }
        let ratio = format.sampleRate / source.sampleRate
        let capacity = AVAudioFrameCount((Double(frames) * ratio).rounded(.up)) + 16
        guard let output = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else {
            return nil
        }
        var failure: NSError?
        var supplied = false
        let status = converter.convert(to: output, error: &failure) { _, inputStatus in
            defer { supplied = true }
            inputStatus.pointee = supplied ? .noDataNow : .haveData
            return supplied ? nil : input
        }
        guard status != .error, failure == nil, output.frameLength > 0 else { return nil }
        return output
    }

    func finish() { stop() }

    func stop() {
        let held = lock.withLock { () -> (AsyncStream<AnalyzerInput>.Continuation?, SpeechAnalyzer?, Task<Void, Never>?, Task<Void, Never>?) in
            guard !stopped else { return (nil, nil, nil, nil) }
            stopped = true
            let held = (continuation, analyzer, setup, results)
            continuation = nil
            analyzer = nil
            setup = nil
            results = nil
            converter = nil
            return held
        }
        held.0?.finish()
        held.2?.cancel()
        held.3?.cancel()
        if let analyzer = held.1 {
            Task { await analyzer.cancelAndFinishNow() }
        }
    }
}
