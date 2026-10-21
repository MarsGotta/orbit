import Foundation
import Darwin
import XCTest
@testable import OrbitNativeState

final class DiagnosticLogTests: XCTestCase {
    private func withRoot(_ body: (URL) throws -> Void) throws {
        let parent = FileManager.default.temporaryDirectory.resolvingSymlinksInPath()
        let root = parent.appendingPathComponent("orbit-log-" + UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false,
                                                attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: root) }
        try body(root)
    }

    private func lines(_ url: URL) throws -> [[String: Any]] {
        let text = try String(contentsOf: url, encoding: .utf8)
        return try text.split(separator: "\n").map { line in
            let object = try JSONSerialization.jsonObject(with: Data(line.utf8))
            return try XCTUnwrap(object as? [String: Any])
        }
    }

    func testEachRecordIsOneDecodableLineWithPrivatePermissions() throws {
        try withRoot { root in
            let directory = root.appendingPathComponent("logs", isDirectory: true)
            let log = DiagnosticLog(directory: directory, now: { Date(timeIntervalSince1970: 0) })
            log.record("host_launched", ["mode": "managed_stdio"])
            log.record("daemon_exited", ["status": "1"])
            let decoded = try lines(log.url)
            XCTAssertEqual(decoded.count, 2)
            XCTAssertEqual(decoded[0]["event"] as? String, "host_launched")
            XCTAssertEqual(decoded[0]["time"] as? String, "1970-01-01T00:00:00.000Z")
            XCTAssertEqual(decoded[0]["pid"] as? Int, Int(ProcessInfo.processInfo.processIdentifier))
            XCTAssertEqual((decoded[0]["fields"] as? [String: String])?["mode"], "managed_stdio")
            XCTAssertEqual(decoded[1]["event"] as? String, "daemon_exited")
            let directoryMode = try FileManager.default.attributesOfItem(atPath: directory.path)
            XCTAssertEqual((directoryMode[.posixPermissions] as? NSNumber)?.intValue, 0o700)
            let fileMode = try FileManager.default.attributesOfItem(atPath: log.url.path)
            XCTAssertEqual((fileMode[.posixPermissions] as? NSNumber)?.intValue, 0o600)
        }
    }

    func testAConversationSizedFieldCannotReachDiskWhole() throws {
        try withRoot { root in
            let log = DiagnosticLog(directory: root)
            let transcript = String(repeating: "secreto ", count: 500)
            log.record("never_log_content", ["leak": transcript])
            let stored = try XCTUnwrap((try lines(log.url)[0]["fields"] as? [String: String])?["leak"])
            XCTAssertLessThanOrEqual(stored.utf8.count, DiagnosticLog.maximumFieldBytes + 4)
            XCTAssertLessThan(stored.count, transcript.count)
            XCTAssertTrue(stored.hasSuffix("…"))
        }
    }

    func testTruncationKeepsTheLineDecodableOnAMultibyteBoundary() throws {
        try withRoot { root in
            let log = DiagnosticLog(directory: root)
            // Every character is three UTF-8 bytes, so the byte budget lands
            // mid-character unless the cut is moved back to a boundary.
            log.record("multibyte", ["value": String(repeating: "€", count: 400)])
            let stored = try XCTUnwrap((try lines(log.url)[0]["fields"] as? [String: String])?["value"])
            XCTAssertFalse(stored.unicodeScalars.contains("\u{FFFD}"))
        }
    }

    func testFieldCountIsBoundedAndSelectionIsStable() throws {
        try withRoot { root in
            let log = DiagnosticLog(directory: root)
            var fields = [String: String]()
            for index in 0..<40 { fields[String(format: "k%02d", index)] = "v" }
            log.record("many", fields)
            let stored = try XCTUnwrap(try lines(log.url)[0]["fields"] as? [String: String])
            XCTAssertEqual(stored.count, DiagnosticLog.maximumFields)
            XCTAssertEqual(stored.keys.sorted().first, "k00")
            XCTAssertNil(stored["k39"])
        }
    }

    func testRotationKeepsExactlyOnePreviousGeneration() throws {
        try withRoot { root in
            let log = DiagnosticLog(directory: root, maximumBytes: 300)
            for index in 0..<40 { log.record("filler", ["n": String(index)]) }
            let previous = root.appendingPathComponent("native.log.1")
            XCTAssertTrue(FileManager.default.fileExists(atPath: previous.path))
            let names = try FileManager.default.contentsOfDirectory(atPath: root.path).sorted()
            XCTAssertEqual(names, ["native.log", "native.log.1"])
            let size = try XCTUnwrap((try FileManager.default
                .attributesOfItem(atPath: log.url.path)[.size] as? NSNumber)?.intValue)
            XCTAssertLessThanOrEqual(size, 300)
            XCTAssertFalse(try lines(log.url).isEmpty)
        }
    }

    func testASymlinkAtTheLogPathIsRefusedAndItsTargetUntouched() throws {
        try withRoot { root in
            let target = root.appendingPathComponent("someone-elses-file")
            let original = Data("preserve this".utf8)
            try original.write(to: target)
            let logs = root.appendingPathComponent("logs", isDirectory: true)
            try FileManager.default.createDirectory(at: logs, withIntermediateDirectories: false,
                                                    attributes: [.posixPermissions: 0o700])
            try FileManager.default.createSymbolicLink(
                at: logs.appendingPathComponent("native.log"), withDestinationURL: target)
            DiagnosticLog(directory: logs).record("host_launched")
            XCTAssertEqual(try Data(contentsOf: target), original)
        }
    }

    func testAnUnwritableDestinationLosesTheLineInsteadOfFailing() throws {
        try withRoot { root in
            let blocked = root.appendingPathComponent("not-a-directory")
            try Data("fixture".utf8).write(to: blocked)
            let log = DiagnosticLog(directory: blocked)
            log.record("host_launched")
            XCTAssertEqual(try Data(contentsOf: blocked), Data("fixture".utf8))
        }
    }
}
