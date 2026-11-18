//
//  Copyright (c) 2026 Viva Republica, Inc.
//

//! Connected apps, the contracts they registered, and the calls panels and the CLI make
//! against them. Both callers go through the same resolve and validation.

use crate::panels::Panel;
use crate::schema;
use crate::session::{self, Hello};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

const PANEL_FETCH: (&str, i64) = ("necto.device.plugins.assets", 1);

/// Receives a stream's `event`, `error` and `end`, in that order of possibility.
pub type Sink = Arc<dyn Fn(&str, Value) + Send + Sync>;

pub struct Registry {
    shell: AppHandle,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// By `targetHandle`, an opaque id issued per connection. Raw serials never reach a panel.
    apps: HashMap<String, App>,
    pending: HashMap<String, Pending>,
    next_subscription: u64,
}

struct App {
    hello: Hello,
    serial: String,
    send: mpsc::UnboundedSender<Vec<u8>>,
    /// Bridge descriptors by plugin id, as registered.
    catalogs: HashMap<String, Vec<Value>>,
    panels: HashMap<String, Arc<Panel>>,
}

enum Pending {
    Once { handle: String, reply: oneshot::Sender<Result<Value, Value>> },
    /// `key` names the stream for its owner: a panel subscription or a control request.
    Stream { handle: String, key: String, operation: Value, sink: Sink },
}

pub fn bridge_error(code: &str, message: impl Into<String>, operation: Option<&str>) -> Value {
    let mut error = json!({ "code": code, "message": message.into() });
    if let Some(operation) = operation {
        error["operationID"] = json!(operation);
    }
    error
}

fn input_of(message: &Value) -> Value {
    message.get("input").filter(|i| !i.is_null()).cloned().unwrap_or_else(|| json!({}))
}

