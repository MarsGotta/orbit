import XCTest
import OrbitNativeProtocol
@testable import OrbitNativeState

final class AudioRouteChangePolicyTests: XCTestCase {
    private func decision(stopped: Bool = false, closing: Bool = false,
                          microphone: CaptureSlot = .active,
                          processing: AudioProcessingMode = .unprocessed,
                          playback: Bool = false, failure: String? = nil,
                          routeChanged: Bool? = true, purpose: NativeMicrophonePurpose? = .meeting) -> AudioRouteChangeAction {
        AudioRouteChangePolicy.decide(stopped: stopped, closing: closing,
            microphone: microphone, processing: processing, playback: playback, failure: failure,
            routeChanged: routeChanged, purpose: purpose)
    }

    func testRecordingRestartsOnTheNewRouteEvenWhileDeimosWasSpeaking() {
        XCTAssertEqual(decision(), .restartMicrophone)
        XCTAssertEqual(decision(playback: true), .restartMicrophone)
    }

    func testPrivacyAndClosingWinEvenOverARecoveryFailure() {
        let failures: [String?] = [nil, "audio_route_start_failed"]
        for failure in failures {
            XCTAssertEqual(decision(stopped: true, playback: true, failure: failure), .none(reason: "privacy_stopped"))
            XCTAssertEqual(decision(closing: true, playback: true, failure: failure), .none(reason: "closing_or_draining"))
        }
    }

    func testInactiveOrDrainingMicrophonesNeverRestart() {
        let slots: [CaptureSlot] = [.idle, .starting, .draining, .unconfirmed(reason: "audio_drain_incomplete")]
        for slot in slots {
            XCTAssertEqual(decision(microphone: slot), .none(reason: "microphone_inactive"))
        }
    }

    func testPlaybackAloneIsCancelledAndIdleDoesNothing() {
        XCTAssertEqual(decision(microphone: .idle, playback: true), .cancelPlayback)
        XCTAssertEqual(decision(microphone: .idle), .none(reason: "microphone_inactive"))
    }

    func testContinuousVoiceKeepsItsExistingSafeCut() {
        XCTAssertEqual(decision(processing: .voiceProcessing),
                       .stop(reason: "audio_device_changed_voice_processing"))
    }

    func testFailedRecoveryCutsWithTheExactCause() {
        for cause in ["microphone_input_missing", "audio_device_query_failed", "microphone_permission_required",
                      "audio_route_start_failed",
                      "audio_route_drain_incomplete"] {
            XCTAssertEqual(decision(microphone: .idle, failure: cause), .stop(reason: cause))
        }
    }

    func testRecoveryAfterDrainRequiresAuthority() {
        for stopped in [false, true] {
            for closing in [false, true] {
                let action = AudioRouteChangePolicy.decide(stopped: stopped, closing: closing,
                    microphone: .idle, processing: .unprocessed, playback: false, recovering: true,
                    routeChanged: true, purpose: .meeting)
                XCTAssertEqual(action, stopped ? .none(reason: "privacy_stopped")
                    : closing ? .none(reason: "closing_or_draining") : .restartMicrophone)
            }
        }
    }

    func testSameRouteNeverRestartsOrCutsAnyFlow() {
        for purpose in [NativeMicrophonePurpose.meeting, .voiceNote] {
            for processing in [AudioProcessingMode.unprocessed, .voiceProcessing] {
                XCTAssertEqual(decision(processing: processing, playback: true,
                    routeChanged: false, purpose: purpose), .none(reason: "route_unchanged"))
            }
        }
    }

    func testVoiceNoteCutsOnceAndNeverRestartsAfterTheCut() {
        XCTAssertEqual(decision(purpose: .voiceNote), .stop(reason: "audio_device_changed_voice_note"))
        for slot in [CaptureSlot.draining, .idle, .unconfirmed(reason: "audio_drain_incomplete")] {
            XCTAssertEqual(decision(microphone: slot, purpose: .voiceNote), .none(reason: "microphone_inactive"))
        }
    }

    func testUnknownPurposeOrRouteNeverAuthorizesRecovery() {
        XCTAssertEqual(decision(purpose: nil), .stop(reason: "audio_device_changed_purpose_unknown"))
        XCTAssertEqual(decision(routeChanged: nil), .stop(reason: "audio_route_unobservable"))
        XCTAssertEqual(decision(microphone: .idle, playback: true, routeChanged: nil),
                       .none(reason: "audio_route_unobservable"))
    }

    func testGapUsesCapturedFrameTimesRatherThanNotificationOrRestartTimes() {
        XCTAssertEqual(AudioRouteResume(previousEndNS: 1_100_000_000)
            .durationNS(until: 3_100_000_000), 2_000_000_000)
    }

    func testMissingOrRegressingFrameTimesNeverInventADuration() {
        XCTAssertNil(AudioRouteResume(previousEndNS: nil).durationNS(until: 3_100_000_000))
        XCTAssertNil(AudioRouteResume(previousEndNS: 4_000_000_000).durationNS(until: 3_100_000_000))
    }
}
