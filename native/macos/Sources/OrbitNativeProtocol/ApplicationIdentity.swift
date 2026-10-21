import Foundation

/// Observed running-code requirement, not notarization or publisher approval.
public struct StableApplicationIdentity: Codable, Equatable, Sendable {
    public let bundle_id: String
    public let bundle_path: String
    public let designated_requirement_sha256: String

    public init(bundleID: String, bundlePath: String, requirementSHA256: String) {
        bundle_id = bundleID
        bundle_path = bundlePath
        designated_requirement_sha256 = requirementSHA256
    }

    public var isWellFormed: Bool {
        ApplicationIdentityPolicy.validBundleID(bundle_id)
            && bundle_path.hasPrefix("/") && bundle_path.utf8.count <= 4096
            && !bundle_path.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) })
            && bundle_path.split(separator: "/", omittingEmptySubsequences: false).dropFirst()
                .allSatisfy { !$0.isEmpty && $0 != "." && $0 != ".." }
            && designated_requirement_sha256.utf8.count == 64
            && designated_requirement_sha256.utf8.allSatisfy {
                (48...57).contains($0) || (97...102).contains($0)
            }
    }

    public var fields: [String: Any] {
        ["bundle_id": bundle_id, "bundle_path": bundle_path,
         "designated_requirement_sha256": designated_requirement_sha256]
    }
}

/// A short-lived read receipt. It grants neither TCC nor permission to capture.
/// Instance ID survives metadata refresh; identity_token identifies this receipt.
public struct ApplicationIdentityBinding: Codable, Equatable, Sendable {
    public let native_instance_id: String
    public let privacy_generation: UInt64
    public let application_pid: Int32
    public let application_instance_id: String
    public let identity_token: String
    public let identity: StableApplicationIdentity
    public let clock_domain: String
    public let observed_monotonic_ns: UInt64
    public let expires_monotonic_ns: UInt64

    public init(nativeInstanceID: String, privacyGeneration: UInt64,
                applicationPID: Int32, applicationInstanceID: String, identityToken: String,
                identity: StableApplicationIdentity, observedNanoseconds: UInt64,
                expiresNanoseconds: UInt64) {
        native_instance_id = nativeInstanceID
        privacy_generation = privacyGeneration
        application_pid = applicationPID
        application_instance_id = applicationInstanceID
        identity_token = identityToken
        self.identity = identity
        clock_domain = "mach_absolute_time"
        observed_monotonic_ns = observedNanoseconds
        expires_monotonic_ns = expiresNanoseconds
    }

    public var isWellFormed: Bool {
        [native_instance_id, application_instance_id, identity_token].allSatisfy(ApplicationIdentityPolicy.validUUID)
            && application_pid > 0 && identity.isWellFormed && clock_domain == "mach_absolute_time"
            && observed_monotonic_ns > 0 && expires_monotonic_ns > observed_monotonic_ns
            && expires_monotonic_ns - observed_monotonic_ns <= ApplicationIdentityPolicy.lifetimeNanoseconds
    }

    public var fields: [String: Any] {
        ["native_instance_id": native_instance_id, "privacy_generation": privacy_generation,
         "application_pid": application_pid, "application_instance_id": application_instance_id,
         "identity_token": identity_token, "identity": identity.fields,
         "clock_domain": clock_domain, "observed_monotonic_ns": observed_monotonic_ns,
         "expires_monotonic_ns": expires_monotonic_ns]
    }
}

public enum ApplicationIdentityPolicy {
    public static let lifetimeNanoseconds: UInt64 = 15_000_000_000
    public static let maximumBundleIDs = 8
    public static let maximumApplications = 32
    // Bounds one inventory reply allocation, not the number of running apps.
    // 192 KiB accommodates 32 identity bindings with their bounded paths.
    // The daemon's MAX_RESPONSE_BYTES is compared by contracts/constants.json.
    public static let maximumResponseBytes = 196_608
    public static let maximumObservations = 256

    public static func validBundleID(_ value: String) -> Bool {
        !value.isEmpty && value.utf8.count <= 256
            && value.utf8.allSatisfy {
                (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0)
                    || [UInt8(45), 46, 95].contains($0)
            }
            && value.split(separator: ".", omittingEmptySubsequences: false).allSatisfy { !$0.isEmpty }
    }

    public static func validUUID(_ value: String) -> Bool {
        value.utf8.count == 36 && UUID(uuidString: value)?.uuidString.lowercased() == value.lowercased()
    }

}
