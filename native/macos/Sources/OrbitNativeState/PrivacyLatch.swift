import Foundation
import Darwin

/// Fail closed if the latch location cannot be created or inspected.
public final class PrivacyLatch {
    private let url: URL
    private let worker = DispatchQueue(label: "orbit.native.privacy-latch", qos: .userInitiated)
    public private(set) var stopped: Bool

    public init(root providedRoot: URL? = nil) {
        let root = providedRoot ?? FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/Orbit", isDirectory: true)
        url = root.appendingPathComponent("native-privacy-stop")
        do {
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true,
                                                    attributes: [.posixPermissions: 0o700])
            stopped = FileManager.default.fileExists(atPath: url.path)
        } catch { stopped = true }
    }

    public func stop() throws {
        stopped = true
        try Self.persistStop(at: url)
    }

    /// Makes revocation visible synchronously; durability follows on the worker.
    package func beginStop() { stopped = true }

    package func persistStopOffMain() async throws {
        let url = self.url
        try await performOffMain { try Self.persistStop(at: url) }
    }

    /// Pipe rearm is valid only after the managed host has established that
    /// the command came from the local daemon that launched it. This object
    /// only performs the durable filesystem step; the host retains the
    /// authority decision.
    package func removeStopOffMain() async throws {
        let url = self.url
        try await performOffMain { try Self.removeStop(at: url) }
    }

    package func finishRearm() { stopped = false }

    private static func persistStop(at url: URL) throws {
        let descriptor = open(url.path, O_WRONLY | O_CREAT | O_TRUNC | O_NOFOLLOW, mode_t(0o600))
        guard descriptor >= 0 else { throw NativeStateFailure.latchWrite }
        defer { close(descriptor) }
        guard fsync(descriptor) == 0 else { throw NativeStateFailure.latchSync }
        try syncParent(of: url)
    }

    public func rearmLocally() throws {
        try Self.removeStop(at: url)
        stopped = false
    }

    private static func removeStop(at url: URL) throws {
        if FileManager.default.fileExists(atPath: url.path) {
            try FileManager.default.removeItem(at: url)
        }
        try syncParent(of: url)
    }

    private static func syncParent(of url: URL) throws {
        let descriptor = open(url.deletingLastPathComponent().path, O_RDONLY)
        guard descriptor >= 0 else { throw NativeStateFailure.latchDirectory }
        defer { close(descriptor) }
        guard fsync(descriptor) == 0 else { throw NativeStateFailure.latchDirectorySync }
    }

    private func performOffMain(_ operation: @escaping @Sendable () throws -> Void) async throws {
        try await withCheckedThrowingContinuation { continuation in
            worker.async {
                do {
                    try operation()
                    continuation.resume(returning: ())
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }
}

enum NativeStateFailure: Error {
    case latchWrite, latchSync, latchDirectory, latchDirectorySync, invalidFixtureDirectory
}
