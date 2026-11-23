//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import CoreBluetooth
import Foundation
import Network
import Testing

@testable import NectoBluetoothCapturePlugin
@testable import NectoRealtimePlugin
@testable import NectoWebSocketCapturePlugin

/// Hooks are process-wide, so the WebSocket tests take turns.
@Suite("WebSocket capture", .serialized)
struct WebSocketCaptureTests {
    @Test("records what a URLSessionWebSocketTask sends and receives")
    func recordsMessages() async throws {
        let (store, url) = try await capture(path: "/echo?signature=secret")

        let task = URLSession(configuration: .default).webSocketTask(with: url)
        task.resume()
        task.send(.string("hello")) { _ in }
        let reply = try await task.receive()
        task.cancel(with: .normalClosure, reason: nil)

        guard case .string("hello") = reply else {
            Issue.record("No echo: \(reply)")
            return
        }
        let records = Array(store.list(limit: 10).reversed())
        // The query, where signed URLs keep credentials, never reaches the channel name.
        #expect(Set(records.map(\.channel)) == ["WS 127.0.0.1/echo"])
        #expect(records.map { "\($0.direction.rawValue) \($0.text)" } == ["state connect", "out hello", "in hello", "state close 1000"])
    }

    @Test("names a channel by the task's description")
    func namesChannelByTaskDescription() async throws {
        let (store, url) = try await capture(path: "/")

        let task = URLSession(configuration: .default).webSocketTask(with: url)
        task.taskDescription = "Sensor"
        task.resume()
        task.send(.string("g")) { _ in }
        _ = try await task.receive()
        task.cancel(with: .normalClosure, reason: nil)

        #expect(Set(store.list(limit: 10).map(\.channel)) == ["Sensor"])
    }

    private func capture(path: String) async throws -> (RealtimeStore, URL) {
        let store = RealtimeStore()
        NectoWebSocketCapture.install(reporting: NectoRealtimePlugin(store: store))
        let port = try await EchoServer.shared.start()
        return (store, try #require(URL(string: "ws://127.0.0.1:\(port)\(path)")))
    }
}

@Suite("Bluetooth capture")
struct BluetoothCaptureTests {
    @Test("describes an advertisement without its RSSI")
    func describesAdvertisementWithoutRSSI() {
        let advertisement: [String: Any] = [
            CBAdvertisementDataLocalNameKey: "Sensor-01",
            CBAdvertisementDataManufacturerDataKey: Data([0x01, 0xAB]),
            CBAdvertisementDataServiceUUIDsKey: [CBUUID(string: "180A")],
        ]

        #expect(NectoBluetoothCapture.describe(advertisement) == "name=Sensor-01 mfr=01ab services=180A")
    }

    @Test("describes a payload as text, or as hex when it is not")
    func describesPayloadAsTextOrHex() {
        #expect(NectoBluetoothCapture.describe(Data(#"{"a":1}"#.utf8)) == #"{"a":1}"#)
        #expect(NectoBluetoothCapture.describe(Data([0xFF, 0xFE])) == "0xfffe")
        #expect(NectoBluetoothCapture.describe(nil as Data?) == "(nil)")
    }
}

/// A loopback WebSocket server that sends every text frame back.
private final class EchoServer: @unchecked Sendable {
    static let shared = EchoServer()

    private let queue = DispatchQueue(label: "EchoServer")
    private var listener: NWListener?
    private var port: UInt16?
    private var connections: [NWConnection] = []

    func start() async throws -> UInt16 {
        if let port = queue.sync(execute: { port }) {
            return port
        }
        let parameters = NWParameters.tcp
        parameters.defaultProtocolStack.applicationProtocols.insert(NWProtocolWebSocket.Options(), at: 0)
        let listener = try NWListener(using: parameters)
        listener.newConnectionHandler = { [weak self] connection in
            guard let self else { return }
            connections.append(connection)
            connection.start(queue: queue)
            echo(connection)
        }
        let port: UInt16 = try await withCheckedThrowingContinuation { continuation in
            listener.stateUpdateHandler = { [listener] state in
                switch state {
                case .ready:
                    listener.stateUpdateHandler = nil
                    continuation.resume(returning: listener.port?.rawValue ?? 0)
                case let .failed(error):
                    listener.stateUpdateHandler = nil
                    continuation.resume(throwing: error)
                default:
                    break
                }
            }
            listener.start(queue: queue)
        }
        queue.sync {
            self.listener = listener
            self.port = port
        }
        return port
    }

    private func echo(_ connection: NWConnection) {
        connection.receiveMessage { [weak self] data, context, _, error in
            let metadata = context?.protocolMetadata(definition: NWProtocolWebSocket.definition) as? NWProtocolWebSocket.Metadata
            guard let data, error == nil, metadata?.opcode == .text else {
                return
            }
            let reply = NWConnection.ContentContext(identifier: "echo", metadata: [NWProtocolWebSocket.Metadata(opcode: .text)])
            connection.send(content: data, contentContext: reply, isComplete: true, completion: .idempotent)
            self?.echo(connection)
        }
    }
}
