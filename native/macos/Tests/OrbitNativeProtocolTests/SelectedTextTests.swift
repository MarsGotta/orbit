import XCTest
@testable import OrbitNativeProtocol

final class SelectedTextTests: XCTestCase {
    private static let target = TextWindowIdentity(pid: 42, title: "Synthetic document", x: -100, y: 20, width: 640, height: 480)

    private struct Fixture: SelectedTextAccess {
        typealias Element = Int
        var windowsRead = 0
        var focusReads = 0
        var textReads = 0
        var rangeReads = 0
        var classificationReads = 0
        var role: String? = "AXTextArea"
        var subrole: String? = "AXUnknown"
        var ambiguous = false
        var excessive = false
        var replaceWindow = false
        var replaceFocus = false
        var replaceRange = false
        var replaceText = false
        var secureOnReread = false
        var incorrectSubstring = false
        var multipleRanges = false
        var wrongOwner = false
        var wrongContainingWindow = false
        var units = Array("Sí\t🙂\r\n\u{1b}\u{202e}".utf16)

        func same(_ lhs: Int, _ rhs: Int) -> Bool { lhs == rhs }
        mutating func application(pid: Int32) throws -> Int { 1 }
        mutating func windows(_ application: Int) throws -> [Int] {
            windowsRead += 1
            if excessive { return Array(10..<43) }
            if ambiguous { return [10, 11] }
            return replaceWindow && windowsRead > 1 ? [20] : [10]
        }
        mutating func identity(_ window: Int) throws -> TextWindowIdentity { SelectedTextTests.target }
        mutating func pid(_ element: Int) throws -> Int32 { wrongOwner ? 99 : 42 }
        mutating func focused(_ application: Int) throws -> Int {
            focusReads += 1
            return replaceFocus && focusReads > 2 ? 99 : 2
        }
        mutating func window(_ element: Int) throws -> Int { wrongContainingWindow ? 99 : 10 }
        mutating func classification(_ element: Int) throws -> TextElementClassification {
            classificationReads += 1
            return TextElementClassification(role: role,
                subrole: secureOnReread && classificationReads > 1 ? "AXSecureTextField" : subrole)
        }
        mutating func range(_ element: Int) throws -> TextSelectionRange {
            rangeReads += 1
            return TextSelectionRange(location: replaceRange && rangeReads > 1 ? 11 : 10, length: Int64(units.count))
        }
        mutating func numberOfCharacters(_ element: Int) throws -> Int64 { 10000 }
        mutating func ranges(_ element: Int) throws -> [TextSelectionRange]? {
            multipleRanges ? [TextSelectionRange(location: 10, length: 1), TextSelectionRange(location: 15, length: 1)] : nil
        }
        mutating func selectedText(_ element: Int) throws -> [UInt16] {
            textReads += 1
            return replaceText && textReads > 1 ? Array("Changed".utf16) : units
        }
        mutating func string(_ element: Int, range: TextSelectionRange) throws -> [UInt16] {
            incorrectSubstring ? Array("Unrelated".utf16) : units
        }
    }

    func testReadsExactSelectionWithoutNormalizingOrDroppingControls() throws {
        var fixture = Fixture()
        let result = try SelectedTextReader.read(using: &fixture, target: Self.target, nativeWindows: [Self.target])
        XCTAssertEqual(Array(result.text.utf16), fixture.units)
        XCTAssertEqual(result.range.location, 10)
        XCTAssertEqual(result.range.length, Int64(fixture.units.count))
        XCTAssertEqual(fixture.textReads, 2)
        XCTAssertEqual(fixture.windowsRead, 2)
        XCTAssertEqual(fixture.classificationReads, 2)
    }

    func testRejectsSecureNontextAndIncompleteClassificationBeforeContent() {
        for variant in 0..<4 {
            var fixture = Fixture()
            switch variant {
            case 0: fixture.subrole = "AXSecureTextField"
            case 1: fixture.subrole = nil
            case 2: fixture.role = nil
            default: fixture.role = "AXButton"
            }
            XCTAssertThrowsError(try SelectedTextReader.read(using: &fixture, target: Self.target, nativeWindows: [Self.target]))
            XCTAssertEqual(fixture.textReads, 0)
            XCTAssertEqual(fixture.rangeReads, 0)
        }
    }

    func testIncompleteOrAmbiguousWindowSetsCannotReadContent() {
        for variant in 0..<4 {
            var fixture = Fixture()
            switch variant {
            case 0: fixture.ambiguous = true
            case 1: fixture.excessive = true
            case 2: fixture.wrongOwner = true
            default: fixture.wrongContainingWindow = true
            }
            XCTAssertThrowsError(try SelectedTextReader.read(using: &fixture, target: Self.target, nativeWindows: [Self.target]))
            XCTAssertEqual(fixture.textReads, 0)
        }
    }

    func testReplacedAXWindowOrFocusCannotDeliverAnEarlierSelection() {
        for variant in 0..<2 {
            var fixture = Fixture()
            fixture.replaceWindow = variant == 0
            fixture.replaceFocus = variant == 1
            XCTAssertThrowsError(try SelectedTextReader.read(using: &fixture, target: Self.target, nativeWindows: [Self.target])) { error in
                XCTAssertEqual(error as? SelectedTextFailure, .identityChanged)
            }
        }
    }

    func testRangeTextAndSubstringMustRemainExactlyTheSame() {
        for variant in 0..<3 {
            var fixture = Fixture()
            fixture.replaceRange = variant == 0
            fixture.replaceText = variant == 1
            fixture.incorrectSubstring = variant == 2
            XCTAssertThrowsError(try SelectedTextReader.read(using: &fixture, target: Self.target, nativeWindows: [Self.target])) { error in
                XCTAssertEqual(error as? SelectedTextFailure, .changed)
            }
        }
    }

