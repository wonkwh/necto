//
//  Copyright (c) 2026 Viva Republica, Inc.
//

//! `necto-cli` for Necto on Windows. Commands and output follow the Mac `necto-cli`, so
//! scripts and the `necto` agent skill work unchanged.

#[allow(dead_code)]
#[path = "../endpoint.rs"]
mod endpoint;
#[allow(dead_code)]
#[path = "../session.rs"]
mod session;

use serde_json::{json, Map, Value};
use std::process::ExitCode;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

const USAGE: &str = "\
Control a running Necto from the terminal.

USAGE:
  necto-cli targets [--json]
  necto-cli device list [--json]
  necto-cli plugin list <scope> [--json]
  necto-cli plugin help <plugin> [<operation>] <scope> [--json]
  necto-cli plugin schema <plugin> <operation> <scope>
  necto-cli plugin send <plugin> <operation> <scope> [--input <json> | --input-file <path|->]
  necto-cli plugin subscribe <plugin> <operation> <scope> [input] [--limit <n>] [--timeout <30s|500ms>]
  necto-cli skills install [--codex] [--claude] [--force]

  <scope> is --device <device-id> --app <bundle-id>, or --desktop.";

/// Exit codes: 1 for a failure, 64 for a usage error, 130 for Ctrl+C.
enum Failure {
    Usage(String),
    Failed(String),
    Interrupted,
}

#[derive(Default)]
struct Args {
    positional: Vec<String>,
    json: bool,
    desktop: bool,
    device: Option<String>,
    app: Option<String>,
    input: Option<String>,
    input_file: Option<String>,
    limit: Option<String>,
    timeout: Option<String>,
    codex: bool,
    claude: bool,
    force: bool,
}

fn parse(raw: impl Iterator<Item = String>) -> Result<Args, Failure> {
    let mut args = Args::default();
    let mut raw = raw.peekable();
    while let Some(argument) = raw.next() {
        let (flag, inline) = match argument.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag.to_string(), Some(value.to_string())),
            _ => (argument.clone(), None),
        };
        let mut value = || inline.clone().or_else(|| raw.next()).ok_or_else(|| Failure::Usage(format!("{flag} needs a value.")));
        match flag.as_str() {
            "--json" => args.json = true,
            "--desktop" => args.desktop = true,
            "--codex" => args.codex = true,
            "--claude" => args.claude = true,
            "--force" => args.force = true,
            "--device" => args.device = Some(value()?),
            "--app" => args.app = Some(value()?),
            "--input" => args.input = Some(value()?),
            "--input-file" => args.input_file = Some(value()?),
            "--limit" => args.limit = Some(value()?),
            "--timeout" => args.timeout = Some(value()?),
            "-h" | "--help" => return Err(Failure::Usage(String::new())),
            other if other.starts_with("--") => return Err(Failure::Usage(format!("Unknown option '{other}'."))),
            _ => args.positional.push(argument),
        }
    }
    Ok(args)
}

impl Args {
    /// The scope fields every plugin request carries.
    fn scope(&self, request: &mut Value) -> Result<(), Failure> {
        if self.desktop {
            if self.device.is_some() || self.app.is_some() {
                return Err(Failure::Usage("--desktop cannot be combined with --device or --app.".into()));
            }
            request["desktop"] = json!(true);
            return Ok(());
        }
        match (self.device.as_deref().filter(|s| !s.is_empty()), self.app.as_deref().filter(|s| !s.is_empty())) {
            (Some(device), Some(app)) => {
                request["device"] = json!(device);
                request["app"] = json!(app);
                Ok(())
            }
            _ => Err(Failure::Usage("Pass both --device and --app, or --desktop. Run necto-cli device list to find targets.".into())),
        }
    }

