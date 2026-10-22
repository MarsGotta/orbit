import AppKit
import Darwin
import Foundation
import ImageIO
import PDFKit
import UniformTypeIdentifiers
import OrbitNativeProtocol

/// The assistant's folders on this Mac: the macOS panel that chooses them,
/// the bookmarks that find them again, and the read-only access to what they
/// hold. The panel choice is the user's decision; nothing here asks again.
///
/// The daemon decides which path a request may reach, on the canonical paths
/// this helper reports (`resolve`); a read happens only on a path that is
/// still canonical. Nothing is written, and no path or content is logged.
package final class AssistantFiles: @unchecked Sendable {
    /// File reads and searches are disk work and never run on AppKit's thread.
    private let queue = DispatchQueue(label: "orbit.native.assistant-files", qos: .userInitiated)
    /// Only touched on the main thread, by `pick`.
    private var panelOpen = false

    package init() {}

    // MARK: The panel

    /// «Añadir carpeta»: folders only, one at a time. Cancelling is an answer.
    @MainActor package func pick() async throws -> [String: Any] {
        guard !panelOpen else { throw AssistantFilesFailure.panelBusy }
        panelOpen = true
        defer { panelOpen = false }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        panel.resolvesAliases = true
        panel.prompt = "Añadir"
        panel.message = "Elige una carpeta que el asistente pueda leer. No podrá cambiar nada en ella."
        // Orbit runs as an accessory; without this the panel can open behind
        // the window the person is looking at.
        NSApplication.shared.activate()
        let response = await withCheckedContinuation { (continuation: CheckedContinuation<NSApplication.ModalResponse, Never>) in
            panel.begin { continuation.resume(returning: $0) }
        }
        guard response == .OK, let url = panel.url else { return ["state": "cancelled"] }
        let bookmark = try AssistantBookmarks.create(for: url)
        return ["state": "chosen", "path": url.standardizedFileURL.path,
                "bookmark_base64": bookmark.base64EncodedString()]
    }

    // MARK: Steps answered off the main thread

    package func resolve(_ fields: AssistantFoldersFields) async throws -> [String: Any] {
        try await run { try Self.resolveNow(fields) }
    }

    package func list(_ fields: AssistantFilesListFields) async throws -> [String: Any] {
        try await run { try Self.listNow(fields) }
    }

    package func search(_ fields: AssistantFilesSearchFields) async throws -> [String: Any] {
        try await run { try Self.searchNow(fields) }
    }

    package func read(_ fields: AssistantFilesReadFields) async throws -> [String: Any] {
        try await run { try Self.readNow(fields) }
    }

    private func run(_ work: @escaping @Sendable () throws -> [String: Any]) async throws -> [String: Any] {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                do { continuation.resume(returning: try work()) }
                catch { continuation.resume(throwing: error) }
            }
        }
    }

    // MARK: Resolution

    static func resolveNow(_ fields: AssistantFoldersFields) throws -> [String: Any] {
        var reports = [[String: Any]]()
        var opened = [AssistantOpenedFolder]()
        defer { opened.forEach { $0.close() } }
        for folder in fields.folders {
            switch AssistantOpenedFolder.open(folder) {
            case .success(let open):
                opened.append(open)
                var report: [String: Any] = ["state": "accessible", "id": folder.id, "path": open.canonical]
                if let renewed = open.renewedBookmark {
                    report["renewed_bookmark_base64"] = renewed.base64EncodedString()
                }
                reports.append(report)
            case .failure(let cause):
                reports.append(["state": "unavailable", "id": folder.id, "cause": cause.fields])
            }
        }
        var result: [String: Any] = ["folders": reports]
        if let path = fields.path {
            result["requested"] = requested(path)
        }
        return result
    }

    /// Where `path` leads, links and `..` followed, and what is there. It is
    /// resolved with the folders' access open, so a path inside a protected
    /// folder resolves as the person's own access does.
    static func requested(_ path: String) -> [String: Any] {
        switch AssistantPaths.canonical(path) {
        case .failure(let error) where error.value == ENOENT || error.value == ENOTDIR:
            return ["state": "not_found"]
        case .failure(let error):
            return ["state": "unavailable", "cause": AssistantFileCause.readFailed(errno: error.value).fields]
        case .success(let canonical):
            var status = stat()
            guard stat(canonical, &status) == 0 else {
                return ["state": "unavailable", "cause": AssistantFileCause.readFailed(errno: errno).fields]
            }
            var found: [String: Any] = ["state": "found", "path": canonical,
                                        "kind": AssistantPaths.kind(status.st_mode),
                                        "size_bytes": UInt64(max(0, status.st_size))]
            found["modified_ms"] = AssistantPaths.milliseconds(status.st_mtimespec)
            return found
        }
    }

    // MARK: Listing

    static func listNow(_ fields: AssistantFilesListFields) throws -> [String: Any] {
        let folder = try AssistantOpenedFolder.require(fields.folder)
        defer { folder.close() }
        try AssistantPathCheck.stillCanonical(fields.path, resolve: AssistantPaths.canonical)
        let names: [String]
        do {
            names = try FileManager.default.contentsOfDirectory(atPath: fields.path).sorted()
        } catch {
            throw AssistantFilesFailure.file(.notAFolder)
        }
        let start = Int(min(fields.after_index, UInt64(names.count)))
        let end = min(names.count, start + Int(fields.max_entries))
        let entries = names[start..<end].map { entry(name: $0, in: fields.path) }
        var result: [String: Any] = ["entries": entries, "total": names.count,
                                     "after_index": fields.after_index]
        if end < names.count { result["next_after_index"] = end }
        return result
    }

    /// One entry as `lstat` sees it: a link is a link, never followed here.
    static func entry(name: String, in directory: String) -> [String: Any] {
        var entry: [String: Any] = ["name": name]
        var status = stat()
        guard lstat((directory as NSString).appendingPathComponent(name), &status) == 0 else {
            entry["kind"] = "other"
            return entry
        }
        let kind = AssistantPaths.kind(status.st_mode, link: true)
        entry["kind"] = kind
        if kind == "file" { entry["size_bytes"] = UInt64(max(0, status.st_size)) }
        entry["modified_ms"] = AssistantPaths.milliseconds(status.st_mtimespec)
        return entry
    }

    // MARK: Search

    static func searchNow(_ fields: AssistantFilesSearchFields) throws -> [String: Any] {
        let limits = fields.limits
        let deadline = Date().addingTimeInterval(TimeInterval(limits.deadline_ms) / 1_000)
        var hits = [[String: Any]]()
        var visited: UInt64 = 0, filesRead: UInt64 = 0, bytesRead: UInt64 = 0, partial: UInt64 = 0
        var stopped: [String: Any]?
        var opened = [AssistantOpenedFolder]()
        defer { opened.forEach { $0.close() } }
        walk: for root in fields.roots {
            opened.append(try AssistantOpenedFolder.require(root.folder))
            try AssistantPathCheck.stillCanonical(root.path, resolve: AssistantPaths.canonical)
            var pending = [root.path]
            while let directory = pending.popLast() {
                guard let names = try? FileManager.default.contentsOfDirectory(atPath: directory) else {
                    continue
                }
                for name in names.sorted() {
                    if Date() >= deadline {
                        stopped = ["cause": "deadline", "limit": limits.deadline_ms]
                        break walk
                    }
                    if visited >= UInt64(limits.max_visited) {
                        stopped = ["cause": "visited", "limit": limits.max_visited]
                        break walk
                    }
                    visited += 1
                    let path = (directory as NSString).appendingPathComponent(name)
                    var status = stat()
                    guard lstat(path, &status) == 0 else { continue }
                    let kind = AssistantPaths.kind(status.st_mode, link: true)
                    let size = UInt64(max(0, status.st_size))
                    // A link is never followed: a search cannot leave the folder.
                    if kind == "folder" { pending.append(path) }
                    var hit: [String: Any]?
                    if AssistantText.matches(name, query: fields.query) {
                        hit = ["path": path, "kind": kind]
                        if kind == "file" { hit?["size_bytes"] = size }
                    } else if fields.content, kind == "file" {
                        if filesRead >= UInt64(limits.max_files_read) {
                            stopped = ["cause": "files_read", "limit": limits.max_files_read]
                            break walk
                        }
                        if bytesRead >= limits.max_bytes_read {
                            stopped = ["cause": "bytes_read", "limit": limits.max_bytes_read]
                            break walk
                        }
                        let wanted = min(size, limits.max_file_bytes, limits.max_bytes_read - bytesRead)
                        guard let bytes = AssistantPaths.head(path, bytes: Int(wanted)) else { continue }
                        filesRead += 1
                        bytesRead += UInt64(bytes.count)
                        if size > UInt64(bytes.count) { partial += 1 }
                        // A file read only in part may end inside a character.
                        let kept = size > UInt64(bytes.count)
                            ? AssistantText.boundaryPrefix(bytes, limit: max(0, bytes.count - 1))
                            : bytes.count
                        if let text = AssistantText.decode(Array(bytes[0..<kept])),
                           let match = AssistantText.firstMatch(in: text, query: fields.query,
                                                                maxChars: Int(limits.max_snippet_chars)) {
                            hit = ["path": path, "kind": kind, "size_bytes": size,
                                   "line": match.line, "snippet": match.snippet]
                        }
                    }
                    if let hit {
                        hits.append(hit)
                        if hits.count >= Int(limits.max_results) {
                            stopped = ["cause": "results", "limit": limits.max_results]
                            break walk
                        }
                    }
                }
            }
        }
        var result: [String: Any] = ["results": hits, "visited": visited, "files_read": filesRead,
                                     "bytes_read": bytesRead, "partial_files": partial]
        if let stopped { result["stopped"] = stopped }
        return result
    }

    // MARK: Reading

    static func readNow(_ fields: AssistantFilesReadFields) throws -> [String: Any] {
        let folder = try AssistantOpenedFolder.require(fields.folder)
        defer { folder.close() }
        try AssistantPathCheck.stillCanonical(fields.path, resolve: AssistantPaths.canonical)
        switch fields.read {
        case .text(let afterIndex, let maxBytes):
            return try readText(fields.path, afterIndex: afterIndex, maxBytes: Int(maxBytes))
        case .pdf(let afterPage, let afterIndex, let maxBytes, let maxPages):
            return try readPDF(fields.path, afterPage: afterPage, afterIndex: afterIndex,
                               maxBytes: Int(maxBytes), maxPages: Int(maxPages))
        case .image(let maxPNGBytes, let maxWidth, let maxHeight):
            return try readImage(fields.path, maxPNGBytes: Int(maxPNGBytes),
                                 maxWidth: Int(maxWidth), maxHeight: Int(maxHeight))
        }
    }

    static func readText(_ path: String, afterIndex: UInt64, maxBytes: Int) throws -> [String: Any] {
        // O_NOFOLLOW: the canonical path holds no link, and one put there
        // since is not followed.
        let descriptor = open(path, O_RDONLY | O_NOFOLLOW)
        guard descriptor >= 0 else {
            throw AssistantFilesFailure.file(errno == ELOOP ? .changed : .readFailed(errno: errno))
        }
        let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        var status = stat()
        guard fstat(descriptor, &status) == 0 else { throw AssistantFilesFailure.file(.readFailed(errno: errno)) }
        guard status.st_mode & S_IFMT == S_IFREG else { throw AssistantFilesFailure.file(.notAFile) }
        let size = UInt64(max(0, status.st_size))
        guard afterIndex <= size else {
            throw AssistantFilesFailure.file(.pastEnd(afterIndex: afterIndex, sizeBytes: size))
        }
        do {
            try handle.seek(toOffset: afterIndex)
            // Three bytes more say whether the limit falls inside a character.
            let bytes = [UInt8](try handle.read(upToCount: maxBytes + 3) ?? Data())
            if afterIndex > 0, let first = bytes.first, first & 0xC0 == 0x80 {
                throw AssistantFilesFailure.file(.notCharacterBoundary(afterIndex: afterIndex))
            }
            let more = afterIndex + UInt64(bytes.count) < size || bytes.count > maxBytes
            let kept = more ? AssistantText.boundaryPrefix(bytes, limit: maxBytes) : bytes.count
            guard let text = AssistantText.decode(Array(bytes[0..<kept])) else {
                throw AssistantFilesFailure.file(.notText)
            }
            var result: [String: Any] = ["content": "text", "text": text, "after_index": afterIndex,
                                         "size_bytes": size]
            let next = afterIndex + UInt64(kept)
            if next < size { result["next_after_index"] = next }
            return result
        } catch let failure as AssistantFilesFailure {
            throw failure
        } catch {
            throw AssistantFilesFailure.file(.readFailed(errno: Int32((error as NSError).code)))
        }
    }

    static func readPDF(_ path: String, afterPage: UInt32, afterIndex: UInt64,
                        maxBytes: Int, maxPages: Int) throws -> [String: Any] {
        guard let document = PDFDocument(url: URL(fileURLWithPath: path)), !document.isLocked else {
            throw AssistantFilesFailure.file(.pdfUnreadable)
        }
        let count = document.pageCount
        guard Int(afterPage) < count || (count == 0 && afterPage == 0) else {
            throw AssistantFilesFailure.file(.pastLastPage(afterPage: afterPage, pageCount: count))
        }
        var pages = [[String: Any]]()
        var used = 0
        var index = Int(afterPage)
        var next: (page: Int, index: UInt64)?
        while index < count {
            if pages.count >= maxPages { next = (index, 0); break }
            let all = [UInt8]((document.page(at: index)?.string ?? "").utf8)
            let offset = index == Int(afterPage) ? Int(min(afterIndex, UInt64(all.count))) : 0
            if offset > 0, offset < all.count, all[offset] & 0xC0 == 0x80 {
                throw AssistantFilesFailure.file(.notCharacterBoundary(afterIndex: afterIndex))
            }
            let rest = Array(all[offset...])
            let room = maxBytes - used
            if rest.count > room {
                let kept = AssistantText.boundaryPrefix(rest, limit: room)
                if kept > 0 {
                    pages.append(["page": index + 1, "text": String(decoding: rest[0..<kept], as: UTF8.self)])
                }
                next = (index, UInt64(offset + kept))
                break
            }
            pages.append(["page": index + 1, "text": String(decoding: rest, as: UTF8.self)])
            used += rest.count
            index += 1
        }
        var result: [String: Any] = ["content": "pdf", "pages": pages, "page_count": count,
                                     "after_page": afterPage, "after_index": afterIndex]
        if let next {
            result["next_after_page"] = next.page
            result["next_after_index"] = next.index
        }
        return result
    }

    static func readImage(_ path: String, maxPNGBytes: Int, maxWidth: Int, maxHeight: Int) throws -> [String: Any] {
        guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let sourceWidth = properties[kCGImagePropertyPixelWidth] as? Int,
              let sourceHeight = properties[kCGImagePropertyPixelHeight] as? Int,
              sourceWidth > 0, sourceHeight > 0 else {
            throw AssistantFilesFailure.file(.imageUnreadable)
        }
        // The longest edge that fits the box whichever way the image turns.
        var edge = min(max(sourceWidth, sourceHeight), min(maxWidth, maxHeight))
        var smallest = Int.max
        while edge >= AssistantImage.smallestEdge {
            if let png = AssistantImage.png(source, edge: edge, maxWidth: maxWidth, maxHeight: maxHeight) {
                if png.data.count <= maxPNGBytes {
                    return ["content": "image", "png_base64": png.data.base64EncodedString(),
                            "width": png.width, "height": png.height,
                            "source_width": sourceWidth, "source_height": sourceHeight]
                }
                smallest = min(smallest, png.data.count)
            } else {
                throw AssistantFilesFailure.file(.imageUnreadable)
            }
            edge = edge * 3 / 4
        }
        throw AssistantFilesFailure.file(.imageDoesNotFit(smallestPNGBytes: smallest == Int.max ? 0 : smallest,
                                                          maxPNGBytes: maxPNGBytes))
    }
}

