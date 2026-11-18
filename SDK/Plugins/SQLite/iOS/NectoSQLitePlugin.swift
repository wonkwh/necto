//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import Foundation
import NectoModel
import NectoSDK

/// The SQLite databases in the app's sandbox, read-only: tables, schema and SQL queries.
///
/// Like Files, it needs nothing reported: register it and it answers.
///
/// ```swift
/// NectoSDK.register(NectoSQLitePlugin())
/// ```
public final class NectoSQLitePlugin: NectoPlugin {
    static let defaultLimit = 200
    static let maximumLimit = 5_000

    public let id = "sqlite"

    public var panel: NectoPluginPanel? { NectoPluginPanel(bundle: .module) }

    private let home: URL

    public convenience init() {
        self.init(home: URL(fileURLWithPath: NSHomeDirectory()))
    }

    init(home: URL) {
        self.home = home
    }

    public func register(_ necto: NectoRegistrar) {
        necto.handle("sqlite.databases") { [home] _ in
            let databases = SQLiteReader.databases(home: home).map { entry -> NectoJSONValue in
                .object([
                    "path": .string(entry.file.relativePath),
                    "name": .string(entry.file.url.lastPathComponent),
                    "size": .number(Double(entry.size)),
                    "modifiedAt": .number(entry.modified.timeIntervalSince1970 * 1_000),
                ])
            }
            return ["databases": .array(databases)]
        }

        necto.handle("sqlite.tables") { [home] input in
            let reader = try Self.open(input, home: home)
            let tables = try Self.run { try reader.tables() }.map { table -> NectoJSONValue in
                var fields: [String: NectoJSONValue] = [
                    "name": .string(table.name),
                    "kind": .string(table.kind.rawValue),
                    "sql": .string(table.sql),
                ]
                fields["rowCount"] = table.rowCount.map { .number(Double($0)) }
                return .object(fields)
            }
            return ["tables": .array(tables)]
        }

        necto.handle("sqlite.query") { [home] input in
            guard let sql = input["sql"]?.stringValue, !sql.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw NectoBridgeError(code: .invalidInput, message: "sql is required")
            }
            let wanted = input["limit"]?.numberValue ?? Double(Self.defaultLimit)
            let limit = wanted.isFinite ? Int(max(1, min(wanted, Double(Self.maximumLimit)))) : Self.defaultLimit
            let reader = try Self.open(input, home: home)
            let started = Date()
            let result = try Self.run { try reader.query(sql, limit: limit) }
            return [
                "columns": .array(result.columns.map(NectoJSONValue.string)),
                "rows": .array(result.rows.map { .array($0.map(Self.json)) }),
                "truncated": .bool(result.truncated),
                "elapsedMs": .number((Date().timeIntervalSince(started) * 1_000).rounded()),
            ]
        }
    }

    private static func open(_ input: NectoJSONValue, home: URL) throws -> SQLiteReader {
        guard let path = input["database"]?.stringValue, let file = SQLiteFile(relativePath: path, home: home) else {
            throw NectoBridgeError(code: .invalidInput, message: "database must be a path inside the app sandbox")
        }
        return try run { try SQLiteReader(file: file) }
    }

    /// Turns a SQLite error into a sentence the panel shows as it is.
    private static func run<T>(_ body: () throws -> T) throws -> T {
        do {
            return try body()
        } catch let error as SQLiteReaderError {
            throw NectoBridgeError(code: error == .notReadOnly ? .permissionDenied : .providerFailed, message: error.description)
        }
    }

    static func json(_ cell: SQLiteCell) -> NectoJSONValue {
        switch cell {
        case .null:
            return .null
        case .integer(let value):
            // Beyond 2^53 a Double changes the value.
            return value.magnitude <= 1 << 53 ? .number(Double(value)) : .string(String(value))
        case .real(let value):
            return value.isFinite ? .number(value) : .string(String(value))
        case .text(let value):
            return .string(value)
        case .blob(let byteCount):
            return .string("<blob \(byteCount) B>")
        }
    }
}
