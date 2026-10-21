import XCTest
@testable import OrbitNativeProtocol

final class AudioProcessingTests: XCTestCase {
    func testExplicitProcessingIsTypedAndLegacyDefaultsRemainPossible() throws {
        guard case .audioStart(let legacy) = try NativeWireCommand.decode(Data(#"{"v":1,"id":"a","command":"audio_start","source":"mic_local","stream_id":"mic-1"}"#.utf8)) else { return XCTFail("audio_start variant") }
        XCTAssertNil(legacy.fields.processing)
        guard case .audioStart(let voice) = try NativeWireCommand.decode(Data(#"{"v":1,"id":"a","command":"audio_start","source":"mic_local","stream_id":"mic-1","processing":"voice_processing"}"#.utf8)) else { return XCTFail("audio_start variant") }
        XCTAssertEqual(voice.fields.processing, .voiceProcessing)
        XCTAssertThrowsError(try NativeWireCommand.decode(Data(#"{"v":1,"id":"a","command":"audio_start","source":"mic_local","stream_id":"mic-1","processing":"pretend_aec"}"#.utf8)))
    }

    func testMeetingAndLiveGraphCannotBeReconfigured() {
        for remote in [false, true] {
            XCTAssertFalse(AudioConfigurationPolicy.permitsCapture(requested: .voiceProcessing,
                current: .unprocessed, configured: true, microphoneActive: true,
                remoteActive: remote, playbackActive: false))
        }
        XCTAssertFalse(AudioConfigurationPolicy.permitsCapture(requested: .voiceProcessing,
            current: .unprocessed, configured: true, microphoneActive: false,
            remoteActive: true, playbackActive: false))
        XCTAssertFalse(AudioConfigurationPolicy.permitsCapture(requested: .voiceProcessing,
            current: .unprocessed, configured: true, microphoneActive: false,
            remoteActive: false, playbackActive: true))
        XCTAssertTrue(AudioConfigurationPolicy.permitsCapture(requested: .unprocessed,
            current: .unprocessed, configured: true, microphoneActive: false,
            remoteActive: true, playbackActive: false))
        XCTAssertTrue(AudioConfigurationPolicy.permitsCapture(requested: .voiceProcessing,
            current: .unprocessed, configured: true, microphoneActive: false,
            remoteActive: false, playbackActive: false))
    }

    func testObservationNeedsEveryRuntimeFactAndNeverCertifiesAEC() {
        for enabled in [false, true] {
            for bypassed in [false, true] {
                for reference in [false, true] {
                    let value = AudioProcessingObservation(mode: .voiceProcessing,
                        enabled: enabled, bypassed: bypassed, agcEnabled: false,
                        sharedPlaybackReference: reference, pipelineID: "fixture-pipeline")
                    XCTAssertEqual(value.usableForContinuousVoice, enabled && !bypassed && reference)
                    XCTAssertNil(value.fields["aec_validated"])
                }
            }
        }
        let raw = AudioProcessingObservation(mode: .unprocessed, enabled: true,
            bypassed: false, agcEnabled: false, sharedPlaybackReference: true, pipelineID: "fixture")
        XCTAssertFalse(raw.usableForContinuousVoice)
        let anonymous = AudioProcessingObservation(mode: .voiceProcessing, enabled: true,
            bypassed: false, agcEnabled: false, sharedPlaybackReference: true, pipelineID: "")
        XCTAssertFalse(anonymous.usableForContinuousVoice)
    }

    func testDuckingReadbackTravelsThroughTheTypedStateEncoder() throws {
        let event = NativeEvent.makeState(stopped: false, connected: true,
            instanceID: "fixture", generation: 1, selectedTextAvailable: false,
            microphone: true, systemAudio: false, playing: true,
            voiceProcessingAvailable: true, playbackReferenceAvailable: true,
            audioProcessing: .init(mode: "voice_processing", enabled: true, bypassed: false,
                agcEnabled: false, playbackReference: "shared_engine", pipelineID: "fixture",
                engineRunning: true, renderSampleRate: 48_000,
                otherAudioDuckingLevel: 10, otherAudioDuckingAdvanced: false),
            calendar: .init(authorization: "not_determined", physicalReadPending: false, failure: nil))
        let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(event)) as? [String: Any])
        let processing = try XCTUnwrap(encoded["audio_processing"] as? [String: Any])
        XCTAssertEqual(processing["other_audio_ducking_level"] as? Int, 10)
        XCTAssertEqual(processing["other_audio_ducking_advanced"] as? Bool, false)
    }

    func testRMSIsFiniteAndTracksTheCapturedPCM() {
        let silence = PCMLevel(pcm16: pcm([0, 0, 0]))
        XCTAssertEqual(silence.rmsDBFS, -120)
        let half = PCMLevel(pcm16: pcm([16384, -16384]))
        XCTAssertEqual(half.rmsDBFS, -6.020599913, accuracy: 0.00001)
        XCTAssertTrue(PCMLevel(pcm16: Data()).rmsDBFS.isFinite)
    }

    func testResamplingPacketBoundariesKeepPhaseAndSamples() throws {
        let source: [Int16] = (0..<701).map { Int16(($0 * 71) % 32000 - 16000) }
        for rate in NativeProtocol.acceptedSampleRates {
            var whole = try PlaybackResampler(sourceRate: rate)
            let expected = try whole.append(pcm(source))
            var streamed = try PlaybackResampler(sourceRate: rate)
            var actual = [Float]()
            for start in stride(from: 0, to: source.count, by: 17) {
                actual += try streamed.append(pcm(Array(source[start..<min(start + 17, source.count)])))
            }
            XCTAssertEqual(actual, expected, "sample rate \(rate)")
            let expectedCount = ((source.count - 1) * PlaybackResampler.outputRate) / rate + 1
            XCTAssertEqual(actual.count, expectedCount)
            XCTAssertTrue(actual.allSatisfy { $0.isFinite && abs($0) <= 1 })
        }
    }

    func testNativeRateHasNoDuplicationAndConverterIsBounded() throws {
        var converter = try PlaybackResampler(sourceRate: 48_000)
        XCTAssertEqual(try converter.append(pcm([0, 16384, -16384])), [0, 0.5, -0.5])
        XCTAssertEqual(try converter.append(pcm([8192])), [0.25])
        XCTAssertThrowsError(try converter.append(Data()))
        XCTAssertThrowsError(try converter.append(Data([0])))
        XCTAssertThrowsError(try converter.append(Data(repeating: 0, count: 96_002)))
        XCTAssertThrowsError(try PlaybackResampler(sourceRate: 96_000))
    }

    private func pcm(_ samples: [Int16]) -> Data {
        Data(samples.flatMap { value in
            let bits = UInt16(bitPattern: value)
            return [UInt8(bits & 255), UInt8(bits >> 8)]
        })
    }
}
