import AVFoundation

/// Runs on the relay worker. A meeting keeps its original wire format even
/// when CoreAudio switches between a Bluetooth and a built-in microphone.
final class MicrophonePCMConverter {
    let outputRate: UInt32
    private var converter: AVAudioConverter?
    private var inputRate: UInt32?

    init(outputRate: UInt32) { self.outputRate = outputRate }

    func convert(_ pcm: Data, sampleRate: UInt32) throws -> Data {
        if sampleRate == outputRate { return pcm }
        if inputRate != sampleRate {
            guard let input = AVAudioFormat(commonFormat: .pcmFormatInt16,
                    sampleRate: Double(sampleRate), channels: 1, interleaved: false),
                  let output = AVAudioFormat(commonFormat: .pcmFormatInt16,
                    sampleRate: Double(outputRate), channels: 1, interleaved: false),
                  let converter = AVAudioConverter(from: input, to: output) else {
                throw NativeFailure("microphone_converter_unavailable")
            }
            converter.primeMethod = .none
            self.converter = converter
            inputRate = sampleRate
        }
        // Size each short ring block by its rate ratio. Sixteen frames cover
        // converter rounding/filter headroom, not a second of unused PCM.
        let frames = pcm.count / 2
        let capacity = ceil(Double(frames) * Double(outputRate) / Double(sampleRate)) + 16
        guard capacity.isFinite, capacity > 0, capacity <= Double(UInt32.max),
              let converter,
              let input = AVAudioPCMBuffer(pcmFormat: converter.inputFormat,
                  frameCapacity: AVAudioFrameCount(pcm.count / 2)),
              let samples = input.int16ChannelData?[0],
              let output = AVAudioPCMBuffer(pcmFormat: converter.outputFormat,
                  frameCapacity: AVAudioFrameCount(capacity)) else {
            throw NativeFailure("audio_memory_unavailable")
        }
        input.frameLength = input.frameCapacity
        pcm.copyBytes(to: UnsafeMutableRawBufferPointer(start: samples, count: pcm.count))
        var supplied = false
        var error: NSError?
        let status = converter.convert(to: output, error: &error) { _, inputStatus in
            guard !supplied else { inputStatus.pointee = .noDataNow; return nil }
            supplied = true
            inputStatus.pointee = .haveData
            return input
        }
        guard status != .error, let converted = output.int16ChannelData?[0] else {
            throw NativeFailure("microphone_conversion_failed", underlying: error)
        }
        return Data(bytes: converted, count: Int(output.frameLength) * 2)
    }
}
