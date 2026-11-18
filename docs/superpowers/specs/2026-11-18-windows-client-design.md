# Windows client (Tauri v2) — design

## Goal

A desktop host for Necto that runs on Windows, written in Rust with Tauri v2. The
first cycle connects to Android apps over adb and shows the panels those apps carry.
Existing web panels and `@necto/bridge` run unchanged.

## Scope

In:

- adb discovery, endpoint read through `run-as`, `adb forward`
- Protocol v1: framing, hello and ack, `plugin.register`, `plugin.invoke`,
  `plugin.result`, `plugin.cancel`
- Operation routing for `necto.device.*` bindings, with input validation against the
  manifest the panel carries
- Panel fetch through `necto.device.plugins.assets`, verified against the registered
  stamp, and served from memory
- A shell window: devices, apps and their plugins in a sidebar, the selected panel on
  the right

Out, for later cycles: iOS over usbmuxd, the loopback simulator path, plugin
installation and updates, desktop plugins and `necto.desktop.*` bridges, shell
policy, `necto-cli` and the control socket, settings, localization, `plugin.event`
channels.

## Layout

```text
NectoWindows/
  src-tauri/
    Cargo.toml, tauri.conf.json, capabilities/
    src/
      main.rs       Tauri setup, commands, scheme registration
      adb.rs        adb process calls and output parsing
      session.rs    Framing, handshake, envelope read and write
      registry.rs   Connected apps, registered catalogs, request pairing
      schema.rs     The JSON Schema subset NectoJSONSchema checks
      panels.rs     Panel archives in memory, hash check, scheme responses
  ui/
    index.html, shell.js, shell.css   The shell; colours and sizes from
                                       WebPackages/Bridge/theme.css
```

## Panel hosting

Panels run in `<iframe sandbox="allow-scripts">` inside the shell's webview, loaded
from the `necto-plugin` custom scheme. The sandbox gives each panel an opaque origin,
so it cannot reach Tauri IPC or the shell's DOM.

When the scheme serves a panel's `index.html`, it inserts a shim script before any
other script. The shim defines `window.webkit.messageHandlers.necto.postMessage`,
which posts the request to `window.parent` with a sequence number and resolves with
the matching reply. Stream messages from the parent call the existing
`__nectoBridgeDeliver`, `__nectoBridgeStreamError` and `__nectoBridgeStreamEnd`.

The shell maps each iframe's `contentWindow` to the panel it loaded, and only that
mapping decides which plugin and target a request comes from. A message from an
unknown source is dropped.

## Data flow

1. Every two seconds `adb.rs` lists local devices (USB or a real emulator), reads
   `no_backup/necto-endpoint` from running third party apps, and for each new endpoint
   runs `adb forward tcp:0 localabstract:<name>`, which prints the local port.
2. `session.rs` connects to `127.0.0.1:<port>`, reads the hello, replies with an ack
   and rejects any protocol version other than 1.
3. Each `plugin.register` with a panel stamp makes `panels.rs` invoke
   `necto.device.plugins.assets`, check the archive hash against the stamp and keep the
   files. The plugin then appears in the sidebar.
4. A panel request (`context`, `ready`, `invoke`, `subscribe`, `unsubscribe`) goes
   iframe → shell → Tauri command → `registry.rs`. `invoke` and `subscribe` look up the
   operation in the panel's manifest, validate the input, and send `plugin.invoke`
   with a new request id.
5. `plugin.result` messages are paired by request id. A single result answers the
   command. Stream results go to the shell as Tauri events and on to the iframe.

## Errors

- A closed connection ends its pending requests and streams with `notConnected`, and
  removes its apps and panels from the sidebar.
- A frame longer than 32 MiB closes the session.
- A handshake that does not finish within 10 seconds closes the connection.
- A panel whose archive does not match its stamp is not shown, and the failure is
  logged to stderr.
- Input that fails the schema is refused before it reaches the app.

## Known differences from the Mac app

- Forwarding uses loopback TCP, not a private Unix socket, because adb on Windows
  cannot rely on `localfilesystem:`. Another local process could connect to the port
  while it is open.
- `targetHandle` is an opaque random string per connection, as on the Mac. Raw serials
  never reach a panel.

## Testing

- Rust unit tests: frame encoding and limit, adb output parsing, endpoint name check,
  schema validation, archive hash.
- Manual check on macOS with an Android emulator running `SDK/Android/sample`: the
  Network and Events panels appear and show data. Windows is checked on a Windows PC
  with the same emulator or a USB phone.
