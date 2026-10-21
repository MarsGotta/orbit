import Foundation
import Security
import CryptoKit
import Darwin
import OrbitNativeProtocol

/// Only immutable values cross the queue. Security/CF objects stay on this
/// serial worker; no code is loaded, launched, registered, or granted access.
struct ApplicationProcessMetadata: Equatable, Sendable {
    let pid: Int32
    let launchDateBits: UInt64
    let bundleID: String
    let bundlePath: String
}

enum ApplicationCodeIdentity {
    private static let worker = DispatchQueue(label: "orbit.native.application-identity", qos: .utility)

    static func read(_ metadata: ApplicationProcessMetadata) async -> StableApplicationIdentity? {
        await withCheckedContinuation { continuation in
            worker.async {
                continuation.resume(returning: observe(metadata))
            }
        }
    }

    private static func observe(_ metadata: ApplicationProcessMetadata) -> StableApplicationIdentity? {
        dispatchPrecondition(condition: .onQueue(worker))
        guard metadata.pid > 0, let requestedPath = canonicalPath(metadata.bundlePath) else { return nil }
        let attributes = [kSecGuestAttributePid as String: NSNumber(value: metadata.pid)] as CFDictionary
        var running: SecCode?
        guard SecCodeCopyGuestWithAttributes(nil, attributes, SecCSFlags(rawValue: 0), &running) == errSecSuccess,
              let running,
              SecCodeCheckValidity(running, .noNetworkAccess, nil) == errSecSuccess else { return nil }
        var code: SecStaticCode?
        guard SecCodeCopyStaticCode(running, SecCSFlags(rawValue: 0), &code) == errSecSuccess,
              let code else { return nil }
        var codeURL: CFURL?
        guard SecCodeCopyPath(code, SecCSFlags(rawValue: 0), &codeURL) == errSecSuccess,
              let codeURL, let codePath = canonicalPath((codeURL as URL).path), codePath == requestedPath else { return nil }
        var information: CFDictionary?
        guard SecCodeCopySigningInformation(code, SecCSFlags(rawValue: kSecCSSigningInformation), &information) == errSecSuccess,
              let information = information as? [String: Any],
              information[kSecCodeInfoIdentifier as String] as? String == metadata.bundleID else { return nil }
        var requirement: SecRequirement?
        guard SecCodeCopyDesignatedRequirement(code, SecCSFlags(rawValue: 0), &requirement) == errSecSuccess,
              let requirement else { return nil }
        var serialized: CFData?
        guard SecRequirementCopyData(requirement, SecCSFlags(rawValue: 0), &serialized) == errSecSuccess,
              let serialized else { return nil }
        let bytes = serialized as Data
        guard !bytes.isEmpty, bytes.count <= 16_384,
              // Apple does not promise a secure SecCode -> filesystem link.
              // Check the extracted requirement against the running code again.
              SecCodeCheckValidity(running, .noNetworkAccess, requirement) == errSecSuccess else { return nil }
        let identity = StableApplicationIdentity(bundleID: metadata.bundleID, bundlePath: requestedPath,
            requirementSHA256: SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined())
        return identity.isWellFormed ? identity : nil
    }

    /// POSIX canonical bytes, not Foundation's /private path presentation rules.
    private static func canonicalPath(_ path: String) -> String? {
        guard path.hasPrefix("/"), path.utf8.count <= 4096,
              !path.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else { return nil }
        var storage = [CChar](repeating: 0, count: Int(PATH_MAX))
        return path.withCString { original in
            storage.withUnsafeMutableBufferPointer { output -> String? in
                guard let pointer = output.baseAddress, Darwin.realpath(original, pointer) != nil else { return nil }
                return String(validatingUTF8: pointer)
            }
        }
    }
}
