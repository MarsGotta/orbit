import XCTest
@testable import OrbitNativeState

final class MeetingApplicationsTests: XCTestCase {
    private func row(_ pid: Int32, _ bundle: String?, _ name: String?,
                     regular: Bool = true, terminated: Bool = false)
        -> (pid: Int32, bundleID: String?, name: String?, regular: Bool, terminated: Bool) {
        (pid: pid, bundleID: bundle, name: name, regular: regular, terminated: terminated)
    }

    func testBackgroundHelpersNeverLookLikeTheAppHoldingTheCall() {
        let reported = MeetingApplications.reportable([
            row(1, "us.zoom.xos", "zoom.us"),
            row(2, "com.apple.some.daemon", "daemon", regular: false)
        ])
        XCTAssertEqual(reported.map(\.applicationPID), [1])
    }

    func testAnApplicationThatAlreadyQuitIsNotOffered() {
        let reported = MeetingApplications.reportable([
            row(3, "us.zoom.xos", "zoom.us", terminated: true)
        ])
        XCTAssertTrue(reported.isEmpty)
    }

    func testEntriesWithoutAnIdentityAreDroppedRatherThanSentBlank() {
        // A nameless row downstream reads as a real candidate with no name.
        let reported = MeetingApplications.reportable([
            row(4, nil, "sin bundle"),
            row(5, "com.example.app", nil),
            row(6, "", "vacio")
        ])
        XCTAssertTrue(reported.isEmpty)
    }

    func testTheListStaysBoundedBecauseItRidesOnAKeystroke() {
        let many = (1...40).map { row(Int32($0), "com.example.app\($0)", "App \($0)") }
        XCTAssertEqual(MeetingApplications.reportable(many).count, MeetingApplications.maximum)
    }

    func testTheSameProcessIsNeverReportedTwice() {
        let reported = MeetingApplications.reportable([
            row(7, "us.zoom.xos", "zoom.us"),
            row(7, "us.zoom.xos", "zoom.us")
        ])
        XCTAssertEqual(reported.count, 1)
    }
}
