import Foundation

public enum SelectedTextFailure: String, Error, Equatable {
    case empty = "selected_text_empty"
    case notExposed = "selected_text_not_exposed"
    case unavailable = "selected_text_unavailable"
    case permissionRequired = "accessibility_permission_required"
    case classification = "selected_text_classification_incomplete"
    case secure = "selected_text_secure_field"
    case ambiguous = "selected_text_window_ambiguous"
    case identityChanged = "selected_text_identity_changed"
    case invalidRange = "selected_text_invalid_range"
    case invalidText = "selected_text_invalid_text"
    case changed = "selected_text_changed"
    case limit = "selected_text_limit"
    case expired = "selected_text_expired"
}

public struct TextSelectionRange: Equatable {
    public let location: Int64
    public let length: Int64

    public init(location: Int64, length: Int64) {
        self.location = location
        self.length = length
    }

    public func validate(total: Int64) throws {
        let (end, overflow) = location.addingReportingOverflow(length)
        guard location >= 0, (0...4096).contains(length), !overflow,
              total >= 0, end <= total else { throw SelectedTextFailure.invalidRange }
        guard length > 0 else { throw SelectedTextFailure.empty }
    }

    public var fields: [String: Any] {
        ["location": location, "length": length, "unit": "utf16_code_units"]
    }
}

public struct TextWindowIdentity: Equatable {
    public let pid: Int32
    public let title: String
    public let x: Double
    public let y: Double
    public let width: Double
    public let height: Double

    public init(pid: Int32, title: String, x: Double, y: Double, width: Double, height: Double) {
        self.pid = pid
        self.title = title
        self.x = x
        self.y = y
        self.width = width
        self.height = height
    }

    public var valid: Bool {
        pid > 0 && title.utf8.count <= 16_384 && !title.utf8.contains(0)
            && [x, y, width, height].allSatisfy(\.isFinite) && width > 0 && height > 0
    }

    public static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.pid == rhs.pid && lhs.title.utf16.elementsEqual(rhs.title.utf16)
            && lhs.x == rhs.x && lhs.y == rhs.y && lhs.width == rhs.width && lhs.height == rhs.height
    }
}

public struct TextElementClassification {
    public let role: String?
    public let subrole: String?

    public init(role: String?, subrole: String?) { self.role = role; self.subrole = subrole }

    public func validate() throws {
        guard let role, let subrole, !subrole.isEmpty else { throw SelectedTextFailure.classification }
        guard subrole != "AXSecureTextField" else { throw SelectedTextFailure.secure }
        guard role == "AXTextField" || role == "AXTextArea" else { throw SelectedTextFailure.classification }
    }
}

/// AX strings may contain unpaired UTF-16. Validate before constructing a Swift
/// String: its normal decoding initializer would silently repair malformed input.
public enum StrictText {
    public static func decode(_ units: [UInt16], maximumUnits: Int = 4096,
                              maximumBytes: Int = 16_384, allowEmpty: Bool = false) throws -> String {
        guard units.count <= maximumUnits, allowEmpty || !units.isEmpty else { throw SelectedTextFailure.invalidText }
        var index = 0
        while index < units.count {
            let value = units[index]
            guard value != 0 else { throw SelectedTextFailure.invalidText }
            if (0xD800...0xDBFF).contains(value) {
                guard index + 1 < units.count, (0xDC00...0xDFFF).contains(units[index + 1]) else {
                    throw SelectedTextFailure.invalidText
                }
                index += 2
            } else {
                guard !(0xDC00...0xDFFF).contains(value) else { throw SelectedTextFailure.invalidText }
                index += 1
            }
        }
        let result = String(decoding: units, as: UTF16.self)
        guard result.utf16.elementsEqual(units), result.utf8.count <= maximumBytes else {
            throw SelectedTextFailure.invalidText
        }
        return result
    }
}

/// Framework-independent contract. An adapter owns object timeouts, call budget
/// and read-only AX operations; the algorithm owns classification and binding.
public protocol SelectedTextAccess {
    associatedtype Element
    func recordCheck(_ name: String, passed: Bool)
    func same(_ lhs: Element, _ rhs: Element) -> Bool
    mutating func application(pid: Int32) throws -> Element
    mutating func windows(_ application: Element) throws -> [Element]
    mutating func identity(_ window: Element) throws -> TextWindowIdentity
    mutating func pid(_ element: Element) throws -> Int32
    mutating func focused(_ application: Element) throws -> Element
    mutating func window(_ element: Element) throws -> Element
    mutating func classification(_ element: Element) throws -> TextElementClassification
    mutating func range(_ element: Element) throws -> TextSelectionRange
    mutating func numberOfCharacters(_ element: Element) throws -> Int64
    mutating func ranges(_ element: Element) throws -> [TextSelectionRange]?
    mutating func selectedText(_ element: Element) throws -> [UInt16]
    mutating func string(_ element: Element, range: TextSelectionRange) throws -> [UInt16]
}

public extension SelectedTextAccess {
    func recordCheck(_ name: String, passed: Bool) {}

    func checked(_ passed: Bool, _ name: String) -> Bool {
        recordCheck(name, passed: passed)
        return passed
    }
}

public struct SelectedTextValue {
    public let text: String
    public let range: TextSelectionRange
}

public enum SelectedTextReader {
    public static let maximumWindows = 32

    public static func validateNativeCount(_ count: Int) throws {
        guard count > 0, count <= WindowSelectionPolicy.maximumWindows else { throw SelectedTextFailure.limit }
    }