    fn input(&self) -> Result<Value, Failure> {
        let text = match (&self.input, &self.input_file) {
            (Some(_), Some(_)) => return Err(Failure::Usage("Use either --input or --input-file, not both.".into())),
            (None, None) => return Ok(json!({})),
            (Some(text), None) => text.clone(),
            (None, Some(path)) if path == "-" => std::io::read_to_string(std::io::stdin()).map_err(|e| Failure::Usage(format!("Could not read standard input: {e}")))?,
            (None, Some(path)) => std::fs::read_to_string(path).map_err(|e| Failure::Usage(format!("Could not open --input-file: {e}")))?,
        };
        if text.len() > session::MAXIMUM_MESSAGE_BYTES {
            return Err(Failure::Usage("Input exceeds the control message size limit.".into()));
        }
        serde_json::from_str(&text).map_err(|_| Failure::Usage("The input must contain valid JSON.".into()))
    }

    fn positional(&self, index: usize, name: &str) -> Result<String, Failure> {
        self.positional.get(index).cloned().ok_or_else(|| Failure::Usage(format!("Missing <{name}>.")))
    }
}

// MARK: Connection

trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

const NOT_RUNNING: &str = "Could not reach Necto. Is the app running?";

#[cfg(unix)]
async fn connect() -> Result<Box<dyn Duplex>, Failure> {
    let stream = tokio::net::UnixStream::connect(endpoint::control_socket()).await.map_err(|_| Failure::Failed(NOT_RUNNING.into()))?;
    Ok(Box::new(stream))
}

#[cfg(windows)]
async fn connect() -> Result<Box<dyn Duplex>, Failure> {
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_PIPE_BUSY: i32 = 231;
    for _ in 0..20 {
        match ClientOptions::new().open(endpoint::control_pipe()) {
            Ok(client) => return Ok(Box::new(client)),
            // Every instance is taken for a moment while the server creates the next one.
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(_) => break,
        }
    }
    Err(Failure::Failed(NOT_RUNNING.into()))
}

async fn send(stream: &mut Box<dyn Duplex>, request: &Value) -> Result<(), Failure> {
    let frame = session::frame(&serde_json::to_vec(request).unwrap_or_default()).map_err(|e| Failure::Failed(e.to_string()))?;
    stream.write_all(&frame).await.map_err(|_| Failure::Failed(NOT_RUNNING.into()))
}

async fn receive(stream: &mut Box<dyn Duplex>) -> Option<Value> {
    let bytes = session::read_frame(stream).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn remote(response: &Value) -> Failure {
    let error = &response["error"];
    Failure::Failed(display(&format!("{}: {}", error["code"].as_str().unwrap_or("FAILED"), error["message"].as_str().unwrap_or("The request failed"))))
}

/// Sends one request and returns its single result.
async fn request(mut request: Value) -> Result<Value, Failure> {
    request["id"] = json!("r1");
    let mut stream = connect().await?;
    send(&mut stream, &request).await?;
    loop {
        let response = receive(&mut stream).await.ok_or_else(|| Failure::Failed("Necto answered something this version of necto-cli does not understand".into()))?;
        if response["id"] != request["id"] {
            continue;
        }
        return match response["kind"].as_str() {
            Some("result") => Ok(response.get("value").cloned().unwrap_or_else(|| json!({}))),
            Some("error") => Err(remote(&response)),
            Some("end") => Err(Failure::Failed("CANCELLED: The request was cancelled.".into())),
            _ => Err(Failure::Failed("Necto answered something this version of necto-cli does not understand".into())),
        };
    }
}

/// Prints each event as a JSON line until `end`, an error, a bound, or Ctrl+C.
async fn stream(mut request: Value, limit: Option<u64>, timeout: Option<Duration>) -> Result<(), Failure> {
    request["id"] = json!("r1");
    let mut stream = connect().await?;
    send(&mut stream, &request).await?;
    let deadline = async {
        match timeout {
            Some(duration) => tokio::time::sleep(duration).await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(deadline);
    let mut count = 0;
    loop {
        tokio::select! {
            response = receive(&mut stream) => {
                let response = response.ok_or_else(|| Failure::Failed("Necto disconnected before the stream completed.".into()))?;
                if response["id"] != request["id"] {
                    continue;
                }
                match response["kind"].as_str() {
                    Some("event") => {
                        println!("{}", serde_json::to_string(&sorted(&response["value"])).unwrap_or_default());
                        count += 1;
                        if limit.is_some_and(|limit| count >= limit) {
                            return Ok(());
                        }
                    }
                    Some("end") => return Ok(()),
                    Some("error") => return Err(remote(&response)),
                    _ => return Err(Failure::Failed("Necto answered something this version of necto-cli does not understand".into())),
                }
            }
            _ = &mut deadline => return Ok(()),
            // Closing the connection is what cancels the stream on the host.
            _ = tokio::signal::ctrl_c() => return Err(Failure::Interrupted),
        }
    }
}

// MARK: Output

/// Keys in order at every level, as the Mac CLI prints them.
fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), sorted(&map[k]))).collect::<Map<_, _>>())
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(&sorted(value)).unwrap_or_default()
}

