import AudioToolbox
import AVFoundation
import OrbitNativeProtocol

private let memoryAudioRead: AudioFile_ReadProc = {
    clientData, position, requestedBytes, destination, actualBytes in
    let source = Unmanaged<MemoryAudioFile>.fromOpaque(clientData).takeUnretainedValue()
    guard position >= 0, position <= Int64(source.data.count) else {
        actualBytes.pointee = 0
        return kAudioFilePositionError
    }
    let start = Int(position)
    let count = min(Int(requestedBytes), source.data.count - start)
    source.data.copyBytes(
        to: UnsafeMutableRawBufferPointer(start: destination, count: count),
        from: start..<(start + count)
    )
    actualBytes.pointee = UInt32(count)
    return noErr
}

private let memoryAudioSize: AudioFile_GetSizeProc = { clientData in
    let source = Unmanaged<MemoryAudioFile>.fromOpaque(clientData).takeUnretainedValue()
    return Int64(source.data.count)
}

private final class MemoryAudioFile {
    let data: Data
    private(set) var file: AudioFileID?

    init(data: Data) throws {
        self.data = data
        var opened: AudioFileID?
        let status = AudioFileOpenWithCallbacks(
            Unmanaged.passUnretained(self).toOpaque(), memoryAudioRead, nil,
            memoryAudioSize, nil, 0, &opened
        )
        guard status == noErr, let opened else {
            throw NativeFailure("unsupported_media_type")
        }
        file = opened
    }

    deinit {
        if let file { AudioFileClose(file) }
    }
}

