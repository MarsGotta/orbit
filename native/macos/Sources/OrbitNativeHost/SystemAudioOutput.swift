import AVFoundation
import ScreenCaptureKit
import CoreMedia
import OrbitAudioRing

/// One immutable destination per SCStream. The callback never reads the capture
/// controller's replaceable session. Retaining this output also retains its ring.
/// ObjectIdentifier avoids an output -> stream ownership cycle; the session owns
/// both objects until stop, the callback-queue marker and relay drain complete.
final class SystemAudioOutput: NSObject, SCStreamOutput {
    private let expectedStream: ObjectIdentifier
    private let relay: AudioFrameRelay
    private let queue: DispatchQueue

    init(stream: SCStream, relay: AudioFrameRelay, queue: DispatchQueue) {
        expectedStream = ObjectIdentifier(stream)
        self.relay = relay
        self.queue = queue
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
                of outputType: SCStreamOutputType) {
        guard outputType == .audio else { return }
        dispatchPrecondition(condition: .onQueue(queue))
        let ring = relay.ring
        orbit_ring_callback_begin(ring)
        defer { orbit_ring_callback_end(ring) }
        guard ObjectIdentifier(stream) == expectedStream else {
            orbit_ring_mark_timing_failure(ring)
            return
        }
        guard sampleBuffer.isValid,
              let description = sampleBuffer.formatDescription?.audioStreamBasicDescription,
              description.mFormatID == kAudioFormatLinearPCM,
              description.mFormatFlags & kAudioFormatFlagIsFloat != 0,
              description.mBitsPerChannel == 32, description.mChannelsPerFrame == 1,
              description.mSampleRate.isFinite, description.mSampleRate >= 8_000,
              description.mSampleRate <= 192_000,
              description.mSampleRate.rounded() == description.mSampleRate else {
            orbit_ring_mark_timing_failure(ring)
            return
        }
        let pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        let frameCount = CMSampleBufferGetNumSamples(sampleBuffer)
        guard let clock = stream.synchronizationClock,
              let captureNS = CaptureClock.nanoseconds(pts),
              let hostNS = CaptureClock.hostNanoseconds(pts, from: clock),
              frameCount > 0, frameCount <= Int(UInt32.max) else {
            orbit_ring_mark_timing_failure(ring)
            return
        }
        do {
            try sampleBuffer.withAudioBufferList { list, _ in
                guard list.count == 1, let data = list[0].mData,
                      Int(list[0].mDataByteSize) >= frameCount * MemoryLayout<Float>.size else {
                    orbit_ring_mark_timing_failure(ring)
                    return
                }
                _ = orbit_ring_push_interleaved_timed(
                    ring, data.assumingMemoryBound(to: Float.self), 1,
                    UInt32(frameCount), UInt32(description.mSampleRate), captureNS, hostNS
                )
            }
        } catch {
            orbit_ring_mark_timing_failure(ring)
        }
    }
}
