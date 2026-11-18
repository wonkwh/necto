//
//  Copyright (c) 2026 Viva Republica, Inc.
//

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adb;
mod panels;
mod registry;
mod schema;
mod session;

use registry::Registry;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::http::{Response, StatusCode};
use tauri::{Manager, State};

const THEME: &str = include_str!("../../../WebPackages/Bridge/theme.css");

#[tauri::command]
fn targets(registry: State<'_, Arc<Registry>>) -> Value {
    registry.targets()
}

#[tauri::command]
async fn bridge(registry: State<'_, Arc<Registry>>, handle: String, plugin: String, message: Value) -> Result<Value, ()> {
    Ok(registry.request(&handle, &plugin, message).await)
}

/// Polls adb and keeps one connection per app endpoint. A failed connection is retried on the next poll.
async fn watch_adb(registry: Arc<Registry>) {
    let Some(adb) = adb::Adb::locate().map(Arc::new) else {
        eprintln!("necto: adb was not found. Set ANDROID_HOME or put adb on PATH.");
        return;
    };
    // Other tools share the server, so it is left running on quit.
    if let Err(error) = adb.start_server().await {
        eprintln!("necto: adb start-server failed: {error}");
    }
    adb.remove_stale_forwards().await;
    let connected: Arc<Mutex<HashSet<(String, String)>>> = Arc::default();
    loop {
        for serial in adb.devices().await.unwrap_or_default() {
            for (package, name) in adb.endpoints(&serial).await.unwrap_or_default() {
                let key = (serial.clone(), name.clone());
                if !connected.lock().unwrap().insert(key.clone()) {
                    continue;
                }
                let (adb, registry, connected, serial) = (adb.clone(), registry.clone(), connected.clone(), serial.clone());
                tauri::async_runtime::spawn(async move {
                    if let Ok(port) = adb.forward(&serial, &name).await {
                        match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                            Ok(stream) => {
                                let ended = registry.serve(serial.clone(), stream).await;
                                eprintln!("necto: {package} on {serial} ended: {ended:?}");
                            }
                            Err(error) => eprintln!("necto: {package} on {serial} refused: {error}"),
                        }
                        adb.remove_forward(&serial, port).await;
                    }
                    connected.lock().unwrap().remove(&key);
                });
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// `/theme.css`, or `/<targetHandle>/<pluginID>/<file>` from a fetched panel.
fn serve_scheme(registry: &Registry, path: &str) -> Response<Vec<u8>> {
    let found = if path == "/theme.css" {
        Some((THEME.as_bytes().to_vec(), panels::media_type(path)))
    } else {
        let mut parts = path.trim_start_matches('/').splitn(3, '/');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(handle), Some(plugin), file) => registry.panel(handle, plugin).and_then(|p| p.file(file.unwrap_or_default())),
            _ => None,
        }
    };
    let builder = Response::builder().header("Access-Control-Allow-Origin", "*");
    match found {
        Some((body, mime)) => builder.header("Content-Type", mime).body(body),
        None => builder.status(StatusCode::NOT_FOUND).body(Vec::new()),
    }
    .expect("static headers are valid")
}

fn main() {
    tauri::Builder::default()
        .register_asynchronous_uri_scheme_protocol("necto-plugin", |context, request, responder| {
            let registry = context.app_handle().state::<Arc<Registry>>().inner().clone();
            responder.respond(serve_scheme(&registry, request.uri().path()));
        })
        .setup(|app| {
            let registry = Registry::new(app.handle().clone());
            app.manage(registry.clone());
            tauri::async_runtime::spawn(watch_adb(registry));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![targets, bridge])
        .run(tauri::generate_context!())
        .expect("Necto failed to start");
}
