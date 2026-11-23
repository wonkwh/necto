//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import Foundation
import Testing

@testable import NectoRealtimePlugin

@Suite("Realtime store")
struct RealtimeStoreTests {
    @Test("keeps the newest records within capacity")
    func keepsNewestRecordsWithinCapacity() {
        let store = RealtimeStore()

        for index in 0 ... RealtimeStore.capacity {
            store.record(channel: "Chat", direction: .outbound, text: "\(index)")
        }

        let page = store.list(limit: .max)
        #expect(page.count == RealtimeStore.capacity)
        #expect(page.first?.text == "\(RealtimeStore.capacity)")
        #expect(page.last?.text == "1")
    }

    @Test("a flooding channel does not push a quiet one out")
    func floodingOneChannelKeepsOtherChannels() {
        let store = RealtimeStore()

        store.record(channel: "Control", direction: .outbound, text: "start")
        for index in 0 ... RealtimeStore.capacity {
            store.record(channel: "Sensor", direction: .inbound, text: "\(index)")
        }

        #expect(store.list(limit: .max).filter { $0.channel == "Control" }.map(\.text) == ["start"])
        #expect(store.list(limit: .max).filter { $0.channel == "Sensor" }.count == RealtimeStore.capacity)
        #expect(store.list(limit: 1).map(\.channel) == ["Sensor", "Control"])
    }

    @Test("binary messages fold into one record per window")
    func coalescesBinaryMessagesIntoOneRecordPerWindow() {
        let clock = TestClock()
        let store = RealtimeStore { clock.now }

        store.recordBinary(channel: "Files", direction: .inbound, byteCount: 1_000)
        store.recordBinary(channel: "Files", direction: .inbound, byteCount: 24)
        #expect(store.find(seq: 1) == nil)

        clock.now += RealtimeStore.binaryWindow
        store.recordBinary(channel: "Files", direction: .inbound, byteCount: 1_024)

        let page = store.list(limit: 10)
        #expect(page.count == 1)
        #expect(page.first?.byteCount == 2_048)
        #expect(page.first?.isBinary == true)
        #expect(page.first?.text.hasPrefix("binary ×3") == true)
    }

    @Test("pending binary is flushed before the next text and into a list")
    func flushesPendingBinary() {
        let store = RealtimeStore()

        store.recordBinary(channel: "Files", direction: .inbound, byteCount: 10)
        store.record(channel: "Chat", direction: .outbound, text: "{}")
        #expect(store.list(limit: 10).map(\.channel) == ["Chat", "Files"])

        store.recordBinary(channel: "Files", direction: .inbound, byteCount: 10)
        #expect(store.list(limit: 10).map(\.channel) == ["Files", "Chat", "Files"])
    }

    @Test("a row carries a preview and the detail carries the whole text")
    func summaryCarriesPreviewAndDetailCarriesFullText() throws {
        let store = RealtimeStore()
        let text = String(repeating: "a", count: 1_000)
        store.record(channel: "Events", direction: .inbound, text: text)
        let record = try #require(store.find(seq: 1))

        #expect(RealtimeStore.summary(record)["preview"]?.stringValue?.count == RealtimeStore.previewLimit)
        #expect(RealtimeStore.summary(record)["text"] == nil)
        #expect(RealtimeStore.detail(record)["text"]?.stringValue == text)
    }
}

private final class TestClock: @unchecked Sendable {
    var now = Date(timeIntervalSince1970: 1_000_000)
}
