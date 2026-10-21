import ApplicationServices
import Foundation
import OrbitNativeProtocol
import OrbitNativeState

/// Used only by the serial AX worker. No AX setters/actions, clipboard or global
/// focus queries are exposed. Timeouts are configured on EACH acquired object;
/// Apple explicitly does not propagate them to an equal AXUIElement object.
struct AXTextAccess: SelectedTextAccess {
    typealias Element = AXUIElement
    private var budget: SelectedTextReadBudget
    private let revocation: WindowReadRevocation
    private let diagnostics: DiagnosticLog

    init(permit: WindowCapturePermit, revocation: WindowReadRevocation, diagnostics: DiagnosticLog) {
        budget = SelectedTextReadBudget(started: permit.startedNanoseconds, deadline: permit.deadlineNanoseconds)
        self.revocation = revocation
        self.diagnostics = diagnostics
    }

    mutating func check() throws {
        guard let now = CaptureClock.currentHostNanoseconds() else { throw SelectedTextFailure.expired }
        try budget.check(now: now, revoked: revocation.revoked)
        guard recordWindowReadCheck(AXIsProcessTrusted(), "ax_accessibility_permission", diagnostics: diagnostics) else { throw SelectedTextFailure.permissionRequired }
    }

    private mutating func admit() throws {
        try check()
        guard let now = CaptureClock.currentHostNanoseconds() else { throw SelectedTextFailure.expired }
        try budget.admit(now: now, revoked: revocation.revoked)
    }

    private mutating func require(_ error: AXError, _ operation: String) throws {
        try check()
        guard recordWindowReadCheck(error == .success, operation, diagnostics: diagnostics,
                                    fields: ["ax_error": String(error.rawValue)]) else {
            if error == .attributeUnsupported || error == .parameterizedAttributeUnsupported || error == .notImplemented {
                throw SelectedTextFailure.notExposed
            }
            // noValue/cannotComplete do not establish that the selection is empty.
            throw SelectedTextFailure.unavailable
        }
    }

    private mutating func configured(_ element: AXUIElement) throws -> AXUIElement {
        try admit()
        try require(AXUIElementSetMessagingTimeout(element, 0.1), "ax_messaging_timeout")
        return element
    }

    private mutating func element(_ value: CFTypeRef) throws -> AXUIElement {
        guard CFGetTypeID(value) == AXUIElementGetTypeID() else { throw SelectedTextFailure.unavailable }
        // The CF runtime type has been checked before this toll-free conversion.
        return try configured(unsafeBitCast(value, to: AXUIElement.self))
    }

    func recordCheck(_ name: String, passed: Bool) {
        _ = recordWindowReadCheck(passed, name, diagnostics: diagnostics)
    }

    func same(_ lhs: AXUIElement, _ rhs: AXUIElement) -> Bool { CFEqual(lhs, rhs) }

    mutating func application(pid: Int32) throws -> AXUIElement {
        guard checked(pid > 0, "ax_application_pid_valid") else { throw SelectedTextFailure.identityChanged }
        try admit()
        return try configured(AXUIElementCreateApplication(pid))
    }

    mutating func pid(_ element: AXUIElement) throws -> Int32 {
        var owner: pid_t = 0
        try admit()
        try require(AXUIElementGetPid(element, &owner), "ax_element_pid")
        guard checked(owner > 0, "ax_owner_valid") else { throw SelectedTextFailure.identityChanged }
        return owner
    }

    private mutating func count(_ element: AXUIElement, _ attribute: String) throws -> Int {
        var count = 0
        try admit()
        try require(AXUIElementGetAttributeValueCount(element, attribute as CFString, &count), "ax_count_\(attribute)")
        return count
    }

    private mutating func values(_ element: AXUIElement, _ attribute: String, count: Int) throws -> [CFTypeRef] {
        var array: CFArray?
        try admit()
        try require(AXUIElementCopyAttributeValues(element, attribute as CFString, 0, count, &array), "ax_values_\(attribute)")
        guard let array else { throw SelectedTextFailure.unavailable }
        return try Self.array(array, exactCount: count)
    }

