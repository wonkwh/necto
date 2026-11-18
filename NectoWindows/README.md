# Necto for Windows

A Tauri v2 host that connects to Android apps over adb and shows the panels they
carry. The design is in
[docs/superpowers/specs/2026-11-18-windows-client-design.md](../docs/superpowers/specs/2026-11-18-windows-client-design.md).
It also builds and runs on macOS, which is how it is checked against an emulator.

## Run

Needs Rust, and the Android SDK platform tools (`ANDROID_HOME`, the SDK's default
folder, or `adb` on PATH). On Windows, WebView2 ships with Windows 10 and 11.

```sh
cd NectoWindows/src-tauri
cargo run
```

Start an app that links the Necto Android SDK on a cabled phone or an emulator. It
appears in the sidebar within a few seconds.

## Package

```sh
cargo install tauri-cli --version "^2" --locked
cd NectoWindows/src-tauri
cargo tauri build
```

On Windows this produces NSIS and MSI installers under `target/release/bundle/`.

## Not yet

iOS over usbmuxd, plugin installation, desktop plugins, shell access, `necto-cli`,
settings and localization of the shell. `plugin.event` channels are ignored.
