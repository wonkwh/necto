//
//  Copyright (c) 2026 Viva Republica, Inc.
//

//! Runs the adb tool. An app's Necto socket is an abstract socket under a random name
//! that only `run-as` can read; Necto reads it and forwards a loopback port to it.

use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

/// The server on this machine, never one the environment names: that could be remote.
const SERVER: [&str; 4] = ["-H", "localhost", "-P", "5037"];

pub struct Adb {
    executable: PathBuf,
}

impl Adb {
    /// The adb in the Android SDK, falling back to one on PATH.
    pub fn locate() -> Option<Adb> {
        let name = if cfg!(windows) { "adb.exe" } else { "adb" };
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
        let default_sdk = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Android").join("Sdk"))
        } else if cfg!(target_os = "macos") {
            home.map(|h| h.join("Library/Android/sdk"))
        } else {
            home.map(|h| h.join("Android/Sdk"))
        };
        let roots = ["ANDROID_HOME", "ANDROID_SDK_ROOT"].iter().filter_map(std::env::var_os).map(PathBuf::from).chain(default_sdk);
        let in_sdk = roots.map(|root| root.join("platform-tools").join(name)).find(|p| p.is_file());
        let on_path = || std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file()));
        in_sdk.or_else(on_path).map(|executable| Adb { executable })
    }

    pub async fn start_server(&self) -> io::Result<()> {
        self.run(&["start-server"], 10).await.map(drop)
    }

    /// Authorized devices on a cable and real emulators. Wireless ADB is left out.
    pub async fn devices(&self) -> io::Result<Vec<String>> {
        let mut result = Vec::new();
        for serial in authorized_serials(&self.run(&["devices"], 5).await?) {
            if is_emulator_serial(&serial) {
                let qemu = self.run(&["-s", &serial, "shell", "getprop", "ro.kernel.qemu"], 5).await;
                if qemu.is_ok_and(|v| v.trim() == "1") {
                    result.push(serial);
                }
            } else if !is_wireless_serial(&serial) {
                result.push(serial);
            }
        }
        Ok(result)
    }

    /// `(package, endpoint name)` for each running third party app with a Necto socket.
    pub async fn endpoints(&self, serial: &str) -> io::Result<Vec<(String, String)>> {
        let packages = parse_packages(&self.run(&["-s", serial, "shell", "pm", "list", "packages", "-3"], 5).await?);
        let running = parse_processes(&self.run(&["-s", serial, "shell", "ps", "-A", "-o", "NAME="], 5).await?);
        let mut tasks = tokio::task::JoinSet::new();
        for package in packages.into_iter().filter(|p| running.contains(p)) {
            let command = self.command(&["-s", serial, "shell", "run-as", &package, "cat", "no_backup/necto-endpoint"]);
            tasks.spawn(async move {
                // run-as fails for an app that is not debuggable or has no file: that app is skipped.
                let text = output(command, 5).await.ok()?;
                endpoint_name(&text).map(|name| (package, name))
            });
        }
        Ok(tasks.join_all().await.into_iter().flatten().collect())
    }

    /// Forwards a free loopback port to the abstract socket `name`, returning the port.
    pub async fn forward(&self, serial: &str, name: &str) -> io::Result<u16> {
        if !is_endpoint_name(name) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a Necto endpoint name"));
        }
        let remote = format!("localabstract:{name}");
        let printed = self.run(&["-s", serial, "forward", "tcp:0", &remote], 5).await?;
        printed.trim().parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "adb printed no port"))
    }

    pub async fn remove_forward(&self, serial: &str, port: u16) {
        let _ = self.run(&["-s", serial, "forward", "--remove", &format!("tcp:{port}")], 5).await;
    }

    /// Forwards to Necto endpoints left by a client that did not quit cleanly.
    pub async fn remove_stale_forwards(&self) {
        let Ok(list) = self.run(&["forward", "--list"], 5).await else { return };
        for (serial, port) in necto_forwards(&list) {
            self.remove_forward(&serial, port).await;
        }
    }

    fn command(&self, arguments: &[&str]) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.executable);
        command.args(SERVER).args(arguments).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
        for key in ["ADB_SERVER_SOCKET", "ANDROID_ADB_SERVER_ADDRESS", "ANDROID_ADB_SERVER_PORT"] {
            command.env_remove(key);
        }
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash per call.
        command
    }

    /// A failed command is an error: an empty answer from a device that left must not read as no apps.
    async fn run(&self, arguments: &[&str], seconds: u64) -> io::Result<String> {
        output(self.command(arguments), seconds).await
    }
}