    mutating func windows(_ application: AXUIElement) throws -> [AXUIElement] {
        let size = try count(application, kAXWindowsAttribute)
        guard (1...SelectedTextReader.maximumWindows).contains(size) else { throw SelectedTextFailure.limit }
        let raw = try values(application, kAXWindowsAttribute, count: size)
        let afterSize = try count(application, kAXWindowsAttribute)
        guard checked(afterSize == size, "ax_window_count_stable") else { throw SelectedTextFailure.identityChanged }
        return try raw.map { try element($0) }
    }

    private mutating func attribute(_ element: AXUIElement, _ attribute: String) throws -> CFTypeRef {
        var value: CFTypeRef?
        try admit()
        try require(AXUIElementCopyAttributeValue(element, attribute as CFString, &value), "ax_attribute_\(attribute)")
        guard let value else { throw SelectedTextFailure.unavailable }
        return value
    }

    private mutating func attributes(_ element: AXUIElement, _ names: [String]) throws -> [CFTypeRef] {
        var values: CFArray?
        try admit()
        try require(AXUIElementCopyMultipleAttributeValues(element, names as CFArray,
                    AXCopyMultipleAttributeOptions(rawValue: 0), &values), "ax_attributes_\(names.joined(separator: "_"))")
        guard let values else { throw SelectedTextFailure.unavailable }
        return try Self.array(values, exactCount: names.count)
    }

    mutating func identity(_ window: AXUIElement) throws -> TextWindowIdentity {
        let owner = try pid(window)
        let fields = try attributes(window, [kAXRoleAttribute, kAXTitleAttribute, kAXPositionAttribute, kAXSizeAttribute])
        guard try checked(Self.text(fields[0], maximumUnits: 128, maximumBytes: 256) == kAXWindowRole, "ax_window_role") else {
            throw SelectedTextFailure.identityChanged
        }
        let title = try Self.text(fields[1], maximumUnits: 16_384, maximumBytes: 16_384, allowEmpty: true)
        let position = try Self.axValue(fields[2], type: .cgPoint)
        let dimensions = try Self.axValue(fields[3], type: .cgSize)
        var point = CGPoint.zero
        var size = CGSize.zero
        guard checked(AXValueGetValue(position, .cgPoint, &point) && AXValueGetValue(dimensions, .cgSize, &size), "ax_window_bounds") else {
            throw SelectedTextFailure.identityChanged
        }
        return TextWindowIdentity(pid: owner, title: title, x: point.x, y: point.y, width: size.width, height: size.height)
    }

    mutating func focused(_ application: AXUIElement) throws -> AXUIElement {
        let value = try attribute(application, kAXFocusedUIElementAttribute)
        return try element(value)
    }

    mutating func window(_ element: AXUIElement) throws -> AXUIElement {
        let value = try attribute(element, kAXWindowAttribute)
        return try self.element(value)
    }

    mutating func classification(_ element: AXUIElement) throws -> TextElementClassification {
        let fields = try attributes(element, [kAXRoleAttribute, kAXSubroleAttribute])
        // Missing/unsupported/error-valued subrole is incomplete, not evidence
        // that a text element is nonsecure. No content getter precedes this.
        guard let role = try? Self.text(fields[0], maximumUnits: 128, maximumBytes: 256),
              let subrole = try? Self.text(fields[1], maximumUnits: 128, maximumBytes: 256) else {
            throw SelectedTextFailure.classification
        }
        return TextElementClassification(role: role, subrole: subrole)
    }

    mutating func range(_ element: AXUIElement) throws -> TextSelectionRange {
        try Self.range(attribute(element, kAXSelectedTextRangeAttribute))
    }

    mutating func numberOfCharacters(_ element: AXUIElement) throws -> Int64 {
        try Self.integer(attribute(element, kAXNumberOfCharactersAttribute))
    }