    func testSecureClassificationOnRereadPreventsSecondContentRead() {
        var fixture = Fixture()
        fixture.secureOnReread = true
        XCTAssertThrowsError(try SelectedTextReader.read(using: &fixture, target: Self.target, nativeWindows: [Self.target])) { error in
            XCTAssertEqual(error as? SelectedTextFailure, .secure)
        }
        XCTAssertEqual(fixture.textReads, 1)
    }

    func testMultipleRangesAndEmptyOrOversizeSelectionsFailBeforeContent() {
        for variant in 0..<3 {
            var fixture = Fixture()
            if variant == 0 { fixture.multipleRanges = true }
            if variant == 1 { fixture.units = [] }
            if variant == 2 { fixture.units = Array(repeating: 0x61, count: 4097) }
            XCTAssertThrowsError(try SelectedTextReader.read(using: &fixture, target: Self.target, nativeWindows: [Self.target]))
            XCTAssertEqual(fixture.textReads, 0)
        }
    }

    func testEmptySelectionIsConfirmedByAValidZeroLengthRangeBeforeReadingContent() {
        var fixture = Fixture()
        fixture.units = []
        XCTAssertThrowsError(try SelectedTextReader.read(using: &fixture, target: Self.target, nativeWindows: [Self.target])) {
            XCTAssertEqual(($0 as? SelectedTextFailure)?.rawValue, "selected_text_empty")
        }
        XCTAssertEqual(fixture.textReads, 0)
    }

    func testRangeUsesUTF16UnitsAndRejectsOverflowNegativeOrOutOfDocumentBounds() throws {
        try TextSelectionRange(location: 0, length: 4096).validate(total: 4096)
        try TextSelectionRange(location: Int64.max - 1, length: 1).validate(total: Int64.max)
        for range in [TextSelectionRange(location: -1, length: 1),
                      TextSelectionRange(location: 0, length: 0),
                      TextSelectionRange(location: 0, length: 4097),
                      TextSelectionRange(location: Int64.max, length: 1),
                      TextSelectionRange(location: 100, length: 1)] {
            XCTAssertThrowsError(try range.validate(total: 100))
        }
    }

    func testStrictUTF16NeverRepairsOrNormalizesInvalidInput() throws {
        let invalidSelections: [[UInt16]] = [[], [0], [0xD800], [0xDC00], [0xD800, 0x61], [0xDC00, 0xD800]]
        for invalid in invalidSelections {
            XCTAssertThrowsError(try StrictText.decode(invalid))
        }
        let decomposed = Array("e\u{301}".utf16)
        let composed = Array("é".utf16)
        XCTAssertNotEqual(decomposed, composed)
        XCTAssertEqual(Array(try StrictText.decode(decomposed).utf16), decomposed)
        XCTAssertEqual(Array(try StrictText.decode([0xD83D, 0xDE42]).utf16), [0xD83D, 0xDE42])
        XCTAssertThrowsError(try StrictText.decode(Array("🙂".utf16), maximumBytes: 3))
        XCTAssertThrowsError(try StrictText.decode(Array(repeating: 0x61, count: 4097)))
    }

    func testWindowIdentityIsByteExactNotCanonicalStringEquivalence() {
        let first = TextWindowIdentity(pid: 42, title: "é", x: -100, y: 20, width: 640, height: 480)
        let other = TextWindowIdentity(pid: 42, title: "e\u{301}", x: -100, y: 20, width: 640, height: 480)
        XCTAssertNotEqual(first, other)
        XCTAssertTrue(first.valid)
        XCTAssertFalse(TextWindowIdentity(pid: 42, title: "window", x: .nan, y: 20, width: 640, height: 480).valid)
        XCTAssertFalse(TextWindowIdentity(pid: 42, title: "window", x: 0, y: 20, width: 0, height: 480).valid)
    }

    func testReadBudgetIsFiniteAndRevocationOrClockChangesStopFurtherCalls() throws {
        var budget = SelectedTextReadBudget(started: 100, deadline: 1000)
        for _ in 0..<256 { try budget.admit(now: 100, revoked: false) }
        XCTAssertEqual(budget.calls, 256)
        XCTAssertThrowsError(try budget.admit(now: 101, revoked: false))
        for observation in [(UInt64(99), false), (1000, false), (UInt64.max, false), (101, true)] {
            XCTAssertThrowsError(try budget.check(now: observation.0, revoked: observation.1))
        }
    }

    func testHiddenNativePeerCannotImpersonateTheVisibleSelectionThroughAX() {
        var fixture = Fixture() // AX exposes only one window, which could be the hidden peer.
        let visible = Self.target
        let hidden = Self.target // Different native IDs may share all public join metadata.
        XCTAssertThrowsError(try SelectedTextReader.read(using: &fixture, target: visible,
                                                        nativeWindows: [visible, hidden])) { error in
            XCTAssertEqual(error as? SelectedTextFailure, .ambiguous)
        }
        XCTAssertEqual(fixture.windowsRead, 0)
        XCTAssertEqual(fixture.textReads, 0)
    }

    func testNativeGlobalCountIsCheckedBeforeInspectingTheReturnedWindowSet() throws {
        try SelectedTextReader.validateNativeCount(200)
        for invalid in [0, -1, 201, Int.max] {
            XCTAssertThrowsError(try SelectedTextReader.validateNativeCount(invalid)) { error in
                XCTAssertEqual(error as? SelectedTextFailure, .limit)
            }
        }
    }
}