/// Strips control characters an app could put in a name to rewrite the terminal.
fn display(text: &str) -> String {
    text.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect()
}

fn text<'a>(value: &'a Value, key: &str, fallback: &'a str) -> &'a str {
    value[key].as_str().unwrap_or(fallback)
}

fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn schema_lines(schema: &Value, name: Option<&str>, required: bool, depth: usize) -> Vec<String> {
    let indent = "  ".repeat(depth + 1);
    if depth >= 12 {
        return vec![format!("{indent}… See --json for the complete schema.")];
    }
    let mut constraints: Vec<String> = if required { vec!["required".into()] } else { vec![] };
    for key in ["minimum", "maximum", "minLength", "maxLength", "minItems", "maxItems"] {
        if let Some(value) = schema.get(key) {
            constraints.push(format!("{key}={value}"));
        }
    }
    if let Some(values) = schema.get("enum") {
        constraints.push(format!("enum={values}"));
    }
    if schema["additionalProperties"] == false {
        constraints.push("additional properties not allowed".into());
    }
    let suffix = if constraints.is_empty() { String::new() } else { format!("  ({})", constraints.join(", ")) };
    let label = name.map(|n| format!("{n}  ")).unwrap_or_default();
    let mut lines = vec![format!("{indent}{label}{}{suffix}", text(schema, "type", "any"))];
    if let Some(description) = schema["description"].as_str().filter(|d| !d.is_empty()) {
        lines.push(format!("{indent}  {description}"));
    }
    let required: Vec<&str> = schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if let Some(properties) = schema["properties"].as_object() {
        let mut keys: Vec<&String> = properties.keys().collect();
        keys.sort();
        for key in keys {
            lines.extend(schema_lines(&properties[key], Some(key), required.contains(&key.as_str()), depth + 1));
        }
    }
    if let Some(items) = schema.get("items") {
        lines.extend(schema_lines(items, Some("items"), false, depth + 1));
    }
    lines
}

