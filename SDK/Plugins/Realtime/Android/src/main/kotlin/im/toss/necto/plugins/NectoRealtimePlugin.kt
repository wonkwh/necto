// Copyright (c) 2026 Viva Republica, Inc.

package im.toss.necto.plugins

import im.toss.necto.model.NectoBridgeError
import im.toss.necto.model.NectoBridgeErrorCode
import im.toss.necto.model.NectoJsonValue
import im.toss.necto.model.jsonArray
import im.toss.necto.model.jsonObject
import im.toss.necto.model.jsonOf
import im.toss.necto.sdk.NectoPlugin
import im.toss.necto.sdk.NectoPluginPanel
import im.toss.necto.sdk.NectoRegistrar
import java.util.Locale

/**
 * Messages on long-lived channels — sockets, Bluetooth, streams — newest first.
 *
 * Like [NectoNetworkPlugin], this is an interface: the app reports and Necto carries.
 * `necto-okhttp-websocket-capture-plugin` is one ready-made source for OkHttp WebSockets.
 *
 * ```kotlin
 * val realtime = NectoRealtimePlugin()
 * NectoAndroid.start(context, listOf(realtime))
 *
 * // Wherever the app already sees a message:
 * realtime.report("Chat", NectoRealtimePlugin.Direction.INBOUND, json)
 * ```
 */
public class NectoRealtimePlugin internal constructor(private val now: () -> Long) : NectoPlugin {
    public constructor() : this(System::currentTimeMillis)

    public enum class Direction(internal val rawValue: String) {
        OUTBOUND("out"),
        INBOUND("in"),

        /** Connected, closed, failed: something that happened to the channel itself. */
        STATE("state"),
    }

    internal class Record(
        val seq: Int,
        val at: Long,
        val channel: String,
        val direction: Direction,
        /** Cut at [TEXT_LIMIT]. */
        val text: String,
        val byteCount: Int,
        val isBinary: Boolean,
    )

    private class BinaryRun(val channel: String, val direction: Direction, val startedAt: Long) {
        var count = 0
        var byteCount = 0
    }

    override val id: String = "realtime"

    override val panel: NectoPluginPanel = NectoPanels.resource("realtime")

    private val lock = Any()
    private val records = ArrayDeque<Record>()
    private val channelCounts = HashMap<String, Int>()
    private val binaryRuns = LinkedHashMap<String, BinaryRun>()
    private val observers = NectoListeners()
    private var nextSeq = 0

    override fun register(necto: NectoRegistrar) {
        necto.handle("realtime.list") { input ->
            val wanted = input["limit"]?.numberValue ?: 500.0
            val limit = if (wanted.isFinite()) wanted.coerceIn(1.0, CAPACITY.toDouble()).toInt() else 500
            jsonObject("records" to jsonArray(list(limit).map(::summary)))
        }

        necto.handle("realtime.detail") { input ->
            val seq = input["id"]?.stringValue?.toIntOrNull()
                ?: throw NectoBridgeError(NectoBridgeErrorCode.INVALID_INPUT, "id is required")
            val record = synchronized(lock) { records.firstOrNull { it.seq == seq } }
                ?: throw NectoBridgeError(NectoBridgeErrorCode.OPERATION_UNAVAILABLE, "No record '$seq'")
            jsonObject("record" to detail(record))
        }

        // Held open until the caller stops listening; `publish` does the sending.
        necto.stream("realtime.observe") { _, out -> observers.hold(out) }

        necto.handle("realtime.clear") {
            synchronized(lock) {
                records.clear()
                channelCounts.clear()
                binaryRuns.clear()
            }
            jsonObject("cleared" to jsonOf(true))
        }
    }

    /** Reports one text message. Safe from any thread, connected or not. */
    public fun report(channel: String, direction: Direction, text: String) {
        publish(synchronized(lock) {
            flushBinaryRuns() + append(channel, direction, text, text.toByteArray().size, isBinary = false)
        })
    }

    /**
     * Reports a binary message by size. Bursts are folded into one row per channel and
     * direction, so file chunks do not push everything else out.
     */
    public fun reportBinary(channel: String, direction: Direction, byteCount: Int) {
        publish(synchronized(lock) {
            val key = "$channel\u001F${direction.rawValue}"
            val run = binaryRuns.getOrPut(key) { BinaryRun(channel, direction, now()) }
            run.count += 1
            run.byteCount += byteCount
            // ponytail: no timer; the last run appears with the next record or list. Add one if that is too late.
            if (now() - run.startedAt < BINARY_WINDOW_MS) return@synchronized emptyList()
            binaryRuns.remove(key)
            listOf(append(run))
        })
    }

    /** Newest first, at most [limit] per channel. */
    internal fun list(limit: Int): List<Record> {
        val (flushed, page) = synchronized(lock) {
            val taken = HashMap<String, Int>()
            flushBinaryRuns() to records.filter { record ->
                val count = taken.getOrDefault(record.channel, 0)
                taken[record.channel] = count + 1
                count < limit
            }
        }
        publish(flushed)
        return page
    }

    private fun flushBinaryRuns(): List<Record> {
        val runs = binaryRuns.values.sortedBy { it.startedAt }
        binaryRuns.clear()
        return runs.map(::append)
    }

    private fun append(run: BinaryRun): Record =
        append(run.channel, run.direction, "binary ×${run.count}, ${byteCount(run.byteCount)}", run.byteCount, isBinary = true)

    private fun append(channel: String, direction: Direction, text: String, byteCount: Int, isBinary: Boolean): Record {
        nextSeq += 1
        val record = Record(nextSeq, now(), channel, direction, text.take(TEXT_LIMIT), byteCount, isBinary)
        records.addFirst(record)
        val count = channelCounts.getOrDefault(channel, 0) + 1
        channelCounts[channel] = count
        if (count > CAPACITY) {
            records.removeAt(records.indexOfLast { it.channel == channel })
            channelCounts[channel] = count - 1
        }
        return record
    }

    private fun publish(added: List<Record>) {
        for (record in added) observers.broadcast(jsonObject("record" to summary(record)))
    }

    public companion object {
        /** Per channel, so a chatty channel cannot push a quiet one out. */
        public const val CAPACITY: Int = 1_000
        internal const val TEXT_LIMIT = 512 * 1_024
        internal const val PREVIEW_LIMIT = 300

        /** How long binary messages are folded into one row. */
        internal const val BINARY_WINDOW_MS = 1_000L

        /** A list row. The full text is behind `detail`. */
        internal fun summary(record: Record): NectoJsonValue = jsonObject(fields(record))

        internal fun detail(record: Record): NectoJsonValue = jsonObject(fields(record) + ("text" to jsonOf(record.text)))

        private fun fields(record: Record) = mapOf(
            "id" to jsonOf(record.seq.toString()),
            "seq" to jsonOf(record.seq),
            "at" to jsonOf(record.at),
            "channel" to jsonOf(record.channel),
            "direction" to jsonOf(record.direction.rawValue),
            "preview" to jsonOf(record.text.take(PREVIEW_LIMIT)),
            "byteCount" to jsonOf(record.byteCount),
            "isBinary" to jsonOf(record.isBinary),
        )

        internal fun byteCount(bytes: Int): String = when {
            bytes < 1_024 -> "$bytes B"
            bytes < 1_024 * 1_024 -> String.format(Locale.ROOT, "%.1f KB", bytes / 1_024.0)
            else -> String.format(Locale.ROOT, "%.1f MB", bytes / (1_024.0 * 1_024))
        }
    }
}
