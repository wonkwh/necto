// Copyright (c) 2026 Viva Republica, Inc.

package im.toss.necto.okhttp

import im.toss.necto.plugins.NectoRealtimePlugin
import im.toss.necto.plugins.NectoRealtimePlugin.Direction
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString

/**
 * Opens an OkHttp WebSocket whose traffic is reported to the realtime plugin, the
 * counterpart of the iOS `NectoWebSocketCapture`. OkHttp has no hook for sockets, so the
 * app opens captured ones through this call instead of `newWebSocket`.
 *
 * ```kotlin
 * val socket = client.newCapturedWebSocket(request, listener, realtime)
 * ```
 *
 * The listener receives the captured socket, so a reply sent from a callback is
 * recorded too. A channel is named [channel] when given, otherwise by host and path; the
 * query is dropped because signed URLs carry credentials there.
 */
public fun OkHttpClient.newCapturedWebSocket(
    request: Request,
    listener: WebSocketListener,
    realtime: NectoRealtimePlugin,
    channel: String = request.url.host + request.url.encodedPath,
): WebSocket {
    val captured = CapturedWebSocket(channel, realtime)
    captured.socket = newWebSocket(request, object : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            captured.socket = webSocket
            realtime.report(channel, Direction.STATE, "connect")
            listener.onOpen(captured, response)
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            captured.socket = webSocket
            realtime.report(channel, Direction.INBOUND, text)
            listener.onMessage(captured, text)
        }

        override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
            captured.socket = webSocket
            realtime.reportBinary(channel, Direction.INBOUND, bytes.size)
            listener.onMessage(captured, bytes)
        }

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            captured.socket = webSocket
            realtime.report(channel, Direction.STATE, "closing $code")
            listener.onClosing(captured, code, reason)
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
            captured.socket = webSocket
            realtime.report(channel, Direction.STATE, "close $code")
            listener.onClosed(captured, code, reason)
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            captured.socket = webSocket
            realtime.report(channel, Direction.STATE, "failed: ${t.message ?: t.javaClass.simpleName}")
            listener.onFailure(captured, t, response)
        }
    })
    return captured
}

/** Reports what the app sends. Callbacks can run before `newWebSocket` returns, so each one hands over the socket. */
private class CapturedWebSocket(private val channel: String, private val realtime: NectoRealtimePlugin) : WebSocket {
    @Volatile lateinit var socket: WebSocket

    override fun request(): Request = socket.request()

    override fun queueSize(): Long = socket.queueSize()

    override fun send(text: String): Boolean = socket.send(text).also {
        if (it) realtime.report(channel, Direction.OUTBOUND, text)
    }

    override fun send(bytes: ByteString): Boolean = socket.send(bytes).also {
        if (it) realtime.reportBinary(channel, Direction.OUTBOUND, bytes.size)
    }

    override fun close(code: Int, reason: String?): Boolean = socket.close(code, reason)

    override fun cancel() = socket.cancel()
}