fn render_help(value: &Value) -> String {
    let plugin = &value["plugin"];
    let id = text(plugin, "id", "?");
    let mut lines = vec![format!("{id}  {}", text(plugin, "version", "")), text(plugin, "description", "").to_string()];
    if let Some(operation) = value.get("operation") {
        let operation_id = text(operation, "id", "?");
        let verb = if operation["kind"] == "stream" { "subscribe" } else { "send" };
        lines.extend(["".into(), format!("{operation_id}  {}", text(operation, "kind", "")), text(operation, "description", "").into()]);
        if operation["available"] == false {
            lines.push(format!("Unavailable: {}", text(operation, "unavailableReason", "No matching provider")));
        }
        lines.extend(["".into(), "Input:".into()]);
        lines.extend(schema_lines(&operation["inputSchema"], None, false, 0));
        lines.extend(["".into(), "Output:".into()]);
        lines.extend(schema_lines(&operation["outputSchema"], None, false, 0));
        if let Some(timeout) = operation["timeoutMs"].as_u64() {
            lines.extend(["".into(), if timeout == 0 { "Operation timeout: none".into() } else { format!("Operation timeout: {timeout}ms") }]);
        }
        let scope = match (value["scope"].as_str(), value["target"]["deviceID"].as_str(), value["target"]["appBundleID"].as_str()) {
            (Some("desktop"), _, _) => "--desktop".to_string(),
            (_, Some(device), Some(app)) => format!("--device {} --app {}", quoted(device), quoted(app)),
            _ => "--device <device-id> --app <bundle-id>".into(),
        };
        lines.extend(["".into(), format!("necto-cli plugin {verb} {} {} {scope} --input '<json>'", quoted(id), quoted(operation_id))]);
        if verb == "subscribe" {
            lines.push("Use --limit <count> or --timeout 30s to bound the subscription.".into());
        }
    } else {
        lines.extend(["".into(), "Operations:".into()]);
        for operation in plugin["operations"].as_array().into_iter().flatten() {
            lines.push(format!("  {}  {}", text(operation, "id", "?"), text(operation, "kind", "")));
            lines.push(format!("    {}", text(operation, "description", "")));
            if operation["available"] == false {
                lines.push(format!("    Unavailable: {}", text(operation, "unavailableReason", "No matching provider")));
            }
        }
        lines.extend(["".into(), format!("Use necto-cli plugin help {id} <operation-id> with the same target flags for schemas.")]);
    }
    display(&lines.join("\n"))
}

/// `devices` grouping the flat target list by device, as `device list --json` prints it.
fn grouped(targets: &Value) -> Value {
    let mut devices: std::collections::BTreeMap<String, Vec<&Value>> = Default::default();
    for target in targets["targets"].as_array().into_iter().flatten() {
        devices.entry(text(target, "deviceID", "").to_string()).or_default().push(target);
    }
    let devices: Vec<Value> = devices
        .into_iter()
        .map(|(id, targets)| {
            let mut apps: Vec<Value> = targets.iter().map(|t| json!({ "bundleID": t["appBundleID"], "name": t["appName"] })).collect();
            apps.sort_by(|a, b| a["bundleID"].as_str().cmp(&b["bundleID"].as_str()));
            json!({ "id": id, "name": targets.first().map(|t| t["deviceName"].clone()).unwrap_or(json!(id)), "apps": apps })
        })
        .collect();
    json!({ "devices": devices })
}

/// `"30s"` or `"500ms"`.
fn duration(text: &str) -> Result<Duration, Failure> {
    let (number, scale) = match text.strip_suffix("ms") {
        Some(number) => (number, 0.001),
        None => (text.strip_suffix('s').ok_or_else(|| Failure::Usage("--timeout requires a unit: 30s or 500ms.".into()))?, 1.0),
    };
    let seconds = number.parse::<f64>().ok().map(|n| n * scale).filter(|s| s.is_finite() && *s > 0.0);
    seconds.map(Duration::from_secs_f64).ok_or_else(|| Failure::Usage("--timeout must be a positive duration in seconds or milliseconds.".into()))
}

// MARK: Skills

/// The agent skill, written for this CLI. It works without a running Necto.
const SKILL: &str = include_str!("../../resources/skills/necto/SKILL.md");

