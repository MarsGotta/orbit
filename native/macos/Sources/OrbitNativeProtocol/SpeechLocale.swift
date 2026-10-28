import Foundation

/// The daemon resolves the app's language using the Mac's ordered preferences.
/// One locale owns both the wake phrase and the question that follows it.
public enum NativeSpeechLocale: String, Codable, Sendable {
    case spanish = "es_ES"
    case englishUS = "en_US"
    case englishGB = "en_GB"

    public var languageCode: String { self == .spanish ? "es" : "en" }
    var greetings: Set<String> { self == .spanish ? ["oye", "hola"] : ["hey"] }
}

public struct NativeInterfaceLanguageFields: Decodable, Sendable {
    public let locale: NativeSpeechLocale
}
