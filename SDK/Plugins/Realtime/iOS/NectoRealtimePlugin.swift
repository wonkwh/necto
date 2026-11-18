//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import Foundation
import NectoModel
import NectoSDK

/// Messages on long-lived channels — sockets, Bluetooth, streams — newest first.
///
/// Like `NectoNetworkPlugin`, this is an interface: the app reports and Necto carries.
/// `NectoWebSocketCapture` and `NectoBluetoothCapture` are ready-made sources in modules
/// of their own.
///
/// ```swift
/// let realtime = NectoRealtimePlugin()
/// NectoSDK.register(realtime)
///
/// // Wherever the app already sees a message:
/// realtime.report(channel: "Chat", direction: .inbound, text: json)
/// ```
public final class NectoRealtimePlugin: NectoPlugin, @unchecked Sendable {
    public enum Direction: String, Sendable {
        case outbound = "out"
        case inbound = "in"
        /// Connected, closed, failed: something that happened to the channel itself.
        case state
    }

    public let id = "realtime"

    public var panel: NectoPluginPanel? { NectoPluginPanel(bundle: .module) }

    let store: RealtimeStore

    public convenience init() {
        self.init(store: RealtimeStore())
    }

    init(store: RealtimeStore) {
        self.store = store
    }

    /// Reports one text message. Safe from any thread, connected or not.
    public func report(channel: String, direction: Direction, text: String) {
        store.record(channel: channel, direction: direction, text: text)
    }

    /// Reports a binary message by size. Bursts are folded into one row per channel and
    /// direction, so file chunks do not push everything else out.
    public func reportBinary(channel: String, direction: Direction, byteCount: Int) {
        store.recordBinary(channel: channel, direction: direction, byteCount: byteCount)
    }

    public func register(_ necto: NectoRegistrar) {
        necto.handle("realtime.list") { [store] input in
            let wanted = input["limit"]?.numberValue ?? 500
            let limit = wanted.isFinite ? Int(max(1, min(wanted, Double(RealtimeStore.capacity)))) : 500
            return ["records": .array(store.list(limit: limit).map(RealtimeStore.summary))]
        }

        necto.handle("realtime.detail") { [store] input in
            guard let id = input["id"]?.stringValue, let seq = Int(id) else {
                throw NectoBridgeError(code: .invalidInput, message: "id is required")
            }
            guard let record = store.find(seq: seq) else {
                throw NectoBridgeError(code: .operationUnavailable, message: "No record '\(id)'")
            }
            return ["record": RealtimeStore.detail(record)]
        }

        necto.handle("realtime.observe") { [store] _, out in
            try await store.observe(out)
        }

        necto.handle("realtime.clear") { [store] _ in
            store.clear()
            return ["cleared": .bool(true)]
        }
    }
}
