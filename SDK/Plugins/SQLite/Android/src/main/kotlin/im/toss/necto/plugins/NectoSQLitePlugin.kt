// Copyright (c) 2026 Viva Republica, Inc.

package im.toss.necto.plugins

import android.content.Context
import android.database.Cursor
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteException
import im.toss.necto.model.NectoBridgeError
import im.toss.necto.model.NectoBridgeErrorCode
import im.toss.necto.model.NectoJsonValue
import im.toss.necto.model.jsonArray
import im.toss.necto.model.jsonObject
import im.toss.necto.model.jsonOf
import im.toss.necto.sdk.NectoPlugin
import im.toss.necto.sdk.NectoPluginPanel
import im.toss.necto.sdk.NectoRegistrar
import java.io.File
import java.nio.file.Files
import java.nio.file.LinkOption
import kotlin.math.abs

/**
 * The SQLite databases under the app's data directory, read-only: tables, schema and
 * SQL queries. Like Files, it needs nothing reported: register it and it answers.
 *
 * ```kotlin
 * NectoAndroid.start(context, listOf(NectoSQLitePlugin(context)))
 * ```
 */
public class NectoSQLitePlugin(private val home: File) : NectoPlugin {
    public constructor(context: Context) : this(context.dataDir)

    override val id: String = "sqlite"

    override val panel: NectoPluginPanel = NectoPanels.resource("sqlite")

    override fun register(necto: NectoRegistrar) {
        necto.handle("sqlite.databases") {
            val root = home.canonicalFile
            jsonObject("databases" to jsonArray(databases(root).map { file ->
                jsonObject(
                    "path" to jsonOf(file.relativeTo(root).invariantSeparatorsPath),
                    "name" to jsonOf(file.name),
                    "size" to jsonOf(file.length()),
                    "modifiedAt" to jsonOf(file.lastModified()),
                )
            }))
        }

        necto.handle("sqlite.tables") { input ->
            open(input).use { database ->
                val master = query(
                    database,
                    "SELECT name, type, IFNULL(sql, '') FROM sqlite_master " +
                        "WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name",
                    Int.MAX_VALUE,
                )
                jsonObject("tables" to jsonArray(master.rows.map { row ->
                    val name = row[0].stringValue.orEmpty()
                    val kind = row[1].stringValue.orEmpty()
                    val fields = linkedMapOf("name" to jsonOf(name), "kind" to jsonOf(kind), "sql" to row[2])
                    // Views are not counted; counting one runs its whole definition.
                    if (kind == "table") fields["rowCount"] = query(database, "SELECT count(*) FROM ${quote(name)}", 1).rows[0][0]
                    jsonObject(fields)
                }))
            }
        }

        necto.handle("sqlite.query") { input ->
            val sql = input["sql"]?.stringValue?.takeIf { it.isNotBlank() } ?: throw invalid("sql is required")
            if (!isReadOnly(sql)) throw NectoBridgeError(NectoBridgeErrorCode.PERMISSION_DENIED, NOT_READ_ONLY)
            val wanted = input["limit"]?.numberValue ?: DEFAULT_LIMIT.toDouble()
            val limit = if (wanted.isFinite()) wanted.coerceIn(1.0, MAXIMUM_LIMIT.toDouble()).toInt() else DEFAULT_LIMIT
            open(input).use { database ->
                val started = System.nanoTime()
                val result = query(database, sql, limit)
                jsonObject(
                    "columns" to jsonArray(result.columns.map { jsonOf(it) }),
                    "rows" to jsonArray(result.rows.map { jsonArray(it) }),
                    "truncated" to jsonOf(result.truncated),
                    "elapsedMs" to jsonOf((System.nanoTime() - started) / 1_000_000),
                )
            }
        }
    }

    private class Result(val columns: List<String>, val rows: List<List<NectoJsonValue>>, val truncated: Boolean)

    private fun open(input: NectoJsonValue): SQLiteDatabase {
        val path = input["database"]?.stringValue ?: throw invalid("database is required")
        val root = home.canonicalFile
        val file = File(root, path).canonicalFile
        if (!file.path.startsWith(root.path + File.separator)) throw invalid("database must be a path inside the app data directory")
        return sqlite { SQLiteDatabase.openDatabase(file.path, null, SQLiteDatabase.OPEN_READONLY) }
    }

