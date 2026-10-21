import AppKit
import CryptoKit
import ScreenCaptureKit
import OrbitNativeProtocol

/// Retained owner and public metadata. None of these fields claims an immutable
/// WindowServer incarnation; every source operation must observe them again.
struct ObservedWindowTarget {
    let window: SCWindow
    let filter: SCContentFilter
    let application: NSRunningApplication
    let launchDate: Date
    let bundleID: String
    let applicationInstanceID: String
    let descriptor: ObservedWindowDescriptor
}

struct ObservedWindowDescriptor: Encodable {
    let windowID: UInt32
    let applicationPID: Int32
    let applicationInstanceID: String
    let launchDateBits: UInt64
    let bundleID: String
    let title: String
    let x: Double
    let y: Double
    let width: Double
    let height: Double
    let level: Int

    var hash: String? {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        guard let encoded = try? encoder.encode(self) else { return nil }
        return SHA256.hash(data: encoded).map { String(format: "%02x", $0) }.joined()
    }

    static func observe(window: SCWindow, applicationInstanceID: String,
                        launchDate: Date, bundleID: String) -> Self? {
        let frame = window.frame
        let title = window.title ?? ""
        let launch = launchDate.timeIntervalSinceReferenceDate
        guard window.windowID > 0, let owner = window.owningApplication, owner.processID > 0,
              frame.origin.x.isFinite, frame.origin.y.isFinite, launch.isFinite,
              WindowPresentation.captureSize(width: frame.width, height: frame.height) != nil,
              title.utf8.count <= 16_384 else { return nil }
        return Self(windowID: window.windowID, applicationPID: owner.processID,
            applicationInstanceID: applicationInstanceID, launchDateBits: launch.bitPattern,
            bundleID: bundleID, title: title, x: frame.origin.x, y: frame.origin.y,
            width: frame.width, height: frame.height, level: window.windowLayer)
    }
}
