//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import NectoFilesPlugin
import NectoPreferencesPlugin
import NectoSDK
import NectoSQLitePlugin
import NectoUIControlPlugin
import NectoURLSessionCapturePlugin
import SQLite3
import SwiftUI

/// Verifies SDK connection and plugin behaviour on a device or a simulator.
@main
struct ExampleApp: App {
    init() {
        // Serves the Network tab's sample requests without internet access.
        LocalAPI.shared.start()

        // This app happens to use URLSession, so it registers the plugin with the
        // ready-made capture wired in. An app with its own stack registers
        // `NectoNetworkPlugin()` instead and calls `report(_:)` from wherever it
        // already knows about a request.
        NectoSDK.register(URLSessionNetworkPlugin())

        // The other direction: a capability the Mac can call into rather than one
        // that only pushes.
        NectoSDK.register(ExampleContractPlugin())

        // Two more of the shipped plugins, registered the same way and just as
        // optional. An app that wants neither simply does not link them.
        NectoSDK.register(ExampleTelemetry.events)
        NectoSDK.register(ExampleTelemetry.performance)

        // These need nothing reported: they read what is already there. The seeds give
        // both panels something real to show on a fresh install.
        NectoSDK.register(NectoPreferencesPlugin())
        NectoSDK.register(NectoFilesPlugin())
        NectoSDK.register(NectoSQLitePlugin())
        NectoSDK.register(NectoUIControlPlugin())
        Self.seedPreferences()
        Self.seedFiles()
        Self.seedDatabase()

        NectoSDK.start()
        ExampleTelemetry.start()
    }

    var body: some Scene {
        WindowGroup {
            RootView()
        }
    }

    /// A few files of each shape a real app keeps, written once.
    private static func seedFiles() {
        guard let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first else {
            return
        }
        let receipts = documents.appending(path: "receipts")
        guard !FileManager.default.fileExists(atPath: receipts.path) else { return }

        try? FileManager.default.createDirectory(at: receipts, withIntermediateDirectories: true)
        try? Data("""
        {
          "id": "rcpt_9f8e7d",
          "total": 24900,
          "currency": "KRW"
        }
        """.utf8).write(to: receipts.appending(path: "2026-07-29.json"))
        try? Data("Notes the app wrote for itself.\n".utf8).write(to: documents.appending(path: "notes.txt"))
        try? Data((0 ..< 512).map { UInt8($0 % 251) }).write(to: documents.appending(path: "cache.bin"))
    }

    /// A small database for the SQLite panel, written once.
    private static func seedDatabase() {
        guard let library = FileManager.default.urls(for: .libraryDirectory, in: .userDomainMask).first else {
            return
        }
        let url = library.appending(path: "orders.sqlite")
        guard !FileManager.default.fileExists(atPath: url.path) else { return }

        var handle: OpaquePointer?
        guard sqlite3_open(url.path, &handle) == SQLITE_OK else { return }
        defer { sqlite3_close(handle) }
        sqlite3_exec(handle, """
        CREATE TABLE orders (id INTEGER PRIMARY KEY, item TEXT NOT NULL, total INTEGER, note TEXT);
        INSERT INTO orders (item, total, note) VALUES
          ('Coffee beans', 18900, NULL), ('Filter papers', 4500, 'gift'), ('Kettle', 59000, NULL);
        CREATE VIEW large_orders AS SELECT * FROM orders WHERE total >= 10000;
        """, nil, nil, nil)
    }

    /// One key of each shape a real app stores, plus a counter that actually counts.
    private static func seedPreferences() {
        let defaults = UserDefaults.standard
        defaults.set(defaults.integer(forKey: "example.launchCount") + 1, forKey: "example.launchCount")
        defaults.set(Date(), forKey: "example.lastLaunch")
        if defaults.object(forKey: "example.theme") == nil {
            defaults.set("system", forKey: "example.theme")
            defaults.set(true, forKey: "example.onboarding.seen")
            defaults.set(["swift", "debugging"], forKey: "example.interests")
            defaults.set(1.25, forKey: "example.playbackRate")
            defaults.set(Data((0 ..< 64).map { UInt8($0) }), forKey: "example.pushToken")
        }
        if defaults.object(forKey: "example.savedNetwork") == nil {
            // A Codable saved as Data, the way apps commonly store a model in defaults.
            defaults.set(try? PropertyListEncoder().encode(["ssid": "Office", "band": "5GHz"]), forKey: "example.savedNetwork")
        }
    }
}

struct RootView: View {
    var body: some View {
        TabView {
            ConnectionView()
                .tabItem { Label("Connection", systemImage: "cable.connector") }

            NetworkView()
                .tabItem { Label("Network", systemImage: "network") }

            ControlFixture()
                .tabItem { Label("Control", systemImage: "hand.tap") }

            AccessibilityFixture()
                .tabItem { Label("Accessibility", systemImage: "accessibility") }

            AboutView()
                .tabItem { Label("About", systemImage: "info.circle") }
        }
    }
}
