import Foundation

// The assistant's folders, as the daemon sends them (`assistant_folders`,
// `assistant_files_list|search|read`) and the pure rules the helper applies
// while answering. Which path a request may reach is decided by the daemon
// (`orbit_core::assistant_files::admit`) on the canonical paths this helper
// reports; here the helper only refuses to read a path that is no longer the
// canonical one it was given, and bounds what it returns by the numbers the
// command carries. No limit is copied here.

public struct AssistantFolderBookmark: Decodable, Sendable, Equatable {
    public let id: String
    public let bookmark_base64: String
    public init(id: String, bookmark_base64: String) {
        self.id = id
        self.bookmark_base64 = bookmark_base64
    }
}

public struct AssistantFoldersFields: Decodable, Sendable {
    public let folders: [AssistantFolderBookmark]
    public let path: String?
}

public struct AssistantFilesListFields: Decodable, Sendable {
    public let folder: AssistantFolderBookmark
    public let path: String
    public let after_index: UInt64
    public let max_entries: UInt32
}

public struct AssistantSearchLimits: Decodable, Sendable, Equatable {
    public let max_results: UInt32
    public let max_visited: UInt32
    public let max_files_read: UInt32
    public let max_bytes_read: UInt64
    public let max_file_bytes: UInt64
    public let max_snippet_chars: UInt32
    public let deadline_ms: UInt64
}

public struct AssistantSearchRoot: Decodable, Sendable {
    public let folder: AssistantFolderBookmark
    public let path: String
}

public struct AssistantFilesSearchFields: Decodable, Sendable {
    public let roots: [AssistantSearchRoot]
    public let query: String
    public let content: Bool
    public let limits: AssistantSearchLimits
}

/// How one file is read, as the daemon decided from its extension.
public enum AssistantReadAs: Decodable, Sendable, Equatable {
    case text(afterIndex: UInt64, maxBytes: UInt32)
    case pdf(afterPage: UInt32, afterIndex: UInt64, maxBytes: UInt32, maxPages: UInt32)
    case image(maxPNGBytes: UInt32, maxWidth: UInt32, maxHeight: UInt32)

    private enum CodingKeys: String, CodingKey {
        case `as`, after_index, max_bytes, after_page, max_pages, max_png_bytes, max_width, max_height
    }

    public init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        switch try fields.decode(String.self, forKey: .as) {
        case "text":
            self = .text(afterIndex: try fields.decode(UInt64.self, forKey: .after_index),
                         maxBytes: try fields.decode(UInt32.self, forKey: .max_bytes))
        case "pdf":
            self = .pdf(afterPage: try fields.decode(UInt32.self, forKey: .after_page),
                        afterIndex: try fields.decode(UInt64.self, forKey: .after_index),
                        maxBytes: try fields.decode(UInt32.self, forKey: .max_bytes),
                        maxPages: try fields.decode(UInt32.self, forKey: .max_pages))
        case "image":
            self = .image(maxPNGBytes: try fields.decode(UInt32.self, forKey: .max_png_bytes),
                          maxWidth: try fields.decode(UInt32.self, forKey: .max_width),
                          maxHeight: try fields.decode(UInt32.self, forKey: .max_height))
        default:
            throw DecodingError.dataCorruptedError(forKey: .as, in: fields,
                debugDescription: "Unknown file read")
        }
    }
}

public struct AssistantFilesReadFields: Decodable, Sendable {
    public let folder: AssistantFolderBookmark
    public let path: String
    public let read: AssistantReadAs
}

/// Why a folder's bookmark does not open it (`FolderCause` in orbit-core).
public enum AssistantFolderCause: Error, Equatable, Sendable {
    case bookmarkUnresolvable(domain: String, code: Int)
    case notAFolder
    case accessDenied(errno: Int32)
    case pathTooLong(bytes: Int, maxBytes: Int)

    public var fields: [String: Any] {
        switch self {
        case .bookmarkUnresolvable(let domain, let code):
            return ["kind": "bookmark_unresolvable", "domain": domain, "code": code]
        case .notAFolder: return ["kind": "not_a_folder"]
        case .accessDenied(let errno): return ["kind": "access_denied", "errno": Int(errno)]
        case .pathTooLong(let bytes, let maxBytes):
            return ["kind": "path_too_long", "bytes": bytes, "max_bytes": maxBytes]
        }
    }
}

/// Why a path inside a folder cannot be read (`FileCause` in orbit-core).
public enum AssistantFileCause: Equatable, Sendable {
    case notFound, notAFile, notAFolder, notText, pdfUnreadable, imageUnreadable, changed
    case imageDoesNotFit(smallestPNGBytes: Int, maxPNGBytes: Int)
    case readFailed(errno: Int32)
    case pastEnd(afterIndex: UInt64, sizeBytes: UInt64)
    case notCharacterBoundary(afterIndex: UInt64)
    case pastLastPage(afterPage: UInt32, pageCount: Int)

