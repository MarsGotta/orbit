import Foundation
import XCTest
@testable import OrbitNativeHost
import OrbitNativeProtocol

final class HelperCopyTests: XCTestCase {
    func testFolderPanelCopyFollowsTheExplicitAppLanguage() throws {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        let temporary = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: temporary) }
        let bundleURL = temporary.appendingPathComponent("Fixture.bundle")
        let resources = bundleURL.appendingPathComponent("Contents/Resources")
        try FileManager.default.createDirectory(at: resources, withIntermediateDirectories: true)
        let info = try PropertyListSerialization.data(fromPropertyList: [
            "CFBundleIdentifier": "test.example.orbit-localization",
            "CFBundleDevelopmentRegion": "en",
            "CFBundleLocalizations": ["en", "es"]
        ], format: .xml, options: 0)
        try info.write(to: bundleURL.appendingPathComponent("Contents/Info.plist"))
        for language in ["en", "es"] {
            try FileManager.default.copyItem(at: root.appendingPathComponent("Resources/\(language).lproj"),
                                             to: resources.appendingPathComponent("\(language).lproj"))
        }
        let bundle = try XCTUnwrap(Bundle(url: bundleURL))
        XCTAssertEqual(HelperCopy.text(.folderAdd, locale: .englishUS, bundle: bundle), "Add")
        XCTAssertEqual(HelperCopy.text(.folderAdd, locale: .englishGB, bundle: bundle), "Add")
        XCTAssertEqual(HelperCopy.text(.folderAdd, locale: .spanish, bundle: bundle), "Añadir")
        XCTAssertTrue(HelperCopy.text(.folderReadOnly, locale: .englishUS, bundle: bundle).hasPrefix("Choose a folder"))
        XCTAssertTrue(HelperCopy.text(.folderReadOnly, locale: .spanish, bundle: bundle).hasPrefix("Elige una carpeta"))
    }
}
