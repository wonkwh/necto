// Copyright (c) 2026 Viva Republica, Inc.

package im.toss.necto

import im.toss.necto.model.jsonObject
import im.toss.necto.model.jsonOf
import im.toss.necto.plugins.NectoRealtimePlugin
import im.toss.necto.plugins.NectoRealtimePlugin.Direction
import im.toss.necto.testing.PluginHarness
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals

class RealtimePluginTests {
    private var clock = 0L
    private val realtime = NectoRealtimePlugin { clock }
    private val harness = PluginHarness(realtime)

    private suspend fun rows(limit: Int = 500) =
        harness.call("realtime.list", jsonObject("limit" to jsonOf(limit)))["records"]!!.arrayValue!!

    @Test
    fun listsNewestFirstWithDetailAndClear() = runBlocking {
        realtime.report("Chat", Direction.STATE, "connect")
        realtime.report("Chat", Direction.INBOUND, "x".repeat(400))

        val records = rows()
        assertEquals(listOf("in", "state"), records.map { it["direction"]!!.stringValue })
        assertEquals(300, records[0]["preview"]!!.stringValue!!.length)

        val detail = harness.call("realtime.detail", jsonObject("id" to records[0]["id"]!!))["record"]!!
        assertEquals(400, detail["text"]!!.stringValue!!.length)

        harness.call("realtime.clear")
        assertEquals(0, rows().size)
    }

    @Test
    fun binaryBurstsFoldIntoOneRowPerChannelAndDirection() = runBlocking {
        repeat(3) { realtime.reportBinary("Upload", Direction.OUTBOUND, 1_024) }
        clock = NectoRealtimePlugin.BINARY_WINDOW_MS
        realtime.reportBinary("Upload", Direction.OUTBOUND, 1_024)
        realtime.reportBinary("Upload", Direction.OUTBOUND, 10)

        val records = rows()
        assertEquals(listOf("binary ×1, 10 B", "binary ×4, 4.0 KB"), records.map { it["preview"]!!.stringValue })
        assertEquals(jsonOf(true), records[0]["isBinary"])
    }

    @Test
    fun aChattyChannelCannotPushAQuietOneOut() = runBlocking {
        realtime.report("Quiet", Direction.INBOUND, "hello")
        repeat(NectoRealtimePlugin.CAPACITY + 5) { realtime.report("Chatty", Direction.INBOUND, "$it") }

        val records = rows(NectoRealtimePlugin.CAPACITY)
        assertEquals(NectoRealtimePlugin.CAPACITY, records.count { it["channel"]!!.stringValue == "Chatty" })
        assertEquals(1, records.count { it["channel"]!!.stringValue == "Quiet" })
        assertEquals(2, rows(limit = 1).size)
    }
}