    public var fields: [String: Any] {
        switch self {
        case .notFound: return ["kind": "not_found"]
        case .notAFile: return ["kind": "not_a_file"]
        case .notAFolder: return ["kind": "not_a_folder"]
        case .notText: return ["kind": "not_text"]
        case .pdfUnreadable: return ["kind": "pdf_unreadable"]
        case .imageUnreadable: return ["kind": "image_unreadable"]
        case .changed: return ["kind": "changed"]
        case .imageDoesNotFit(let smallest, let maximum):
            return ["kind": "image_does_not_fit", "smallest_png_bytes": smallest, "max_png_bytes": maximum]
        case .readFailed(let errno): return ["kind": "read_failed", "errno": Int(errno)]
        case .pastEnd(let afterIndex, let size):
            return ["kind": "past_end", "after_index": afterIndex, "size_bytes": size]
        case .notCharacterBoundary(let afterIndex):
            return ["kind": "not_character_boundary", "after_index": afterIndex]
        case .pastLastPage(let afterPage, let count):
            return ["kind": "past_last_page", "after_page": Int(afterPage), "page_count": count]
        }
    }
}

/// A refusal of a folder order, by code and with its cause.
public enum AssistantFilesFailure: Error, Equatable {
    case file(AssistantFileCause)
    case folder(id: String, AssistantFolderCause)
    /// The panel is already open; it is not opened twice.
    case panelBusy

    public var code: String {
        switch self {
        case .file: "assistant_file_unavailable"
        case .folder: "assistant_folder_unavailable"
        case .panelBusy: "assistant_folder_panel_busy"
        }
    }

    public var detail: [String: Any] {
        switch self {
        case .file(let cause): ["cause": cause.fields]
        case .folder(let id, let cause): ["id": id, "cause": cause.fields]
        case .panelBusy: [:]
        }
    }
}

public enum AssistantText {
    /// How many of `bytes` to keep, at most `limit`, so the kept bytes end
    /// on a whole UTF-8 character: a cut never splits one.
    public static func boundaryPrefix(_ bytes: [UInt8], limit: Int) -> Int {
        var end = min(limit, bytes.count)
        guard end < bytes.count else { return end }
        // Step back over continuation bytes (10xxxxxx) to the start of the
        // character the limit falls in; that character is left for next time.
        while end > 0 && bytes[end] & 0xC0 == 0x80 {
            end -= 1
        }
        return end
    }

    /// Whether `bytes` is text Orbit reads: UTF-8 without NUL.
    public static func decode(_ bytes: [UInt8]) -> String? {
        guard !bytes.contains(0) else { return nil }
        return String(bytes: bytes, encoding: .utf8)
    }

    /// Whether a name matches a search, ignoring case and accents.
    public static func matches(_ name: String, query: String) -> Bool {
        name.range(of: query, options: [.caseInsensitive, .diacriticInsensitive]) != nil
    }

    /// The first line of `text` that matches, from 1, with at most `maxChars`
    /// characters around the match.
    public static func firstMatch(in text: String, query: String, maxChars: Int)
        -> (line: Int, snippet: String)? {
        var number = 0
        for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
            number += 1
            guard let range = line.range(of: query, options: [.caseInsensitive, .diacriticInsensitive])
            else { continue }
            let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
            guard trimmed.count > maxChars else { return (number, trimmed) }
            let before = max(0, (maxChars - line[range].count) / 2)
            let start = line.index(range.lowerBound, offsetBy: -before, limitedBy: line.startIndex)
                ?? line.startIndex
            let end = line.index(start, offsetBy: maxChars, limitedBy: line.endIndex) ?? line.endIndex
            return (number, String(line[start..<end]))
        }
        return nil
    }
}

/// A system error number (`errno`) as an `Error`, so `realpath`'s failure can
/// travel in a `Result`.
public struct SystemErrorNumber: Error, Equatable {
    public let value: Int32
    public init(_ value: Int32) { self.value = value }
}

/// The rule for a read: the path the daemon admitted is canonical, and it is
/// read only while it still is (no link swapped into it on the way).
public enum AssistantPathCheck {
    /// `resolve` is `realpath`; its failure is the system's error number.
    public static func stillCanonical(_ path: String,
                                      resolve: (String) -> Result<String, SystemErrorNumber>) throws {
        switch resolve(path) {
        case .success(let canonical) where canonical == path:
            return
        case .success:
            throw AssistantFilesFailure.file(.changed)
        case .failure(let error) where error.value == ENOENT || error.value == ENOTDIR:
            throw AssistantFilesFailure.file(.notFound)
        case .failure(let error):
            throw AssistantFilesFailure.file(.readFailed(errno: error.value))
        }
    }
}
