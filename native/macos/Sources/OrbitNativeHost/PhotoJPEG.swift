import Foundation
import ImageIO
import OrbitNativeProtocol
import UniformTypeIdentifiers

/// A photo no agent reads (an iPhone's HEIC) written as JPEG where the
/// daemon asks, before the daemon attaches it. ImageIO reads any photo
/// macOS opens. Nothing about the photo is logged; a failure is a fixed code.
package final class PhotoJPEG: @unchecked Sendable {
    /// Decoding and encoding a photo is CPU work and never runs on AppKit's thread.
    private let queue = DispatchQueue(label: "orbit.native.photo-jpeg", qos: .userInitiated)

    /// What JPEG keeps of the photo: 0.9 is indistinguishable from it to a
    /// person and to an agent, at a third of a lossless size.
    static let quality = 0.9

    package init() {}

    package func convert(_ fields: NativePhotoJPEGFields) async throws -> [String: Any] {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                do { continuation.resume(returning: try Self.convertNow(fields)) }
                catch { continuation.resume(throwing: error) }
            }
        }
    }

    static func convertNow(_ fields: NativePhotoJPEGFields) throws -> [String: Any] {
        guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: fields.source_path) as CFURL, nil) else {
            throw NativeFailure("photo_unreadable")
        }
        let image = try upright(source)
        guard let destination = CGImageDestinationCreateWithURL(URL(fileURLWithPath: fields.destination_path) as CFURL,
                                                                UTType.jpeg.identifier as CFString, 1, nil) else {
            throw NativeFailure("photo_jpeg_failed")
        }
        try write(image, to: destination)
        return ["width": image.width, "height": image.height]
    }

    /// The photo's first image at its full size, turned as its EXIF says:
    /// the JPEG then needs no orientation tag, and carries none of the
    /// photo's metadata (its location included).
    static func upright(_ source: CGImageSource) throws -> CGImage {
        guard CGImageSourceGetCount(source) > 0,
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0 else {
            throw NativeFailure("photo_unreadable")
        }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: max(width, height),
            kCGImageSourceShouldCacheImmediately: true,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else {
            throw NativeFailure("photo_unreadable")
        }
        return image
    }

    static func write(_ image: CGImage, to destination: CGImageDestination) throws {
        let options: [CFString: Any] = [kCGImageDestinationLossyCompressionQuality: quality]
        CGImageDestinationAddImage(destination, image, options as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { throw NativeFailure("photo_jpeg_failed") }
    }
}
