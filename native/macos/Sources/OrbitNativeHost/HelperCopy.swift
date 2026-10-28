import Foundation
import OrbitNativeProtocol

/// Helper-owned panels follow the app language, including an explicit override.
/// macOS selects permission purpose strings from the bundle's InfoPlist.strings.
package enum HelperCopy {
    package enum Key: String {
        case folderAdd = "folder_add"
        case folderReadOnly = "folder_read_only"
    }

    package static func text(_ key: Key, locale: NativeSpeechLocale, bundle: Bundle = .main) -> String {
        guard let path = bundle.path(forResource: locale.languageCode, ofType: "lproj"),
              let catalogue = Bundle(path: path) else { return key.rawValue }
        return catalogue.localizedString(forKey: key.rawValue, value: nil, table: "Localizable")
    }
}