/// Writes SKILL.md for Codex and Claude Code. A changed copy is replaced only with `--force`.
fn install_skills(args: &Args) -> Result<(), Failure> {
    if !args.codex && !args.claude {
        return Err(Failure::Usage("Choose --codex, --claude, or both.".into()));
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(std::path::PathBuf::from)
        .ok_or_else(|| Failure::Failed("Could not find the home folder.".into()))?;
    let agents = [(args.codex, ".agents"), (args.claude, ".claude")];
    let destinations: Vec<_> = agents.iter().filter(|(on, _)| *on).map(|(_, root)| home.join(root).join("skills").join("necto").join("SKILL.md")).collect();
    let fail = |message: String| Failure::Failed(message);

    // Every destination is checked before any is written.
    for destination in &destinations {
        let mut current = destination.as_path();
        while current != home {
            if std::fs::symlink_metadata(current).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(fail(format!("Refusing to install through a symbolic link: {}", current.display())));
            }
            current = current.parent().ok_or_else(|| fail("The skill folder is outside the home folder.".into()))?;
        }
        if let Ok(metadata) = std::fs::symlink_metadata(destination) {
            if !metadata.is_file() {
                return Err(fail(format!("Expected a regular file at {}", destination.display())));
            }
            if std::fs::read(destination).ok().as_deref() != Some(SKILL.as_bytes()) && !args.force {
                return Err(fail(format!("{} differs from the bundled skill. Use --force to replace it.", destination.display())));
            }
        }
    }
    for destination in destinations {
        if std::fs::read(&destination).ok().as_deref() == Some(SKILL.as_bytes()) {
            println!("Already installed: {}", destination.display());
            continue;
        }
        let folder = destination.parent().expect("SKILL.md has a folder");
        std::fs::create_dir_all(folder).map_err(|e| fail(format!("Could not create {}: {e}", folder.display())))?;
        let staged = folder.join(".SKILL.md.partial");
        std::fs::write(&staged, SKILL).and_then(|_| std::fs::rename(&staged, &destination)).map_err(|e| fail(format!("Could not write {}: {e}", destination.display())))?;
        println!("Installed: {}", destination.display());
    }
    Ok(())
}

// MARK: Commands