package enum CompressedAudioDecoder {
    private static let maximumSeconds = 60.0
    private static let conversionChunkFrames: AVAudioFrameCount = 2_048

    private static func append(
        _ source: AVAudioPCMBuffer,
        to destination: AVAudioPCMBuffer
    ) throws {
        guard source.format == destination.format else {
            throw NativeFailure("unsupported_media_type")
        }
        let sourceFrames = source.frameLength
        let destinationFrames = destination.frameLength
        let (combinedFrames, overflow) = destinationFrames.addingReportingOverflow(sourceFrames)
        guard !overflow, combinedFrames <= destination.frameCapacity else {
            throw NativeFailure("clip_too_long")
        }
        guard sourceFrames > 0 else { return }

        let bytesPerFrame = Int(destination.format.streamDescription.pointee.mBytesPerFrame)
        guard bytesPerFrame > 0 else { throw NativeFailure("unsupported_media_type") }
        let byteOffset = Int(destinationFrames) * bytesPerFrame
        let byteCount = Int(sourceFrames) * bytesPerFrame
        destination.frameLength = combinedFrames

        let sourceBuffers = UnsafeMutableAudioBufferListPointer(source.mutableAudioBufferList)
        let destinationBuffers = UnsafeMutableAudioBufferListPointer(
            destination.mutableAudioBufferList
        )
        guard sourceBuffers.count == destinationBuffers.count else {
            throw NativeFailure("unsupported_media_type")
        }
        for index in sourceBuffers.indices {
            let sourceBuffer = sourceBuffers[index]
            let destinationBuffer = destinationBuffers[index]
            guard Int(sourceBuffer.mDataByteSize) >= byteCount,
                  Int(destinationBuffer.mDataByteSize) >= byteOffset + byteCount,
                  let sourceData = sourceBuffer.mData,
                  let destinationData = destinationBuffer.mData else {
                throw NativeFailure("audio_memory_unavailable")
            }
            destinationData.advanced(by: byteOffset).copyMemory(
                from: sourceData,
                byteCount: byteCount
            )
        }
    }

    package static func decode(_ clip: PlaybackClip, to outputFormat: AVAudioFormat) throws
        -> AVAudioPCMBuffer {
        guard !clip.data.isEmpty else { throw NativeFailure("empty_audio") }
        let source = try MemoryAudioFile(data: clip.data)
        guard let file = source.file else { throw NativeFailure("unsupported_media_type") }

        var stream = AudioStreamBasicDescription()
        var streamSize = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
        guard AudioFileGetProperty(file, kAudioFilePropertyDataFormat, &streamSize, &stream) == noErr,
              let inputFormat = AVAudioFormat(streamDescription: &stream) else {
            throw NativeFailure("unsupported_media_type")
        }

        guard inputFormat.isInterleaved || inputFormat.channelCount == 1 else {
            throw NativeFailure("unsupported_media_type")
        }

        var packetCount: UInt64 = 0
        var packetCountSize = UInt32(MemoryLayout<UInt64>.size)
        var maximumPacketSize: UInt32 = 0
        var maximumPacketSizeSize = UInt32(MemoryLayout<UInt32>.size)
        guard AudioFileGetProperty(
                file, kAudioFilePropertyAudioDataPacketCount, &packetCountSize, &packetCount
              ) == noErr,
              AudioFileGetProperty(
                file, kAudioFilePropertyPacketSizeUpperBound,
                &maximumPacketSizeSize, &maximumPacketSize
              ) == noErr,
              packetCount <= UInt64(UInt32.max) else {
            throw NativeFailure("invalid_audio")
        }
        guard packetCount > 0 else { throw NativeFailure("empty_audio") }
        guard maximumPacketSize > 0,
              Int(maximumPacketSize) <= NativeProtocol.maximumPlaybackClipBytes else {
            throw NativeFailure("invalid_audio")
        }

        var duration = 0.0
        var durationSize = UInt32(MemoryLayout<Double>.size)
        guard AudioFileGetProperty(
            file, kAudioFilePropertyEstimatedDuration, &durationSize, &duration
        ) == noErr, duration.isFinite, duration >= 0 else {
            throw NativeFailure("invalid_audio")
        }
        guard duration > 0 else { throw NativeFailure("empty_audio") }
        guard duration <= maximumSeconds else { throw NativeFailure("clip_too_long") }

        guard let converter = AVAudioConverter(from: inputFormat, to: outputFormat) else {
            throw NativeFailure("unsupported_media_type")
        }
        let maximumFrames = AVAudioFrameCount(PlaybackResampler.outputRate * 60)
        guard let output = AVAudioPCMBuffer(
            pcmFormat: outputFormat, frameCapacity: maximumFrames + 1
        ) else { throw NativeFailure("audio_memory_unavailable") }
        var packetIndex: Int64 = 0
        var currentInput: AVAudioBuffer?
        var readFailed = false
        var inputBufferUnavailable = false
        var inputFormatUnsupported = false
        var inputExhausted = false
        var reachedEnd = false
        while !reachedEnd {
            let remainingOutputFrames = output.frameCapacity - output.frameLength
            guard remainingOutputFrames > 0 else { throw NativeFailure("clip_too_long") }
            let chunkCapacity = min(conversionChunkFrames, remainingOutputFrames)
            guard let chunk = AVAudioPCMBuffer(
                pcmFormat: outputFormat, frameCapacity: chunkCapacity
            ) else { throw NativeFailure("audio_memory_unavailable") }
            let packetIndexBeforeConversion = packetIndex
            var conversionError: NSError?
            let status = converter.convert(to: chunk, error: &conversionError) {
                requestedPackets, inputStatus in
                if inputExhausted {
                    inputStatus.pointee = .endOfStream
                    return nil
                }
                let remaining = packetCount - UInt64(packetIndex)
                if remaining == 0 {
                    inputExhausted = true
                    inputStatus.pointee = .endOfStream
                    return nil
                }
                let capacity = min(requestedPackets, AVAudioPacketCount(remaining))
                var packets = capacity
                let buffer: AVAudioBuffer
                if stream.mFormatID == kAudioFormatLinearPCM {
                    let framesPerPacket = max(1, stream.mFramesPerPacket)
                    let frameCapacity = UInt64(capacity) * UInt64(framesPerPacket)
                    guard frameCapacity <= UInt64(UInt32.max) else {
                        inputFormatUnsupported = true
                        inputStatus.pointee = .endOfStream
                        return nil
                    }
                    guard let pcm = AVAudioPCMBuffer(
                            pcmFormat: inputFormat,
                            frameCapacity: AVAudioFrameCount(frameCapacity)
                          ),
                          let destination = pcm.mutableAudioBufferList.pointee.mBuffers.mData else {
                        inputBufferUnavailable = true
                        inputStatus.pointee = .endOfStream
                        return nil
                    }
                    pcm.frameLength = pcm.frameCapacity
                    var bytes = pcm.mutableAudioBufferList.pointee.mBuffers.mDataByteSize
                    let readStatus = AudioFileReadPacketData(
                        file, false, &bytes, nil, packetIndex, &packets, destination
                    )
                    if readStatus != noErr {
                        readFailed = true
                        inputStatus.pointee = .endOfStream
                        return nil
                    }
                    guard packets > 0 else {
                        inputExhausted = true
                        inputStatus.pointee = .endOfStream
                        return nil
                    }
                    guard bytes > 0 else {
                        readFailed = true
                        inputStatus.pointee = .endOfStream
                        return nil
                    }
                    pcm.frameLength = AVAudioFrameCount(packets) * framesPerPacket
                    buffer = pcm
                } else {
                    let compressed = AVAudioCompressedBuffer(
                        format: inputFormat, packetCapacity: capacity,
                        maximumPacketSize: Int(maximumPacketSize)
                    )
                    var bytes = compressed.byteCapacity
                    let readStatus = AudioFileReadPacketData(
                        file, false, &bytes, compressed.packetDescriptions, packetIndex,
                        &packets, compressed.data
                    )
                    if readStatus != noErr {
                        readFailed = true
                        inputStatus.pointee = .endOfStream
                        return nil
                    }
                    guard packets > 0 else {
                        inputExhausted = true
                        inputStatus.pointee = .endOfStream
                        return nil
                    }
                    guard bytes > 0 else {
                        readFailed = true
                        inputStatus.pointee = .endOfStream
                        return nil
                    }
                    compressed.packetCount = packets
                    compressed.byteLength = bytes
                    buffer = compressed
                }
                packetIndex += Int64(packets)
                currentInput = buffer
                inputStatus.pointee = .haveData
                return currentInput
            }
            guard !readFailed else { throw NativeFailure("invalid_audio") }
            guard !inputFormatUnsupported else {
                throw NativeFailure("unsupported_media_type")
            }
            guard !inputBufferUnavailable else {
                throw NativeFailure("audio_memory_unavailable")
            }
            guard conversionError == nil, status != .error else {
                throw NativeFailure("unsupported_media_type")
            }
            try append(chunk, to: output)

            switch status {
            case .haveData:
                break
            case .inputRanDry:
                // A final partial read may already have produced frames. Convert again
                // so the input block can report the latched end and flush the converter.
                if chunk.frameLength == 0, packetIndex == packetIndexBeforeConversion {
                    if inputExhausted {
                        reachedEnd = true
                    } else {
                        throw NativeFailure("invalid_audio")
                    }
                }
            case .endOfStream:
                reachedEnd = true
            case .error:
                throw NativeFailure("unsupported_media_type")
            @unknown default:
                throw NativeFailure("unsupported_media_type")
            }
        }
        guard output.frameLength > 0 else { throw NativeFailure("empty_audio") }
        guard output.frameLength <= maximumFrames else { throw NativeFailure("clip_too_long") }
        return output
    }
}
