import Foundation

/// A capability for one observed window, not an immutable WindowServer incarnation.
public struct WindowBinding: Codable, Equatable, Sendable {
    public let native_instance_id: String
    public let privacy_generation: UInt64
    public let inventory_id: String
    public let selection_token: String
    public let window_id: UInt32
    public let application_pid: Int32
    public let application_instance_id: String
    public let binding_quality: String
    public let descriptor_hash: String

    public init(nativeInstanceID: String, privacyGeneration: UInt64, inventoryID: String,
                selectionToken: String, windowID: UInt32, applicationPID: Int32,
                applicationInstanceID: String, descriptorHash: String) {
        native_instance_id = nativeInstanceID
        privacy_generation = privacyGeneration
        inventory_id = inventoryID
        selection_token = selectionToken
        window_id = windowID
        application_pid = applicationPID
        application_instance_id = applicationInstanceID
        binding_quality = "observed_owner_and_window"
        descriptor_hash = descriptorHash
    }

    public var isWellFormed: Bool {
        [native_instance_id, inventory_id, selection_token, application_instance_id].allSatisfy {
            $0.utf8.count == 36 && UUID(uuidString: $0)?.uuidString.lowercased() == $0.lowercased()
        } && window_id > 0 && application_pid > 0 && binding_quality == "observed_owner_and_window"
            && descriptor_hash.utf8.count == 64
            && descriptor_hash.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
    }

    public var fields: [String: Any] {
        ["native_instance_id": native_instance_id, "privacy_generation": privacy_generation,
         "inventory_id": inventory_id, "selection_token": selection_token, "window_id": window_id,
         "application_pid": application_pid, "application_instance_id": application_instance_id,
         "binding_quality": binding_quality, "descriptor_hash": descriptor_hash]
    }
}

public enum WindowSelectionFailure: String, Error, Equatable {
    case invalidInventory = "invalid_screen_inventory"
    case invalidBinding = "invalid_window_binding"
    case selectionUnavailable = "screen_selection_unavailable"
    case selectionExpired = "screen_selection_expired"
    case privacyStopped = "privacy_stopped"
    case identityChanged = "window_identity_changed"
    case captureBusy = "screenshot_busy"
    case captureExpired = "screenshot_expired"
}

public struct WindowCapturePermit: Equatable, Sendable {
    public let binding: WindowBinding
    public let kind: WindowCaptureKind
    public let startedNanoseconds: UInt64
    public let deadlineNanoseconds: UInt64
}

public enum WindowCaptureKind: Equatable, Sendable {
    case screenshot, selectedText

    public var lifetimeNanoseconds: UInt64 {
        self == .screenshot ? 10_000_000_000 : 5_000_000_000
    }
}

/// Pure bounded state machine. Time, identity and privacy facts are supplied by adapters.
/// Revoking delivery NEVER frees the physical capture slot: only its completion does.
public struct WindowSelectionPolicy {
    public static let maximumWindows = 200
    public static let inventoryLifetimeNanoseconds: UInt64 = 30_000_000_000
    public static let captureLifetimeNanoseconds: UInt64 = 10_000_000_000
    public static let maximumInventoryBytes = NativeLimits.maximumWindowInventoryBytes
    private let nativeInstanceID: String
    private var inventoryID: String?
    private var observedNanoseconds: UInt64 = 0
    private var expiresNanoseconds: UInt64 = 0
    private var available = [String: WindowBinding]()
    private var pending: WindowCapturePermit?
    private var deliveryAllowed = false

    public init(nativeInstanceID: String) { self.nativeInstanceID = nativeInstanceID }
    public var capturePending: Bool { pending != nil }

    public mutating func issue(inventoryID: String, observedNanoseconds: UInt64,
                               expiresNanoseconds: UInt64, privacyGeneration: UInt64,
                               bindings: [WindowBinding]) throws {
        invalidate()
        guard bindings.count <= Self.maximumWindows, expiresNanoseconds > observedNanoseconds,
              expiresNanoseconds - observedNanoseconds <= Self.inventoryLifetimeNanoseconds,
              inventoryID.utf8.count == 36, UUID(uuidString: inventoryID) != nil else {
            throw WindowSelectionFailure.invalidInventory
        }
        var issued = [String: WindowBinding]()
        for binding in bindings {
            guard binding.isWellFormed, binding.native_instance_id == nativeInstanceID,
                  binding.inventory_id == inventoryID, binding.privacy_generation == privacyGeneration,
                  issued.updateValue(binding, forKey: binding.selection_token) == nil else {
                throw WindowSelectionFailure.invalidInventory
            }
        }
        self.inventoryID = inventoryID
        self.observedNanoseconds = observedNanoseconds
        self.expiresNanoseconds = expiresNanoseconds
        available = issued
    }

