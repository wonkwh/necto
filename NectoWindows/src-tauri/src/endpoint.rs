//
//  Copyright (c) 2026 Viva Republica, Inc.
//

//! Where the control server listens, shared by the app and `necto-cli`.

/// A pipe per user: pipe names are machine wide, so the name keeps two accounts apart
/// and the pipe's DACL keeps one out of the other's.
#[cfg(windows)]
pub fn control_pipe() -> String {
    let user: String = std::env::var("USERNAME").unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    format!(r"\\.\pipe\necto-control-{user}")
}

/// Development on macOS and Linux. Kept apart from the Mac app's own socket.
#[cfg(unix)]
pub fn control_socket() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    home.join(".necto-windows").join("necto.sock")
}
