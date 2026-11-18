//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import Foundation
import ObjectiveC

// ponytail: a copy of the one in BluetoothCapture; each capture module stays linkable on its own.
/// Replaces a method's implementation, once per implementing class.
enum Swizzle {
    private static let lock = NSLock()
    private nonisolated(unsafe) static var replaced: Set<UInt> = []

    /// Replaces the implementation `cls` uses for `selector` with the block `makeBlock` returns.
    ///
    /// - Parameters:
    ///   - arguments: Including `self` and `_cmd`. A different count means the OS changed, so nothing is touched.
    ///   - makeBlock: Receives the original IMP and returns a `@convention(block)` block.
    static func replace(_ cls: AnyClass, _ selector: Selector, arguments: UInt32, _ makeBlock: (IMP) -> Any) {
        lock.lock()
        defer { lock.unlock() }
        guard let method = class_getInstanceMethod(cls, selector), method_getNumberOfArguments(method) == arguments,
              replaced.insert(UInt(bitPattern: method)).inserted else {
            return
        }
        method_setImplementation(method, imp_implementationWithBlock(makeBlock(method_getImplementation(method))))
    }
}
