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

The installer carries `necto-cli.exe`, so the CLI is built first and staged where
Tauri's `externalBin` looks for it. `tauri.bundle.conf.json` adds that and the NSIS
hooks only for packaging, so `cargo run` and `cargo test` need neither.

```powershell
cd NectoWindows\src-tauri
cargo build --release --bin necto-cli
New-Item -ItemType Directory -Force binaries | Out-Null
Copy-Item target\release\necto-cli.exe binaries\necto-cli-x86_64-pc-windows-msvc.exe
npx @tauri-apps/cli@2 build --config tauri.bundle.conf.json
```

This writes an NSIS installer under `target\release\bundle\nsis\`. It installs for
the current user into `%LOCALAPPDATA%\Necto`, with `necto-cli.exe` beside the app,
and puts that folder on the user's PATH; uninstalling takes it off again
([windows/hooks.nsh](src-tauri/windows/hooks.nsh)). Open a new terminal after
installing. CI installs, calls and uninstalls each build with
[script/check-installer.ps1](script/check-installer.ps1).

## Release

Pushing a `windows-v<version>` tag builds, tests and checks the installer as above,
then publishes a GitHub prerelease with the installer, `necto-cli` and
`SHA256SUMS.txt`. The tag sets the version; `tauri.conf.json` is not edited.

```sh
git tag windows-v0.1.0
git push origin windows-v0.1.0
```

## Not yet

Plugin installation, desktop plugins, shell access, settings and localization of the
shell. `plugin.event` channels are ignored. iOS apps are read with the Mac app.
