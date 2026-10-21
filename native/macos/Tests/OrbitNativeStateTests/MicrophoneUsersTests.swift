import XCTest
@testable import OrbitNativeState

final class MicrophoneUsersTests: XCTestCase {
    private let zoom = ResponsibleApplication(pid: 300, bundleID: "us.zoom.xos", name: "zoom.us")
    private let chrome = ResponsibleApplication(pid: 200, bundleID: "com.google.Chrome", name: "Google Chrome")
    private let orbit = ResponsibleApplication(pid: 100, bundleID: "app.orbit.companion", name: "Orbit")

    private func resolve(_ table: [Int32: ResponsibleApplication]) -> (Int32) -> ResponsibleApplication? {
        { table[$0] }
    }

    func testOnlyProcessesRunningInputCount() {
        let result = MicrophoneUsers.reportable(
            [.init(pid: 300, runningInput: true), .init(pid: 200, runningInput: false)],
            ownPID: 1, ownBundlePrefix: "app.orbit", resolve: resolve([300: zoom, 200: chrome]))
        XCTAssertEqual(result.map(\.bundleID), ["us.zoom.xos"])
    }

    func testABrowserHelperIsReportedAsItsBrowserOnce() {
        let result = MicrophoneUsers.reportable(
            [.init(pid: 201, runningInput: true), .init(pid: 202, runningInput: true)],
            ownPID: 1, ownBundlePrefix: "app.orbit", resolve: resolve([201: chrome, 202: chrome]))
        XCTAssertEqual(result, [MeetingApplication(applicationPID: 200, bundleID: "com.google.Chrome", name: "Google Chrome")])
    }

    func testOrbitNeverDetectsItsOwnRecording() {
        let byPID = MicrophoneUsers.reportable(
            [.init(pid: 100, runningInput: true)],
            ownPID: 100, ownBundlePrefix: "nothing", resolve: resolve([100: orbit]))
        let byBundle = MicrophoneUsers.reportable(
            [.init(pid: 101, runningInput: true)],
            ownPID: 1, ownBundlePrefix: "app.orbit", resolve: resolve([101: orbit]))
        XCTAssertEqual(byPID, [])
        XCTAssertEqual(byBundle, [])
    }

    func testAProcessWithoutAVisibleApplicationIsLeftOut() {
        let result = MicrophoneUsers.reportable(
            [.init(pid: 50, runningInput: true)],
            ownPID: 1, ownBundlePrefix: "app.orbit", resolve: { _ in nil })
        XCTAssertEqual(result, [])
    }

    func testTheSameSetAlwaysComparesEqual() {
        let processes: [MicrophoneProcess] = [.init(pid: 300, runningInput: true), .init(pid: 201, runningInput: true)]
        let table = resolve([300: zoom, 201: chrome])
        let first = MicrophoneUsers.reportable(processes, ownPID: 1, ownBundlePrefix: "app.orbit", resolve: table)
        let second = MicrophoneUsers.reportable(processes.reversed(), ownPID: 1, ownBundlePrefix: "app.orbit", resolve: table)
        XCTAssertEqual(first, second)
        XCTAssertEqual(first.map(\.applicationPID), [200, 300])
    }

    func testTheListIsBounded() {
        var processes = [MicrophoneProcess]()
        var table = [Int32: ResponsibleApplication]()
        for pid in Int32(1000)..<1040 {
            processes.append(.init(pid: pid, runningInput: true))
            table[pid] = .init(pid: pid, bundleID: "com.example.\(pid)", name: "App \(pid)")
        }
        let result = MicrophoneUsers.reportable(processes, ownPID: 1, ownBundlePrefix: "app.orbit", resolve: resolve(table))
        XCTAssertEqual(result.count, MicrophoneUsers.maximum)
    }
}
