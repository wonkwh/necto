//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import Foundation
import NectoModel
import NectoSDK

/// One message, or one change of a channel's state.
struct RealtimeRecord: Sendable, Equatable {
    let seq: Int
    let at: Date
    let channel: String
    let direction: NectoRealtimePlugin.Direction
    /// Cut at `RealtimeStore.textLimit`.
    let text: String
    let byteCount: Int
    let isBinary: Bool
}

/// Holds records newest first and streams new ones to listening panels. Thread-safe.
final class RealtimeStore: @unchecked Sendable {
    /// Per channel, so a chatty channel cannot push a quiet one out.
    static let capacity = 1_000
    static let textLimit = 512 * 1_024
    static let previewLimit = 300
    /// How long binary messages are folded into one row.
    static let binaryWindow: TimeInterval = 1

    private struct BinaryRun {
        let channel: String
        let direction: NectoRealtimePlugin.Direction
        let startedAt: Date
        var count = 0
        var byteCount = 0
    }

    private let lock = NSLock()
    private let now: @Sendable () -> Date
    private var records: [RealtimeRecord] = []
    private var channelCounts: [String: Int] = [:]
    private var listeners: [UUID: NectoRegistrar.Out] = [:]
    private var nextSeq = 0
    private var binaryRuns: [String: BinaryRun] = [:]

    init(now: @escaping @Sendable () -> Date = { Date() }) {
        self.now = now
    }

    // MARK: Recording

    func record(channel: String, direction: NectoRealtimePlugin.Direction, text: String) {
        let added = lock.withLock {
            flushBinaryRuns() + [append(channel: channel, direction: direction, text: text, byteCount: text.utf8.count, isBinary: false)]
        }
        publish(added)
    }

    func recordBinary(channel: String, direction: NectoRealtimePlugin.Direction, byteCount: Int) {
        let added = lock.withLock { () -> [RealtimeRecord] in
            let key = "\(channel)\u{1F}\(direction.rawValue)"
            var run = binaryRuns[key] ?? BinaryRun(channel: channel, direction: direction, startedAt: now())
            run.count += 1
            run.byteCount += byteCount
            binaryRuns[key] = run
            // ponytail: no timer; the last run appears with the next record or list. Add one if that is too late.
            guard now().timeIntervalSince(run.startedAt) >= Self.binaryWindow else {
                return []
            }
            binaryRuns[key] = nil
            return [append(run)]
        }
        publish(added)
    }

    // MARK: Reading

    /// Newest first, at most `limit` per channel.
    func list(limit: Int) -> [RealtimeRecord] {
        let (flushed, page) = lock.withLock { () -> ([RealtimeRecord], [RealtimeRecord]) in
            let flushed = flushBinaryRuns()
            var taken: [String: Int] = [:]
            let page = records.filter { record in
                let count = taken[record.channel, default: 0]
                taken[record.channel] = count + 1
                return count < limit
            }
            return (flushed, page)
        }
        publish(flushed)
        return page
    }

    func find(seq: Int) -> RealtimeRecord? {
        lock.withLock { records.first { $0.seq == seq } }
    }

    func clear() {
        lock.withLock {
            records.removeAll()
            channelCounts.removeAll()
            binaryRuns.removeAll()
        }
    }

    /// Held open until the caller stops listening; `publish` does the sending.
    func observe(_ out: NectoRegistrar.Out) async throws {
        let token = UUID()
        lock.withLock { listeners[token] = out }
        defer { lock.withLock { _ = listeners.removeValue(forKey: token) } }
        while !Task.isCancelled {
            try await Task.sleep(for: .seconds(60))
        }
    }

    // MARK: Internals, called with the lock held

    private func flushBinaryRuns() -> [RealtimeRecord] {
        let runs = binaryRuns.values.sorted { $0.startedAt < $1.startedAt }
        binaryRuns.removeAll()
        return runs.map { append($0) }
    }

    private func append(_ run: BinaryRun) -> RealtimeRecord {
        let size = ByteCountFormatter.string(fromByteCount: Int64(run.byteCount), countStyle: .binary)
        return append(
            channel: run.channel,
            direction: run.direction,
            text: "binary ×\(run.count), \(size)",
            byteCount: run.byteCount,
            isBinary: true
        )
    }

    private func append(
        channel: String, direction: NectoRealtimePlugin.Direction, text: String, byteCount: Int, isBinary: Bool
    ) -> RealtimeRecord {
        nextSeq += 1
        let record = RealtimeRecord(
            seq: nextSeq,
            at: now(),
            channel: channel,
            direction: direction,
            text: byteCount > Self.textLimit ? String(text.prefix(Self.textLimit)) : text,
            byteCount: byteCount,
            isBinary: isBinary
        )
        records.insert(record, at: 0)
        channelCounts[channel, default: 0] += 1
        if channelCounts[channel, default: 0] > Self.capacity,
           let oldest = records.lastIndex(where: { $0.channel == channel }) {
            records.remove(at: oldest)
            channelCounts[channel, default: 0] -= 1
        }
        return record
    }

    private func publish(_ added: [RealtimeRecord]) {
        guard !added.isEmpty else {
            return
        }
        let outs = lock.withLock { Array(listeners.values) }
        for record in added {
            let payload: NectoJSONValue = ["record": Self.summary(record)]
            for out in outs {
                Task { await out.send(payload) }
            }
        }
    }

    // MARK: Coding

    /// A list row. The full text is behind `detail`.
    static func summary(_ record: RealtimeRecord) -> NectoJSONValue {
        [
            "id": .string(String(record.seq)),
            "seq": .number(Double(record.seq)),
            "at": .number(record.at.timeIntervalSince1970 * 1_000),
            "channel": .string(record.channel),
            "direction": .string(record.direction.rawValue),
            "preview": .string(String(record.text.prefix(previewLimit))),
            "byteCount": .number(Double(record.byteCount)),
            "isBinary": .bool(record.isBinary),
        ]
    }

    static func detail(_ record: RealtimeRecord) -> NectoJSONValue {
        guard case var .object(fields) = summary(record) else {
            return summary(record)
        }
        fields["text"] = .string(record.text)
        return .object(fields)
    }
}
