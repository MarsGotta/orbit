import XCTest
@testable import OrbitNativeProtocol

final class NativeLaunchOptionsTests: XCTestCase {
    private func isolated(_ path: String = "/tmp/orbit-native-fixture") throws -> NativeLaunchOptions {
        try NativeLaunchOptions.parse(["--stdio", "--test-state-directory", path])
    }

    /// Without arguments the helper used to become the menu bar
    /// host that launched the daemon. That chain is gone; the helper only runs
    /// as the daemon's child or by hand over stdio, so no arguments is refused.
    func testNoArgumentsIsRefusedNowThatTheHelperNeverHostsTheDaemon() {
        assertRejected([], as: .invalidArguments)
    }

    func testUnmanagedStdioIsRefused() {
        assertRejected(["--stdio"], as: .invalidArguments)
    }

    func testExplicitFixtureKeepsTheExactDirectory() throws {
        let path = "/private/tmp/Orbit fixture/cafe\u{0301}-\u{1F680}"
        let options = try isolated(path)
        XCTAssertEqual(options.mode, .isolatedStdio(stateDirectory: path))
        XCTAssertTrue(options.isIsolatedFixture)
        let observed = try XCTUnwrap(options.stateDirectory)
        XCTAssertEqual(Array(observed.utf8), Array(path.utf8))
    }

    func testFixtureAllowsOnlyStateAndStopCommands() throws {
        let options = try isolated()
        for command in ["hello", "heartbeat", "status", "privacy_stop", "capture_stop", "shutdown"] {
            XCTAssertTrue(options.permits(command: command), command)
        }
    }

    func testFixtureRefusesCapturePermissionsPlaybackRearmAndUnknownCommands() throws {
        let options = try isolated()
        for command in ["rearm", "permissions", "audio_start", "audio_stop", "playback",
            "playback_stop", "screen_sources", "screenshot", "selected_text",
            "", "unknown", "STATUS", "status ", " status", "status\0", "status\n"] {
            XCTAssertFalse(options.permits(command: command), command.debugDescription)
        }
    }

    func testNormalModesLeaveCommandAuthorityToTheExistingProtocol() throws {
        for arguments in [["--stdio", "--managed"]] {
            let options = try NativeLaunchOptions.parse(arguments)
            for command in ["hello", "rearm", "permissions", "audio_start", "unknown", ""] {
                XCTAssertTrue(options.permits(command: command), command)
            }
        }
    }

    func testUnknownFlagsAndExtraArgumentsAreNotIgnored() {
        for arguments in [["--unknown"], ["--help"], ["--stdio", "extra"],
            ["OrbitNative", "--stdio"], ["--stdio", "--test-state-directory", "/tmp/test", "extra"],
            ["--stdio", "--test-state-directory=/tmp/test"], ["--stdio "]] {
            assertRejected(arguments, as: .invalidArguments)
        }
    }

    func testDuplicateFlagsAreRejectedInsteadOfTakingTheLastValue() {
        for arguments in [["--stdio", "--stdio"], ["--stdio", "--stdio", "/tmp/test"],
            ["--stdio", "--test-state-directory", "/tmp/test", "--stdio"],
            ["--stdio", "--test-state-directory", "/tmp/a", "--test-state-directory", "/tmp/b"]] {
            assertRejected(arguments, as: .invalidArguments)
        }
    }

    func testMissingValuesAndUnsupportedOrderAreRejected() {
        for arguments in [["--test-state-directory"], ["--test-state-directory", "/tmp/test"],
            ["--stdio", "--test-state-directory"],
            ["--test-state-directory", "/tmp/test", "--stdio"]] {
            assertRejected(arguments, as: .invalidArguments)
        }
        assertRejected(["--stdio", "--test-state-directory", "--stdio"], as: .invalidStateDirectory)
    }

    func testRelativeEmptyAndRootPathsAreRejected() {
        for path in ["", "tmp/test", "~/test", ".", "..", "/"] {
            assertRejected(["--stdio", "--test-state-directory", path], as: .invalidStateDirectory)
        }
    }

    func testNonCanonicalSeparatorsAndDotSegmentsAreNotRepaired() {
        for path in ["//tmp/test", "/tmp//test", "/tmp/test/", "/tmp/./test",
            "/tmp/../test", "/tmp/.", "/tmp/..", "/./test", "/../test"] {
            assertRejected(["--stdio", "--test-state-directory", path], as: .invalidStateDirectory)
        }
    }

    func testEveryC0DelAndC1ControlIsRejected() {
        for code in Array(UInt32(0)...31) + Array(UInt32(127)...159) {
            guard let scalar = UnicodeScalar(code) else {
                XCTFail("The tested control code must be a valid Unicode scalar")
                continue
            }
            let path = "/tmp/test" + String(scalar) + "directory"
            assertRejected(["--stdio", "--test-state-directory", path], as: .invalidStateDirectory)
        }
    }

    func testPathLimitCountsUTF8BytesNotCharacters() throws {
        let asciiLimit = "/" + String(repeating: "a", count: 4095)
        XCTAssertEqual(try isolated(asciiLimit).stateDirectory, asciiLimit)
        let unicodeLimit = "/" + String(repeating: "\u{00E9}", count: 2047) + "a"
        XCTAssertEqual(unicodeLimit.utf8.count, 4096)
        XCTAssertEqual(try isolated(unicodeLimit).stateDirectory, unicodeLimit)
        for path in [asciiLimit + "a", unicodeLimit + "a"] {
            assertRejected(["--stdio", "--test-state-directory", path], as: .invalidStateDirectory)
        }
    }

    func testParsingDoesNotExpandOrNormalizePathContent() throws {
        for path in ["/tmp/$HOME", "/tmp/$(command)", "/tmp/test..directory",
            "/tmp/.hidden", "/tmp/a\\b", "/tmp/%2e%2e", "/tmp/name with spaces"] {
            let observed = try XCTUnwrap(isolated(path).stateDirectory)
            XCTAssertEqual(Array(observed.utf8), Array(path.utf8))
        }
    }

    /// `--stdio --managed` is the production reverse mode the daemon
    /// launches. It keeps the pipe transport and admits `rearm` over it, with
    /// the readiness checks living in the application.
    func testManagedStdioParsesWithStdioAndManagedSemanticsAndAdmitsRearm() throws {
        let options = try NativeLaunchOptions.parse(["--stdio", "--managed"])
        XCTAssertEqual(options.mode, .managedStdio)
        XCTAssertTrue(options.isManaged)
        XCTAssertFalse(options.isIsolatedFixture)
        XCTAssertTrue(options.permits(command: "rearm"),
                      "managed stdio must admit rearm over the daemon's private pipe")
    }

    /// The name each mode reports in `ready` and in its diagnostics; the
    /// isolated one is what `scripts/macos/smoke-native.py` requires.
    func testLaunchModeNamesMatchWhatReadyReports() throws {
        XCTAssertEqual(try NativeLaunchOptions.parse(["--stdio", "--managed"]).launchModeName,
                       "managed_stdio")
        XCTAssertEqual(try isolated().launchModeName, "isolated_test_stdio")
    }

    private func assertRejected(
        _ arguments: [String], as expected: NativeLaunchOptions.ParseError,
        file: StaticString = #filePath, line: UInt = #line
    ) {
        XCTAssertThrowsError(try NativeLaunchOptions.parse(arguments), file: file, line: line) { error in
            XCTAssertEqual(error as? NativeLaunchOptions.ParseError, expected, file: file, line: line)
        }
    }
}
