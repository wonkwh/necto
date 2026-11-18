//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import CoreBluetooth
import Foundation
import NectoRealtimePlugin
import ObjectiveC

/// Captures CoreBluetooth central traffic — scans, connections, writes and the values
/// the app's delegates receive — and reports it on the `BLE` channel.
///
/// One possible source, in a module of its own so an app without Bluetooth never links
/// it. The original methods always run; nothing is changed or delayed.
///
/// ```swift
/// let realtime = NectoRealtimePlugin()
/// NectoSDK.register(realtime)
/// NectoBluetoothCapture.install(reporting: realtime)
/// ```
///
/// Delegates are hooked when they are set, or when the app first scans or connects, so
/// the state update a `CBCentralManager(delegate:queue:)` sends at creation is missed.
public enum NectoBluetoothCapture {
    static let channel = "BLE"

    /// Hooks stay installed for the life of the process; the last plugin passed here
    /// receives the records.
    public static func install(reporting realtime: NectoRealtimePlugin) {
        lock.withLock { reporter = realtime }
        _ = installed
    }

    private typealias Original3 = @convention(c) (AnyObject, Selector, AnyObject?) -> Void
    private typealias Original4 = @convention(c) (AnyObject, Selector, AnyObject?, AnyObject?) -> Void
    private typealias Original5 = @convention(c) (AnyObject, Selector, AnyObject?, AnyObject?, AnyObject?) -> Void

    private static let states = ["unknown", "resetting", "unsupported", "unauthorized", "poweredOff", "poweredOn"]
    private static let lock = NSLock()
    private nonisolated(unsafe) static var reporter: NectoRealtimePlugin?
    /// The last advertisement per peripheral; the same one arrives many times a second.
    private nonisolated(unsafe) static var lastAdvertisement: [UUID: String] = [:]

    private static let installed: Void = {
        hookDelegateSetter(CBCentralManager.self, hookCentralDelegate)
        hookDelegateSetter(CBPeripheral.self, hookPeripheralDelegate)
        hookCalls()
    }()

    // MARK: Delegates

    /// Values and state arrive at the app's delegate, so its class is hooked when it is set.
    private static func hookDelegateSetter(_ cls: AnyClass, _ hookDelegate: @escaping (AnyObject) -> Void) {
        let selector = NSSelectorFromString("setDelegate:")
        Swizzle.replace(cls, selector, arguments: 3) { imp in
            let original = unsafeBitCast(imp, to: Original3.self)
            let block: @convention(block) (AnyObject, AnyObject?) -> Void = { target, delegate in
                if let delegate {
                    hookDelegate(delegate)
                }
                original(target, selector, delegate)
            }
            return block
        }
    }

    private static func hookCentralDelegate(_ delegate: AnyObject) {
        guard let cls = object_getClass(delegate) else {
            return
        }
        hook(cls, "centralManagerDidUpdateState:") { (central: AnyObject?) in
            let state = (central as? CBCentralManager)?.state.rawValue ?? 0
            record(.state, "state \(states.indices.contains(state) ? states[state] : "\(state)")")
        }
        hook(cls, "centralManager:didDiscoverPeripheral:advertisementData:RSSI:") {
            (_: AnyObject?, peripheral: AnyObject?, advertisement: AnyObject?, rssi: AnyObject?) in
            guard let peripheral = peripheral as? CBPeripheral else {
                return
            }
            let summary = describe(advertisement as? [String: Any] ?? [:])
            let changed = lock.withLock {
                lastAdvertisement.updateValue(summary, forKey: peripheral.identifier) != summary
            }
            if changed {
                record(.inbound, "discover \(name(of: peripheral)) rssi \(rssi.map { "\($0)" } ?? "?") \(summary)")
            }
        }
        hook(cls, "centralManager:didConnectPeripheral:") { (_: AnyObject?, peripheral: AnyObject?) in
            record(.state, "connected \(name(of: peripheral))")
        }
        hook(cls, "centralManager:didFailToConnectPeripheral:error:") {
            (_: AnyObject?, peripheral: AnyObject?, error: AnyObject?) in
            record(.state, "connect failed \(name(of: peripheral))\(describe(error: error))")
        }
        hook(cls, "centralManager:didDisconnectPeripheral:error:") {
            (_: AnyObject?, peripheral: AnyObject?, error: AnyObject?) in
            record(.state, "disconnected \(name(of: peripheral))\(describe(error: error))")
        }
    }

