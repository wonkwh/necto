//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import Foundation
import SQLite3
import Testing

@testable import NectoSQLitePlugin

@Suite("SQLite reader")
final class SQLiteReaderTests {
    private let home = FileManager.default.temporaryDirectory.appending(path: "necto-sqlite-\(UUID().uuidString)")

    init() throws {
        try FileManager.default.createDirectory(at: home.appending(path: "db"), withIntermediateDirectories: true)
        try execute(
            at: home.appending(path: "db/history.db"),
            """
            CREATE TABLE HISTORY (ID INTEGER PRIMARY KEY, NAME TEXT, SPEED REAL, RAW BLOB, BIG INTEGER);
            INSERT INTO HISTORY VALUES (1, 'a', 1.5, x'0102', 9007199254740993);
            INSERT INTO HISTORY VALUES (2, NULL, NULL, NULL, NULL);
            INSERT INTO HISTORY VALUES (3, 'c', 3.0, NULL, 3);
            CREATE VIEW NAMED AS SELECT NAME FROM HISTORY WHERE NAME IS NOT NULL;
            """
        )
        try Data("not sqlite".utf8).write(to: home.appending(path: "db/fake.db"))
    }

    deinit {
        try? FileManager.default.removeItem(at: home)
    }

    @Test("only files with a SQLite header are listed")
    func findsOnlyRealSQLiteFiles() {
        #expect(SQLiteReader.databases(home: home).map(\.file.relativePath) == ["db/history.db"])
    }

    @Test("a path outside the home is refused")
    func rejectsPathsOutsideHome() {
        #expect(SQLiteFile(relativePath: "../etc/passwd", home: home) == nil)
        #expect(SQLiteFile(relativePath: "db/../../x.db", home: home) == nil)
        #expect(SQLiteFile(relativePath: "db/./history.db", home: home)?.relativePath == "db/history.db")
    }

    @Test("cells keep their type and truncation is reported")
    func queriesCellsAndMarksTruncation() throws {
        let reader = try SQLiteReader(file: try file())

        let result = try reader.query("SELECT ID, NAME, SPEED, RAW, BIG FROM HISTORY ORDER BY ID", limit: 2)

        #expect(result.columns == ["ID", "NAME", "SPEED", "RAW", "BIG"])
        #expect(result.rows == [
            [.integer(1), .text("a"), .real(1.5), .blob(byteCount: 2), .integer(9_007_199_254_740_993)],
            [.integer(2), .null, .null, .null, .null],
        ])
        #expect(result.truncated)
        #expect(try !reader.query("SELECT ID FROM HISTORY", limit: 3).truncated)
    }

    @Test("writes and ATTACH are refused and the data is left alone")
    func refusesWritesAndLeavesDataAlone() throws {
        let reader = try SQLiteReader(file: try file())

        #expect(throws: SQLiteReaderError.notReadOnly) { try reader.query("DELETE FROM HISTORY", limit: 1) }
        #expect(throws: SQLiteReaderError.step("too many attached databases - max 0")) {
            try reader.query("ATTACH ':memory:' AS other", limit: 1)
        }
        #expect(try reader.query("SELECT count(*) FROM HISTORY", limit: 1).rows == [[.integer(3)]])
    }

    @Test("tables carry row counts and views do not")
    func listsTablesWithRowCountsAndViewsWithout() throws {
        let tables = try SQLiteReader(file: try file()).tables()

        #expect(tables.map(\.name) == ["HISTORY", "NAMED"])
        #expect(tables.map(\.rowCount) == [3, nil])
        #expect(tables.map(\.kind) == [.table, .view])
    }

    @Test("integers beyond 2^53 travel as strings")
    func keepsLargeIntegersExactInJSON() {
        #expect(NectoSQLitePlugin.json(.integer(9_007_199_254_740_993)) == .string("9007199254740993"))
        #expect(NectoSQLitePlugin.json(.integer(Int64.min)) == .string(String(Int64.min)))
        #expect(NectoSQLitePlugin.json(.integer(42)) == .number(42))
    }

    private func file() throws -> SQLiteFile {
        try #require(SQLiteFile(relativePath: "db/history.db", home: home))
    }

    private func execute(at url: URL, _ sql: String) throws {
        var handle: OpaquePointer?
        try #require(sqlite3_open(url.path, &handle) == SQLITE_OK)
        defer { sqlite3_close(handle) }
        try #require(sqlite3_exec(handle, sql, nil, nil, nil) == SQLITE_OK)
    }
}
