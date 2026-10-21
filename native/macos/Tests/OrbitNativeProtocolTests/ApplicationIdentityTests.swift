import Foundation
import XCTest
@testable import OrbitNativeProtocol

final class ApplicationIdentityTests: XCTestCase {
    private let native = "00000000-0000-0000-0000-000000000001"
    private let instance = "00000000-0000-0000-0000-000000000002"
    private let token = "00000000-0000-0000-0000-000000000003"

    private func binding(pid: Int32 = 321, generation: UInt64 = 7,
                         observed: UInt64 = 1, expires: UInt64 = 15_000_000_001) -> ApplicationIdentityBinding {
        ApplicationIdentityBinding(nativeInstanceID: native, privacyGeneration: generation,
            applicationPID: pid, applicationInstanceID: instance, identityToken: token,
            identity: StableApplicationIdentity(bundleID: "com.google.Chrome",
                bundlePath: "/Applications/Google Chrome.app", requirementSHA256: String(repeating: "a", count: 64)),
            observedNanoseconds: observed, expiresNanoseconds: expires)
    }

    func testReceiptLimitsAndUnsignedIdentityAreNotInvented() {
        XCTAssertTrue(binding().isWellFormed)
        XCTAssertFalse(binding(pid: 0).isWellFormed)
        XCTAssertFalse(binding(observed: 0).isWellFormed)
        XCTAssertFalse(binding(expires: 15_000_000_002).isWellFormed)
        XCTAssertFalse(binding(observed: UInt64.max, expires: UInt64.max).isWellFormed)
        XCTAssertFalse(StableApplicationIdentity(bundleID: "com.google.Chrome",
            bundlePath: "/Applications/Google Chrome.app", requirementSHA256: "").isWellFormed)
        XCTAssertFalse(ApplicationIdentityPolicy.validBundleID("com..Chrome"))
        XCTAssertFalse(ApplicationIdentityPolicy.validBundleID("com.Chrome\n"))
    }

    func testIdentityPathsAndHashAreExactNotNormalized() {
        for path in ["Applications/Chrome.app", "/Applications//Chrome.app", "/Applications/../Chrome.app", "/"] {
            XCTAssertFalse(StableApplicationIdentity(bundleID: "com.google.Chrome", bundlePath: path,
                requirementSHA256: String(repeating: "a", count: 64)).isWellFormed)
        }
        XCTAssertFalse(StableApplicationIdentity(bundleID: "com.google.Chrome", bundlePath: "/Applications/Chrome.app",
            requirementSHA256: String(repeating: "A", count: 64)).isWellFormed)
    }

    func testProtocolRoundTripDoesNotChangeIdentityOrInventAuthority() throws {
        let original = binding()
        let decoded = try JSONDecoder().decode(ApplicationIdentityBinding.self, from: JSONEncoder().encode(original))
        XCTAssertEqual(original, decoded)
        var object = original.fields
        object["clock_domain"] = "utc"
        let wrongClock = try JSONDecoder().decode(ApplicationIdentityBinding.self,
            from: JSONSerialization.data(withJSONObject: object))
        XCTAssertFalse(wrongClock.isWellFormed)
        object = original.fields
        object["identity_token"] = instance
        let differentReceipt = try JSONDecoder().decode(ApplicationIdentityBinding.self,
            from: JSONSerialization.data(withJSONObject: object))
        XCTAssertNotEqual(original, differentReceipt)
    }
}