    /// The native join must cover hidden windows too. An AX provider may omit
    /// the selected visible window but expose a hidden peer with equal metadata.
    public static func read<A: SelectedTextAccess>(using access: inout A, target: TextWindowIdentity,
                                                  nativeWindows: [TextWindowIdentity]) throws -> SelectedTextValue {
        guard access.checked(!nativeWindows.isEmpty && nativeWindows.count <= maximumWindows, "text_native_window_count"),
              access.checked(nativeWindows.allSatisfy({ $0.valid && $0.pid == target.pid }), "text_native_window_identities") else {
            throw SelectedTextFailure.limit
        }
        guard access.checked(nativeWindows.filter({ $0 == target }).count == 1, "text_unique_native_target") else { throw SelectedTextFailure.ambiguous }
        return try readAccessibility(using: &access, target: target)
    }

    private static func readAccessibility<A: SelectedTextAccess>(using access: inout A,
                                                                target: TextWindowIdentity) throws -> SelectedTextValue {
        guard access.checked(target.valid, "text_target_valid") else { throw SelectedTextFailure.identityChanged }
        let app = try access.application(pid: target.pid)
        let bound = try uniqueWindow(using: &access, app: app, target: target)
        let element = try access.focused(app)
        try validateFocus(using: &access, app: app, element: element, window: bound, pid: target.pid)
        let range = try checkedRange(using: &access, element: element)
        let units = try access.selectedText(element)
        guard access.checked(Int64(units.count) == range.length, "text_selection_length") else { throw SelectedTextFailure.changed }
        let substring = try access.string(element, range: range)
        guard access.checked(substring == units, "text_selection_substring") else { throw SelectedTextFailure.changed }
        let text = try StrictText.decode(units)
        // Re-enumerate the complete bounded set. Equal titles/bounds are not an
        // excuse to choose the first object, nor to accept a replacement object.
        let after = try uniqueWindow(using: &access, app: app, target: target)
        guard access.checked(access.same(bound, after), "text_ax_window_after_read") else { throw SelectedTextFailure.identityChanged }
        try validateFocus(using: &access, app: app, element: element, window: bound, pid: target.pid)
        let afterRange = try checkedRange(using: &access, element: element)
        guard access.checked(afterRange == range, "text_range_after_read") else { throw SelectedTextFailure.changed }
        let afterUnits = try access.selectedText(element)
        guard access.checked(afterUnits == units, "text_selection_after_read") else { throw SelectedTextFailure.changed }
        let afterSubstring = try access.string(element, range: range)
        guard access.checked(afterSubstring == units, "text_substring_after_read") else { throw SelectedTextFailure.changed }
        return SelectedTextValue(text: text, range: range)
    }

    private static func uniqueWindow<A: SelectedTextAccess>(using access: inout A, app: A.Element,
                                                            target: TextWindowIdentity) throws -> A.Element {
        let windows = try access.windows(app)
        guard access.checked(!windows.isEmpty && windows.count <= maximumWindows, "text_ax_window_count") else { throw SelectedTextFailure.limit }
        var matches = [A.Element]()
        for window in windows {
            let identity = try access.identity(window)
            guard access.checked(identity.valid && identity.pid == target.pid, "text_ax_window_identity") else { throw SelectedTextFailure.identityChanged }
            if identity == target { matches.append(window) }
        }
        guard access.checked(matches.count == 1, "text_unique_ax_target"), let only = matches.first else { throw SelectedTextFailure.ambiguous }
        return only
    }

    private static func validateFocus<A: SelectedTextAccess>(using access: inout A, app: A.Element,
                                                            element: A.Element, window: A.Element, pid: Int32) throws {
        let focused = try access.focused(app)
        let containing = try access.window(element)
        guard access.checked(access.same(element, focused), "text_ax_focus"),
              access.checked(access.same(window, containing), "text_ax_containing_window") else { throw SelectedTextFailure.identityChanged }
        let owner = try access.pid(element)
        guard access.checked(owner == pid, "text_ax_element_owner") else { throw SelectedTextFailure.identityChanged }
        // Classification is checked before ANY text getter, and again on reread.
        do {
            try access.classification(element).validate()
            access.recordCheck("text_field_classification", passed: true)
        } catch {
            access.recordCheck("text_field_classification", passed: false)
            throw error
        }
    }

    private static func checkedRange<A: SelectedTextAccess>(using access: inout A,
                                                           element: A.Element) throws -> TextSelectionRange {
        let range = try access.range(element)
        do {
            try range.validate(total: access.numberOfCharacters(element))
            access.recordCheck("text_selection_range", passed: true)
        } catch {
            access.recordCheck("text_selection_range", passed: false)
            throw error
        }
        if let ranges = try access.ranges(element), !access.checked(ranges == [range], "text_single_selection_range") {
            throw SelectedTextFailure.invalidRange
        }
        return range
    }
}

public struct SelectedTextReadBudget {
    public static let maximumCalls = 256
    private let started: UInt64
    private let deadline: UInt64
    public private(set) var calls = 0

    public init(started: UInt64, deadline: UInt64) { self.started = started; self.deadline = deadline }

    public mutating func admit(now: UInt64, revoked: Bool) throws {
        try check(now: now, revoked: revoked)
        guard calls < Self.maximumCalls else { throw SelectedTextFailure.limit }
        calls += 1
    }

    public func check(now: UInt64, revoked: Bool) throws {
        guard !revoked, now >= started, now < deadline else { throw SelectedTextFailure.expired }
    }
}
