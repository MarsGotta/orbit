import Foundation
import XCTest
@testable import OrbitNativeProtocol

/// The walk that turns a window's Accessibility elements into its text, on
/// an invented tree: reading order, a secure field never read, an element
/// that cannot be read left out, and each bound that stops it.
final class WindowTextTests: XCTestCase {
    private struct Unreadable: Error {}

    /// Elements by number; 0 is the window.
    private struct Tree: WindowTextAccess {
        var nodes: [Int: WindowTextNode<Int>]
        var unreadable: Set<Int> = []
        private(set) var read = [Int]()

        init(_ nodes: [Int: WindowTextNode<Int>], unreadable: Set<Int> = []) {
            self.nodes = nodes
            self.unreadable = unreadable
        }

        mutating func node(_ element: Int) throws -> WindowTextNode<Int> {
            read.append(element)
            if unreadable.contains(element) { throw Unreadable() }
            return nodes[element] ?? WindowTextNode(secure: false, texts: [], children: [])
        }
    }

    private static func node(_ texts: [String], _ children: [Int] = [], secure: Bool = false) -> WindowTextNode<Int> {
        WindowTextNode(secure: secure, texts: texts, children: children)
    }

    private static let roomy = WindowTextLimits(max_bytes: 131_072, max_nodes: 2_000, deadline_ms: 3_000)

    private static let notes: [Int: WindowTextNode<Int>] = [
        0: node(["Lista de la compra"], [1, 4, 5]),
        1: node([], [2, 3]),
        2: node(["Pan", "Pan", " "]),
        3: node(["Agua", "", "Botella de agua"]),
        4: node(["clave-sintetica"], [6], secure: true),
        5: node(["Enviar"]),
        6: node(["no se lee"]),
    ]

    func testTheTextIsReadInReadingOrderAndASecureFieldNever() {
        var tree = Tree(Self.notes)
        let read = WindowTextReader.read(using: &tree, window: 0, limits: Self.roomy) { 0 }
        XCTAssertEqual(read.text, "Lista de la compra\nPan\nAgua · Botella de agua\nEnviar")
        XCTAssertNil(read.stopped)
        // The secure field's children are never visited.
        XCTAssertFalse(tree.read.contains(6))
    }

    func testAnElementThatCannotBeReadIsLeftOutWithItsChildren() {
        var tree = Tree(Self.notes, unreadable: [1])
        let read = WindowTextReader.read(using: &tree, window: 0, limits: Self.roomy) { 0 }
        XCTAssertEqual(read.text, "Lista de la compra\nEnviar")
        XCTAssertFalse(tree.read.contains(2))
    }

    func testTheSameTextTwiceInARowIsOneLine() {
        var tree = Tree([0: Self.node([], [1, 2]), 1: Self.node(["Celda"]), 2: Self.node(["Celda"])])
        XCTAssertEqual(WindowTextReader.read(using: &tree, window: 0, limits: Self.roomy) { 0 }.text, "Celda")
    }

    func testTheByteCeilingCutsOnACharacterAndSaysSo() {
        var tree = Tree([0: Self.node(["año"], [1]), 1: Self.node(["ñañaña"])])
        // «año» is 4 bytes, the line break 1; 4 bytes are left: «ña».
        let limits = WindowTextLimits(max_bytes: 9, max_nodes: 2_000, deadline_ms: 3_000)
        let read = WindowTextReader.read(using: &tree, window: 0, limits: limits) { 0 }
        XCTAssertEqual(read.text, "año\nña")
        XCTAssertEqual(read.stopped, .bytes)
        XCTAssertLessThanOrEqual(read.text.utf8.count, 9)
    }

    func testTheNodeCeilingStopsTheWalk() {
        var tree = Tree(Self.notes)
        let limits = WindowTextLimits(max_bytes: 131_072, max_nodes: 2, deadline_ms: 3_000)
        let read = WindowTextReader.read(using: &tree, window: 0, limits: limits) { 0 }
        XCTAssertEqual(read.text, "Lista de la compra")
        XCTAssertEqual(read.stopped, .nodes)
        XCTAssertEqual(tree.read, [0, 1])
    }

    func testTheDeadlineStopsTheWalkWithWhatWasRead() {
        var tree = Tree(Self.notes)
        var now: UInt64 = 0
        let limits = WindowTextLimits(max_bytes: 131_072, max_nodes: 2_000, deadline_ms: 3_000)
        let read = WindowTextReader.read(using: &tree, window: 0, limits: limits) {
            defer { now += 1_000 }
            return now
        }
        XCTAssertEqual(read.text, "Lista de la compra\nPan")
        XCTAssertEqual(read.stopped, .deadline)
    }

    func testBothCommandFixturesDecode() throws {
        var fixtures = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { fixtures.deleteLastPathComponent() }
        fixtures.appendPathComponent("contracts/fixtures/native/commands")
        let text = try NativeWireCommand.decode(Data(contentsOf: fixtures.appendingPathComponent("window_text.json")))
        guard case .windowText(let wire) = text else { return XCTFail(text.command) }
        XCTAssertEqual(wire.fields.window_id, 42)
        XCTAssertEqual(wire.fields.limits, WindowTextLimits(max_bytes: 131_072, max_nodes: 2_000, deadline_ms: 3_000))
        let photo = try NativeWireCommand.decode(Data(contentsOf: fixtures.appendingPathComponent("photo_jpeg.json")))
        guard case .photoJPEG(let fields) = photo else { return XCTFail(photo.command) }
        XCTAssertTrue(fields.fields.destination_path.hasSuffix("IMG_0001.jpg"))
    }
}