    public mutating func consume(_ binding: WindowBinding?, now: UInt64,
                                 privacyGeneration: UInt64, privacyStopped: Bool,
                                 kind: WindowCaptureKind = .screenshot) throws -> WindowCapturePermit {
        guard !privacyStopped else { throw WindowSelectionFailure.privacyStopped }
        guard !capturePending else { throw WindowSelectionFailure.captureBusy }
        guard let binding, binding.isWellFormed else { throw WindowSelectionFailure.invalidBinding }
        guard binding.native_instance_id == nativeInstanceID, binding.privacy_generation == privacyGeneration,
              binding.inventory_id == inventoryID, available[binding.selection_token] == binding else {
            throw WindowSelectionFailure.selectionUnavailable
        }
        // A backwards clock is not fresh. Expiration is exclusive.
        guard now >= observedNanoseconds, now < expiresNanoseconds else {
            throw WindowSelectionFailure.selectionExpired
        }
        let lifetime = min(kind.lifetimeNanoseconds, expiresNanoseconds - now)
        let permit = WindowCapturePermit(binding: binding, kind: kind, startedNanoseconds: now,
                                         deadlineNanoseconds: now + lifetime)
        available.removeValue(forKey: binding.selection_token)
        pending = permit
        deliveryAllowed = true
        return permit
    }

    public func validate(_ permit: WindowCapturePermit, now: UInt64, privacyGeneration: UInt64,
                         privacyStopped: Bool, descriptorHash: String?) throws {
        guard !privacyStopped, permit.binding.privacy_generation == privacyGeneration else {
            throw WindowSelectionFailure.privacyStopped
        }
        guard pending == permit, deliveryAllowed, now >= permit.startedNanoseconds,
              now < permit.deadlineNanoseconds else { throw WindowSelectionFailure.captureExpired }
        guard descriptorHash == permit.binding.descriptor_hash else {
            throw WindowSelectionFailure.identityChanged
        }
    }

    public mutating func expire(_ permit: WindowCapturePermit) {
        if pending == permit { deliveryAllowed = false }
    }

    public mutating func complete(_ permit: WindowCapturePermit) {
        guard pending == permit else { return }
        pending = nil
        deliveryAllowed = false
    }

    public mutating func invalidate() {
        available.removeAll()
        inventoryID = nil
        deliveryAllowed = false
    }
}

public enum WindowPresentation {
    /// String scalars are copied only while they fit; no split UTF-8 sequences.
    public static func bounded(_ text: String, maximumBytes: Int) -> String {
        var output = String.UnicodeScalarView()
        var count = 0
        for scalar in text.unicodeScalars {
            let size = scalar.utf8.count
            guard count + size <= maximumBytes else { break }
            output.append(scalar)
            count += size
        }
        return String(output)
    }

    public static func captureSize(width: Double, height: Double) -> (width: Int, height: Int)? {
        guard width.isFinite, height.isFinite, width > 0, height > 0 else { return nil }
        let scale = min(1, min(1280 / width, 1024 / height))
        let w = width * scale
        let h = height * scale
        guard w.isFinite, h.isFinite, w >= 1, h >= 1, w <= 1280, h <= 1024 else { return nil }
        return (Int(w), Int(h))
    }
}

/// A selected-text binding joins one chosen SCK window to its CG observation.
/// Other windows owned by the PID are not part of that identity proof.
public enum WindowIdentityJoin {
    public static func containsSelectedWindow(_ windowID: UInt32,
                                              screenCaptureWindowIDs: [UInt32],
                                              coreGraphicsWindowIDs: [UInt32]) -> Bool {
        windowID > 0 && screenCaptureWindowIDs.contains(windowID)
            && coreGraphicsWindowIDs.contains(windowID)
    }
}
