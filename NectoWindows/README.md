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

## Command line

`necto-cli` talks to the running app, with the commands and output of the Mac
`necto-cli` ([docs/control-socket.md](../docs/control-socket.md)). Device IDs are adb
serials.

```sh
cargo run --bin necto-cli -- device list
cargo run --bin necto-cli -- plugin list --device emulator-5554 --app com.example.app
cargo run --bin necto-cli -- plugin subscribe network-logger records.observe \
  --device emulator-5554 --app com.example.app --limit 10
```

`necto-cli skills install --codex --claude` writes the agent skill for this CLI
([resources/skills/necto/SKILL.md](src-tauri/resources/skills/necto/SKILL.md)) to
`~/.agents/skills/necto` and `~/.claude/skills/necto`, without a running Necto. A
changed copy is replaced only with `--force`.

On Windows it connects through the named pipe `\\.\pipe\necto-control-<user>`, which
only its owner can open; on macOS and Linux, during development, through
`~/.necto-windows/necto.sock`. Installing, deleting and `shell run` are not available.

## Package

```sh
cargo install tauri-cli --version "^2" --locked
cd NectoWindows/src-tauri
cargo tauri build
```

On Windows this produces NSIS and MSI installers under `target/release/bundle/`, and
`target/release/necto-cli.exe` beside them; the installers do not carry the CLI.

## Not yet

Plugin installation, desktop plugins, shell access, settings and localization of the
shell. `plugin.event` channels are ignored. iOS apps are read with the Mac app.