    private static func hookPeripheralDelegate(_ delegate: AnyObject) {
        guard let cls = object_getClass(delegate) else {
            return
        }
        hook(cls, "peripheral:didUpdateValueForCharacteristic:error:") {
            (_: AnyObject?, characteristic: AnyObject?, error: AnyObject?) in
            let characteristic = characteristic as? CBCharacteristic
            record(.inbound, "value \(characteristic?.uuid.uuidString ?? "?") \(describe(characteristic?.value))\(describe(error: error))")
        }
        hook(cls, "peripheral:didWriteValueForCharacteristic:error:") {
            (_: AnyObject?, characteristic: AnyObject?, error: AnyObject?) in
            let uuid = (characteristic as? CBCharacteristic)?.uuid.uuidString ?? "?"
            record(.state, "write done \(uuid)\(describe(error: error))")
        }
        hook(cls, "peripheral:didDiscoverServices:") { (peripheral: AnyObject?, error: AnyObject?) in
            let services = (peripheral as? CBPeripheral)?.services?.map(\.uuid.uuidString) ?? []
            record(.state, "services \(services.joined(separator: ","))\(describe(error: error))")
        }
    }

    // MARK: Calls the app makes

    private static func hookCalls() {
        let write = NSSelectorFromString("writeValue:forCharacteristic:type:")
        typealias Write = @convention(c) (AnyObject, Selector, AnyObject, AnyObject, Int) -> Void
        Swizzle.replace(CBPeripheral.self, write, arguments: 5) { imp in
            let original = unsafeBitCast(imp, to: Write.self)
            let block: @convention(block) (AnyObject, AnyObject, AnyObject, Int) -> Void = {
                peripheral, data, characteristic, type in
                let uuid = (characteristic as? CBCharacteristic)?.uuid.uuidString ?? "?"
                record(.outbound, "write \(uuid) \(describe(data as? Data))")
                original(peripheral, write, data, characteristic, type)
            }
            return block
        }

        let connect = NSSelectorFromString("connectPeripheral:options:")
        Swizzle.replace(CBCentralManager.self, connect, arguments: 4) { imp in
            let original = unsafeBitCast(imp, to: Original4.self)
            let block: @convention(block) (AnyObject, AnyObject?, AnyObject?) -> Void = { central, peripheral, options in
                hookDelegate(of: central)
                record(.outbound, "connect \(name(of: peripheral))")
                original(central, connect, peripheral, options)
            }
            return block
        }

        let cancel = NSSelectorFromString("cancelPeripheralConnection:")
        Swizzle.replace(CBCentralManager.self, cancel, arguments: 3) { imp in
            let original = unsafeBitCast(imp, to: Original3.self)
            let block: @convention(block) (AnyObject, AnyObject?) -> Void = { central, peripheral in
                record(.outbound, "cancel connection \(name(of: peripheral))")
                original(central, cancel, peripheral)
            }
            return block
        }

        let scan = NSSelectorFromString("scanForPeripheralsWithServices:options:")
        Swizzle.replace(CBCentralManager.self, scan, arguments: 4) { imp in
            let original = unsafeBitCast(imp, to: Original4.self)
            let block: @convention(block) (AnyObject, AnyObject?, AnyObject?) -> Void = { central, services, options in
                hookDelegate(of: central)
                // A new scan shows repeated advertisements again.
                lock.withLock { lastAdvertisement.removeAll() }
                let uuids = (services as? [CBUUID])?.map(\.uuidString).joined(separator: ",") ?? "all"
                record(.outbound, "scan \(uuids)")
                original(central, scan, services, options)
            }
            return block
        }
    }

    // MARK: Hooking

    /// `CBCentralManager(delegate:queue:)` does not go through the setter, so the delegate
    /// is checked when the app scans or connects.
    private static func hookDelegate(of central: AnyObject) {
        if let delegate = (central as? CBCentralManager)?.delegate {
            hookCentralDelegate(delegate)
        }
    }

