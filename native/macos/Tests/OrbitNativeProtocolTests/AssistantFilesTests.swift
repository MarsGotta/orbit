import AppKit
import CoreGraphics
import CoreText
import Foundation
import ImageIO
import UniformTypeIdentifiers
import XCTest
@testable import OrbitNativeHost
@testable import OrbitNativeProtocol

/// The helper's half of the assistant's folders, on a real temporary folder:
/// what it resolves (links and `..` followed), what it refuses to read once a
/// path is no longer canonical, PDF text through PDFKit, an image fitted as a
/// PNG, and a bookmark that no longer opens. No TCC folder, no panel and no
/// person's files are touched; the folder names are invented.
final class AssistantFilesTests: XCTestCase {
    private var root: URL!
    private var inside: URL!
    private var outside: URL!

    override func setUpWithError() throws {
        let base = FileManager.default.temporaryDirectory
            .appendingPathComponent("orbit-assistant-files-\(UUID().uuidString)")
        inside = base.appendingPathComponent("Proyectos")
        outside = base.appendingPathComponent("Privado")
        try FileManager.default.createDirectory(at: inside, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: outside, withIntermediateDirectories: true)
        root = base
    }

    override func tearDownWithError() throws {
        if let root { try? FileManager.default.removeItem(at: root) }
    }

    private func canonical(_ url: URL) throws -> String {
        guard case .success(let path) = AssistantPaths.canonical(url.path) else {
            throw XCTSkip("realpath failed on the fixture")
        }
        return path
    }

    private func bookmark(_ url: URL) throws -> AssistantFolderBookmark {
        AssistantFolderBookmark(id: "folder-1",
                                bookmark_base64: try AssistantBookmarks.create(for: url).base64EncodedString())
    }

    private func fields<T: Decodable>(_ type: T.Type, _ value: [String: Any]) throws -> T {
        try JSONDecoder().decode(type, from: JSONSerialization.data(withJSONObject: value))
    }

    // MARK: Pure rules

    func testATextCutNeverSplitsACharacter() {
        let bytes = [UInt8]("añb".utf8) // 61 C3 B1 62
        XCTAssertEqual(AssistantText.boundaryPrefix(bytes, limit: 2), 1)
        XCTAssertEqual(AssistantText.boundaryPrefix(bytes, limit: 3), 3)
        XCTAssertEqual(AssistantText.boundaryPrefix(bytes, limit: 10), 4)
        XCTAssertNil(AssistantText.decode([0x61, 0x00]))
        XCTAssertNil(AssistantText.decode([0xC3]))
        XCTAssertEqual(AssistantText.decode([UInt8]("ñ".utf8)), "ñ")
    }

    func testSearchMatchesIgnoreCaseAndAccents() {
        XCTAssertTrue(AssistantText.matches("Presupuesto-Revisión.md", query: "revision"))
        XCTAssertFalse(AssistantText.matches("plan.md", query: "presupuesto"))
        let match = AssistantText.firstMatch(in: "uno\ndos\nEl PRESUPUESTO del trimestre",
                                             query: "presupuesto", maxChars: 160)
        XCTAssertEqual(match?.line, 3)
        XCTAssertEqual(match?.snippet, "El PRESUPUESTO del trimestre")
        let long = String(repeating: "a", count: 500) + "clave" + String(repeating: "b", count: 500)
        XCTAssertEqual(AssistantText.firstMatch(in: long, query: "clave", maxChars: 40)?.snippet.count, 40)
    }

    func testAPathThatIsNoLongerCanonicalIsNotRead() {
        XCTAssertNoThrow(try AssistantPathCheck.stillCanonical("/a/b") { .success($0) })
        XCTAssertThrowsError(try AssistantPathCheck.stillCanonical("/a/b") { _ in .success("/c/b") }) {
            XCTAssertEqual($0 as? AssistantFilesFailure, .file(.changed))
        }
        XCTAssertThrowsError(try AssistantPathCheck.stillCanonical("/a/b") { _ in .failure(ENOENT) }) {
            XCTAssertEqual($0 as? AssistantFilesFailure, .file(.notFound))
        }
    }

    func testEveryNewCommandFixtureDecodes() throws {
        var fixtures = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { fixtures.deleteLastPathComponent() }
        fixtures.appendPathComponent("contracts/fixtures/native/commands")
        for name in ["assistant_folder_pick", "assistant_folders", "assistant_files_list",
                     "assistant_files_search", "assistant_files_read"] {
            let frame = try NativeWireCommand.decode(
                Data(contentsOf: fixtures.appendingPathComponent("\(name).json")))
            XCTAssertEqual(frame.command, name)
        }
    }

