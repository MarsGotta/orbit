import ApplicationServices
import CoreGraphics
import Foundation
import OrbitNativeProtocol
import OrbitNativeState

struct TextWindowCandidate: Equatable {
    let windowID: UInt32
    let title: String
    let descriptorHash: String

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.windowID == rhs.windowID && lhs.title.utf16.elementsEqual(rhs.title.utf16)
            && lhs.descriptorHash == rhs.descriptorHash
    }
}

struct TextWindowScope: Equatable {
    let windowID: UInt32
    let applicationPID: Int32
    /// The complete SCK set for this PID, including hidden/minimized windows.
    let candidates: [TextWindowCandidate]
}

/// One worker shared by all selected-text reads. The coordinator owns admission,
/// the physical slot and delivery; this worker owns blocking public AX/CG reads.
final class AXSelectedTextCapture {
    private let queue = DispatchQueue(label: "orbit.selected-text", qos: .userInitiated)

    private let diagnostics: DiagnosticLog

    init(diagnostics: DiagnosticLog) { self.diagnostics = diagnostics }

    private func check(_ passed: Bool, _ name: String, _ fields: [String: String] = [:]) -> Bool {
        recordWindowReadCheck(passed, name, diagnostics: diagnostics, fields: fields)
    }

    func read(scope: TextWindowScope, permit: WindowCapturePermit,
              revocation: WindowReadRevocation) async throws -> SelectedTextValue {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                do {
                    var access = AXTextAccess(permit: permit, revocation: revocation, diagnostics: self.diagnostics)
                    try access.check()
                    let before = try self.nativeWindows(scope)
                    guard let target = before[scope.windowID] else {
                        _ = self.check(false, "cg_selected_window_lookup")
                        throw SelectedTextFailure.ambiguous
                    }
                    try access.check()
                    let value = try SelectedTextReader.read(using: &access, target: target,
                                                           nativeWindows: Array(before.values))
                    try access.check()
                    let after = try self.nativeWindows(scope)
                    guard self.check(after == before, "cg_identity_after_read") else { throw SelectedTextFailure.identityChanged }
                    try access.check()
                    continuation.resume(returning: value)
                } catch {
                    self.diagnostics.recordOffMain("window_read_refused", ["check": "ax_selection_read",
                        "code": (error as? SelectedTextFailure)?.rawValue ?? (error as? NativeFailure)?.code ?? "selected_text_unavailable"])
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    /// Use the documented CG screen-space rectangle, NOT a guess based on SCK's
    /// frame or AppKit's flipped coordinates. Both AXPosition and CGWindowBounds
    /// specify a main-display upper-left origin. Exact equality, no tolerance.
    private func nativeWindows(_ scope: TextWindowScope) throws -> [UInt32: TextWindowIdentity] {
        guard check(CGPreflightScreenCaptureAccess(), "cg_screen_permission") else {
            throw NativeFailure("screen_permission_required")
        }
        guard check(!scope.candidates.isEmpty && scope.candidates.count <= SelectedTextReader.maximumWindows,
                    "cg_candidate_count", ["count": String(scope.candidates.count), "max": String(SelectedTextReader.maximumWindows)]) else {
            throw SelectedTextFailure.unavailable
        }
        guard let array = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) else {
            _ = check(false, "cg_window_inventory")
            throw SelectedTextFailure.unavailable
        }
        _ = check(CFArrayGetCount(array) > 0 && CFArrayGetCount(array) <= WindowSelectionPolicy.maximumWindows,
                  "cg_inventory_count", ["count": String(CFArrayGetCount(array)), "max": String(WindowSelectionPolicy.maximumWindows)])
        try SelectedTextReader.validateNativeCount(CFArrayGetCount(array))
        guard let rows = array as? [[String: Any]] else {
            _ = check(false, "cg_inventory_rows")
            throw SelectedTextFailure.unavailable
        }
        let screenCaptureWindowIDs = scope.candidates.map(\.windowID)
        guard let expected = scope.candidates.first(where: { $0.windowID == scope.windowID }) else {
            _ = check(false, "cg_expected_window")
            throw SelectedTextFailure.identityChanged
        }
        var coreGraphicsWindowIDs = [UInt32]()
        var observed = [UInt32: TextWindowIdentity]()
        for row in rows {
            guard let owner = row[kCGWindowOwnerPID as String] else {
                _ = check(false, "cg_window_owner_available")
                throw SelectedTextFailure.identityChanged
            }
            let pid = try AXTextAccess.integer(owner as AnyObject)
            guard pid == Int64(scope.applicationPID) else { continue }
            guard let number = row[kCGWindowNumber as String] else {
                _ = check(false, "cg_window_id_available")
                throw SelectedTextFailure.identityChanged
            }
            let identity = try AXTextAccess.integer(number as AnyObject)
            guard check(identity > 0 && identity <= Int64(UInt32.max), "cg_window_id_valid") else { throw SelectedTextFailure.identityChanged }
            let windowID = UInt32(identity)
            coreGraphicsWindowIDs.append(windowID)
            guard windowID == scope.windowID else { continue }
            guard observed.count < SelectedTextReader.maximumWindows,
                  let name = row[kCGWindowName as String],
                  let bounds = row[kCGWindowBounds as String] as? [String: Any] else {
                _ = check(false, "cg_selected_title_and_bounds_available")
                throw SelectedTextFailure.identityChanged
            }
            let title = try AXTextAccess.text(name as AnyObject, maximumUnits: 16_384,
                                              maximumBytes: 16_384, allowEmpty: true)
            guard check(expected.title.utf16.elementsEqual(title.utf16), "cg_selected_title_matches", ["window_id": String(windowID)]) else {
                throw SelectedTextFailure.identityChanged
            }
            var rect = CGRect.zero
            guard check(CGRectMakeWithDictionaryRepresentation(bounds as CFDictionary, &rect), "cg_selected_bounds_valid") else {
                throw SelectedTextFailure.identityChanged
            }
            let entry = TextWindowIdentity(pid: scope.applicationPID, title: title,
                x: rect.origin.x, y: rect.origin.y, width: rect.width, height: rect.height)
            guard check(entry.valid, "cg_selected_identity_valid"),
                  check(observed.updateValue(entry, forKey: windowID) == nil, "cg_unique_selected_window") else {
                throw SelectedTextFailure.identityChanged
            }
        }
        guard check(WindowIdentityJoin.containsSelectedWindow(scope.windowID,
                  screenCaptureWindowIDs: screenCaptureWindowIDs,
                  coreGraphicsWindowIDs: coreGraphicsWindowIDs), "cg_sck_window_join",
                  ["window_id": String(scope.windowID), "pid": String(scope.applicationPID),
                   "sck_windows": String(screenCaptureWindowIDs.count), "cg_windows": String(coreGraphicsWindowIDs.count)]),
              check(observed.count == 1 && observed[scope.windowID] != nil, "cg_selected_window_observed") else {
            throw SelectedTextFailure.identityChanged
        }
        return observed
    }
}
