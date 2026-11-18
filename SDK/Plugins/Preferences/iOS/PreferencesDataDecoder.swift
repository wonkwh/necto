//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import Foundation
import NectoModel

/// Reads a stored `Data` without the app's types.
///
/// A Codable value saved with `PropertyListEncoder` or `JSONEncoder` is a plain plist or
/// JSON, so its keys and values show even though the type that wrote it is unknown here.
enum PreferencesDataDecoder {
    /// Larger values stay described by size; a list must not decode megabytes per row.
    static let limit = 1024 * 1024

    private static let binaryPropertyListHeader = Data("bplist00".utf8)

    /// `nil` for bytes with no such shape: an image, a key, anything opaque.
    static func decode(_ data: Data) -> NectoJSONValue? {
        guard data.count <= limit else { return nil }
        // Binary plist first: the text plist parser accepts almost any string.
        if data.starts(with: binaryPropertyListHeader),
           let object = try? PropertyListSerialization.propertyList(from: data, format: nil) {
            return json(object)
        }
        if let object = try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed]) {
            return json(object)
        }
        if data.starts(with: Data("<?xml".utf8)),
           let object = try? PropertyListSerialization.propertyList(from: data, format: nil) {
            return json(object)
        }
        return nil
    }

    static func json(_ object: Any) -> NectoJSONValue {
        switch object {
        case let number as NSNumber:
            if CFGetTypeID(number) == CFBooleanGetTypeID() {
                return .bool(number.boolValue)
            }
            // Beyond 2^53 a Double changes the value.
            guard CFNumberIsFloatType(number) || abs(number.doubleValue) < 9_007_199_254_740_992 else {
                return .string(number.stringValue)
            }
            return .number(number.doubleValue)
        case let string as String:
            return .string(string)
        case let date as Date:
            return .string(ISO8601DateFormatter().string(from: date))
        case let data as Data:
            return decode(data) ?? .string("<\(data.count) bytes>")
        case let array as [Any]:
            return .array(array.map(json))
        case let dictionary as [String: Any]:
            return .object(dictionary.mapValues(json))
        default:
            return .string(String(describing: object))
        }
    }
}