/// Fitting an image into one capture.
enum AssistantImage {
    /// Below this edge an image says nothing a person could read; one that
    /// still does not fit is refused with its smallest size.
    static let smallestEdge = 64

    /// The image at `edge` pixels on its longest side, turned as its EXIF
    /// says, redrawn in 8-bit sRGB (what the daemon's PNG check accepts) and
    /// encoded as PNG.
    static func png(_ source: CGImageSource, edge: Int, maxWidth: Int, maxHeight: Int)
        -> (data: Data, width: Int, height: Int)? {
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: edge,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        guard let thumbnail = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else {
            return nil
        }
        let width = min(thumbnail.width, maxWidth), height = min(thumbnail.height, maxHeight)
        guard width > 0, height > 0,
              let space = CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
                                      bytesPerRow: 0, space: space,
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
            return nil
        }
        context.interpolationQuality = .high
        context.draw(thumbnail, in: CGRect(x: 0, y: 0, width: width, height: height))
        guard let drawn = context.makeImage() else { return nil }
        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(data as CFMutableData, UTType.png.identifier as CFString, 1, nil)
        else { return nil }
        CGImageDestinationAddImage(destination, drawn, nil)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return (data as Data, width, height)
    }
}

/// A folder opened from its bookmark for the length of one order.
struct AssistantOpenedFolder {
    let url: URL
    let canonical: String
    let renewedBookmark: Data?
    let scoped: Bool

