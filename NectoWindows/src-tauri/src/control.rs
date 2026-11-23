//
//  Copyright (c) 2026 Viva Republica, Inc.
//

//! The control server `necto-cli` talks to. Requests and responses follow
//! docs/control-socket.md over the same length prefixed framing as every Necto connection.

use crate::endpoint;
use crate::registry::{bridge_error, Registry, Sink};
use crate::session;
use serde_json::{json, Value};
use std::io;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

pub async fn listen(registry: Arc<Registry>) {
    if let Err(error) = run(registry).await {
        eprintln!("necto: the control server stopped: {error}");
    }
}

#[cfg(unix)]
async fn run(registry: Arc<Registry>) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = endpoint::control_socket();
    let folder = path.parent().expect("the socket lives in a folder");
    std::fs::create_dir_all(folder)?;
    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))?;
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    for number in 1u64.. {
        let (stream, _) = listener.accept().await?;
        tauri::async_runtime::spawn(connection(registry.clone(), stream, number));
    }
    Ok(())
}

#[cfg(windows)]
async fn run(registry: Arc<Registry>) -> io::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

    let name = endpoint::control_pipe();
    let descriptor = owner_only_descriptor()?;
    let create = |first: bool| {
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor as *mut _,
            bInheritHandle: 0,
        };
        // SAFETY: the attributes and the descriptor they point at outlive the call.
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(&name, &mut attributes as *mut _ as *mut _)
        }
    };
    // A first instance that already exists is another Necto, or someone squatting the name.
    let mut server = create(true)?;
    for number in 1u64.. {
        server.connect().await?;
        let connected = std::mem::replace(&mut server, create(false)?);
        tauri::async_runtime::spawn(connection(registry.clone(), connected, number));
    }
    Ok(())
}

/// Full access for the pipe's owner and the system, nobody else. The default DACL lets
/// every account read.
// ponytail: OWNER RIGHTS is the Administrators group when Necto runs elevated; use the token's user SID if that matters.
#[cfg(windows)]
fn owner_only_descriptor() -> io::Result<usize> {
    use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    let sddl: Vec<u16> = "D:P(A;;GA;;;OW)(A;;GA;;;SY)".encode_utf16().chain([0]).collect();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: a NUL terminated string in, an allocation that lives for the process out.
    let converted = unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, std::ptr::null_mut()) };
    if converted == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(descriptor as usize)
}

/// One CLI connection. Requests may interleave; closing it cancels the streams it started.
async fn connection(registry: Arc<Registry>, stream: impl AsyncRead + AsyncWrite + Send + 'static, number: u64) {
    let (mut reader, mut writer) = tokio::io::split(stream);
    let (out, mut outgoing) = mpsc::unbounded_channel::<Value>();
    let writing = tauri::async_runtime::spawn(async move {
        while let Some(response) = outgoing.recv().await {
            let Ok(frame) = session::frame(&serde_json::to_vec(&response).unwrap_or_default()) else { continue };
            if writer.write_all(&frame).await.is_err() {
                break;
            }
        }
    });
    let prefix = format!("control:{number}:");
    while let Ok(bytes) = session::read_frame(&mut reader).await {
        let Ok(request) = serde_json::from_slice::<Value>(&bytes) else { break };
        let (registry, out, prefix) = (registry.clone(), out.clone(), prefix.clone());
        tauri::async_runtime::spawn(async move {
            let id = request["id"].clone();
            match answer(&registry, &request, &out, &prefix).await {
                Ok(Some(value)) => drop(out.send(json!({ "id": id, "kind": "result", "value": value }))),
                Ok(None) => {}
                Err(error) => drop(out.send(json!({ "id": id, "kind": "error", "error": error }))),
            }
        });
    }
    registry.cancel_streams(|key| key.starts_with(&prefix));
    writing.abort();
}

/// The scope a request names: `None` for `desktop`, an app's handle otherwise.
fn scope(registry: &Registry, request: &Value) -> Result<Option<String>, Value> {
    let (device, app) = (request["device"].as_str().filter(|s| !s.is_empty()), request["app"].as_str().filter(|s| !s.is_empty()));
    if request["desktop"] == true {
        if device.is_some() || app.is_some() {
            return Err(bridge_error("INVALID_INPUT", "--desktop cannot be combined with --app or --device.", None));
        }
        return Ok(None);
    }
    let (Some(device), Some(app)) = (device, app) else {
        let candidates: Vec<String> = registry.control_targets()["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| format!("{} / {}", t["deviceID"].as_str().unwrap_or("?"), t["appBundleID"].as_str().unwrap_or("?")))
            .collect();
        let candidates = if candidates.is_empty() { "none".to_string() } else { candidates.join(", ") };
        return Err(bridge_error("INVALID_INPUT", format!("Pass both --device and --app, or --desktop. Run necto device list. Connected targets: {candidates}"), None));
    };
    registry.control_target(device, app).map(Some)
}

/// `Ok(None)` when the answer is streamed through `out` instead of one result.
async fn answer(registry: &Arc<Registry>, request: &Value, out: &mpsc::UnboundedSender<Value>, prefix: &str) -> Result<Option<Value>, Value> {
    let plugin = request["pluginID"].as_str();
    let operation = request["operationID"].as_str();
    let input = request.get("input").filter(|i| !i.is_null()).cloned().unwrap_or_else(|| json!({}));
    let key = format!("{prefix}{}", request["id"]);
    let named = || match (plugin, operation) {
        (Some(plugin), Some(operation)) => Ok((plugin, operation)),
        _ => Err(bridge_error("INVALID_INPUT", "pluginID and operationID are required", None)),
    };
    // This Necto installs no desktop plugins, so the desktop scope is always empty.
    let device_scope = |handle: Option<String>, plugin: &str| {
        handle.ok_or_else(|| bridge_error("OPERATION_NOT_FOUND", format!("No plugin '{plugin}' in this scope. Run plugin list with the same target flags."), None))
    };

    match request["kind"].as_str().unwrap_or_default() {
        "targets" => Ok(Some(registry.control_targets())),
        "plugins" => registry.control_plugins(scope(registry, request)?.as_deref(), plugin, operation).map(Some),
        "invoke" => {
            let (plugin, operation) = named()?;
            let handle = device_scope(scope(registry, request)?, plugin)?;
            registry.invoke(&handle, plugin, operation, input).await.map(Some)
        }
        "subscribe" => {
            let (plugin, operation) = named()?;
            let handle = device_scope(scope(registry, request)?, plugin)?;
            let (id, out) = (request["id"].clone(), out.clone());
            let sink: Sink = Arc::new(move |kind, payload| {
                let response = match kind {
                    "event" => json!({ "id": id, "kind": "event", "value": payload }),
                    "end" => json!({ "id": id, "kind": "end" }),
                    _ => json!({ "id": id, "kind": "error", "error": payload }),
                };
                let _ = out.send(response);
            });
            registry.subscribe(&handle, plugin, operation, input, key, sink).map(|_| None)
        }
        "cancel" => {
            registry.cancel_streams(|k| k == key);
            let _ = out.send(json!({ "id": request["id"], "kind": "end" }));
            Ok(None)
        }
        kind @ ("installPlugin" | "deletePlugin") => {
            Err(bridge_error("OPERATION_UNAVAILABLE", format!("'{kind}' is not available in Necto for Windows yet."), None))
        }
        other => Err(bridge_error("INVALID_INPUT", format!("Unknown request kind '{other}'"), None)),
    }
}
