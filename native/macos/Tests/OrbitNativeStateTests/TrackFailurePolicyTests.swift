import Foundation
import XCTest
@testable import OrbitNativeState

/// One track's failure never cuts the other.
final class TrackFailurePolicyTests: XCTestCase {
    func testAStoppedSystemAudioStreamIsRestartedOnceWhileTheMicrophoneRecords() {
        XCTAssertEqual(TrackFailurePolicy.decide(track: .systemAudio, otherTrack: .active,
                                                 closing: false, restartAttempted: false),
                       .restartSystemAudio)
        XCTAssertEqual(TrackFailurePolicy.decide(track: .systemAudio, otherTrack: .active,
                                                 closing: false, restartAttempted: true),
                       .endTrack)
    }

    func testAFailedMicrophoneEndsAloneWhileTheSystemAudioRecords() {
        for attempted in [false, true] {
            XCTAssertEqual(TrackFailurePolicy.decide(track: .microphone, otherTrack: .active,
                                                     closing: false, restartAttempted: attempted),
                           .endTrack)
        }
    }

    func testWithNothingElseCapturedOrAStopUnderWayTheCaptureStopsAsBefore() {
        for track in [CaptureTrack.microphone, .systemAudio] {
            for other in [CaptureSlot.idle, .draining, .unconfirmed(reason: "audio_drain_incomplete")] {
                XCTAssertEqual(TrackFailurePolicy.decide(track: track, otherTrack: other,
                                                         closing: false, restartAttempted: false),
                               .stopCapture, "\(track) \(other)")
            }
            XCTAssertEqual(TrackFailurePolicy.decide(track: track, otherTrack: .active,
                                                     closing: true, restartAttempted: false),
                           .stopCapture)
        }
    }

    func testTheRestartBoundIsStatedInSeconds() {
        XCTAssertEqual(TrackFailurePolicy.systemAudioRestartBound, 10)
    }

    func testTheActivityIsHeldOnceWhileCapturingAndReleasedOnceAfter() {
        var begun = 0
        var ended = 0
        let activity = CaptureActivity(begin: { begun += 1; return NSObject() },
                                       end: { _ in ended += 1 })
        XCTAssertFalse(activity.update(capturing: false))
        XCTAssertTrue(activity.update(capturing: true))
        XCTAssertTrue(activity.held)
        XCTAssertFalse(activity.update(capturing: true))
        XCTAssertEqual(begun, 1)
        XCTAssertTrue(activity.update(capturing: false))
        XCTAssertFalse(activity.held)
        XCTAssertFalse(activity.update(capturing: false))
        XCTAssertEqual(ended, 1)
        XCTAssertTrue(activity.update(capturing: true))
        XCTAssertEqual(begun, 2)
    }
}