    func close() {
        if scoped { url.stopAccessingSecurityScopedResource() }
    }

    /// The folder, or why its bookmark does not open it.
    static func open(_ folder: AssistantFolderBookmark) -> Result<AssistantOpenedFolder, AssistantFolderCause> {
        guard let data = Data(base64Encoded: folder.bookmark_base64) else {
            return .failure(.bookmarkUnresolvable(domain: "orbit.bookmark_base64", code: 0))
        }
        let resolved: (url: URL, stale: Bool)
        do {
            resolved = try AssistantBookmarks.resolve(data)
        } catch {
            let error = error as NSError
            return .failure(.bookmarkUnresolvable(domain: error.domain, code: error.code))
        }
        let scoped = resolved.url.startAccessingSecurityScopedResource()
        let close = { if scoped { resolved.url.stopAccessingSecurityScopedResource() } }
        switch AssistantPaths.canonical(resolved.url.path) {
        case .failure(let error) where error.value == EACCES || error.value == EPERM:
            close()
            return .failure(.accessDenied(errno: error.value))
        case .failure(let error):
            close()
            return .failure(.bookmarkUnresolvable(domain: NSPOSIXErrorDomain, code: Int(error.value)))
        case .success(let canonical):
            var status = stat()
            guard stat(canonical, &status) == 0 else {
                let error = errno
                close()
                return .failure(error == EACCES || error == EPERM
                                ? .accessDenied(errno: error)
                                : .bookmarkUnresolvable(domain: NSPOSIXErrorDomain, code: Int(error)))
            }
            guard status.st_mode & S_IFMT == S_IFDIR else {
                close()
                return .failure(.notAFolder)
            }
            guard canonical.utf8.count <= Int(PATH_MAX) else {
                close()
                return .failure(.pathTooLong(bytes: canonical.utf8.count, maxBytes: Int(PATH_MAX)))
            }
            let renewed: Data? = resolved.stale ? (try? AssistantBookmarks.create(for: resolved.url)) : nil
            return .success(AssistantOpenedFolder(url: resolved.url, canonical: canonical,
                                                  renewedBookmark: renewed, scoped: scoped))
        }
    }

