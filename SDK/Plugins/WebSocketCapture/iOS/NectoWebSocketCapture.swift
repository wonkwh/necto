//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import Foundation
import NectoRealtimePlugin
import ObjectiveC

/// Captures `URLSessionWebSocketTask` traffic and reports it to the realtime plugin.
///
/// One possible source, in a module of its own so an app that does not use
/// `URLSessionWebSocketTask` never links it. Uses private Foundation classes; do not
/// include this product in apps that prohibit private APIs.
///
/// ```swift
/// let realtime = NectoRealtimePlugin()
/// NectoSDK.register(realtime)
/// NectoWebSocketCapture.install(reporting: realtime)
/// ```
///
/// A channel is named by the task's `taskDescription` when the app sets one, otherwise
/// by host and path. The query is dropped: signed URLs carry credentials there.
public enum NectoWebSocketCapture {
    /// Hooks stay installed for the life of the process; the last plugin passed here
    /// receives the records.
    public static func install(reporting realtime: NectoRealtimePlugin) {
        lock.withLock { reporter = realtime }
        _ = installed
    }

    private typealias ReceiveHandler = @convention(block) (AnyObject?, NSError?) -> Void

    private static let sendSelector = NSSelectorFromString("sendMessage:completionHandler:")
    private static let receiveSelector = NSSelectorFromString("receiveMessageWithCompletionHandler:")
    private static let cancelSelector = NSSelectorFromString("cancelWithCloseCode:reason:")
    private static let lock = NSLock()
    private nonisolated(unsafe) static var reporter: NectoRealtimePlugin?
    private nonisolated(unsafe) static var seenKey: UInt8 = 0
    private nonisolated(unsafe) static var announcedKey: UInt8 = 0

    /// Hooks the session methods that make tasks, then follows each task's real class.
    /// Task class names change with the OS, so they are never looked up by name.
    private static let installed: Void = {
        let probe = URLSession(configuration: .default)
        defer { probe.invalidateAndCancel() }
        let sessionClasses = [URLSession.self, object_getClass(URLSession.shared), object_getClass(probe)].compactMap { $0 }
        for cls in sessionClasses {
            hookFactory(cls, "webSocketTaskWithURL:")
            hookFactory(cls, "webSocketTaskWithRequest:")
            hookFactoryWithProtocols(cls)
        }
    }()

    // MARK: Task creation

    private static func hookFactory(_ cls: AnyClass, _ name: String) {
        let selector = NSSelectorFromString(name)
        Swizzle.replace(cls, selector, arguments: 3) { imp in
            typealias Original = @convention(c) (AnyObject, Selector, AnyObject) -> AnyObject?
            let original = unsafeBitCast(imp, to: Original.self)
            let block: @convention(block) (AnyObject, AnyObject) -> AnyObject? = { session, target in
                let task = original(session, selector, target)
                didCreate(task)
                return task
            }
            return block
        }
    }

    private static func hookFactoryWithProtocols(_ cls: AnyClass) {
        let selector = NSSelectorFromString("webSocketTaskWithURL:protocols:")
        Swizzle.replace(cls, selector, arguments: 4) { imp in
            typealias Original = @convention(c) (AnyObject, Selector, AnyObject, AnyObject) -> AnyObject?
            let original = unsafeBitCast(imp, to: Original.self)
            let block: @convention(block) (AnyObject, AnyObject, AnyObject) -> AnyObject? = { session, url, protocols in
                let task = original(session, selector, url, protocols)
                didCreate(task)
                return task
            }
            return block
        }
    }

    /// Creation methods may call one another, so each task is counted once.
    private static func didCreate(_ task: AnyObject?) {
        guard let task, objc_getAssociatedObject(task, &seenKey) == nil, let cls = object_getClass(task) else {
            return
        }
        objc_setAssociatedObject(task, &seenKey, true, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
        hookTask(cls)
    }

    /// Logged on first traffic rather than at creation: apps name the task right after making it.
    private static func announceIfNeeded(_ task: AnyObject) {
        guard objc_getAssociatedObject(task, &announcedKey) == nil else {
            return
        }
        objc_setAssociatedObject(task, &announcedKey, true, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
        report(task, .state, "connect")
    }

    // MARK: Traffic

    private static func hookTask(_ cls: AnyClass) {
        Swizzle.replace(cls, sendSelector, arguments: 4) { imp in
            typealias Original = @convention(c) (AnyObject, Selector, AnyObject, AnyObject?) -> Void
            let original = unsafeBitCast(imp, to: Original.self)
            let block: @convention(block) (AnyObject, AnyObject, AnyObject?) -> Void = { task, message, handler in
                announceIfNeeded(task)
                record(message, on: task, direction: .outbound)
                original(task, sendSelector, message, handler)
            }
            return block
        }

        Swizzle.replace(cls, receiveSelector, arguments: 3) { imp in
            typealias Original = @convention(c) (AnyObject, Selector, AnyObject) -> Void
            let original = unsafeBitCast(imp, to: Original.self)
            let block: @convention(block) (AnyObject, AnyObject) -> Void = { task, handler in
                announceIfNeeded(task)
                // Wraps the app's completion to read the message, then hands it on unchanged.
                let forward = unsafeBitCast(handler, to: ReceiveHandler.self)
                let wrapped: ReceiveHandler = { message, error in
                    if let message {
                        record(message, on: task, direction: .inbound)
                    } else if let error {
                        report(task, .state, "receive failed: \(error.localizedDescription)")
                    }
                    forward(message, error)
                }
                original(task, receiveSelector, unsafeBitCast(wrapped, to: AnyObject.self))
            }
            return block
        }

        Swizzle.replace(cls, cancelSelector, arguments: 4) { imp in
            typealias Original = @convention(c) (AnyObject, Selector, Int, AnyObject?) -> Void
            let original = unsafeBitCast(imp, to: Original.self)
            let block: @convention(block) (AnyObject, Int, AnyObject?) -> Void = { task, closeCode, reason in
                report(task, .state, "close \(closeCode)")
                original(task, cancelSelector, closeCode, reason)
            }
            return block
        }
    }

    /// `NSURLSessionWebSocketMessage` has no Swift type, so it is read through KVC.
    private static func record(_ message: AnyObject, on task: AnyObject, direction: NectoRealtimePlugin.Direction) {
        guard let reporter = lock.withLock({ reporter }), let message = message as? NSObject else {
            return
        }
        if message.responds(to: NSSelectorFromString("string")), let text = message.value(forKey: "string") as? String {
            reporter.report(channel: channel(of: task), direction: direction, text: text)
        } else if message.responds(to: NSSelectorFromString("data")), let data = message.value(forKey: "data") as? Data {
            reporter.reportBinary(channel: channel(of: task), direction: direction, byteCount: data.count)
        }
    }

    private static func report(_ task: AnyObject, _ direction: NectoRealtimePlugin.Direction, _ text: String) {
        lock.withLock { reporter }?.report(channel: channel(of: task), direction: direction, text: text)
    }

    static func channel(of task: AnyObject) -> String {
        let task = task as? URLSessionTask
        if let name = task?.taskDescription, !name.isEmpty {
            return name
        }
        guard let url = task?.originalRequest?.url else {
            return "WebSocket"
        }
        return "WS \(url.host ?? "")\(url.path)"
    }
}
