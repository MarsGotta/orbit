import Foundation
import XCTest
@testable import OrbitNativeProtocol

final class NativeQuitPolicyTests: XCTestCase {
    private func id(_ value: UInt8) -> UUID {
        UUID(uuid: (0, 0, 0, 0, 0, 0, 0x40, 0,
                    0x80, 0, 0, 0, 0, 0, 0, value))
    }

    private static let failures = [
        "daemon_disconnected", "daemon_output_disconnected", "invalid_protocol_or_disconnected",
        "native_command_backpressure", "native_event_too_large",
    ]

    func testInitialStateHasNoRequestAndIgnoresUnsolicitedFlushResults() {
        var policy = NativeQuitPolicy()
        let idle = policy
        XCTAssertFalse(policy.isRequested)
        for succeeded in [false, true] {
            XCTAssertEqual(policy.completeFlush(id: id(1), succeeded: succeeded), .none)
            XCTAssertEqual(policy, idle)
        }
    }

    func testBeginFlushAdmitsOnlyOneIdentityWithoutClaimingTermination() {
        var policy = NativeQuitPolicy()
        XCTAssertTrue(policy.beginFlush(id: id(1)))
        XCTAssertTrue(policy.isRequested)
        let flushing = policy
        XCTAssertFalse(policy.beginFlush(id: id(1)))
        XCTAssertFalse(policy.beginFlush(id: id(2)))
        XCTAssertEqual(policy, flushing)
    }

    func testStaleSuccessAndFailureCannotConsumeCurrentFlush() {
        var policy = NativeQuitPolicy()
        XCTAssertTrue(policy.beginFlush(id: id(2)))
        let current = policy
        for succeeded in [false, true] {
            XCTAssertEqual(policy.completeFlush(id: id(1), succeeded: succeeded), .none)
            XCTAssertEqual(policy, current)
        }
        XCTAssertEqual(policy.completeFlush(id: id(2), succeeded: true), .terminate)
    }

    func testSuccessfulFlushTerminatesOnceWithoutWaitingForEOF() {
        var policy = NativeQuitPolicy()
        XCTAssertTrue(policy.beginFlush(id: id(1)))
        XCTAssertEqual(policy.completeFlush(id: id(1), succeeded: true), .terminate)
        XCTAssertTrue(policy.isRequested)
        let finished = policy
        XCTAssertFalse(policy.beginFlush(id: id(2)))
        XCTAssertEqual(policy.completeFlush(id: id(1), succeeded: true), .none)
        XCTAssertEqual(policy.disconnected(reason: "daemon_disconnected"), .none)
        XCTAssertEqual(policy, finished)
    }

    func testFinishedStateAbsorbsEveryLateResultAndDisconnectWithoutAnotherEffect() {
        var policy = NativeQuitPolicy()
        XCTAssertTrue(policy.beginFlush(id: id(1)))
        XCTAssertEqual(policy.completeFlush(id: id(1), succeeded: true), .terminate)
        let finished = policy
        for reason in ["daemon_disconnected", "daemon_output_disconnected", "unknown_failure"] {
            XCTAssertEqual(policy.disconnected(reason: reason), .none)
            XCTAssertEqual(policy.completeFlush(id: id(1), succeeded: false), .none)
            XCTAssertEqual(policy.completeFlush(id: id(2), succeeded: true), .none)
            XCTAssertFalse(policy.beginFlush(id: id(3)))
            XCTAssertTrue(policy.isRequested)
            XCTAssertEqual(policy, finished)
        }
    }

    func testPolicyCopiesDoNotShareStateOrGenerateEffectsImplicitly() {
        let original = NativeQuitPolicy()
        var copy = original
        XCTAssertTrue(copy.beginFlush(id: id(1)))
        XCTAssertFalse(original.isRequested)
        XCTAssertTrue(copy.isRequested)
        XCTAssertNotEqual(original, copy)
        XCTAssertEqual(NativeQuitPolicy(), original)
    }

    // MARK: The managed helper does not outlive its daemon.

    /// The daemon closed the pipe — an orderly shutdown's EOF or a crash's —
    /// and nobody can command this helper again: it stops and leaves, once.
    func testManagedHelperLeavesWhenItsDaemonClosesThePipe() {
        for reason in Self.failures {
            var policy = NativeQuitPolicy()
            XCTAssertEqual(policy.disconnected(reason: reason), .stopThenTerminate, reason)
            XCTAssertTrue(policy.isRequested)
            let leaving = policy
            XCTAssertEqual(policy.disconnected(reason: "daemon_disconnected"), .none, reason)
            XCTAssertFalse(policy.beginFlush(id: id(1)), reason)
            XCTAssertEqual(policy.completeFlush(id: id(1), succeeded: true), .none, reason)
            XCTAssertEqual(policy, leaving, reason)
        }
    }

    /// The daemon sends `shutdown` and then closes the pipe, so the EOF can
    /// arrive while the helper is still flushing its answer. It leaves anyway;
    /// the flush's late success adds nothing.
    func testManagedHelperLeavesWhenTheEOFArrivesDuringItsShutdownFlush() {
        var policy = NativeQuitPolicy()
        XCTAssertTrue(policy.beginFlush(id: id(1)))
        XCTAssertEqual(policy.disconnected(reason: "daemon_disconnected"), .stopThenTerminate)
        XCTAssertEqual(policy.completeFlush(id: id(1), succeeded: true), .none)
        XCTAssertEqual(policy.completeFlush(id: id(1), succeeded: false), .none)
    }

    /// A failed flush used to leave the helper latched and alive for good.
    /// Managed, the EOF that follows still makes it leave.
    func testManagedHelperLeavesOnEOFAfterAFailedFlush() {
        var policy = NativeQuitPolicy()
        XCTAssertTrue(policy.beginFlush(id: id(1)))
        XCTAssertEqual(policy.completeFlush(id: id(1), succeeded: false), .none)
        XCTAssertTrue(policy.isRequested)
        XCTAssertFalse(policy.beginFlush(id: id(2)))
        XCTAssertEqual(policy.completeFlush(id: id(1), succeeded: true), .none)
        XCTAssertEqual(policy.disconnected(reason: "daemon_disconnected"), .stopThenTerminate)
    }

    /// The helper must be able to leave by itself before the daemon kills it:
    /// the daemon's `HELPER_EXIT_GRACE` is 3 s.
    func testTheOrphanGraceIsShortAndUnderTheDaemonsKillGrace() {
        XCTAssertGreaterThan(NativeQuitPolicy.orphanExitGrace, 0)
        XCTAssertLessThan(NativeQuitPolicy.orphanExitGrace, 3)
    }
}