async fn output(mut command: tokio::process::Command, seconds: u64) -> io::Result<String> {
    let result = tokio::time::timeout(Duration::from_secs(seconds), command.output())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "adb timed out"))??;
    if !result.status.success() {
        return Err(io::Error::other("adb failed"));
    }
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}

pub fn authorized_serials(output: &str) -> Vec<String> {
    output
        .lines()
        .skip(1)
        .filter_map(|line| match line.split_whitespace().collect::<Vec<_>>()[..] {
            [serial, "device"] => Some(serial.to_string()),
            _ => None,
        })
        .collect()
}

fn is_emulator_serial(serial: &str) -> bool {
    serial.strip_prefix("emulator-").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// `host:port` from `adb connect`, or an mDNS name from wireless debugging.
fn is_wireless_serial(serial: &str) -> bool {
    serial.contains(':') || serial.contains("._adb-tls-")
}

/// Lines of `<serial> tcp:<port> localabstract:necto.<hex>`.
pub fn necto_forwards(output: &str) -> Vec<(String, u16)> {
    output
        .lines()
        .filter_map(|line| match line.split(' ').collect::<Vec<_>>()[..] {
            [serial, local, remote] if remote.strip_prefix("localabstract:").is_some_and(is_endpoint_name) => {
                Some((serial.to_string(), local.strip_prefix("tcp:")?.parse().ok()?))
            }
            _ => None,
        })
        .collect()
}

pub fn parse_packages(output: &str) -> Vec<String> {
    output.lines().filter_map(|l| l.strip_prefix("package:")).map(str::trim).filter(|n| is_package_name(n)).map(String::from).collect()
}

/// Process names, with a `:service` suffix dropped to leave the package.
pub fn parse_processes(output: &str) -> std::collections::HashSet<String> {
    output.lines().filter_map(|l| l.split(':').next()).map(str::trim).filter(|n| is_package_name(n)).map(String::from).collect()
}

/// One line and nothing else. A second line or a stray `\r` fails rather than being read past.
pub fn endpoint_name(contents: &str) -> Option<String> {
    let name = contents.strip_suffix('\n').unwrap_or(contents);
    is_endpoint_name(name).then(|| name.to_string())
}

fn is_endpoint_name(name: &str) -> bool {
    name.strip_prefix("necto.").is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

fn is_package_name(name: &str) -> bool {
    let segment = |s: &str| s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let parts: Vec<_> = name.split('.').collect();
    parts.len() >= 2 && parts.iter().all(|p| segment(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_adb_output() {
        let devices = "List of devices attached\nemulator-5554\tdevice\nR58M\tunauthorized\n10.0.0.2:5555\tdevice\n\n";
        assert_eq!(authorized_serials(devices), ["emulator-5554", "10.0.0.2:5555"]);
        assert!(is_wireless_serial("10.0.0.2:5555") && !is_wireless_serial("R58M"));
        assert!(is_emulator_serial("emulator-5554") && !is_emulator_serial("emulator-x"));

        assert_eq!(parse_packages("package:im.toss.sample\npackage:bad\n"), ["im.toss.sample"]);
        assert!(parse_processes("im.toss.sample:remote\nsystem_server\n").contains("im.toss.sample"));

        let name = format!("necto.{}", "a".repeat(64));
        assert_eq!(endpoint_name(&format!("{name}\n")), Some(name.clone()));
        assert_eq!(endpoint_name(&format!("{name}\r\n")), None);
        assert_eq!(endpoint_name(&format!("{name}\nx")), None);
        assert_eq!(endpoint_name("necto.ABC"), None);

        let forwards = format!("emulator-5554 tcp:64462 localabstract:{name}\nemulator-5554 tcp:64368 tcp:38629\n");
        assert_eq!(necto_forwards(&forwards), [("emulator-5554".to_string(), 64462)]);
    }
}
