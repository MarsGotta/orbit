import Foundation

// «Adjuntar ventana» without the engine (`window_text`) and a HEIC photo
// written as JPEG before it is attached (`photo_jpeg`), as the daemon sends
// them, and the pure walk that turns a window's Accessibility elements into
// its text. The bounds come with the command; no limit is copied here. The
// text is the user's content: nothing here logs it.

/// The bounds of one read of a window's text.
public struct WindowTextLimits: Decodable, Sendable, Equatable {
    public let max_bytes: UInt32
    public let max_nodes: UInt32
    public let deadline_ms: UInt64

    public init(max_bytes: UInt32, max_nodes: UInt32, deadline_ms: UInt64) {
        self.max_bytes = max_bytes
        self.max_nodes = max_nodes
        self.deadline_ms = deadline_ms
    }
}

/// The window just captured, by its process and its window number.
public struct NativeWindowTextFields: Decodable, Sendable {
    public let application_pid: Int32
    public let window_id: UInt32
    public let limits: WindowTextLimits
}

/// The photo to read and where the daemon wants its JPEG.
public struct NativePhotoJPEGFields: Decodable, Sendable {
    public let source_path: String
    public let destination_path: String
}

/// One Accessibility element as the walk needs it: whether it is a secure
/// field (whose text is never asked for), the texts it shows (title, value,
/// description) and its children.
public struct WindowTextNode<Element> {
    public let secure: Bool
    public let texts: [String]
    public let children: [Element]

    public init(secure: Bool, texts: [String], children: [Element]) {
        self.secure = secure
        self.texts = texts
        self.children = children
    }
}

/// Read-only access to a window's elements.
public protocol WindowTextAccess {
    associatedtype Element
    mutating func node(_ element: Element) throws -> WindowTextNode<Element>
}

/// Which bound stopped the walk before the window's end.
public enum WindowTextStop: String, Equatable, Sendable {
    case bytes, nodes, deadline
}

public enum WindowTextReader {
    /// The window's text: one line per element that shows some, in reading
    /// order (depth first), within `limits`. An element that cannot be read
    /// is left out with what hangs from it; a secure field and its children
    /// are never read. `elapsedMilliseconds` says how long the walk has run.
    public static func read<A: WindowTextAccess>(using access: inout A, window: A.Element,
                                                 limits: WindowTextLimits,
                                                 elapsedMilliseconds: () -> UInt64)
        -> (text: String, stopped: WindowTextStop?) {
        var lines = [String]()
        var bytes = 0
        var visited: UInt32 = 0
        var pending = [window]
        while let element = pending.popLast() {
            if visited >= limits.max_nodes { return (lines.joined(separator: "\n"), .nodes) }
            if elapsedMilliseconds() >= limits.deadline_ms {
                return (lines.joined(separator: "\n"), .deadline)
            }
            visited += 1
            guard let node = try? access.node(element), !node.secure else { continue }
            pending.append(contentsOf: node.children.reversed())
            let line = Self.line(node.texts)
            // A cell and the text inside it often say the same.
            guard !line.isEmpty, line != lines.last else { continue }
            let separator = lines.isEmpty ? 0 : 1
            let room = Int(limits.max_bytes) - bytes - separator
            if line.utf8.count > room {
                let kept = Self.prefix(line, bytes: room)
                if !kept.isEmpty { lines.append(kept) }
                return (lines.joined(separator: "\n"), .bytes)
            }
            lines.append(line)
            bytes += separator + line.utf8.count
        }
        return (lines.joined(separator: "\n"), nil)
    }

    /// What one element shows, each different text once.
    static func line(_ texts: [String]) -> String {
        var seen = [String]()
        for text in texts {
            let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmed.isEmpty, !seen.contains(trimmed) { seen.append(trimmed) }
        }
        return seen.joined(separator: " · ")
    }

    /// The longest start of `text` within `bytes` of UTF-8, whole characters only.
    static func prefix(_ text: String, bytes: Int) -> String {
        var used = 0
        var kept = ""
        for character in text {
            let size = String(character).utf8.count
            if used + size > bytes { break }
            used += size
            kept.append(character)
        }
        return kept
    }
}