impl Registry {
    pub fn new(shell: AppHandle) -> Arc<Registry> {
        Arc::new(Registry { shell, state: Mutex::default() })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn changed(&self) {
        let _ = self.shell.emit("necto://targets", ());
    }

    /// Runs one connection from handshake to close.
    pub async fn serve(self: &Arc<Self>, serial: String, mut stream: TcpStream) -> io::Result<()> {
        let hello = session::handshake(&mut stream).await?;
        let (mut reader, mut writer) = stream.into_split();
        let (send, mut outgoing) = mpsc::unbounded_channel::<Vec<u8>>();
        let writing = tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                let Ok(frame) = session::frame(&message) else { continue };
                if writer.write_all(&frame).await.is_err() {
                    break;
                }
            }
        });
        let handle = uuid::Uuid::new_v4().to_string();
        let app = App { hello, serial, send, catalogs: HashMap::new(), panels: HashMap::new() };
        self.state().apps.insert(handle.clone(), app);
        self.changed();

        let ended = loop {
            let bytes = match session::read_frame(&mut reader).await {
                Ok(bytes) => bytes,
                Err(error) => break error,
            };
            let Ok(envelope) = serde_json::from_slice::<Value>(&bytes) else { continue };
            let payload = envelope["payload"].clone();
            match envelope["type"].as_str() {
                Some("plugin.register") => self.registered(&handle, payload),
                Some("plugin.result") => self.result(&handle, payload),
                // ponytail: plugin.event channels have no host adapter yet.
                _ => {}
            }
        };
        writing.abort();
        self.disconnected(&handle);
        Err(ended)
    }

    fn send(&self, handle: &str, kind: &str, payload: Value) -> bool {
        self.state().apps.get(handle).is_some_and(|app| app.send.send(session::envelope(kind, payload)).is_ok())
    }

    fn registered(self: &Arc<Self>, handle: &str, registration: Value) {
        let Some(plugin) = registration["pluginID"].as_str().map(String::from) else { return };
        let bridges = registration["catalog"]["bridges"].as_array().cloned().unwrap_or_default();
        let stamp = registration["panel"].clone();
        {
            let mut state = self.state();
            let Some(app) = state.apps.get_mut(handle) else { return };
            // An empty registration is how a plugin is taken away.
            if bridges.is_empty() && stamp.is_null() {
                app.catalogs.remove(&plugin);
                app.panels.remove(&plugin);
                drop(state);
                self.changed();
                return;
            }
            app.catalogs.insert(plugin.clone(), bridges);
        }
        if stamp.is_null() {
            return;
        }
        let registry = Arc::clone(self);
        let handle = handle.to_string();
        tauri::async_runtime::spawn(async move {
            let fetched = registry.call(&handle, PANEL_FETCH.0, PANEL_FETCH.1, json!({ "pluginID": plugin }), 30_000).await;
            let panel = fetched.map_err(|e| e.to_string()).and_then(|archive| Panel::decode(&archive, &stamp));
            match panel {
                Ok(panel) => {
                    if let Some(app) = registry.state().apps.get_mut(&handle) {
                        app.panels.insert(plugin, Arc::new(panel));
                    }
                    registry.changed();
                }
                Err(error) => eprintln!("necto: the panel of '{plugin}' could not be fetched: {error}"),
            }
        });
    }

    /// Sends one `once` invocation and waits for its single result.
    async fn call(&self, handle: &str, name: &str, version: i64, input: Value, timeout_ms: u64) -> Result<Value, Value> {
        let request = uuid::Uuid::new_v4().to_string();
        let (reply, answer) = oneshot::channel();
        self.state().pending.insert(request.clone(), Pending::Once { handle: handle.into(), reply });
        let invocation = json!({ "requestID": request, "name": name, "version": version, "kind": "once", "input": input });
        if !self.send(handle, "plugin.invoke", invocation) {
            self.state().pending.remove(&request);
            return Err(bridge_error("TARGET_DISCONNECTED", "The app is not connected", None));
        }
        let waited = if timeout_ms == 0 {
            Ok(answer.await)
        } else {
            tokio::time::timeout(Duration::from_millis(timeout_ms), answer).await
        };
        match waited {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(bridge_error("TARGET_DISCONNECTED", "The app disconnected before answering", None)),
            Err(_) => {
                self.state().pending.remove(&request);
                self.send(handle, "plugin.cancel", json!({ "requestID": request }));
                Err(bridge_error("TIMEOUT", "The app did not answer in time", None))
            }
        }
    }

    fn result(&self, handle: &str, result: Value) {
        let Some(request) = result["requestID"].as_str() else { return };
        let error = result.get("error").filter(|e| !e.is_null()).cloned();
        let output = result.get("output").filter(|o| !o.is_null()).cloned();
        let is_final = result["isFinal"].as_bool().unwrap_or(true);

        let mut state = self.state();
        let owned = match state.pending.get(request) {
            Some(Pending::Once { handle: h, .. } | Pending::Stream { handle: h, .. }) => h == handle,
            None => false,
        };
        if !owned {
            return;
        }
        if let Some(Pending::Stream { operation, sink, .. }) = state.pending.get(request) {
            let (operation, sink) = (operation.clone(), sink.clone());
            if let (Some(event), false) = (output, is_final) {
                let Some(invalid) = schema::validate(&event, &operation["outputSchema"]) else {
                    drop(state);
                    return sink("event", event);
                };
                state.pending.remove(request);
                drop(state);
                self.send(handle, "plugin.cancel", json!({ "requestID": request }));
                return sink("error", bridge_error("INVALID_OUTPUT", invalid, operation["id"].as_str()));
            }
            state.pending.remove(request);
            drop(state);
            return match error {
                Some(error) => sink("error", error),
                None => sink("end", Value::Null),
            };
        }
        if let Some(Pending::Once { reply, .. }) = state.pending.remove(request) {
            let _ = reply.send(match error {
                Some(error) => Err(error),
                None => Ok(output.unwrap_or_else(|| json!({}))),
            });
        }
    }

    fn disconnected(&self, handle: &str) {
        let mut state = self.state();
        let Some(app) = state.apps.remove(handle) else { return };
        let message = format!("'{}' disconnected before answering", app.hello.app_bundle_id);
        let requests: Vec<String> = state
            .pending
            .iter()
            .filter(|(_, p)| matches!(p, Pending::Once { handle: h, .. } | Pending::Stream { handle: h, .. } if h == handle))
            .map(|(id, _)| id.clone())
            .collect();
        let ended: Vec<Pending> = requests.iter().filter_map(|id| state.pending.remove(id)).collect();
        drop(state);
        for pending in ended {
            let error = bridge_error("TARGET_DISCONNECTED", message.clone(), None);
            match pending {
                Pending::Once { reply, .. } => drop(reply.send(Err(error))),
                Pending::Stream { sink, .. } => sink("error", error),
            }
        }
        self.changed();
    }

    // MARK: Shell queries

    /// Every connected app with the panels it carries, for the sidebar.
    pub fn targets(&self) -> Value {
        let state = self.state();
        let mut apps: Vec<Value> = state
            .apps
            .iter()
            .map(|(handle, app)| {
                let mut plugins: Vec<Value> = app.panels.iter().map(|(id, p)| json!({ "id": id, "name": p.name() })).collect();
                plugins.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                json!({
                    "handle": handle,
                    "serial": app.serial,
                    "deviceName": app.hello.device_name,
                    "osVersion": app.hello.os_version,
                    "appName": app.hello.app_name,
                    "appBundleID": app.hello.app_bundle_id,
                    "appVersion": app.hello.app_version,
                    "appIcon": app.hello.app_icon,
                    "plugins": plugins,
                })
            })
            .collect();
        apps.sort_by(|a, b| (a["deviceName"].as_str(), a["appName"].as_str()).cmp(&(b["deviceName"].as_str(), b["appName"].as_str())));
        Value::Array(apps)
    }

    pub fn panel(&self, handle: &str, plugin: &str) -> Option<Arc<Panel>> {
        self.state().apps.get(handle)?.panels.get(plugin).cloned()
    }

    // MARK: Panel requests

    /// Answers one request from a panel with `{ok, value}` or `{ok: false, error}`.
    pub async fn request(self: &Arc<Self>, handle: &str, plugin: &str, message: Value) -> Value {
        let operation = message["operationID"].as_str();
        let result = match message["type"].as_str().unwrap_or_default() {
            "context" => self.context(handle, plugin),
            "ready" => Ok(Value::Null),
            "invoke" => match operation {
                Some(operation) => self.invoke(handle, plugin, operation, input_of(&message)).await,
                None => Err(bridge_error("INVALID_INPUT", "operationID is missing", None)),
            },
            "subscribe" => match operation {
                Some(operation) => {
                    let subscription = {
                        let mut state = self.state();
                        state.next_subscription += 1;
                        format!("s{}", state.next_subscription)
                    };
                    let (shell, handle_, plugin_, subscription_) = (self.shell.clone(), handle.to_string(), plugin.to_string(), subscription.clone());
                    let sink: Sink = Arc::new(move |kind, payload| {
                        let message = json!({ "handle": handle_, "plugin": plugin_, "subscriptionID": subscription_, "kind": kind, "payload": payload });
                        let _ = shell.emit("necto://stream", message);
                    });
                    let key = format!("panel:{handle}:{plugin}:{subscription}");
                    self.subscribe(handle, plugin, operation, input_of(&message), key, sink).map(|_| json!(subscription))
                }
                None => Err(bridge_error("INVALID_INPUT", "operationID is missing", None)),
            },
            "unsubscribe" => {
                let subscription = message["subscriptionID"].as_str().unwrap_or_default();
                self.cancel_streams(|key| key == format!("panel:{handle}:{plugin}:{subscription}"));
                Ok(Value::Null)
            }
            other => Err(bridge_error("OPERATION_NOT_FOUND", format!("Unknown request '{other}'"), None)),
        };
        match result {
            Ok(value) => json!({ "ok": true, "value": value }),
            Err(error) => json!({ "ok": false, "error": error }),
        }
    }

    /// Why an operation cannot run against this app, or `None` when it can.
    fn unavailable(app: &App, operation: &Value) -> Option<(&'static str, String)> {
        let binding = &operation["binding"];
        let name = binding["name"].as_str().unwrap_or_default();
        if !name.starts_with("necto.device.") {
            return Some(("OPERATION_UNAVAILABLE", "This Necto answers device bridges only".into()));
        }
        let providers: Vec<&Value> = app
            .catalogs
            .values()
            .flatten()
            .filter(|d| d["binding"]["name"] == binding["name"] && d["binding"]["version"] == binding["version"])
            .collect();
        let [provider] = providers[..] else {
            return Some(("OPERATION_UNAVAILABLE", "The selected app does not provide this operation".into()));
        };
        if provider["kind"] != operation["kind"] {
            let (has, declared) = (provider["kind"].as_str().unwrap_or("?"), operation["kind"].as_str().unwrap_or("?"));
            return Some(("OPERATION_UNAVAILABLE", format!("The provider for '{name}' is a {has}, the manifest declares a {declared}")));
        }
        None
    }

    fn context(&self, handle: &str, plugin: &str) -> Result<Value, Value> {
        let state = self.state();
        let app = state.apps.get(handle).ok_or_else(|| bridge_error("TARGET_DISCONNECTED", "The app is not connected", None))?;
        let panel = app.panels.get(plugin).ok_or_else(|| bridge_error("OPERATION_NOT_FOUND", format!("Plugin '{plugin}' is not installed"), None))?;
        let operations: Vec<Value> = panel
            .operations()
            .map(|op| {
                let failure = Self::unavailable(app, op);
                let mut entry = json!({ "id": op["id"], "kind": op["kind"], "available": failure.is_none() });
                if let Some((_, reason)) = failure {
                    entry["unavailableReason"] = json!(reason);
                }
                entry
            })
            .collect();
        Ok(json!({
            "protocolVersion": session::PROTOCOL_VERSION,
            "pluginID": plugin,
            "pluginVersion": panel.manifest["version"],
            "sourceIdentity": format!("device:{}", app.hello.app_bundle_id),
            "operations": operations,
            "target": {
                "targetHandle": handle,
                "deviceID": handle,
                "appBundleID": app.hello.app_bundle_id,
                "name": app.hello.device_name,
                "appName": app.hello.app_name,
            },
        }))
    }

    /// The manifest operation behind a call, checked for kind, availability and input.
    fn resolve(&self, handle: &str, plugin: &str, operation_id: &str, input: &Value, kind: &str) -> Result<Value, Value> {
        let fail = |code: &str, text: String| bridge_error(code, text, Some(operation_id));
        let state = self.state();
        let app = state.apps.get(handle).ok_or_else(|| fail("TARGET_DISCONNECTED", "The app is not connected".into()))?;
        let panel = app.panels.get(plugin).ok_or_else(|| fail("OPERATION_NOT_FOUND", format!("Plugin '{plugin}' is not installed")))?;
        let operation = panel.operation(operation_id).ok_or_else(|| fail("OPERATION_NOT_FOUND", format!("Plugin '{plugin}' declares no operation '{operation_id}'")))?;
        if operation["kind"] != kind {
            let declared = operation["kind"].as_str().unwrap_or("?");
            return Err(fail("OPERATION_UNAVAILABLE", format!("A {declared} operation cannot be called this way")));
        }
        if let Some((code, reason)) = Self::unavailable(app, operation) {
            return Err(fail(code, reason));
        }
        if let Some(failure) = schema::validate(input, &operation["inputSchema"]) {
            return Err(fail("INVALID_INPUT", failure));
        }
        Ok(operation.clone())
    }

    pub async fn invoke(&self, handle: &str, plugin: &str, operation_id: &str, input: Value) -> Result<Value, Value> {
        let operation = self.resolve(handle, plugin, operation_id, &input, "once")?;
        let binding = &operation["binding"];
        let timeout = operation["timeoutMs"].as_u64().unwrap_or(0);
        let name = binding["name"].as_str().unwrap_or_default();
        let output = self.call(handle, name, binding["version"].as_i64().unwrap_or(1), input, timeout).await.map_err(|mut error| {
            error["operationID"] = json!(operation_id);
            error
        })?;
        match schema::validate(&output, &operation["outputSchema"]) {
            None => Ok(output),
            Some(failure) => Err(bridge_error("INVALID_OUTPUT", failure, Some(operation_id))),
        }
    }

    /// Starts a stream whose messages go to `sink` until it ends or `key` is cancelled.
    pub fn subscribe(&self, handle: &str, plugin: &str, operation_id: &str, input: Value, key: String, sink: Sink) -> Result<(), Value> {
        let operation = self.resolve(handle, plugin, operation_id, &input, "stream")?;
        let binding = operation["binding"].clone();
        let request = uuid::Uuid::new_v4().to_string();
        let pending = Pending::Stream { handle: handle.into(), key, operation, sink };
        self.state().pending.insert(request.clone(), pending);
        let invocation = json!({ "requestID": request, "name": binding["name"], "version": binding["version"], "kind": "stream", "input": input });
        if !self.send(handle, "plugin.invoke", invocation) {
            self.state().pending.remove(&request);
            return Err(bridge_error("TARGET_DISCONNECTED", "The app is not connected", Some(operation_id)));
        }
        Ok(())
    }

    /// Stops every stream whose key matches, telling each app to stop too. The sinks hear nothing.
    pub fn cancel_streams(&self, matches: impl Fn(&str) -> bool) {
        let stopped: Vec<(String, String)> = {
            let mut state = self.state();
            let ids: Vec<String> = state
                .pending
                .iter()
                .filter(|(_, p)| matches!(p, Pending::Stream { key, .. } if matches(key)))
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| match state.pending.remove(&id) {
                    Some(Pending::Stream { handle, .. }) => Some((handle, id)),
                    _ => None,
                })
                .collect()
        };
        for (handle, request) in stopped {
            self.send(&handle, "plugin.cancel", json!({ "requestID": request }));
        }
    }

    // MARK: Control queries, in the shape docs/control-socket.md gives them

    /// `deviceID` is the adb serial: the CLI is the user's own tool, not a plugin.
    pub fn control_targets(&self) -> Value {
        let state = self.state();
        let mut targets: Vec<Value> = state
            .apps
            .values()
            .map(|app| {
                json!({
                    "appBundleID": app.hello.app_bundle_id,
                    "appName": app.hello.app_name,
                    "deviceID": app.serial,
                    "deviceName": app.hello.device_name,
                })
            })
            .collect();
        targets.sort_by_key(|t| (t["deviceID"].as_str().unwrap_or_default().to_string(), t["appBundleID"].as_str().unwrap_or_default().to_string()));
        json!({ "targets": targets })
    }

    /// The handle of the one app `app` on device `device`.
    pub fn control_target(&self, device: &str, app: &str) -> Result<String, Value> {
        let state = self.state();
        let matches: Vec<&String> = state.apps.iter().filter(|(_, a)| a.serial == device && a.hello.app_bundle_id == app).map(|(h, _)| h).collect();
        match matches[..] {
            [handle] => Ok(handle.clone()),
            _ => Err(bridge_error("TARGET_DISCONNECTED", format!("No unique connected app '{app}' on device '{device}'. Run necto device list."), None)),
        }
    }

    /// Discovery: plugins in a scope, one plugin's operations, or one operation's schemas.
    /// `handle` is `None` for the desktop scope, which holds no plugins in this Necto.
    pub fn control_plugins(&self, handle: Option<&str>, plugin: Option<&str>, operation: Option<&str>) -> Result<Value, Value> {
        if operation.is_some() && plugin.is_none() {
            return Err(bridge_error("INVALID_INPUT", "An operation ID requires a plugin ID.", None));
        }
        let state = self.state();
        let app = handle.and_then(|h| state.apps.get(h));
        let mut result = json!({
            "scope": if handle.is_some() { "device" } else { "desktop" },
            "target": app.map(|a| json!({ "deviceID": a.serial, "appBundleID": a.hello.app_bundle_id })),
        });
        let metadata = |id: &str, panel: &Panel| {
            json!({ "id": id, "name": panel.name(), "version": panel.manifest["version"], "description": panel.manifest["description"] })
        };
        let mut panels: Vec<(&String, &Arc<Panel>)> = app.map(|a| a.panels.iter().collect()).unwrap_or_default();
        panels.sort_by(|a, b| a.0.cmp(b.0));
        let Some(plugin_id) = plugin else {
            result["plugins"] = json!(panels.iter().map(|(id, p)| metadata(id, p)).collect::<Vec<_>>());
            return Ok(result);
        };
        let (Some(app), Some((_, panel))) = (app, panels.iter().find(|(id, _)| *id == plugin_id)) else {
            return Err(bridge_error("OPERATION_NOT_FOUND", format!("No plugin '{plugin_id}' in this scope. Run plugin list with the same target flags."), None));
        };
        let info = |op: &Value, schemas: bool| {
            let mut entry = json!({
                "id": op["id"], "title": op["title"], "description": op["description"], "kind": op["kind"],
                "timeoutMs": op["timeoutMs"].as_u64().unwrap_or(0),
            });
            let failure = Self::unavailable(app, op);
            entry["available"] = json!(failure.is_none());
            if let Some((_, reason)) = failure {
                entry["unavailableReason"] = json!(reason);
            }
            if schemas {
                entry["inputSchema"] = op["inputSchema"].clone();
                entry["outputSchema"] = op["outputSchema"].clone();
            }
            entry
        };
        let mut detail = metadata(plugin_id, panel);
        match operation {
            Some(operation_id) => {
                let op = panel.operation(operation_id).ok_or_else(|| {
                    bridge_error("OPERATION_NOT_FOUND", format!("No operation '{operation_id}' in '{plugin_id}'. Read plugin help for this target."), None)
                })?;
                result["operation"] = info(op, true);
            }
            None => detail["operations"] = json!(panel.operations().map(|op| info(op, false)).collect::<Vec<_>>()),
        }
        result["plugin"] = detail;
        Ok(result)
    }
}