    /** Runs the first statement only, on a connection that cannot write. */
    private fun query(database: SQLiteDatabase, sql: String, limit: Int): Result = sqlite {
        database.rawQuery(sql, null).use { cursor ->
            val columns = cursor.columnNames.toList()
            val rows = ArrayList<List<NectoJsonValue>>()
            while (cursor.moveToNext()) {
                if (rows.size == limit) return@sqlite Result(columns, rows, truncated = true)
                rows.add(columns.indices.map { cell(cursor, it) })
            }
            Result(columns, rows, truncated = false)
        }
    }

    /** Turns a SQLite error into a sentence the panel shows as it is. */
    private fun <T> sqlite(body: () -> T): T = try {
        body()
    } catch (error: SQLiteException) {
        throw NectoBridgeError(NectoBridgeErrorCode.PROVIDER_FAILED, error.message ?: "SQLite failed")
    }

    internal companion object {
        const val DEFAULT_LIMIT = 200
        const val MAXIMUM_LIMIT = 5_000
        const val TEXT_LIMIT = 10_000
        const val NOT_READ_ONLY = "Only read-only statements (SELECT, PRAGMA, EXPLAIN …) are allowed"
        private const val EXACT_INTEGER = 1L shl 53
        private val readOnlyKeywords = setOf("SELECT", "WITH", "PRAGMA", "EXPLAIN", "VALUES")
        private val leadingNoise = Regex("""^(\s+|--[^\n]*\n?|/\*.*?\*/)+""", RegexOption.DOT_MATCHES_ALL)
        private val header = "SQLite format 3\u0000".toByteArray(Charsets.US_ASCII)
        private val sidecars = listOf("-wal", "-shm", "-journal")

        /**
         * Android has no `sqlite3_stmt_readonly`, so the first keyword decides; the read-only
         * connection refuses any write that gets past it. Leaving out ATTACH keeps files
         * outside the data directory unreachable.
         */
        fun isReadOnly(sql: String): Boolean {
            val keyword = sql.replaceFirst(leadingNoise, "").takeWhile { it.isLetter() }.uppercase()
            return keyword in readOnlyKeywords
        }

        /** Regular files under the root whose header says SQLite; Android names them without an extension. */
        fun databases(root: File): List<File> = root.walkTopDown()
            .onEnter { !Files.isSymbolicLink(it.toPath()) }
            .filter { file ->
                Files.isRegularFile(file.toPath(), LinkOption.NOFOLLOW_LINKS) &&
                    sidecars.none { file.name.endsWith(it) } && hasHeader(file)
            }
            .sortedBy { it.relativeTo(root).invariantSeparatorsPath }
            .toList()

        fun quote(identifier: String): String = "\"" + identifier.replace("\"", "\"\"") + "\""

        private fun hasHeader(file: File): Boolean = runCatching {
            // readNBytes needs API 33.
            file.inputStream().use { stream -> ByteArray(header.size).also { stream.read(it) }.contentEquals(header) }
        }.getOrDefault(false)

        private fun cell(cursor: Cursor, index: Int): NectoJsonValue = when (cursor.getType(index)) {
            Cursor.FIELD_TYPE_INTEGER -> cursor.getLong(index).let {
                // Beyond 2^53 a Double changes the value.
                if (abs(it) <= EXACT_INTEGER) jsonOf(it) else jsonOf(it.toString())
            }
            Cursor.FIELD_TYPE_FLOAT -> cursor.getDouble(index).let { if (it.isFinite()) jsonOf(it) else jsonOf(it.toString()) }
            Cursor.FIELD_TYPE_STRING -> cursor.getString(index).let {
                jsonOf(if (it.length > TEXT_LIMIT) it.take(TEXT_LIMIT) + "…" else it)
            }
            Cursor.FIELD_TYPE_BLOB -> jsonOf("<blob ${cursor.getBlob(index).size} B>")
            else -> NectoJsonValue.Null
        }

        private fun invalid(message: String) = NectoBridgeError(NectoBridgeErrorCode.INVALID_INPUT, message)
    }
}
