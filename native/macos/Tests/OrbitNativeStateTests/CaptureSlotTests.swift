import XCTest
@testable import OrbitNativeState

final class CaptureSlotTests: XCTestCase {
    func testA1EveryCaptureSlotTransitionIncludingFailedStart() {
        let reason = "capture_callback_at_stop_boundary"
        let unconfirmed = CaptureSlot.unconfirmed(reason: reason)
        XCTAssertFalse(unconfirmed.isActive)
        XCTAssertTrue(unconfirmed.drainUnconfirmed)
        let legalTransitions: [(CaptureSlot, CaptureSlot)] = [
            (.idle, .starting),
            (.starting, .active),
            (.starting, .idle),
            (.starting, .draining),
            (.active, .draining),
            (.draining, .idle),
            (.draining, .unconfirmed(reason: reason)),
            (.unconfirmed(reason: reason), .starting),
        ]

        for (origin, destination) in legalTransitions {
            var slot = origin
            XCTAssertTrue(slot.transition(to: destination))
            XCTAssertEqual(slot, destination)
        }

        let illegalTransitions: [(CaptureSlot, CaptureSlot)] = [
            (.idle, .active),
            (.active, .idle),
            (.unconfirmed(reason: reason), .active),
        ]
        for (origin, destination) in illegalTransitions {
            var slot = origin
            XCTAssertFalse(slot.transition(to: destination))
            XCTAssertEqual(slot, origin)
        }
    }

}

