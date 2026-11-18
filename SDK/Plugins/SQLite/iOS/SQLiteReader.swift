//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import Foundation
import SQLite3

/// A SQLite file under the sandbox home. A path that leaves the home cannot make one.
struct SQLiteFile: Equatable {
    /// Relative to the home; the identifier the panel sends back.
    let relativePath: String
    let url: URL

    init?(relativePath: String, home: URL) {
        let root = home.standardizedFileURL.resolvingSymlinksInPath().path
        let url = home.appendingPathComponent(relativePath).standardizedFileURL.resolvingSymlinksInPath()
        guard url.path.hasPrefix(root + "/") else {
            return nil
        }
        self.relativePath = String(url.path.dropFirst(root.count + 1))
        self.url = url
    }
}

/// One result cell. Integers stay exact because a JSON number cannot hold all of them.
enum SQLiteCell: Equatable {
    case null
    case integer(Int64)
    case real(Double)
    case text(String)
    case blob(byteCount: Int)
}

struct SQLiteResult: Equatable {
    let columns: [String]
    let rows: [[SQLiteCell]]
    /// More rows existed than `limit`.
    let truncated: Bool
}

struct SQLiteTable: Equatable {
    enum Kind: String {
        case table
        case view
    }

    let name: String
    let kind: Kind
    let sql: String
    /// Views are not counted; counting one runs its whole definition.
    let rowCount: Int?
}

enum SQLiteReaderError: Error, Equatable, CustomStringConvertible {
    case open(String)
    case prepare(String)
    case step(String)
    /// The connection is read-only anyway; this says why before anything runs.
    case notReadOnly

    var description: String {
        switch self {
        case .open(let message): "Open failed: \(message)"
        case .prepare(let message): "SQL error: \(message)"
        case .step(let message): "Query failed: \(message)"
        case .notReadOnly: "Only read-only statements (SELECT, PRAGMA, EXPLAIN …) are allowed"
        }
    }
}

/// One read-only connection. Opening it alongside the app never blocks the app's writes.
final class SQLiteReader {
    static let textLimit = 10_000

    private let handle: OpaquePointer

    init(file: SQLiteFile) throws {
        var handle: OpaquePointer?
        let result = sqlite3_open_v2(file.url.path, &handle, SQLITE_OPEN_READONLY | SQLITE_OPEN_NOMUTEX, nil)
        guard result == SQLITE_OK, let handle else {
            let message = handle.map { String(cString: sqlite3_errmsg($0)) } ?? String(cString: sqlite3_errstr(result))
            sqlite3_close(handle)
            throw SQLiteReaderError.open(message)
        }
        self.handle = handle
        // ATTACH counts as read-only, and would reach a file outside the home.
        sqlite3_limit(handle, SQLITE_LIMIT_ATTACHED, 0)
        // Waits briefly while the app holds a write lock (rollback-journal databases).
        sqlite3_busy_timeout(handle, 1_000)
    }

    deinit {
        sqlite3_close(handle)
    }

    /// Runs the first statement only.
    func query(_ sql: String, limit: Int) throws -> SQLiteResult {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(handle, sql, -1, &statement, nil) == SQLITE_OK, let statement else {
            throw SQLiteReaderError.prepare(errorMessage)
        }
        defer { sqlite3_finalize(statement) }
        guard sqlite3_stmt_readonly(statement) != 0 else {
            throw SQLiteReaderError.notReadOnly
        }

        let columns = (0 ..< sqlite3_column_count(statement)).map { index in
            sqlite3_column_name(statement, index).map { String(cString: $0) } ?? ""
        }
        var rows: [[SQLiteCell]] = []
        while true {
            let result = sqlite3_step(statement)
            guard result == SQLITE_ROW else {
                guard result == SQLITE_DONE else {
                    throw SQLiteReaderError.step(errorMessage)
                }
                return SQLiteResult(columns: columns, rows: rows, truncated: false)
            }
            guard rows.count < limit else {
                return SQLiteResult(columns: columns, rows: rows, truncated: true)
            }
            rows.append((0 ..< Int32(columns.count)).map { cell(statement, $0) })
        }
    }

    func tables() throws -> [SQLiteTable] {
        let master = try query(
            "SELECT name, type, IFNULL(sql, '') FROM sqlite_master "
                + "WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name",
            limit: .max
        )
        return try master.rows.compactMap { row in
            guard case .text(let name) = row[0], case .text(let type) = row[1],
                  let kind = SQLiteTable.Kind(rawValue: type), case .text(let sql) = row[2] else {
                return nil
            }
            var rowCount: Int?
            if kind == .table,
               case .integer(let count) = try query("SELECT count(*) FROM \(Self.quote(name))", limit: 1).rows.first?.first {
                rowCount = Int(count)
            }
            return SQLiteTable(name: name, kind: kind, sql: sql, rowCount: rowCount)
        }
    }

    static func quote(_ identifier: String) -> String {
        "\"" + identifier.replacingOccurrences(of: "\"", with: "\"\"") + "\""
    }

    /// SQLite files under the home: filtered by extension, then confirmed by header.
    static func databases(home: URL) -> [(file: SQLiteFile, size: Int, modified: Date)] {
        let extensions: Set = ["db", "sqlite", "sqlite3", "store"]
        let keys: [URLResourceKey] = [.isRegularFileKey, .fileSizeKey, .contentModificationDateKey]
        guard let enumerator = FileManager.default.enumerator(at: home, includingPropertiesForKeys: keys) else {
            return []
        }
        // The home is a symlink (/var → /private/var), so match the enumerated paths' prefix.
        let root = home.standardizedFileURL.resolvingSymlinksInPath().path + "/"
        var found: [(file: SQLiteFile, size: Int, modified: Date)] = []
        for case let url as URL in enumerator where extensions.contains(url.pathExtension.lowercased()) {
            let path = url.resolvingSymlinksInPath().path
            guard path.hasPrefix(root), let values = try? url.resourceValues(forKeys: Set(keys)),
                  values.isRegularFile == true, hasSQLiteHeader(url),
                  let file = SQLiteFile(relativePath: String(path.dropFirst(root.count)), home: home) else {
                continue
            }
            found.append((file, values.fileSize ?? 0, values.contentModificationDate ?? .distantPast))
        }
        return found.sorted { $0.file.relativePath < $1.file.relativePath }
    }

    private static func hasSQLiteHeader(_ url: URL) -> Bool {
        guard let handle = try? FileHandle(forReadingFrom: url) else {
            return false
        }
        defer { try? handle.close() }
        return (try? handle.read(upToCount: 16)) == Data("SQLite format 3\0".utf8)
    }

    private var errorMessage: String {
        String(cString: sqlite3_errmsg(handle))
    }

    private func cell(_ statement: OpaquePointer, _ index: Int32) -> SQLiteCell {
        switch sqlite3_column_type(statement, index) {
        case SQLITE_INTEGER:
            return .integer(sqlite3_column_int64(statement, index))
        case SQLITE_FLOAT:
            return .real(sqlite3_column_double(statement, index))
        case SQLITE_TEXT:
            let text = sqlite3_column_text(statement, index).map { String(cString: $0) } ?? ""
            return .text(text.count > Self.textLimit ? String(text.prefix(Self.textLimit)) + "…" : text)
        case SQLITE_BLOB:
            return .blob(byteCount: Int(sqlite3_column_bytes(statement, index)))
        default:
            return .null
        }
    }
}
