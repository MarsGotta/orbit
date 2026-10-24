import Foundation
import CoreFoundation

/// A JSON value the helper sends: what a `[String: Any]` result or detail becomes on the wire.
public enum NativeJSON: Encodable {
    case string(String), integer(Int64), unsigned(UInt64), decimal(Double), bool(Bool), array([NativeJSON])
    case object([String: NativeJSON]), null

    public static func fromAny(_ value: Any, event: String, field: String) throws -> NativeJSON {
        if value is NSNull { return .null }
        if let number = value as? NSNumber {
            if CFGetTypeID(number) == CFBooleanGetTypeID() { return .bool(number.boolValue) }
            if let integer = Int64(number.stringValue) { return .integer(integer) }
            if let unsigned = UInt64(number.stringValue) { return .unsigned(unsigned) }
            guard number.doubleValue.isFinite else {
                throw NativeEventFieldError(event: event, field: field, expected: "finite number")
            }
            return .decimal(number.doubleValue)
        }
        if let string = value as? String { return .string(string) }
        if let values = value as? [Any] {
            return .array(try values.enumerated().map {
                try fromAny($0.element, event: event, field: "\(field)[\($0.offset)]")
            })
        }
        if let values = value as? [String: Any] {
            var result = [String: NativeJSON]()
            for (key, item) in values {
                result[key] = try fromAny(item, event: event, field: field.isEmpty ? key : "\(field).\(key)")
            }
            return .object(result)
        }
        throw NativeEventFieldError(event: event, field: field, expected: "JSON value")
    }

    public func encode(to encoder: Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .string(let item): try value.encode(item)
        case .integer(let item): try value.encode(item)
        case .unsigned(let item): try value.encode(item)
        case .decimal(let item):
            guard item.isFinite else {
                throw NativeEventFieldError(event: "response",
                    field: encoder.codingPath.map(\.stringValue).joined(separator: "."),
                    expected: "finite Double")
            }
            try value.encode(item)
        case .bool(let item): try value.encode(item)
        case .array(let item): try value.encode(item)
        case .object(let item): try value.encode(item)
        case .null: try value.encodeNil()
        }
    }
}

public struct NativeEventFieldError: Error, CustomStringConvertible {
    public let event: String
    public let field: String
    public let expected: String
    public var description: String {
        "native_event_\(event)_\(field)_expected_\(expected.replacingOccurrences(of: " ", with: "_"))"
    }
}
