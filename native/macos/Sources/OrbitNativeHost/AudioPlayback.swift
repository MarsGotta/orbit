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
    private var generation: UInt64 = 0
    package private(set) var playing = false
    package var becameIdle: (() -> Void)?

    package init(session: SharedAudioSession, channel: NativeChannel) {
        self.session = session
        self.channel = channel
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
        if let currentID = fence.currentID, currentID != id { stop() }
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
        player.scheduleBuffer(buffer, completionCallbackType: .dataPlayedBack) { [weak self] _ in
            DispatchQueue.main.async {
                guard let self, self.generation == thisGeneration else { return }
                self.queuedFrames = max(0, self.queuedFrames - frames)
                self.playing = self.queuedFrames > 0
                self.session.playbackActivity.update(active: self.playing, playbackID: id)
                self.channel.send(.makePlaybackProgress(id: id, state: "played", active: self.playing,
                    sequence: sequence, frames: UInt32(sourceFrames), renderedFrames: UInt32(frames),
                    remainingFrames: UInt32(self.queuedFrames)))
                if !self.playing { self.becameIdle?() }
            }
        }
        player.play()
        playing = true
        session.playbackActivity.update(active: true, playbackID: id)
        channel.send(.makePlaybackProgress(id: id, state: "started", active: true, sequence: sequence))
        return ["playback_id": id, "sequence": sequence, "frames": sourceFrames,
                "rendered_frames": frames, "render_sample_rate": PlaybackResampler.outputRate, "state": "queued"]
    }

    package func enqueueClip(_ command: NativePlaybackClipFields) throws -> [String: Any] {
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
        if let currentID = fence.currentID, currentID != clip.playbackID { stop() }
        guard !playing, queuedFrames == 0 else { throw NativeFailure("playback_backpressure") }
        guard let format = session.playbackFormat else {
            throw NativeFailure("unsupported_playback_format")
        }
        let buffer = try CompressedAudioDecoder.decode(clip, to: format)
        try session.startPlayback()
        fence = candidate
        formatRate = nil
        resampler = nil
        let frames = Int(buffer.frameLength)
        let thisGeneration = generation
        queuedFrames = frames
        player.scheduleBuffer(buffer, completionCallbackType: .dataPlayedBack) { [weak self] _ in
            DispatchQueue.main.async {
                guard let self, self.generation == thisGeneration else { return }
                self.queuedFrames = 0
                self.playing = false
                self.session.playbackActivity.update(active: false, playbackID: nil)
                self.channel.send(.makePlaybackProgress(id: clip.playbackID, state: "played",
                    active: false, remainingFrames: 0, mimeType: clip.mimeType, provider: clip.provider))
                self.becameIdle?()
            }
        }
        player.play()
        playing = true
        session.playbackActivity.update(active: true, playbackID: clip.playbackID)
        channel.send(.makePlaybackProgress(id: clip.playbackID, state: "started", active: true,
            mimeType: clip.mimeType, provider: clip.provider))
        return [
            "playback_id": clip.playbackID, "mime_type": clip.mimeType,
            "provider": clip.provider, "rendered_frames": frames,
            "render_sample_rate": PlaybackResampler.outputRate, "state": "queued"
        ]
    }

    package func stop() {
        let id = queuedFrames > 0 ? fence.currentID : nil
        generation &+= 1
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
