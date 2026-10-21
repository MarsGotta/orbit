import Foundation
import Darwin
import XCTest
@testable import OrbitNativeState

final class NativeStateTests: XCTestCase {
    private func physicalPath(_ path: String) throws -> String {
        var buffer = [CChar](repeating: 0, count: Int(PATH_MAX))
        let resolved: String? = buffer.withUnsafeMutableBufferPointer { storage in
            guard let address = storage.baseAddress,
                  realpath(path, address) != nil else { return nil }
            return String(validatingUTF8: address)
        }
        return try XCTUnwrap(resolved)
    }

    private func withRoot(_ body: (URL) throws -> Void) throws {
        // Match an external POSIX caller such as Python Path.resolve(), not
        // Foundation's system-directory aliases. Never mutate the system root.
        let parent = try physicalPath(FileManager.default.temporaryDirectory.path)
        let root = URL(fileURLWithPath: parent, isDirectory: true)
            .appendingPathComponent("orbit-native-state-" + UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false,
                                                attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: root) }
        try body(root)
    }

    func testFreshPrivateRootIsAcceptedWithoutCreatingFiles() throws {
        try withRoot { root in
            XCTAssertTrue(root.path.utf8.elementsEqual(try physicalPath(root.path).utf8))
            XCTAssertEqual(try NativeFixtureDirectory.validate(root.path), root)
            XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: root.path), [])
        }
    }

    func testSystemAliasCannotReplaceThePhysicalTemporaryRoot() throws {
        try withRoot { root in
            // This conditional is filesystem-dependent, not a claim that all
            // Macs place temporary files under /private. Explicit alias refusal
            // is also covered unconditionally by the local symlink test below.
            let alias = root.resolvingSymlinksInPath().path
            if !alias.utf8.elementsEqual(root.path.utf8) {
                XCTAssertThrowsError(try NativeFixtureDirectory.validate(alias))
            }
            XCTAssertEqual(try NativeFixtureDirectory.validate(root.path), root)
        }
    }

    func testGroupOrWorldAccessibleDirectoryIsRejectedWithoutChangingPermissions() throws {
        try withRoot { root in
            for mode in [0o755, 0o750, 0o777] {
                try FileManager.default.setAttributes([.posixPermissions: mode], ofItemAtPath: root.path)
                XCTAssertThrowsError(try NativeFixtureDirectory.validate(root.path))
                let attributes = try FileManager.default.attributesOfItem(atPath: root.path)
                XCTAssertEqual((attributes[.posixPermissions] as? NSNumber)?.intValue, mode)
            }
        }
    }

    func testNonemptyRootIsRejectedAndExistingContentsRemainExact() throws {
        try withRoot { root in
            let sentinel = root.appendingPathComponent("existing-state")
            let original = Data("fixture: preserve this existing file".utf8)
            try original.write(to: sentinel)
            XCTAssertThrowsError(try NativeFixtureDirectory.validate(root.path))
            XCTAssertEqual(try Data(contentsOf: sentinel), original)
            XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: root.path), ["existing-state"])
        }
    }

    func testSymlinkToPrivateEmptyDirectoryCannotBorrowItsEligibility() throws {
        try withRoot { root in
            let target = root.appendingPathComponent("target", isDirectory: true)
            let alias = root.appendingPathComponent("alias", isDirectory: true)
            try FileManager.default.createDirectory(at: target, withIntermediateDirectories: false,
                                                    attributes: [.posixPermissions: 0o700])
            try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: target)
            XCTAssertEqual(try NativeFixtureDirectory.validate(target.path), target)
            XCTAssertThrowsError(try NativeFixtureDirectory.validate(alias.path))
            XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: target.path), [])
        }
    }

    func testMissingFileAndMalformedPathsAreRejectedWithoutCreatingDirectories() throws {
        try withRoot { root in
            let missing = root.appendingPathComponent("missing", isDirectory: true)
            XCTAssertThrowsError(try NativeFixtureDirectory.validate(missing.path))
            XCTAssertFalse(FileManager.default.fileExists(atPath: missing.path))
            let file = root.appendingPathComponent("file")
            try Data().write(to: file)
            XCTAssertThrowsError(try NativeFixtureDirectory.validate(file.path))
            for path in ["relative", "/", root.path + "/", root.path + "/.",
                         root.path + "//child", root.path + "/../child", root.path + "\0",
                         root.path + "\u{0085}", "/" + String(repeating: "x", count: 4096)] {
                XCTAssertThrowsError(try NativeFixtureDirectory.validate(path))
            }
        }
    }

    func testInjectedLatchWritesOnlyItsRootAndSurvivesReopening() throws {
        try withRoot { root in
            let latch = PrivacyLatch(root: root)
            XCTAssertFalse(latch.stopped)
            try latch.stop()
            XCTAssertTrue(latch.stopped)
            let file = root.appendingPathComponent("native-privacy-stop")
            XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: root.path), ["native-privacy-stop"])
            XCTAssertEqual(try Data(contentsOf: file), Data())
            let attributes = try FileManager.default.attributesOfItem(atPath: file.path)
            XCTAssertEqual((attributes[.posixPermissions] as? NSNumber)?.intValue, 0o600)
            XCTAssertTrue(PrivacyLatch(root: root).stopped)
            XCTAssertThrowsError(try NativeFixtureDirectory.validate(root.path))
        }
    }

    func testExplicitLocalRearmRemovesOnlyTheLatch() throws {
        try withRoot { root in
            let latch = PrivacyLatch(root: root)
            try latch.stop()
            let sentinel = root.appendingPathComponent("unrelated-fixture")
            let original = Data("fixture".utf8)
            try original.write(to: sentinel)
            try latch.rearmLocally()
            XCTAssertFalse(latch.stopped)
            XCTAssertFalse(PrivacyLatch(root: root).stopped)
            XCTAssertEqual(try Data(contentsOf: sentinel), original)
        }
    }

    func testLatchStopDoesNotFollowOrTruncateALeafSymlink() throws {
        try withRoot { root in
            let target = root.appendingPathComponent("preserved")
            let original = Data("never truncate this fixture".utf8)
            try original.write(to: target)
            let latchFile = root.appendingPathComponent("native-privacy-stop")
            try FileManager.default.createSymbolicLink(at: latchFile, withDestinationURL: target)
            let latch = PrivacyLatch(root: root)
            XCTAssertThrowsError(try latch.stop())
            XCTAssertTrue(latch.stopped)
            XCTAssertEqual(try Data(contentsOf: target), original)
        }
    }

    func testUnusableLatchRootFailsClosed() throws {
        try withRoot { root in
            let file = root.appendingPathComponent("not-a-directory")
            try Data("fixture".utf8).write(to: file)
            let latch = PrivacyLatch(root: file)
            XCTAssertTrue(latch.stopped)
            XCTAssertThrowsError(try latch.stop())
            XCTAssertTrue(latch.stopped)
        }
    }
}
