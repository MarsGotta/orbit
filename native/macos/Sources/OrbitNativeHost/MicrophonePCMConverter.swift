import AVFoundation

/// Runs on the relay worker. A meeting keeps its original wire format even
/// when CoreAudio switches between a Bluetooth and a built-in microphone.
final class MicrophonePCMConverter {
    let outputRate: UInt32
    private var converter: AVAudioConverter?
    private var inputRate: UInt32?
    /// Frames given to and taken from the current converter. Each block asks
    /// for exactly its share of the output, carrying the fraction forward.
    private var framesIn: UInt64 = 0
    private var framesOut: UInt64 = 0

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
            framesIn = 0
            framesOut = 0
        }
        // The output must be exactly this block's share. Measured on a Mac
        // (2026-10-29): with a larger buffer, AVAudioConverter keeps part of
        // every block when upsampling 44.1 to 48 kHz (86,016 frames out of
        // 89,164 over 20 blocks); with the exact share it keeps under one.
        let frames = UInt64(pcm.count / 2)
        let due = (framesIn + frames) * UInt64(outputRate) / UInt64(sampleRate)
        let capacity = max(due - min(due, framesOut), 1)
        guard capacity <= UInt64(UInt32.max),
              let converter,
              let input = AVAudioPCMBuffer(pcmFormat: converter.inputFormat,
                  frameCapacity: AVAudioFrameCount(frames)),
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
        framesIn += frames
        framesOut += UInt64(output.frameLength)
        return Data(bytes: converted, count: Int(output.frameLength) * 2)
    }
}