    /// Delegate methods are optional; only implemented ones are hooked.
    private static func hook(_ cls: AnyClass, _ name: String, _ observe: @escaping (AnyObject?) -> Void) {
        let selector = NSSelectorFromString(name)
        guard class_respondsToSelector(cls, selector) else {
            return
        }
        Swizzle.replace(cls, selector, arguments: 3) { imp in
            let original = unsafeBitCast(imp, to: Original3.self)
            let block: @convention(block) (AnyObject, AnyObject?) -> Void = { target, a in
                observe(a)
                original(target, selector, a)
            }
            return block
        }
    }

    private static func hook(_ cls: AnyClass, _ name: String, _ observe: @escaping (AnyObject?, AnyObject?) -> Void) {
        let selector = NSSelectorFromString(name)
        guard class_respondsToSelector(cls, selector) else {
            return
        }
        Swizzle.replace(cls, selector, arguments: 4) { imp in
            let original = unsafeBitCast(imp, to: Original4.self)
            let block: @convention(block) (AnyObject, AnyObject?, AnyObject?) -> Void = { target, a, b in
                observe(a, b)
                original(target, selector, a, b)
            }
            return block
        }
    }

    private static func hook(
        _ cls: AnyClass, _ name: String, _ observe: @escaping (AnyObject?, AnyObject?, AnyObject?) -> Void
    ) {
        let selector = NSSelectorFromString(name)
        guard class_respondsToSelector(cls, selector) else {
            return
        }
        Swizzle.replace(cls, selector, arguments: 5) { imp in
            let original = unsafeBitCast(imp, to: Original5.self)
            let block: @convention(block) (AnyObject, AnyObject?, AnyObject?, AnyObject?) -> Void = { target, a, b, c in
                observe(a, b, c)
                original(target, selector, a, b, c)
            }
            return block
        }
    }

    private static func hook(
        _ cls: AnyClass, _ name: String,
        _ observe: @escaping (AnyObject?, AnyObject?, AnyObject?, AnyObject?) -> Void
    ) {
        typealias Original6 = @convention(c) (AnyObject, Selector, AnyObject?, AnyObject?, AnyObject?, AnyObject?) -> Void
        let selector = NSSelectorFromString(name)
        guard class_respondsToSelector(cls, selector) else {
            return
        }
        Swizzle.replace(cls, selector, arguments: 6) { imp in
            let original = unsafeBitCast(imp, to: Original6.self)
            let block: @convention(block) (AnyObject, AnyObject?, AnyObject?, AnyObject?, AnyObject?) -> Void = {
                target, a, b, c, d in
                observe(a, b, c, d)
                original(target, selector, a, b, c, d)
            }
            return block
        }
    }

    // MARK: Describing

    private static func record(_ direction: NectoRealtimePlugin.Direction, _ text: String) {
        lock.withLock { reporter }?.report(channel: channel, direction: direction, text: text)
    }

    private static func name(of peripheral: AnyObject?) -> String {
        guard let peripheral = peripheral as? CBPeripheral else {
            return "?"
        }
        return peripheral.name ?? String(peripheral.identifier.uuidString.prefix(8))
    }

    /// Name, manufacturer data and services; RSSI changes every time, so it is left out.
    static func describe(_ advertisement: [String: Any]) -> String {
        var parts: [String] = []
        if let name = advertisement[CBAdvertisementDataLocalNameKey] as? String {
            parts.append("name=\(name)")
        }
        if let data = advertisement[CBAdvertisementDataManufacturerDataKey] as? Data {
            parts.append("mfr=\(data.map { String(format: "%02x", $0) }.joined())")
        }
        if let services = advertisement[CBAdvertisementDataServiceUUIDsKey] as? [CBUUID] {
            parts.append("services=\(services.map(\.uuidString).joined(separator: ","))")
        }
        return parts.joined(separator: " ")
    }

    /// UTF-8 text as itself, anything else as hex.
    static func describe(_ data: Data?) -> String {
        guard let data else {
            return "(nil)"
        }
        if let text = String(data: data, encoding: .utf8), !text.isEmpty {
            return text
        }
        return "0x" + data.map { String(format: "%02x", $0) }.joined()
    }

    private static func describe(error: AnyObject?) -> String {
        (error as? NSError).map { " error: \($0.localizedDescription)" } ?? ""
    }
}
