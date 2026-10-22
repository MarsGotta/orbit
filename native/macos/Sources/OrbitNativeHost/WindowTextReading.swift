import ApplicationServices
import CoreGraphics
import Foundation
import OrbitNativeProtocol

/// «Adjuntar ventana» without the engine: the text Accessibility shows in
/// the window just captured. Read-only: no AX setter or action, and a secure
/// field's value is never asked for. Nothing read is logged; a failure is a
/// fixed code.
package final class WindowTextReading: @unchecked Sendable {
    /// Accessibility calls block on the other app and never run on AppKit's thread.
    private let queue = DispatchQueue(label: "orbit.native.window-text", qos: .userInitiated)

    package init() {}

    package func read(_ fields: NativeWindowTextFields) async throws -> [String: Any] {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                do { continuation.resume(returning: try Self.readNow(fields)) }
                catch { continuation.resume(throwing: error) }
            }
        }
    }

    static func readNow(_ fields: NativeWindowTextFields) throws -> [String: Any] {
        guard AXIsProcessTrusted() else { throw NativeFailure("accessibility_permission_required") }
        let started = DispatchTime.now().uptimeNanoseconds
        let window = try AXWindowText.window(pid: fields.application_pid, windowID: fields.window_id)
        var access = AXWindowText()
        let read = WindowTextReader.read(using: &access, window: window, limits: fields.limits) {
            (DispatchTime.now().uptimeNanoseconds &- started) / 1_000_000
        }
        var result: [String: Any] = ["text": read.text]
        if let stopped = read.stopped { result["stopped"] = stopped.rawValue }
        return result
    }
}

/// One window's elements through the public Accessibility API.
struct AXWindowText: WindowTextAccess {
    /// Set on each element: Apple does not carry a timeout from one element
    /// to another. A quarter of a second lets a busy app answer; the walk's
    /// own deadline bounds the whole read.
    static let messagingTimeout: Float = 0.25

    mutating func node(_ element: AXUIElement) throws -> WindowTextNode<AXUIElement> {
        AXUIElementSetMessagingTimeout(element, Self.messagingTimeout)
        let shape = try Self.values(element, [kAXSubroleAttribute, kAXChildrenAttribute])
        // The subrole is read before any text getter.
        guard Self.string(shape[0]) != "AXSecureTextField" else {
            return WindowTextNode(secure: true, texts: [], children: [])
        }
        let texts = try Self.values(element, [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute])
            .compactMap(Self.string)
        return WindowTextNode(secure: false, texts: texts, children: Self.elements(shape[1]))
    }

    /// The window `windowID` of `pid`, found among the app's Accessibility
    /// windows by its frame, which CoreGraphics and Accessibility give in the
    /// same screen space. Exactly one must match.
    static func window(pid: Int32, windowID: UInt32) throws -> AXUIElement {
        guard pid > 0,
              let rows = CGWindowListCopyWindowInfo([.optionIncludingWindow], CGWindowID(windowID)) as? [[String: Any]],
              let row = rows.first(where: { ($0[kCGWindowNumber as String] as? NSNumber)?.uint32Value == windowID }),
              (row[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid,
              let bounds = row[kCGWindowBounds as String] as? [String: Any] else {
            throw NativeFailure("window_text_window_gone")
        }
        var frame = CGRect.zero
        guard CGRectMakeWithDictionaryRepresentation(bounds as CFDictionary, &frame) else {
            throw NativeFailure("window_text_window_gone")
        }
        let application = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(application, messagingTimeout)
        var windows: CFTypeRef?
        guard AXUIElementCopyAttributeValue(application, kAXWindowsAttribute as CFString, &windows) == .success,
              let windows else {
            throw NativeFailure("window_text_window_not_found")
        }
        let matches = elements(windows).filter { Self.frame(of: $0) == frame }
        guard matches.count == 1, let window = matches.first else {
            throw NativeFailure(matches.isEmpty ? "window_text_window_not_found" : "window_text_window_ambiguous")
        }
        return window
    }

    private static func frame(of window: AXUIElement) -> CGRect? {
        AXUIElementSetMessagingTimeout(window, messagingTimeout)
        guard let values = try? values(window, [kAXPositionAttribute, kAXSizeAttribute]),
              CFGetTypeID(values[0]) == AXValueGetTypeID(), CFGetTypeID(values[1]) == AXValueGetTypeID() else {
            return nil
        }
        var origin = CGPoint.zero
        var size = CGSize.zero
        // The CF runtime types have been checked before these conversions.
        guard AXValueGetValue(unsafeBitCast(values[0], to: AXValue.self), .cgPoint, &origin),
              AXValueGetValue(unsafeBitCast(values[1], to: AXValue.self), .cgSize, &size) else {
            return nil
        }
        return CGRect(origin: origin, size: size)
    }

    private static func values(_ element: AXUIElement, _ names: [String]) throws -> [AnyObject] {
        var values: CFArray?
        guard AXUIElementCopyMultipleAttributeValues(element, names as CFArray,
                                                     AXCopyMultipleAttributeOptions(rawValue: 0), &values) == .success,
              let values, CFArrayGetCount(values) == names.count else {
            throw NativeFailure("window_text_element_unreadable")
        }
        return objects(values)
    }

    /// An attribute that is a string; any other value (a missing attribute
    /// comes back as an error value) is none.
    private static func string(_ value: AnyObject) -> String? {
        guard CFGetTypeID(value) == CFStringGetTypeID() else { return nil }
        return unsafeBitCast(value, to: CFString.self) as String
    }

    /// The elements of an array attribute; anything else is none.
    private static func elements(_ value: AnyObject) -> [AXUIElement] {
        guard CFGetTypeID(value) == CFArrayGetTypeID() else { return [] }
        return objects(unsafeBitCast(value, to: CFArray.self)).compactMap { item in
            CFGetTypeID(item) == AXUIElementGetTypeID() ? unsafeBitCast(item, to: AXUIElement.self) : nil
        }
    }

    private static func objects(_ array: CFArray) -> [AnyObject] {
        (0..<CFArrayGetCount(array)).compactMap { index in
            guard let raw = CFArrayGetValueAtIndex(array, index) else { return nil }
            return Unmanaged<AnyObject>.fromOpaque(raw).takeUnretainedValue()
        }
    }
}