async fn run(args: Args) -> Result<(), Failure> {
    let words: Vec<&str> = args.positional.iter().map(String::as_str).collect();
    match words[..] {
        ["targets"] => {
            let value = request(json!({ "kind": "targets" })).await?;
            if args.json {
                println!("{}", pretty(&value));
                return Ok(());
            }
            let targets = value["targets"].as_array().cloned().unwrap_or_default();
            if targets.is_empty() {
                println!("No app is connected.");
            }
            for target in targets {
                println!("{}", display(&format!("{}  ·  {}", text(&target, "appBundleID", "?"), text(&target, "deviceName", text(&target, "deviceID", "?")))));
            }
        }
        ["device", "list"] => {
            let value = grouped(&request(json!({ "kind": "targets" })).await?);
            if args.json {
                println!("{}", pretty(&value));
                return Ok(());
            }
            let devices = value["devices"].as_array().cloned().unwrap_or_default();
            if devices.is_empty() {
                println!("No connected apps. Launch an SDK-enabled app and keep Necto running.");
            }
            for device in devices {
                println!("{}", display(&format!("{}  {}", text(&device, "id", "?"), text(&device, "name", ""))));
                for app in device["apps"].as_array().into_iter().flatten() {
                    println!("{}", display(&format!("  {}  {}", text(app, "bundleID", "?"), text(app, "name", ""))));
                }
            }
        }
        ["plugin", "list"] => {
            let mut message = json!({ "kind": "plugins" });
            args.scope(&mut message)?;
            let value = request(message).await?;
            if args.json {
                println!("{}", pretty(&value));
                return Ok(());
            }
            let plugins = value["plugins"].as_array().cloned().unwrap_or_default();
            if plugins.is_empty() {
                println!("No plugins in this scope.");
            }
            for plugin in plugins {
                println!("{}", display(&format!("{}  {}", text(&plugin, "id", "?"), text(&plugin, "version", ""))));
                println!("{}", display(&format!("  {}", text(&plugin, "description", ""))));
            }
        }
        ["plugin", "help", plugin, ..] => {
            let mut message = json!({ "kind": "plugins", "pluginID": plugin });
            if let Some(operation) = words.get(3) {
                message["operationID"] = json!(operation);
            }
            args.scope(&mut message)?;
            let value = request(message).await?;
            println!("{}", if args.json { pretty(&value) } else { render_help(&value) });
        }
        ["plugin", "schema", ..] => {
            let mut message = json!({ "kind": "plugins", "pluginID": args.positional(2, "plugin")?, "operationID": args.positional(3, "operation")? });
            args.scope(&mut message)?;
            let operation = request(message).await?["operation"].clone();
            println!("{}", pretty(&json!({ "input": operation["inputSchema"], "output": operation["outputSchema"] })));
        }
        ["plugin", "send" | "invoke", ..] => {
            let mut message = json!({ "kind": "invoke", "pluginID": args.positional(2, "plugin")?, "operationID": args.positional(3, "operation")?, "input": args.input()? });
            args.scope(&mut message)?;
            println!("{}", pretty(&request(message).await?));
        }
        ["plugin", "subscribe", ..] => {
            let mut message = json!({ "kind": "subscribe", "pluginID": args.positional(2, "plugin")?, "operationID": args.positional(3, "operation")?, "input": args.input()? });
            args.scope(&mut message)?;
            let limit = match &args.limit {
                Some(text) => Some(text.parse::<u64>().ok().filter(|n| *n > 0).ok_or_else(|| Failure::Usage("--limit must be greater than zero.".into()))?),
                None => None,
            };
            let timeout = args.timeout.as_deref().map(duration).transpose()?;
            stream(message, limit, timeout).await?;
        }
        ["skills", "install"] => install_skills(&args)?,
        ["install" | "delete" | "shell" | "pack", ..] | ["plugin", "install" | "delete" | "pack", ..] => {
            return Err(Failure::Failed(format!("'{}' is not available in Necto for Windows yet.", words.join(" "))));
        }
        _ => return Err(Failure::Usage(String::new())),
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let result = match parse(std::env::args().skip(1)) {
        Ok(args) => run(args).await,
        Err(failure) => Err(failure),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            if !message.is_empty() {
                eprintln!("Error: {message}\n");
            }
            eprintln!("{USAGE}");
            ExitCode::from(64)
        }
        Err(Failure::Failed(message)) => {
            eprintln!("Error: {message}");
            ExitCode::FAILURE
        }
        Err(Failure::Interrupted) => ExitCode::from(130),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags_scope_and_durations() {
        let args = parse(["plugin", "send", "p", "op", "--device=emulator-5554", "--app", "a.b", "--input", "{\"x\":1}"].map(String::from).into_iter()).ok().unwrap();
        let mut message = json!({});
        assert!(args.scope(&mut message).is_ok());
        assert_eq!(message, json!({ "device": "emulator-5554", "app": "a.b" }));
        assert_eq!(args.input().ok().unwrap(), json!({ "x": 1 }));

        let both = parse(["--desktop", "--app", "a"].map(String::from).into_iter()).ok().unwrap();
        assert!(both.scope(&mut json!({})).is_err());
        assert!(parse(["--bogus"].map(String::from).into_iter()).is_err());

        assert_eq!(duration("500ms").ok(), Some(Duration::from_millis(500)));
        assert_eq!(duration("2s").ok(), Some(Duration::from_secs(2)));
        assert!(duration("2").is_err() && duration("0s").is_err());
    }

    #[test]
    fn groups_targets_and_renders_help() {
        let targets = json!({ "targets": [
            { "deviceID": "d1", "deviceName": "Pixel", "appBundleID": "b.app", "appName": "B" },
            { "deviceID": "d1", "deviceName": "Pixel", "appBundleID": "a.app", "appName": "A" },
        ]});
        assert_eq!(grouped(&targets)["devices"][0]["apps"][0]["bundleID"], "a.app");

        let help = json!({
            "scope": "device", "target": { "deviceID": "d1", "appBundleID": "a.app" },
            "plugin": { "id": "p", "version": "1.0.0", "description": "P" },
            "operation": { "id": "list", "kind": "once", "description": "List", "available": true, "timeoutMs": 0,
                "inputSchema": { "type": "object", "properties": { "limit": { "type": "number", "minimum": 1 } }, "required": ["limit"] },
                "outputSchema": { "type": "object" } }
        });
        let rendered = render_help(&help);
        assert!(rendered.contains("    limit  number  (required, minimum=1)"));
        assert!(rendered.contains("necto-cli plugin send 'p' 'list' --device 'd1' --app 'a.app' --input '<json>'"));
        assert!(rendered.contains("Operation timeout: none"));
    }
}