    mutating func ranges(_ element: AXUIElement) throws -> [TextSelectionRange]? {
        var size = 0
        try admit()
        let error = AXUIElementGetAttributeValueCount(element, kAXSelectedTextRangesAttribute as CFString, &size)
        try check()
        if error == .attributeUnsupported { return nil }
        try require(error, "ax_selected_ranges_count")
        guard size == 1 else { throw SelectedTextFailure.invalidRange }
        return try values(element, kAXSelectedTextRangesAttribute, count: 1).map(Self.range)
    }

    mutating func selectedText(_ element: AXUIElement) throws -> [UInt16] {
        try Self.units(attribute(element, kAXSelectedTextAttribute), maximumUnits: 4096)
    }

    mutating func string(_ element: AXUIElement, range: TextSelectionRange) throws -> [UInt16] {
        var raw = CFRange(location: Int(range.location), length: Int(range.length))
        guard let parameter = AXValueCreate(.cfRange, &raw) else { throw SelectedTextFailure.invalidRange }
        var output: CFTypeRef?
        try admit()
        try require(AXUIElementCopyParameterizedAttributeValue(element,
                    kAXStringForRangeParameterizedAttribute as CFString, parameter, &output), "ax_string_for_selected_range")
        guard let output else { throw SelectedTextFailure.unavailable }
        return try Self.units(output, maximumUnits: 4096)
    }

    static func integer(_ value: CFTypeRef) throws -> Int64 {
        guard CFGetTypeID(value) == CFNumberGetTypeID() else { throw SelectedTextFailure.unavailable }
        let number = unsafeBitCast(value, to: CFNumber.self)
        var result: Int64 = 0
        guard !CFNumberIsFloatType(number), CFNumberGetValue(number, .sInt64Type, &result) else {
            throw SelectedTextFailure.unavailable
        }
        return result
    }

    static func units(_ value: CFTypeRef, maximumUnits: Int) throws -> [UInt16] {
        guard CFGetTypeID(value) == CFStringGetTypeID() else { throw SelectedTextFailure.invalidText }
        let string = unsafeBitCast(value, to: CFString.self)
        let count = CFStringGetLength(string)
        guard count >= 0, count <= maximumUnits else { throw SelectedTextFailure.limit }
        var units = [UInt16](repeating: 0, count: count)
        if count > 0 { CFStringGetCharacters(string, CFRange(location: 0, length: count), &units) }
        return units
    }

    static func text(_ value: CFTypeRef, maximumUnits: Int, maximumBytes: Int, allowEmpty: Bool = false) throws -> String {
        try StrictText.decode(units(value, maximumUnits: maximumUnits), maximumUnits: maximumUnits,
                              maximumBytes: maximumBytes, allowEmpty: allowEmpty)
    }

    private static func range(_ value: CFTypeRef) throws -> TextSelectionRange {
        let value = try axValue(value, type: .cfRange)
        var range = CFRange(location: 0, length: 0)
        guard AXValueGetValue(value, .cfRange, &range) else { throw SelectedTextFailure.invalidRange }
        return TextSelectionRange(location: Int64(range.location), length: Int64(range.length))
    }

    private static func axValue(_ value: CFTypeRef, type: AXValueType) throws -> AXValue {
        guard CFGetTypeID(value) == AXValueGetTypeID() else { throw SelectedTextFailure.unavailable }
        let typed = unsafeBitCast(value, to: AXValue.self)
        guard AXValueGetType(typed) == type else { throw SelectedTextFailure.unavailable }
        return typed
    }

    private static func array(_ array: CFArray, exactCount: Int) throws -> [CFTypeRef] {
        guard exactCount <= SelectedTextReader.maximumWindows, CFArrayGetCount(array) == exactCount else {
            throw SelectedTextFailure.limit
        }
        return try (0..<exactCount).map { index in
            guard let raw = CFArrayGetValueAtIndex(array, index) else { throw SelectedTextFailure.unavailable }
            return Unmanaged<AnyObject>.fromOpaque(raw).takeUnretainedValue()
        }
    }
}