    /// The folder of a second step, which the first step found open.
    static func require(_ folder: AssistantFolderBookmark) throws -> AssistantOpenedFolder {
        switch open(folder) {
        case .success(let opened): return opened
        case .failure(let cause): throw AssistantFilesFailure.folder(id: folder.id, cause)
        }
    }
}

enum AssistantBookmarks {
    /// A security-scoped, read-only bookmark; a plain one where macOS does not
    /// make scoped bookmarks for this app. Both find a moved folder again.
    static func create(for url: URL) throws -> Data {
        do {
            return try url.bookmarkData(options: [.withSecurityScope, .securityScopeAllowOnlyReadAccess],
                                        includingResourceValuesForKeys: nil, relativeTo: nil)
        } catch {
            return try url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil)
        }
    }

    static func resolve(_ data: Data) throws -> (url: URL, stale: Bool) {
        var stale = false
        do {
            let url = try URL(resolvingBookmarkData: data, options: [.withSecurityScope, .withoutUI, .withoutMounting],
                              relativeTo: nil, bookmarkDataIsStale: &stale)
            return (url, stale)
        } catch let scoped {
            do {
                let url = try URL(resolvingBookmarkData: data, options: [.withoutUI, .withoutMounting],
                                  relativeTo: nil, bookmarkDataIsStale: &stale)
                return (url, stale)
            } catch {
                throw scoped
            }
        }
    }
}

enum AssistantPaths {
    /// `realpath`: links and `..` followed, or the system's error number.
    static func canonical(_ path: String) -> Result<String, SystemErrorNumber> {
        guard let resolved = realpath(path, nil) else { return .failure(SystemErrorNumber(errno)) }
        defer { free(resolved) }
        return .success(String(cString: resolved))
    }

    static func kind(_ mode: mode_t, link: Bool = false) -> String {
        switch mode & S_IFMT {
        case S_IFREG: return "file"
        case S_IFDIR: return "folder"
        case S_IFLNK where link: return "link"
        default: return "other"
        }
    }

    static func milliseconds(_ time: timespec) -> Int64 {
        Int64(time.tv_sec) * 1_000 + Int64(time.tv_nsec) / 1_000_000
    }

    /// The first `bytes` bytes of a regular file, never through a link.
    static func head(_ path: String, bytes: Int) -> [UInt8]? {
        let descriptor = open(path, O_RDONLY | O_NOFOLLOW)
        guard descriptor >= 0 else { return nil }
        let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        guard bytes > 0 else { return [] }
        return (try? handle.read(upToCount: bytes)).map { [UInt8]($0 ?? Data()) }
    }
}
