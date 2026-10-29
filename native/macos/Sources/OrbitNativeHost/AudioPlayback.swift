import AVFoundation
import OrbitNativeProtocol
import OrbitNativeTransport

package final class AudioPlayback {
    private let session: SharedAudioSession
    private var player: AVAudioPlayerNode { session.player }
    private let channel: NativeChannel
    private var fence = PlaybackFence()
    private var formatRate: Int?
    private var resampler: PlaybackResampler?
    private var queuedFrames = 0
    package private(set) var paused = false
    private var generation: UInt64 = 0
    private var decodingTask: Task<AVAudioPCMBuffer, Error>?
    private let decodeClip: @Sendable (PlaybackClip, AVAudioFormat) throws -> AVAudioPCMBuffer
    package private(set) var playing = false
    package var becameIdle: (() -> Void)?

    package init(session: SharedAudioSession, channel: NativeChannel,
                 decodeClip: @escaping @Sendable (PlaybackClip, AVAudioFormat) throws -> AVAudioPCMBuffer = {
                     try CompressedAudioDecoder.decode($0, to: $1)
                 }) {
        self.session = session
        self.channel = channel
        self.decodeClip = decodeClip
    }

    package func enqueue(_ command: NativePlaybackFields) throws -> [String: Any] {
        let data = try command.playbackData()
        let id = command.playback_id
        let sequence = command.sequence
        let rate = command.sample_rate
        guard fence.generationCount < 10_000 else { throw NativeFailure("playback_session_exhausted") }
        var candidate = fence
        guard candidate.accept(id: id, sequence: sequence) else {
            throw NativeFailure("stale_or_out_of_order_playback")
        }
        if let currentID = fence.currentID, currentID != id {
            let wasPaused = paused
            stop()
            paused = wasPaused
        }
        guard decodingTask == nil else { throw NativeFailure("playback_backpressure") }
        if formatRate != rate {
            guard !playing, queuedFrames == 0 else { throw NativeFailure("playback_format_changed") }
            resampler = try PlaybackResampler(sourceRate: rate)
            formatRate = rate
        }
        guard var nextResampler = resampler else { throw NativeFailure("playback_converter_unavailable") }
        let converted = try nextResampler.append(data)
        guard !converted.isEmpty, queuedFrames + converted.count <= PlaybackResampler.outputRate * 5 else {
            throw NativeFailure("playback_backpressure")
        }
        guard let format = AVAudioFormat(standardFormatWithSampleRate: Double(PlaybackResampler.outputRate), channels: 1),
              let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(converted.count)),
              let samples = buffer.floatChannelData?[0] else { throw NativeFailure("audio_memory_unavailable") }
        buffer.frameLength = buffer.frameCapacity
        for index in converted.indices { samples[index] = converted[index] }
        try session.startPlayback()
        fence = candidate
        resampler = nextResampler
        let frames = Int(buffer.frameLength)
        let sourceFrames = data.count / 2
        let thisGeneration = generation
        queuedFrames += frames
        player.scheduleBuffer(buffer, completionCallbackType: .dataPlayedBack) { [weak self] (_: AVAudioPlayerNodeCompletionCallbackType) -> Void in
            DispatchQueue.main.async { () -> Void in
                guard let self, self.generation == thisGeneration else { return }
                self.queuedFrames = max(0, self.queuedFrames - frames)
                self.playing = self.queuedFrames > 0
                let active = self.playing && !self.paused
                self.session.playbackActivity.update(active: active, playbackID: id)
                self.channel.send(.makePlaybackProgress(id: id, state: "played", active: active,
                    sequence: sequence, frames: UInt32(sourceFrames), renderedFrames: UInt32(frames),
                    remainingFrames: UInt32(self.queuedFrames)))
                if !self.playing { self.becameIdle?() }
            }
        }
        if !paused { player.play() }
        playing = true
        session.playbackActivity.update(active: !paused, playbackID: id)
        channel.send(.makePlaybackProgress(id: id, state: paused ? "paused" : "started", active: !paused, sequence: sequence))
        return ["playback_id": id, "sequence": sequence, "frames": sourceFrames,
                "rendered_frames": frames, "render_sample_rate": PlaybackResampler.outputRate, "state": "queued"]
    }

    @MainActor package func enqueueClip(_ command: NativePlaybackClipFields) async throws -> [String: Any] {
        let clip: PlaybackClip
        do {
            clip = try command.playbackClip()
        } catch ProtocolError.unsupportedMediaType {
            throw NativeFailure("unsupported_media_type")
        } catch {
            throw NativeFailure("invalid_audio")
        }
        guard fence.generationCount < 10_000 else {
            throw NativeFailure("playback_session_exhausted")
        }
        var candidate = fence
        guard candidate.accept(id: clip.playbackID, sequence: 0) else {
            throw NativeFailure("stale_or_out_of_order_playback")
        }
        if let currentID = fence.currentID, currentID != clip.playbackID {
            let wasPaused = paused
            stop()
            paused = wasPaused
        }
        guard !playing, queuedFrames == 0, decodingTask == nil else { throw NativeFailure("playback_backpressure") }
        guard let format = session.playbackFormat else {
            throw NativeFailure("unsupported_playback_format")
        }
        // Reserve the sequence before yielding, so replacement and stop retire it.
        fence = candidate
        let decodingGeneration = generation
        let decode = decodeClip
        let task = Task.detached(priority: .userInitiated) {
            try Task.checkCancellation()
            return try decode(clip, format)
        }
        decodingTask = task
        defer { if generation == decodingGeneration { decodingTask = nil } }
        let buffer: AVAudioPCMBuffer
        do { buffer = try await task.value }
        catch {
            guard generation == decodingGeneration else { return ["state": "cancelled"] }
            throw error
        }
        guard generation == decodingGeneration, !task.isCancelled else { return ["state": "cancelled"] }
        try session.startPlayback()
        formatRate = nil
        resampler = nil
        let frames = Int(buffer.frameLength)
        let thisGeneration = generation
        queuedFrames = frames
        player.scheduleBuffer(buffer, completionCallbackType: .dataPlayedBack) { [weak self] (_: AVAudioPlayerNodeCompletionCallbackType) -> Void in
            DispatchQueue.main.async { () -> Void in
                guard let self, self.generation == thisGeneration else { return }
                self.queuedFrames = 0
                self.playing = false
                self.session.playbackActivity.update(active: false, playbackID: nil)
                self.channel.send(.makePlaybackProgress(id: clip.playbackID, state: "played",
                    active: false, remainingFrames: 0, mimeType: clip.mimeType, provider: clip.provider))
                self.becameIdle?()
            }
        }
        if !paused { player.play() }
        playing = true
        session.playbackActivity.update(active: !paused, playbackID: clip.playbackID)
        channel.send(.makePlaybackProgress(id: clip.playbackID, state: paused ? "paused" : "started", active: !paused,
            mimeType: clip.mimeType, provider: clip.provider))
        return [
            "playback_id": clip.playbackID, "mime_type": clip.mimeType,
            "provider": clip.provider, "rendered_frames": frames,
            "render_sample_rate": PlaybackResampler.outputRate, "state": "queued"
        ]
    }

    // Keep the scheduled buffer and its render position. A fetch that finishes
    // while paused is queued silently until resume; stop retires both states.
    package func pause() {
        paused = true
        if playing { player.pause() }
        session.playbackActivity.update(active: false, playbackID: fence.currentID)
        if let id = fence.currentID {
            channel.send(.makePlaybackProgress(id: id, state: "paused", active: false))
        }
    }

    package func resume() {
        guard paused else { return }
        paused = false
        if queuedFrames > 0 {
            player.play()
            session.playbackActivity.update(active: true, playbackID: fence.currentID)
            if let id = fence.currentID {
                channel.send(.makePlaybackProgress(id: id, state: "resumed", active: true))
            }
        }
    }

    package func stop() {
        paused = false
        let id = queuedFrames > 0 ? fence.currentID : nil
        generation &+= 1
        decodingTask?.cancel()
        decodingTask = nil
        session.stopPlayerIfCreated()
        fence.stop()
        queuedFrames = 0
        playing = false
        formatRate = nil
        resampler = nil
        session.playbackActivity.update(active: false, playbackID: nil)
        if let id {
            channel.send(.makePlaybackProgress(id: id, state: "cancelled", active: false))
        }
        becameIdle?()
    }
}
