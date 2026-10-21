import Foundation
import Darwin

/// Durable diagnostic trail for the macOS host process.
///
/// Orbit had no persistent log at all, so a host that died during launch left
/// nothing behind: stderr goes nowhere for a bundled app, and the only evidence
/// was the absence of a menu bar icon. This writes one JSON object per line to a
/// file that survives the crash it is describing.
///
/// Two rules are structural, not stylistic:
///   - No conversation content and no secrets. Callers pass an event name and
///     short labelled fields; every value is truncated, so a caller that leaks a
///     transcript into a field still cannot write a transcript to disk.
///   - Logging never fails the product. A log that cannot be written is a lost
///     diagnostic, not a reason to refuse a recording, so writes swallow errors.
public final class DiagnosticLog {
    public static let maximumFieldBytes = 200
    public static let maximumFields = 12

    private let directory: URL
    private let fileName: String
    private let maximumBytes: Int
    private let now: () -> Date
    private let worker = DispatchQueue(label: "orbit.native.diagnostics", qos: .utility)
    private let mutex = NSLock()
    private var descriptor: Int32 = -1

    public var url: URL { directory.appendingPathComponent(fileName) }

    /// `directory` defaults to the user's own log folder, where Console.app and
    /// a plain `cat` both find it without special tooling.
    public init(directory providedDirectory: URL? = nil,
                fileName: String = "native.log",
                maximumBytes: Int = 1_048_576,
                now: @escaping () -> Date = Date.init) {
        self.directory = providedDirectory ?? FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/Orbit", isDirectory: true)
        self.fileName = fileName
        self.maximumBytes = maximumBytes
        self.now = now
    }

    deinit {
        if descriptor >= 0 { close(descriptor) }
    }

    /// Records one event. Safe to call from any queue and from a failure path.
    /// Not for per-sample or per-frame use: every line is flushed to disk so the
    /// final line before a crash is not lost in a buffer.
    public func record(_ event: String, _ fields: [String: String] = [:]) {
        guard let line = encode(event: event, fields: fields) else { return }
        mutex.lock()
        defer { mutex.unlock() }
        rotateIfNeeded(adding: line.count)
        guard ensureOpen() else { return }
        write(line)
    }

    /// Runtime callers use this path so JSON encoding, writes and fsync never
    /// occupy AppKit's run loop. The queued closure retains the log until done.
    public func recordOffMain(_ event: String, _ fields: [String: String] = [:]) {
        worker.async { [self] in record(event, fields) }
    }

    /// Termination keeps the durability guarantee while executing the syscall
    /// on the worker. Waiting here cannot delay an interactive run loop.
    public func recordAndWaitOffMain(_ event: String, _ fields: [String: String] = [:]) {
        worker.sync { [self] in record(event, fields) }
    }

    private func encode(event: String, fields: [String: String]) -> Data? {
        var payload: [String: Any] = [
            "time": Self.timestamp(now()),
            "pid": ProcessInfo.processInfo.processIdentifier,
            "event": Self.clamp(event)
        ]
        if !fields.isEmpty {
            var clamped = [String: String]()
            // Sorted so a truncated set is the same on every run rather than
            // whichever keys the dictionary happened to hash first.
            for key in fields.keys.sorted().prefix(Self.maximumFields) {
                clamped[Self.clamp(key)] = Self.clamp(fields[key] ?? "")
            }
            payload["fields"] = clamped
        }
        guard var data = try? JSONSerialization.data(withJSONObject: payload,
                                                     options: [.sortedKeys]) else { return nil }
        data.append(0x0A)
        return data
    }

    /// Truncates on a UTF-8 boundary: a cut mid-character would produce bytes
    /// that no JSON reader can decode, losing the whole line instead of a tail.
    static func clamp(_ value: String) -> String {
        var bytes = Array(value.utf8)
        guard bytes.count > maximumFieldBytes else { return value }
        bytes = Array(bytes.prefix(maximumFieldBytes))
        while !bytes.isEmpty, String(decoding: bytes, as: UTF8.self).unicodeScalars.contains("\u{FFFD}") {
            bytes.removeLast()
        }
        return String(decoding: bytes, as: UTF8.self) + "…"
    }

    static func timestamp(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy-MM-dd'T'HH:mm:ss.SSS'Z'"
        return formatter.string(from: date)
    }

    private func ensureOpen() -> Bool {
        if descriptor >= 0 { return true }
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
                                                 attributes: [.posixPermissions: 0o700])
        // O_NOFOLLOW for the same reason the privacy latch uses it: a symlink
        // planted at this path must not redirect writes to somebody else's file.
        let opened = open(url.path, O_WRONLY | O_APPEND | O_CREAT | O_NOFOLLOW, mode_t(0o600))
        guard opened >= 0 else { return false }
        descriptor = opened
        return true
    }

    private func write(_ line: Data) {
        line.withUnsafeBytes { buffer in
            guard var pointer = buffer.baseAddress else { return }
            var remaining = buffer.count
            while remaining > 0 {
                let written = Darwin.write(descriptor, pointer, remaining)
                // A partial write is normal for a pipe and possible for a file;
                // stopping here would leave a half line that breaks the reader.
                if written > 0 {
                    pointer += written
                    remaining -= written
                } else if written < 0 && errno == EINTR {
                    continue
                } else {
                    return
                }
            }
        }
        fsync(descriptor)
    }

    /// Keeps one previous file. Unbounded logs on a laptop are a disk problem,
    /// and a single generation is enough to hold the launch before the crash.
    private func rotateIfNeeded(adding bytes: Int) {
        guard maximumBytes > 0 else { return }
        var info = stat()
        let size: Int
        if descriptor >= 0, fstat(descriptor, &info) == 0 {
            size = Int(info.st_size)
        } else if stat(url.path, &info) == 0 {
            size = Int(info.st_size)
        } else {
            return
        }
        guard size + bytes > maximumBytes else { return }
        if descriptor >= 0 {
            close(descriptor)
            descriptor = -1
        }
        let previous = directory.appendingPathComponent(fileName + ".1")
        try? FileManager.default.removeItem(at: previous)
        try? FileManager.default.moveItem(at: url, to: previous)
    }
}
