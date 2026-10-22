import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers
import XCTest
@testable import OrbitNativeHost
@testable import OrbitNativeProtocol

/// A HEIC photo written as JPEG with ImageIO, as the helper does before the
/// daemon attaches it. Every photo here is made in the test; none is real.
final class PhotoJPEGTests: XCTestCase {
    private var folder: URL!

    override func setUpWithError() throws {
        folder = FileManager.default.temporaryDirectory
            .appendingPathComponent("orbit-photo-jpeg-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        if let folder { try? FileManager.default.removeItem(at: folder) }
    }

    /// An 80×60 HEIC made in memory; `orientation` is its EXIF orientation.
    private func heic(orientation: Int = 1) throws -> Data {
        let space = try XCTUnwrap(CGColorSpace(name: CGColorSpace.sRGB))
        let context = try XCTUnwrap(CGContext(data: nil, width: 80, height: 60, bitsPerComponent: 8,
                                              bytesPerRow: 0, space: space,
                                              bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.setFillColor(red: 0.2, green: 0.5, blue: 0.8, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: 80, height: 60))
        let image = try XCTUnwrap(context.makeImage())
        let data = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(
            data as CFMutableData, UTType.heic.identifier as CFString, 1, nil), "this Mac cannot write HEIC")
        let properties: [CFString: Any] = [kCGImagePropertyOrientation: orientation]
        CGImageDestinationAddImage(destination, image, properties as CFDictionary)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        return data as Data
    }

    private func size(ofJPEG data: Data) throws -> (width: Int, height: Int) {
        XCTAssertEqual(Array(data.prefix(3)), [0xFF, 0xD8, 0xFF])
        let source = try XCTUnwrap(CGImageSourceCreateWithData(data as CFData, nil))
        XCTAssertEqual(CGImageSourceGetType(source) as String?, UTType.jpeg.identifier)
        let properties = try XCTUnwrap(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
        return (try XCTUnwrap(properties[kCGImagePropertyPixelWidth] as? Int),
                try XCTUnwrap(properties[kCGImagePropertyPixelHeight] as? Int))
    }

    func testAHEICPhotoBecomesAJPEGOfTheSameSize() throws {
        let source = try XCTUnwrap(CGImageSourceCreateWithData(try heic() as CFData, nil))
        XCTAssertEqual(CGImageSourceGetType(source) as String?, UTType.heic.identifier)
        let image = try PhotoJPEG.upright(source)
        let jpeg = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(
            jpeg as CFMutableData, UTType.jpeg.identifier as CFString, 1, nil))
        try PhotoJPEG.write(image, to: destination)
        let written = try size(ofJPEG: jpeg as Data)
        XCTAssertEqual(written.width, 80)
        XCTAssertEqual(written.height, 60)
    }

    /// A photo taken sideways is turned as its EXIF says, so the JPEG needs
    /// no orientation tag to be seen upright.
    func testASidewaysPhotoIsWrittenUpright() throws {
        let source = try XCTUnwrap(CGImageSourceCreateWithData(try heic(orientation: 6) as CFData, nil))
        let image = try PhotoJPEG.upright(source)
        XCTAssertEqual(image.width, 60)
        XCTAssertEqual(image.height, 80)
    }

    func testThePhotoOnDiskIsWrittenWhereTheDaemonAsks() throws {
        let photo = folder.appendingPathComponent("IMG_0001.HEIC")
        try heic().write(to: photo)
        let jpeg = folder.appendingPathComponent("IMG_0001.jpg")
        let fields = try JSONDecoder().decode(NativePhotoJPEGFields.self, from: JSONSerialization.data(
            withJSONObject: ["source_path": photo.path, "destination_path": jpeg.path]))
        let result = try PhotoJPEG.convertNow(fields)
        XCTAssertEqual(result["width"] as? Int, 80)
        XCTAssertEqual(result["height"] as? Int, 60)
        XCTAssertEqual(try size(ofJPEG: Data(contentsOf: jpeg)).width, 80)
    }

    func testWhatIsNotAPhotoIsRefusedByItsCause() throws {
        let notes = folder.appendingPathComponent("notas.heic")
        try Data("esto no es una foto".utf8).write(to: notes)
        let fields = try JSONDecoder().decode(NativePhotoJPEGFields.self, from: JSONSerialization.data(
            withJSONObject: ["source_path": notes.path,
                             "destination_path": folder.appendingPathComponent("notas.jpg").path]))
        XCTAssertThrowsError(try PhotoJPEG.convertNow(fields)) {
            XCTAssertEqual(($0 as? NativeFailure)?.code, "photo_unreadable")
        }
    }
}
