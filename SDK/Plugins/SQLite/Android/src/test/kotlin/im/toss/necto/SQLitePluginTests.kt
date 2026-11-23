// Copyright (c) 2026 Viva Republica, Inc.

package im.toss.necto

import android.database.sqlite.SQLiteDatabase
import im.toss.necto.model.NectoBridgeError
import im.toss.necto.model.NectoBridgeErrorCode
import im.toss.necto.model.jsonObject
import im.toss.necto.model.jsonOf
import im.toss.necto.plugins.NectoSQLitePlugin
import im.toss.necto.testing.PluginHarness
import kotlinx.coroutines.runBlocking
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.io.File
import java.nio.file.Files
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertTrue

@RunWith(RobolectricTestRunner::class)
class SQLitePluginTests {
    private val home: File = Files.createTempDirectory("necto").toFile()
    private val sqlite = PluginHarness(NectoSQLitePlugin(home))

    private fun database(name: String, vararg statements: String) {
        val file = File(home, name).apply { parentFile!!.mkdirs() }
        SQLiteDatabase.openOrCreateDatabase(file, null).use { db -> statements.forEach(db::execSQL) }
    }

    private suspend fun query(sql: String, limit: Int? = null) = sqlite.call(
        "sqlite.query",
        jsonObject(
            buildMap {
                put("database", jsonOf("databases/app"))
                put("sql", jsonOf(sql))
                limit?.let { put("limit", jsonOf(it)) }
            },
        ),
    )

    @Test
    fun listsDatabasesTablesAndRows() = runBlocking {
        database(
            "databases/app",
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, photo BLOB, big INTEGER)",
            "INSERT INTO users VALUES (1, 'Kim', x'010203', 9007199254740993)",
            "INSERT INTO users VALUES (2, NULL, NULL, 2)",
            "CREATE VIEW names AS SELECT name FROM users",
        )
        File(home, "files").mkdirs()
        File(home, "files/notes.txt").writeText("not a database")

        val databases = sqlite.call("sqlite.databases")["databases"]!!.arrayValue!!
        assertEquals(listOf("databases/app"), databases.map { it["path"]!!.stringValue })

        val tables = sqlite.call("sqlite.tables", jsonObject("database" to jsonOf("databases/app")))["tables"]!!.arrayValue!!
        // Android adds android_metadata to every database it creates; it is shown like any table.
        assertEquals(listOf("android_metadata", "names", "users"), tables.map { it["name"]!!.stringValue })
        assertEquals(null, tables[1]["rowCount"])
        assertEquals(jsonOf(2), tables[2]["rowCount"])

        val result = query("-- comment\n SELECT * FROM users ORDER BY id")
        assertEquals(listOf("id", "name", "photo", "big"), result["columns"]!!.arrayValue!!.map { it.stringValue })
        val first = result["rows"]!!.arrayValue!![0].arrayValue!!
        assertEquals(listOf(jsonOf(1), jsonOf("Kim"), jsonOf("<blob 3 B>"), jsonOf("9007199254740993")), first)
        assertEquals(jsonOf(false), result["truncated"])

        assertEquals(jsonOf(true), query("SELECT * FROM users", limit = 1)["truncated"])
    }

    @Test
    fun refusesWritesAttachAndPathsOutsideTheHome() = runBlocking {
        database("databases/app", "CREATE TABLE t (x)")
        for (sql in listOf("DELETE FROM t", "ATTACH '/etc/x' AS x", "/* SELECT */ DROP TABLE t")) {
            val error = assertFailsWith<NectoBridgeError> { query(sql) }
            assertEquals(NectoBridgeErrorCode.PERMISSION_DENIED, error.code)
        }
        // The keyword check is not the only guard: the connection itself cannot write.
        assertFailsWith<NectoBridgeError> { query("WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x") }
        assertTrue(query("SELECT count(*) FROM t")["rows"]!!.arrayValue!![0].arrayValue!![0] == jsonOf(0))

        val error = assertFailsWith<NectoBridgeError> {
            sqlite.call("sqlite.tables", jsonObject("database" to jsonOf("../outside")))
        }
        assertEquals(NectoBridgeErrorCode.INVALID_INPUT, error.code)
    }

    @Test
    fun readOnlyKeywordSkipsLeadingComments() {
        assertTrue(NectoSQLitePlugin.isReadOnly("  /* a */ -- b\n pragma table_info(t)"))
        assertTrue(NectoSQLitePlugin.isReadOnly("explain query plan select 1"))
        assertFalse(NectoSQLitePlugin.isReadOnly("UPDATE t SET x = 1"))
        assertFalse(NectoSQLitePlugin.isReadOnly("DETACH x"))
    }
}