    // MARK: Resolution on a real folder

    func testTheFolderAndAPathInsideResolveToCanonicalPaths() throws {
        let plan = inside.appendingPathComponent("plan.md")
        try Data("# Plan\n".utf8).write(to: plan)
        let result = try AssistantFiles.resolveNow(
            fields(AssistantFoldersFields.self, ["folders": [["id": "folder-1",
                "bookmark_base64": try bookmark(inside).bookmark_base64]], "path": plan.path]))
        let folders = try XCTUnwrap(result["folders"] as? [[String: Any]])
        XCTAssertEqual(folders.first?["state"] as? String, "accessible")
        XCTAssertEqual(folders.first?["path"] as? String, try canonical(inside))
        let requested = try XCTUnwrap(result["requested"] as? [String: Any])
        XCTAssertEqual(requested["state"] as? String, "found")
        XCTAssertEqual(requested["kind"] as? String, "file")
        XCTAssertEqual(requested["size_bytes"] as? UInt64, 7)
    }

    func testALinkAndDotDotResolveToWhereTheyLeadOutsideTheFolder() throws {
        let secret = outside.appendingPathComponent("claves.txt")
        try Data("secreto".utf8).write(to: secret)
        let link = inside.appendingPathComponent("atajo.txt")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: secret)
        let target = try canonical(secret)
        for asked in [link.path, inside.path + "/../Privado/claves.txt"] {
            let requested = AssistantFiles.requested(asked)
            XCTAssertEqual(requested["state"] as? String, "found", asked)
            // The canonical path leads outside: the daemon refuses it by that.
            XCTAssertEqual(requested["path"] as? String, target, asked)
            XCTAssertFalse((requested["path"] as? String ?? "").hasPrefix(try canonical(inside) + "/"))
        }
        // Reading the link's own path is refused: it is not canonical.
        let read = try fields(AssistantFilesReadFields.self, [
            "folder": ["id": "folder-1", "bookmark_base64": try bookmark(inside).bookmark_base64],
            "path": link.path, "read": ["as": "text", "after_index": 0, "max_bytes": 48_000]])
        XCTAssertThrowsError(try AssistantFiles.readNow(read)) {
            XCTAssertEqual($0 as? AssistantFilesFailure, .file(.changed))
        }
    }

    func testAListingNamesALinkWithoutFollowingIt() throws {
        try FileManager.default.createSymbolicLink(at: inside.appendingPathComponent("atajo"),
                                                   withDestinationURL: outside)
        try Data("x".utf8).write(to: inside.appendingPathComponent("b.txt"))
        let list = try AssistantFiles.listNow(fields(AssistantFilesListFields.self, [
            "folder": ["id": "folder-1", "bookmark_base64": try bookmark(inside).bookmark_base64],
            "path": try canonical(inside), "after_index": 0, "max_entries": 1]))
        let entries = try XCTUnwrap(list["entries"] as? [[String: Any]])
        XCTAssertEqual(entries.map { $0["name"] as? String }, ["atajo"])
        XCTAssertEqual(entries.first?["kind"] as? String, "link")
        XCTAssertEqual(list["next_after_index"] as? Int, 1)
        XCTAssertEqual(list["total"] as? Int, 2)
    }

    func testASearchNeverWalksThroughALink() throws {
        try Data("presupuesto secreto".utf8).write(to: outside.appendingPathComponent("fuera.txt"))
        try FileManager.default.createSymbolicLink(at: inside.appendingPathComponent("atajo"),
                                                   withDestinationURL: outside)
        try Data("linea\nEl presupuesto del trimestre".utf8).write(to: inside.appendingPathComponent("plan.md"))
        let search = try AssistantFiles.searchNow(fields(AssistantFilesSearchFields.self, [
            "roots": [["folder": ["id": "folder-1", "bookmark_base64": try bookmark(inside).bookmark_base64],
                       "path": try canonical(inside)]],
            "query": "presupuesto", "content": true,
            "limits": ["max_results": 50, "max_visited": 20_000, "max_files_read": 1_000,
                       "max_bytes_read": 33_554_432, "max_file_bytes": 1_048_576,
                       "max_snippet_chars": 160, "deadline_ms": 10_000]]))
        let hits = try XCTUnwrap(search["results"] as? [[String: Any]])
        XCTAssertEqual(hits.count, 1)
        XCTAssertEqual(hits.first?["line"] as? Int, 2)
        XCTAssertTrue((hits.first?["path"] as? String ?? "").hasSuffix("/Proyectos/plan.md"))
    }

    // MARK: Reading

    func testATextFileIsReadAPageAtATimeAtCharacterBoundaries() throws {
        let notes = inside.appendingPathComponent("notas.md")
        try Data(String(repeating: "ñ", count: 10).utf8).write(to: notes)
        let path = try canonical(notes)
        let first = try AssistantFiles.readText(path, afterIndex: 0, maxBytes: 5)
        XCTAssertEqual(first["text"] as? String, "ññ")
        XCTAssertEqual(first["next_after_index"] as? UInt64, 4)
        XCTAssertThrowsError(try AssistantFiles.readText(path, afterIndex: 1, maxBytes: 5)) {
            XCTAssertEqual($0 as? AssistantFilesFailure, .file(.notCharacterBoundary(afterIndex: 1)))
        }
        let last = try AssistantFiles.readText(path, afterIndex: 16, maxBytes: 5)
        XCTAssertEqual(last["text"] as? String, "ññ")
        XCTAssertNil(last["next_after_index"])
        let binary = inside.appendingPathComponent("datos.bin")
        try Data([0x00, 0x01, 0x02]).write(to: binary)
        XCTAssertThrowsError(try AssistantFiles.readText(try canonical(binary), afterIndex: 0, maxBytes: 5)) {
            XCTAssertEqual($0 as? AssistantFilesFailure, .file(.notText))
        }
    }

    func testAPDFIsReadAsTheTextOfItsPages() throws {
        let manual = inside.appendingPathComponent("manual.pdf")
        try writePDF(to: manual, pages: ["Capítulo uno", "Capítulo dos", "Capítulo tres"])
        let path = try canonical(manual)
        let read = try AssistantFiles.readPDF(path, afterPage: 1, afterIndex: 0, maxBytes: 48_000, maxPages: 1)
        XCTAssertEqual(read["page_count"] as? Int, 3)
        let pages = try XCTUnwrap(read["pages"] as? [[String: Any]])
        XCTAssertEqual(pages.first?["page"] as? Int, 2)
        XCTAssertTrue((pages.first?["text"] as? String ?? "").contains("dos"))
        XCTAssertEqual(read["next_after_page"] as? Int, 2)
        XCTAssertThrowsError(try AssistantFiles.readPDF(path, afterPage: 3, afterIndex: 0,
                                                        maxBytes: 48_000, maxPages: 50)) {
            XCTAssertEqual($0 as? AssistantFilesFailure, .file(.pastLastPage(afterPage: 3, pageCount: 3)))
        }
        let notPDF = inside.appendingPathComponent("falso.pdf")
        try Data("no soy un pdf".utf8).write(to: notPDF)
        XCTAssertThrowsError(try AssistantFiles.readPDF(try canonical(notPDF), afterPage: 0, afterIndex: 0,
                                                        maxBytes: 48_000, maxPages: 50)) {
            XCTAssertEqual($0 as? AssistantFilesFailure, .file(.pdfUnreadable))
        }
    }

    func testAnImageIsFittedAsAPNGWithinTheCaptureBounds() throws {
        let photo = inside.appendingPathComponent("foto.jpg")
        try writeJPEG(to: photo, width: 4_000, height: 3_000)
        let read = try AssistantFiles.readImage(try canonical(photo), maxPNGBytes: 700_000,
                                                maxWidth: 1_280, maxHeight: 1_024)
        XCTAssertEqual(read["source_width"] as? Int, 4_000)
        let width = try XCTUnwrap(read["width"] as? Int), height = try XCTUnwrap(read["height"] as? Int)
        XCTAssertLessThanOrEqual(width, 1_280)
        XCTAssertLessThanOrEqual(height, 1_024)
        let png = try XCTUnwrap(Data(base64Encoded: try XCTUnwrap(read["png_base64"] as? String)))
        XCTAssertLessThanOrEqual(png.count, 700_000)
        XCTAssertEqual(Array(png.prefix(8)), [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        // A ceiling nothing can meet is refused with the smallest size tried.
        XCTAssertThrowsError(try AssistantFiles.readImage(try canonical(photo), maxPNGBytes: 10,
                                                          maxWidth: 1_280, maxHeight: 1_024)) {
            guard case .file(.imageDoesNotFit(let smallest, let maximum))? = $0 as? AssistantFilesFailure else {
                return XCTFail("\($0)")
            }
            XCTAssertGreaterThan(smallest, 10)
            XCTAssertEqual(maximum, 10)
        }
    }

    // MARK: Bookmarks

    func testABookmarkWhoseFolderIsGoneSaysWhyAndARefreshedOneOpens() throws {
        let gone = root.appendingPathComponent("Temporal")
        try FileManager.default.createDirectory(at: gone, withIntermediateDirectories: true)
        let stale = try bookmark(gone)
        try FileManager.default.removeItem(at: gone)
        guard case .failure(let cause) = AssistantOpenedFolder.open(stale) else {
            return XCTFail("a deleted folder's bookmark opened")
        }
        guard case .bookmarkUnresolvable(let domain, _) = cause else {
            return XCTFail("\(cause)")
        }
        XCTAssertFalse(domain.isEmpty)
        let result = try AssistantFiles.resolveNow(fields(AssistantFoldersFields.self,
            ["folders": [["id": "folder-1", "bookmark_base64": stale.bookmark_base64]]]))
        let report = try XCTUnwrap((result["folders"] as? [[String: Any]])?.first)
        XCTAssertEqual(report["state"] as? String, "unavailable")
        XCTAssertEqual((report["cause"] as? [String: Any])?["kind"] as? String, "bookmark_unresolvable")
        // A file where a folder was is not a folder.
        let file = root.appendingPathComponent("Ahora-archivo")
        try FileManager.default.createDirectory(at: file, withIntermediateDirectories: true)
        let marked = try bookmark(file)
        try FileManager.default.removeItem(at: file)
        try Data("x".utf8).write(to: file)
        if case .success = AssistantOpenedFolder.open(marked) {
            XCTFail("a bookmark that now leads to a file opened as a folder")
        }
        XCTAssertEqual(AssistantOpenedFolder.open(AssistantFolderBookmark(id: "x", bookmark_base64: "%%")).failure,
                       .bookmarkUnresolvable(domain: "orbit.bookmark_base64", code: 0))
    }

    func testAFolderThatMovedIsFoundAgainByItsBookmark() throws {
        let before = root.appendingPathComponent("Antes")
        try FileManager.default.createDirectory(at: before, withIntermediateDirectories: true)
        let marked = try bookmark(before)
        let after = root.appendingPathComponent("Despues")
        try FileManager.default.moveItem(at: before, to: after)
        guard case .success(let opened) = AssistantOpenedFolder.open(marked) else {
            return XCTFail("a moved folder was not found by its bookmark")
        }
        defer { opened.close() }
        XCTAssertEqual(opened.canonical, try canonical(after))
    }

    // MARK: Fixtures

    private func writePDF(to url: URL, pages: [String]) throws {
        var box = CGRect(x: 0, y: 0, width: 300, height: 200)
        guard let context = CGContext(url as CFURL, mediaBox: &box, nil) else {
            throw XCTSkip("no PDF context")
        }
        for text in pages {
            context.beginPDFPage(nil)
            let line = CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: [
                .font: NSFont.systemFont(ofSize: 14)]))
            context.textPosition = CGPoint(x: 20, y: 100)
            CTLineDraw(line, context)
            context.endPDFPage()
        }
        context.closePDF()
    }

    private func writeJPEG(to url: URL, width: Int, height: Int) throws {
        guard let space = CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
                                      bytesPerRow: 0, space: space,
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
            throw XCTSkip("no bitmap context")
        }
        // Noise, so the PNG has to shrink to fit 700 000 bytes.
        for index in 0..<2_000 {
            context.setFillColor(red: CGFloat(index % 7) / 7, green: CGFloat(index % 11) / 11,
                                 blue: CGFloat(index % 13) / 13, alpha: 1)
            context.fill(CGRect(x: (index * 37) % width, y: (index * 53) % height, width: 90, height: 60))
        }
        guard let image = context.makeImage(),
              let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.jpeg.identifier as CFString, 1, nil)
        else { throw XCTSkip("no JPEG destination") }
        CGImageDestinationAddImage(destination, image, nil)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
    }
}

private extension Result {
    var failure: Failure? {
        if case .failure(let failure) = self { return failure }
        return nil
    }
}
