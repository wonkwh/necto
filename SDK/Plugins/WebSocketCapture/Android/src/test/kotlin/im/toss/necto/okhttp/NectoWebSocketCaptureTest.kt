// Copyright (c) 2026 Viva Republica, Inc.

package im.toss.necto.okhttp

import im.toss.necto.model.jsonObject
import im.toss.necto.plugins.NectoRealtimePlugin
import im.toss.necto.testing.PluginHarness
import kotlinx.coroutines.runBlocking
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class NectoWebSocketCaptureTest {
    @Test
    fun recordsBothDirectionsAndStateUnderHostAndPath() = runBlocking {
        val server = MockWebServer()
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onMessage(webSocket: WebSocket, text: String) {
                webSocket.send("echo $text")
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(code, reason)
            }
        }))
        server.start()

        val realtime = NectoRealtimePlugin()
        val closed = CountDownLatch(1)
        val request = Request.Builder().url(server.url("/chat?token=secret")).build()
        OkHttpClient().newCapturedWebSocket(request, object : WebSocketListener() {
            // Sent from the callback through the socket it is handed: recorded as outbound.
            override fun onOpen(webSocket: WebSocket, response: Response) {
                webSocket.send("hi")
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                webSocket.close(1000, null)
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                closed.countDown()
            }
        }, realtime)
        assertTrue(closed.await(5, TimeUnit.SECONDS))
        server.shutdown()

        val records = PluginHarness(realtime).call("realtime.list", jsonObject())["records"]!!.arrayValue!!.reversed()
        assertEquals(
            listOf("state connect", "out hi", "in echo hi", "state closing 1000", "state close 1000"),
            records.map { "${it["direction"]!!.stringValue} ${it["preview"]!!.stringValue}" },
        )
        assertEquals(setOf("${server.hostName}/chat"), records.map { it["channel"]!!.stringValue }.toSet())
    }
}
